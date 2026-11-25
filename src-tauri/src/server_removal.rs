//! Deleting a managed server's folder through the Windows Recycle Bin, the
//! list of instances removed from management, and opening a server folder.
//! The parent dispatch holds the instance lock for `delete-server`.
use super::*;
use std::{os::windows::fs::MetadataExt, path::Component};

const REPARSE_POINT: u32 = 0x400;

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    let result = match (verb, args) {
        ("delete-server", [name, confirmation]) => delete_server(root, name, confirmation),
        ("removed-instances", []) => removed_instances(root),
        ("open-server-folder", [name]) => open_server_folder(root, name),
        ("delete-server" | "removed-instances" | "open-server-folder", _) => {
            Err("InvalidPresentationArguments".into())
        }
        _ => return None,
    };
    Some(result)
}

/// Comparable spelling of a Windows path: physical drive or UNC form,
/// forward slashes, no trailing slash, case-insensitive.
fn key(path: &Path) -> String {
    path_format::display(path)
        .trim_end_matches('/')
        .to_lowercase()
}

/// `child` is `parent` itself or a folder below it.
fn within(parent: &str, child: &str) -> bool {
    child == parent || child.strip_prefix(parent).is_some_and(|rest| rest.starts_with('/'))
}

fn resolved(path: &Path) -> String {
    key(&fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// Folders a server deletion must never touch. `folder` is the canonical
/// server folder; `others` are the folders of the other registered instances.
pub(super) fn check_target(
    root: &Path,
    folder: &Path,
    management_roots: &[PathBuf],
    others: &[PathBuf],
) -> Result<()> {
    match folder.components().next() {
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)) => {}
        _ => return Err("DeleteServerNetworkPath".into()),
    }
    if !folder.components().any(|part| matches!(part, Component::Normal(_))) {
        return Err("DeleteServerProtectedFolder".into());
    }
    let target = key(folder);
    let workspace = resolved(root);
    let instances = resolved(&root.join("instances"));
    if within(&target, &workspace) {
        return Err("DeleteServerProtectedFolder".into());
    }
    if within(&workspace, &target) && (target == instances || !within(&instances, &target)) {
        return Err("DeleteServerProtectedFolder".into());
    }
    // Creating or adopting an instance lists its own folder as a management
    // path, so equality alone does not mean "folder of many servers". Refuse
    // only when a direct child also looks like a server folder.
    if management_roots.iter().any(|path| resolved(path) == target) && holds_server_folder(folder)? {
        return Err("DeleteServerProtectedFolder".into());
    }
    if others.iter().any(|path| within(&target, &resolved(path))) {
        return Err("DeleteServerContainsOtherInstance".into());
    }
    Ok(())
}

/// Drop the management paths that name exactly `folder`.
fn forget_folder(root: &Path, folder: &Path) -> Result<()> {
    let (mut policy, revision) = scan_policy::load(root)?;
    let target = key(folder);
    let changed = policy.retain_allow_paths(|rule| {
        scan_policy::rule_path(root, rule).map_or(true, |path| key(&path) != target)
    });
    if changed {
        scan_policy::persist(root, &policy, &revision)?;
    }
    Ok(())
}

