//! Check for the WebView2 Runtime before any window, tray icon or single-instance
//! lock exists. Without it Tauri shows its own dialog and then keeps the process
//! alive with no window, which holds the GUI lock and silently blocks every later
//! launch. This check runs on every launch, so a later removal is reported too.
use instanceharbor::desktop_channel;
use std::{env, fs, os::windows::fs::MetadataExt, path::PathBuf};

/// Microsoft's permanent links to the Evergreen Standalone Installer, from the download page.
/// Each build names the installer of its own architecture (#66).
#[cfg(not(target_arch = "aarch64"))]
const STANDALONE: (&str, &str) = ("x64", "https://go.microsoft.com/fwlink/?linkid=2124701");
#[cfg(target_arch = "aarch64")]
const STANDALONE: (&str, &str) = ("ARM64", "https://go.microsoft.com/fwlink/?linkid=2099616");
const DOWNLOAD_PAGE: &str = "https://developer.microsoft.com/microsoft-edge/webview2/#download-section";

#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(owner: *mut std::ffi::c_void, text: *const u16, caption: *const u16, style: u32) -> i32;
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        window: *mut std::ffi::c_void,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// The workspace the GUI would bind, found without creating or initializing anything.
fn configured_workspace() -> Option<PathBuf> {
    let stable_default = !desktop_channel::DEVELOPMENT
        && env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--stable-default"));
    if !stable_default {
        if let Some(workspace) = env::var_os("IH_WORKSPACE").filter(|path| !path.is_empty()) {
            return Some(workspace.into());
        }
    }
    // The development channel always names its workspace.
    if desktop_channel::DEVELOPMENT {
        return None;
    }
    let appdata = PathBuf::from(env::var_os("LOCALAPPDATA")?);
    Some(appdata.join("InstanceHarbor").join("workspaces").join("default"))
}

/// The `language` saved in the workspace's preferences.json, read only. The GUI
/// cannot start at this point, so any problem simply means "no saved setting".
fn saved_language() -> Option<String> {
    let path = configured_workspace()?.join("preferences.json");
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 || metadata.len() > 1 << 20 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    value.get("language")?.as_str().map(str::to_owned)
}

/// InstanceHarbor's setting when it names a language; otherwise the Windows
/// display language: Traditional Chinese (Taiwan, Hong Kong, Macao) or English.
pub(super) fn chinese(saved: Option<&str>, ui_language: u16) -> bool {
    match saved {
        Some("zh-TW") => true,
        Some("en") => false,
        _ => matches!(ui_language, 0x0404 | 0x0C04 | 0x1404),
    }
}

pub(super) fn message(chinese: bool) -> String {
    if chinese {
        format!(
            "找不到 Microsoft Edge WebView2 Runtime，InstanceHarbor 需要它才能顯示視窗。\n\n\
             請安裝 Microsoft 官方的「Evergreen 獨立安裝程式」{} 版（Evergreen Standalone Installer），\
             安裝完成後再開啟 InstanceHarbor。\n\n\
             選「是」會從 Microsoft 下載這個安裝程式（約 280 MB）。\n\
             下載頁面：{DOWNLOAD_PAGE}",
            STANDALONE.0
        )
    } else {
        format!(
            "Microsoft Edge WebView2 Runtime was not found. InstanceHarbor needs it to show its window.\n\n\
             Install Microsoft's Evergreen Standalone Installer for {}, then open InstanceHarbor again.\n\n\
             Choose Yes to download that installer from Microsoft (about 280 MB).\n\
             Download page: {DOWNLOAD_PAGE}",
            STANDALONE.0
        )
    }
}

/// Returns false after telling the user what to install.
pub(super) fn ensure_available() -> bool {
    if tauri::webview_version().is_ok() {
        return true;
    }
    let ui_language = unsafe { GetUserDefaultUILanguage() };
    let text = wide(&message(chinese(saved_language().as_deref(), ui_language)));
    let title = wide(&desktop_channel::product_title(desktop_channel::workspace_name().as_deref()));
    // MB_YESNO | MB_ICONERROR | MB_DEFBUTTON1; IDYES = 6.
    if unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x4 | 0x10) } == 6 {
        let (open, link) = (wide("open"), wide(STANDALONE.1));
        let opened = unsafe {
            ShellExecuteW(std::ptr::null_mut(), open.as_ptr(), link.as_ptr(), std::ptr::null(), std::ptr::null(), 1)
        };
        if opened <= 32 {
            eprintln!("Could not open the WebView2 download: {opened}");
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_names_the_exact_installer_in_both_languages() {
        let label = if cfg!(target_arch = "aarch64") { "ARM64" } else { "x64" };
        assert_eq!(STANDALONE.0, label);
        assert!(message(true).contains(&format!("Evergreen 獨立安裝程式」{label} 版")));
        assert!(message(false).contains(&format!("Evergreen Standalone Installer for {label},")));
        for chinese in [true, false] {
            assert!(message(chinese).contains(DOWNLOAD_PAGE));
        }
    }

    #[test]
    fn a_saved_language_wins_and_otherwise_english_is_the_default() {
        // zh-TW, en-US, zh-CN and ja-JP Windows display languages.
        for ui in [0x0404, 0x0409, 0x0804, 0x0411] {
            assert!(chinese(Some("zh-TW"), ui));
            assert!(!chinese(Some("en"), ui));
        }
        for saved in [None, Some("system"), Some("unknown")] {
            assert!(chinese(saved, 0x0404));
            assert!(chinese(saved, 0x0C04));
            assert!(chinese(saved, 0x1404));
            assert!(!chinese(saved, 0x0409));
            assert!(!chinese(saved, 0x0804));
            assert!(!chinese(saved, 0x0411));
        }
    }
}
