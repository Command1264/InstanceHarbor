//! Shared cores (#62). Each core JAR is stored once in the workspace, in
//! `shared-cores/<SHA-256>.jar`, and marked read-only. An instance folder keeps
//! its core JAR as another name (an NTFS hardlink) of that file, so the folder
//! still starts on its own, and V1 fallback, hand-over and export are unchanged.
//! When the instance folder is on another drive, or the link fails, the core is
//! copied as before.
//!
//! The number of names of a stored file tells how many instances use it: a
//! stored file with only its own name and no open handle is unused, and the
//! "Clear cache" page may delete it.
use super::*;
use std::os::windows::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawHandle,
};

pub(super) const DIRECTORY: &str = "shared-cores";
const READONLY: u32 = 0x1;
const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_DELETE: u32 = 0x4;
const DELETE: u32 = 0x0001_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const REPARSE_POINT: u32 = 0x400;

#[repr(C)]
#[derive(Default)]
struct ByHandle {
    attributes: u32,
    creation: [u32; 2],
    access: [u32; 2],
    write: [u32; 2],
    volume: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

#[repr(C)]
struct RenameInfo {
    flags: u32,
    root: *mut std::ffi::c_void,
    length: u32,
    name: [u16; 1],
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(file: *mut std::ffi::c_void, info: *mut ByHandle) -> i32;
    fn SetFileInformationByHandle(file: *mut std::ffi::c_void, class: i32, info: *const std::ffi::c_void, size: u32) -> i32;
}

/// The volume, file index and number of names of an open file.
fn information(file: &File) -> Result<(u32, u64, u32)> {
    let mut info = ByHandle::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(fail(io::Error::last_os_error()));
    }
    Ok((info.volume, (u64::from(info.index_high) << 32) | u64::from(info.index_low), info.links))
}

/// Opens a plain file to read its attributes, without blocking any other user.
fn inspect(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(7)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| file_error("inspect-shared-core", path, error))?;
    let metadata = file.metadata().map_err(fail)?;
    if metadata.file_attributes() & REPARSE_POINT != 0 || !metadata.is_file() {
        return Err("CoreMaintenanceUnsafePath".into());
    }
    Ok(file)
}

/// The number of names of a file; 1 for a file that is not linked.
pub(super) fn names(path: &Path) -> Result<u32> {
    information(&inspect(path)?).map(|(_, _, links)| links)
}

fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn directory(root: &Path) -> Result<PathBuf> {
    match fs::create_dir(root.join(DIRECTORY)) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(fail(error)),
    }
    child_directory(root, DIRECTORY)
}

/// The stored file of `sha256`, when the store has one.
pub(super) fn stored(root: &Path, sha256: &str) -> Result<Option<PathBuf>> {
    if !digest(sha256) {
        return Ok(None);
    }
    let path = root.join(DIRECTORY).join(format!("{sha256}.jar"));
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && metadata.file_attributes() & REPARSE_POINT == 0 => {
            no_redirect(&path)?;
            Ok(Some(path))
        }
        Ok(_) => Err("CoreMaintenanceUnsafePath".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(fail(error)),
    }
}

/// Stores the bytes of `source` once as `shared-cores/<sha256>.jar`, read-only.
/// A stored file whose bytes no longer match its name is replaced.
pub(super) fn publish(root: &Path, source: &Path, sha256: &str) -> Result<PathBuf> {
    if !digest(sha256) {
        return Err("CoreArtifactHashMismatch".into());
    }
    let home = directory(root)?;
    let _lock = command_lock(&home, &format!("{sha256}.lock"), false)?;
    let path = home.join(format!("{sha256}.jar"));
    if stored(root, sha256)?.is_some() {
        if content_revision::file_revision(&path)? == sha256 {
            return Ok(path);
        }
        // Only this name is removed: other names keep their bytes, and their
        // start is refused because the bytes no longer match their registration.
        fs::remove_file(&path).map_err(|error| file_error("replace-shared-core", &path, error))?;
    }
    let stage = home.join(format!("{sha256}.{}.{}.tmp", std::process::id(), clock()));
    let published = (|| {
        core_maintenance::copy_verified(source, &stage, sha256)?;
        let mut permissions = fs::metadata(&stage).map_err(fail)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&stage, permissions).map_err(fail)?;
        core_maintenance::move_durable(&stage, &path, false)
    })();
    if published.is_err() && fs::symlink_metadata(&stage).is_ok() {
        fs::remove_file(&stage).map_err(|error| file_error("discard-shared-core", &stage, error))?;
    }
    published.map(|()| path)
}

/// How a core reached an instance folder.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Placement {
    /// Another name of the stored file.
    Linked,
    /// A separate copy: another drive, or the link failed.
    Copied,
}

