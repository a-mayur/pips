//! Device card: a small popup above a clicked strip chip, with an arrow pointing at it.
//! Closes when it loses focus or on Esc.

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_END_ELLIPSIS, DT_LEFT, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FW_NORMAL, FW_SEMIBOLD, GdiFlush, GetMonitorInfoW, HDC, HFONT,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, IDC_ARROW, IsWindowVisible, LoadCursorW, RegisterClassW, SW_HIDE, SW_SHOW, SetForegroundWindow,
    ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WA_INACTIVE, WM_ACTIVATE, WM_KEYDOWN, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Result, w};

use crate::strip::{Rgb, font, width};

pub struct Card {
    pub glyph: char,
    pub name: String,
    /// e.g. "Mouse · Connected"
    pub subtitle: String,
    pub level: u8,
    /// level bar and, when low, the percentage
    pub accent: Rgb,
    pub low: bool,
    pub dark: bool,
}

pub fn create(instance: HINSTANCE) -> Result<HWND> {
    unsafe {
        let class = w!("PipsFlyout");
        let arrow = LoadCursorW(None, IDC_ARROW)?; // otherwise the busy cursor can stick over the card
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance, lpszClassName: class, hCursor: arrow, ..Default::default() });
        CreateWindowExW(WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST, class, w!("Pips"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None)
    }
}

pub fn is_open(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd) }.as_bool()
}

pub fn hide(hwnd: HWND) {
    let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let lost_focus = msg == WM_ACTIVATE && (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE;
    let escape = msg == WM_KEYDOWN && wparam.0 == VK_ESCAPE.0 as usize;
    if lost_focus || escape {
        hide(hwnd);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Draws `card` with its arrow tip at screen point (`anchor_x`, `anchor_y`), opening upward.
pub fn show(hwnd: HWND, card: &Card, anchor_x: i32, anchor_y: i32) -> Result<()> {
    let scale = unsafe { GetDpiForWindow(hwnd) } as i32;
    let px = |v: i32| v * scale / 96;
    let (w, pad, radius, arrow_half, arrow_height) = (px(260), px(14), px(8), px(9), px(9));
    let (row, gap, bar_height) = (px(40), px(10), px(6));
    let bar_top = pad + row + gap;
    let body = bar_top + bar_height + pad;
    let h = body + arrow_height;

    // keep the card on the anchor's monitor
    let mut monitor = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(MonitorFromPoint(POINT { x: anchor_x, y: anchor_y }, MONITOR_DEFAULTTONEAREST), &mut monitor) };
    let area = monitor.rcWork;
    let left = (anchor_x - w / 2).clamp(area.left + px(8), area.right - w - px(8));
    let top = anchor_y - h - px(2);
    let arrow_x = (anchor_x - left).clamp(radius + arrow_half, w - radius - arrow_half) as f32;

    let (bg, primary, secondary, track) = if card.dark {
        (Rgb(0x2B, 0x2B, 0x2B), Rgb(0xFF, 0xFF, 0xFF), Rgb(0xA8, 0xA8, 0xA8), Rgb(0x45, 0x45, 0x45))
    } else {
        (Rgb(0xF9, 0xF9, 0xF9), Rgb(0x1A, 0x1A, 0x1A), Rgb(0x66, 0x66, 0x66), Rgb(0xDD, 0xDD, 0xDD))
    };
    let percent_color = if card.low { card.accent } else { primary };

    unsafe {
        let dc = CreateCompatibleDC(None);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, bitmap.into());

        // text as a coverage mask, one channel per color: red = name and glyph, green = subtitle, blue = percent
        SetBkMode(dc, TRANSPARENT);
        let glyph_font = font(px(26), FW_NORMAL.0 as i32, w!("Segoe Fluent Icons"));
        let name_font = font(px(14), FW_SEMIBOLD.0 as i32, w!("Segoe UI"));
        let sub_font = font(px(12), FW_NORMAL.0 as i32, w!("Segoe UI"));
        let pct_font = font(px(22), FW_SEMIBOLD.0 as i32, w!("Segoe UI"));
        let percent: Vec<u16> = format!("{}%", card.level).encode_utf16().collect();
        let text_left = pad + px(38);
        let pct_left = w - pad - width(dc, pct_font, &percent);
        let text_right = pct_left - px(8);
        let glyph: Vec<u16> = card.glyph.to_string().encode_utf16().collect();
        let name: Vec<u16> = card.name.encode_utf16().collect();
        let subtitle: Vec<u16> = card.subtitle.encode_utf16().collect();
        text(dc, glyph_font, &glyph, RECT { left: pad, top: pad, right: pad + px(32), bottom: pad + row }, 0x0000FF, DT_LEFT);
        text(dc, name_font, &name, RECT { left: text_left, top: pad, right: text_right, bottom: pad + px(20) }, 0x0000FF, DT_LEFT | DT_END_ELLIPSIS);
        text(dc, sub_font, &subtitle, RECT { left: text_left, top: pad + px(20), right: text_right, bottom: pad + row }, 0x00FF00, DT_LEFT | DT_END_ELLIPSIS);
        text(dc, pct_font, &percent, RECT { left: pct_left, top: pad, right: w - pad, bottom: pad + row }, 0xFF0000, DT_RIGHT);
        let _ = GdiFlush();

        let bar = (pad as f32, bar_top as f32, (w - pad) as f32, (bar_top + bar_height) as f32);
        let fill_right = bar.0 + (bar.2 - bar.0) * card.level.min(100) as f32 / 100.0;
        let pixels = std::slice::from_raw_parts_mut(bits as *mut [u8; 4], (w * h) as usize);
        for (i, p) in pixels.iter_mut().enumerate() {
            let (x, y) = ((i as i32 % w) as f32 + 0.5, (i as i32 / w) as f32 + 0.5);
            let shape = rounded(x, y, (0.0, 0.0, w as f32, body as f32), radius as f32)
                .max(arrow(x, y, arrow_x, body as f32, arrow_half as f32, arrow_height as f32));
            let mask = (p[2] as f32 / 255.0, p[1] as f32 / 255.0, p[0] as f32 / 255.0); // GDI pixels are BGR
            let mut c = bg;
            let r = bar_height as f32 / 2.0;
            c = mix(c, track, rounded(x, y, bar, r));
            c = mix(c, card.accent, rounded(x, y, (bar.0, bar.1, fill_right.max(bar.0 + bar_height as f32), bar.3), r));
            c = mix(c, primary, mask.0);
            c = mix(c, secondary, mask.1);
            c = mix(c, percent_color, mask.2);
            let a = shape;
            *p = [(c.2 as f32 * a) as u8, (c.1 as f32 * a) as u8, (c.0 as f32 * a) as u8, (a * 255.0) as u8]; // premultiplied
        }

        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8, ..Default::default() };
        let result = UpdateLayeredWindow(
            hwnd,
            None,
            Some(&POINT { x: left, y: top }),
            Some(&SIZE { cx: w, cy: h }),
            Some(dc),
            Some(&POINT::default()),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
        SelectObject(dc, old);
        for f in [glyph_font, name_font, sub_font, pct_font] {
            let _ = DeleteObject(f.into());
        }
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        result?;
        if !is_open(hwnd) {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd); // focus, so clicking elsewhere closes it
        }
    }
    Ok(())
}

