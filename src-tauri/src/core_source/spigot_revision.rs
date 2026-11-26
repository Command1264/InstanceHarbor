//! Read which Spigot and CraftBukkit commits a BuildTools JAR was built from (#75).
//!
//! Older JARs state `Implementation-Version: git-Spigot-<Spigot>-<CraftBukkit>` in their
//! own manifest; 1.17 adds the Jenkins build in front, for example `3284a-Spigot-…`.
//! Bundler JARs (1.18 and later) keep the server JAR in `META-INF/versions/`, and its
//! manifest states `<build>-Spigot-<Spigot>-<CraftBukkit>`. Only the first entries of
//! that inner JAR are decompressed: its manifest is one of them, so the 20+ MB inner
//! JAR is never read in full.
use super::*;
use crate::windows::core_identity::{le16, le32, read_directory_with};
use flate2::bufread::DeflateDecoder;
use std::io::{BufRead, BufReader, Seek, SeekFrom};

pub(super) const UNREADABLE: &str = "CoreSourceSpigotRevisionUnreadable";
const MANIFEST: &[u8] = b"META-INF/MANIFEST.MF";
const MAX_MANIFEST: u64 = 64 * 1024;
const MAX_LEADING_ENTRIES: usize = 8;
const MAX_ENTRY_NAME: usize = 1024;
const MAX_INNER_JAR: u64 = 512 * 1024 * 1024;
const LOCAL_HEADER: u32 = 0x0403_4b50;
const DATA_DESCRIPTOR: u32 = 0x0807_4b50;

/// The Jenkins build (absent in `git-Spigot-…`) and the first seven digits of each commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SpigotRevision {
    pub(super) build: Option<String>,
    pub(super) spigot: String,
    pub(super) craftbukkit: String,
}

pub(super) fn read(jar: &Path) -> Result<SpigotRevision> {
    let mut file = File::open(jar).map_err(fail)?;
    read_file(&mut file).ok_or_else(|| UNREADABLE.into())
}

fn read_file(file: &mut File) -> Option<SpigotRevision> {
    if let Some(revision) = plugin_descriptor::core_metadata(file, MANIFEST)?
        .as_deref()
        .and_then(manifest_revision)
    {
        return Some(revision);
    }
    let list = plugin_descriptor::core_metadata(file, b"META-INF/versions.list")??;
    let inner = bundled_jar(&list)?;
    manifest_revision(&inner_manifest(file, inner.as_bytes())?)
}

fn manifest_revision(manifest: &[u8]) -> Option<SpigotRevision> {
    std::str::from_utf8(manifest)
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Implementation-Version:"))
        .and_then(|value| parse_version(value.trim()))
}

/// A Jenkins build name: digits with at most one letter suffix, as in `1573k`.
pub(super) fn build_name(value: &str) -> bool {
    let number = value
        .strip_suffix(|character: char| character.is_ascii_alphabetic())
        .unwrap_or(value);
    (1..=12).contains(&number.len()) && number.bytes().all(|byte| byte.is_ascii_digit())
}

pub(super) fn parse_version(value: &str) -> Option<SpigotRevision> {
    let (build, commits) = value.split_once("-Spigot-")?;
    let (spigot, craftbukkit) = commits.split_once('-')?;
    let commit = |text: &str| {
        (text.len() == 7 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| text.to_ascii_lowercase())
    };
    let build = match build {
        "git" => None,
        name if build_name(name) => Some(name.to_owned()),
        _ => return None,
    };
    Some(SpigotRevision {
        build,
        spigot: commit(spigot)?,
        craftbukkit: commit(craftbukkit)?,
    })
}