/// Puts the core `source` with `sha256` at `destination`, which must not exist:
/// a hardlink to the stored file, or else a writable copy.
pub(super) fn place(root: &Path, source: &Path, sha256: &str, destination: &Path) -> Result<Placement> {
    // A link cannot leave its drive, so another drive gets a plain copy and
    // the store is not filled for it.
    if !same_drive(root, destination) {
        core_maintenance::copy_verified(source, destination, sha256)?;
        return Ok(Placement::Copied);
    }
    let store = publish(root, source, sha256)?;
    if fs::hard_link(&store, destination).is_ok() {
        return Ok(Placement::Linked);
    }
    core_maintenance::copy_verified(&store, destination, sha256)?;
    Ok(Placement::Copied)
}

/// Whether two absolute paths start with the same drive letter.
fn same_drive(left: &Path, right: &Path) -> bool {
    use std::path::{Component, Prefix};
    let drive = |path: &Path| match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => Some(letter.to_ascii_uppercase()),
            _ => None,
        },
        _ => None,
    };
    drive(left).is_some_and(|letter| drive(right) == Some(letter))
}

/// How many names besides its own the stored file of an instance core has, when
/// `jar` is a name of the stored file of `sha256`. The detail panel shows it.
pub(super) fn users(root: &Path, jar: &Path, sha256: &str) -> Option<u32> {
    let store = stored(root, sha256).ok().flatten()?;
    let (volume, index, links) = information(&inspect(jar).ok()?).ok()?;
    let (store_volume, store_index, _) = information(&inspect(&store).ok()?).ok()?;
    (volume == store_volume && index == store_index && links >= 2).then(|| links - 1)
}

/// Opens the core JAR that a Host launches. Reading and removing names stay
/// allowed, writing is denied while Java runs (#62): A and B share a core, B
/// runs, and stopped A can still be updated because only A's name changes.
/// A shared core is checked against its registered SHA-256 first.
pub(super) fn open_for_launch(jar: &Path, expected: Option<&str>) -> Result<File> {
    no_redirect(jar)?;
    let guard = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(jar)
        .map_err(|error| file_error("open-core-for-launch", jar, error))?;
    let metadata = guard.metadata().map_err(fail)?;
    if metadata.file_attributes() & REPARSE_POINT != 0 || !metadata.is_file() {
        return Err("CoreMaintenanceUnsafePath".into());
    }
    let (_, _, links) = information(&guard)?;
    if let (true, Some(expected)) = (links > 1, expected) {
        if content_revision::file_revision(jar)? != expected {
            return Err("SharedCoreDigestMismatch".into());
        }
    }
    Ok(guard)
}

/// Whether a file has the read-only attribute, which every name of a stored
/// core shares.
pub(super) fn read_only(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_attributes() & READONLY != 0)
}

fn verbatim(path: &Path) -> Vec<u16> {
    let text = path.as_os_str();
    let mut wide: Vec<u16> = text.encode_wide().collect();
    let disk = matches!(path.components().next(), Some(std::path::Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_)));
    if disk && path.is_absolute() {
        // A verbatim path takes no forward slashes.
        let slash = u16::from(b'/');
        let backslash = u16::from(b'\\');
        wide = r"\\?\".encode_utf16().chain(wide.into_iter().map(|unit| if unit == slash { backslash } else { unit })).collect();
    }
    wide
}

/// Replaces `destination` with `source` although `destination` is read-only, a
/// name of a stored core (#62). The rename is atomic and keeps the attribute of
/// the stored file for its other names.
pub(super) fn replace_read_only(source: &Path, destination: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .access_mode(DELETE | SYNCHRONIZE)
        .share_mode(7)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(source)
        .map_err(|error| file_error("publish-core-file", source, error))?;
    let name = verbatim(destination);
    let offset = std::mem::offset_of!(RenameInfo, name);
    let size = offset + name.len() * 2 + 2;
    let mut buffer = vec![0u64; size.div_ceil(8)];
    let info = buffer.as_mut_ptr().cast::<RenameInfo>();
    unsafe {
        // REPLACE_IF_EXISTS | POSIX_SEMANTICS | IGNORE_READONLY_ATTRIBUTE
        (*info).flags = 0x1 | 0x2 | 0x40;
        (*info).root = std::ptr::null_mut();
        (*info).length = (name.len() * 2) as u32;
        let target = std::ptr::addr_of_mut!((*info).name).cast::<u16>();
        std::ptr::copy_nonoverlapping(name.as_ptr(), target, name.len());
        // FileRenameInfoEx
        if SetFileInformationByHandle(file.as_raw_handle(), 22, buffer.as_ptr().cast(), size as u32) == 0 {
            return Err(file_error("publish-core-file", destination, io::Error::last_os_error()));
        }
    }
    Ok(())
}

/// A stored core that no instance names any more, for "Clear cache" (#29).
pub(super) struct Unused {
    pub(super) path: PathBuf,
    pub(super) bytes: u64,
    /// A process has the file open.
    pub(super) open: bool,
}

/// Whether a process has `path` open for reading, writing or removal: an
/// exclusive read fails. Attribute-only access is not subject to sharing.
fn held(path: &Path) -> bool {
    OpenOptions::new()
        .read(true)
        .share_mode(0)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)
        .is_err()
}

