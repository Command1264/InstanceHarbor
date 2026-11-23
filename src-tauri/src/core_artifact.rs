//! Find, identify and inspect the server core JAR of an instance folder.
//!
//! A registration records the core's file name. When that file is gone, for
//! example after a Velocity update whose file name carries the build number,
//! the single core of the same kind in the folder is used instead. A core that
//! the built-in hash table does not know is trusted only after its SHA-256
//! matches the official PaperMC catalog.
use super::*;
use std::{
    collections::HashMap,
    os::windows::fs::MetadataExt,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

pub(super) const DEFAULT: &str = "server.jar";
const REPARSE_POINT: u32 = 0x400;
const CATALOG_RETRY: Duration = Duration::from_secs(60);

/// A core file name is a plain `.jar` name inside the instance folder.
pub(super) fn valid_name(name: &str) -> bool {
    name.len() <= 128
        && name.len() > 4
        && name.to_ascii_lowercase().ends_with(".jar")
        && !name.starts_with('.')
        && !name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '"' | '*' | '?' | '<' | '>' | '|') || c.is_control())
}

pub(super) fn recorded(config: &Instance) -> &str {
    config.core_file.as_deref().unwrap_or(DEFAULT)
}

/// The recorded core path of a registered instance.
pub(super) fn jar(path: &Path, config: &Instance) -> PathBuf {
    path.join(recorded(config))
}

/// Pass the stored name only when it differs from the default.
pub(super) fn stored(name: &str) -> Option<String> {
    (name != DEFAULT).then(|| name.to_owned())
}

fn plain_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_file() && metadata.file_attributes() & REPARSE_POINT == 0),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(fail(error)),
    }
}

/// The family and version a JAR states about itself. This is a claim only;
/// `identify` confirms it by hash.
pub(super) fn claim(jar: &Path) -> Option<(&'static str, String)> {
    let mut file = File::open(jar).ok()?;
    if let Some(Some(list)) = plugin_descriptor::core_metadata(&mut file, b"META-INF/patches.list") {
        let text = String::from_utf8(list).ok()?;
        let output = text.lines().find(|line| line.starts_with("versions\t"))?.rsplit('\t').next()?;
        let (version, file_name) = output.split_once('/')?;
        let family = match file_name.strip_suffix(&format!("-{version}.jar"))? {
            "paper" => "PAPER",
            "folia" => "FOLIA",
            _ => return None,
        };
        return Some((family, version.to_owned()));
    }
    let manifest = plugin_descriptor::core_metadata(&mut file, b"META-INF/MANIFEST.MF")??;
    let text = String::from_utf8(manifest).ok()?;
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key).map(str::trim))
            .map(str::to_owned)
    };
    if field("Implementation-Title:")? != "Velocity" {
        return None;
    }
    let version = field("Implementation-Version:")?;
    Some(("VELOCITY", version.split_whitespace().next()?.to_owned()))
}

enum Cached {
    Known(Option<CoreProfile>),
    Failed(Instant),
}

fn cache() -> &'static Mutex<HashMap<String, Cached>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The verified profile of a core: the built-in table first, then the
/// official PaperMC catalog. `CoreCatalogUnavailable` means the catalog could
/// not be reached; the result is not cached as unknown.
pub(super) fn identify(jar: &Path) -> Result<Option<CoreProfile>> {
    let sha256 = content_revision::file_revision(jar)?;
    if let Some(profile) = pinned_core_profile(sha256.clone()) {
        return Ok(Some(profile));
    }
    match cache().lock().map_err(|_| "CoreCatalogUnavailable")?.get(&sha256) {
        Some(Cached::Known(profile)) => return Ok(profile.clone()),
        Some(Cached::Failed(at)) if at.elapsed() < CATALOG_RETRY => return Err("CoreCatalogUnavailable".into()),
        _ => (),
    }
    let Some((family, version)) = claim(jar) else {
        return Ok(None);
    };
    let result = core_source::catalog_profile(family, &version, &sha256);
    let entry = match &result {
        Ok(profile) => Cached::Known(profile.clone()),
        Err(_) => Cached::Failed(Instant::now()),
    };
    cache().lock().map_err(|_| "CoreCatalogUnavailable")?.insert(sha256, entry);
    result
}

