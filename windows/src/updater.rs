//! Checks GitHub Releases for a newer Pips, shows what changed, and replaces pips.exe in place.

use std::path::Path;
use std::sync::{Arc, Mutex};

use windows::Data::Json::{JsonArray, JsonObject, JsonValueType};
use windows::Foundation::Uri;
use windows::Security::Cryptography::Core::{HashAlgorithmNames, HashAlgorithmProvider};
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Storage::Streams::{DataReader, IBuffer};
use windows::Web::Http::HttpClient;
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HINSTANCE, HWND, LPARAM, S_OK, WAIT_TIMEOUT, WPARAM};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOG_NOTIFICATIONS, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TDCBF_CANCEL_BUTTON,
    TDF_ALLOW_DIALOG_CANCELLATION, TDF_EXPANDED_BY_DEFAULT, TDF_SHOW_MARQUEE_PROGRESS_BAR,
    TDF_VERIFICATION_FLAG_CHECKED, TDM_CLICK_BUTTON, TDM_SET_PROGRESS_BAR_MARQUEE, TDN_CREATED, TaskDialogIndirect,
};
use windows::Win32::UI::WindowsAndMessaging::{IDCANCEL, PostMessageW, SW_HIDE, SendMessageW};
use windows::core::{BOOL, HRESULT, HSTRING, PCWSTR, Result, w};

use crate::about::VERSION;

const RELEASES: &str = "https://api.github.com/repos/a-mayur/pips/releases?per_page=20";
const ASSET: &str = "pips.exe";
const SUMS: &str = "SHA256SUMS.txt";

#[derive(Clone, Debug)]
pub struct Update {
    pub version: String,
    /// release notes of every version newer than this one, newest first
    pub notes: String,
    exe_url: String,
    sums_url: String,
}

/// The newest published release above `VERSION`, if any. Blocks on the network: call off the UI thread.
pub fn check() -> Result<Option<Update>> {
    let releases = JsonArray::Parse(&http()?.GetStringAsync(&Uri::CreateUri(&HSTRING::from(RELEASES))?)?.join()?)?;
    let mut newer = Vec::new();
    for value in &releases {
        let r = value.GetObject()?;
        let version = string(&r, "tag_name")?.trim_start_matches('v').to_string();
        if r.GetNamedBoolean(&"draft".into())? || r.GetNamedBoolean(&"prerelease".into())? || !is_newer(&version, VERSION) {
            continue;
        }
        let mut assets = (String::new(), String::new());
        for a in &r.GetNamedArray(&"assets".into())? {
            let a = a.GetObject()?;
            match string(&a, "name")?.as_str() {
                ASSET => assets.0 = string(&a, "browser_download_url")?,
                SUMS => assets.1 = string(&a, "browser_download_url")?,
                _ => {}
            }
        }
        newer.push((version, string(&r, "body")?, assets));
    }
    newer.sort_by(|a, b| if is_newer(&a.0, &b.0) { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater });
    let Some((version, _, (exe_url, sums_url))) = newer.first().cloned() else { return Ok(None) };
    if exe_url.is_empty() || sums_url.is_empty() {
        return Err(windows::core::Error::new(HRESULT(-1), format!("Release {version} has no {ASSET} or {SUMS}")));
    }
    let notes = newer.iter().map(|(v, body, _)| format!("What's new in {v}\n{}\n", body.trim())).collect::<Vec<_>>().join("\n");
    Ok(Some(Update { version, notes, exe_url, sums_url }))
}

/// "1.0.10" is newer than "1.0.9"; missing parts count as 0.
fn is_newer(a: &str, b: &str) -> bool {
    let parts = |s: &str| s.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (x, y) = (parts(a), parts(b));
    for i in 0..x.len().max(y.len()) {
        let (p, q) = (x.get(i).copied().unwrap_or(0), y.get(i).copied().unwrap_or(0));
        if p != q {
            return p > q;
        }
    }
    false
}

/// Null or missing values read as "".
fn string(o: &JsonObject, key: &str) -> Result<String> {
    let v = o.GetNamedValue(&HSTRING::from(key))?;
    Ok(if v.ValueType()? == JsonValueType::String { v.GetString()?.to_string() } else { String::new() })
}

fn http() -> Result<HttpClient> {
    let client = HttpClient::new()?;
    client.DefaultRequestHeaders()?.UserAgent()?.TryParseAdd(&HSTRING::from(format!("Pips/{VERSION}")))?; // GitHub rejects requests without one
    Ok(client)
}

