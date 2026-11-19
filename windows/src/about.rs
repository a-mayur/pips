//! "About Pips" dialog: icon, name, version and a clickable GitHub link.

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, S_OK, WPARAM};
use windows::Win32::UI::Controls::{
    TASKDIALOG_NOTIFICATIONS, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TDCBF_OK_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_ENABLE_HYPERLINKS, TDN_HYPERLINK_CLICKED, TaskDialogIndirect,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HRESULT, HSTRING, PCWSTR, Result, w};

/// The release tag's version in CI builds (`PIPS_VERSION`), otherwise the one in Cargo.toml.
pub const VERSION: &str = match option_env!("PIPS_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
const URL: &str = "https://github.com/a-mayur/pips";

pub fn show(owner: HWND, instance: HINSTANCE) -> Result<()> {
    let content = HSTRING::from(format!(
        "Version {VERSION}\n<a href=\"{URL}\">github.com/a-mayur/pips</a>\n\nFree and open source · GPL-3.0"
    ));
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        hInstance: instance,
        dwFlags: TDF_ENABLE_HYPERLINKS | TDF_ALLOW_DIALOG_CANCELLATION,
        dwCommonButtons: TDCBF_OK_BUTTON,
        pszWindowTitle: w!("About Pips"),
        Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: PCWSTR(1 as _) }, // the app icon embedded by build.rs
        pszMainInstruction: w!("Pips"),
        pszContent: PCWSTR(content.as_ptr()),
        pfCallback: Some(on_event),
        ..Default::default()
    };
    unsafe { TaskDialogIndirect(&config, None, None, None) }
}

unsafe extern "system" fn on_event(_: HWND, msg: TASKDIALOG_NOTIFICATIONS, _: WPARAM, lparam: LPARAM, _: isize) -> HRESULT {
    if msg == TDN_HYPERLINK_CLICKED {
        // lparam is the clicked link's href
        unsafe { ShellExecuteW(None, w!("open"), PCWSTR(lparam.0 as _), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    }
    S_OK
}
