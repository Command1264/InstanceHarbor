//! Small presentation-facing commands for the isolated Windows preview.
//! The parent dispatch selects the appropriate manager or instance lock.
use super::*;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    io::{Seek, SeekFrom},
    sync::OnceLock,
};

const LOG_LIMIT: u64 = 128 * 1024;
const DEFAULT_CONSOLE_TITLE: &str = "{instance}";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    theme: String,
    #[serde(default, rename = "selectionColor")]
    selection_color: SelectionColor,
    #[serde(default = "default_selection_opacity", rename = "selectionOpacity")]
    selection_opacity: u8,
    pins: Vec<String>,
    #[serde(default = "default_path_separator", rename = "pathSeparator")]
    path_separator: String,
    #[serde(default)]
    language: DisplayLanguage,
    #[serde(default, rename = "timeFormat")]
    time_format: TimeFormat,
    #[serde(default = "default_console_title", rename = "consoleTitleTemplate")]
    console_title_template: String,
    #[serde(default, rename = "consoleTitleOverrides")]
    console_title_overrides: BTreeMap<String, String>,
    #[serde(default, rename = "corePrefetch")]
    core_prefetch: bool,
    /// Keep the GUI running with its window hidden after X, so the next open is instant (#56).
    /// Off is left out of the file, so versions that reject unknown fields still read it.
    #[serde(default, rename = "fastOpen", skip_serializing_if = "std::ops::Not::not")]
    fast_open: bool,
    // No longer used: the Manager owns the tray and closing the window ends the GUI.
    // Kept so files from 1.0.0 still load and a rollback keeps the user's choice.
    // Omit the default so 1.0.0-rc2, which rejects unknown fields, can still read the file.
    #[serde(default = "default_close_to_tray", rename = "closeToTray", skip_serializing_if = "is_default_close_to_tray")]
    close_to_tray: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SelectionColor {
    #[default]
    Blue,
    // Read previously saved preferences without discarding them.
    Product,
    System,
    Red,
    Green,
    Magenta,
    Yellow,
    Cyan,
    Purple,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TimeFormat {
    #[default]
    Local24,
    Local12,
    Utc,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(super) enum DisplayLanguage {
    #[serde(rename = "zh-TW")]
    TraditionalChinese,
    #[serde(rename = "en")]
    English,
    /// The default: follow the Windows display language, falling back to English.
    #[default]
    #[serde(rename = "system")]
    System,
}

fn default_path_separator() -> String {
    "forward".into()
}

fn default_selection_opacity() -> u8 {
    25
}

fn default_console_title() -> String {
    DEFAULT_CONSOLE_TITLE.into()
}

fn default_close_to_tray() -> bool {
    true
}

fn is_default_close_to_tray(value: &bool) -> bool {
    *value == default_close_to_tray()
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            selection_color: SelectionColor::default(),
            selection_opacity: default_selection_opacity(),
            pins: vec![],
            path_separator: default_path_separator(),
            language: DisplayLanguage::default(),
            time_format: TimeFormat::default(),
            console_title_template: default_console_title(),
            console_title_overrides: BTreeMap::new(),
            core_prefetch: false,
            fast_open: false,
            close_to_tray: default_close_to_tray(),
        }
    }
}

fn preferences_path(root: &Path) -> PathBuf {
    root.join("preferences.json")
}

/// Whether X only hides the GUI window. An unreadable file means the default, off.
/// `root` may be spelled as the launcher gave it; the file is read through its canonical
/// path, as every other command does.
pub fn fast_open_enabled(root: &Path) -> bool {
    fs::canonicalize(root)
        .map_err(fail)
        .and_then(|root| load_preferences(&root))
        .is_ok_and(|preferences| preferences.fast_open)
}

pub(super) fn core_prefetch_enabled(root: &Path) -> Result<bool> {
    Ok(load_preferences(root)?.core_prefetch)
}

/// Whether Manager-owned text such as the tray menu uses Traditional Chinese.
#[cfg_attr(test, allow(dead_code))]
pub(super) fn chinese_ui(root: &Path) -> bool {
    chinese_interface(load_preferences(root).map(|preferences| preferences.language).unwrap_or_default())
}

fn load_preferences(root: &Path) -> Result<Preferences> {
    Ok(load_preferences_version(root)?.0)
}

fn load_preferences_version(root: &Path) -> Result<(Preferences, String, bool)> {
    let path = preferences_path(root);
    if !verify_preferences_path(&path)? {
        return Ok((Preferences::default(), content_revision::revision(b"")?, false));
    }
    let bytes = retry_shared_file(|| fs::read(&path)).map_err(|error| file_error("read", &path, error))?;
    let value: Preferences =
        serde_json::from_slice(&bytes).map_err(|error| format!("PreferencesInvalidJson: preferences.json: {error}"))?;
    if !matches!(value.theme.as_str(), "system" | "light" | "dark")
        || !matches!(value.path_separator.as_str(), "forward" | "native")
        || value.selection_opacity > 100
    {
        return Err("PreferencesInvalidValues".into());
    }
    validate_console_preferences(&value)?;
    Ok((value, content_revision::revision(&bytes)?, true))
}

fn verify_preferences_path(path: &Path) -> Result<bool> {
    use std::os::windows::fs::MetadataExt;
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() || meta.file_attributes() & 0x400 != 0 => {
            Err("PreferencesUnexpectedFileType".into())
        }
        Ok(_) => {
            let resolved =
                retry_shared_file(|| fs::canonicalize(path)).map_err(|error| file_error("resolve", path, error))?;
            if resolved != path {
                return Err("PathRedirected".into());
            }
            Ok(true)
        }
        Err(error) if error.raw_os_error() == Some(2) => Ok(false),
        Err(error) => Err(file_error("inspect", path, error)),
    }
}

