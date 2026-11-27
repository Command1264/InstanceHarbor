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
pub(super) fn webview_profile(
    local_app_data: &Path,
    inherited: Option<&OsStr>,
    workspace: Option<&str>,
) -> Option<PathBuf> {
    webview_profile_for(desktop_channel::DEVELOPMENT, local_app_data, inherited, workspace)
}

fn webview_profile_for(
    development: bool,
    local_app_data: &Path,
    inherited: Option<&OsStr>,
    workspace: Option<&str>,
) -> Option<PathBuf> {
    // The development channel always isolates its profile, one per workspace; a stable
    // launcher such as scripts/launch-acceptance.ps1 may pick a per-workspace profile.
    if !development && inherited.is_some_and(|folder| !folder.is_empty()) {
        return None;
    }
    Some(desktop_channel::webview_profile_for(development, local_app_data, workspace))
}

/// The WebView2 profile folder chosen by `configure_desktop` or the launcher. Tauri on
/// Windows otherwise creates `%LOCALAPPDATA%\<identifier>` for every window, even
/// when `WEBVIEW2_USER_DATA_FOLDER` points elsewhere, so each window passes this.
pub(super) fn webview_data_directory() -> Option<PathBuf> {
    env::var_os("WEBVIEW2_USER_DATA_FOLDER").filter(|folder| !folder.is_empty()).map(PathBuf::from)
}

pub(super) fn configure_desktop() -> Result<(), String> {
    let local = desktop_channel::local_app_data()?;
    let workspace = desktop_channel::workspace_name();
    let inherited = env::var_os("WEBVIEW2_USER_DATA_FOLDER");
    if let Some(profile) = webview_profile(&local, inherited.as_deref(), workspace.as_deref()) {
        // Create each missing level; all of them stay under the app folder.
        let app = local.join(desktop_channel::APP_DIRECTORY);
        create_plain_directory(&app)?;
        let mut folder = app;
        for part in profile.strip_prefix(&folder).map_err(|error| error.to_string())?.components() {
            folder.push(part);
            create_plain_directory(&folder)?;
        }
        // Configure only this process and its children before WebView2 starts.
        env::set_var("WEBVIEW2_USER_DATA_FOLDER", profile);
    }
    if desktop_channel::DEVELOPMENT {
        // Each workspace gets its own taskbar button.
        let id: Vec<_> = desktop_channel::app_user_model_id(workspace.as_deref()).encode_utf16().chain([0]).collect();
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
    let title = desktop_channel::product_title(desktop_channel::workspace_name().as_deref());
    let title: Vec<_> = title.encode_utf16().chain([0]).collect();
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10); }
}

/// `System.AppUserModel.*` property ids (format id 9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3).
const RELAUNCH_COMMAND: u32 = 2;
const RELAUNCH_DISPLAY_NAME: u32 = 4;
const APP_USER_MODEL_ID: u32 = 5;

/// What the main window tells the taskbar, in the order it is set. The Jump List names the
/// app by its relaunch display name, so each development workspace shows its own name there.
/// The stable channel keeps the installed shortcut's name and sets nothing.
fn taskbar_properties(development: bool, workspace: Option<&str>, executable: &Path) -> Vec<(u32, String)> {
    if !development {
        return Vec::new();
    }
    vec![
        (RELAUNCH_COMMAND, format!("\"{}\"", executable.display())),
        (RELAUNCH_DISPLAY_NAME, desktop_channel::product_title_for(true, workspace)),
        (APP_USER_MODEL_ID, desktop_channel::app_user_model_id_for(true, workspace)),
    ]
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
#[repr(C)]
struct PropertyKey { format: Guid, id: u32 }
#[repr(C)]
struct PropVariant { kind: u16, reserved: [u16; 3], text: *const u16, rest: usize }
#[repr(C)]
struct PropertyStore { table: *const PropertyStoreTable }
#[repr(C)]
struct PropertyStoreTable {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut PropertyStore) -> u32,
    get_count: usize,
    get_at: usize,
    get_value: usize,
    set_value: unsafe extern "system" fn(*mut PropertyStore, *const PropertyKey, *const PropVariant) -> i32,
    commit: usize,
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetPropertyStoreForWindow(window: *mut std::ffi::c_void, interface: *const Guid, store: *mut *mut PropertyStore) -> i32;
}

