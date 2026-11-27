//! The strip window: owns the taskbar strip, refresh timers, menu and device watchers.

use std::cell::RefCell;

use windows::Devices::Enumeration::DeviceWatcher;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND, GetClassNameW, GetCursorPos, GetMessageW,
    GetWindowRect, GetWindowThreadProcessId, IDC_ARROW, LoadCursorW, HMENU, HWND_TOPMOST, KillTimer, MB_ICONERROR, MB_ICONINFORMATION, MF_CHECKED, MF_GRAYED,
    MF_SEPARATOR, MF_STRING, MSG, MessageBoxW, OBJID_WINDOW, PBT_APMRESUMEAUTOMATIC, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_DESTROY, WM_LBUTTONUP, WM_NULL,
    WM_POWERBROADCAST, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WM_USER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::about;
use crate::updater::{self, Choice};
use crate::battery::{self, Kind, Reading};
use crate::connected;
use crate::flyout::{self, Card};
use crate::settings;
use crate::strip::{self, Chip, Rgb};

const WM_DEVICES_CHANGED: u32 = WM_USER + 1;
/// update check finished: wparam 1 if the user asked, lparam a Box<UpdateResult>
const WM_UPDATE_RESULT: u32 = WM_USER + 2;
type UpdateResult = std::result::Result<Option<updater::Update>, String>;

const TIMER_BACKGROUND: usize = 1;
const TIMER_SOON: usize = 2; // after a connect/disconnect, battery change or wake
const TIMER_LATE: usize = 3; // headphones report battery several seconds after connecting
const TIMER_TOPMOST: usize = 4; // the taskbar can end up above the strip, e.g. after clicking it
const TIMER_HOVER: usize = 5; // while the device card is open: close it once the pointer leaves
const TIMER_UPDATE: usize = 6; // daily update check
const TIMER_UPDATE_FIRST: usize = 7; // update check shortly after launch
const UPDATE_MS: u32 = 24 * 60 * 60 * 1000;
const UPDATE_FIRST_MS: u32 = 10_000;
const HOVER_MS: u32 = 200;
const HOVER_TICKS: u8 = 3; // ~0.6 s away from the card and its chip
/// Safety net for devices whose battery changes Windows does not report as events.
const BACKGROUND_MS: u32 = 10 * 60 * 1000;
const SOON_MS: u32 = 3_000;
const LATE_MS: u32 = 15_000;
const WAKE_MS: u32 = 15_000;
const TOPMOST_MS: u32 = 5_000;

const ID_REFRESH: usize = 1;
const ID_LOGIN: usize = 2;
const ID_QUIT: usize = 3;
const ID_ABOUT: usize = 4;
const ID_CHECK_UPDATES: usize = 5;

const LOW: u8 = 20;
const RED: Rgb = Rgb(0xFF, 0x5F, 0x57);
const GREEN: Rgb = Rgb(0x4A, 0xDE, 0x80);
const WHITE: Rgb = Rgb(0xFF, 0xFF, 0xFF);
const BLACK: Rgb = Rgb(0x1A, 0x1A, 0x1A);
const GLYPH_BLUETOOTH: char = '\u{E702}';

