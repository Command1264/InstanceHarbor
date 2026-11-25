use std::{
    env, fs, io,
    os::windows::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

pub const DEVELOPMENT: bool = cfg!(feature = "development-channel");
const TITLE_STABLE: &str = "InstanceHarbor";
const TITLE_DEVELOPMENT: &str = "InstanceHarbor Dev";
/// The channel name without a workspace. Windows and message boxes use [`product_title`].
pub const TITLE: &str = if DEVELOPMENT { TITLE_DEVELOPMENT } else { TITLE_STABLE };
pub const APP_DIRECTORY: &str = TITLE;
pub const GUI_EXECUTABLE: &str = if DEVELOPMENT { "InstanceHarborDev.exe" } else { "InstanceHarbor.exe" };
const IDENTIFIER_STABLE: &str = "local.instanceharbor";
const IDENTIFIER_DEVELOPMENT: &str = "local.instanceharbor.dev";
pub const IDENTIFIER: &str = if DEVELOPMENT { IDENTIFIER_DEVELOPMENT } else { IDENTIFIER_STABLE };

static WORKSPACE_NAME: OnceLock<String> = OnceLock::new();

/// A development workspace name: the folder name rules `validate_workspace` enforces.
fn workspace_folder_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    (!name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
        .then(|| name.to_owned())
}

/// The workspace name a channel shows; the stable channel never names one.
fn channel_workspace(development: bool, path: Option<&Path>) -> Option<String> {
    development.then(|| path.and_then(workspace_folder_name)).flatten()
}

/// Names the development workspace this process serves, for its titles and message
/// boxes. Processes that get the workspace as an argument (Manager, console) call this.
pub fn bind_workspace(path: &Path) {
    if let Some(name) = channel_workspace(DEVELOPMENT, Some(path)) {
        let _ = WORKSPACE_NAME.set(name);
    }
}

/// The development workspace of this process: the bound one, else the folder that
/// `IH_WORKSPACE` names. Always None in the stable channel.
pub fn workspace_name() -> Option<String> {
    if let Some(name) = WORKSPACE_NAME.get() {
        return Some(name.clone());
    }
    let workspace = env::var_os("IH_WORKSPACE").filter(|path| !path.is_empty()).map(PathBuf::from);
    channel_workspace(DEVELOPMENT, workspace.as_deref())
}

/// The development workspace that `root` names, for code that has the root at hand.
pub fn workspace_of(root: &Path) -> Option<String> {
    channel_workspace(DEVELOPMENT, Some(root))
}

fn product_title_for(development: bool, workspace: Option<&str>) -> String {
    let title = if development { TITLE_DEVELOPMENT } else { TITLE_STABLE };
    match workspace.filter(|_| development) {
        Some(name) => format!("{title} \u{2014} {name}"),
        None => title.into(),
    }
}

/// The product name with the development workspace, for example
/// `InstanceHarbor Dev — accept-52b`: main window, taskbar, tray and message boxes.
pub fn product_title(workspace: Option<&str>) -> String {
    product_title_for(DEVELOPMENT, workspace)
}

fn window_title_for(development: bool, base: &str, workspace: Option<&str>) -> String {
    match workspace.filter(|_| development) {
        Some(_) => format!("{base} [{}]", product_title_for(development, workspace)),
        None => base.into(),
    }
}

/// Another window's title with the development workspace appended, for example
/// `lobby [InstanceHarbor Dev — accept-52b]`. The stable channel keeps `base`.
pub fn window_title(base: &str, workspace: Option<&str>) -> String {
    window_title_for(DEVELOPMENT, base, workspace)
}

/// A short fixed-length key for a workspace name, for names with length limits.
fn workspace_key(name: &str) -> String {
    // FNV-1a over the lowercase name, because NTFS names are case-insensitive.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in name.to_ascii_lowercase().bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn gui_lock_name_for(development: bool, workspace: Option<&str>) -> String {
    match (development, workspace) {
        (false, _) => r"Local\InstanceHarbor-GUI".into(),
        (true, None) => r"Local\InstanceHarbor-GUI-Dev".into(),
        (true, Some(name)) => format!(r"Local\InstanceHarbor-GUI-Dev-{}", workspace_key(name)),
    }
}

/// The session mutex that keeps one GUI per channel, and per workspace in development.
pub fn gui_lock_name(workspace: Option<&str>) -> String {
    gui_lock_name_for(DEVELOPMENT, workspace)
}

fn app_user_model_id_for(development: bool, workspace: Option<&str>) -> String {
    match (development, workspace) {
        (false, _) => IDENTIFIER_STABLE.into(),
        (true, None) => IDENTIFIER_DEVELOPMENT.into(),
        (true, Some(name)) => format!("{IDENTIFIER_DEVELOPMENT}.{}", workspace_key(name)),
    }
}

/// The taskbar identity; each development workspace gets its own taskbar button.
pub fn app_user_model_id(workspace: Option<&str>) -> String {
    app_user_model_id_for(DEVELOPMENT, workspace)
}

fn webview_profile_for(development: bool, local_app_data: &Path, workspace: Option<&str>) -> PathBuf {
    let app = local_app_data.join(if development { TITLE_DEVELOPMENT } else { TITLE_STABLE });
    match workspace.filter(|_| development) {
        Some(name) => app.join("webviews").join(name),
        None => app.join("webview"),
    }
}

/// The WebView2 profile folder; each development workspace has its own.
pub fn webview_profile(local_app_data: &Path, workspace: Option<&str>) -> PathBuf {
    webview_profile_for(DEVELOPMENT, local_app_data, workspace)
}

pub fn local_app_data() -> Result<PathBuf, String> {
    let path = PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("MissingLocalAppData")?);
    if !path.is_absolute() || !path.is_dir() {
        return Err("InvalidLocalAppData".into());
    }
    path.canonicalize().map_err(|error| error.to_string())
}

