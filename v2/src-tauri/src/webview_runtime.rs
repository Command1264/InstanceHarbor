//! Check for the WebView2 Runtime before any window, tray icon or single-instance
//! lock exists. Without it Tauri shows its own dialog and then keeps the process
//! alive with no window, which holds the GUI lock and silently blocks every later
//! launch. This check runs on every launch, so a later removal is reported too.
use instanceharbor_v2::desktop_channel;

/// Microsoft's permanent link to the Evergreen Standalone Installer for x64.
const STANDALONE_X64: &str = "https://go.microsoft.com/fwlink/?linkid=2124701";
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

/// The GUI's language setting lives in the workspace, which is not bound yet.
/// Like the GUI's resolveLanguage, only an English system gets English.
fn chinese() -> bool {
    (unsafe { GetUserDefaultUILanguage() } & 0x3ff) != 0x09
}

pub(super) fn message(chinese: bool) -> String {
    if chinese {
        format!(
            "找不到 Microsoft Edge WebView2 Runtime，InstanceHarbor 需要它才能顯示視窗。\n\n\
             請安裝 Microsoft 官方的「Evergreen 獨立安裝程式」x64 版（Evergreen Standalone Installer），\
             安裝完成後再開啟 InstanceHarbor。\n\n\
             選「是」會從 Microsoft 下載這個安裝程式（約 280 MB）。\n\
             下載頁面：{DOWNLOAD_PAGE}"
        )
    } else {
        format!(
            "Microsoft Edge WebView2 Runtime was not found. InstanceHarbor needs it to show its window.\n\n\
             Install Microsoft's Evergreen Standalone Installer for x64, then open InstanceHarbor again.\n\n\
             Choose Yes to download that installer from Microsoft (about 280 MB).\n\
             Download page: {DOWNLOAD_PAGE}"
        )
    }
}

/// Returns false after telling the user what to install.
pub(super) fn ensure_available() -> bool {
    if tauri::webview_version().is_ok() {
        return true;
    }
    let text = wide(&message(chinese()));
    let title = wide(desktop_channel::TITLE);
    // MB_YESNO | MB_ICONERROR | MB_DEFBUTTON1; IDYES = 6.
    if unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x4 | 0x10) } == 6 {
        let (open, link) = (wide("open"), wide(STANDALONE_X64));
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
        assert!(message(true).contains("Evergreen 獨立安裝程式」x64"));
        assert!(message(false).contains("Evergreen Standalone Installer for x64"));
        for chinese in [true, false] {
            assert!(message(chinese).contains(DOWNLOAD_PAGE));
        }
    }
}