unsafe fn text(dc: HDC, font: HFONT, text: &[u16], mut rect: RECT, color: u32, align: DRAW_TEXT_FORMAT) {
    unsafe {
        let old = SelectObject(dc, font.into());
        SetTextColor(dc, COLORREF(color));
        let mut buffer = text.to_vec();
        DrawTextW(dc, &mut buffer, &mut rect, align | DT_VCENTER | DT_SINGLELINE);
        SelectObject(dc, old);
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)) as u8;
    Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// Antialiased coverage (0..1) of a rounded rectangle at pixel center (x, y).
fn rounded(x: f32, y: f32, (l, t, r, b): (f32, f32, f32, f32), radius: f32) -> f32 {
    let cx = x.clamp(l + radius, r - radius);
    let cy = y.clamp(t + radius, b - radius);
    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
    (radius - d + 0.5).clamp(0.0, 1.0)
}

/// Antialiased coverage of the downward arrow: base on y = `top`, tip at (`tip_x`, `top + height`).
fn arrow(x: f32, y: f32, tip_x: f32, top: f32, half: f32, height: f32) -> f32 {
    if y < top - 1.0 || y > top + height + 1.0 {
        return 0.0;
    }
    // distance inside each slanted edge; the arrow overlaps the card above its base
    let slope = (half * half + height * height).sqrt();
    let inside = |dx: f32| ((top + height - y) * half - dx * height) / slope;
    (inside(x - tip_x).min(inside(tip_x - x)) + 0.5).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_corners_and_inside() {
        let r = (0.0, 0.0, 100.0, 50.0);
        assert_eq!(rounded(50.0, 25.0, r, 8.0), 1.0);
        assert_eq!(rounded(0.5, 0.5, r, 8.0), 0.0); // corner cut off
        assert_eq!(rounded(150.0, 25.0, r, 8.0), 0.0);
    }

    #[test]
    fn arrow_tip_and_sides() {
        assert!(arrow(50.0, 52.0, 50.0, 50.0, 9.0, 9.0) > 0.9); // just below the base, centered
        assert_eq!(arrow(70.0, 52.0, 50.0, 50.0, 9.0, 9.0), 0.0); // beside the arrow
    }
}