pub fn plain_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err("DesktopDirectoryRedirected".into());
    }
    Ok(())
}

pub fn create_plain_directory(path: &Path) -> Result<(), String> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    plain_directory(path)
}

fn starts_with(path: &Path, base: &Path) -> bool {
    let mut path = path.components();
    base.components().all(|component| {
        path.next().is_some_and(|part| {
            part.as_os_str().to_string_lossy().eq_ignore_ascii_case(&component.as_os_str().to_string_lossy())
        })
    })
}

pub fn validate_workspace(path: &Path, initializing: bool) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("ExpectedAbsoluteWorkspace".into());
    }
    // Resolve the parent for a new workspace and the workspace itself for an existing one.
    // Validate the target before initialization can create any files.
    let local = local_app_data()?;
    let dev = local.join("InstanceHarbor Dev");
    let base = dev.join("workspaces");
    if DEVELOPMENT {
        if path.components().any(|part| matches!(part, Component::ParentDir))
            || !path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            })
        {
            return Err("DevelopmentWorkspaceOutsideChannel".into());
        }
        let parent = path.parent().ok_or("DevelopmentWorkspaceOutsideChannel")?;
        // Windows canonicalization adds the extended drive prefix. Resolve only the
        // existing LocalAppData ancestor, then compare the reserved two direct children.
        let app = parent.parent().ok_or("DevelopmentWorkspaceOutsideChannel")?;
        let local_parent = app.parent().ok_or("DevelopmentWorkspaceOutsideChannel")?;
        if !parent.file_name().is_some_and(|name| name.eq_ignore_ascii_case("workspaces"))
            || !app.file_name().is_some_and(|name| name.eq_ignore_ascii_case("InstanceHarbor Dev"))
            || local_parent.canonicalize().ok().as_ref() != Some(&local)
        {
            return Err("DevelopmentWorkspaceOutsideChannel".into());
        }
        if initializing {
            create_plain_directory(&dev)?;
            create_plain_directory(&base)?;
        } else {
            plain_directory(&dev)?;
            plain_directory(&base)?;
            plain_directory(path)?;
        }
        return Ok(());
    }

    let (candidate, comparable) = if initializing {
        (path.parent().ok_or("ExpectedAbsoluteWorkspace")?, path.file_name())
    } else {
        (path, None)
    };
    if let Ok(mut candidate) = candidate.canonicalize() {
        if let Some(name) = comparable {
            candidate.push(name);
        }
        if starts_with(&candidate, &dev)
            || dev.canonicalize().is_ok_and(|resolved| starts_with(&candidate, &resolved))
        {
            return Err("StableWorkspaceOutsideChannel".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_names_never_carry_a_workspace() {
        assert_eq!(product_title_for(false, Some("agent-a")), "InstanceHarbor");
        assert_eq!(window_title_for(false, "lobby", Some("agent-a")), "lobby");
        assert_eq!(gui_lock_name_for(false, Some("agent-a")), r"Local\InstanceHarbor-GUI");
        assert_eq!(app_user_model_id_for(false, Some("agent-a")), "local.instanceharbor");
        let local = Path::new(r"C:\Users\someone\AppData\Local");
        assert_eq!(webview_profile_for(false, local, Some("agent-a")), local.join("InstanceHarbor").join("webview"));
        assert_eq!(channel_workspace(false, Some(Path::new(r"C:\x\agent-a"))), None);
    }

    #[test]
    fn development_names_show_the_workspace() {
        assert_eq!(product_title_for(true, Some("accept-52b")), "InstanceHarbor Dev \u{2014} accept-52b");
        assert_eq!(product_title_for(true, None), "InstanceHarbor Dev");
        assert_eq!(
            window_title_for(true, "lobby", Some("accept-52b")),
            "lobby [InstanceHarbor Dev \u{2014} accept-52b]"
        );
        assert_eq!(window_title_for(true, "lobby", None), "lobby");
        let local = Path::new(r"C:\Users\someone\AppData\Local");
        assert_eq!(
            webview_profile_for(true, local, Some("agent-a")),
            local.join("InstanceHarbor Dev").join("webviews").join("agent-a")
        );
    }

    #[test]
    fn each_development_workspace_has_its_own_gui_lock_and_taskbar_identity() {
        let a = gui_lock_name_for(true, Some("agent-a"));
        assert_eq!(a, format!(r"Local\InstanceHarbor-GUI-Dev-{}", workspace_key("agent-a")));
        assert_ne!(a, gui_lock_name_for(true, Some("agent-b")));
        assert_eq!(a, gui_lock_name_for(true, Some("AGENT-A")), "NTFS names are case-insensitive");
        assert_eq!(gui_lock_name_for(true, None), r"Local\InstanceHarbor-GUI-Dev");
        let id = app_user_model_id_for(true, Some(&"x".repeat(200)));
        assert!(id.starts_with("local.instanceharbor.dev.") && id.len() <= 128, "{id}");
        assert_ne!(app_user_model_id_for(true, Some("agent-a")), app_user_model_id_for(true, Some("agent-b")));
    }

    #[test]
    fn only_valid_workspace_folder_names_are_shown() {
        assert_eq!(channel_workspace(true, Some(Path::new(r"C:\x\agent_73-a"))).as_deref(), Some("agent_73-a"));
        assert_eq!(channel_workspace(true, Some(Path::new(r"C:\x\bad name"))), None);
        assert_eq!(channel_workspace(true, Some(Path::new(r"C:\x\a]b"))), None);
        assert_eq!(channel_workspace(true, None), None);
    }
}
