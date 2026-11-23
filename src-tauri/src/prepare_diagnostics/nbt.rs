//! Bounded reads of the saved overworld. No game or external package is required.
use super::*;
use std::ffi::OsString;
use std::os::windows::{ffi::OsStringExt, fs::OpenOptionsExt};

const MAX_GZIP: u64 = 8 * 1024 * 1024;
const MAX_NBT: u64 = 8 * 1024 * 1024;
const DECOMPRESS: &str = r#"
$ErrorActionPreference='Stop'
$zip=[IO.Compression.GZipStream]::new([Console]::OpenStandardInput(),[IO.Compression.CompressionMode]::Decompress)
$output=[Console]::OpenStandardOutput()
$buffer=[byte[]]::new(8192)
$total=0
try {
    while (($read=$zip.Read($buffer,0,$buffer.Length)) -gt 0) {
        $total+=$read
        if ($total -gt __MAX_NBT__) { exit 2 }
        $output.Write($buffer,0,$read)
    }
    $output.Flush()
} finally {
    $zip.Dispose()
}
"#;

fn system_directory() -> Result<PathBuf> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }
    let mut buffer = [0u16; 32768];
    let count = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if count == 0 || count >= buffer.len() {
        return Err("WorldSystemDirectoryUnavailable".into());
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..count])))
}

fn inflate(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.len() < 18 || !bytes.starts_with(&[0x1f, 0x8b, 8]) {
        return Err("WorldGzipInvalid".into());
    }
    let trailer = &bytes[bytes.len() - 8..];
    let expected_crc = u32::from_le_bytes(trailer[..4].try_into().unwrap());
    let expected_size = u32::from_le_bytes(trailer[4..].try_into().unwrap()) as u64;
    if expected_size > MAX_NBT {
        return Err("WorldGzipTooLarge".into());
    }

    let system = system_directory()?;
    let program = system.join("WindowsPowerShell/v1.0/powershell.exe");
    let command = DECOMPRESS.replace("__MAX_NBT__", &MAX_NBT.to_string());
    let mut child = Command::new(program)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &command,
        ])
        .env_clear()
        .env(
            "SystemRoot",
            system.parent().ok_or("WorldSystemDirectoryUnavailable")?,
        )
        .current_dir(&system)
        .creation_flags(HIDDEN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "WorldGzipUnavailable")?;
    // The child receives bytes only; it has no instance path or inherited user profile.
    let mut input = child.stdin.take().expect("piped decompressor stdin");
    let output = child.stdout.take().expect("piped decompressor stdout");
    let writer = thread::spawn(move || input.write_all(&bytes));
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        output
            .take(MAX_NBT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let finished = loop {
        match child.try_wait() {
            Ok(Some(exit)) => break exit.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                if child.kill().is_err() && child.try_wait().ok().flatten().is_none() {
                    return Err("WorldGzipTimeout".into());
                }
                child.wait().map_err(fail)?;
                break false;
            }
        }
    };
    let written = writer.join().map_err(|_| "WorldGzipWriteFailed")?;
    let output = reader
        .join()
        .map_err(|_| "WorldGzipReadFailed")?
        .map_err(fail)?;
    if !finished
        || written.is_err()
        || output.len() as u64 != expected_size
        || crc32(&output) != expected_crc
    {
        return Err("WorldGzipInvalid".into());
    }
    Ok(output)
}

fn crc32(bytes: &[u8]) -> u32 {
    const fn table() -> [u32; 256] {
        let mut values = [0; 256];
        let mut index = 0;
        while index < 256 {
            let mut entry = index as u32;
            let mut step = 0;
            while step < 8 {
                entry = (entry >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(entry & 1));
                step += 1;
            }
            values[index] = entry;
            index += 1;
        }
        values
    }
    const TABLE: [u32; 256] = table();
    !bytes.iter().fold(!0u32, |crc, byte| {
        (crc >> 8) ^ TABLE[((crc as u8) ^ byte) as usize]
    })
}

