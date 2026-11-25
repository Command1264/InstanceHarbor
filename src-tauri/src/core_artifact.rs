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

/// What a JAR states about itself. This is a claim only; `identify` decides
/// whether it is verified.
#[derive(Debug, PartialEq)]
pub(super) struct Claim {
    pub(super) family: String,
    pub(super) minecraft: String,
    pub(super) java: Option<u16>,
}

fn token(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 32
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')))
    .then(|| value.to_owned())
}

fn family_name(value: &str) -> Option<String> {
    (!value.is_empty() && value.len() <= 24 && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then(|| value.to_ascii_uppercase())
}

fn text(file: &mut File, name: &[u8]) -> Option<String> {
    String::from_utf8(plugin_descriptor::core_metadata(file, name)??).ok()
}

fn manifest_field(manifest: &str, key: &str) -> Option<String> {
    manifest
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':').map(str::trim))
        .map(str::to_owned)
}

/// The Java major the main class was compiled for: the lowest Java that can
/// load it. Class file major 52 is Java 8.
fn main_class_java(file: &mut File, manifest: &str) -> Option<u16> {
    let main = manifest_field(manifest, "Main-Class")?;
    if main.is_empty()
        || main.len() > 200
        || !main.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'$'))
    {
        return None;
    }
    let class = plugin_descriptor::core_metadata(file, format!("{}.class", main.replace('.', "/")).as_bytes())??;
    if class.get(..4)? != [0xca, 0xfe, 0xba, 0xbe] {
        return None;
    }
    let major = u16::from_be_bytes([*class.get(6)?, *class.get(7)?]);
    (major >= 49).then(|| major - 44)
}

/// Read the family, Minecraft version and Java requirement a core states,
/// without network access and without running it:
/// - Paperclip cores (Paper, Folia, Purpur and other forks): `META-INF/patches.list`
///   and `version.json`.
/// - Velocity: `MANIFEST.MF`.
/// - Spigot bundler JARs: `META-INF/versions.list`.
/// - Older Spigot or CraftBukkit: Maven `pom.properties`.
pub(super) fn claim(jar: &Path) -> Option<Claim> {
    claim_file(&mut File::open(jar).ok()?)
}

/// `claim` for a core that the caller already holds open.
pub(super) fn claim_file(file: &mut File) -> Option<Claim> {
    let manifest = text(file, b"META-INF/MANIFEST.MF").unwrap_or_default();
    let mojang = text(file, b"version.json").and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let stated_java = mojang
        .as_ref()
        .and_then(|value| value["java_version"].as_u64())
        .and_then(|value| u16::try_from(value).ok());
    let java = stated_java.or_else(|| main_class_java(file, &manifest));
    if let Some(list) = text(file, b"META-INF/patches.list") {
        let output = list.lines().find(|line| line.starts_with("versions\t"))?.rsplit('\t').next()?;
        let (version, file_name) = output.split_once('/')?;
        let family = family_name(file_name.strip_suffix(&format!("-{version}.jar"))?)?;
        let minecraft = mojang.as_ref().and_then(|value| value["id"].as_str()).unwrap_or(version);
        return Some(Claim { family, minecraft: token(minecraft)?, java });
    }
    if manifest_field(&manifest, "Implementation-Title").as_deref() == Some("Velocity") {
        let version = manifest_field(&manifest, "Implementation-Version")?;
        let version = token(version.split_whitespace().next()?)?;
        return Some(Claim { family: "VELOCITY".into(), minecraft: version, java });
    }
    if let Some(list) = text(file, b"META-INF/versions.list") {
        // For example `<sha256>\tspigot-26.2-R0.1-SNAPSHOT.jar`, or as BuildTools
        // writes it, `<sha256> *spigot-26.2-R0.1-SNAPSHOT.jar`.
        let last = list.lines().next()?.rsplit(['\t', ' ']).next()?;
        let name = last.strip_prefix('*').unwrap_or(last).strip_suffix(".jar")?;
        let mut parts = name.split('-');
        let family = family_name(parts.next()?)?;
        let minecraft = token(parts.next()?)?;
        return Some(Claim { family, minecraft, java });
    }
    for (pom, family) in [
        (&b"META-INF/maven/org.spigotmc/spigot/pom.properties"[..], "SPIGOT"),
        (&b"META-INF/maven/org.bukkit/craftbukkit/pom.properties"[..], "CRAFTBUKKIT"),
    ] {
        if let Some(properties) = text(file, pom) {
            let version = properties.lines().find_map(|line| line.strip_prefix("version="))?.trim();
            // `1.8.8-R0.1-SNAPSHOT` is Minecraft 1.8.8.
            let minecraft = token(version.split('-').next()?)?;
            return Some(Claim { family: family.into(), minecraft, java });
        }
    }
    None
}