/// Downloads the new pips.exe and checks it against the release's SHA256SUMS.txt.
fn download_verified(u: &Update) -> Result<Vec<u8>> {
    let client = http()?;
    let exe = client.GetBufferAsync(&Uri::CreateUri(&HSTRING::from(&u.exe_url))?)?.join()?;
    let sums = client.GetStringAsync(&Uri::CreateUri(&HSTRING::from(&u.sums_url))?)?.join()?.to_string();
    let hash = HashAlgorithmProvider::OpenAlgorithm(&HashAlgorithmNames::Sha256()?)?.HashData(&exe)?;
    let actual = CryptographicBuffer::EncodeToHexString(&hash)?.to_string();
    if expected_hash(&sums, ASSET).map(|h| h.eq_ignore_ascii_case(&actual)) != Some(true) {
        return Err(windows::core::Error::new(HRESULT(-1), "The download does not match its checksum; nothing was changed."));
    }
    bytes(&exe)
}

/// The hash for `name` in `sha256sum` output ("<hash>  <name>" per line).
fn expected_hash<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let (hash, file) = (parts.next()?, parts.next()?);
        (file.trim_start_matches('*') == name).then_some(hash)
    })
}

fn bytes(buffer: &IBuffer) -> Result<Vec<u8>> {
    let mut out = vec![0; buffer.Length()? as usize];
    DataReader::FromBuffer(buffer)?.ReadBytes(&mut out)?;
    Ok(out)
}

/// Swaps in the new exe in place of the running one and starts it. The caller then quits.
/// Where this user can't write (C:\\, Program Files), the swap runs elevated after a UAC prompt.
pub fn install(new_exe: &[u8]) -> std::result::Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let folder = exe.parent().ok_or("Pips cannot find its own folder.")?;
    if writable(folder) {
        let new = exe.with_extension("new.exe");
        std::fs::write(&new, new_exe).map_err(|e| e.to_string())?;
        swap(&new, &exe).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&new);
    } else {
        install_elevated(new_exe, &exe)?;
    }
    std::process::Command::new(&exe).arg("--after-update").spawn().map_err(|e| e.to_string())?;
    Ok(())
}

