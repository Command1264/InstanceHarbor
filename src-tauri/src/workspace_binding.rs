use instanceharbor::desktop_channel::{self, create_plain_directory};
use std::{env, ffi::OsStr, fs, io, os::windows::fs::MetadataExt, path::{Path, PathBuf}};

const REPARSE_POINT: u32 = 0x400;

pub(super) fn development_workspace() -> Result<PathBuf, String> {
    let workspace = PathBuf::from(env::var_os("IH_WORKSPACE")
        .filter(|path| !path.is_empty())
        .ok_or("請以開發版啟動腳本明確指定已初始化的 IH_WORKSPACE。")?);
    desktop_channel::validate_workspace(&workspace, false).map_err(|error| {
        format!("請指定 %LOCALAPPDATA%/InstanceHarbor Dev/workspaces 下已初始化的工作區。\n{error}")
    })?;
    if !workspace.join("workspace.json").is_file() {
        return Err("開發工作區尚未初始化，請先使用開發版 CLI 執行 init。".into());
    }
    Ok(workspace)
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn SetCurrentProcessExplicitAppUserModelID(identifier: *const u16) -> i32;
}

/// The WebView2 profile folder this process should use, or None to keep the one a
/// launcher already chose. Both channels keep it beside their workspaces, so the
/// app's data stays in one folder instead of Tauri's identifier folder.
pub(super) fn webview_profile(local_app_data: &Path, inherited: Option<&OsStr>) -> Option<PathBuf> {
    // The development channel always isolates its profile; a stable launcher such as
    // launch-acceptance.ps1 may pick a per-workspace profile.
    if !desktop_channel::DEVELOPMENT && inherited.is_some_and(|folder| !folder.is_empty()) {
        return None;
    }
    Some(local_app_data.join(desktop_channel::APP_DIRECTORY).join("webview"))
}

/// The WebView2 profile folder chosen by `configure_desktop` or the launcher. Tauri on
/// Windows otherwise creates `%LOCALAPPDATA%\<identifier>` for every window, even
/// when `WEBVIEW2_USER_DATA_FOLDER` points elsewhere, so each window passes this.
pub(super) fn webview_data_directory() -> Option<PathBuf> {
    env::var_os("WEBVIEW2_USER_DATA_FOLDER").filter(|folder| !folder.is_empty()).map(PathBuf::from)
}

pub(super) fn configure_desktop() -> Result<(), String> {
    let local = desktop_channel::local_app_data()?;
    if let Some(profile) = webview_profile(&local, env::var_os("WEBVIEW2_USER_DATA_FOLDER").as_deref()) {
        create_plain_directory(&local.join(desktop_channel::APP_DIRECTORY))?;
        create_plain_directory(&profile)?;
        // Configure only this process and its children before WebView2 starts.
        env::set_var("WEBVIEW2_USER_DATA_FOLDER", profile);
    }
    if desktop_channel::DEVELOPMENT {
        let id: Vec<_> = desktop_channel::IDENTIFIER.encode_utf16().chain([0]).collect();
        let status = unsafe { SetCurrentProcessExplicitAppUserModelID(id.as_ptr()) };
        if status < 0 {
            return Err(format!("DevelopmentAppUserModelIdFailed: {status:#x}"));
        }
    }
    Ok(())
}

pub(super) fn default_workspace() -> Result<PathBuf, String> {
    let appdata = PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("找不到 LOCALAPPDATA。")?);
    if !appdata.is_absolute() || !appdata.is_dir() {
        return Err("LOCALAPPDATA 不是有效的本機路徑。".into());
    }
    let app = appdata.join("InstanceHarbor");
    create_plain_directory(&app)?;
    let workspaces = app.join("workspaces");
    create_plain_directory(&workspaces)?;
    let workspace = workspaces.join("default");
    match fs::symlink_metadata(&workspace) {
        Ok(metadata) if metadata.is_dir() && metadata.file_attributes() & REPARSE_POINT == 0 => {}
        Ok(_) => return Err("預設工作區是連結或不是有效目錄。".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let path = workspace.to_str().ok_or("無法辨識預設工作區路徑。")?;
            instanceharbor::dispatch(&[path.into(), "init".into()])?;
        }
        Err(error) => return Err(error.to_string()),
    }
    if !workspace.join("workspace.json").is_file() {
        return Err(format!("工作區未初始化，請保留原有檔案並檢查 {}。", workspace.display()));
    }
    Ok(workspace)
}

#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(
        owner: *mut std::ffi::c_void,
        text: *const u16,
        caption: *const u16,
        style: u32,
    ) -> i32;
}

pub(super) fn show_startup_error(error: &str) {
    let text: Vec<_> = format!("無法開啟 InstanceHarbor 工作區。\n{error}")
        .encode_utf16().chain([0]).collect();
    let title: Vec<_> = desktop_channel::TITLE.encode_utf16().chain([0]).collect();
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stable_webview_profile_sits_beside_the_workspaces_unless_a_launcher_chose_one() {
        let local = Path::new(r"C:\Users\someone\AppData\Local");
        let profile = Some(local.join("InstanceHarbor").join("webview"));
        assert_eq!(webview_profile(local, None), profile);
        assert_eq!(webview_profile(local, Some(OsStr::new(""))), profile);
        assert_eq!(webview_profile(local, Some(OsStr::new(r"D:\evidence\manual-webview"))), None);
    }
}
