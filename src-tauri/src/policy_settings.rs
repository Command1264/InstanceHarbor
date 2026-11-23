use super::*;
use serde_json::Map;
use std::collections::{BTreeMap, BTreeSet};
use std::os::windows::ffi::OsStrExt;

#[path = "policy_catalog.rs"]
mod policy_catalog;
#[path = "policy_settings/yaml.rs"]
mod yaml;
#[path = "policy_settings/yaml_preflight.rs"]
pub(super) mod yaml_preflight;
#[path = "policy_settings/resource_pack.rs"]
mod resource_pack;
#[path = "policy_settings/world_batch_preview.rs"]
mod world_batch_preview;
use policy_catalog::{reason_text, Binding, Field, Profile, FIELDS, PROFILES};

const BASE_FIELDS: &[(&str, &str, u64)] = &[
    ("viewDistance", "view-distance", 10),
    ("simulationDistance", "simulation-distance", 4),
    ("maxPlayers", "max-players", 20),
    ("spawnProtection", "spawn-protection", 16),
];

const FLAT_GENERATOR: &str = r#"{"layers":[{"block":"minecraft:bedrock","height":1},{"block":"minecraft:dirt","height":2},{"block":"minecraft:grass_block","height":1}],"biome":"minecraft:plains","features":false,"lakes":false}"#;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn MoveFileExW(source: *const u16, target: *const u16, flags: u32) -> i32;
}

fn field(id: &str) -> Result<&'static Field> {
    FIELDS
        .iter()
        .find(|field| field.id() == id)
        .ok_or_else(|| format!("UnknownPolicyField: {id}"))
}

fn property_field(key: &str) -> Result<&'static Field> {
    field(&format!("serverProperties.{key}"))
}

fn default_profile() -> Profile {
    Profile::resolve("PAPER", "26.3").expect("default profile in catalog")
}

fn defaults() -> Value {
    Value::Object(
        BASE_FIELDS
            .iter()
            .map(|(key, _, value)| ((*key).to_owned(), json!(value)))
            .collect(),
    )
}

fn validate_profile_fields(profile: Profile, fields: &Map<String, Value>) -> Result<()> {
    for (key, value) in fields {
        let field = field(key)?;
        if field.gate(profile, false).0 != "available" {
            return Err(format!("InitializationFieldUnavailable: {key}"));
        }
        field.normalize(
            value
                .as_str()
                .ok_or_else(|| format!("InvalidPolicyValue: {key}"))?,
            profile,
        )?;
    }
    resource_pack::validate_group(fields)?;
    Ok(())
}

fn valid_initialization(values: &Map<String, Value>) -> Result<()> {
    for (key, value) in values {
        match key.as_str() {
            "serverProperties" => {
                for (key, value) in value.as_object().ok_or("InvalidInitialization")? {
                    if !matches!(
                        key.as_str(),
                        "gamemode"
                            | "difficulty"
                            | "hardcore"
                            | "force-gamemode"
                            | "generate-structures"
                            | "allow-flight"
                            | "player-idle-timeout"
                    ) {
                        return Err(format!("UnsupportedInitializationKey: {key}"));
                    }
                    legacy_value(key, value)?;
                }
            }
            "profiles" => {
                for (key, fields) in value.as_object().ok_or("InvalidInitialization")? {
                    let (family, version) =
                        key.split_once('@').ok_or("InvalidInitializationProfile")?;
                    let profile = Profile::resolve(family, version)?;
                    if key != &profile.key() {
                        return Err("InvalidInitializationProfile".into());
                    }
                    validate_profile_fields(
                        profile,
                        fields.as_object().ok_or("InvalidInitialization")?,
                    )?;
                }
            }
            _ => {
                let (_, property, _) = BASE_FIELDS
                    .iter()
                    .find(|(base, _, _)| key == base)
                    .ok_or_else(|| format!("UnsupportedInitializationKey: {key}"))?;
                property_field(property)?.normalize(
                    &value.as_u64().ok_or("InvalidInitialization")?.to_string(),
                    default_profile(),
                )?;
            }
        }
    }
    Ok(())
}

fn legacy_value(key: &str, value: &Value) -> Result<String> {
    let value = value.as_str().ok_or("InvalidInitialization")?;
    let value = match (key, value) {
        ("difficulty", "0") => "peaceful",
        ("difficulty", "1") => "easy",
        ("difficulty", "2") => "normal",
        ("difficulty", "3") => "hard",
        _ => value,
    };
    property_field(key)?.normalize(value, default_profile())
}

fn initialization(root: &Path) -> Result<Value> {
    initialization_with_sources(root).map(|(settings, _)| settings)
}

fn initialization_with_sources(root: &Path) -> Result<(Value, BTreeSet<String>)> {
    let path = root.join("initialization.json");
    let mut result = defaults();
    let mut sources = BTreeSet::new();
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            no_redirect(&path)?;
            let saved: Value = read(&path)?;
            let values = saved.as_object().ok_or("InvalidInitialization")?;
            valid_initialization(values)?;
            sources.extend(values.keys().cloned());
            result.as_object_mut().unwrap().extend(values.clone());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(file_error("inspect-initialization", &path, error)),
    }
    Ok((result, sources))
}

fn selected_defaults(settings: &Value, profile: Profile) -> Result<Map<String, Value>> {
    let mut values = Map::new();
    for (legacy, key, _) in BASE_FIELDS {
        if property_field(key)?.gate(profile, false).0 == "available" {
            values.insert(
                format!("serverProperties.{key}"),
                json!(settings[*legacy]
                    .as_u64()
                    .ok_or("InvalidInitialization")?
                    .to_string()),
            );
        }
    }
    if let Some(advanced) = settings["serverProperties"].as_object() {
        for (key, value) in advanced {
            values.insert(
                format!("serverProperties.{key}"),
                json!(legacy_value(key, value)?),
            );
        }
    }
    if let Some(fields) = settings["profiles"][profile.key()].as_object() {
        values.extend(fields.clone());
    }
    resource_pack::validate_group(&values)?;
    Ok(values)
}

fn initialization_response(settings: &Value, profile: Profile) -> Result<Value> {
    let bytes = serde_json::to_vec(settings).map_err(fail)?;
    let editable = settings["profiles"][profile.key()].as_object();
    let effective = selected_defaults(settings, profile)?;
    let rows: Vec<_> = FIELDS
        .iter()
        .map(|field| {
            let key = field.id();
            let mut row = field.describe(Some(profile), false);
            row["value"] = editable
                .and_then(|values| values.get(&key))
                .cloned()
                .unwrap_or(Value::Null);
            row["inherited"] = effective.get(&key).cloned().unwrap_or(Value::Null);
            row
        })
        .collect();
    let profiles: Vec<_> = PROFILES
        .iter()
        .flat_map(|(family, versions)| {
            versions
                .iter()
                .map(move |version| json!({"family":family,"version":version}))
        })
        .collect();
    Ok(
        json!({"values":settings,"revision":content_revision::revision(&bytes)?,"rows":rows,
        "profile":profile_json(profile),"profiles":profiles}),
    )
}