fn locked_bytes(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    no_redirect(path.parent().ok_or("WorldFileUnavailable")?)?;
    let file = match OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x0020_0000)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("WorldFileUnavailable".into()),
    };
    no_redirect(path)?;
    let metadata = file.metadata().map_err(fail)?;
    use std::os::windows::fs::MetadataExt;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 || metadata.len() > limit {
        return Err("WorldFileUnavailable".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(fail)?;
    if bytes.len() as u64 != metadata.len() {
        return Err("WorldFileChanged".into());
    }
    Ok(Some(bytes))
}

fn saved_file(path: &Path) -> Option<Vec<u8>> {
    locked_bytes(path, MAX_GZIP)
        .ok()
        .flatten()
        .and_then(|bytes| inflate(bytes).ok())
}

fn world_directory(instance: &Path) -> Option<PathBuf> {
    let bytes = locked_bytes(&instance.join("server.properties"), 128 * 1024).ok()??;
    let text = std::str::from_utf8(&bytes).ok()?;
    let mut world = None;
    for line in text.lines().map(str::trim_start) {
        if line.is_empty() || line.starts_with(['#', '!']) {
            continue;
        }
        if line.ends_with('\\') {
            return None;
        }
        let index = line
            .find(|character: char| matches!(character, '=' | ':' | ' ' | '\t'))
            .unwrap_or(line.len());
        let key = &line[..index];
        if key.contains('\\') {
            return None;
        }
        if key == "level-name" {
            let value = line[index..]
                .trim_start()
                .trim_start_matches(['=', ':'])
                .trim_start();
            if !token(value) || world.replace(value).is_some() {
                return None;
            }
        }
    }
    child_directory(instance, world.unwrap_or("world")).ok()
}

#[derive(Debug, PartialEq, Eq)]
enum Leaf<'a> {
    Byte(u8),
    Int(i32),
    Long(i64),
    Text(&'a [u8]),
}

struct Reader<'a> {
    rest: &'a [u8],
    budget: usize,
}
type Parsed<T> = std::result::Result<T, ()>;

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Parsed<&'a [u8]> {
        let (value, rest) = self.rest.split_at_checked(count).ok_or(())?;
        self.rest = rest;
        Ok(value)
    }

    fn byte(&mut self) -> Parsed<u8> {
        Ok(self.take(1)?[0])
    }

    fn text(&mut self) -> Parsed<&'a [u8]> {
        let length = u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| ())?);
        self.take(length as usize)
    }

    fn count(&mut self) -> Parsed<usize> {
        let size = i32::from_be_bytes(self.take(4)?.try_into().map_err(|_| ())?);
        usize::try_from(size).map_err(|_| ())
    }

    fn visit(&mut self, depth: usize) -> Parsed<()> {
        if depth > 64 || self.budget == 0 {
            return Err(());
        }
        self.budget -= 1;
        Ok(())
    }

    fn tag(&mut self, kind: u8, depth: usize) -> Parsed<Option<Leaf<'a>>> {
        self.visit(depth)?;
        match kind {
            1 => Ok(Some(Leaf::Byte(self.byte()?))),
            2 => self.take(2).map(|_| None),
            3 => Ok(Some(Leaf::Int(i32::from_be_bytes(
                self.take(4)?.try_into().map_err(|_| ())?,
            )))),
            4 => Ok(Some(Leaf::Long(i64::from_be_bytes(
                self.take(8)?.try_into().map_err(|_| ())?,
            )))),
            5 | 6 => self.take(if kind == 5 { 4 } else { 8 }).map(|_| None),
            7 | 11 | 12 => {
                let width = match kind {
                    7 => 1,
                    11 => 4,
                    _ => 8,
                };
                let count = self.count()?.checked_mul(width).ok_or(())?;
                self.take(count).map(|_| None)
            }
            8 => self.text().map(|text| Some(Leaf::Text(text))),
            9 => {
                let kind = self.byte()?;
                let count = self.count()?;
                if kind > 12 || (kind == 0 && count > 0) {
                    return Err(());
                }
                for _ in 0..count {
                    self.tag(kind, depth + 1)?;
                }
                Ok(None)
            }
            10 => self.compound(&[], depth + 1),
            _ => Err(()),
        }
    }

    fn compound(&mut self, path: &[&str], depth: usize) -> Parsed<Option<Leaf<'a>>> {
        let mut value = None;
        let mut matched = false;
        self.visit(depth)?;
        loop {
            let kind = self.byte()?;
            if kind == 0 {
                break;
            }
            let key = self.text()?;
            if path.first().is_some_and(|part| part.as_bytes() == key) {
                if matched {
                    return Err(());
                }
                matched = true;
                if path.len() == 1 {
                    value = self.tag(kind, depth + 1)?;
                } else if kind == 10 {
                    value = self.compound(&path[1..], depth + 1)?;
                } else {
                    self.tag(kind, depth + 1)?;
                }
            } else {
                self.tag(kind, depth + 1)?;
            }
        }
        Ok(value)
    }
}