struct App {
    hwnd: HWND,
    readings: Vec<Reading>,
    error: Option<String>,
    updated: Option<String>,
    taskbar_created: u32,
    taskbar_hook: HWINEVENTHOOK,
    fullscreen: bool,
    /// horizontal range of each chip, same order as `readings`
    chips: Vec<(i32, i32)>,
    flyout: HWND,
    /// device whose card is open; the card redraws on every refresh
    flyout_device: Option<String>,
    /// the open card's chip, in screen coordinates
    flyout_anchor: RECT,
    /// consecutive hover checks with the pointer away from the card and its chip
    pointer_away: u8,
    watchers: [DeviceWatcher; 2],
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn run() -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("PipsStrip");
        // arrow cursor: without one Windows keeps showing whatever cursor was last set, often the busy one
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance.into(), lpszClassName: class, hCursor: arrow, ..Default::default() });
        // layered: per-pixel transparency; tool window: no taskbar button; no-activate: clicks never steal focus
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!("Pips"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        let target = hwnd.0 as usize; // HWND is not Send; the watcher threads only post to it
        let changed = move || {
            let _ = PostMessageW(Some(HWND(target as _)), WM_DEVICES_CHANGED, WPARAM(0), LPARAM(0));
        };
        let watchers = [connected::watch_connected(changed)?, battery::watch(changed)?];
        let app = App {
            hwnd,
            readings: Vec::new(),
            error: None,
            updated: None,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            taskbar_hook: hook_taskbar()?,
            fullscreen: false,
            chips: Vec::new(),
            flyout: flyout::create(instance.into())?,
            flyout_device: None,
            flyout_anchor: RECT::default(),
            pointer_away: 0,
            watchers,
        };
        APP.with(|a| *a.borrow_mut() = Some(app));
        // any window becoming active: hide over fullscreen apps, stay above the taskbar
        SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT);
        SetTimer(Some(hwnd), TIMER_BACKGROUND, BACKGROUND_MS, None);
        SetTimer(Some(hwnd), TIMER_TOPMOST, TOPMOST_MS, None);
        SetTimer(Some(hwnd), TIMER_UPDATE, UPDATE_MS, None);
        SetTimer(Some(hwnd), TIMER_UPDATE_FIRST, UPDATE_FIRST_MS, None);
        with_app(|a| a.refresh());
        with_app(|a| a.render()).unwrap_or(Ok(()))?; // fail fast if the taskbar is not found
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// None before startup finishes, or when a Windows call inside `f` re-entered `window_proc`.
fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|a| a.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Follows the taskbar and its tray area as they move, resize or auto-hide. Scoped to Explorer's
/// taskbar thread, so it costs nothing for other windows.
fn hook_taskbar() -> Result<HWINEVENTHOOK> {
    let bar = strip::taskbar()?;
    let mut pid = 0;
    let thread = unsafe { GetWindowThreadProcessId(bar.window, Some(&mut pid)) };
    Ok(unsafe { SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, None, Some(on_taskbar_moved), pid, thread, WINEVENT_OUTOFCONTEXT) })
}

unsafe extern "system" fn on_taskbar_moved(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, _: i32, _: u32, _: u32) {
    if object != OBJID_WINDOW.0 {
        return;
    }
    if let Ok(bar) = strip::taskbar() {
        if hwnd == bar.window || hwnd == bar.tray_area {
            with_app(|a| a.render());
        }
    }
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    let fullscreen = is_fullscreen(hwnd);
    with_app(|a| {
        if fullscreen != a.fullscreen {
            a.fullscreen = fullscreen;
            let _ = unsafe { ShowWindow(a.hwnd, if fullscreen { SW_HIDE } else { SW_SHOWNOACTIVATE }) };
        }
        a.keep_on_top();
    });
}