fn writable(folder: &Path) -> bool {
    let probe = folder.join(".pips-update-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Replaces `target` (possibly running) with a copy of `new`; on failure `target` is left as it was.
fn swap(new: &Path, target: &Path) -> std::io::Result<()> {
    let old = target.with_extension("old.exe");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(target, &old)?; // Windows allows renaming a running exe, not overwriting it
    if let Err(e) = std::fs::copy(new, target) {
        let _ = std::fs::rename(&old, target);
        return Err(e);
    }
    Ok(())
}

/// Saves the new exe to %TEMP% and runs it elevated with `--replace <target>`; waits for it.
fn install_elevated(new_exe: &[u8], target: &Path) -> std::result::Result<(), String> {
    let staged = std::env::temp_dir().join("pips-update.exe");
    std::fs::write(&staged, new_exe).map_err(|e| e.to_string())?;
    let file = HSTRING::from(staged.as_os_str());
    let parameters = HSTRING::from(format!("--replace \"{}\"", target.display()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"), // UAC prompt
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    let result = unsafe { ShellExecuteExW(&mut info) };
    let outcome = match result {
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => Err("Update cancelled: administrator permission was not given.".to_string()),
        Err(e) => Err(format!("Could not start the update with administrator permission: {}", e.message())),
        Ok(()) => unsafe {
            let finished = WaitForSingleObject(info.hProcess, 60_000) != WAIT_TIMEOUT;
            let mut code = 1;
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
            if finished && code == 0 { Ok(()) } else { Err(format!("Replacing {} failed.", target.display())) }
        },
    };
    let _ = std::fs::remove_file(&staged);
    outcome
}

/// `pips.exe --replace <target>`, run elevated by `install_elevated` from the staged new exe.
pub fn replace_from_elevated(target: &Path) -> std::io::Result<()> {
    swap(&std::env::current_exe()?, target)
}

/// Removes the previous exe left behind by an update.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old.exe"));
    }
}

pub enum Choice {
    Install,
    Later,
    Skip,
}

const ID_INSTALL: i32 = 100;
const ID_LATER: i32 = 101;
const ID_SKIP: i32 = 102;

/// The update dialog. Returns the choice and whether "Check for updates automatically" is ticked.
pub fn offer(owner: HWND, instance: HINSTANCE, u: &Update, auto: bool) -> Result<(Choice, bool)> {
    let content = HSTRING::from(format!("Pips {} is available. You have {VERSION}.", u.version));
    let notes = HSTRING::from(&u.notes);
    let buttons = [
        TASKDIALOG_BUTTON { nButtonID: ID_INSTALL, pszButtonText: w!("Install update") },
        TASKDIALOG_BUTTON { nButtonID: ID_LATER, pszButtonText: w!("Remind me later") },
        TASKDIALOG_BUTTON { nButtonID: ID_SKIP, pszButtonText: w!("Skip this version") },
    ];
    let mut flags = TDF_EXPANDED_BY_DEFAULT | TDF_ALLOW_DIALOG_CANCELLATION;
    if auto {
        flags |= TDF_VERIFICATION_FLAG_CHECKED;
    }
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        hInstance: instance,
        dwFlags: flags,
        pszWindowTitle: w!("Pips update"),
        Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: PCWSTR(1 as _) }, // the app icon embedded by build.rs
        pszMainInstruction: w!("A new version of Pips is available"),
        pszContent: PCWSTR(content.as_ptr()),
        pszExpandedInformation: PCWSTR(notes.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: ID_INSTALL,
        pszVerificationText: w!("Check for updates automatically"),
        ..Default::default()
    };
    let (mut button, mut checked) = (0, BOOL(0));
    unsafe { TaskDialogIndirect(&config, Some(&mut button), None, Some(&mut checked))? };
    let choice = match button {
        ID_INSTALL => Choice::Install,
        ID_SKIP => Choice::Skip,
        _ => Choice::Later,
    };
    Ok((choice, checked.as_bool()))
}

/// Download result, filled by the worker thread; None while running or when cancelled.
type Slot = Arc<Mutex<Option<std::result::Result<Vec<u8>, String>>>>;

struct Job {
    update: Update,
    slot: Slot,
}

/// A progress dialog while `download_verified` runs on a worker thread. None if the user cancelled.
pub fn download_with_progress(owner: HWND, instance: HINSTANCE, u: &Update) -> Option<std::result::Result<Vec<u8>, String>> {
    let job = Job { update: u.clone(), slot: Arc::new(Mutex::new(None)) };
    let content = HSTRING::from(format!("Downloading Pips {}…", u.version));
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        hInstance: instance,
        dwFlags: TDF_SHOW_MARQUEE_PROGRESS_BAR,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: w!("Pips update"),
        Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: PCWSTR(1 as _) },
        pszMainInstruction: w!("Updating Pips…"),
        pszContent: PCWSTR(content.as_ptr()),
        pfCallback: Some(on_progress_event),
        lpCallbackData: &job as *const Job as isize, // job outlives the modal dialog
        ..Default::default()
    };
    if let Err(e) = unsafe { TaskDialogIndirect(&config, None, None, None) } {
        return Some(Err(e.message()));
    }
    job.slot.lock().ok()?.take()
}

unsafe extern "system" fn on_progress_event(hwnd: HWND, msg: TASKDIALOG_NOTIFICATIONS, _: WPARAM, _: LPARAM, data: isize) -> HRESULT {
    if msg == TDN_CREATED {
        let job = unsafe { &*(data as *const Job) };
        let (update, slot, dialog) = (job.update.clone(), job.slot.clone(), hwnd.0 as usize);
        unsafe { SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_MARQUEE.0 as u32, Some(WPARAM(1)), Some(LPARAM(0))) };
        std::thread::spawn(move || {
            let result = download_verified(&update).map_err(|e| e.message());
            if let Ok(mut s) = slot.lock() {
                *s = Some(result);
            }
            // closes the dialog; the filled slot tells it apart from the user pressing Cancel
            let _ = unsafe { PostMessageW(Some(HWND(dialog as _)), TDM_CLICK_BUTTON.0 as u32, WPARAM(IDCANCEL.0 as usize), LPARAM(0)) };
        });
    }
    S_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("1.0.10", "1.0.9"));
        assert!(is_newer("1.1", "1.0.9"));
        assert!(!is_newer("1.0.1", "1.0.1"));
        assert!(!is_newer("1.0.0", "1.0.1"));
    }

    #[test]
    fn finds_hash_in_sha256sum_output() {
        let sums = "aaa  Pips.dmg\nbbb  Pips.zip\nccc  pips.exe\n";
        assert_eq!(expected_hash(sums, "pips.exe"), Some("ccc"));
        assert_eq!(expected_hash(sums, "missing"), None);
    }
}