pub(super) fn registry_records(root: &Path) -> Result<Vec<Instance>> {
    let mut rows = vec![];
    for entry in fs::read_dir(root.join("registry")).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or("InvalidInstanceName")?;
        if !instance_name(name) {
            return Err("InvalidInstanceName".into());
        }
        rows.push(instance(root, name)?);
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

fn validate_preferences(value: &Preferences, names: &[String]) -> Result<()> {
    if !matches!(value.theme.as_str(), "system" | "light" | "dark") {
        return Err("InvalidTheme".into());
    }
    if !matches!(value.path_separator.as_str(), "forward" | "native") {
        return Err("InvalidPathSeparator".into());
    }
    if value.selection_opacity > 100 {
        return Err("InvalidSelectionOpacity".into());
    }
    validate_console_preferences(value)?;
    let mut seen = std::collections::HashSet::new();
    for name in &value.pins {
        if !names.contains(name) || !seen.insert(name) {
            return Err("InvalidPinnedInstance".into());
        }
    }
    Ok(())
}

fn title_control(character: char) -> bool {
    character.is_control()
        || matches!(character,
            '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn render_title(template: &str, resolve: impl Fn(&str) -> Option<String>) -> Result<String> {
    if template.trim().is_empty()
        || template.encode_utf16().count() > 256
        || template.chars().any(title_control)
    {
        return Err("InvalidConsoleTitle".into());
    }
    static FIELDS: OnceLock<Vec<[String; 3]>> = OnceLock::new();
    let fields = FIELDS.get_or_init(|| {
        serde_json::from_str(include_str!("../../src/console-title-tokens.json"))
            .expect("embedded console title catalog must be valid")
    });
    let mut rendered = String::new();
    let mut remaining = template;
    while let Some((literal, tail)) = remaining.split_once('{') {
        let (field, rest) = tail.split_once('}').ok_or("InvalidConsoleTitlePlaceholder")?;
        if literal.contains('}') || !fields.iter().any(|entry| entry[0] == field) {
            return Err("InvalidConsoleTitlePlaceholder".into());
        }
        rendered.push_str(literal);
        let mut value = resolve(field).unwrap_or_default();
        value.retain(|character| !title_control(character));
        let cleaned = value.trim();
        rendered.push_str(if cleaned.is_empty() { "N/A" } else { cleaned });
        remaining = rest;
    }
    if remaining.contains('}') {
        return Err("InvalidConsoleTitlePlaceholder".into());
    }
    rendered.push_str(remaining);
    Ok(rendered.chars().take(1024).collect())
}

fn validate_console_preferences(value: &Preferences) -> Result<()> {
    render_title(&value.console_title_template, |_| None)?;
    if value.console_title_overrides.len() > 4096 {
        return Err("TooManyConsoleTitleOverrides".into());
    }
    for (name, template) in &value.console_title_overrides {
        if !instance_name(name) {
            return Err("InvalidConsoleTitleOverride".into());
        }
        render_title(template, |_| None)?;
    }
    Ok(())
}

fn filetime_title(ticks: u64) -> Option<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FileTimeToSystemTime(filetime: *const [u32; 2], output: *mut [u16; 8]) -> i32;
    }
    let filetime = [ticks as u32, (ticks >> 32) as u32];
    let mut parts = [0; 8];
    if unsafe { FileTimeToSystemTime(&filetime, &mut parts) } == 0 {
        return None;
    }
    let [year, month, _, day, hour, minute, second, _] = parts;
    Some(format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC"))
}

pub(super) fn console_appearance(root: &Path, name: &str, run: &str) -> Result<(String, DisplayLanguage)> {
    let config = active_instance(root, name)?;
    let current = status(root, &config)?;
    if current["run"]["id"] != run
        || !matches!(current["state"].as_str(), Some("Listening" | "Starting"))
    {
        return Err("ConsoleRunChanged".into());
    }
    let observed_at = epoch_ms();
    let preferences = load_preferences(root)?;
    let template = preferences.console_title_overrides.get(name)
        .unwrap_or(&preferences.console_title_template);
    let core = OnceLock::new();
    let directory = instance_path(root, name)?;
    let title = render_title(template, |field| {
        let value = match field {
            "instance" => name.into(),
            "path" => path_format::display(&directory),
            "port" => config.port.to_string(),
            "java" => path_format::display(&config.java),
            "state" => current["state"].as_str()?.into(),
            "ownership" => "Managed".into(),
            "mode" => "ConPTY".into(),
            "run_id" => run.into(),
            "server_pid" => current["run"]["java"]["pid"].as_u64()?.to_string(),
            "host_pid" => current["run"]["host"]["pid"].as_u64()?.to_string(),
            "started_at" => filetime_title(current["run"]["java"]["created"].as_u64()?)?,
            "observed_at" => filetime_title(observed_at.checked_mul(10_000)?
                .checked_add(116_444_736_000_000_000)?)?,
            "type" | "minecraft_version" | "server_build" | "server_sha256" => {
                let profile = core.get_or_init(|| {
                    let jar = core_artifact::jar(&directory, &config);
                    no_redirect(&jar).ok()?;
                    let hash = content_revision::file_revision(&jar).ok()?;
                    core_maintenance::trusted_profile(&config, hash)
                }).as_ref()?;
                match field {
                    "type" => profile.family.clone(),
                    "minecraft_version" => profile.minecraft.clone(),
                    "server_build" => profile.build.clone(),
                    _ => profile.sha256.clone(),
                }
            }
            _ => return None,
        };
        Some(value)
    })?;
    Ok((title, preferences.language))
}

pub(super) fn instance_metadata(root: &Path, row: &Instance, path: &Path) -> Value {
    let jar = core_artifact::jar(path, row);
    let mut core = core_metadata(row, &jar, |sha| core_maintenance::trusted_profile(row, sha));
    // How many instances use the same shared core file (#62), counted by its names.
    if let Some(users) = row.core_profile.as_ref().and_then(|profile| shared_core::users(root, &jar, &profile.sha256)) {
        core["sharedUsers"] = json!(users);
    }
    json!({"name":row.name,"java":path_format::display(&row.java),"port":row.port,"path":path_format::display(&path),"core":core,
        "managementDiscovered":row.management_discovered})
}

fn known_core(family: &str, minecraft: &str, build: &str) -> Value {
    let label = if build.is_empty() {
        format!("{family} {minecraft}")
    } else {
        format!("{family} {minecraft} build {build}")
    };
    json!({"state":"known","family":family,"minecraft":minecraft,"build":build,"label":label})
}

/// The core the list and the details panel show. It agrees with core maintenance:
/// the built-in table or a registered profile whose hash matches the current JAR
/// (`trust`) is verified. Otherwise a Paper, Folia or Spigot JAR that states its
/// own version, or an offline-accepted profile with the same hash, is shown and
/// marked `offline`: only the JAR states that version. `reason` is a stable code
/// that the interface translates.
fn core_metadata(row: &Instance, jar: &Path, trust: impl Fn(String) -> Option<CoreProfile>) -> Value {
    let mut core = current_core(row, jar, trust);
    // Accepted without the built-in table or the official catalog (#14 offline use).
    if row.core_profile.as_ref().is_some_and(|profile| profile.unverified) {
        core["unverified"] = json!(true);
    }
    core
}

fn current_core(row: &Instance, jar: &Path, trust: impl Fn(String) -> Option<CoreProfile>) -> Value {
    current_core_with(row, jar, trust, content_revision::file_revision)
}

fn current_core_with(
    row: &Instance,
    jar: &Path,
    trust: impl Fn(String) -> Option<CoreProfile>,
    digest: impl Fn(&Path) -> Result<String>,
) -> Value {
    if no_redirect(jar).is_err() {
        return json!({"state":"unknown","reason":"CorePathUnverified"});
    }
    let identity = core_identity::read_identity(jar);
    let sha = match &identity {
        Ok(Some(identity)) => core_hash(jar, &identity.directory_revision, &digest),
        // Without a readable ZIP directory the file's size and Windows timestamps tell
        // whether it changed; replacing or rewriting the JAR changes them.
        _ => file_stamp(jar).and_then(|stamp| core_hash(jar, &stamp, &digest)),
    };
    let Ok(sha) = sha else {
        return json!({"state":"unknown","reason":"CoreUnreadable"});
    };
    if let Some(profile) = trust(sha.clone()) {
        return known_core(&profile.family, &profile.minecraft, &profile.build);
    }
    let saved = row.core_profile.as_ref().filter(|profile| profile.sha256 == sha);
    let mut core = match (identity, saved) {
        (Ok(Some(identity)), _) => {
            let mut core = known_core(identity.family, &identity.minecraft_version, &identity.build);
            core["directoryRevision"] = json!(identity.directory_revision);
            core
        }
        (_, Some(profile)) => known_core(&profile.family, &profile.minecraft, &profile.build),
        (Ok(None), None) => return json!({"state":"unknown","reason":"CoreProfileUnverified"}),
        (Err(_), None) => return json!({"state":"unknown","reason":"CoreUnreadable"}),
    };
    core["offline"] = json!(true);
    core
}

/// The size and the creation and last-write times of a file, as a cache key.
fn file_stamp(path: &Path) -> Result<String> {
    use std::os::windows::fs::MetadataExt;
    let metadata = fs::metadata(path).map_err(fail)?;
    Ok(format!("file:{}:{}:{}", metadata.file_size(), metadata.creation_time(), metadata.last_write_time()))
}

/// The SHA-256 of a core. The list refreshes every 1.5 s and a core is 40 to 90 MB,
/// so the hash is kept while the file length and `revision` stay the same:
/// the ZIP directory, which records every entry's CRC, or else `file_stamp`.
fn core_hash(jar: &Path, revision: &str, digest: impl Fn(&Path) -> Result<String>) -> Result<String> {
    use std::{collections::HashMap, sync::Mutex};
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (u64, String, String)>>> = OnceLock::new();
    let length = fs::metadata(jar).map_err(fail)?.len();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((saved_length, saved, sha)) = cache.lock().map_err(|_| "CoreHashCacheUnavailable")?.get(jar) {
        if *saved_length == length && saved == revision {
            return Ok(sha.clone());
        }
    }
    let sha = digest(jar)?;
    let mut cache = cache.lock().map_err(|_| "CoreHashCacheUnavailable")?;
    if cache.len() >= 64 && !cache.contains_key(jar) {
        cache.clear();
    }
    cache.insert(jar.to_path_buf(), (length, revision.to_owned(), sha.clone()));
    Ok(sha)
}

/// `nativeConsole` reports whether this installation can show a server console
/// in InstanceHarbor's own viewer: through the bundled runtime (tier 1) or the
/// pseudoconsole that ships with Windows (tier 2). Each run still records the
/// tier it chose, which can be lower for an unsupported core or path.
pub(super) fn capabilities(_root: &Path) -> Value {
    let bundled = native_console::bundled_runtime().err();
    let system = native_console::system_runtime().err();
    json!({"instanceSettings":true,"javaDiscovery":true,"javaSelection":true,"logs":true,
        "removeManagement":true,"adopt":true,"openLogFolder":true,
        "deleteServer":true,"removedInstances":true,"openServerFolder":true,
        "coreMaintenance":true,"nativeConsole":bundled.is_none() || system.is_none(),
        "consoleRuntime":{"bundled":bundled.is_none(),"bundledReason":bundled,"system":system.is_none(),"systemReason":system},
        "worldSettings":true,"initialization":true,"scanPaths":true})
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
}

fn chinese_interface(language: DisplayLanguage) -> bool {
    match language {
        DisplayLanguage::TraditionalChinese => true,
        DisplayLanguage::English => false,
        // Like the GUI's resolveLanguage: Traditional Chinese (Taiwan, Hong Kong, Macao), else English.
        DisplayLanguage::System => system_chinese(unsafe { GetUserDefaultUILanguage() }),
    }
}

fn system_chinese(ui_language: u16) -> bool {
    matches!(ui_language, 0x0404 | 0x0C04 | 0x1404)
}

/// Title of a console window run (tier 3). It states the consequence of closing it.
pub(super) fn console_window_title(root: &Path, name: &str) -> String {
    let language = load_preferences(root).map(|preferences| preferences.language).unwrap_or_default();
    let title = if chinese_interface(language) {
        format!("{name} - InstanceHarbor 伺服器後台（關閉此視窗會停止伺服器）")
    } else {
        format!("{name} - InstanceHarbor server console (closing this window stops the server)")
    };
    crate::desktop_channel::window_title(&title, crate::desktop_channel::workspace_of(root).as_deref())
}

/// Turns the VT output a pseudoconsole writes into `stdout.log` into plain
/// lines for the Logs dialog. The file on disk keeps the original bytes.
pub(super) fn readable_terminal(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    let mut parameters = String::new();
                    let mut last = None;
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            last = Some(next);
                            break;
                        }
                        parameters.push(next);
                    }
                    let count = || parameters.parse::<usize>().unwrap_or(1).clamp(1, 500);
                    match last {
                        Some('C') => output.extend(std::iter::repeat_n(' ', count())),
                        Some('H' | 'f') if !output.is_empty() && !output.ends_with('\n') => output.push('\n'),
                        _ => {}
                    }
                }
                Some(']' | 'P' | '_' | '^') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' || (next == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' if chars.peek() == Some(&'\n') => {}
            '\r' => output.truncate(output.rfind('\n').map_or(0, |line| line + 1)),
            '\u{8}' => {
                if !output.ends_with('\n') {
                    output.pop();
                }
            }
            '\n' | '\t' => output.push(ch),
            ch if ch.is_control() => {}
            ch => output.push(ch),
        }
    }
    output
}

fn presentation(root: &Path, include_preferences: bool) -> Result<Value> {
    let mut result = instance_snapshot::query(root, true)?;
    let rows = result.as_object_mut().and_then(|object| object.remove("rows")).unwrap_or_default();
    if include_preferences {
        let mut prefs = load_preferences(root)?;
        prefs.pins.retain(|name| rows.as_array().is_some_and(|rows| rows.iter().any(|row| row["name"].as_str() == Some(name))));
        result["preferences"] = json!(prefs);
    }
    Ok(result)
}

