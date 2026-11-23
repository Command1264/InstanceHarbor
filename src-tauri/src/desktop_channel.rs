use std::{
    env, fs, io,
    os::windows::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

pub const DEVELOPMENT: bool = cfg!(feature = "development-channel");
pub const TITLE: &str = if DEVELOPMENT { "InstanceHarbor Dev" } else { "InstanceHarbor" };
pub const APP_DIRECTORY: &str = TITLE;
pub const GUI_EXECUTABLE: &str = if DEVELOPMENT { "InstanceHarborDev.exe" } else { "InstanceHarbor.exe" };
pub const IDENTIFIER: &str = if DEVELOPMENT { "local.instanceharbor.dev" } else { "local.instanceharbor" };

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