/// Names the development main window in the taskbar Jump List. These are properties of the
/// window only: nothing is written to the registry or to a shortcut.
pub(super) fn name_taskbar_window(window: *mut std::ffi::c_void) -> Result<(), String> {
    let executable = env::current_exe().map_err(|error| error.to_string())?;
    let workspace = desktop_channel::workspace_name();
    let properties = taskbar_properties(desktop_channel::DEVELOPMENT, workspace.as_deref(), &executable);
    if properties.is_empty() {
        return Ok(());
    }
    const PROPERTY_STORE: Guid = Guid(0x886d8eeb, 0x8cf2, 0x4446, [0x8d, 0x02, 0xcd, 0xba, 0x1d, 0xbd, 0xcf, 0x99]);
    const VT_LPWSTR: u16 = 31;
    let mut store = std::ptr::null_mut();
    let status = unsafe { SHGetPropertyStoreForWindow(window, &PROPERTY_STORE, &mut store) };
    if status < 0 || store.is_null() {
        return Err(format!("TaskbarPropertyStoreUnavailable: {status:#x}"));
    }
    let mut result = Ok(());
    for (id, value) in properties {
        let text: Vec<u16> = value.encode_utf16().chain([0]).collect();
        let key = PropertyKey { format: Guid(0x9F4C2855, 0x9F79, 0x4B39, [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3]), id };
        let variant = PropVariant { kind: VT_LPWSTR, reserved: [0; 3], text: text.as_ptr(), rest: 0 };
        // SetValue copies the string.
        let status = unsafe { ((*(*store).table).set_value)(store, &key, &variant) };
        if status < 0 {
            result = Err(format!("TaskbarPropertyRejected {id}: {status:#x}"));
            break;
        }
    }
    unsafe { ((*(*store).table).release)(store) };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stable_webview_profile_sits_beside_the_workspaces_unless_a_launcher_chose_one() {
        let local = Path::new(r"C:\Users\someone\AppData\Local");
        let profile = Some(local.join("InstanceHarbor").join("webview"));
        assert_eq!(webview_profile_for(false, local, None, None), profile);
        assert_eq!(webview_profile_for(false, local, Some(OsStr::new("")), Some("agent-a")), profile);
        assert_eq!(webview_profile_for(false, local, Some(OsStr::new(r"D:\evidence\manual-webview")), None), None);
    }

    #[test]
    fn the_development_webview_profile_ignores_a_launcher_folder() {
        let local = Path::new(r"C:\Users\someone\AppData\Local");
        let profile = Some(local.join("InstanceHarbor Dev").join("webviews").join("agent-a"));
        let inherited = Some(OsStr::new(r"D:\evidence\manual-webview"));
        assert_eq!(webview_profile_for(true, local, inherited, Some("agent-a")), profile);
    }

    #[test]
    fn each_development_window_names_its_workspace_in_the_jump_list() {
        let executable = Path::new(r"D:\builds\InstanceHarborDev.exe");
        assert!(taskbar_properties(false, Some("agent-a"), executable).is_empty(), "stable keeps its shortcut name");
        let named = taskbar_properties(true, Some("agent-a"), executable);
        assert_eq!(named[0], (RELAUNCH_COMMAND, r#""D:\builds\InstanceHarborDev.exe""#.to_owned()));
        assert_eq!(named[1], (RELAUNCH_DISPLAY_NAME, desktop_channel::product_title_for(true, Some("agent-a"))));
        assert_eq!(named[2], (APP_USER_MODEL_ID, desktop_channel::app_user_model_id_for(true, Some("agent-a"))));
        assert!(named[1].1.ends_with("agent-a"));
    }
}