fn save_preferences(root: &Path, raw: &str, expected: Option<&str>) -> Result<Value> {
    let input: Value = serde_json::from_str(raw).map_err(fail)?;
    let mut value: Preferences = serde_json::from_value(input.clone()).map_err(fail)?;
    let path = preferences_path(root);
    let (previous, revision, existed) = load_preferences_version(root)?;
    if expected.is_some_and(|expected| expected != revision) {
        return Err("PreferencesSettingsConflict".into());
    }
    let mut names = previous.pins.clone();
    for name in &value.pins {
        if instance_name(name) && !names.contains(name) && admitted(root, name)? && !instance(root, name)?.removed {
            names.push(name.clone());
        }
    }
    if input.get("selectionColor").is_none() {
        value.selection_color = previous.selection_color;
    }
    if input.get("selectionOpacity").is_none() {
        value.selection_opacity = previous.selection_opacity;
    }
    if input.get("language").is_none() {
        value.language = previous.language;
    }
    if input.get("timeFormat").is_none() {
        value.time_format = previous.time_format;
    }
    if input.get("consoleTitleTemplate").is_none() {
        value.console_title_template = previous.console_title_template;
    }
    if input.get("consoleTitleOverrides").is_none() {
        value.console_title_overrides = previous.console_title_overrides;
    }
    if input.get("corePrefetch").is_none() {
        value.core_prefetch = previous.core_prefetch;
    }
    if input.get("fastOpen").is_none() {
        value.fast_open = previous.fast_open;
    }
    if input.get("closeToTray").is_none() {
        value.close_to_tray = previous.close_to_tray;
    }
    validate_preferences(&value, &names)?;
    if value.core_prefetch && !previous.core_prefetch {
        resident_manager::client(root, "manager-connect", &[])?;
    }
    let bytes = serde_json::to_vec_pretty(&value).map_err(fail)?;
    publish_json(&path, &bytes, existed, || {
        let (_, current, exists) = load_preferences_version(root)?;
        if current != revision || exists != existed {
            return Err("PreferencesSettingsConflict".into());
        }
        Ok(())
    })?;
    Ok(json!({"preferences":value,"revision":content_revision::revision(&bytes)?}))
}

pub(super) fn active_instance(root: &Path, name: &str) -> Result<Instance> {
    if !instance_name(name) || !admitted(root, name)? {
        return Err("InstanceExcludedOrInvalid".into());
    }
    let item = instance(root, name)?;
    if item.removed {
        return Err("InstanceRemovedFromManagement".into());
    }
    Ok(item)
}

pub(super) fn stopped(root: &Path, item: &Instance) -> Result<()> {
    if status(root, item)?["state"] != "Stopped" {
        return Err("InstanceMustBeStopped".into());
    }
    Ok(())
}

fn shim_path(path: &Path) -> bool {
    path.components().any(|part| {
        part.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("shims")
    })
}

fn validated_java(path: &str) -> Result<PathBuf> {
    if shim_path(Path::new(path)) || shim_path(&fs::canonicalize(path).map_err(fail)?) {
        return Err("JavaShimNotAllowed".into());
    }
    java(path)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsInput {
    java: String,
    port: u16,
    revision: String,
    #[serde(default)]
    jvm: Option<Jvm>,
    #[serde(default)]
    timing: Option<LifecycleTiming>,
    #[serde(default, rename = "autoRestart")]
    auto_restart: Option<AutoRestart>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingInstance {
    #[serde(default = "queued_state")]
    state: String,
    #[serde(default)]
    error: Option<String>,
    original_record: String,
    updated_record: String,
    original_properties: String,
    updated_properties: String,
    values: Value,
}

fn queued_state() -> String { "Queued".into() }

fn settings_value(item: &Instance) -> Result<Value> {
    let bytes = serde_json::to_vec(item).map_err(fail)?;
    Ok(
        json!({"name":item.name,"java":path_format::display(&item.java),"canSelectJavaAutomatically":item.can_inherit_java(),"port":item.port,"jvm":item.jvm,"timing":item.timing,"autoRestart":item.auto_restart,"revision":content_revision::revision(&bytes)?}),
    )
}

fn settings_draft(prior: &Instance, raw: &str) -> Result<SettingsInput> {
    let draft: SettingsInput = serde_json::from_str(raw).map_err(fail)?;
    assert_settings_revision(prior, &draft.revision)?;
    Ok(draft)
}

fn assert_settings_revision(instance: &Instance, expected: &str) -> Result<()> {
    if settings_value(instance)?["revision"].as_str() != Some(expected) {
        return Err("SettingsConflict".into());
    }
    Ok(())
}

fn probe_settings_java(executable: &str, options: Option<&Jvm>) -> Result<PathBuf> {
    let arguments = options.map(Jvm::arguments).transpose()?;
    if ["JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "_JAVA_OPTIONS"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
    {
        return Err("JavaEnvironmentOverride".into());
    }
    let path = validated_java(executable)?;
    if let Some(arguments) = arguments {
        probe_java(&path, &arguments).map_err(|error| format!("JvmProbeFailed:{error}"))?;
    }
    Ok(path)
}

fn java_check(root: &Path, name: &str, raw: &str) -> Result<Value> {
    let prior = active_instance(root, name)?;
    let draft = settings_draft(&prior, raw)?;
    let default = Jvm::default();
    let options = draft
        .jvm
        .as_ref()
        .or(prior.jvm.as_ref())
        .unwrap_or(&default);
    let java = if draft.java.trim().is_empty() && prior.can_inherit_java() {
        let mut automatic = prior.clone();
        automatic.java.clear();
        let java = launch_java(root, &automatic, &instance_path(root, name)?)?;
        probe_java(&java, &options.arguments()?).map_err(|error| format!("JvmProbeFailed:{error}"))?;
        java
    } else {
        probe_settings_java(&draft.java, Some(options))?
    };
    assert_settings_revision(&active_instance(root, name)?, &draft.revision)?;
    Ok(json!({"valid":true,"path":path_format::display(&java),"checkedAt":epoch_ms()}))
}

fn replace_port(text: &str, old: u16, new: u16) -> Result<String> {
    let mut found = 0;
    let mut result = String::with_capacity(text.len() + 8);
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let body = body.strip_suffix('\r').unwrap_or(body);
        if let Some(value) = body.strip_prefix("server-port=") {
            found += 1;
            if value.parse::<u16>().ok() != Some(old) {
                return Err("ServerPortRegistryMismatch".into());
            }
            result.push_str(&format!("server-port={new}"));
            result.push_str(&line[body.len()..]);
        } else {
            result.push_str(line);
        }
    }
    if found != 1 {
        return Err("InvalidServerPortProperty".into());
    }
    Ok(result)
}

pub(super) fn replace_port_for_deferred(text: &str, old: u16, new: u16) -> Result<String> {
    replace_port(text, old, new)
}

pub(super) fn deferred_ports(root: &Path, name: &str) -> Result<Option<(u16, u16)>> {
    let Some(pending) = deferred_settings::load::<PendingInstance>(root, "instance", name)? else {
        return Ok(None);
    };
    let original: Instance = serde_json::from_str(&pending.original_record).map_err(fail)?;
    let target: Instance = serde_json::from_str(&pending.updated_record).map_err(fail)?;
    Ok(Some((original.port, target.port)))
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.{}.tmp", std::process::id(), clock()));
    let outcome = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(fail)?;
        file.write_all(bytes).map_err(fail)?;
        file.sync_all().map_err(fail)?;
        drop(file);
        fs::rename(&temp, path).map_err(fail)
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temp);
    }
    outcome
}

fn save_instance_settings(root: &Path, name: &str, raw: &str) -> Result<Value> {
    let prior = active_instance(root, name)?;
    if status(root, &prior)?["state"] != "Stopped" {
        return queue_instance_settings(root, name, &prior, raw);
    }
    let resubmitting_conflict = deferred_settings::state(root, "instance", name)?.as_deref() == Some("Conflict")
        && deferred_settings::state(root, "world", name)?.is_none();
    if deferred_settings::has_pending(root, name)? && !resubmitting_conflict {
        deferred_settings::apply_locked(root, name)?;
    }
    let prior = active_instance(root, name)?;
    stopped(root, &prior)?;
    let input = settings_draft(&prior, raw)?;
    let _port_lock = command_lock(root, "port-registration.lock", false)?;
    if input.port == 0 {
        return Err("InvalidServerPort".into());
    }
    if let Some(ref timing) = input.timing {
        timing.validate()?;
    }
    if input.auto_restart.as_ref().is_some_and(|policy| !policy.valid()) {
        return Err("InvalidAutoRestartPolicy".into());
    }
    let new_java = if input.java.trim().is_empty() && prior.can_inherit_java() {
        PathBuf::new()
    } else {
        probe_settings_java(&input.java, input.jvm.as_ref().or(prior.jvm.as_ref()))?
    };
    assert_settings_revision(&instance(root, name)?, &input.revision)?;
    let folder = instance_path(root, name)?;
    let props = folder.join("server.properties");
    no_redirect(&props)?;
    if property_port(&folder)? != prior.port {
        return Err("ServerPortRegistryMismatch".into());
    }
    let old_text = fs::read_to_string(&props).map_err(fail)?;
    let new_text = replace_port(&old_text, prior.port, input.port)?;
    if input.port != prior.port {
        port_unassigned(root, input.port, name)?;
        port_free(input.port)?;
    }
    let mut updated = prior;
    updated.java = new_java;
    updated.port = input.port;
    if input.jvm.is_some() {
        updated.jvm = input.jvm;
    }
    if input.timing.is_some() {
        updated.timing = input.timing;
    }
    if let Some(policy) = input.auto_restart {
        if policy.on_crash || policy.on_unexpected_exit {
            resident_manager::client(root, "manager-connect", &[])?;
        }
        updated.auto_restart = Some(policy);
    }
    let record = record_path(root, name);
    no_redirect(&record)?;
    if new_text != old_text {
        replace_file(&props, new_text.as_bytes())?;
    }
    if let Err(error) = save(&record, &updated) {
        if new_text != old_text {
            if let Err(rollback) = replace_file(&props, old_text.as_bytes()) {
                return Err(format!(
                    "RegistrySaveFailed:{error}; PropertyRollbackFailed:{rollback}"
                ));
            }
        }
        return Err(error);
    }
    if resubmitting_conflict {
        deferred_settings::clear(root, "instance", name)?;
    }
    settings_value(&updated)
}