/// Find the core file of a folder. `server.jar` wins when present. Otherwise
/// the single top-level JAR that states the expected kind is used; a folder
/// with exactly one JAR is accepted for a non-proxy server whose core states
/// nothing (for example Spigot).
pub(super) fn locate(directory: &Path, velocity: bool) -> Result<Option<String>> {
    if plain_file(&directory.join(DEFAULT))? {
        return Ok(Some(DEFAULT.to_owned()));
    }
    let mut jars = Vec::new();
    for item in fs::read_dir(directory).map_err(fail)? {
        let item = item.map_err(fail)?;
        let Some(name) = item.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if valid_name(&name) && plain_file(&item.path())? {
            jars.push(name);
        }
    }
    jars.sort();
    let matching: Vec<_> = jars
        .iter()
        .filter(|name| {
            claim(&directory.join(name)).is_some_and(|(family, _)| (family == "VELOCITY") == velocity)
        })
        .cloned()
        .collect();
    match (matching.as_slice(), jars.as_slice()) {
        ([name], _) => Ok(Some(name.clone())),
        ([], [name]) if !velocity && claim(&directory.join(name)).is_none() => Ok(Some(name.clone())),
        ([], _) => Ok(None),
        _ => Err("CoreArtifactAmbiguous".into()),
    }
}

/// Before a start, follow a core that was replaced under another file name and
/// accept a newer verified build of the same family (and, for a game server,
/// the same Minecraft version). The caller holds the instance lock.
pub(super) fn adopt(root: &Path, path: &Path, config: Instance) -> Result<Instance> {
    let current = jar(path, &config);
    let velocity = registered_velocity(&config).is_some();
    let name = if plain_file(&current)? {
        recorded(&config).to_owned()
    } else {
        match locate(path, velocity)? {
            Some(name) => name,
            None => return Ok(config),
        }
    };
    let mut next = config.clone();
    next.core_file = stored(&name);
    if let Some(profile) = &config.core_profile {
        let core = path.join(&name);
        if content_revision::file_revision(&core)? != profile.sha256 {
            match identify(&core)? {
                Some(found)
                    if found.family == profile.family
                        && (found.family == "VELOCITY" || found.minecraft == profile.minecraft) =>
                {
                    next.core_profile = Some(found);
                }
                _ => return Ok(config),
            }
        }
    }
    let changed = next.core_file != config.core_file
        || next.core_profile.as_ref().map(|profile| &profile.sha256)
            != config.core_profile.as_ref().map(|profile| &profile.sha256);
    if changed {
        core_maintenance::write_durable(&record_path(root, &config.name), &next, true)?;
    }
    Ok(next)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UniqueProcess {
    process_id: u32,
    start_time: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessInfo {
    process: UniqueProcess,
    app_name: [u16; 256],
    service_name: [u16; 64],
    app_type: u32,
    app_status: u32,
    session_id: u32,
    restartable: i32,
}

#[link(name = "rstrtmgr")]
unsafe extern "system" {
    fn RmStartSession(session: *mut u32, flags: u32, key: *mut u16) -> u32;
    fn RmEndSession(session: u32) -> u32;
    fn RmRegisterResources(
        session: u32,
        files: u32,
        names: *const *const u16,
        processes: u32,
        unique: *const UniqueProcess,
        services: u32,
        service_names: *const *const u16,
    ) -> u32;
    fn RmGetList(
        session: u32,
        needed: *mut u32,
        count: *mut u32,
        info: *mut ProcessInfo,
        reasons: *mut u32,
    ) -> u32;
}

const ERROR_MORE_DATA: u32 = 234;

/// Processes that hold the file open, from the Windows Restart Manager. This
/// only reads; it never asks a process to close.
pub(super) fn holders(file: &Path) -> Result<Vec<(u32, String)>> {
    use std::os::windows::ffi::OsStrExt;
    let mut session = 0;
    let mut key = [0u16; 33];
    let status = unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) };
    if status != 0 {
        return Err(format!("CoreHolderQueryFailed:{status}"));
    }
    let result = (|| {
        let wide: Vec<u16> = file.as_os_str().encode_wide().chain([0]).collect();
        let names = [wide.as_ptr()];
        let status = unsafe {
            RmRegisterResources(session, 1, names.as_ptr(), 0, std::ptr::null(), 0, std::ptr::null())
        };
        if status != 0 {
            return Err(format!("CoreHolderQueryFailed:{status}"));
        }
        let mut list = Vec::new();
        for _ in 0..3 {
            let (mut needed, mut count, mut reasons) = (0, list.len() as u32, 0);
            let status = unsafe {
                RmGetList(session, &mut needed, &mut count, list.as_mut_ptr(), &mut reasons)
            };
            match status {
                0 => {
                    list.truncate(count as usize);
                    return Ok(list
                        .iter()
                        .map(|info| {
                            let end = info.app_name.iter().position(|c| *c == 0).unwrap_or(info.app_name.len());
                            (info.process.process_id, String::from_utf16_lossy(&info.app_name[..end]))
                        })
                        .collect());
                }
                ERROR_MORE_DATA => list.resize(
                    needed as usize,
                    ProcessInfo {
                        process: UniqueProcess { process_id: 0, start_time: [0; 2] },
                        app_name: [0; 256],
                        service_name: [0; 64],
                        app_type: 0,
                        app_status: 0,
                        session_id: 0,
                        restartable: 0,
                    },
                ),
                _ => return Err(format!("CoreHolderQueryFailed:{status}")),
            }
        }
        Err("CoreHolderQueryFailed".into())
    })();
    unsafe { RmEndSession(session) };
    result
}