/// The result of identifying a core. `warning` says why a core is accepted
/// without verification; a verified core has none.
pub(super) struct Identity {
    pub(super) profile: Option<CoreProfile>,
    pub(super) warning: Option<&'static str>,
}

enum Cached {
    Known(Option<CoreProfile>),
    Failed(Instant),
}

fn cache() -> &'static Mutex<HashMap<String, Cached>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn catalog(family: &str, minecraft: &str, sha256: &str) -> std::result::Result<Option<CoreProfile>, ()> {
    if let Ok(cache) = cache().lock() {
        match cache.get(sha256) {
            Some(Cached::Known(profile)) => return Ok(profile.clone()),
            Some(Cached::Failed(at)) if at.elapsed() < CATALOG_RETRY => return Err(()),
            _ => (),
        }
    }
    let result = core_source::catalog_profile(family, minecraft, sha256).map_err(|_| ());
    if let Ok(mut cache) = cache().lock() {
        let entry = match &result {
            Ok(profile) => Cached::Known(profile.clone()),
            Err(()) => Cached::Failed(Instant::now()),
        };
        cache.insert(sha256.to_owned(), entry);
    }
    result
}

/// Identify a core. The built-in table and an exact match in the official
/// PaperMC catalog give a verified profile. Otherwise a core that states its
/// family and Minecraft version is accepted as unverified with a warning, so
/// the program keeps working offline. A core that states nothing has no
/// profile; it needs Java chosen by hand.
pub(super) fn identify(jar: &Path) -> Result<Identity> {
    let sha256 = content_revision::file_revision(jar)?;
    if let Some(profile) = pinned_core_profile(sha256.clone()) {
        return Ok(Identity { profile: Some(profile), warning: None });
    }
    let Some(claim) = claim(jar) else {
        return Ok(Identity { profile: None, warning: Some("CoreUnrecognized") });
    };
    let unverified = |warning| Identity {
        profile: Some(CoreProfile {
            family: claim.family.clone(),
            minecraft: claim.minecraft.clone(),
            build: String::new(),
            sha256: sha256.clone(),
            unverified: true,
            java: claim.java,
        }),
        warning: Some(warning),
    };
    if !matches!(claim.family.as_str(), "PAPER" | "FOLIA" | "VELOCITY") {
        return Ok(unverified("CoreUnverified"));
    }
    Ok(match catalog(&claim.family, &claim.minecraft, &sha256) {
        Ok(Some(profile)) => Identity { profile: Some(profile), warning: None },
        Ok(None) => unverified("CoreNotInCatalog"),
        Err(()) => unverified("CoreCatalogUnavailable"),
    })
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
            claim(&directory.join(name)).is_some_and(|claim| (claim.family == "VELOCITY") == velocity)
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
            match identify(&core)?.profile {
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
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ih-core-artifact-{}-{}-{}",
            std::process::id(),
            clock(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        safe_directory(&path).unwrap()
    }

    fn claimed(family: &str, minecraft: &str, java: Option<u16>) -> Option<Claim> {
        Some(Claim { family: family.into(), minecraft: minecraft.into(), java })
    }

    /// A class file header for the given Java major (major + 44).
    fn class(java: u16) -> Vec<u8> {
        let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0];
        bytes.extend((java + 44).to_be_bytes());
        bytes
    }

    #[test]
    fn a_core_states_its_family_version_and_java_without_running() {
        let dir = folder();
        velocity(&dir.join("velocity-4.2.1-SNAPSHOT-38.jar"), "4.2.1-SNAPSHOT");
        paper(&dir.join("paper.jar"), "paper", "26.3");
        paper(&dir.join("folia.jar"), "folia", "26.1.2");
        assert_eq!(claim(&dir.join("velocity-4.2.1-SNAPSHOT-38.jar")), claimed("VELOCITY", "4.2.1-SNAPSHOT", None));
        assert_eq!(claim(&dir.join("paper.jar")), claimed("PAPER", "26.3", None));
        assert_eq!(claim(&dir.join("folia.jar")), claimed("FOLIA", "26.1.2", None));

        // A Paper fork names itself in patches.list; version.json states Java.
        let list = "versions\ta\tb\tc\t26.2/server-26.2.jar\t26.2/server.patch\t26.2/purpur-26.2.jar\n";
        jar_with(&dir.join("purpur.jar"), &[
            ("META-INF/patches.list", list.as_bytes()),
            ("version.json", br#"{"id":"26.2","java_version":25}"#),
        ]);
        assert_eq!(claim(&dir.join("purpur.jar")), claimed("PURPUR", "26.2", Some(25)));

        // A Spigot bundler lists its inner JAR; Java comes from the main class.
        let main = class(21);
        jar_with(&dir.join("spigot-bundler.jar"), &[
            ("META-INF/MANIFEST.MF", b"Main-Class: org.bukkit.craftbukkit.bootstrap.Main\r\n"),
            ("org/bukkit/craftbukkit/bootstrap/Main.class", &main),
            ("META-INF/versions.list", b"0c45\tspigot-26.2-R0.1-SNAPSHOT.jar\n"),
        ]);
        assert_eq!(claim(&dir.join("spigot-bundler.jar")), claimed("SPIGOT", "26.2", Some(21)));
        // BuildTools writes the list like `sha256sum`: `<hash> *<file>` with CRLF (#48).
        jar_with(&dir.join("spigot-buildtools.jar"), &[
            ("META-INF/MANIFEST.MF", b"Main-Class: org.bukkit.craftbukkit.bootstrap.Main\r\n"),
            ("org/bukkit/craftbukkit/bootstrap/Main.class", &main),
            ("META-INF/versions.list", b"0c45 *spigot-1.21.11-R0.2-SNAPSHOT.jar\r\n"),
        ]);
        assert_eq!(claim(&dir.join("spigot-buildtools.jar")), claimed("SPIGOT", "1.21.11", Some(21)));

        // Old Spigot keeps its Maven coordinates.
        let main = class(8);
        jar_with(&dir.join("spigot-1.8.8.jar"), &[
            ("META-INF/MANIFEST.MF", b"Main-Class: org.bukkit.craftbukkit.Main\r\n"),
            ("org/bukkit/craftbukkit/Main.class", &main),
            ("META-INF/maven/org.spigotmc/spigot/pom.properties", b"#Generated\nversion=1.8.8-R0.1-SNAPSHOT\ngroupId=org.spigotmc\n"),
        ]);
        assert_eq!(claim(&dir.join("spigot-1.8.8.jar")), claimed("SPIGOT", "1.8.8", Some(8)));

        // Nothing usable, or an unsafe name, states nothing.
        jar_with(&dir.join("plain.jar"), &[("README", b"x")]);
        assert_eq!(claim(&dir.join("plain.jar")), None);
        let list = "versions\ta\tb\tc\t26.2/x.jar\t26.2/x.patch\t26.2/../evil-26.2.jar\n";
        jar_with(&dir.join("evil.jar"), &[("META-INF/patches.list", list.as_bytes())]);
        assert_eq!(claim(&dir.join("evil.jar")), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unlisted_fork_is_accepted_unverified_with_java_from_its_version() {
        let dir = folder();
        let list = "versions\ta\tb\tc\t26.2/server-26.2.jar\t26.2/server.patch\t26.2/purpur-26.2.jar\n";
        jar_with(&dir.join("server.jar"), &[("META-INF/patches.list", list.as_bytes())]);
        let identity = identify(&dir.join("server.jar")).unwrap();
        assert_eq!(identity.warning, Some("CoreUnverified"));
        let profile = identity.profile.unwrap();
        assert!(profile.unverified);
        assert_eq!((profile.family.as_str(), profile.minecraft.as_str()), ("PURPUR", "26.2"));
        assert_eq!(global_java::supported_major(&profile), Some(25));

        jar_with(&dir.join("server.jar"), &[("README", b"x")]);
        let identity = identify(&dir.join("server.jar")).unwrap();
        assert!(identity.profile.is_none());
        assert_eq!(identity.warning, Some("CoreUnrecognized"));
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
        assert_eq!(claim(&dir.join("velocity.jar")), claimed("VELOCITY", "4.2.1-SNAPSHOT", None));
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