fn queue_instance_settings(root: &Path, name: &str, prior: &Instance, raw: &str) -> Result<Value> {
    let _port_lock = command_lock(root, "port-registration.lock", false)?;
    if !matches!(status(root, prior)?["state"].as_str(), Some("Starting" | "Listening" | "Stopping")) {
        return Err("InstanceStateUnconfirmed".into());
    }
    let input = settings_draft(prior, raw)?;
    if input.port == 0 {
        return Err("InvalidServerPort".into());
    }
    if let Some(ref timing) = input.timing { timing.validate()?; }
    if input.auto_restart.as_ref().is_some_and(|policy| !policy.valid()) {
        return Err("InvalidAutoRestartPolicy".into());
    }
    let previous = deferred_settings::load::<PendingInstance>(root, "instance", name)?;
    let mut updated = if let Some(previous) = &previous {
        serde_json::from_str::<Instance>(&previous.updated_record).map_err(fail)?
    } else {
        prior.clone()
    };
    if let Some(jvm) = input.jvm { updated.jvm = Some(jvm); }
    if let Some(timing) = input.timing { updated.timing = Some(timing); }
    if let Some(policy) = input.auto_restart { updated.auto_restart = Some(policy); }
    let new_java = if input.java.trim().is_empty() && prior.can_inherit_java() {
        PathBuf::new()
    } else {
        probe_settings_java(&input.java, updated.jvm.as_ref())?
    };
    let folder = instance_path(root, name)?;
    let props = folder.join("server.properties");
    no_redirect(&props)?;
    if property_port(&folder)? != prior.port {
        return Err("ServerPortRegistryMismatch".into());
    }
    if input.port != prior.port {
        port_unassigned(root, input.port, name)?;
    }
    let record = record_path(root, name);
    no_redirect(&record)?;
    let original_record = fs::read_to_string(&record).map_err(fail)?;
    let original_properties = fs::read_to_string(&props).map_err(fail)?;
    let updated_properties = replace_port(&original_properties, prior.port, input.port)?;
    updated.java = new_java;
    updated.port = input.port;
    let values = settings_value(&updated)?;
    let pending = PendingInstance {
        state: queued_state(), error: None,
        original_record,
        updated_record: serde_json::to_string_pretty(&updated).map_err(fail)?,
        original_properties,
        updated_properties,
        values: values.clone(),
    };
    if let Some(previous) = previous {
        if previous.original_record != pending.original_record
            || previous.original_properties != pending.original_properties
        {
            return Err("SettingsConflict".into());
        }
    }
    // A second observation protects the durable intent from a run that exited
    // during Java probing. The next stopped sweep will apply it automatically.
    assert_settings_revision(&active_instance(root, name)?, &input.revision)?;
    deferred_settings::store(root, "instance", name, &pending)?;
    let mut response = settings_value(prior)?;
    response["queued"] = json!(true);
    response["applied"] = json!(false);
    response["pending"] = json!({"state":pending.state,"values":pending.values});
    Ok(response)
}

pub(super) fn apply_deferred_instance(root: &Path, name: &str) -> Result<()> {
    let Some(pending) = deferred_settings::load::<PendingInstance>(root, "instance", name)? else {
        return Ok(());
    };
    let _port_lock = command_lock(root, "port-registration.lock", false)?;
    let record = record_path(root, name);
    let props = instance_path(root, name)?.join("server.properties");
    no_redirect(&record)?;
    no_redirect(&props)?;
    let current_record = fs::read_to_string(&record).map_err(fail)?;
    let current_properties = fs::read_to_string(&props).map_err(fail)?;
    let original: Instance = serde_json::from_str(&pending.original_record).map_err(fail)?;
    let target: Instance = serde_json::from_str(&pending.updated_record).map_err(fail)?;
    let world_transition = policy_settings::deferred_world_properties(root, name)?;
    let effective_target_properties = if let Some((world_original, world_updated)) = world_transition {
        if world_original != pending.original_properties {
            return Err("DeferredInstanceConflict".into());
        }
        replace_port(&world_updated, original.port, target.port)?
    } else {
        pending.updated_properties.clone()
    };
    if current_record != pending.original_record && current_record != pending.updated_record {
        return Err("DeferredInstanceConflict".into());
    }
    if current_properties != pending.original_properties
        && current_properties != pending.updated_properties
        && current_properties != effective_target_properties
        && policy_settings::deferred_world_properties(root, name)?
            .as_ref().is_none_or(|(_, updated)| &current_properties != updated) {
        return Err("DeferredInstanceConflict".into());
    }
    let current: Instance = serde_json::from_str(&current_record).map_err(fail)?;
    stopped(root, &current)?;
    drop(claim_runtime_directory(&instance_path(root, name)?)?);
    port_free(current.port)?;
    if !target.can_inherit_java() || !target.java.as_os_str().is_empty() {
        let target_java = target.java.to_str().ok_or("InvalidJavaPath")?;
        probe_settings_java(target_java, target.jvm.as_ref())?;
    }
    if target.port != current.port {
        port_unassigned(root, target.port, name)?;
        port_free(target.port)?;
    }
    if current_properties != effective_target_properties {
        policy_settings::replace_file(&props, effective_target_properties.as_bytes())?;
    }
    if current_record == pending.original_record {
        policy_settings::replace_file(&record, pending.updated_record.as_bytes())?;
    }
    if fs::read_to_string(&record).map_err(fail)? != pending.updated_record
        || fs::read_to_string(&props).map_err(fail)? != effective_target_properties {
        return Err("DeferredInstanceReadbackMismatch".into());
    }
    deferred_settings::clear(root, "instance", name)
}

fn log_tail(path: &Path) -> Result<String> {
    no_redirect(path)?;
    let mut file = File::open(path).map_err(fail)?;
    let size = file.metadata().map_err(fail)?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(LOG_LIMIT)))
        .map_err(fail)?;
    let mut bytes = Vec::new();
    file.take(LOG_LIMIT).read_to_end(&mut bytes).map_err(fail)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn logs(root: &Path, name: &str) -> Result<Value> {
    let _ = active_instance(root, name)?;
    let runs = child_directory(root, "runs")?;
    let folder = child_directory(&runs, name)?;
    let identity = folder.join("identity.json");
    let read_run = || -> Result<Option<String>> {
        match fs::symlink_metadata(&identity) {
            Ok(_) => {
                no_redirect(&identity)?;
                Ok(Some(read::<Run>(&identity)?.id))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(fail(error)),
        }
    };
    let run = read_run()?;
    let sources: Vec<_> = [
        ("terminal", "stdout.log"),
        ("stderr", "stderr.log"),
        ("host", "host-diagnostic.json"),
        ("control", "control-error.txt"),
    ].into_iter().map(|(source, name)| {
        let path = folder.join(name);
        let tail = log_tail(&path).map(|text| if source == "terminal" { readable_terminal(&text) } else { text });
        let (text, state) = match tail {
            Ok(text) if !text.is_empty() => (Some(text), "Available"),
            Ok(_) => (None, "Empty"),
            Err(_) if !path.exists() => (None, "Missing"),
            Err(_) => (None, "Unavailable"),
        };
        json!({"source":source,"state":state,"path":path_format::display(&path),"text":text})
    }).collect();
    let terminal = &sources[0];
    if run != read_run()? {
        return Err("LogRunChanged".into());
    }
    Ok(json!({"text":terminal["text"],"path":terminal["path"],"sources":sources,"run":run,
        "consoleReady":folder.join("console-ready").is_file(),"merged":folder.join("native-console").is_file()}))
}

fn open_log_folder(root: &Path, name: &str) -> Result<Value> {
    let _ = active_instance(root, name)?;
    let runs = child_directory(root, "runs")?;
    let folder = child_directory(&runs, name)?;
    Command::new("explorer.exe")
        .arg(&folder)
        .creation_flags(HIDDEN)
        .spawn()
        .map_err(fail)?;
    Ok(json!({"opened":true,"path":path_format::display(&folder)}))
}

/// Whether a management folder scan would register this instance again once its
/// record is gone. A folder that no longer exists cannot be found by a scan.
fn scanned_again(root: &Path, item: &Instance) -> Result<bool> {
    let folder = scan_policy::registered_path(root, &item.name, item.directory.as_deref())?;
    match fs::symlink_metadata(&folder) {
        Ok(_) => scan_policy::load(root)?.0.manages_path(root, &folder),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(fail(error)),
    }
}

pub(super) fn remove_management(root: &Path, name: &str, mode: &str) -> Result<Value> {
    let mut item = active_instance(root, name)?;
    stopped(root, &item)?;
    let record = record_path(root, name);
    no_redirect(&record)?;
    match mode {
        "retain" => {
            item.removed = true;
            save(&record, &item)?;
        }
        "delete" if item.management_discovered || scanned_again(root, &item)? => {
            // Keep only the opt-out identity. Otherwise the next scan would
            // immediately recreate the settings the user asked us to remove.
            item.removed = true;
            item.java.clear();
            item.jvm = None;
            item.timing = None;
            item.auto_restart = None;
            item.core_profile = None;
            item.velocity_listen_address = None;
            item.core_local_create = false;
            save(&record, &item)?;
        }
        "delete" => fs::remove_file(&record).map_err(fail)?,
        _ => return Err("InvalidRemovalMode".into()),
    }
    Ok(json!({"name":name,"mode":mode,"serverDataRetained":true}))
}

fn adopt(root: &Path, name: &str, request: &str, executable: &str) -> Result<Value> {
    let created_at = Some((clock() / 1_000_000) as u64);
    if !instance_name(name) || !admitted(root, name)? {
        return Err("InstanceExcludedOrInvalid".into());
    }
    if !token(request) {
        return Err("InvalidRequestId".into());
    }
    let arguments = vec![
        "adopt".to_owned(),
        name.to_owned(),
        request.to_owned(),
        executable.to_owned(),
    ];
    let receipt_path = root.join("jobs").join(format!("{request}.json"));
    if receipt_path.exists() {
        no_redirect(&receipt_path)?;
        let receipt: Receipt = read(&receipt_path)?;
        if receipt.arguments != arguments {
            return Err("RequestConflict".into());
        }
        if !receipt.complete {
            return Err("ResultUnconfirmed".into());
        }
        let mut output = receipt.output.ok_or("MissingWorkResult")?;
        output["replayed"] = json!(true);
        return Ok(output);
    }
    let mut item = instance(root, name)?;
    if !item.removed {
        return Err("InstanceAlreadyManaged".into());
    }
    stopped(root, &item)?;
    item.java = validated_java(executable)?;
    let folder = instance_path(root, name)?;
    if property_port(&folder)? != item.port {
        return Err("ServerPortRegistryMismatch".into());
    }
    port_unassigned(root, item.port, name)?;
    item.removed = false;
    let record = record_path(root, name);
    no_redirect(&record)?;
    reserve_receipt(
        root,
        &receipt_path,
        &Receipt {
            arguments: arguments.clone(),
            complete: false,
            output: None,
            created_at,
        },
    )?;
    save(&record, &item)?;
    let output = json!({"name":item.name,"java":path_format::display(&item.java),"port":item.port,"adopted":true,"workId":request,"replayed":false});
    save(
        &receipt_path,
        &Receipt {
            arguments,
            complete: true,
            output: Some(output.clone()),
            created_at,
        },
    )?;
    Ok(output)
}

pub(super) fn probe_version(path: &Path) -> Result<String> {
    let mut child = Command::new(path)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(HIDDEN)
        .spawn()
        .map_err(fail)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(exit) = child.try_wait().map_err(fail)? {
            if !exit.success() {
                return Err("JavaProbeFailed".into());
            }
            let mut bytes = Vec::new();
            child
                .stderr
                .take()
                .ok_or("JavaProbeOutputMissing")?
                .take(4096)
                .read_to_end(&mut bytes)
                .map_err(fail)?;
            let text = String::from_utf8_lossy(&bytes);
            return Ok(text
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect());
        }
        if Instant::now() >= deadline {
            child.kill().map_err(fail)?;
            child.wait().map_err(fail)?;
            return Err("JavaProbeTimedOut".into());
        }
        thread::sleep(Duration::from_millis(30));
    }
}

