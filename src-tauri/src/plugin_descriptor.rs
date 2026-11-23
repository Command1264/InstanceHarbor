//! Read only a bounded plugin identity from a Bukkit or Paper JAR.
use super::core_identity::{le16, le32, read_directory};
use flate2::{Decompress, FlushDecompress, Status};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

const MAX_JAR_BYTES: u64 = 128 * 1024 * 1024;
const MAX_DESCRIPTOR_BYTES: usize = 64 * 1024;
const MAX_COMPRESSED_BYTES: usize = 128 * 1024;
const MAX_NAME_BYTES: usize = 64;
const DATA_DESCRIPTOR: u16 = 1 << 3;
const UTF8_NAME: u16 = 1 << 11;

struct Entry<'a> {
    name: &'a [u8],
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u32,
    plain: u32,
    offset: u64,
}

pub(super) fn name(file: &mut File) -> Option<String> {
    let length = file.metadata().ok()?.len();
    archive_name(file, length)
}

/// Read one small ZIP metadata entry without extraction. A missing entry is
/// distinct from an invalid ZIP, duplicate entry or invalid local header.
pub(super) fn named_metadata(file: &mut File, wanted: &[u8]) -> Option<Option<Vec<u8>>> {
    let length = file.metadata().ok()?.len();
    archive_metadata(file, length, wanted)
}

fn archive_metadata(
    file: &mut (impl Read + Seek),
    length: u64,
    wanted: &[u8],
) -> Option<Option<Vec<u8>>> {
    let (directory, offset) = read_directory(file, length).ok()??;
    let mut at = 0;
    let mut found = None;
    while at < directory.len() {
        let name_length = usize::from(le16(&directory, at + 28)?);
        let extra_length = usize::from(le16(&directory, at + 30)?);
        let comment_length = usize::from(le16(&directory, at + 32)?);
        let name = directory.get(at + 46..at + 46 + name_length)?;
        if name == wanted {
            if found.is_some() || le16(&directory, at + 34)? != 0 {
                return None;
            }
            let entry = Entry {
                name,
                flags: le16(&directory, at + 8)?,
                method: le16(&directory, at + 10)?,
                crc: le32(&directory, at + 16)?,
                compressed: le32(&directory, at + 20)?,
                plain: le32(&directory, at + 24)?,
                offset: u64::from(le32(&directory, at + 42)?),
            };
            found = Some(read_entry(file, &entry, offset)?);
        }
        at += 46 + name_length + extra_length + comment_length;
    }
    Some(found)
}

fn archive_name(file: &mut (impl Read + Seek), length: u64) -> Option<String> {
    if length > MAX_JAR_BYTES {
        return None;
    }
    let (directory, offset) = read_directory(file, length).ok()??;
    let mut at = 0;
    let mut names = [None, None];
    while at < directory.len() {
        let name_length = usize::from(le16(&directory, at + 28)?);
        let extra_length = usize::from(le16(&directory, at + 30)?);
        let comment_length = usize::from(le16(&directory, at + 32)?);
        let name = directory.get(at + 46..at + 46 + name_length)?;
        let slot = match name {
            b"plugin.yml" => Some(0),
            b"paper-plugin.yml" => Some(1),
            _ => None,
        };
        if let Some(slot) = slot {
            if names[slot].is_some() || le16(&directory, at + 34)? != 0 {
                return None;
            }
            let entry = Entry {
                name,
                flags: le16(&directory, at + 8)?,
                method: le16(&directory, at + 10)?,
                crc: le32(&directory, at + 16)?,
                compressed: le32(&directory, at + 20)?,
                plain: le32(&directory, at + 24)?,
                offset: u64::from(le32(&directory, at + 42)?),
            };
            names[slot] = Some(descriptor_name(&read_entry(file, &entry, offset)?)?);
        }
        at += 46 + name_length + extra_length + comment_length;
    }
    match names {
        [Some(bukkit), Some(paper)] if bukkit == paper => Some(bukkit),
        [Some(name), None] | [None, Some(name)] => Some(name),
        _ => None,
    }
}