/// Stored cores with only their own name.
pub(super) fn unused(root: &Path) -> Result<Vec<Unused>> {
    let home = root.join(DIRECTORY);
    if fs::symlink_metadata(&home).is_err() {
        return Ok(Vec::new());
    }
    let home = child_directory(root, DIRECTORY)?;
    let mut found = Vec::new();
    for entry in fs::read_dir(&home).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        if !entry.file_name().to_str().and_then(|name| name.strip_suffix(".jar")).is_some_and(digest) {
            continue;
        }
        let path = home.join(entry.file_name());
        let Ok(file) = inspect(&path) else { continue };
        let (_, _, links) = information(&file)?;
        let bytes = file.metadata().map_err(fail)?.len();
        drop(file);
        if links == 1 {
            found.push(Unused { open: held(&path), path, bytes });
        }
    }
    found.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(found)
}

/// Deletes a stored core that is still unused: one name and no open handle.
pub(super) fn remove_unused(root: &Path, sha256: &str) -> std::result::Result<(), String> {
    let home = child_directory(root, DIRECTORY)?;
    let _lock = command_lock(&home, &format!("{sha256}.lock"), false).map_err(|_| "InUse".to_owned())?;
    let Some(path) = stored(root, sha256)? else { return Ok(()) };
    if names(&path)? != 1 {
        return Err("SharedCoreInUse".into());
    }
    if held(&path) {
        return Err("InUse".into());
    }
    fs::remove_file(&path).map_err(|error| file_error("remove-shared-core", &path, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn folder() -> Folder {
        let path = std::env::temp_dir().join(format!("ih-shared-core-{}-{}", std::process::id(), clock()));
        fs::create_dir(&path).unwrap();
        Folder(fs::canonicalize(path).unwrap())
    }

    #[test]
    fn a_link_never_leaves_its_drive() {
        assert!(same_drive(Path::new(r"C:\workspace"), Path::new(r"c:\instances\a\server.jar")));
        assert!(same_drive(Path::new(r"\\?\D:\workspace"), Path::new(r"D:\a\server.jar")));
        assert!(!same_drive(Path::new(r"C:\workspace"), Path::new(r"D:\a\server.jar")));
        assert!(!same_drive(Path::new(r"\server\share\workspace"), Path::new(r"\server\share\a.jar")));
        assert!(!same_drive(Path::new("relative"), Path::new("relative/a.jar")));
    }

    // #62: one stored, read-only file per SHA-256; changed bytes are stored again.
    #[test]
    fn a_core_is_stored_once_read_only_and_replaced_when_its_bytes_changed() {
        let root = folder();
        let source = root.0.join("core.jar");
        fs::write(&source, b"core").unwrap();
        let sha = content_revision::revision(b"core").unwrap();
        assert_eq!(publish(&root.0, &source, &"A".repeat(64)).unwrap_err(), "CoreArtifactHashMismatch");
        let first = root.0.join("a.jar");
        let second = root.0.join("b.jar");
        assert_eq!(place(&root.0, &source, &sha, &first).unwrap(), Placement::Linked);
        assert_eq!(place(&root.0, &source, &sha, &second).unwrap(), Placement::Linked);
        let stored = stored(&root.0, &sha).unwrap().unwrap();
        assert_eq!((names(&stored).unwrap(), read_only(&stored)), (3, true));
        assert_eq!(users(&root.0, &first, &sha), Some(2));
        assert_eq!(users(&root.0, &source, &sha), None, "the source is not a name of the stored file");
        assert!(place(&root.0, &source, &sha, &first).is_err(), "an existing destination is never overwritten");
        // A stored file whose bytes changed is published again, apart from old names.
        let mut permissions = fs::metadata(&stored).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&stored, permissions).unwrap();
        fs::write(&stored, b"tampered").unwrap();
        let third = root.0.join("c.jar");
        place(&root.0, &source, &sha, &third).unwrap();
        assert_eq!(fs::read(&third).unwrap(), b"core");
        assert_eq!(users(&root.0, &third, &sha), Some(1));
        assert_eq!(open_for_launch(&first, Some(&sha)).unwrap_err(), "SharedCoreDigestMismatch");
    }

    // #62: a read-only name is replaced atomically; other names keep their bytes.
    #[test]
    fn a_read_only_name_is_replaced_without_touching_its_other_names() {
        let root = folder();
        let source = root.0.join("core.jar");
        fs::write(&source, b"old").unwrap();
        let sha = content_revision::revision(b"old").unwrap();
        let name = root.0.join("server.jar");
        place(&root.0, &source, &sha, &name).unwrap();
        let next = root.0.join("next.jar");
        fs::write(&next, b"new").unwrap();
        core_maintenance::move_durable(&next, &name, true).unwrap();
        assert_eq!(fs::read(&name).unwrap(), b"new");
        assert!(!read_only(&name) && !next.exists());
        let stored = stored(&root.0, &sha).unwrap().unwrap();
        assert_eq!((fs::read(&stored).unwrap(), names(&stored).unwrap(), read_only(&stored)), (b"old".to_vec(), 1, true));
    }
}