/// A window covering its whole monitor, other than the desktop and taskbar.
fn is_fullscreen(hwnd: HWND) -> bool {
    let mut class = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
    let class = String::from_utf16_lossy(&class[..len]);
    if hwnd.is_invalid() || ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"].contains(&class.as_str()) {
        return false;
    }
    let mut window = RECT::default();
    let mut monitor = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        if GetWindowRect(hwnd, &mut window).is_err() || !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut monitor).as_bool() {
            return false;
        }
    }
    let m = monitor.rcMonitor;
    window.left <= m.left && window.top <= m.top && window.right >= m.right && window.bottom >= m.bottom
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONUP => show_device(hwnd, (lparam.0 & 0xFFFF) as i16 as i32),
        WM_RBUTTONUP => show_menu(hwnd),
        WM_DEVICES_CHANGED => {
            // re-setting a timer restarts it, so a burst of events causes one refresh
            unsafe {
                SetTimer(Some(hwnd), TIMER_SOON, SOON_MS, None);
                SetTimer(Some(hwnd), TIMER_LATE, LATE_MS, None);
            }
        }
        WM_TIMER if wparam.0 == TIMER_UPDATE || wparam.0 == TIMER_UPDATE_FIRST => {
            if wparam.0 == TIMER_UPDATE_FIRST {
                let _ = unsafe { KillTimer(Some(hwnd), TIMER_UPDATE_FIRST) };
            }
            if settings::auto_update() {
                start_update_check(hwnd, false);
            }
        }
        WM_UPDATE_RESULT => {
            let result = unsafe { Box::from_raw(lparam.0 as *mut UpdateResult) };
            handle_update(hwnd, wparam.0 == 1, *result);
        }
        WM_TIMER if wparam.0 == TIMER_HOVER => {
            with_app(|a| a.close_flyout_if_pointer_left());
        }
        WM_TIMER if wparam.0 == TIMER_TOPMOST => {
            with_app(|a| a.keep_on_top());
        }
        WM_TIMER => {
            if wparam.0 != TIMER_BACKGROUND {
                let _ = unsafe { KillTimer(Some(hwnd), wparam.0) };
            }
            with_app(|a| a.refresh());
        }
        WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize => unsafe {
            SetTimer(Some(hwnd), TIMER_SOON, WAKE_MS, None);
        },
        WM_SETTINGCHANGE => {
            with_app(|a| a.render()); // light/dark taskbar or display scaling may have changed
        }
        WM_DESTROY => {
            with_app(|a| {
                a.watchers.iter().for_each(|w| drop(w.Stop()));
                let _ = unsafe { UnhookWinEvent(a.taskbar_hook) };
            });
            unsafe { PostQuitMessage(0) };
        }
        _ => {
            if with_app(|a| msg == a.taskbar_created) == Some(true) {
                // Explorer restarted: the taskbar is a new window
                with_app(|a| {
                    let _ = unsafe { UnhookWinEvent(a.taskbar_hook) };
                    if let Ok(hook) = hook_taskbar() {
                        a.taskbar_hook = hook;
                    }
                    a.render()
                });
            }
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Left click: opens the clicked device's card, or closes it when it is already open.
fn show_device(_hwnd: HWND, x: i32) {
    with_app(|a| {
        let Some(i) = a.chips.iter().position(|(left, right)| x >= *left && x < *right) else { return };
        let name = a.readings.get(i).map(|r| r.name.clone());
        if flyout::is_open(a.flyout) && a.flyout_device == name {
            flyout::hide(a.flyout);
            a.flyout_device = None;
            return;
        }
        a.flyout_device = name;
        a.update_flyout();
    });
}

/// Asks GitHub on a worker thread; the answer comes back as WM_UPDATE_RESULT.
fn start_update_check(hwnd: HWND, manual: bool) {
    let target = hwnd.0 as usize; // HWND is not Send; the worker only posts to it
    std::thread::spawn(move || {
        let result: UpdateResult = updater::check().map_err(|e| e.message());
        let boxed = Box::into_raw(Box::new(result)) as isize;
        if unsafe { PostMessageW(Some(HWND(target as _)), WM_UPDATE_RESULT, WPARAM(manual as usize), LPARAM(boxed)) }.is_err() {
            drop(unsafe { Box::from_raw(boxed as *mut UpdateResult) });
        }
    });
}

/// Runs outside `with_app`: the dialogs pump messages that re-enter `window_proc`.
fn handle_update(hwnd: HWND, manual: bool, result: UpdateResult) {
    let instance = unsafe { GetModuleHandleW(None) }.map(Into::into).unwrap_or_default();
    let message = |text: &str, icon| unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Pips update"), icon) };
    let update = match result {
        Ok(Some(u)) => u,
        Ok(None) => {
            if manual {
                message(&format!("Pips is up to date. You have the latest version, {}.", about::VERSION), MB_ICONINFORMATION);
            }
            return;
        }
        Err(e) => {
            if manual {
                message(&format!("Could not check for updates: {e}"), MB_ICONERROR);
            }
            return;
        }
    };
    if !manual && settings::skipped_version().as_deref() == Some(update.version.as_str()) {
        return;
    }
    let (choice, auto) = match updater::offer(hwnd, instance, &update, settings::auto_update()) {
        Ok(c) => c,
        Err(e) => {
            message(&format!("Showing the update failed: {}", e.message()), MB_ICONERROR);
            return;
        }
    };
    let _ = settings::set_auto_update(auto);
    match choice {
        Choice::Later => {}
        Choice::Skip => {
            let _ = settings::set_skipped_version(&update.version);
        }
        Choice::Install => match updater::download_with_progress(hwnd, instance, &update) {
            None => {} // cancelled
            Some(Err(e)) => {
                message(&format!("Update failed: {e}"), MB_ICONERROR);
            }
            Some(Ok(exe)) => match updater::install(&exe) {
                Ok(()) => {
                    let _ = unsafe { DestroyWindow(hwnd) }; // the new copy takes over once this one exits
                }
                Err(e) => {
                    message(&format!("Update failed: {e}"), MB_ICONERROR);
                }
            },
        },
    }
}

/// Right click: devices and options. Runs outside `with_app`: the menu and message boxes pump
/// messages that re-enter `window_proc`.
fn show_menu(hwnd: HWND) {
    let Some(menu) = with_app(|a| a.menu()) else { return };
    let command = unsafe { popup(hwnd, menu) };
    let result = match command {
        ID_REFRESH => {
            with_app(|a| a.refresh());
            Ok(())
        }
        ID_LOGIN => settings::set_launch_at_login(!settings::launch_at_login()),
        ID_QUIT => unsafe { DestroyWindow(hwnd) },
        ID_ABOUT => about::show(hwnd, unsafe { GetModuleHandleW(None) }.map(Into::into).unwrap_or_default()),
        ID_CHECK_UPDATES => {
            start_update_check(hwnd, true);
            Ok(())
        }
        _ => Ok(()),
    };
    if let Err(e) = result {
        unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(e.message()), w!("Pips"), MB_ICONERROR) };
    }
}