fn profile_json(profile: Profile) -> Value {
    json!({"family":profile.family,"version":profile.version})
}

fn request_revision(request: &Map<String, Value>) -> Result<&str> {
    request
        .get("revision")
        .and_then(Value::as_str)
        .ok_or_else(|| "InvalidSettingsRevision".into())
}

pub(super) fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.{}.tmp", std::process::id(), clock()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(fail)?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(fail(error));
    }
    drop(file);
    let source: Vec<u16> = temp.as_os_str().encode_wide().chain([0]).collect();
    let target: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // Replace within the same directory so readers observe either complete version.
    let ok = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0x1 | 0x8) };
    if ok == 0 {
        let error = io::Error::last_os_error();
        let _ = fs::remove_file(&temp);
        return Err(fail(error));
    }
    Ok(())
}

fn save_initialization(root: &Path, input: &str) -> Result<Value> {
    let request: Value = serde_json::from_str(input).map_err(fail)?;
    let request = request.as_object().ok_or("InvalidInitialization")?;
    let revision = request_revision(request)?;
    let update = request
        .get("values")
        .and_then(Value::as_object)
        .ok_or("InvalidInitialization")?;
    let current = initialization(root)?;
    if initialization_response(&current, default_profile())?["revision"] != revision {
        return Err("SettingsConflict".into());
    }
    let mut updated = current.clone();
    let profile = if request.len() == 3 {
        let profile = request
            .get("profile")
            .and_then(Value::as_object)
            .filter(|profile| profile.len() == 2)
            .ok_or("InvalidInitializationProfile")?;
        let profile = Profile::resolve(
            profile
                .get("family")
                .and_then(Value::as_str)
                .ok_or("InvalidInitializationProfile")?,
            profile
                .get("version")
                .and_then(Value::as_str)
                .ok_or("InvalidInitializationProfile")?,
        )?;
        validate_profile_fields(profile, update)?;
        let normalized = update
            .iter()
            .map(|(key, value)| {
                Ok((
                    key.clone(),
                    json!(field(key)?
                        .normalize(value.as_str().ok_or("InvalidInitialization")?, profile)?),
                ))
            })
            .collect::<Result<Map<String, Value>>>()?;
        if !update.is_empty() || updated["profiles"].is_object() {
            if !updated["profiles"].is_object() {
                updated["profiles"] = json!({});
            }
            let profiles = updated["profiles"].as_object_mut().unwrap();
            if update.is_empty() {
                profiles.remove(&profile.key());
            } else {
                profiles.insert(profile.key(), json!(normalized));
            }
            if profiles.is_empty() {
                updated.as_object_mut().unwrap().remove("profiles");
            }
        }
        profile
    } else if request.len() == 2 {
        valid_initialization(update)?;
        updated.as_object_mut().unwrap().extend(update.clone());
        default_profile()
    } else {
        return Err("InvalidInitialization".into());
    };
    if updated != current {
        let path = root.join("initialization.json");
        if initialization(root)? != current {
            return Err("SettingsConflict".into());
        }
        let bytes = serde_json::to_vec_pretty(&updated).map_err(fail)?;
        replace_file(&path, &bytes)?;
    }
    let readback = initialization(root)?;
    if readback != updated {
        return Err("InitializationReadbackMismatch".into());
    }
    initialization_response(&readback, profile)
}

pub(super) fn write_creation_settings(
    root: &Path,
    stage: &Path,
    port: u16,
    core: &CoreProfile,
) -> Result<()> {
    write_creation_settings_for(root, stage, port, core, true)
}

pub(super) fn write_creation_settings_for(
    root: &Path,
    stage: &Path,
    port: u16,
    core: &CoreProfile,
    use_workspace_defaults: bool,
) -> Result<()> {
    let profile = Profile::resolve(&core.family, &core.minecraft)?;
    let settings = if use_workspace_defaults { initialization(root)? } else { defaults() };
    fs::write(
        stage.join("server.properties"),
        creation_properties(&settings, port, profile)?,
    )
    .map_err(fail)?;
    let mut documents = BTreeMap::<&str, BTreeMap<&str, BTreeMap<&str, String>>>::new();
    let selected = selected_defaults(&settings, profile)?;
    world_live::first_ready::stage(stage, core, &selected)?;
    for (id, raw) in selected {
        let field = field(&id)?;
        let Binding::Yaml(path, key) = field.binding(profile) else {
            continue;
        };
        if field.gate(profile, false).0 != "available" {
            continue;
        }
        let (section, key) = key.rsplit_once('.').map_or_else(
            || ("world-settings.default", key),
            |(section, key)| (section, key),
        );
        let value = field.normalize(raw.as_str().ok_or("InvalidInitialization")?, profile)?;
        documents
            .entry(path)
            .or_default()
            .entry(section)
            .or_default()
            .insert(key, value);
    }
    for (path, sections) in documents {
        let target = stage.join(path);
        if let Some(parent) = target.parent().filter(|dir| *dir != stage) {
            fs::create_dir_all(parent).map_err(fail)?;
        }
        let mut text = String::new();
        for (section, fields) in sections {
            let mut indent = "";
            for level in section.split('.') {
                text.push_str(&format!("{indent}{level}:\n"));
                indent = if indent.is_empty() { "  " } else { "    " };
            }
            for (key, value) in fields {
                text.push_str(&format!("{indent}{key}: {value}\n"));
            }
        }
        fs::write(target, text).map_err(fail)?;
    }
    Ok(())
}

fn base_creation_properties(port: u16) -> String {
    format!("server-ip=127.0.0.1\nserver-port={port}\nonline-mode=true\nenable-rcon=false\n")
}

pub(super) fn has_creation_defaults(core: &CoreProfile) -> bool {
    Profile::resolve(&core.family, &core.minecraft).is_ok()
}

pub(super) fn write_basic_creation_settings(stage: &Path, port: u16) -> Result<()> {
    fs::write(stage.join("server.properties"), base_creation_properties(port)).map_err(fail)
}

fn creation_properties(settings: &Value, port: u16, profile: Profile) -> Result<String> {
    let mut properties = base_creation_properties(port);
    let mut selected = selected_defaults(settings, profile)?;
    selected
        .entry("serverProperties.motd")
        .or_insert(json!("A Minecraft Server"));
    for (key, value) in selected {
        let field = field(&key)?;
        let value = field.normalize(value.as_str().ok_or("InvalidInitialization")?, profile)?;
        if field.gate(profile, false).0 != "available" {
            return Err(format!("InitializationFieldUnavailable: {key}"));
        }
        match field.binding(profile) {
            Binding::Properties => properties.push_str(&format!(
                "{}={}\n",
                field.key,
                encode_settings_value(field, &value, profile)?
            )),
            Binding::WorldType => {
                let namespaced =
                    profile.current_storage() || matches!(profile.version, "1.20.6" | "1.21.8");
                let level_type = match (value.as_str(), namespaced) {
                    ("Flat", true) => "minecraft:flat",
                    ("Flat", false) => "FLAT",
                    ("Normal", true) => "minecraft:normal",
                    _ => "DEFAULT",
                };
                properties.push_str(&format!("level-type={level_type}\n"));
                if value == "Flat" && namespaced {
                    properties.push_str(&format!("generator-settings={FLAT_GENERATOR}\n"));
                }
            }
            Binding::Yaml(_, _) | Binding::Rule(_) | Binding::Time => (),
        }
    }
    Ok(properties)
}

