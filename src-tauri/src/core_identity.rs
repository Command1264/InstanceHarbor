//! Read the installed core's own ZIP directory without starting Java or extracting files.
use super::*;
use std::{
    io::{Seek, SeekFrom},
    sync::{Mutex, OnceLock},
};

const MAX_DIRECTORY: u64 = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const MAX_TAIL: u64 = 22 + u16::MAX as u64;
const MAX_ENTRY_NAME: usize = 1024;
const PAPER_API_PREFIX: &[u8] = b"META-INF/libraries/io/papermc/paper/paper-api/";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CoreIdentity {
    pub(super) family: &'static str,
    pub(super) minecraft_version: String,
    pub(super) build: String,
    pub(super) version: String,
    /// SHA-256 of the ZIP directory, not of the executable JAR payload.
    pub(super) directory_revision: String,
}

struct CachedIdentity {
    path: PathBuf,
    file_length: u64,
    directory: Vec<u8>,
    identity: Option<CoreIdentity>,
}

struct PaperCandidate {
    minecraft_version: String,
    build: String,
    version: String,
    name: Vec<u8>,
    local_offset: u64,
    compressed_size: u64,
    flags: u16,
    method: u16,
}

static CACHE: OnceLock<Mutex<Option<CachedIdentity>>> = OnceLock::new();

/// Returns `None` for unknown or malformed cores. I/O failures remain errors.
/// Every call reads the current bounded ZIP directory, so a replacement with the
/// same length and modification time cannot reuse a stale version from the cache.
pub(super) fn read_identity(jar: &Path) -> Result<Option<CoreIdentity>> {
    if fs::symlink_metadata(jar)
        .map_err(fail)?
        .file_type()
        .is_symlink()
    {
        return Ok(None);
    }
    read_open_identity(jar, File::open(jar).map_err(fail)?)
}

pub(super) fn read_open_identity(jar: &Path, mut file: File) -> Result<Option<CoreIdentity>> {
    let file_length = file.metadata().map_err(fail)?.len();
    let Some((directory, directory_offset)) =
        read_directory(&mut file, file_length).map_err(fail)?
    else {
        return Ok(None);
    };
    let Some(candidate) = paper_coordinate(&directory) else {
        return Ok(None);
    };
    if !local_matches(&mut file, &candidate, directory_offset).map_err(fail)? {
        return Ok(None);
    }
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().map_err(|_| "CoreIdentityCacheUnavailable")?;
    if let Some(saved) = guard.as_ref() {
        if saved.path == jar && saved.file_length == file_length && saved.directory == directory {
            return Ok(saved.identity.clone());
        }
    }
    let identity = Some(CoreIdentity {
        family: "Paper",
        minecraft_version: candidate.minecraft_version,
        build: candidate.build,
        version: candidate.version,
        directory_revision: content_revision::revision(&directory)?,
    });
    *guard = Some(CachedIdentity {
        path: jar.to_path_buf(),
        file_length,
        directory,
        identity: identity.clone(),
    });
    Ok(identity)
}

pub(super) fn le16(bytes: &[u8], index: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(index..index + 2)?.try_into().ok()?,
    ))
}

pub(super) fn le32(bytes: &[u8], index: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(index..index + 4)?.try_into().ok()?,
    ))
}

pub(super) fn read_directory(
    file: &mut (impl Read + Seek),
    length: u64,
) -> io::Result<Option<(Vec<u8>, u64)>> {
    read_directory_with(file, length, MAX_ENTRIES)
}