/// Shows `menu` at the cursor, destroys it, and returns the chosen command id (0 for none).
unsafe fn popup(hwnd: HWND, menu: HMENU) -> usize {
    unsafe {
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(hwnd); // so the menu closes when clicking elsewhere
        let command = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, point.x, point.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        command.0 as usize
    }
}

impl App {
    fn refresh(&mut self) {
        match battery::read() {
            Ok(readings) => {
                self.readings = readings;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Reading battery failed: {}", e.message())),
        }
        self.updated = Some(now());
        let _ = self.render(); // a missing taskbar (Explorer restarting) recovers on TaskbarCreated
    }

    fn render(&mut self) -> Result<()> {
        let normal = if settings::light_taskbar() { BLACK } else { WHITE };
        let mut chips: Vec<Chip> = self
            .readings
            .iter()
            .map(|r| Chip { glyph: glyph(r.kind), text: format!("{}%", r.level), color: if r.level <= LOW { RED } else { normal } })
            .collect();
        if chips.is_empty() {
            // the strip stays so the menu (and Quit) is always reachable
            chips.push(Chip { glyph: GLYPH_BLUETOOTH, text: String::new(), color: normal });
        }
        self.chips = strip::update(self.hwnd, &chips)?;
        if flyout::is_open(self.flyout) {
            self.update_flyout(); // live: follows new levels and strip moves
        }
        Ok(())
    }

    /// Draws the card for `flyout_device` above its chip; closes it when the device is gone.
    fn update_flyout(&mut self) {
        let index = self.flyout_device.as_ref().and_then(|n| self.readings.iter().position(|r| &r.name == n));
        let (Some(i), Some(&(left, right))) = (index, index.and_then(|i| self.chips.get(i))) else {
            flyout::hide(self.flyout);
            self.flyout_device = None;
            return;
        };
        let r = &self.readings[i];
        let mut strip_rect = RECT::default();
        let _ = unsafe { GetWindowRect(self.hwnd, &mut strip_rect) };
        let low = r.level <= LOW;
        let card = Card {
            glyph: glyph(r.kind),
            name: r.name.clone(),
            subtitle: format!("{} · Connected", kind_name(r.kind)),
            level: r.level,
            accent: if low { RED } else { GREEN },
            low,
            dark: !settings::light_taskbar(),
        };
        if let Err(e) = flyout::show(self.flyout, &card, strip_rect.left + (left + right) / 2, strip_rect.top) {
            self.error = Some(format!("Showing device card failed: {}", e.message()));
            return;
        }
        self.flyout_anchor = RECT { left: strip_rect.left + left, top: strip_rect.top, right: strip_rect.left + right, bottom: strip_rect.bottom };
        unsafe { SetTimer(Some(self.hwnd), TIMER_HOVER, HOVER_MS, None) };
    }

    /// Closes the card once the pointer has been away from it and its chip for a moment.
    fn close_flyout_if_pointer_left(&mut self) {
        if !flyout::is_open(self.flyout) {
            let _ = unsafe { KillTimer(Some(self.hwnd), TIMER_HOVER) };
            self.pointer_away = 0;
            return;
        }
        let mut card = RECT::default();
        let mut point = POINT::default();
        unsafe {
            let _ = GetWindowRect(self.flyout, &mut card);
            let _ = GetCursorPos(&mut point);
        }
        // one area covering the chip, the card and the arrow gap between them
        let a = self.flyout_anchor;
        let margin = 6;
        let inside = point.x >= card.left.min(a.left) - margin
            && point.x <= card.right.max(a.right) + margin
            && point.y >= card.top.min(a.top) - margin
            && point.y <= card.bottom.max(a.bottom) + margin;
        self.pointer_away = if inside { 0 } else { self.pointer_away + 1 };
        if self.pointer_away >= HOVER_TICKS {
            flyout::hide(self.flyout);
            self.flyout_device = None;
            self.pointer_away = 0;
            let _ = unsafe { KillTimer(Some(self.hwnd), TIMER_HOVER) };
        }
    }

    fn keep_on_top(&self) {
        let _ = unsafe { SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
    }

    fn menu(&self) -> HMENU {
        unsafe {
            let menu = CreatePopupMenu().unwrap_or_default();
            for r in &self.readings {
                append(menu, MF_STRING, 0, &format!("{}\t{}%", r.name, r.level));
            }
            if self.readings.is_empty() {
                append(menu, MF_STRING | MF_GRAYED, 0, "No Bluetooth devices reporting battery");
            }
            if let Some(e) = &self.error {
                append(menu, MF_STRING | MF_GRAYED, 0, e);
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            if let Some(t) = &self.updated {
                append(menu, MF_STRING | MF_GRAYED, 0, &format!("Updated {t}"));
            }
            append(menu, MF_STRING, ID_REFRESH, "Refresh now");
            let login = if settings::launch_at_login() { MF_CHECKED } else { MF_STRING };
            append(menu, MF_STRING | login, ID_LOGIN, "Launch at login");
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            append(menu, MF_STRING, ID_CHECK_UPDATES, "Check for updates…");
            append(menu, MF_STRING, ID_ABOUT, "About Pips");
            append(menu, MF_STRING, ID_QUIT, "Quit Pips");
            menu
        }
    }
}

unsafe fn append(menu: HMENU, flags: windows::Win32::UI::WindowsAndMessaging::MENU_ITEM_FLAGS, id: usize, text: &str) {
    let _ = unsafe { AppendMenuW(menu, flags, id, &HSTRING::from(text)) };
}

/// Segoe Fluent Icons glyphs.
fn glyph(kind: Kind) -> char {
    match kind {
        Kind::Mouse => '\u{E962}',
        Kind::Keyboard => '\u{E765}',
        Kind::Headphones => '\u{E7F6}',
        Kind::Phone => '\u{E8EA}',
        Kind::Other => GLYPH_BLUETOOTH,
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Mouse => "Mouse",
        Kind::Keyboard => "Keyboard",
        Kind::Headphones => "Headphones",
        Kind::Phone => "Phone",
        Kind::Other => "Bluetooth device",
    }
}

/// Current time in the user's short time format, e.g. "2:14 PM".
fn now() -> String {
    let mut buffer = [0u16; 64];
    let len = unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, None, PCWSTR::null(), Some(&mut buffer)) };
    String::from_utf16_lossy(&buffer[..(len.max(1) - 1) as usize])
}