fn exact<'a>(bytes: &'a [u8], path: &[&str]) -> Option<Leaf<'a>> {
    let mut reader = Reader {
        rest: bytes,
        budget: 500_000,
    };
    if reader.byte().ok()? != 10 {
        return None;
    }
    reader.text().ok()?;
    let value = reader.compound(path, 0).ok()?;
    reader.rest.is_empty().then_some(value).flatten()
}

pub(super) fn persisted_field(key: &str, modern: bool) -> bool {
    matches!(key, "worldType" | "initialTime")
        || world_live::RULE_KEYS
            .iter()
            .any(|(logical, old, _)| *logical == key && (modern || !old.is_empty()))
}

fn generator(bytes: &[u8], modern: bool, compounds: bool) -> Option<&'static str> {
    if compounds {
        let mut path = vec![if modern { "data" } else { "Data" }];
        if !modern {
            path.push("WorldGenSettings");
        }
        path.extend(["dimensions", "minecraft:overworld", "generator", "type"]);
        match exact(bytes, &path)? {
            Leaf::Text(b"minecraft:flat") => Some("Flat"),
            Leaf::Text(b"minecraft:noise") => {
                *path.last_mut()? = "settings";
                match exact(bytes, &path)? {
                    Leaf::Text(b"minecraft:overworld") => Some("Normal"),
                    Leaf::Text(_) => Some("Other"),
                    _ => None,
                }
            }
            Leaf::Text(_) => Some("Other"),
            _ => None,
        }
    } else {
        match exact(bytes, &["Data", "generatorName"])? {
            Leaf::Text(b"flat") => Some("Flat"),
            Leaf::Text(b"default") => Some("Normal"),
            Leaf::Text(_) => Some("Other"),
            _ => None,
        }
    }
}

fn rule_value(bytes: &[u8], key: &str, modern: bool) -> Option<String> {
    let (_, old, new) = world_live::RULE_KEYS
        .iter()
        .copied()
        .find(|(logical, _, _)| *logical == key)?;
    if modern {
        let namespaced = format!("minecraft:{new}");
        return match (key, exact(bytes, &["data", &namespaced])?) {
            ("gamerules.max_entity_cramming", Leaf::Int(value)) if value >= 0 => {
                Some(value.to_string())
            }
            ("gamerules.max_entity_cramming", _) => None,
            (_, Leaf::Byte(0)) => Some("false".into()),
            (_, Leaf::Byte(1)) => Some("true".into()),
            _ => None,
        };
    }
    if old.is_empty() {
        return None;
    }
    let Leaf::Text(raw) = exact(bytes, &["Data", "GameRules", old])? else {
        return None;
    };
    let raw = std::str::from_utf8(raw).ok()?;
    match (key, raw) {
        ("gamerules.max_entity_cramming", _) => {
            raw.parse::<u32>().ok().map(|number| number.to_string())
        }
        (_, "true" | "false") => Some(raw.to_owned()),
        _ => None,
    }
}

fn clock_value(bytes: &[u8], modern: bool) -> Option<String> {
    let path: &[&str] = if modern {
        &["data", "minecraft:overworld", "total_ticks"]
    } else {
        &["Data", "DayTime"]
    };
    let Leaf::Long(ticks) = exact(bytes, path)? else {
        return None;
    };
    (ticks >= 0).then(|| (ticks % 24000).to_string())
}