/// A direct child folder holds a server marker file, as a management folder
/// scan would find it.
fn holds_server_folder(folder: &Path) -> Result<bool> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(fail(error)),
    };
    for entry in entries {
        let child = entry.map_err(fail)?.path();
        if child.is_dir()
            && ["server.properties", "server.jar", "velocity.toml"].iter().any(|marker| child.join(marker).is_file())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Refuse a folder whose path or contents include a link (reparse point),
/// so the Recycle Bin never moves or follows anything outside the folder.
pub(super) fn check_no_links(folder: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for part in folder.components() {
        current.push(part);
        if !matches!(part, Component::Normal(_)) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current).map_err(fail)?;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err("DeleteServerContainsLink".into());
        }
    }
    let mut pending = vec![folder.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(fail)? {
            let entry = entry.map_err(fail)?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(fail)?;
            if metadata.file_attributes() & REPARSE_POINT != 0 {
                return Err("DeleteServerContainsLink".into());
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

fn registered_names(root: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(root.join("registry")).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json")) {
            if let Some(name) = path.file_stem().and_then(|stem| stem.to_str()).filter(|name| instance_name(name)) {
                names.push(name.to_owned());
            }
        }
    }
    names.sort();
    Ok(names)
}

fn delete_server(root: &Path, name: &str, confirmation: &str) -> Result<Value> {
    if confirmation != name {
        return Err("DeleteServerNameMismatch".into());
    }
    let item = presentation::active_instance(root, name)?;
    presentation::stopped(root, &item)?;
    let folder = instance_path(root, name)?;
    let (policy, _) = scan_policy::load(root)?;
    let management_roots = policy
        .management_roots()
        .iter()
        .map(|rule| scan_policy::rule_path(root, rule))
        .collect::<Result<Vec<_>>>()?;
    let others = registered_names(root)?
        .into_iter()
        .filter(|other| other != name)
        .filter_map(|other| {
            let record = instance(root, &other).ok()?;
            scan_policy::registered_path(root, &other, record.directory.as_deref()).ok()
        })
        .collect::<Vec<_>>();
    check_target(root, &folder, &management_roots, &others)?;
    check_no_links(&folder)?;
    let core = core_artifact::jar(&folder, &item);
    if core.is_file() {
        core_artifact::assert_unused(&core)?;
    }
    recycle(&folder)?;
    // The folder is in the Recycle Bin now. Unregister it the same way as
    // "remove and delete settings", which keeps the opt-out record for a
    // server found by a management folder scan.
    presentation::remove_management(root, name, "delete")
        .map_err(|error| format!("DeleteServerRecycledButRegistered:{error}"))?;
    // Creating or adopting the instance listed its folder as a management
    // path. The folder is gone, so drop that path too.
    forget_folder(root, &folder).map_err(|error| format!("DeleteServerRecycledButPathListed:{error}"))?;
    Ok(json!({"name":name,"recycled":true,"path":path_format::display(&folder)}))
}

#[repr(C)]
struct FileOperation {
    window: *mut c_void,
    function: u32,
    from: *const u16,
    to: *const u16,
    flags: u16,
    aborted: i32,
    mappings: *mut c_void,
    title: *const u16,
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHFileOperationW(operation: *mut FileOperation) -> i32;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, mode: u32) -> i32;
    fn CoUninitialize();
}

const FO_DELETE: u32 = 3;
// No Windows progress window; InstanceHarbor shows its own progress text.
const FOF_SILENT: u16 = 0x0004;
const FOF_NOCONFIRMATION: u16 = 0x0010;
const FOF_ALLOWUNDO: u16 = 0x0040;
const FOF_NOERRORUI: u16 = 0x0400;
// Windows still asks before deleting permanently, for example when the folder
// is too large for the Recycle Bin. This overrides FOF_NOCONFIRMATION for that case only.
const FOF_WANTNUKEWARNING: u16 = 0x4000;
const RECYCLE_FLAGS: u16 = FOF_ALLOWUNDO | FOF_SILENT | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_WANTNUKEWARNING;

fn recycle(folder: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    // The shell does not accept the \\?\ namespace; it takes the ordinary
    // drive spelling, terminated by two NULs.
    let physical = path_format::display(folder).replace('/', "\\");
    let from: Vec<u16> = std::ffi::OsStr::new(&physical).encode_wide().chain([0, 0]).collect();
    let initialized = unsafe { CoInitializeEx(std::ptr::null_mut(), 0x2 | 0x4) };
    let mut operation = FileOperation {
        window: std::ptr::null_mut(),
        function: FO_DELETE,
        from: from.as_ptr(),
        to: std::ptr::null(),
        flags: RECYCLE_FLAGS,
        aborted: 0,
        mappings: std::ptr::null_mut(),
        title: std::ptr::null(),
    };
    let status = unsafe { SHFileOperationW(&mut operation) };
    if initialized >= 0 {
        unsafe { CoUninitialize() };
    }
    if operation.aborted != 0 {
        return Err("DeleteServerCancelled".into());
    }
    if status != 0 {
        return Err(format!("DeleteServerRecycleFailed:{status:#x}"));
    }
    if fs::symlink_metadata(folder).is_ok() {
        return Err("DeleteServerIncomplete".into());
    }
    Ok(())
}

