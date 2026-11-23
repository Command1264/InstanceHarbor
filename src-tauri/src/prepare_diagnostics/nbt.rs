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
    total: usize,
}
type Parsed<T> = std::result::Result<T, ()>;
/// A matched value and the byte range of its payload, including a string's length prefix.
type Located<'a> = (Leaf<'a>, std::ops::Range<usize>);

impl<'a> Reader<'a> {
    fn offset(&self) -> usize {
        self.total - self.rest.len()
    }

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
            10 => self.compound(&[], depth + 1).map(|_| None),
            _ => Err(()),
        }
    }

    fn compound(&mut self, path: &[&str], depth: usize) -> Parsed<Option<Located<'a>>> {
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
                    let start = self.offset();
                    value = self.tag(kind, depth + 1)?.map(|leaf| (leaf, start..self.offset()));
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
    located(bytes, path).map(|(leaf, _)| leaf)
}

/// The value at `path` and its span, only when the whole document parses.
fn located<'a>(bytes: &'a [u8], path: &[&str]) -> Option<Located<'a>> {
    let mut reader = Reader {
        rest: bytes,
        budget: 500_000,
        total: bytes.len(),
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

/// The save file holding the overworld's game rules for this core.
fn rules_file(world: &Path, core: &CoreProfile) -> PathBuf {
    if !core.minecraft.starts_with("26.") {
        world.join("level.dat")
    } else if core.family == "SPIGOT" {
        world.join("data/minecraft/game_rules.dat")
    } else {
        world.join("dimensions/minecraft/overworld/data/minecraft/game_rules.dat")
    }
}

/// `bytes` with one rule changed to `target`. The rule must already be stored with
/// the type this version uses; anything else is left to the next start.
fn with_rule(bytes: &[u8], key: &str, target: &str, modern: bool) -> Option<Vec<u8>> {
    let (_, old, new) = world_live::RULE_KEYS.iter().copied().find(|(logical, _, _)| *logical == key)?;
    let cramming = key == "gamerules.max_entity_cramming";
    let number = || target.parse::<u32>().ok().filter(|value| *value <= 9999 && target.bytes().all(|b| b.is_ascii_digit()));
    let flag = || match target {
        "true" => Some(1u8),
        "false" => Some(0u8),
        _ => None,
    };
    let (payload, span) = if modern {
        let name = format!("minecraft:{new}");
        match (cramming, located(bytes, &["data", &name])?) {
            (true, (Leaf::Int(_), span)) => ((number()? as i32).to_be_bytes().to_vec(), span),
            (false, (Leaf::Byte(0 | 1), span)) => (vec![flag()?], span),
            _ => return None,
        }
    } else {
        if old.is_empty() {
            return None;
        }
        if cramming { number()?; } else { flag()?; }
        let (Leaf::Text(_), span) = located(bytes, &["Data", "GameRules", old])? else {
            return None;
        };
        let mut payload = (target.len() as u16).to_be_bytes().to_vec();
        payload.extend_from_slice(target.as_bytes());
        (payload, span)
    };
    let mut changed = Vec::with_capacity(bytes.len() + payload.len());
    changed.extend_from_slice(&bytes[..span.start]);
    changed.extend_from_slice(&payload);
    changed.extend_from_slice(&bytes[span.end..]);
    Some(changed)
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    use flate2::{write::GzEncoder, Compression};
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).map_err(fail)?;
    encoder.finish().map_err(fail)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = File::create_new(path).map_err(|_| "WorldSaveBackupFailed")?;
    file.write_all(bytes).map_err(|_| "WorldSaveBackupFailed")?;
    file.sync_all().map_err(|_| "WorldSaveBackupFailed")?;
    Ok(())
}

fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_file_name(format!(
        ".{}.instanceharbor-{}.tmp",
        path.file_name().ok_or("WorldSaveUnavailable")?.to_string_lossy(),
        std::process::id()
    ));
    write_new(&temporary, bytes).map_err(|_| "WorldSaveWriteFailed")?;
    if let Err(error) = core_maintenance::move_durable_with_stage(&temporary, path, true, "replace-world-rules") {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

/// Writes game rules into the saved overworld of a server that stopped normally.
/// The caller holds the instance lock and the world's runtime lease. The original
/// file is copied to `backup` first; any failure leaves the save as it was.
pub(in crate::windows) fn write_rules(
    instance: &Path,
    core: &CoreProfile,
    targets: &[(String, String)],
    backup: &Path,
) -> Result<()> {
    let world = world_directory(instance).ok_or("WorldSaveUnavailable")?;
    let modern = core.minecraft.starts_with("26.");
    let file = rules_file(&world, core);
    let original = locked_bytes(&file, MAX_GZIP)?.ok_or("WorldSaveUnavailable")?;
    let before = inflate(original.clone()).map_err(|_| "WorldSaveUnreadable")?;
    let mut changed = before.clone();
    for (key, target) in targets {
        changed = with_rule(&changed, key, target, modern).ok_or("WorldSaveFieldUnsupported")?;
    }
    if targets.iter().any(|(key, target)| rule_value(&changed, key, modern).as_deref() != Some(target.as_str())) {
        return Err("WorldSaveVerifyFailed".into());
    }
    let encoded = gzip(&changed)?;
    if inflate(encoded.clone()).ok().as_deref() != Some(changed.as_slice()) {
        return Err("WorldSaveVerifyFailed".into());
    }
    write_new(backup, &original)?;
    if locked_bytes(&file, MAX_GZIP)?.as_deref() != Some(original.as_slice()) {
        return Err("WorldFileChanged".into());
    }
    replace(&file, &encoded)?;
    if saved_file(&file).as_deref() != Some(changed.as_slice()) {
        replace(&file, &original)?;
        return Err("WorldSaveVerifyFailed".into());
    }
    Ok(())
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
    /// Unnamed root compound with the given body (already encoded tags).
    fn document(body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![10, 0, 0];
        bytes.extend_from_slice(body);
        bytes.push(0);
        bytes
    }

    fn named(kind: u8, name: &str, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![kind];
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn compound(name: &str, children: &[Vec<u8>]) -> Vec<u8> {
        let mut payload: Vec<u8> = children.concat();
        payload.push(0);
        named(10, name, &payload)
    }

    fn text(value: &str) -> Vec<u8> {
        let mut payload = (value.len() as u16).to_be_bytes().to_vec();
        payload.extend_from_slice(value.as_bytes());
        payload
    }

    fn modern_rules() -> Vec<u8> {
        document(&[
            named(3, "DataVersion", &4903i32.to_be_bytes()),
            compound("data", &[
                named(1, "minecraft:spawn_mobs", &[1]),
                named(3, "minecraft:max_entity_cramming", &24i32.to_be_bytes()),
                named(1, "minecraft:pvp", &[1]),
            ]),
        ].concat())
    }

    fn legacy_level() -> Vec<u8> {
        document(&compound("Data", &[
            named(4, "Time", &5i64.to_be_bytes()),
            compound("GameRules", &[
                named(8, "doMobSpawning", &text("true")),
                named(8, "maxEntityCramming", &text("24")),
            ]),
        ]))
    }

    fn core(family: &str, minecraft: &str) -> CoreProfile {
        CoreProfile { family: family.into(), minecraft: minecraft.into(), build: "1".into(), sha256: "fixture".into() }
    }

    #[test]
    fn modern_rules_change_in_place_with_their_stored_types() {
        let bytes = modern_rules();
        let changed = with_rule(&bytes, "gamerules.spawn_mobs", "false", true).unwrap();
        let changed = with_rule(&changed, "gamerules.max_entity_cramming", "9999", true).unwrap();
        let changed = with_rule(&changed, "serverProperties.pvp", "false", true).unwrap();
        assert_eq!(changed.len(), bytes.len());
        assert_eq!(rule_value(&changed, "gamerules.spawn_mobs", true).as_deref(), Some("false"));
        assert_eq!(rule_value(&changed, "gamerules.max_entity_cramming", true).as_deref(), Some("9999"));
        assert_eq!(rule_value(&changed, "serverProperties.pvp", true).as_deref(), Some("false"));
        assert_eq!(exact(&changed, &["DataVersion"]), Some(Leaf::Int(4903)));
        // Values outside the rule's range, a missing rule and a rule of another type stay unwritten.
        assert!(with_rule(&bytes, "gamerules.spawn_mobs", "maybe", true).is_none());
        assert!(with_rule(&bytes, "gamerules.max_entity_cramming", "10000", true).is_none());
        assert!(with_rule(&bytes, "gamerules.max_entity_cramming", "+24", true).is_none());
        assert!(with_rule(&bytes, "gamerules.advance_time", "false", true).is_none());
        let wrong_type = document(&compound("data", &[named(3, "minecraft:spawn_mobs", &1i32.to_be_bytes())]));
        assert!(with_rule(&wrong_type, "gamerules.spawn_mobs", "false", true).is_none());
    }

    #[test]
    fn legacy_rules_rewrite_the_string_and_keep_the_rest() {
        let bytes = legacy_level();
        let changed = with_rule(&bytes, "gamerules.spawn_mobs", "false", false).unwrap();
        let changed = with_rule(&changed, "gamerules.max_entity_cramming", "100", false).unwrap();
        assert_eq!(rule_value(&changed, "gamerules.spawn_mobs", false).as_deref(), Some("false"));
        assert_eq!(rule_value(&changed, "gamerules.max_entity_cramming", false).as_deref(), Some("100"));
        assert_eq!(exact(&changed, &["Data", "Time"]), Some(Leaf::Long(5)));
        // 26.x-only rules have no legacy name.
        assert!(with_rule(&bytes, "serverProperties.pvp", "false", false).is_none());
    }

    struct Server(PathBuf);

    impl Server {
        fn new(rules: &str, bytes: &[u8]) -> Self {
            let root = std::env::temp_dir().canonicalize().unwrap().join(format!("ih-world-write-{}-{}", std::process::id(), clock()));
            let file = root.join("world").join(rules);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(root.join("server.properties"), "level-name=world\n").unwrap();
            fs::write(file, gzip(bytes).unwrap()).unwrap();
            Self(root)
        }

        fn saved(&self, rules: &str) -> Vec<u8> {
            inflate(fs::read(self.0.join("world").join(rules)).unwrap()).unwrap()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn writing_backs_up_the_original_and_failures_leave_the_save_unchanged() {
        let modern = "dimensions/minecraft/overworld/data/minecraft/game_rules.dat";
        for family in ["PAPER", "FOLIA"] {
            let server = Server::new(modern, &modern_rules());
            let original = fs::read(server.0.join("world").join(modern)).unwrap();
            let backup = server.0.join("backup.dat");
            let targets = vec![
                ("gamerules.spawn_mobs".to_owned(), "false".to_owned()),
                ("gamerules.max_entity_cramming".to_owned(), "50".to_owned()),
            ];
            write_rules(&server.0, &core(family, "26.1.2"), &targets, &backup).unwrap();
            let saved = server.saved(modern);
            assert_eq!(rule_value(&saved, "gamerules.spawn_mobs", true).as_deref(), Some("false"));
            assert_eq!(rule_value(&saved, "gamerules.max_entity_cramming", true).as_deref(), Some("50"));
            assert_eq!(fs::read(&backup).unwrap(), original);

            // One unsupported rule stops the whole write before the backup or the save changes.
            let before = fs::read(server.0.join("world").join(modern)).unwrap();
            let refused = server.0.join("refused.dat");
            let mixed = vec![
                ("gamerules.spawn_mobs".to_owned(), "true".to_owned()),
                ("gamerules.advance_time".to_owned(), "false".to_owned()),
            ];
            assert_eq!(write_rules(&server.0, &core(family, "26.3"), &mixed, &refused).unwrap_err(), "WorldSaveFieldUnsupported");
            assert_eq!(fs::read(server.0.join("world").join(modern)).unwrap(), before);
            assert!(!refused.exists());
        }

        let spigot = Server::new("data/minecraft/game_rules.dat", &modern_rules());
        let targets = vec![("gamerules.spawn_mobs".to_owned(), "false".to_owned())];
        write_rules(&spigot.0, &core("SPIGOT", "26.2"), &targets, &spigot.0.join("backup.dat")).unwrap();
        assert_eq!(rule_value(&spigot.saved("data/minecraft/game_rules.dat"), "gamerules.spawn_mobs", true).as_deref(), Some("false"));

        let legacy = Server::new("level.dat", &legacy_level());
        write_rules(&legacy.0, &core("PAPER", "1.21.8"), &targets, &legacy.0.join("backup.dat")).unwrap();
        assert_eq!(rule_value(&legacy.saved("level.dat"), "gamerules.spawn_mobs", false).as_deref(), Some("false"));

        // A damaged save is reported and kept as it was.
        let damaged = Server::new(modern, &modern_rules());
        let path = damaged.0.join("world").join(modern);
        fs::write(&path, b"not gzip").unwrap();
        assert_eq!(write_rules(&damaged.0, &core("PAPER", "26.3"), &targets, &damaged.0.join("backup.dat")).unwrap_err(), "WorldSaveUnreadable");
        assert_eq!(fs::read(&path).unwrap(), b"not gzip");
        assert!(!damaged.0.join("backup.dat").exists());
    }
}