fn java_installations(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(parent) else { return Vec::new() };
    let mut homes: Vec<_> = entries.take(128).filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
    homes.sort_unstable();
    homes.into_iter().map(|home| home.join("bin/java.exe")).collect()
}

// Resolve the account's install location independently of an isolated GUI log directory.
fn windows_local_app_data() -> Option<PathBuf> {
    use std::{ffi::{c_void, OsString}, os::windows::ffi::OsStringExt, ptr};

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    const LOCAL_APP_DATA: Guid = Guid(
        0xf1b32785, 0x6fba, 0x4fcf, [0x9d, 0x55, 0x7b, 0x8e, 0x7f, 0x15, 0x70, 0x91],
    );
    const RPC_E_CHANGED_MODE: i32 = 0x80010106u32 as i32;
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            folder: *const Guid, flags: u32, token: *mut c_void, path: *mut *mut u16,
        ) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, mode: u32) -> i32;
        fn CoUninitialize();
        fn CoTaskMemFree(memory: *mut c_void);
    }

    // An existing apartment in another mode may still use the API.
    unsafe {
        let apartment = CoInitializeEx(ptr::null_mut(), 0);
        if apartment < 0 && apartment != RPC_E_CHANGED_MODE {
            return None;
        }
        let mut value = ptr::null_mut();
        let result = SHGetKnownFolderPath(&LOCAL_APP_DATA, 0, ptr::null_mut(), &mut value);
        let path = if result >= 0 && !value.is_null() {
            let length = (0..32768).find(|offset| *value.add(*offset) == 0);
            length.map(|length| PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(value, length))))
        } else {
            None
        };
        CoTaskMemFree(value.cast());
        if apartment >= 0 {
            CoUninitialize();
        }
        path
    }
}

fn version_rank(version: &str) -> Option<(bool, [u32; 8])> {
    let value = version.split('"').nth(1)?;
    let stable = !value.split(['-', '+']).skip(1).any(|part| {
        matches!(part.to_ascii_lowercase().as_str(), "ea" | "internal" | "beta" | "alpha")
    });
    let version = value.strip_prefix("1.").unwrap_or(value);
    let mut parts = version.split(['.', '_', '+', '-']);
    let mut numbers = [0; 8];
    for (index, part) in parts.by_ref().take(8).enumerate() {
        match part.parse() {
            Ok(value) => numbers[index] = value,
            Err(_) if index == 0 => return None,
            Err(_) => break,
        }
    }
    Some((stable, numbers))
}

fn java_release(path: &Path) -> (Option<String>, Option<String>) {
    let mut text = String::new();
    let result = path.parent().and_then(Path::parent)
        .and_then(|home| File::open(home.join("release")).ok())
        .and_then(|file| file.take(16 * 1024).read_to_string(&mut text).ok());
    if result.is_none() {
        return (None, None);
    }
    let get = |key| text.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name == key).then(|| value.trim_matches('"').chars()
            .filter(|character| !character.is_control()).take(80).collect())
    });
    (get("IMPLEMENTOR"), get("OS_ARCH"))
}

enum JavaDirectoryLayout {
    Installations,
    JavaHome,
    Executables,
}

struct JavaSourceRoot {
    directory: PathBuf,
    source: &'static str,
    layout: JavaDirectoryLayout,
}

fn configured_java_sources(sources: &global_java::DiscoverySources) -> Vec<JavaSourceRoot> {
    use JavaDirectoryLayout::{Executables, Installations, JavaHome};
    let mut roots = Vec::new();
    let mut add = |directory: PathBuf, source, layout| {
        if directory.is_absolute() && directory.parent().is_some() {
            roots.push(JavaSourceRoot { directory, source, layout });
        }
    };
    if sources.mise {
        let root = std::env::var_os("MISE_DATA_DIR")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                windows_local_app_data()
                    .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
                    .map(|local| local.join("mise"))
            });
        if let Some(root) = root {
            add(root.join("installs/java"), "mise", Installations);
        }
    }
    if sources.java_home {
        if let Some(home) = std::env::var_os("JAVA_HOME") {
            add(PathBuf::from(home), "JAVA_HOME", JavaHome);
        }
    }
    if sources.windows {
        for env_key in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(programs) = std::env::var_os(env_key) {
                let programs = PathBuf::from(programs);
                for vendor in ["Java", "Eclipse Adoptium", "Microsoft", "Amazon Corretto", "Zulu", "BellSoft"] {
                    let folder = programs.join(vendor);
                    let latest = (vendor == "Java").then(|| folder.join("latest"));
                    add(folder, "Windows", Installations);
                    if let Some(latest) = latest {
                        add(latest, "Windows", Installations);
                    }
                }
            }
        }
    }
    if sources.path {
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path).take(128) {
                if !shim_path(&directory) {
                    add(directory, "PATH", Executables);
                }
            }
        }
    }
    roots
}

fn eligible_java(
    path: &Path,
    custom_root: Option<&Path>,
    seen: &mut HashSet<String>,
) -> Option<PathBuf> {
    let resolved = fs::canonicalize(path).ok()?;
    if custom_root.is_some_and(|root| !scan_policy::contains(root, &resolved))
        || shim_path(&resolved)
        || !resolved.is_file()
    {
        return None;
    }
    seen.insert(resolved.to_string_lossy().to_lowercase()).then_some(resolved)
}

fn custom_java_paths(directory: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    use std::os::windows::fs::MetadataExt;

    const MAX_DEPTH: usize = 6;
    const MAX_ENTRIES: usize = 32_768;
    const MAX_ENTRIES_PER_DIRECTORY: usize = 1_024;
    const REPARSE_POINT: u32 = 0x400;
    let mut pending = VecDeque::from([(directory.to_path_buf(), 0)]);
    let mut remaining = MAX_ENTRIES;
    // Scan breadth-first so a large first subtree cannot delay nearby JDKs.
    std::iter::from_fn(move || {
        while let Some((home, depth)) = pending.pop_front() {
            if depth < MAX_DEPTH && remaining > 0 && no_redirect(&home).is_ok() {
                if let Ok(entries) = fs::read_dir(&home) {
                    for entry in entries.take(remaining.min(MAX_ENTRIES_PER_DIRECTORY)) {
                        remaining -= 1;
                        let Ok(entry) = entry else { continue };
                        if entry.metadata().is_ok_and(|metadata| {
                            metadata.is_dir() && metadata.file_attributes() & REPARSE_POINT == 0
                        }) {
                            pending.push_back((entry.path(), depth + 1));
                        }
                    }
                }
            }
            let executable = home.join("bin/java.exe");
            if executable.is_file() {
                // Some older JDKs bundle a separate JRE. Check it alongside
                // its JDK even when unrelated directories fill the walk budget.
                if depth < MAX_DEPTH {
                    let bundled = home.join("jre");
                    if bundled.join("bin/java.exe").is_file() && no_redirect(&bundled).is_ok() {
                        pending.push_front((bundled, depth + 1));
                    }
                }
                return Some(executable);
            }
        }
        None
    })
}