fn encode_settings_value(field: &Field, value: &str, profile: Profile) -> Result<String> {
    if field.key == "resource-pack-prompt" && !value.is_empty() {
        return Ok(encode_property(&serde_json::to_string(value).map_err(fail)?));
    }
    Ok(field.encode(value, profile))
}

fn encode_property(value: &str) -> String {
    let mut encoded = String::new();
    for (index, unit) in value.encode_utf16().enumerate() {
        if !(32..=126).contains(&unit) || unit == 92 || (index == 0 && unit == 32) {
            use std::fmt::Write as _;
            write!(encoded, "\\u{unit:04x}").expect("string formatting");
        } else {
            encoded.push(char::from(unit as u8));
        }
    }
    encoded
}

fn decode_property(raw: &str) -> Option<String> {
    let mut decoded = Vec::new();
    let mut units = raw.encode_utf16();
    while let Some(unit) = units.next() {
        let unit = if unit == 92 {
            match units.next()? {
                117 => {
                    let mut unicode = 0u16;
                    for _ in 0..4 {
                        unicode = (unicode << 4)
                            | char::from_u32(units.next()?.into())?.to_digit(16)? as u16;
                    }
                    unicode
                }
                116 => 9,
                110 => 10,
                114 => 13,
                102 => 12,
                escaped => escaped,
            }
        } else {
            unit
        };
        decoded.push(unit);
    }
    String::from_utf16(&decoded).ok()
}

fn property_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\x0c')
}

struct Property {
    value: String,
    start: usize,
    end: usize,
    editable: bool,
}

// Minecraft uses Java Properties syntax, including alternate delimiters and continued lines.
// Track the original physical span so unrelated properties keep their exact bytes.
fn properties(text: &str) -> Result<BTreeMap<String, Vec<Property>>> {
    let mut parsed: BTreeMap<String, Vec<Property>> = BTreeMap::new();
    let mut offset = 0;
    let mut lines = text.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let start = offset;
        offset += line.len();
        let physical = line
            .trim_end_matches(['\r', '\n'])
            .trim_start_matches(property_space);
        if physical.is_empty() || physical.starts_with(['#', '!']) {
            continue;
        }
        let mut logical = physical.to_owned();
        let mut continuation = false;
        while logical
            .as_bytes()
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count()
            % 2
            == 1
        {
            continuation = true;
            logical.pop();
            let Some(next) = lines.next() else {
                break;
            };
            offset += next.len();
            logical.push_str(
                next.trim_end_matches(['\r', '\n'])
                    .trim_start_matches(property_space),
            );
        }
        let mut escaped = false;
        let key_end = logical
            .char_indices()
            .find_map(|(index, c)| {
                if escaped {
                    escaped = false;
                    return None;
                }
                if c == '\\' {
                    escaped = true;
                    return None;
                }
                (c == '=' || c == ':' || property_space(c)).then_some(index)
            })
            .unwrap_or(logical.len());
        let remainder = logical[key_end..].trim_start_matches(property_space);
        let raw_value = remainder
            .strip_prefix(['=', ':'])
            .unwrap_or(remainder)
            .trim_start_matches(property_space);
        let key = decode_property(&logical[..key_end]).ok_or("InvalidPropertiesEscape")?;
        let value = decode_property(raw_value).ok_or("InvalidPropertiesEscape")?;
        let single_line = offset - start == line.len();
        let value_start = if single_line {
            let physical_start = line.len() - line.trim_start_matches(property_space).len();
            let inline_start = physical.len() - raw_value.len();
            start + physical_start + inline_start
        } else {
            start
        };
        let line_ending = text[start..offset]
            .as_bytes()
            .iter()
            .rev()
            .take_while(|b| matches!(b, b'\r' | b'\n'))
            .count();
        parsed.entry(key).or_default().push(Property {
            value,
            start: value_start,
            end: offset - line_ending,
            editable: single_line && !continuation && key_end < logical.len(),
        });
    }
    Ok(parsed)
}

fn file_text(path: &Path) -> Result<String> {
    no_redirect(path)?;
    let file = File::open(path).map_err(fail)?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    if bytes.len() > 1024 * 1024 || bytes.starts_with(b"\xef\xbb\xbf") || bytes.contains(&b'\0') {
        return Err("UnsupportedSettingsFile".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "UnsupportedSettingsEncoding".to_owned())?;
    if text
        .as_bytes()
        .windows(2)
        .filter(|pair| pair[0] == b'\r')
        .any(|pair| pair[1] != b'\n')
        || text.ends_with('\r')
    {
        return Err("UnsupportedSettingsLineEnding".into());
    }
    Ok(text)
}

fn is_resource_pack(key: &str) -> bool {
    key.starts_with("resource-pack") || key == "require-resource-pack"
}

fn logical_value(field: &Field, value: &str, profile: Profile) -> Result<String> {
    if field.key == "resource-pack-prompt" && !value.is_empty() {
        let text = serde_json::from_str::<String>(value).map_err(|_| "RichResourcePackPrompt")?;
        return field.normalize(&text, profile);
    }
    field.normalize(value, profile)
}

fn spigot_override(text: &str) -> BTreeSet<&'static str> {
    let mut blocked = BTreeSet::new();
    let mut world_section = false;
    let mut world_lines = Vec::new();
    for line in text.lines() {
        let content = line.trim_start();
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        if line == content {
            let header = content
                .split_once(" #")
                .map_or(content, |(key, _)| key)
                .trim_end();
            match header.split_once(':') {
                Some(("world-settings", value)) if value.trim().is_empty() => world_section = true,
                _ if header.contains("world-settings") || header.starts_with("<<:") => {
                    blocked.extend(["view-distance", "simulation-distance"]);
                    break;
                }
                _ => world_section = false,
            }
        } else if world_section {
            world_lines.push(content);
        }
    }
    for key in ["view-distance", "simulation-distance"] {
        // Refuse any override outside the ordinary inherited default. YAML aliases,
        // inline maps and custom quoting require a structured config reader.
        let readable = world_lines.iter().all(|line| {
            let content = line
                .split_once(" #")
                .map_or(*line, |(content, _)| content)
                .trim_end();
            if content.starts_with("<<:") || content.contains(['{', '}', '&', '*']) {
                return false;
            }
            !content.contains(key)
                || content.split_once(':').is_some_and(|(name, value)| {
                    name == key && matches!(value.trim(), "default" | "-1")
                })
        });
        if !readable {
            blocked.insert(key);
        }
    }
    blocked
}

