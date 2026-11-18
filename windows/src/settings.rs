//! Settings in the current user's registry: Launch at login, taskbar theme.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::core::{PCWSTR, Result, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const THEME_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");

pub fn launch_at_login() -> bool {
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Pips"), RRF_RT_REG_SZ, None, None, None) };
    status == ERROR_SUCCESS
}

pub fn set_launch_at_login(on: bool) -> Result<()> {
    if !on {
        let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Pips")) };
        return if status == ERROR_FILE_NOT_FOUND { Ok(()) } else { status.ok() };
    }
    let exe = std::env::current_exe().map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
    let command: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain([0]).collect();
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            w!("Pips"),
            REG_SZ.0,
            Some(command.as_ptr().cast()),
            (command.len() * 2) as u32,
        )
    }
    .ok()
}

/// True when the taskbar uses the light theme, so icons must be dark.
pub fn light_taskbar() -> bool {
    dword(THEME_KEY, w!("SystemUsesLightTheme")) == Some(1)
}

fn dword(key: PCWSTR, name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = 4u32;
    let status: WIN32_ERROR = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}