fn collect_java_entries(scope: &global_java::JavaScanScope) -> Vec<(PathBuf, &'static str)> {
    const CUSTOM_PROBE_LIMIT: usize = 32;
    const DEFAULT_PROBE_LIMIT: usize = 32;
    let mut seen = HashSet::new();
    let mut custom: VecDeque<_> = scope.directories.iter()
        .map(|directory| (directory.as_path(), custom_java_paths(directory))).collect();
    let mut entries = Vec::new();
    // Visit each configured root before taking a second candidate from any one root.
    while entries.len() < CUSTOM_PROBE_LIMIT {
        let Some((directory, mut paths)) = custom.pop_front() else { break };
        if let Some(path) = paths.find_map(|path| eligible_java(&path, Some(directory), &mut seen)) {
            entries.push((path, "custom"));
            custom.push_back((directory, paths));
        }
    }
    let candidates = configured_java_sources(&scope.sources).into_iter().flat_map(|root| {
        use JavaDirectoryLayout::{Executables, Installations, JavaHome};
        let paths = match root.layout {
            Installations => java_installations(&root.directory),
            JavaHome => vec![root.directory.join("bin/java.exe")],
            Executables => vec![root.directory.join("java.exe")],
        };
        paths.into_iter().map(move |path| (path, root.source))
    });
    entries.extend(candidates.filter_map(|(path, source)| {
        eligible_java(&path, None, &mut seen).map(|path| (path, source))
    }).take(DEFAULT_PROBE_LIMIT));
    entries
}

pub(super) fn discover_java(workspace: &Path) -> Result<Value> {
    scan_java(&global_java::scan_scope(workspace)?)
}