struct YamlSetting {
    path: &'static str,
    text: String,
    values: BTreeMap<&'static str, Property>,
}

impl YamlSetting {
    fn read(directory: &Path, path: &'static str, profile: Option<Profile>) -> Result<Option<Self>> {
        let target = directory.join(path);
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(file_error("inspect-world-yaml", &target, error)),
            Ok(_) => (),
        }
        let text = file_text(&target)?;
        let values = profile
            .into_iter()
            .flat_map(|profile| FIELDS.iter().filter_map(move |field| {
                let Binding::Yaml(file, key) = field.binding(profile) else {
                    return None;
                };
                (file == path && field.gate(profile, true).0 == "available").then_some(key)
            }))
            .filter_map(|key| {
                let property = if path == "spigot.yml" {
                    yaml::spigot_integer(&text, key)
                } else {
                    yaml::boolean(&text, key)
                };
                property.map(|property| (key, property))
            })
            .collect();
        Ok(Some(Self { path, text, values }))
    }
}

struct WorldSnapshot {
    path: PathBuf,
    text: String,
    properties: BTreeMap<String, Vec<Property>>,
    profile: Option<Profile>,
    core_sha256: Option<String>,
    revision: String,
    distance_overrides: BTreeSet<&'static str>,
    nether: Option<YamlSetting>,
    spigot: Option<YamlSetting>,
}

impl WorldSnapshot {
    fn read(root: &Path, name: &str) -> Result<Self> {
        Self::read_with_core(root, name, None)
    }