fn read_entry(
    file: &mut (impl Read + Seek),
    entry: &Entry<'_>,
    directory_offset: u64,
) -> Option<Vec<u8>> {
    let permitted_flags = match entry.method {
        0 => DATA_DESCRIPTOR | UTF8_NAME,
        8 => DATA_DESCRIPTOR | UTF8_NAME | 0b110,
        _ => return None,
    };
    if entry.flags & !permitted_flags != 0
        || entry.plain as usize > MAX_DESCRIPTOR_BYTES
        || entry.compressed as usize > MAX_COMPRESSED_BYTES
        || entry.offset + 30 > directory_offset
    {
        return None;
    }
    file.seek(SeekFrom::Start(entry.offset)).ok()?;
    let mut header = [0; 30];
    file.read_exact(&mut header).ok()?;
    if le32(&header, 0)? != 0x0403_4b50
        || le16(&header, 6)? != entry.flags
        || le16(&header, 8)? != entry.method
        || usize::from(le16(&header, 26)?) != entry.name.len()
    {
        return None;
    }
    let descriptor_follows = entry.flags & DATA_DESCRIPTOR != 0;
    for (offset, expected) in [(14, entry.crc), (18, entry.compressed), (22, entry.plain)] {
        let value = le32(&header, offset)?;
        if value != expected && !(descriptor_follows && value == 0) {
            return None;
        }
    }
    let start = entry.offset + 30 + entry.name.len() as u64 + u64::from(le16(&header, 28)?);
    let end = start + u64::from(entry.compressed);
    if end > directory_offset {
        return None;
    }
    let mut local_name = vec![0; entry.name.len()];
    file.read_exact(&mut local_name).ok()?;
    if local_name.as_slice() != entry.name {
        return None;
    }
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut data = vec![0; entry.compressed as usize];
    file.read_exact(&mut data).ok()?;
    if descriptor_follows && !matches_data_descriptor(file, entry, directory_offset - end) {
        return None;
    }

    let data = if entry.method == 8 {
        // A spare byte detects streams larger than the advertised plain length.
        let mut plain = vec![0; entry.plain as usize + 1];
        let mut inflater = Decompress::new(false);
        if inflater
            .decompress(&data, &mut plain, FlushDecompress::Finish)
            .ok()?
            != Status::StreamEnd
            || inflater.total_in() != u64::from(entry.compressed)
            || inflater.total_out() != u64::from(entry.plain)
        {
            return None;
        }
        plain.truncate(entry.plain as usize);
        plain
    } else {
        data
    };
    if data.len() == entry.plain as usize && crc32fast::hash(&data) == entry.crc {
        Some(data)
    } else {
        None
    }
}

