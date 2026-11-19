#![windows_subsystem = "windows"] // tray app: no console window

mod about;
mod app;
mod battery;
mod connected;
mod flyout;
mod settings;
mod strip;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
use windows::core::{HSTRING, w};

fn main() {
    if std::env::args().any(|a| a == "--list") {
        list();
        return;
    }
    unsafe {
        // a second copy would add duplicate tray icons
        let _mutex = CreateMutexW(None, true, w!("Local\\Pips"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if let Err(e) = app::run() {
            MessageBoxW(None, &HSTRING::from(format!("Pips failed to start: {}", e.message())), w!("Pips"), MB_ICONERROR);
            std::process::exit(1);
        }
    }
}

/// `pips.exe --list`: prints readings to the terminal it was started from.
fn list() {
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    match battery::read() {
        Ok(readings) if readings.is_empty() => println!("No Bluetooth devices reporting battery"),
        Ok(readings) => readings.iter().for_each(|r| println!("{}: {}% ({:?})", r.name, r.level, r.kind)),
        Err(e) => {
            eprintln!("Reading Bluetooth battery failed: {e}");
            std::process::exit(1);
        }
    }
}