    fn read_with_core(
        root: &Path,
        name: &str,
        diagnostic_core: Option<&CoreProfile>,
    ) -> Result<Self> {
        if !token(name) || !admitted(root, name)? {
            return Err("InstanceExcludedOrInvalid".into());
        }
        let instance = instance(root, name)?;
        if instance.removed {
            return Err("InstanceRemoved".into());
        }
        let directory = instance_path(root, name)?;
        let jar = core_artifact::jar(&directory, &instance);
        let verified = match fs::symlink_metadata(&jar) {
            Ok(_) => {
                no_redirect(&jar)?;
                if let Some(core) = diagnostic_core {
                    if content_revision::file_revision(&jar)? != core.sha256
                        || instance
                            .core_profile
                            .as_ref()
                            .is_some_and(|saved| saved.sha256 != core.sha256)
                    {
                        return Err("CoreIdentityChanged".into());
                    }
                    Some(core.clone())
                } else {
                    let digest = content_revision::file_revision(&jar)?;
                    match instance.core_profile.as_ref() {
                        Some(saved) if saved.sha256 == digest => Some(saved.clone()),
                        Some(_) => None,
                        None => pinned_core_profile(digest),
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && diagnostic_core.is_some() => {
                return Err("CoreIdentityChanged".into());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(file_error("inspect-server-core", &jar, error)),
        };
        let verified = verified.filter(|actual| {
            instance
                .core_profile
                .as_ref()
                .is_none_or(|saved| saved.sha256 == actual.sha256)
        });
        let profile = verified
            .as_ref()
            .and_then(|profile| Profile::resolve(&profile.family, &profile.minecraft).ok());
        let path = directory.join("server.properties");
        let text = file_text(&path)?;
        let spigot = YamlSetting::read(&directory, "spigot.yml", profile)?;
        let distance_overrides = spigot
            .as_ref()
            .map(|yaml| spigot_override(&yaml.text))
            .unwrap_or_default();
        let spigot_digest = spigot
            .as_ref()
            .map(|yaml| content_revision::revision(yaml.text.as_bytes()))
            .transpose()?;
        let properties = properties(&text)?;
        let nether =
            match profile.map(|profile| property_field("allow-nether").unwrap().binding(profile)) {
                Some(Binding::Yaml(path, _)) => YamlSetting::read(&directory, path, profile)?,
                _ => None,
            };
        let yaml_digest = nether
            .as_ref()
            .map(|yaml| content_revision::revision(yaml.text.as_bytes()))
            .transpose()?;
        let core_sha256 = verified.map(|profile| profile.sha256);
        let binding = json!([
            name,
            directory,
            content_revision::revision(text.as_bytes())?,
            spigot_digest,
            core_sha256,
            yaml_digest
        ]);
        let revision = content_revision::revision(&serde_json::to_vec(&binding).map_err(fail)?)?;
        Ok(Self {
            path,
            text,
            properties,
            profile,
            core_sha256,
            revision,
            distance_overrides,
            nether,
            spigot,
        })
    }

    fn yaml(&self, path: &str) -> Option<&YamlSetting> {
        [self.nether.as_ref(), self.spigot.as_ref()]
            .into_iter()
            .flatten()
            .find(|yaml| yaml.path == path)
    }

    fn source(&self, field: &Field) -> (&'static str, &str) {
        if let Some(Binding::Yaml(path, _)) = self.profile.map(|profile| field.binding(profile)) {
            let yaml = self.yaml(path).expect("validated YAML observation");
            return (path, &yaml.text);
        }
        ("server.properties", &self.text)
    }

    fn observe(&self, field: &Field) -> Option<&Property> {
        if let Some(Binding::Yaml(path, key)) = self.profile.map(|profile| field.binding(profile)) {
            return self.yaml(path)?.values.get(key);
        }
        if field.section != "serverProperties"
            || self
                .profile
                .is_some_and(|profile| field.binding(profile) != Binding::Properties)
        {
            return None;
        }
        let rows = self.properties.get(field.key)?;
        (rows.len() == 1).then(|| &rows[0])
    }

    fn reason(&self, field: &Field) -> &str {
        let Some(profile) = self.profile else {
            return "profileUnknown";
        };
        let (status, reason) = field.gate(profile, true);
        if status != "available" {
            return reason;
        }
        if field.section == "serverProperties" && self.distance_overrides.contains(field.key) {
            return "spigotOverride";
        }
        let Some(row) = self.observe(field) else {
            return "noUniqueValue";
        };
        if !row.editable {
            return "manualFormat";
        }
        if logical_value(field, &row.value, profile).is_err() {
            return "valueOutOfFormat";
        }
        ""
    }

    fn response(&self) -> Value {
        let mut values = Map::new();
        let rows: Vec<_> = FIELDS
            .iter()
            .map(|field| {
                let mut row = field.describe(self.profile, true);
                let reason = self.reason(field);
                let editable = reason.is_empty();
                let value = self.observe(field).map(|observed| {
                    let key = if field.section == "serverProperties" {
                        field.key.to_owned()
                    } else {
                        field.id()
                    };
                    values.insert(key, json!(observed.value));
                    self.profile
                        .and_then(|profile| logical_value(field, &observed.value, profile).ok())
                        .unwrap_or_else(|| observed.value.clone())
                });
                row["value"] = json!(value);
                row["editable"] = json!(editable);
                if editable {
                    row["file"] = json!(self.source(field).0);
                }
                row["reason"] = json!(reason_text(reason));
                row["reasonCode"] = json!(reason);
                if !editable && row["status"] == "available" {
                    row["status"] = json!("unverified");
                }
                row
            })
            .collect();
        json!({"revision":self.revision,"values":values,"rows":rows,
            "profile":self.profile.map(profile_json),"capabilities":{"gamerules":"unavailable"}})
    }

    fn plan(&self, input: &str) -> Result<(Option<FilePatch<'_>>, Vec<Value>, String)> {
        let request: Value = serde_json::from_str(input).map_err(fail)?;
        let request = request.as_object().ok_or("InvalidWorldSettings")?;
        if request.len() != 2 && !(request.len() == 3 && request.contains_key("preview")) {
            return Err("InvalidWorldSettings".into());
        }
        if request_revision(request)? != self.revision {
            return Err("SettingsConflict".into());
        }
        let profile = self.profile.ok_or("WorldProfileUnverified")?;
        let changes = request
            .get("values")
            .and_then(Value::as_object)
            .ok_or("InvalidWorldSettings")?;
        let mut operations = BTreeMap::new();
        let mut diff = Vec::new();
        let mut destination = None;
        let mut pack_edits = Map::new();
        for (id, value) in changes {
            let field = if id.contains('.') || matches!(id.as_str(), "initialTime" | "worldType") {
                field(id)?
            } else {
                property_field(id)?
            };
            let reason = self.reason(field);
            if !reason.is_empty() {
                return Err(format!("WorldFieldUnavailable: {id} ({})", reason_text(reason)));
            }
            let before = self.observe(field).ok_or("WorldFieldUnconfirmed")?;
            let target = field.normalize(value.as_str().ok_or("InvalidWorldSettings")?, profile)?;
            let current = logical_value(field, &before.value, profile)?;
            if is_resource_pack(field.key) {
                pack_edits.insert(field.id(), json!(target));
            }
            if current != target {
                let (path, source) = self.source(field);
                if destination.is_some_and(|(previous, _)| previous != path) {
                    return Err("WorldSettingsMultipleFiles".into());
                }
                destination = Some((path, source));
                let replacement = encode_settings_value(field, &target, profile)?;
                if operations
                    .insert(before.start, (before.end, replacement))
                    .is_some()
                {
                    return Err(format!("DuplicateWorldField: {id}"));
                }
                diff.push(json!({"key":field.id(),"before":current,"after":target,"path":path}));
            }
        }
        if !pack_edits.is_empty() {
            for field in FIELDS.iter().filter(|field| is_resource_pack(field.key)) {
                let id = field.id();
                if !pack_edits.contains_key(&id) {
                    let observed = self.observe(field).ok_or("ResourcePackGroupUnconfirmed")?;
                    let inherited = logical_value(field, &observed.value, profile)?;
                    pack_edits.insert(id, json!(inherited));
                }
            }
            resource_pack::validate_group(&pack_edits)?;
        }
        let patch = destination.map(|(path, original)| {
            let mut updated = original.to_owned();
            for (start, (end, value)) in operations.into_iter().rev() {
                updated.replace_range(start..end, &value);
            }
            FilePatch {
                path,
                original,
                updated,
            }
        });
        let digest = content_revision::revision(
            &serde_json::to_vec(&json!([self.revision, diff])).map_err(fail)?,
        )?;
        if let Some(expected) = request.get("preview") {
            if expected != &digest {
                return Err("WorldPreviewChanged".into());
            }
        }
        Ok((patch, diff, digest))
    }
}

#[derive(Debug)]
struct FilePatch<'a> {
    path: &'static str,
    original: &'a str,
    updated: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeferredWorldPatch {
    original: String,
    updated: String,
    values: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingWorld {
    #[serde(default = "queued_world_state")]
    state: String,
    #[serde(default)]
    error: Option<String>,
    revision: String,
    core_sha256: String,
    registration_revision: String,
    directory: PathBuf,
    original_sources: BTreeMap<String, String>,
    patches: BTreeMap<String, DeferredWorldPatch>,
    #[serde(default)]
    port_transition: Option<(u16, u16)>,
}

fn queued_world_state() -> String { "Queued".into() }

fn source_texts(snapshot: &WorldSnapshot) -> BTreeMap<String, String> {
    let mut sources = BTreeMap::from([("server.properties".into(), snapshot.text.clone())]);
    for yaml in [snapshot.spigot.as_ref(), snapshot.nether.as_ref()].into_iter().flatten() {
        sources.insert(yaml.path.into(), yaml.text.clone());
    }
    sources
}

fn core_sha256(root: &Path, name: &str) -> Result<String> {
    content_revision::file_revision(&core_artifact::jar(&instance_path(root, name)?, &instance(root, name)?))
}

fn pending_world_values(pending: &PendingWorld) -> Map<String, Value> {
    pending.patches.values().flat_map(|patch| patch.values.iter())
        .map(|(key, value)| (key.clone(), value.clone())).collect()
}

pub(super) fn deferred_world_properties(root: &Path, name: &str) -> Result<Option<(String, String)>> {
    let Some(pending) = deferred_settings::load::<PendingWorld>(root, "world", name)? else {
        return Ok(None);
    };
    if pending.state != "Applying" { return Ok(None); }
    Ok(pending.patches.get("server.properties")
        .map(|patch| (patch.original.clone(), patch.updated.clone())))
}

fn sources_compatible(pending: &PendingWorld, current: &BTreeMap<String, String>) -> bool {
    pending.original_sources.keys().eq(current.keys())
        && pending.original_sources.iter().all(|(source, original)| {
            let observed = current.get(source);
            observed == Some(original)
                || observed == pending.patches.get(source).map(|patch| &patch.updated)
        })
}

fn queue_world_settings(root: &Path, name: &str, snapshot: &WorldSnapshot, input: &str) -> Result<Value> {
    let (patch, changes, _) = snapshot.plan(input)?;
    if patch.is_none() && deferred_settings::load::<PendingWorld>(root, "world", name)?.is_none() {
        let mut response = snapshot.response();
        response["queued"] = json!(false);
        response["applied"] = json!(true);
        return Ok(response);
    }
    let request: Value = serde_json::from_str(input).map_err(fail)?;
    let values = request["values"].as_object().ok_or("InvalidWorldSettings")?;
    let state = status(root, &instance(root, name)?)?;
    if !matches!(state["state"].as_str(), Some("Starting" | "Listening" | "Stopping")) {
        return Err("WorldSettingsStateUnconfirmed".into());
    }
    let original_sources = source_texts(snapshot);
    let sha = core_sha256(root, name)?;
    let registration = registration_revision(root, name)?;
    let directory = instance_path(root, name)?;
    let mut pending = match deferred_settings::load::<PendingWorld>(root, "world", name)? {
        Some(prior) => {
            if prior.revision != snapshot.revision || prior.core_sha256 != sha
                || prior.registration_revision != registration
                || prior.directory != directory || prior.original_sources != original_sources {
                return Err("SettingsConflict".into());
            }
            prior
        }
        None => PendingWorld { state: queued_world_state(), error: None,
            revision: snapshot.revision.clone(), core_sha256: sha,
            registration_revision: registration, directory,
            original_sources, patches: BTreeMap::new(), port_transition: None },
    };
    let source = patch.as_ref().map(|patch| patch.path.to_owned()).or_else(|| {
        pending.patches.iter().find(|(_, prior)| values.keys().any(|key| prior.values.contains_key(key)))
            .map(|(name, _)| name.clone())
    });
    if let Some(source) = source {
        let mut merged = pending.patches.get(&source)
            .map(|prior| prior.values.clone()).unwrap_or_default();
        merged.extend(values.iter().map(|(key, value)| (key.clone(), value.clone())));
        let merged_request = json!({"revision":snapshot.revision,"values":merged});
        let (combined, _, _) = snapshot.plan(&merged_request.to_string())?;
        if let Some(combined) = combined {
            pending.patches.insert(combined.path.into(), DeferredWorldPatch {
                original: combined.original.into(), updated: combined.updated,
                values: merged,
            });
        } else {
            pending.patches.remove(&source);
        }
    }
    if pending.patches.is_empty() {
        deferred_settings::clear(root, "world", name)?;
    } else {
        deferred_settings::store(root, "world", name, &pending)?;
    }
    let mut response = snapshot.response();
    response["changes"] = json!(changes);
    response["queued"] = json!(true);
    response["applied"] = json!(false);
    response["pending"] = json!({"state":pending.state,"values":pending_world_values(&pending)});
    yaml_preflight::add_to_response(root, name, snapshot, &mut response);
    Ok(response)
}

pub(super) fn apply_deferred_world(root: &Path, name: &str) -> Result<()> {
    let Some(mut pending) = deferred_settings::load::<PendingWorld>(root, "world", name)? else {
        return Ok(());
    };
    let snapshot = WorldSnapshot::read(root, name)?;
    if pending.state == "Applying" {
        if core_sha256(root, name)? != pending.core_sha256
            || instance_path(root, name)? != pending.directory
            || status(root, &instance(root, name)?)?["state"] != "Stopped" {
            return Err("DeferredWorldConflict".into());
        }
        let current = source_texts(&snapshot);
        for (source, original) in &pending.original_sources {
            let expected = pending.patches.get(source).map_or(original, |patch| &patch.updated);
            if current.get(source) != Some(expected) {
                if source == "server.properties" {
                    if let Some((_, world_target)) = deferred_world_properties(root, name)? {
                        if let Some((old, new)) = pending.port_transition {
                                let rebased = super::presentation::replace_port_for_deferred(&world_target, old, new)?;
                                if current.get(source) == Some(&rebased) { continue; }
                        }
                    }
                }
                return Err("DeferredWorldConflict".into());
            }
        }
        return Ok(());
    }
    assert_world_stopped(root, name, &snapshot)?;
    if core_sha256(root, name)? != pending.core_sha256
        || registration_revision(root, name)? != pending.registration_revision
        || instance_path(root, name)? != pending.directory {
        return Err("DeferredWorldCoreChanged".into());
    }
    let current_sources = source_texts(&snapshot);
    if !sources_compatible(&pending, &current_sources) {
        return Err("DeferredWorldConflict".into());
    }
    let directory = instance_path(root, name)?;
    for (source, patch) in &pending.patches {
        let path = directory.join(source);
        no_redirect(&path)?;
        let current_text = file_text(&path)?;
        if current_text == patch.updated {
            continue;
        }
        if current_text != patch.original {
            return Err("DeferredWorldConflict".into());
        }
        let backup_folder = child_directory(root, "history")?;
        let backup = backup_folder.join(format!("world-settings-{name}-{}-{}-{}",
            std::process::id(), clock(), source.rsplit('/').next().ok_or("InvalidInstancePath")?));
        let mut backup_file = OpenOptions::new().write(true).create_new(true).open(&backup).map_err(fail)?;
        backup_file.write_all(patch.original.as_bytes()).map_err(fail)?;
        backup_file.sync_all().map_err(fail)?;
        drop(backup_file);
        if file_text(&path)? != patch.original {
            return Err("DeferredWorldConflict".into());
        }
        replace_file(&path, patch.updated.as_bytes())?;
        if file_text(&path)? != patch.updated {
            return Err("DeferredWorldReadbackMismatch".into());
        }
    }
    let yaml_keys = pending.patches.iter()
        .filter(|(file, _)| file.ends_with(".yml"))
        .flat_map(|(_, patch)| patch.values.keys().map(String::as_str))
        .collect::<BTreeSet<_>>();
    if !yaml_keys.is_empty() {
        let readback = WorldSnapshot::read(root, name)?;
        let originals = pending.patches.iter()
            .filter(|(file, _)| file.ends_with(".yml"))
            .map(|(file, patch)| Ok((file.as_str(), content_revision::revision(patch.original.as_bytes())?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        yaml_preflight::record_save(root, name, &readback, &yaml_keys, &originals)?;
    }
    if deferred_settings::load::<Value>(root, "instance", name)?.is_some() {
        pending.port_transition = super::presentation::deferred_ports(root, name)?;
        pending.state = "Applying".into();
        pending.error = None;
        deferred_settings::store(root, "world", name, &pending)
    } else {
        deferred_settings::clear(root, "world", name)
    }
}

fn assert_world_stopped(root: &Path, name: &str, snapshot: &WorldSnapshot) -> Result<()> {
    let config = instance(root, name)?;
    let server_port = snapshot
        .properties
        .get("server-port")
        .filter(|rows| rows.len() == 1)
        .and_then(|rows| rows[0].value.parse::<u16>().ok());
    if server_port != Some(config.port) {
        return Err(
            "WorldSettingsPortUnconfirmed: confirm the registered port matches server.properties"
                .into(),
        );
    }
    if status(root, &config)?["state"] != "Stopped" {
        return Err("InstanceMustBeStopped".into());
    }
    port_free(config.port)?;
    scan_policy::verify_import_idle(snapshot.path.parent().ok_or("InvalidInstancePath")?)?;
    Ok(())
}

pub(super) fn diagnostic_snapshot(root: &Path, name: &str, core: &CoreProfile) -> Result<Value> {
    Ok(WorldSnapshot::read_with_core(root, name, Some(core))?.response())
}

fn world_settings(root: &Path, name: &str, input: Option<&str>) -> Result<Value> {
    let snapshot = WorldSnapshot::read(root, name)?;
    let Some(input) = input else {
        let mut response = snapshot.response();
        if let Some(pending) = deferred_settings::load::<PendingWorld>(root, "world", name)? {
            response["pending"] = json!({"state":pending.state,"values":pending_world_values(&pending),"error":pending.error});
        }
        yaml_preflight::add_to_response(root, name, &snapshot, &mut response);
        return Ok(response);
    };
    let (_, diff, preview) = snapshot.plan(input)?;
    Ok(json!({"revision":snapshot.revision,"changes":diff,"preview":preview}))
}

fn save_world_settings(root: &Path, name: &str, input: &str) -> Result<Value> {
    let snapshot = WorldSnapshot::read(root, name)?;
    let (patch, diff, _) = snapshot.plan(input)?;
    if status(root, &instance(root, name)?)?["state"] != "Stopped" {
        return queue_world_settings(root, name, &snapshot, input);
    }
    let resubmitting_conflict = deferred_settings::state(root, "world", name)?.as_deref() == Some("Conflict")
        && deferred_settings::state(root, "instance", name)?.is_none();
    if deferred_settings::has_pending(root, name)? && !resubmitting_conflict {
        deferred_settings::apply_locked(root, name)?;
        // The submitted revision described the pre-commit files. Require a
        // fresh snapshot instead of replaying a stale patch over queued edits.
        return Err("SettingsConflict".into());
    }
    assert_world_stopped(root, name, &snapshot)?;
    let Some(patch) = patch else {
        let mut response = snapshot.response();
        yaml_preflight::add_to_response(root, name, &snapshot, &mut response);
        return Ok(response);
    };
    if patch.path != "server.properties"
        && serde_json::from_str::<Value>(input)
            .map_err(fail)?
            .get("preview")
            .is_none()
    {
        return Err("WorldPreviewRequired".into());
    }

    let directory = root.join("history");
    no_redirect(&directory)?;
    let backup = directory.join(format!(
        "world-settings-{name}-{}-{}-{}",
        std::process::id(),
        clock(),
        patch.path.rsplit('/').next().ok_or("InvalidInstancePath")?
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
        .map_err(fail)?;
    file.write_all(patch.original.as_bytes()).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    drop(file);
    let rechecked = WorldSnapshot::read(root, name)?;
    if rechecked.revision != snapshot.revision {
        return Err("SettingsConflict".into());
    }
    assert_world_stopped(root, name, &rechecked)?;
    let path = snapshot
        .path
        .parent()
        .ok_or("InvalidInstancePath")?
        .join(patch.path);
    no_redirect(&path)?;
    replace_file(&path, patch.updated.as_bytes())?;
    let readback = WorldSnapshot::read(root, name)?;
    let actual = if patch.path == "server.properties" {
        Some(readback.text.as_str())
    } else {
        readback.yaml(patch.path).map(|yaml| yaml.text.as_str())
    };
    if actual != Some(patch.updated.as_str()) {
        return Err(format!(
            "WorldSettingsReadbackMismatch: {}",
            backup.display()
        ));
    }
    if patch.path.ends_with(".yml") {
        let yaml_keys = diff.iter().filter_map(|change| change["key"].as_str()).collect();
        let originals = BTreeMap::from([(patch.path, content_revision::revision(patch.original.as_bytes())?)]);
        yaml_preflight::record_save(root, name, &readback, &yaml_keys, &originals)?;
    }
    let mut response = readback.response();
    response["changes"] = json!(diff);
    if resubmitting_conflict {
        deferred_settings::clear(root, "world", name)?;
    }
    yaml_preflight::add_to_response(root, name, &readback, &mut response);
    Ok(response)
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    match (verb, args) {
        ("get-initialization", []) => Some(
            initialization(root)
                .and_then(|values| initialization_response(&values, default_profile())),
        ),
        ("get-initialization", [core, version]) => {
            Some(Profile::resolve(core, version).and_then(|profile| {
                initialization(root).and_then(|values| initialization_response(&values, profile))
            }))
        }
        ("save-initialization", [input]) => Some(save_initialization(root, input)),
        ("world-batch-preview", [request]) => Some(world_batch_preview::preview(root, request)),
        ("world-batch-preview", _) => Some(Err("InvalidWorldPreviewTargets".into())),
        ("world-settings", [name]) => Some(world_settings(root, name, None)),
        ("world-settings", [name, input]) => Some(world_settings(root, name, Some(input))),
        ("save-world-settings", [name, input]) => Some(save_world_settings(root, name, input)),
        (
            "get-initialization" | "save-initialization" | "world-settings" | "save-world-settings",
            _,
        ) => Some(Err("InvalidSettingsArguments".into())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(text: &str, profile: Option<Profile>) -> WorldSnapshot {
        WorldSnapshot {
            path: PathBuf::from("server.properties"),
            text: text.into(),
            properties: properties(text).unwrap(),
            profile,
            core_sha256: None,
            revision: "fixture".into(),
            distance_overrides: BTreeSet::new(),
            nether: None,
            spigot: None,
        }
    }

    #[test]
    fn deferred_world_accepts_only_original_or_exact_completed_files() {
        let pending = PendingWorld {
            state: "Queued".into(), error: None, revision: "original".into(),
            core_sha256: "core".into(), registration_revision: "registration".into(),
            directory: PathBuf::from("fixture"),
            original_sources: BTreeMap::from([
                ("server.properties".into(), "a=1\n".into()),
                ("spigot.yml".into(), "a: 1\n".into()),
            ]),
            patches: BTreeMap::from([
                ("server.properties".into(), DeferredWorldPatch {
                    original: "a=1\n".into(), updated: "a=2\n".into(), values: Map::new(),
                }),
            ]),
            port_transition: None,
        };
        assert!(sources_compatible(&pending, &pending.original_sources));
        assert!(sources_compatible(&pending, &BTreeMap::from([
            ("server.properties".into(), "a=2\n".into()),
            ("spigot.yml".into(), "a: 1\n".into()),
        ])));
        assert!(!sources_compatible(&pending, &BTreeMap::from([
            ("server.properties".into(), "a=3\n".into()),
            ("spigot.yml".into(), "a: 1\n".into()),
        ])));
        assert!(!sources_compatible(&pending, &BTreeMap::from([
            ("server.properties".into(), "a=2\n".into()),
            ("spigot.yml".into(), "a: 2\n".into()),
        ])));
    }

    fn plan(
        snapshot: &WorldSnapshot,
        values: Value,
    ) -> Result<(Option<FilePatch<'_>>, Vec<Value>, String)> {
        snapshot.plan(&json!({"revision":"fixture","values":values}).to_string())
    }

    #[test]
    fn catalog_covers_inventory_and_explicit_version_bindings() {
        let profiles = PROFILES
            .iter()
            .map(|(_, versions)| versions.len())
            .sum::<usize>();
        assert_eq!((FIELDS.len(), profiles), (49, 13));
        let paper = default_profile();
        let spigot = Profile::resolve("spigot", "26.2").unwrap();
        let folia = Profile::resolve("FOLIA", "26.1.2").unwrap();
        let old = Profile::resolve("SPIGOT", "1.8.8").unwrap();
        let nether = property_field("allow-nether").unwrap();
        assert_eq!(
            nether.describe(Some(paper), false)["source"],
            "config/paper-global.yml · misc.enable-nether"
        );
        assert_eq!(
            nether.describe(Some(folia), false)["source"],
            "config/paper-global.yml · misc.enable-nether"
        );
        assert_eq!(
            nether.describe(Some(spigot), false)["source"],
            "bukkit.yml · settings.allow-nether"
        );
        assert_eq!(
            nether.describe(Some(old), false)["source"],
            "server.properties · allow-nether"
        );
        assert_eq!(
            property_field("pvp").unwrap().describe(Some(paper), true)["source"],
            "遊戲規則 · minecraft:pvp"
        );
        assert_eq!(
            field("gamerules.advance_time")
                .unwrap()
                .describe(Some(old), true)["source"],
            "遊戲規則 · doDaylightCycle"
        );
        assert_eq!(
            field("gamerules.max_entity_cramming")
                .unwrap()
                .gate(old, false)
                .0,
            "unsupported"
        );
        assert_eq!(
            field("initialTime").unwrap().gate(paper, true).0,
            "unverified"
        );
        assert!(Profile::resolve("PAPER", "999.0").is_err());
    }

    #[test]
    fn creation_combines_legacy_defaults_with_only_the_target_profile() {
        let mut initial = defaults();
        initial["serverProperties"] = json!({"difficulty":"2"});
        initial["profiles"] = json!({
            "PAPER@26.3":{"serverProperties.max-players":"8","serverProperties.motd":" 中文", "worldType":"Flat"},
            "SPIGOT@1.8.8":{"serverProperties.pvp":"false", "worldType":"Normal"}
        });
        valid_initialization(initial.as_object().unwrap()).unwrap();
        let paper = creation_properties(&initial, 25565, default_profile()).unwrap();
        let legacy = creation_properties(
            &initial,
            25566,
            Profile::resolve("spigot", "1.8.8").unwrap(),
        )
        .unwrap();
        assert!(paper.contains("max-players=8\n") && paper.contains("simulation-distance=4\n"));
        assert!(
            paper.contains("motd=\\u0020\\u4e2d\\u6587\n")
                && paper.contains("level-type=minecraft:flat\n")
        );
        assert!(paper.contains("difficulty=normal\n") && !paper.contains("pvp="));
        assert!(legacy.contains("max-players=20\n") && !legacy.contains("simulation-distance"));
        assert!(
            legacy.contains("difficulty=2\n")
                && legacy.contains("pvp=false\n")
                && legacy.contains("level-type=DEFAULT\n")
        );
    }

    #[test]
    fn selected_properties_preserve_other_bytes_and_bind_the_preview() {
        let source =
            "# custom\r\nserver-port=25565\nmax-players : 20\r\nmotd=\\u4e2d\nview-distance=4";
        let snapshot = snapshot(source, Some(default_profile()));
        let values = json!({"serverProperties.max-players":"8","serverProperties.motd":"中文"});
        let (patched, changes, preview) = plan(&snapshot, values.clone()).unwrap();
        assert_eq!(patched.unwrap().updated, "# custom\r\nserver-port=25565\nmax-players : 8\r\nmotd=\\u4e2d\\u6587\nview-distance=4");
        assert_eq!(
            changes,
            json!([
                {"key":"serverProperties.max-players","before":"20","after":"8","path":"server.properties"},
                {"key":"serverProperties.motd","before":"中","after":"中文","path":"server.properties"}
            ])
            .as_array()
            .unwrap()
            .to_owned()
        );
        let request = json!({"revision":"fixture","values":values,"preview":preview});
        assert!(snapshot.plan(&request.to_string()).is_ok());
        let mut modified = request;
        modified["values"]["serverProperties.max-players"] = json!("9");
        assert_eq!(
            snapshot.plan(&modified.to_string()).unwrap_err(),
            "WorldPreviewChanged"
        );
        assert!(plan(&snapshot, json!({"max-players":"20"}))
            .unwrap()
            .0
            .is_none());
    }

    #[test]
    fn nether_selects_one_verified_file_and_does_not_patch_other_bytes() {
        for (family, version, path, key) in [
            ("SPIGOT", "26.2", "bukkit.yml", "settings.allow-nether"),
            (
                "PAPER",
                "26.3",
                "config/paper-global.yml",
                "misc.enable-nether",
            ),
            (
                "FOLIA",
                "26.1.2",
                "config/paper-global.yml",
                "misc.enable-nether",
            ),
        ] {
            let mut snapshot = snapshot(
                "allow-nether=true\r\nmax-players=20\r\n",
                Some(Profile::resolve(family, version).unwrap()),
            );
            let (section, short_key) = key.split_once('.').unwrap();
            let text = format!("{section}:\r\n  {short_key}: true  # comment\r\n  unchanged: 100\r\nother: true\r\n");
            snapshot.nether = Some(YamlSetting {
                path,
                values: BTreeMap::from([(key, yaml::boolean(&text, key).unwrap())]),
                text: text.clone(),
            });
            let (patch, diff, preview) =
                plan(&snapshot, json!({"serverProperties.allow-nether":"false"})).unwrap();
            let patch = patch.unwrap();
            assert_eq!(patch.path, path);
            assert_eq!(
                patch.updated,
                text.replace(
                    &format!("{short_key}: true"),
                    &format!("{short_key}: false")
                )
            );
            assert_eq!(diff[0]["path"], path);
            assert_eq!(snapshot.properties["allow-nether"][0].value, "true");
            assert_eq!(
                plan(
                    &snapshot,
                    json!({"allow-nether":"false","max-players":"18"})
                )
                .unwrap_err(),
                "WorldSettingsMultipleFiles"
            );
            let request =
                json!({"revision":"fixture","values":{"allow-nether":"false"},"preview":preview});
            snapshot.revision = "changed".into();
            assert_eq!(
                snapshot.plan(&request.to_string()).unwrap_err(),
                "SettingsConflict"
            );
        }
    }
    #[test]
    fn existing_patches_fail_closed_on_unverified_or_ambiguous_inputs() {
        let source = "max-players=20\nmax\\-players=21\nmotd=One\\\r\n  Two\nview-distance=8\ndifficulty=easy\n";
        let mut snapshot = snapshot(source, Some(default_profile()));
        snapshot.distance_overrides.insert("view-distance");
        for selected in [
            json!({"max-players":"9"}),
            json!({"motd":"Three"}),
            json!({"view-distance":"10"}),
            json!({"difficulty":"bad"}),
            json!({"pvp":"true"}),
            json!({"server-ip":"127.0.0.2"}),
            json!({"generate-structures":"true"}),
            json!({"initialTime":"1000"}),
        ] {
            assert!(plan(&snapshot, selected).is_err());
        }
        snapshot.profile = None;
        assert_eq!(
            plan(&snapshot, json!({"difficulty":"hard"})).unwrap_err(),
            "WorldProfileUnverified"
        );
    }
}
