//! The taskbar strip: a small always-on-top window just left of the tray area, with one
//! "glyph 85%" chip per device. Windows has no API for taskbar widgets; like other such
//! tools, the strip floats above the taskbar and follows it.

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    DIB_RGB_COLORS, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FW_NORMAL, FW_SEMIBOLD,
    GdiFlush, GetTextExtentPoint32W, HDC, HFONT, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowRect, ULW_ALPHA, UpdateLayeredWindow};
use windows::core::{PCWSTR, Result, w};

#[derive(Clone, Copy)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub struct Chip {
    pub glyph: char,
    /// e.g. "85%"; empty for a glyph-only chip
    pub text: String,
    pub color: Rgb,
}

/// The taskbar window and its tray area (clock and system icons), which the strip sits next to.
pub struct Taskbar {
    pub window: HWND,
    pub tray_area: HWND,
}

pub fn taskbar() -> Result<Taskbar> {
    unsafe {
        let window = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null())?;
        let tray_area = FindWindowExW(Some(window), None, w!("TrayNotifyWnd"), PCWSTR::null())?;
        Ok(Taskbar { window, tray_area })
    }
}

/// Draws `chips` into the layered window `hwnd` and moves it left of the tray area.
/// Returns each chip's horizontal range in window coordinates, for hit-testing clicks.
pub fn update(hwnd: HWND, chips: &[Chip]) -> Result<Vec<(i32, i32)>> {
    let bar = taskbar()?;
    let (mut bar_rect, mut tray_rect) = (RECT::default(), RECT::default());
    unsafe {
        GetWindowRect(bar.window, &mut bar_rect)?;
        GetWindowRect(bar.tray_area, &mut tray_rect)?;
    }
    let scale = unsafe { GetDpiForWindow(hwnd) } as i32;
    let px = |v: i32| v * scale / 96;
    let (pad, gap, spacing, margin) = (px(8), px(4), px(12), px(4));
    let height = bar_rect.bottom - bar_rect.top;

    unsafe {
        let dc = CreateCompatibleDC(None);
        let glyph_font = font(px(16), FW_NORMAL.0 as i32, w!("Segoe Fluent Icons"));
        let text_font = font(px(13), FW_SEMIBOLD.0 as i32, w!("Segoe UI"));

        // layout: x range of each chip's glyph and text
        let mut parts = Vec::new();
        let mut x = pad;
        for chip in chips {
            let glyph: Vec<u16> = chip.glyph.to_string().encode_utf16().collect();
            let text: Vec<u16> = chip.text.encode_utf16().collect();
            let gw = width(dc, glyph_font, &glyph);
            let tw = if text.is_empty() { 0 } else { width(dc, text_font, &text) };
            let chip_width = gw + if tw > 0 { gap + tw } else { 0 };
            parts.push((x, gw, glyph, text, chip_width));
            x += chip_width + spacing;
        }
        let strip_width = (x - spacing + pad).max(pad * 2);

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: strip_width,
                biHeight: -height, // top-down
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

        // white on black: brightness becomes alpha below, and each chip gets its own color
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        for (x, gw, glyph, text, _) in &parts {
            draw(dc, glyph_font, glyph, *x, *x + gw, height);
            if !text.is_empty() {
                draw(dc, text_font, text, x + gw + gap, strip_width, height);
            }
        }
        let _ = GdiFlush();

        let pixels = std::slice::from_raw_parts_mut(bits as *mut [u8; 4], (strip_width * height) as usize);
        for (i, p) in pixels.iter_mut().enumerate() {
            let px_x = i as i32 % strip_width;
            let color = parts
                .iter()
                .zip(chips)
                .find(|((x, _, _, _, w), _)| px_x >= *x && px_x < x + w)
                .map_or(Rgb(0, 0, 0), |(_, c)| c.color);
            // alpha 1 keeps the whole strip clickable; fully transparent pixels pass clicks through
            let a = p[0].max(p[1]).max(p[2]).max(1) as u32;
            let pre = |c: u8| (c as u32 * a / 255) as u8; // UpdateLayeredWindow wants premultiplied alpha
            *p = [pre(color.2), pre(color.1), pre(color.0), a as u8];
        }

        let position = POINT { x: tray_rect.left - strip_width - margin, y: bar_rect.top };
        let size = SIZE { cx: strip_width, cy: height };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
            ..Default::default()
        };
        let result = UpdateLayeredWindow(hwnd, None, Some(&position), Some(&size), Some(dc), Some(&POINT::default()), COLORREF(0), Some(&blend), ULW_ALPHA);

        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteObject(glyph_font.into());
        let _ = DeleteObject(text_font.into());
        let _ = DeleteDC(dc);
        result?;
        Ok(parts.iter().map(|(x, _, _, _, w)| (*x, x + w)).collect())
    }
}

pub(crate) unsafe fn width(dc: HDC, font: HFONT, text: &[u16]) -> i32 {
    unsafe {
        let old = SelectObject(dc, font.into());
        let mut extent = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, text, &mut extent);
        SelectObject(dc, old);
        extent.cx
    }
}

unsafe fn draw(dc: HDC, font: HFONT, text: &[u16], left: i32, right: i32, height: i32) {
    unsafe {
        let old = SelectObject(dc, font.into());
        let mut rect = RECT { left, top: 0, right, bottom: height };
        let mut buffer = text.to_vec();
        DrawTextW(dc, &mut buffer, &mut rect, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        SelectObject(dc, old);
    }
}

pub(crate) unsafe fn font(height: i32, weight: i32, face: PCWSTR) -> HFONT {
    unsafe {
        CreateFontW(-height, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, DEFAULT_PITCH.0 as u32, face)
    }
}