/// `META-INF/versions/<file>` from the first line of `versions.list`, which BuildTools
/// writes as `<sha256> *spigot-26.3-R0.1-SNAPSHOT.jar`.
fn bundled_jar(list: &[u8]) -> Option<String> {
    let line = std::str::from_utf8(list).ok()?.lines().next()?;
    let last = line.rsplit(['\t', ' ']).next()?;
    let name = last.strip_prefix('*').unwrap_or(last);
    (name.len() <= 128
        && name.starts_with("spigot-")
        && name.ends_with(".jar")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')))
    .then(|| format!("META-INF/versions/{name}"))
}

fn inner_manifest(file: &mut File, name: &[u8]) -> Option<Vec<u8>> {
    let length = file.metadata().ok()?.len();
    let (directory, directory_offset) =
        read_directory_with(file, length, usize::from(u16::MAX)).ok()??;
    let mut found = None;
    let mut at = 0;
    while at < directory.len() {
        let name_length = usize::from(le16(&directory, at + 28)?);
        let extra_length = usize::from(le16(&directory, at + 30)?);
        let comment_length = usize::from(le16(&directory, at + 32)?);
        if directory.get(at + 46..at + 46 + name_length)? == name {
            if found.is_some() {
                return None;
            }
            found = Some((
                le16(&directory, at + 8)?,
                le16(&directory, at + 10)?,
                le32(&directory, at + 20)?,
                le32(&directory, at + 24)?,
                le32(&directory, at + 42)?,
            ));
        }
        at += 46 + name_length + extra_length + comment_length;
    }
    let (flags, method, compressed, plain, offset) = found?;
    // Encrypted and ZIP64 entries are outside this bounded reader.
    if flags & 1 != 0
        || [compressed, plain, offset].contains(&u32::MAX)
        || u64::from(plain) > MAX_INNER_JAR
    {
        return None;
    }
    file.seek(SeekFrom::Start(offset.into())).ok()?;
    let mut header = [0u8; 30];
    file.read_exact(&mut header).ok()?;
    if le32(&header, 0)? != LOCAL_HEADER
        || le16(&header, 8)? != method
        || usize::from(le16(&header, 26)?) != name.len()
    {
        return None;
    }
    let mut local_name = vec![0; name.len()];
    file.read_exact(&mut local_name).ok()?;
    let start = u64::from(offset) + 30 + name.len() as u64 + u64::from(le16(&header, 28)?);
    if local_name != name || start.checked_add(compressed.into())? > directory_offset {
        return None;
    }
    file.seek(SeekFrom::Start(start)).ok()?;
    let data = file.take(compressed.into());
    match method {
        0 => leading_manifest(BufReader::new(data)),
        8 => leading_manifest(BufReader::new(flate2::read::DeflateDecoder::new(data))),
        _ => None,
    }
}

/// Walk the first local headers of a JAR stream until its manifest. Each entry is
/// checked against its CRC and sizes; a stored entry needs its sizes in the header.
fn leading_manifest(mut stream: impl BufRead) -> Option<Vec<u8>> {
    for _ in 0..MAX_LEADING_ENTRIES {
        let mut header = [0u8; 30];
        stream.read_exact(&mut header).ok()?;
        let flags = le16(&header, 6)?;
        let method = le16(&header, 8)?;
        let name_length = usize::from(le16(&header, 26)?);
        if le32(&header, 0)? != LOCAL_HEADER || flags & 1 != 0 || name_length > MAX_ENTRY_NAME {
            return None;
        }
        let mut name = vec![0; name_length];
        stream.read_exact(&mut name).ok()?;
        let extra = u64::from(le16(&header, 28)?);
        if io::copy(&mut (&mut stream).take(extra), &mut io::sink()).ok()? != extra {
            return None;
        }
        let follows = flags & 0x8 != 0;
        let (data, consumed) = match method {
            0 if !follows => {
                let length = le32(&header, 22)?;
                if u64::from(length) > MAX_MANIFEST {
                    return None;
                }
                let mut data = vec![0; length as usize];
                stream.read_exact(&mut data).ok()?;
                (data, u64::from(length))
            }
            8 => {
                let mut data = Vec::new();
                let mut decoder = DeflateDecoder::new(&mut stream);
                (&mut decoder).take(MAX_MANIFEST + 1).read_to_end(&mut data).ok()?;
                if data.len() as u64 > MAX_MANIFEST {
                    return None;
                }
                (data, decoder.total_in())
            }
            _ => return None,
        };
        let mut sizes = [le32(&header, 14)?, le32(&header, 18)?, le32(&header, 22)?];
        if follows {
            let mut descriptor = [0u8; 12];
            stream.read_exact(&mut descriptor).ok()?;
            if le32(&descriptor, 0) == Some(DATA_DESCRIPTOR) {
                descriptor.copy_within(4.., 0);
                stream.read_exact(&mut descriptor[8..]).ok()?;
            }
            sizes = [le32(&descriptor, 0)?, le32(&descriptor, 4)?, le32(&descriptor, 8)?];
        }
        if sizes
            != [
                crc32fast::hash(&data),
                u32::try_from(consumed).ok()?,
                u32::try_from(data.len()).ok()?,
            ]
        {
            return None;
        }
        if name == MANIFEST {
            return Some(data);
        }
    }
    None
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use flate2::{write::DeflateEncoder, Compression};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// A ZIP whose entries are deflated with a data descriptor, as `jar` writes them,
    /// or stored with their sizes in the local header.
    pub(in crate::windows) fn zip(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let crc = crc32fast::hash(data);
            let packed = if deflate {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(data).unwrap();
                encoder.finish().unwrap()
            } else {
                data.to_vec()
            };
            let (flags, method) = if deflate { (0x8u16, 8u16) } else { (0, 0) };
            let fields = [crc, packed.len() as u32, data.len() as u32];
            let offset = bytes.len() as u32;
            bytes.extend_from_slice(&LOCAL_HEADER.to_le_bytes());
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&flags.to_le_bytes());
            bytes.extend_from_slice(&method.to_le_bytes());
            bytes.extend_from_slice(&[0; 4]);
            for field in if deflate { [0; 3] } else { fields } {
                bytes.extend_from_slice(&field.to_le_bytes());
            }
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(&packed);
            if deflate {
                bytes.extend_from_slice(&DATA_DESCRIPTOR.to_le_bytes());
                for field in fields {
                    bytes.extend_from_slice(&field.to_le_bytes());
                }
            }
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&flags.to_le_bytes());
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            for field in fields {
                central.extend_from_slice(&field.to_le_bytes());
            }
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let offset = bytes.len() as u32;
        bytes.extend_from_slice(&central);
        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(central.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }

    pub(in crate::windows) fn manifest(version: &str) -> Vec<u8> {
        format!("Manifest-Version: 1.0\r\nImplementation-Version: {version}\r\nSpecification-Version: unknown\r\n")
            .into_bytes()
    }

    /// An old JAR: the revision is in its own manifest, next to its Maven properties.
    pub(in crate::windows) fn legacy_jar(version: &str) -> Vec<u8> {
        zip(
            &[
                ("META-INF/MANIFEST.MF", &manifest(version)),
                (
                    "META-INF/maven/org.spigotmc/spigot/pom.properties",
                    b"version=1.12.2-R0.1-SNAPSHOT\n",
                ),
            ],
            true,
        )
    }

    /// A bundler JAR: the outer manifest has no revision; the inner JAR's does.
    pub(in crate::windows) fn bundler_jar(minecraft: &str, version: &str) -> Vec<u8> {
        let file = format!("spigot-{minecraft}-R0.1-SNAPSHOT.jar");
        let inner = zip(
            &[
                ("META-INF/", b""),
                ("META-INF/MANIFEST.MF", &manifest(version)),
                ("org/bukkit/craftbukkit/Main.class", &[0xca, 0xfe, 0xba, 0xbe]),
            ],
            true,
        );
        let list = format!("{} *{file}\r\n", "0".repeat(64));
        zip(
            &[
                ("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\r\nMain-Class: org.bukkit.craftbukkit.bootstrap.Main\r\n"),
                ("META-INF/versions.list", list.as_bytes()),
                (&format!("META-INF/versions/{file}"), &inner),
            ],
            true,
        )
    }

    pub(in crate::windows) fn temporary(bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "instanceharbor-spigot-revision-{}-{}.jar",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, bytes).unwrap();
        path
    }

    fn read_bytes(bytes: &[u8]) -> Result<SpigotRevision> {
        let path = temporary(bytes);
        let revision = read(&path);
        fs::remove_file(&path).unwrap();
        revision
    }

    fn revision(build: Option<&str>, spigot: &str, craftbukkit: &str) -> SpigotRevision {
        SpigotRevision {
            build: build.map(str::to_owned),
            spigot: spigot.into(),
            craftbukkit: craftbukkit.into(),
        }
    }

    #[test]
    fn reads_both_implementation_version_formats() {
        assert_eq!(
            parse_version("git-Spigot-79a30d7-f4830a1"),
            Some(revision(None, "79a30d7", "f4830a1"))
        );
        assert_eq!(
            parse_version("4632-Spigot-566f972-690a402"),
            Some(revision(Some("4632"), "566f972", "690a402"))
        );
        assert_eq!(
            parse_version("3284a-Spigot-3892929-0AB8487"),
            Some(revision(Some("3284a"), "3892929", "0ab8487"))
        );
        for invalid in [
            "",
            "git-Paper-79a30d7-f4830a1",
            "git-Spigot-79a30d-f4830a1",
            "git-Spigot-79a30d7",
            "git-Spigot-79a30d7-f4830a1-extra",
            "x1-Spigot-79a30d7-f4830a1",
            "4.2.1-SNAPSHOT (git-0-b38)",
        ] {
            assert_eq!(parse_version(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn reads_the_revision_of_legacy_and_bundler_jars() {
        assert_eq!(
            read_bytes(&legacy_jar("git-Spigot-79a30d7-f4830a1")).unwrap(),
            revision(None, "79a30d7", "f4830a1")
        );
        assert_eq!(
            read_bytes(&bundler_jar("26.3", "4664-Spigot-bed90fc-e89124d")).unwrap(),
            revision(Some("4664"), "bed90fc", "e89124d")
        );
        // A stored inner JAR is read the same way.
        let inner = zip(&[("META-INF/MANIFEST.MF", &manifest("4648-Spigot-8db49a2-96ae8cd"))], false);
        let stored = zip(
            &[
                ("META-INF/versions.list", b"0\tspigot-26.2-R0.1-SNAPSHOT.jar\n"),
                ("META-INF/versions/spigot-26.2-R0.1-SNAPSHOT.jar", &inner),
            ],
            false,
        );
        assert_eq!(read_bytes(&stored).unwrap(), revision(Some("4648"), "8db49a2", "96ae8cd"));
    }

    #[test]
    fn an_unreadable_revision_is_reported_and_never_guessed() {
        let unreadable = |bytes: &[u8]| read_bytes(bytes).unwrap_err() == UNREADABLE;
        assert!(unreadable(&legacy_jar("unknown")));
        assert!(unreadable(b"not a jar"));
        // The list names a file the JAR does not hold.
        assert!(unreadable(&zip(
            &[("META-INF/versions.list", b"0 *spigot-26.3-R0.1-SNAPSHOT.jar\n")],
            true
        )));
        // The list names something other than a Spigot JAR.
        let inner = zip(&[("META-INF/MANIFEST.MF", &manifest("4664-Spigot-bed90fc-e89124d"))], true);
        assert!(unreadable(&zip(
            &[
                ("META-INF/versions.list", b"0 *paper-26.3.jar\n"),
                ("META-INF/versions/paper-26.3.jar", &inner),
            ],
            true
        )));
        // A damaged inner manifest fails its CRC instead of being read.
        let mut inner = zip(
            &[("META-INF/", b""), ("META-INF/MANIFEST.MF", &manifest("4664-Spigot-bed90fc-e89124d"))],
            true,
        );
        let descriptor = DATA_DESCRIPTOR.to_le_bytes();
        let manifest_descriptor = inner
            .windows(4)
            .enumerate()
            .filter(|(_, window)| *window == descriptor)
            .nth(1)
            .unwrap()
            .0;
        inner[manifest_descriptor + 4] ^= 0xff;
        let damaged = zip(
            &[
                ("META-INF/versions.list", b"0 *spigot-26.3-R0.1-SNAPSHOT.jar\n"),
                ("META-INF/versions/spigot-26.3-R0.1-SNAPSHOT.jar", &inner),
            ],
            true,
        );
        assert!(unreadable(&damaged));
        assert!(matches!(read(Path::new("Z:/missing/spigot.jar")), Err(error) if error != UNREADABLE));
    }
}