fn matches_data_descriptor(file: &mut impl Read, entry: &Entry<'_>, available: u64) -> bool {
    let expected = [entry.crc, entry.compressed, entry.plain];
    let mut descriptor = [0; 12];
    if available < 12 || file.read_exact(&mut descriptor).is_err() {
        return false;
    }
    let matches = |descriptor: &[u8]| {
        expected
            .iter()
            .enumerate()
            .all(|(index, &field)| le32(descriptor, index * 4) == Some(field))
    };
    if matches(&descriptor) {
        return true;
    }
    // A signature before the three fields is optional.
    if available < 16 || le32(&descriptor, 0) != Some(0x0807_4b50) {
        return false;
    }
    descriptor.copy_within(4..12, 0);
    file.read_exact(&mut descriptor[8..]).is_ok() && matches(&descriptor)
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NAME_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn unquote(value: &str) -> Option<&str> {
    match value.as_bytes().first() {
        Some(b'\'') => value.strip_prefix('\'')?.strip_suffix('\''),
        Some(b'"') => value.strip_prefix('"')?.strip_suffix('"'),
        _ => Some(value),
    }
}

fn descriptor_name(bytes: &[u8]) -> Option<String> {
    let source = std::str::from_utf8(bytes).ok()?;
    let mut name = None;
    let mut root_started = false;
    let mut has_start_marker = false;
    let mut ended = false;
    let mut after_name = false;
    let mut allows_indentless_sequence = false;
    for line in source.strip_prefix('\u{feff}').unwrap_or(source).lines() {
        if line.contains(['\r', '\0', '\u{85}', '\u{2028}', '\u{2029}', '\u{feff}']) {
            return None;
        }
        let visible = line.trim();
        if visible.is_empty() || visible.starts_with('#') {
            continue;
        }
        if ended {
            return None;
        }
        if line.starts_with([' ', '\t']) {
            if !root_started || after_name {
                return None;
            }
            continue;
        }
        if visible == "---" && !root_started && !has_start_marker {
            has_start_marker = true;
            continue;
        }
        if visible == "..." && root_started {
            ended = true;
            continue;
        }
        if allows_indentless_sequence && line.starts_with("- ") {
            continue; // YAML allows sequences indented at the same depth as their mapping key.
        }
        let (key, raw) = line.split_once(':')?;
        let key = unquote(key.trim_end())?;
        if !identifier(key) || (!raw.is_empty() && !raw.starts_with([' ', '\t'])) {
            return None;
        }
        root_started = true;
        after_name = key == "name";
        allows_indentless_sequence = raw.trim().is_empty() || raw.trim_start().starts_with('#');
        if key != "name" {
            continue;
        }
        let value = raw.trim_start();
        let value = value
            .split_once(" #")
            .or_else(|| value.split_once("\t#"))
            .map_or(value, |(value, _)| value)
            .trim_end();
        let quoted = value.starts_with(['\'', '"']);
        let value = unquote(value)?;
        let plain_string = value.starts_with(|character: char| character.is_ascii_alphabetic())
            && !["true", "false", "null", "yes", "no", "on", "off", "y", "n"]
                .iter()
                .any(|keyword| value.eq_ignore_ascii_case(keyword));
        if name.is_some() || !identifier(value) || (!quoted && !plain_string) {
            return None;
        }
        name = Some(value.to_owned());
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::DeflateEncoder};
    use std::io::{Cursor, Write};

    fn write_u16(bytes: &mut [u8], index: usize, value: u16) {
        bytes[index..index + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(bytes: &mut [u8], index: usize, value: u32) {
        bytes[index..index + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn jar(entries: &[(&str, &str)], deflate: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut directory = Vec::new();
        for (name, descriptor) in entries {
            let local_offset = bytes.len() as u32;
            let (method, flags, data) = if deflate {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(descriptor.as_bytes()).unwrap();
                (8, DATA_DESCRIPTOR | UTF8_NAME, encoder.finish().unwrap())
            } else {
                (0, 0, descriptor.as_bytes().to_vec())
            };
            let fields = [
                crc32fast::hash(descriptor.as_bytes()),
                data.len() as u32,
                descriptor.len() as u32,
            ];

            let mut local = [0; 30];
            write_u32(&mut local, 0, 0x0403_4b50);
            write_u16(&mut local, 6, flags);
            write_u16(&mut local, 8, method);
            write_u16(&mut local, 26, name.len() as u16);
            if !deflate {
                for (i, field) in fields.into_iter().enumerate() {
                    write_u32(&mut local, 14 + i * 4, field);
                }
            }
            bytes.extend_from_slice(&local);
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(&data);
            if deflate {
                bytes.extend_from_slice(&0x0807_4b50u32.to_le_bytes());
                for field in fields {
                    bytes.extend_from_slice(&field.to_le_bytes());
                }
            }

            let mut central = [0; 46];
            write_u32(&mut central, 0, 0x0201_4b50);
            write_u16(&mut central, 8, flags);
            write_u16(&mut central, 10, method);
            write_u16(&mut central, 28, name.len() as u16);
            write_u32(&mut central, 42, local_offset);
            for (i, field) in fields.into_iter().enumerate() {
                write_u32(&mut central, 16 + i * 4, field);
            }
            directory.extend_from_slice(&central);
            directory.extend_from_slice(name.as_bytes());
        }

        let mut end = [0; 22];
        write_u32(&mut end, 0, 0x0605_4b50);
        write_u16(&mut end, 8, entries.len() as u16);
        write_u16(&mut end, 10, entries.len() as u16);
        write_u32(&mut end, 12, directory.len() as u32);
        write_u32(&mut end, 16, bytes.len() as u32);
        bytes.extend_from_slice(&directory);
        bytes.extend_from_slice(&end);
        bytes
    }

    fn read_fixture(bytes: &[u8]) -> Option<String> {
        archive_name(&mut Cursor::new(bytes), bytes.len() as u64)
    }

    #[test]
    fn named_zip_metadata_distinguishes_missing_duplicate_and_corrupt_entries() {
        let name = "META-INF/download-context";
        let read = |bytes: &[u8]| {
            archive_metadata(&mut Cursor::new(bytes), bytes.len() as u64, name.as_bytes())
        };
        for deflate in [false, true] {
            assert_eq!(read(&jar(&[(name, "context")], deflate)), Some(Some(b"context".to_vec())));
            assert_eq!(read(&jar(&[("version.json", "context")], deflate)), Some(None));
            assert_eq!(read(&jar(&[(name, "one"), (name, "two")], deflate)), None);
        }
        let mut corrupt = jar(&[(name, "context")], false);
        corrupt[30 + name.len()] = 0;
        assert_eq!(read(&corrupt), None);
    }

    #[test]
    fn reads_bukkit_and_paper_names_from_stored_and_deflated_descriptors() {
        for deflate in [false, true] {
            let bytes = jar(
                &[
                    ("plugin.yml", "name: LuckPerms\r\nversion: 5.5.85\r\n"),
                    ("paper-plugin.yml", "---\n'name': \"LuckPerms\" # note\n"),
                    ("nested/plugin.yml", "name: IgnoreMe\n"),
                ],
                deflate,
            );
            assert_eq!(read_fixture(&bytes).as_deref(), Some("LuckPerms"));
            let bytes = jar(&[("paper-plugin.yml", "name: PaperOnly\n")], deflate);
            assert_eq!(read_fixture(&bytes).as_deref(), Some("PaperOnly"));
        }
    }

    #[test]
    fn rejects_split_identity_and_untrustworthy_names() {
        for entries in [
            [
                ("plugin.yml", "name: One"),
                ("paper-plugin.yml", "name: Two"),
            ],
            [("plugin.yml", "name: Same"), ("plugin.yml", "name: Same")],
        ] {
            assert!(read_fixture(&jar(&entries, false)).is_none());
        }
        for yaml in [
            "name: One\n\"name\": Two",
            "name: One\n  Continued",
            "? name\n: One",
            "description: sample\n  name: Nested",
            "name: On",
            "name: >\n  Folded",
            "name: One\n---\nname: Two",
        ] {
            assert!(descriptor_name(yaml.as_bytes()).is_none(), "{yaml}");
        }
        assert!(read_fixture(&jar(&[("nested/plugin.yml", "name: One")], false)).is_none());
    }

    #[test]
    fn rejects_corrupt_zip_payload_and_oversized_descriptor() {
        let filename = "plugin.yml";
        let mut bytes = jar(&[(filename, "name: Example")], false);
        bytes[30 + filename.len() + 6] = b'X';
        assert!(read_fixture(&bytes).is_none());
        let mut bytes = jar(&[(filename, "name: Example")], true);
        let central = le32(&bytes, bytes.len() - 6).unwrap() as usize;
        bytes[central - 1] = 1;
        assert!(read_fixture(&bytes).is_none());
        let huge = format!("name: Example\n#{}", "x".repeat(MAX_DESCRIPTOR_BYTES));
        assert!(read_fixture(&jar(&[(filename, &huge)], true)).is_none());
    }
}