fn removed_instances(root: &Path) -> Result<Value> {
    let mut rows = Vec::new();
    for name in registered_names(root)? {
        let Ok(item) = instance(root, &name) else { continue };
        if !item.removed || !admitted(root, &name).unwrap_or(false) {
            continue;
        }
        let folder = scan_policy::registered_path(root, &name, item.directory.as_deref()).ok();
        let present = folder.as_ref().is_some_and(|path| path.is_dir());
        rows.push(json!({
            "name": name,
            "port": item.port,
            "path": folder.as_deref().map(path_format::display),
            "folderPresent": present,
            "java": (!item.java.as_os_str().is_empty()).then(|| path_format::display(&item.java)),
            "managementDiscovered": item.management_discovered,
        }));
    }
    Ok(json!(rows))
}

fn open_server_folder(root: &Path, name: &str) -> Result<Value> {
    if !instance_name(name) || !admitted(root, name)? {
        return Err("InstanceExcludedOrInvalid".into());
    }
    let folder = instance_path(root, name)?;
    Command::new("explorer.exe")
        .arg(&folder)
        .creation_flags(HIDDEN)
        .spawn()
        .map_err(fail)?;
    Ok(json!({"opened":true,"path":path_format::display(&folder)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(label: &str) -> (Fixture, PathBuf) {
        let base = std::env::temp_dir().join(format!("ih-removal-{label}-{}-{}", std::process::id(), clock()));
        let root = base.join("workspace");
        for folder in ["instances/alpha", "registry", "runs"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        let base = fs::canonicalize(&base).unwrap();
        (Fixture(base.clone()), base)
    }

    #[test]
    fn name_confirmation_must_match_exactly_before_anything_is_read() {
        let (_guard, base) = fixture("confirm");
        let root = base.join("workspace");
        for typed in ["", "alph", "Alpha", "alpha "] {
            assert_eq!(delete_server(&root, "alpha", typed).unwrap_err(), "DeleteServerNameMismatch");
        }
        assert!(root.join("instances/alpha").is_dir());
    }

    #[test]
    fn deleted_folder_leaves_the_management_paths() {
        // A stale path to a folder in the Recycle Bin would make every later
        // management path update fail with ScanDirectoryUnavailable.
        let (_guard, base) = fixture("forget");
        let root = base.join("workspace");
        let alpha = root.join("instances/alpha");
        let keep = base.join("servers");
        fs::create_dir_all(&keep).unwrap();
        let policy = json!({
            "allow": [], "exclude": [], "excludePaths": [], "managedFolders": true,
            "allowPaths": [path_format::display(&alpha).to_uppercase(), path_format::display(&keep)],
        });
        fs::write(root.join("scan-policy.json"), serde_json::to_vec(&policy).unwrap()).unwrap();
        fs::remove_dir_all(&alpha).unwrap();
        forget_folder(&root, &alpha).unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(root.join("scan-policy.json")).unwrap()).unwrap();
        assert_eq!(saved["allowPaths"], json!([path_format::display(&keep)]));
        // Nothing to forget: the file stays as it is.
        let before = fs::read(root.join("scan-policy.json")).unwrap();
        forget_folder(&root, &alpha).unwrap();
        assert_eq!(fs::read(root.join("scan-policy.json")).unwrap(), before);
    }

    #[test]
    fn recycling_hides_progress_but_keeps_the_permanent_delete_warning() {
        assert_ne!(RECYCLE_FLAGS & FOF_SILENT, 0);
        assert_ne!(RECYCLE_FLAGS & FOF_ALLOWUNDO, 0);
        assert_ne!(RECYCLE_FLAGS & FOF_WANTNUKEWARNING, 0);
    }

    #[test]
    fn workspace_instances_folder_child_may_be_deleted() {
        let (_guard, base) = fixture("child");
        let root = base.join("workspace");
        assert!(check_target(&root, &root.join("instances/alpha"), &[], &[]).is_ok());
    }

    #[test]
    fn workspace_and_its_data_folders_are_refused() {
        let (_guard, base) = fixture("workspace");
        let root = base.join("workspace");
        for folder in [root.clone(), root.join("instances"), root.join("registry"), root.join("runs")] {
            assert_eq!(check_target(&root, &folder, &[], &[]).unwrap_err(), "DeleteServerProtectedFolder");
        }
        // A folder that contains the workspace.
        assert_eq!(check_target(&root, &base, &[], &[]).unwrap_err(), "DeleteServerProtectedFolder");
    }

    #[test]
    fn drive_roots_network_paths_and_management_roots_are_refused() {
        let (_guard, base) = fixture("roots");
        let root = base.join("workspace");
        assert_eq!(check_target(&root, Path::new(r"C:\"), &[], &[]).unwrap_err(), "DeleteServerProtectedFolder");
        assert_eq!(check_target(&root, Path::new(r"\\?\C:\"), &[], &[]).unwrap_err(), "DeleteServerProtectedFolder");
        assert_eq!(
            check_target(&root, Path::new(r"\\server\share\srv"), &[], &[]).unwrap_err(),
            "DeleteServerNetworkPath"
        );
        assert_eq!(
            check_target(&root, Path::new(r"\\?\UNC\server\share\srv"), &[], &[]).unwrap_err(),
            "DeleteServerNetworkPath"
        );
        let managed = base.join("servers");
        fs::create_dir_all(managed.join("one")).unwrap();
        fs::write(managed.join("one/server.properties"), b"").unwrap();
        let roots = [PathBuf::from(path_format::display(&managed))];
        assert_eq!(check_target(&root, &managed, &roots, &[]).unwrap_err(), "DeleteServerProtectedFolder");
        assert!(check_target(&root, &managed.join("one"), &roots, &[]).is_ok());
    }

    #[test]
    fn server_folder_listed_as_its_own_management_path_may_be_deleted() {
        // Creating or adopting an instance adds its own folder to the
        // management paths; that alone must not protect it.
        let (_guard, base) = fixture("own-root");
        let root = base.join("workspace");
        for folder in [root.join("instances/alpha"), base.join("elsewhere")] {
            fs::create_dir_all(folder.join("world")).unwrap();
            fs::write(folder.join("server.properties"), b"").unwrap();
            let roots = [PathBuf::from(path_format::display(&folder))];
            assert!(check_target(&root, &folder, &roots, &[]).is_ok());
        }
    }

    #[test]
    fn folder_holding_another_instance_is_refused() {
        let (_guard, base) = fixture("nested");
        let root = base.join("workspace");
        let outer = base.join("outer");
        fs::create_dir_all(outer.join("inner")).unwrap();
        assert_eq!(
            check_target(&root, &outer, &[], &[outer.join("inner")]).unwrap_err(),
            "DeleteServerContainsOtherInstance"
        );
        assert!(check_target(&root, &outer.join("inner"), &[], std::slice::from_ref(&outer)).is_ok());
    }

    #[test]
    fn plain_folder_passes_link_check_and_junction_is_refused() {
        let (_guard, base) = fixture("links");
        let folder = base.join("server");
        fs::create_dir_all(folder.join("world/region")).unwrap();
        fs::write(folder.join("world/level.dat"), b"x").unwrap();
        assert!(check_no_links(&folder).is_ok());
        let target = base.join("elsewhere");
        fs::create_dir_all(&target).unwrap();
        let junction = folder.join("plugins");
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .creation_flags(HIDDEN)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(check_no_links(&folder).unwrap_err(), "DeleteServerContainsLink");
        fs::remove_dir(&junction).unwrap();
    }
}