/// A server core can hold far more entries than a plugin; a Velocity JAR has
/// about 25,000. The directory size limit still bounds the read.
pub(super) fn read_directory_with(
    file: &mut (impl Read + Seek),
    length: u64,
    max_entries: usize,
) -> io::Result<Option<(Vec<u8>, u64)>> {
    if length < 22 {
        return Ok(None);
    }
    let tail_length = length.min(MAX_TAIL) as usize;
    file.seek(SeekFrom::Start(length - tail_length as u64))?;
    let mut tail = vec![0; tail_length];
    file.read_exact(&mut tail)?;
    let Some(end) = (0..=tail.len() - 22).rev().find(|&i| {
        le32(&tail, i) == Some(0x0605_4b50)
            && le16(&tail, i + 20).is_some_and(|comment| i + 22 + comment as usize == tail.len())
    }) else {
        return Ok(None);
    };
    if le16(&tail, end + 4) != Some(0)
        || le16(&tail, end + 6) != Some(0)
        || le16(&tail, end + 8) != le16(&tail, end + 10)
    {
        return Ok(None);
    }
    let Some(count) = le16(&tail, end + 10).map(usize::from) else {
        return Ok(None);
    };
    let Some(size) = le32(&tail, end + 12).map(u64::from) else {
        return Ok(None);
    };
    let Some(offset) = le32(&tail, end + 16).map(u64::from) else {
        return Ok(None);
    };
    let end_offset = length - tail_length as u64 + end as u64;
    if count == 0
        || count > max_entries
        || size > MAX_DIRECTORY
        || offset.checked_add(size) != Some(end_offset)
    {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut directory = vec![0; size as usize];
    file.read_exact(&mut directory)?;
    if valid_directory(&directory, count) {
        Ok(Some((directory, offset)))
    } else {
        Ok(None)
    }
}

fn valid_directory(directory: &[u8], count: usize) -> bool {
    let mut at = 0usize;
    for _ in 0..count {
        if le32(directory, at) != Some(0x0201_4b50) {
            return false;
        }
        let Some(name) = le16(directory, at + 28).map(usize::from) else {
            return false;
        };
        let Some(extra) = le16(directory, at + 30).map(usize::from) else {
            return false;
        };
        let Some(comment) = le16(directory, at + 32).map(usize::from) else {
            return false;
        };
        let Some(next) = at
            .checked_add(46)
            .and_then(|v| v.checked_add(name))
            .and_then(|v| v.checked_add(extra))
            .and_then(|v| v.checked_add(comment))
        else {
            return false;
        };
        if next > directory.len() {
            return false;
        }
        at = next;
    }
    at == directory.len()
}

fn paper_coordinate(directory: &[u8]) -> Option<PaperCandidate> {
    let mut found = None;
    let mut at = 0;
    while at < directory.len() {
        let name_length = le16(directory, at + 28)? as usize;
        let extra_length = le16(directory, at + 30)? as usize;
        let comment_length = le16(directory, at + 32)? as usize;
        let name = directory.get(at + 46..at + 46 + name_length)?;
        if let Some(rest) = name.strip_prefix(PAPER_API_PREFIX) {
            if let Some((minecraft_version, build, version)) = parse_paper_path(rest) {
                if found.is_some() || name_length > MAX_ENTRY_NAME {
                    return None;
                }
                let local_offset = le32(directory, at + 42)?;
                let compressed_size = le32(directory, at + 20)?;
                if local_offset == u32::MAX || compressed_size == u32::MAX {
                    return None; // ZIP64 entries are outside this bounded reader.
                }
                found = Some(PaperCandidate {
                    minecraft_version,
                    build,
                    version,
                    name: name.to_vec(),
                    local_offset: u64::from(local_offset),
                    compressed_size: u64::from(compressed_size),
                    flags: le16(directory, at + 8)?,
                    method: le16(directory, at + 10)?,
                });
            }
        }
        at += 46 + name_length + extra_length + comment_length;
    }
    found
}

fn local_matches(
    file: &mut (impl Read + Seek),
    candidate: &PaperCandidate,
    directory_offset: u64,
) -> io::Result<bool> {
    let mut header = [0u8; 30];
    if candidate
        .local_offset
        .checked_add(header.len() as u64)
        .is_none_or(|end| end > directory_offset)
    {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(candidate.local_offset))?;
    file.read_exact(&mut header)?;
    if le32(&header, 0) != Some(0x0403_4b50)
        || le16(&header, 6) != Some(candidate.flags)
        || le16(&header, 8) != Some(candidate.method)
        || candidate.flags & 1 != 0
        || le16(&header, 26).map(usize::from) != Some(candidate.name.len())
    {
        return Ok(false);
    }
    let extra = u64::from(le16(&header, 28).unwrap_or(0));
    let data_start =
        candidate.local_offset + header.len() as u64 + candidate.name.len() as u64 + extra;
    if data_start
        .checked_add(candidate.compressed_size)
        .is_none_or(|end| end > directory_offset)
    {
        return Ok(false);
    }
    let mut name = vec![0; candidate.name.len()];
    file.read_exact(&mut name)?;
    Ok(name == candidate.name)
}

fn parse_paper_path(rest: &[u8]) -> Option<(String, String, String)> {
    let rest = std::str::from_utf8(rest).ok()?;
    let (coordinate, file) = rest.split_once('/')?;
    if file != format!("paper-api-{coordinate}.jar") {
        return None;
    }
    let coordinate = coordinate.strip_suffix("-stable")?;
    let (minecraft, build) = coordinate.split_once(".build.")?;
    if minecraft.split('.').count() < 2
        || minecraft
            .split('.')
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
        || build.is_empty()
        || !build.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((
        minecraft.into(),
        build.into(),
        format!("{minecraft}-{build}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn zip(names: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut entries = Vec::new();
        for name in names {
            entries.push((*name, bytes.len() as u32));
            let mut local = [0u8; 30];
            local[..4].copy_from_slice(&0x0403_4b50u32.to_le_bytes());
            local[4..6].copy_from_slice(&20u16.to_le_bytes());
            local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&local);
            bytes.extend_from_slice(name.as_bytes());
        }
        let offset = bytes.len() as u32;
        for (name, local_offset) in entries {
            let mut central = [0u8; 46];
            central[..4].copy_from_slice(&0x0201_4b50u32.to_le_bytes());
            central[4..6].copy_from_slice(&20u16.to_le_bytes());
            central[6..8].copy_from_slice(&20u16.to_le_bytes());
            central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
            central[42..46].copy_from_slice(&local_offset.to_le_bytes());
            bytes.extend_from_slice(&central);
            bytes.extend_from_slice(name.as_bytes());
        }
        let size = bytes.len() as u32 - offset;
        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&(names.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(names.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }

    fn directory(bytes: &[u8]) -> &[u8] {
        let end = bytes.len() - 22;
        let offset = le32(bytes, end + 16).unwrap() as usize;
        &bytes[offset..end]
    }

    fn paper(version: &str) -> String {
        format!("META-INF/libraries/io/papermc/paper/paper-api/{version}/paper-api-{version}.jar")
    }

    #[test]
    fn reads_paper_coordinate_and_rejects_ambiguity() {
        let dir = zip(&[&paper("26.3.build.205-stable")]);
        let candidate = paper_coordinate(directory(&dir)).unwrap();
        assert_eq!(
            (
                candidate.minecraft_version,
                candidate.build,
                candidate.version
            ),
            ("26.3".into(), "205".into(), "26.3-205".into())
        );
        let other = zip(&[
            &paper("26.3.build.205-stable"),
            &paper("26.3.build.206-stable"),
        ]);
        assert!(paper_coordinate(directory(&other)).is_none());
        assert_eq!(
            parse_paper_path(b"26.3.build.205-stable/not-the-api.jar"),
            None
        );
    }

    #[test]
    fn rejects_malformed_and_excessive_directories() {
        let mut malformed = zip(&[&paper("26.3.build.205-stable")]);
        let directory_start = le32(&malformed, malformed.len() - 22 + 16).unwrap() as usize;
        malformed[directory_start] = 0;
        let mut cursor = io::Cursor::new(malformed.clone());
        assert!(read_directory(&mut cursor, malformed.len() as u64)
            .unwrap()
            .is_none());
        let mut excessive = zip(&[&paper("26.3.build.205-stable")]);
        let end = excessive.len() - 22;
        excessive[end + 10..end + 12].copy_from_slice(&u16::MAX.to_le_bytes());
        let mut cursor = io::Cursor::new(excessive.clone());
        assert!(read_directory(&mut cursor, excessive.len() as u64)
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_missing_or_inconsistent_local_header() {
        let mut jar = zip(&[&paper("26.3.build.205-stable")]);
        let (dir, offset) = read_directory(&mut io::Cursor::new(&jar), jar.len() as u64)
            .unwrap()
            .unwrap();
        let candidate = paper_coordinate(&dir).unwrap();
        assert!(local_matches(&mut io::Cursor::new(&jar), &candidate, offset).unwrap());
        jar[0] = 0;
        assert!(!local_matches(&mut io::Cursor::new(&jar), &candidate, offset).unwrap());
        jar[0] = b'P';
        jar[30] = b'X';
        assert!(!local_matches(&mut io::Cursor::new(&jar), &candidate, offset).unwrap());
    }

    #[test]
    fn cache_refreshes_when_same_length_jar_is_replaced() {
        let path = std::env::temp_dir().join(format!(
            "instanceharbor-core-identity-{}-{}.jar",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, zip(&[&paper("26.3.build.205-stable")])).unwrap();
        let first = read_identity(&path).unwrap().unwrap();
        fs::write(&path, zip(&[&paper("26.3.build.206-stable")])).unwrap();
        let second = read_identity(&path).unwrap().unwrap();
        assert_eq!(first.version, "26.3-205");
        assert_eq!(second.version, "26.3-206");
        assert_ne!(first.directory_revision, second.directory_revision);
        fs::write(&path, b"not a jar").unwrap();
        assert!(read_identity(&path).unwrap().is_none());
        fs::remove_file(&path).unwrap();
        assert!(read_identity(&path).is_err());
    }
}