pub(super) fn scan_java(scope: &global_java::JavaScanScope) -> Result<Value> {
    let entries = collect_java_entries(scope);
    let mut rows = vec![];
    for chunk in entries.chunks(4) {
        thread::scope(|scope| -> Result<()> {
            let handles: Vec<_> = chunk.iter().map(|(path, source)| scope.spawn(move || {
                let outcome = probe_version(path);
                let stable = outcome.as_deref().ok().and_then(version_rank).is_some_and(|rank| rank.0);
                let (vendor, architecture) = java_release(path);
                json!({
                    "path":path_format::display(path),
                    "version":outcome.as_ref().ok(),
                    "valid":outcome.is_ok(),
                    "source":source, "stable":stable, "vendor":vendor, "architecture":architecture
                })
            })).collect();
            for handle in handles {
                rows.push(handle.join().map_err(|_| "JavaProbeFailed")?);
            }
            Ok(())
        })?;
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row["version"].as_str().and_then(version_rank)));
    Ok(json!(rows))
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    let result = match (verb, args) {
        ("presentation", []) => presentation(root, true),
        ("presentation-metadata", []) => presentation(root, false),
        ("preferences", []) => load_preferences(root).map(|prefs| json!(prefs)),
        ("preferences-snapshot", []) => load_preferences_version(root)
            .map(|(preferences, revision, _)| json!({"preferences":preferences,"revision":revision})),
        ("save-preferences", [raw]) => save_preferences(root, raw, None),
        ("save-preferences", [raw, revision]) => save_preferences(root, raw, Some(revision)),
        ("instance-settings", [name]) => active_instance(root, name).and_then(|item| {
            let mut response = settings_value(&item)?;
            if let Some(pending) = deferred_settings::load::<PendingInstance>(root, "instance", name)? {
                response["pending"] = json!({"state":pending.state,"values":pending.values,"error":pending.error});
            }
            Ok(response)
        }),
        ("save-instance-settings", [name, raw]) => save_instance_settings(root, name, raw),
        ("java-check", [name, raw]) => java_check(root, name, raw),
        ("discover-java", []) => discover_java(root),
        ("logs", [name]) => logs(root, name),
        ("open-log-folder", [name]) => open_log_folder(root, name),
        ("remove-management", [name, mode]) => remove_management(root, name, mode),
        ("adopt", [name, request, executable]) => adopt(root, name, request, executable),
        (
            "presentation"
            | "presentation-metadata"
            | "preferences"
            | "preferences-snapshot"
            | "save-preferences"
            | "instance-settings"
            | "save-instance-settings"
            | "java-check"
            | "discover-java"
            | "logs"
            | "open-log-folder"
            | "remove-management"
            | "adopt",
            _,
        ) => Err("InvalidPresentationArguments".into()),
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn custom_root_finds_direct_and_nested_java_installations() {
        let fixture = std::env::temp_dir().join(format!("ih-java-mise-{}-{}", std::process::id(), clock()));
        let mise = fixture.join("mise");
        let data = mise.join("data");
        let installs = data.join("installs");
        let java_installs = installs.join("java");
        let jdk = java_installs.join("jdk-25");
        let bin = jdk.join("bin");
        let java = bin.join("java.exe");
        let bundled = jdk.join("jre/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(&java, []).unwrap();
        fs::create_dir_all(&bundled).unwrap();
        fs::write(bundled.join("java.exe"), []).unwrap();
        let direct = fixture.join("jdk/bin");
        fs::create_dir_all(&direct).unwrap();
        fs::write(direct.join("java.exe"), []).unwrap();
        let sources = global_java::DiscoverySources {
            windows: false, mise: false, java_home: false, path: false,
        };
        for directory in [&fixture, &mise, &data, &java_installs] {
            let scope = global_java::JavaScanScope {
                sources,
                directories: vec![fs::canonicalize(directory).unwrap()],
            };
            let mut expected = Vec::new();
            if directory == &fixture {
                expected.push((fs::canonicalize(direct.join("java.exe")).unwrap(), "custom"));
            }
            expected.push((fs::canonicalize(&java).unwrap(), "custom"));
            expected.push((fs::canonicalize(bundled.join("java.exe")).unwrap(), "custom"));
            assert_eq!(collect_java_entries(&scope), expected);
        }
        fs::remove_file(java).unwrap();
        fs::remove_file(bundled.join("java.exe")).unwrap();
        fs::remove_dir(&bundled).unwrap();
        fs::remove_dir(jdk.join("jre")).unwrap();
        fs::remove_file(direct.join("java.exe")).unwrap();
        fs::remove_dir(&direct).unwrap();
        fs::remove_dir(fixture.join("jdk")).unwrap();
        for path in [bin, jdk, java_installs, installs, data, mise, fixture] {
            fs::remove_dir(path).unwrap();
        }
    }

    #[test]
    fn custom_candidates_stay_within_roots_and_are_deduplicated() {
        let folder = std::env::temp_dir().join(format!(
            "ih-java-discovery-{}-{}", std::process::id(), clock()
        ));
        let home = folder.join("jdk");
        let sibling = folder.join("jdk-sibling");
        for home in [&home, &sibling] {
            fs::create_dir_all(home.join("bin")).unwrap();
            fs::write(home.join("bin/java.exe"), []).unwrap();
        }
        let resolved_folder = fs::canonicalize(&folder).unwrap();
        let resolved_home = fs::canonicalize(&home).unwrap();
        let sibling_candidate = sibling.join("bin/java.exe");
        let mut seen = HashSet::new();

        assert!(eligible_java(&sibling_candidate, Some(&resolved_home), &mut seen).is_none());
        assert!(seen.is_empty());
        assert!(eligible_java(&sibling_candidate, Some(&resolved_folder), &mut seen).is_some());
        assert!(eligible_java(&sibling_candidate, None, &mut seen).is_none());

        let sources = global_java::DiscoverySources {
            windows: false, mise: false, java_home: false, path: false,
        };
        assert!(configured_java_sources(&sources).is_empty());
        let scope = global_java::JavaScanScope {
            sources,
            directories: vec![resolved_folder, resolved_home],
        };
        assert_eq!(
            collect_java_entries(&scope),
            vec![
                (fs::canonicalize(home.join("bin/java.exe")).unwrap(), "custom"),
                (fs::canonicalize(sibling_candidate).unwrap(), "custom"),
            ],
        );

        for home in [home, sibling] {
            fs::remove_file(home.join("bin/java.exe")).unwrap();
            fs::remove_dir(home.join("bin")).unwrap();
            fs::remove_dir(home).unwrap();
        }
        fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn runtime_ranking_compares_numbers_and_prefers_stable_versions() {
        assert!(version_rank(r#"openjdk version "25.0.10""#) > version_rank(r#"openjdk version "25.0.9""#));
        assert!(version_rank(r#"openjdk version "25.0.5""#) > version_rank(r#"openjdk version "26-ea+2""#));
        assert!(version_rank(r#"openjdk version "1.8.0_513""#) > version_rank(r#"openjdk version "1.8.0_512""#));
        assert_eq!(version_rank("invalid"), None);
    }

    use super::*;

    #[test]
    fn native_console_capability_describes_the_installation_not_a_switch_file() {
        let root = std::env::temp_dir().join(format!("ih-console-cap-{}-{}", std::process::id(), clock()));
        fs::create_dir(&root).unwrap();
        // The test executable has no bundled runtime beside it; Windows still has its own.
        let value = capabilities(&root);
        assert_eq!(value["nativeConsole"], true);
        assert_eq!(value["consoleRuntime"]["bundled"], false);
        assert_eq!(value["consoleRuntime"]["bundledReason"], "ConsoleRuntimeMissing");
        assert_eq!(value["consoleRuntime"]["system"], true);
        // The retired marker file has no effect.
        fs::write(root.join("native-console.enabled"), b"").unwrap();
        assert_eq!(capabilities(&root), value);
        fs::remove_file(root.join("native-console.enabled")).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn logs_dialog_reads_pseudoconsole_output_as_plain_lines() {
        let raw = "\u{1b}[?25l\u{1b}[2J\u{1b}[m\u{1b}[H[12:00:00 INFO]: Starting\u{1b}[K\r\n\u{1b}]0;java\u{7}[12:00:01 INFO]:\u{1b}[3CDone (1.2s)! For help\r\n> lisx\u{8}t\u{1b}[?25h";
        assert_eq!(
            readable_terminal(raw),
            "[12:00:00 INFO]: Starting\n[12:00:01 INFO]:   Done (1.2s)! For help\n> list"
        );
        assert_eq!(readable_terminal("progress 10%\rprogress 99%\r\ndone\n"), "progress 99%\ndone\n");
        assert_eq!(readable_terminal("\u{1b}]8;;x\u{1b}\\link\u{1b}[0;32mok\u{1b}[0m"), "linkok");
        let plain = "[12:00:00 INFO]: Done\n";
        assert_eq!(readable_terminal(plain), plain);
    }

    #[test]
    fn console_window_title_states_that_closing_stops_the_server() {
        assert!(chinese_interface(DisplayLanguage::TraditionalChinese));
        assert!(!chinese_interface(DisplayLanguage::English));
        let root = std::env::temp_dir().join(format!("ih-console-title-{}-{}", std::process::id(), clock()));
        fs::create_dir(&root).unwrap();
        assert!(console_window_title(&root, "lobby").contains("關閉此視窗會停止伺服器"));
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn retired_close_to_tray_is_kept_for_rollback_and_only_written_when_off() {
        let fixture = std::env::temp_dir().join(format!("ih-tray-pref-{}-{}", std::process::id(), clock()));
        fs::create_dir(&fixture).unwrap();
        let root = fs::canonicalize(&fixture).unwrap();
        let stored = |root: &Path| load_preferences(root).unwrap().close_to_tray;
        assert!(stored(&root));

        // The default stays out of the file so rc2 can still read it after a rollback.
        save_preferences(&root, r#"{"theme":"dark","pins":[]}"#, None).unwrap();
        assert!(!fs::read_to_string(preferences_path(&root)).unwrap().contains("closeToTray"));

        // A 1.0.0 file that turned it off still loads, and later saves keep that choice.
        save_preferences(&root, r#"{"theme":"dark","pins":[],"closeToTray":false}"#, None).unwrap();
        save_preferences(&root, r#"{"theme":"light","pins":[]}"#, None).unwrap();
        assert!(!stored(&root));
        fs::remove_file(preferences_path(&root)).unwrap();
        fs::remove_dir(&fixture).unwrap();
    }

    #[test]
    fn fast_open_is_off_by_default_written_only_when_on_and_kept_by_other_saves() {
        let fixture = std::env::temp_dir().join(format!("ih-fast-open-{}-{}", std::process::id(), clock()));
        fs::create_dir(&fixture).unwrap();
        let root = fs::canonicalize(&fixture).unwrap();
        assert!(!load_preferences(&root).unwrap().fast_open);
        assert!(!fast_open_enabled(&root));

        // Off stays out of the file, so older versions that reject unknown fields still read it.
        save_preferences(&root, r#"{"theme":"dark","pins":[]}"#, None).unwrap();
        assert!(!fs::read_to_string(preferences_path(&root)).unwrap().contains("fastOpen"));

        save_preferences(&root, r#"{"theme":"dark","pins":[],"fastOpen":true}"#, None).unwrap();
        assert!(fast_open_enabled(&root));
        // The GUI passes IH_WORKSPACE as the launcher spelled it, not the canonical path.
        assert_ne!(fixture, root);
        assert!(fast_open_enabled(&fixture));
        // A save from another settings tab leaves it on.
        save_preferences(&root, r#"{"theme":"light","pins":[]}"#, None).unwrap();
        assert!(fast_open_enabled(&root));
        save_preferences(&root, r#"{"theme":"light","pins":[],"fastOpen":false}"#, None).unwrap();
        assert!(!fast_open_enabled(&root));
        assert!(!fs::read_to_string(preferences_path(&root)).unwrap().contains("fastOpen"));
        fs::remove_file(preferences_path(&root)).unwrap();
        fs::remove_dir(&fixture).unwrap();
    }

    #[test]
    fn preferences_reject_invalid_or_duplicate_pins() {
        let names = vec!["one".to_owned()];
        assert!(validate_preferences(
            &Preferences {
                theme: "dark".into(),
                pins: vec!["one".into()],
                ..Preferences::default()
            },
            &names
        )
        .is_ok());
        assert!(validate_preferences(
            &Preferences {
                theme: "blue".into(),
                pins: vec![],
                ..Preferences::default()
            },
            &names
        )
        .is_err());
        assert!(validate_preferences(
            &Preferences {
                theme: "system".into(),
                pins: vec!["one".into(), "one".into()],
                ..Preferences::default()
            },
            &names
        )
        .is_err());
        assert!(validate_preferences(
            &Preferences {
                theme: "system".into(),
                pins: vec!["other".into()],
                ..Preferences::default()
            },
            &names
        )
        .is_err());
        assert!(validate_preferences(
            &Preferences {
                path_separator: "other".into(),
                ..Preferences::default()
            },
            &names
        )
        .is_err());
    }

    #[test]
    fn old_preferences_default_to_forward_paths_and_the_system_language() {
        let old: Preferences = serde_json::from_str(r#"{"theme":"system","pins":[]}"#).unwrap();
        assert_eq!(old.path_separator, "forward");
        assert_eq!(old.language, DisplayLanguage::System);
        assert_eq!(old.time_format, TimeFormat::Local24);
        assert_eq!(old.console_title_template, "{instance}");
        assert!(old.console_title_overrides.is_empty());
        assert_eq!(
            serde_json::to_value(&old).unwrap()["pathSeparator"],
            "forward"
        );
    }

    #[test]
    fn list_selection_opacity_defaults_and_checks_bounds() {
        let legacy = json!({"theme": "system", "pins": []});
        let loaded: Preferences = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(loaded.selection_opacity, 25);
        for accepted in [0, 25, 100] {
            let mut raw = legacy.clone();
            raw["selectionOpacity"] = json!(accepted);
            let prefs: Preferences = serde_json::from_value(raw).unwrap();
            assert!(validate_preferences(&prefs, &[]).is_ok());
            assert_eq!(serde_json::to_value(prefs).unwrap()["selectionOpacity"], accepted);
        }
        let prefs = Preferences {selection_opacity: 101, ..Default::default()};
        assert_eq!(validate_preferences(&prefs, &[]).unwrap_err(), "InvalidSelectionOpacity");
    }

    #[test]
    fn selection_palettes_round_trip_and_old_product_remains_readable() {
        for name in ["system", "blue", "red", "green", "magenta", "yellow", "cyan", "purple", "product"] {
            let raw = json!({"theme":"system","pins":[],"selectionColor":name});
            let parsed: Preferences = serde_json::from_value(raw.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap()["selectionColor"], raw["selectionColor"]);
        }
        assert!(serde_json::from_value::<Preferences>(json!({"theme":"system","pins":[],"selectionColor":"orange"})).is_err());
        assert_eq!(serde_json::to_value(Preferences::default()).unwrap()["selectionColor"], "blue");
    }

    #[test]
    fn display_preferences_round_trip_titles_and_clock_without_adding_overrides() {
        let raw = json!({"theme":"system","pins":[],"timeFormat":"utc",
            "consoleTitleTemplate":"{instance} :{port}","consoleTitleOverrides":{"one":"{type} / {instance}"}});
        let value: Preferences = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(value.time_format, TimeFormat::Utc);
        assert!(validate_preferences(&value, &[]).is_ok());
        let saved = serde_json::to_value(value).unwrap();
        for field in ["timeFormat", "consoleTitleTemplate", "consoleTitleOverrides"] {
            assert_eq!(saved[field], raw[field]);
        }
        assert!(serde_json::from_value::<Preferences>(json!({"theme":"system","pins":[],"timeFormat":"custom"})).is_err());
    }

    #[test]
    fn only_registered_placeholders_expand_and_label_values_cannot_embed_controls() {
        assert_eq!(render_title("測試 {instance} :{port} {type}", |key| match key {
            "instance" => Some("{type}\u{1b}\n".into()),
            "port" => Some("25566".into()),
            _ => None,
        }).unwrap(), "測試 {type} :25566 N/A");
        for invalid in ["", " ", "{bogus}", "{{instance}}", "before } {instance}", "{port", "{instance}\n"] {
            assert!(render_title(invalid, |_| None).is_err(), "{invalid}");
        }
        assert!(render_title(&"𠮷".repeat(129), |_| None).is_err());
    }

    #[test]
    fn console_process_time_converts_windows_filetime_to_utc() {
        assert_eq!(
            filetime_title(116_444_736_000_000_000 + 1_000 * 10_000).as_deref(),
            Some("1970-01-01 00:00:01 UTC")
        );
    }

    #[test]
    fn the_system_language_is_traditional_chinese_only_for_taiwan_hong_kong_and_macao() {
        for chinese in [0x0404, 0x0C04, 0x1404] {
            assert!(system_chinese(chinese));
        }
        // English, Simplified Chinese, Japanese and German fall back to English.
        for other in [0x0409, 0x0804, 0x0411, 0x0407] {
            assert!(!system_chinese(other));
        }
    }

    #[test]
    fn accepts_only_supported_display_languages() {
        for (raw, language) in [
            ("zh-TW", DisplayLanguage::TraditionalChinese),
            ("en", DisplayLanguage::English),
            ("system", DisplayLanguage::System),
        ] {
            let value: Preferences = serde_json::from_value(json!({"theme":"system","pins":[],"language":raw})).unwrap();
            assert_eq!(value.language, language);
            assert_eq!(serde_json::to_value(&value).unwrap()["language"], raw);
        }
        for bad in [json!("zh-CN"), json!("en-US"), json!(""), Value::Null, json!(1)] {
            assert!(serde_json::from_value::<Preferences>(json!({"theme":"system","pins":[],"language":bad})).is_err());
        }
    }

    #[test]
    fn port_replacement_preserves_other_lines_and_line_endings() {
        let input =
            "# comment\r\nserver-ip=127.0.0.1\r\nserver-port=25565\r\nonline-mode=false\r\n";
        assert_eq!(
            replace_port(input, 25565, 25566).unwrap(),
            input.replace("server-port=25565", "server-port=25566")
        );
        assert!(replace_port("server-port=1\nserver-port=1\n", 1, 2).is_err());
        assert!(replace_port("server-port=2\n", 1, 2).is_err());
    }

    #[test]
    fn log_tail_is_bounded_to_last_128_kib() {
        let folder = std::env::temp_dir().join(format!(
            "ih-presentation-{}-{}",
            std::process::id(),
            clock()
        ));
        fs::create_dir(&folder).unwrap();
        let path = folder.join("stdout.log");
        let mut content = vec![b'x'; LOG_LIMIT as usize];
        content.extend_from_slice(&[b'y'; 32]);
        fs::write(&path, content).unwrap();
        let resolved = fs::canonicalize(&path).unwrap();
        let result = log_tail(&resolved);
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&folder).unwrap();
        let tail = result.unwrap();
        assert_eq!(tail.len(), LOG_LIMIT as usize);
        assert!(tail.ends_with(&"y".repeat(32)));
        assert_eq!(tail.as_bytes()[LOG_LIMIT as usize - 33], b'x');
    }

    fn core_fixture(bytes: &[u8]) -> (PathBuf, PathBuf) {
        let folder = std::env::temp_dir().join(format!("ih-core-meta-{}-{}", std::process::id(), clock()));
        fs::create_dir(&folder).unwrap();
        let folder = fs::canonicalize(&folder).unwrap();
        let jar = folder.join("server.jar");
        fs::write(&jar, bytes).unwrap();
        (folder, jar)
    }

    fn core_row(profile: Option<CoreProfile>) -> Instance {
        Instance { core_file: None, name: "core-meta".into(), java: PathBuf::new(), port: 25565,
            velocity_listen_address: None, removed: false, directory: None, jvm: None, timing: None,
            auto_restart: None, core_profile: profile, core_local_create: false, management_discovered: false }
    }

    fn core_profile(family: &str, minecraft: &str, build: &str, sha256: String) -> CoreProfile {
        CoreProfile { family: family.into(), minecraft: minecraft.into(), build: build.into(), sha256,
            unverified: false, java: None }
    }

    fn known(core: &Value) -> (&str, &str, &str) {
        assert_eq!(core["state"], "known", "{core}");
        (core["family"].as_str().unwrap(), core["minecraft"].as_str().unwrap(), core["build"].as_str().unwrap())
    }

    // #48: the list must agree with core maintenance, which trusts the
    // built-in table first and then a registered profile with the same hash.
    #[test]
    fn list_core_uses_the_same_trusted_profile_as_core_maintenance() {
        let (folder, jar) = core_fixture(b"legacy paperclip without a paper-api library");
        let sha = content_revision::file_revision(&jar).unwrap();
        let pinned = core_profile("SPIGOT", "1.14", "c8d4cef", sha.clone());
        let imported = core_metadata(&core_row(None), &jar, |hash| (hash == sha).then(|| pinned.clone()));
        assert_eq!(known(&imported), ("SPIGOT", "1.14", "c8d4cef"));
        // Old Paper (1.14, 1.14.4) has no paper-api library to read offline.
        let old_paper = core_row(Some(core_profile("PAPER", "1.14.4", "245", sha.clone())));
        let saved = core_metadata(&old_paper, &jar, |hash| core_maintenance::trusted_profile(&old_paper, hash));
        assert_eq!(known(&saved), ("PAPER", "1.14.4", "245"));
        fs::remove_dir_all(&folder).unwrap();
    }

    // #118: the GUI reads the list every 1.5 s through the Manager's only pipe thread. A
    // core without a readable identity (Velocity, vanilla, ZIP64) was hashed in full on
    // every read, 40 to 90 MB each time.
    #[test]
    fn list_core_hashes_an_unidentified_jar_once_until_the_file_changes() {
        let (folder, jar) = core_fixture(b"core without a readable identity");
        let row = core_row(None);
        let hashed = std::cell::Cell::new(0);
        let digest = |path: &Path| {
            hashed.set(hashed.get() + 1);
            content_revision::file_revision(path)
        };
        let first = current_core_with(&row, &jar, |_| None, digest);
        let second = current_core_with(&row, &jar, |_| None, digest);
        assert_eq!(first, second);
        assert_eq!(hashed.get(), 1, "an unchanged core is hashed once");
        fs::write(&jar, b"core without a readable identity, replaced").unwrap();
        current_core_with(&row, &jar, |_| None, digest);
        assert_eq!(hashed.get(), 2, "a changed core is hashed again");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn list_core_does_not_reuse_a_profile_after_the_jar_changes() {
        let (folder, jar) = core_fixture(b"replaced core");
        let row = core_row(Some(core_profile("FOLIA", "26.1.2", "8", "0".repeat(64))));
        let core = core_metadata(&row, &jar, |hash| core_maintenance::trusted_profile(&row, hash));
        assert_eq!(core["state"], "unknown");
        assert_eq!(core["reason"], "CoreProfileUnverified");
        fs::remove_file(&jar).unwrap();
        assert_eq!(core_metadata(&row, &jar, |_| None)["reason"], "CorePathUnverified");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn list_core_prefers_a_registered_non_paper_profile_over_paperclip_metadata() {
        let paper_api = "META-INF/libraries/io/papermc/paper/paper-api/26.1.2.build.74-stable/paper-api-26.1.2.build.74-stable.jar";
        let (folder, jar) = core_fixture(&core_identity::zip(&[paper_api]));
        let sha = content_revision::file_revision(&jar).unwrap();
        let unregistered = core_metadata(&core_row(None), &jar, |_| None);
        assert_eq!(known(&unregistered), ("Paper", "26.1.2", "74"));
        let folia = core_row(Some(core_profile("FOLIA", "26.1.2", "8", sha)));
        let core = core_metadata(&folia, &jar, |hash| core_maintenance::trusted_profile(&folia, hash));
        assert_eq!(known(&core), ("FOLIA", "26.1.2", "8"));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn list_core_keeps_an_offline_accepted_profile_marked_unverified() {
        let (folder, jar) = core_fixture(b"offline accepted core");
        let mut profile = core_profile("PAPER", "1.21.4", "232", content_revision::file_revision(&jar).unwrap());
        profile.unverified = true;
        let row = core_row(Some(profile));
        let core = core_metadata(&row, &jar, |hash| core_maintenance::trusted_profile(&row, hash));
        assert_eq!(known(&core), ("PAPER", "1.21.4", "232"));
        assert_eq!(core["unverified"], true);
        fs::remove_dir_all(&folder).unwrap();
    }

    const FOLIA_API: &str = "META-INF/libraries/dev/folia/folia-api/26.3.build.28-stable/folia-api-26.3.build.28-stable.jar";

    // #48: a Folia or Spigot imported without a registration is identified
    // from the JAR alone, and the list says the version was read offline.
    #[test]
    fn list_core_reads_unregistered_folia_and_spigot_offline() {
        let (folder, jar) = core_fixture(&core_identity::zip(&[FOLIA_API]));
        let core = core_metadata(&core_row(None), &jar, |_| None);
        assert_eq!(known(&core), ("Folia", "26.3", "28"));
        assert_eq!(core["offline"], true);
        fs::write(&jar, core_identity::zip_with(&[(
            "META-INF/versions.list",
            b"0000000000000000000000000000000000000000000000000000000000000000\tspigot-26.2-R0.1-SNAPSHOT.jar\n",
        )])).unwrap();
        let core = core_metadata(&core_row(None), &jar, |_| None);
        assert_eq!(known(&core), ("Spigot", "26.2", ""));
        assert_eq!(core["offline"], true);
        fs::remove_dir_all(&folder).unwrap();
    }

    // An unregistered Spigot 1.12.2 is in the built-in table: core maintenance
    // shows it verified with its build, so the list must too.
    #[test]
    fn list_core_checks_the_built_in_table_for_an_unregistered_core() {
        let (folder, jar) = core_fixture(&core_identity::zip_with(&[(
            "META-INF/maven/org.spigotmc/spigot/pom.properties",
            b"version=1.12.2-R0.1-SNAPSHOT\n",
        )]));
        let sha = content_revision::file_revision(&jar).unwrap();
        let pinned = core_profile("SPIGOT", "1.12.2", "79a30d7", sha.clone());
        let core = core_metadata(&core_row(None), &jar, |hash| (hash == sha).then(|| pinned.clone()));
        assert_eq!(known(&core), ("SPIGOT", "1.12.2", "79a30d7"));
        assert!(core.get("offline").is_none(), "{core}");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn list_core_marks_only_unchecked_versions_offline() {
        let (folder, jar) = core_fixture(&core_identity::zip(&[FOLIA_API]));
        let sha = content_revision::file_revision(&jar).unwrap();
        let verified = core_row(Some(core_profile("FOLIA", "26.3", "28", sha.clone())));
        let core = core_metadata(&verified, &jar, |hash| core_maintenance::trusted_profile(&verified, hash));
        // The verified profile, exactly as core maintenance shows it.
        assert_eq!(known(&core), ("FOLIA", "26.3", "28"));
        assert!(core.get("offline").is_none(), "{core}");
        // An offline import records no build; the JAR still names it.
        let mut accepted = core_profile("FOLIA", "26.3", "", sha);
        accepted.unverified = true;
        let accepted = core_row(Some(accepted));
        let core = core_metadata(&accepted, &jar, |hash| core_maintenance::trusted_profile(&accepted, hash));
        assert_eq!(known(&core), ("Folia", "26.3", "28"));
        assert_eq!((&core["offline"], &core["unverified"]), (&json!(true), &json!(true)));
        fs::remove_dir_all(&folder).unwrap();
    }
}