/// Refuse a core that another process is using, such as a server that is
/// still running from this folder. This process's own evidence handles on the
/// file do not count.
pub(super) fn assert_unused(file: &Path) -> Result<()> {
    let own = std::process::id();
    if holders(file)?.iter().all(|(pid, _)| *pid == own) { Ok(()) } else { Err("CoreInUse".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jar_with(path: &Path, entries: &[(&str, &[u8])]) {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = out.len() as u32;
            let crc = crc32fast::hash(data);
            let mut header = vec![0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            header.extend(crc.to_le_bytes());
            header.extend((data.len() as u32).to_le_bytes());
            header.extend((data.len() as u32).to_le_bytes());
            header.extend((name.len() as u16).to_le_bytes());
            header.extend(0u16.to_le_bytes());
            out.extend(&header);
            out.extend(name.as_bytes());
            out.extend(*data);
            let mut entry = vec![0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            entry.extend(crc.to_le_bytes());
            entry.extend((data.len() as u32).to_le_bytes());
            entry.extend((data.len() as u32).to_le_bytes());
            entry.extend((name.len() as u16).to_le_bytes());
            entry.extend([0u8; 12]);
            entry.extend(offset.to_le_bytes());
            entry.extend(name.as_bytes());
            central.extend(entry);
        }
        let start = out.len() as u32;
        out.extend(&central);
        out.extend([0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        fs::write(path, out).unwrap();
    }

    fn velocity(path: &Path, version: &str) {
        let manifest = format!(
            "Manifest-Version: 1.0\r\nImplementation-Version: {version} (git-0-b1)\r\nImplementation-Title: Velocity\r\n"
        );
        jar_with(path, &[("META-INF/MANIFEST.MF", manifest.as_bytes())]);
    }

    fn paper(path: &Path, family: &str, version: &str) {
        let list = format!("versions\ta\tb\tc\t{version}/server-{version}.jar\t{version}/server.patch\t{version}/{family}-{version}.jar\n");
        jar_with(path, &[("META-INF/patches.list", list.as_bytes())]);
    }

    fn folder() -> PathBuf {
        let path = std::env::temp_dir().join(format!("ih-core-artifact-{}-{}", std::process::id(), clock()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_core_states_its_family_and_version() {
        let dir = folder();
        velocity(&dir.join("velocity-4.2.1-SNAPSHOT-38.jar"), "4.2.1-SNAPSHOT");
        paper(&dir.join("paper.jar"), "paper", "26.3");
        paper(&dir.join("folia.jar"), "folia", "26.1.2");
        assert_eq!(claim(&dir.join("velocity-4.2.1-SNAPSHOT-38.jar")), Some(("VELOCITY", "4.2.1-SNAPSHOT".into())));
        assert_eq!(claim(&dir.join("paper.jar")), Some(("PAPER", "26.3".into())));
        assert_eq!(claim(&dir.join("folia.jar")), Some(("FOLIA", "26.1.2".into())));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_core_with_more_entries_than_a_plugin_still_states_its_family() {
        let dir = folder();
        let manifest = b"Implementation-Title: Velocity
Implementation-Version: 4.2.1-SNAPSHOT (git-0-b38)
";
        let names: Vec<String> = (0..6000).map(|i| format!("c/{i}.class")).collect();
        let mut entries: Vec<(&str, &[u8])> = names.iter().map(|name| (name.as_str(), &b"x"[..])).collect();
        entries.push(("META-INF/MANIFEST.MF", manifest));
        jar_with(&dir.join("velocity.jar"), &entries);
        assert_eq!(claim(&dir.join("velocity.jar")), Some(("VELOCITY", "4.2.1-SNAPSHOT".into())));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_core_is_found_under_any_file_name() {
        let dir = folder();
        velocity(&dir.join("velocity-4.3.0-50.jar"), "4.3.0");
        jar_with(&dir.join("library.jar"), &[("README", b"x")]);
        assert_eq!(locate(&dir, true).unwrap().as_deref(), Some("velocity-4.3.0-50.jar"));
        velocity(&dir.join("velocity-4.3.1-51.jar"), "4.3.1");
        assert_eq!(locate(&dir, true).unwrap_err(), "CoreArtifactAmbiguous");
        paper(&dir.join("server.jar"), "paper", "26.3");
        assert_eq!(locate(&dir, true).unwrap().as_deref(), Some("server.jar"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_single_unlabelled_jar_is_a_game_server_core_but_never_a_proxy() {
        let dir = folder();
        jar_with(&dir.join("spigot-1.8.8.jar"), &[("README", b"x")]);
        assert_eq!(locate(&dir, false).unwrap().as_deref(), Some("spigot-1.8.8.jar"));
        assert_eq!(locate(&dir, true).unwrap(), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_plain_jar_names_are_recorded() {
        assert!(valid_name("velocity-4.2.1-SNAPSHOT-38.jar"));
        assert!(valid_name("Paper.JAR"));
        for name in ["", ".jar", "a.zip", "..\\a.jar", "a/b.jar", "C:a.jar", "a\".jar", ".hidden.jar"] {
            assert!(!valid_name(name), "{name}");
        }
        assert_eq!(stored(DEFAULT), None);
        assert_eq!(stored("p.jar").as_deref(), Some("p.jar"));
    }

    #[test]
    fn a_file_held_open_by_another_process_is_reported_in_use() {
        let dir = folder();
        let file = dir.join("server.jar");
        fs::write(&file, b"core").unwrap();
        assert_unused(&file).unwrap();
        let script = format!(
            "$f=[IO.File]::Open('{}','Open','Read','ReadWrite'); Start-Sleep 20",
            file.display()
        );
        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .spawn()
            .unwrap();
        let mut seen = false;
        for _ in 0..100 {
            if holders(&file).unwrap().iter().any(|(pid, _)| *pid == child.id()) {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let refused = assert_unused(&file);
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(seen);
        assert_eq!(refused.unwrap_err(), "CoreInUse");
        fs::remove_dir_all(dir).unwrap();
    }
}