pub(in crate::windows) fn observe(
    instance: &Path,
    core: &CoreProfile,
    keys: &[&str],
) -> std::collections::BTreeMap<String, String> {
    let mut values = std::collections::BTreeMap::new();
    let Some(world) = world_directory(instance) else {
        return values;
    };
    let modern = core.minecraft.starts_with("26.");
    let needs_generator = keys.contains(&"worldType");
    let needs_clock = keys.contains(&"initialTime");
    let needs_rules = keys
        .iter()
        .any(|key| !matches!(*key, "worldType" | "initialTime"));
    let legacy = (!modern && !keys.is_empty())
        .then(|| saved_file(&world.join("level.dat")))
        .flatten();
    let modern_data = if core.family == "SPIGOT" {
        world.join("data/minecraft")
    } else {
        world.join("dimensions/minecraft/overworld/data/minecraft")
    };
    let gen = (modern && needs_generator)
        .then(|| saved_file(&modern_data.join("world_gen_settings.dat")))
        .flatten();
    let clock = (modern && needs_clock)
        .then(|| saved_file(&modern_data.join("world_clocks.dat")))
        .flatten();
    let rules = (modern && needs_rules)
        .then(|| saved_file(&modern_data.join("game_rules.dat")))
        .flatten();
    let generator_compound = modern
        || core.minecraft == "1.16.5"
        || core.minecraft.starts_with("1.20.")
        || core.minecraft.starts_with("1.21.");
    for key in keys.iter().copied() {
        let value = match key {
            "worldType" => gen
                .as_deref()
                .or(legacy.as_deref())
                .and_then(|bytes| generator(bytes, modern, generator_compound))
                .map(str::to_owned),
            "initialTime" => clock
                .as_deref()
                .or(legacy.as_deref())
                .and_then(|bytes| clock_value(bytes, modern)),
            _ => rules
                .as_deref()
                .or(legacy.as_deref())
                .and_then(|bytes| rule_value(bytes, key, modern)),
        };
        if let Some(value) = value {
            values.insert(key.to_owned(), value);
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gzip_checks_size_and_crc_before_accepting_nbt() {
        // Fixed gzip member with unnamed compound root and one legacy string rule.
        let raw =
            b"\x0a\0\0\x0a\0\x04Data\x0a\0\x09GameRules\x08\0\x0ddoMobSpawning\0\x05false\0\0\0";
        let source = [
            31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 227, 98, 96, 224, 98, 96, 113, 73, 44, 73, 228, 98,
            224, 116, 79, 204, 77, 13, 42, 205, 73, 45, 230, 96, 224, 77, 201, 247, 205, 79, 10,
            46, 72, 44, 207, 203, 204, 75, 103, 96, 77, 75, 204, 41, 78, 101, 96, 96, 0, 0, 178,
            78, 242, 45, 48, 0, 0, 0,
        ];
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
        assert_eq!(
            rule_value(raw, "gamerules.spawn_mobs", false).as_deref(),
            Some("false")
        );
        let mut trailing = raw.to_vec();
        trailing.push(0);
        assert!(exact(&trailing, &["Data", "GameRules", "doMobSpawning"]).is_none());
        assert_eq!(inflate(source.to_vec()).unwrap(), raw);
        assert!(inflate(source[..source.len() - 1].to_vec()).is_err());
    }

    #[test]
    fn modern_rules_and_time_require_exact_paths_and_nbt_types() {
        let boolean = b"\x0a\0\0\x0a\0\x04data\x01\0\x14minecraft:spawn_mobs\x01\0\0";
        let wrong_type = b"\x0a\0\0\x0a\0\x04data\x03\0\x14minecraft:spawn_mobs\0\0\0\x01\0\0";
        let wrong_depth = b"\x0a\0\0\x01\0\x14minecraft:spawn_mobs\x01\0";
        let clock = b"\x0a\0\0\x0a\0\x04data\x0a\0\x13minecraft:overworld\x04\0\x0btotal_ticks\0\0\0\0\0\0\x75\x30\0\0\0";
        let legacy_clock = b"\x0a\0\0\x0a\0\x04Data\x04\0\x07DayTime\0\0\0\0\0\0\x75\x30\0\0";
        assert_eq!(
            rule_value(boolean, "gamerules.spawn_mobs", true).as_deref(),
            Some("true")
        );
        assert!(rule_value(wrong_type, "gamerules.spawn_mobs", true).is_none());
        assert!(rule_value(wrong_depth, "gamerules.spawn_mobs", true).is_none());
        assert_eq!(clock_value(clock, true).as_deref(), Some("6000"));
        assert_eq!(clock_value(legacy_clock, false).as_deref(), Some("6000"));
        assert!(clock_value(legacy_clock, true).is_none());
    }
}
