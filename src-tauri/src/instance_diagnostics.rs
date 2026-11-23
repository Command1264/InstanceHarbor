//! Bounded, local evidence for the instance diagnostics shared by CLI and GUI.
use super::*;
use std::{
    ffi::OsString,
    io::{Seek, SeekFrom},
    net::SocketAddrV4,
    os::windows::{ffi::OsStringExt, fs::MetadataExt, io::AsRawHandle},
};

const CONFIG_LIMIT: u64 = 1024 * 1024;
const LOG_LIMIT: u64 = 512 * 1024;
const PLUGIN_LIMIT: u64 = 64 * 1024 * 1024;
const REPARSE_POINT: u32 = 0x400;

#[link(name = "kernel32")]
extern "system" {
    fn GetFinalPathNameByHandleW(
        file: *mut std::ffi::c_void,
        path: *mut u16,
        size: u32,
        flags: u32,
    ) -> u32;
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    proxy: Option<String>,
    route: Option<String>,
    plugin: Option<String>,
}

#[derive(Serialize)]
struct Finding {
    state: &'static str,
    source: &'static str,
    reason: &'static str,
}

fn finding(state: &'static str, source: &'static str, reason: &'static str) -> Finding {
    Finding {
        state,
        source,
        reason,
    }
}

#[derive(Serialize)]
struct Configuration {
    #[serde(flatten)]
    finding: Finding,
    proxy: Option<String>,
    route: Option<String>,
}

fn configuration(finding: Finding, proxy: Option<String>, route: Option<String>) -> Configuration {
    Configuration {
        finding,
        proxy,
        route,
    }
}

fn label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        && value.as_bytes()[0].is_ascii_alphanumeric()
}

fn jar_basename(value: &str) -> bool {
    value.len() <= 160
        && value.to_ascii_lowercase().ends_with(".jar")
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b" .+_-".contains(&byte))
        && !value.contains("..")
        && ![
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(
            &value
                .split('.')
                .next()
                .unwrap_or("")
                .to_ascii_uppercase()
                .as_str(),
        )
}

fn open_local(path: &Path) -> Result<File> {
    no_redirect(path)?;
    let attrs = fs::symlink_metadata(path).map_err(fail)?;
    if !attrs.is_file() || attrs.file_attributes() & REPARSE_POINT != 0 {
        return Err("DiagnosticEvidenceUnavailable".into());
    }
    let file = File::open(path).map_err(fail)?;
    let mut resolved = vec![0u16; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            resolved.as_mut_ptr(),
            resolved.len() as u32,
            0,
        )
    } as usize;
    if length == 0
        || length >= resolved.len()
        || Path::new(&OsString::from_wide(&resolved[..length])) != path
        || !file.metadata().map_err(fail)?.is_file()
    {
        return Err("DiagnosticEvidenceUnavailable".into());
    }
    Ok(file)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    created: u64,
    changed: u64,
    size: u64,
}

fn stamp(file: &File) -> Result<Stamp> {
    let metadata = file.metadata().map_err(fail)?;
    Ok(Stamp {
        created: metadata.creation_time(),
        changed: metadata.last_write_time(),
        size: metadata.len(),
    })
}

fn current_stamp(path: &Path) -> Result<Stamp> {
    stamp(&open_local(path)?)
}

pub(super) fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = open_local(path)?;
    let before = stamp(&file)?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(fail)?;
    if bytes.len() as u64 != before.size || before.size > limit || current_stamp(path)? != before {
        return Err("DiagnosticEvidenceUnavailable".into());
    }
    Ok(bytes)
}

pub(super) fn backend_endpoint(bytes: &[u8]) -> Option<SocketAddrV4> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut address = None;
    let mut port = None;
    let whitespace = [' ', '\t', '\x0c'];
    for line in text
        .split(['\r', '\n'])
        .map(|line| line.trim_start_matches(whitespace))
    {
        // Match Java properties separators. Refuse complex spellings and
        // continuations because they can hide an overriding key.
        if line.starts_with(['#', '!']) || line.is_empty() {
            continue;
        }
        if line.ends_with('\\') {
            return None;
        }
        let split = line
            .find(['=', ':', ' ', '\t', '\x0c'])
            .unwrap_or(line.len());
        let (key, value) = line.split_at(split);
        if key.contains('\\') {
            return None;
        }
        let value = value.trim_start_matches(whitespace);
        let value = value
            .strip_prefix(['=', ':'])
            .unwrap_or(value)
            .trim_start_matches(whitespace);
        match key {
            "server-ip" if address.is_none() => address = Some(value.parse::<Ipv4Addr>().ok()?),
            "server-port" if port.is_none() => port = Some(value.parse::<u16>().ok()?),
            "server-ip" | "server-port" => return None,
            _ => {}
        }
    }
    let endpoint = SocketAddrV4::new(address?, port?);
    valid_endpoint(endpoint).then_some(endpoint)
}

fn valid_endpoint(endpoint: SocketAddrV4) -> bool {
    endpoint.port() != 0
        && !endpoint.ip().is_unspecified()
        && !endpoint.ip().is_broadcast()
        && !endpoint.ip().is_multicast()
}

fn endpoint(value: &toml::Value) -> Option<SocketAddrV4> {
    let address = value.as_str()?.parse().ok()?;
    valid_endpoint(address).then_some(address)
}

fn route_finding(
    bytes: &[u8],
    proxy: SocketAddrV4,
    backend: SocketAddrV4,
    selected: Option<&str>,
) -> (&'static str, &'static str, Option<String>) {
    let parsed = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok());
    let Some(parsed) = parsed else {
        return ("Unconfirmed", "VelocityConfigUnreadable", None);
    };
    // The proxy may listen on every interface; backend routes stay specific.
    let bind = parsed.get("bind").and_then(|value| value.as_str()?.parse::<SocketAddrV4>().ok())
        .filter(|bind| bind.port() != 0 && !bind.ip().is_broadcast() && !bind.ip().is_multicast());
    let Some(bind) = bind else {
        return ("Unconfirmed", "ProxyBindingUnconfirmed", None);
    };
    if bind != proxy {
        return (
            "Mismatch",
            "ProxyBindingChanged",
            selected.map(str::to_owned),
        );
    }
    let Some(servers) = parsed.get("servers") else {
        return ("NotConfigured", "RouteMissing", selected.map(str::to_owned));
    };
    let Some(servers) = servers.as_table() else {
        return ("Unconfirmed", "VelocityConfigUnreadable", None);
    };
    if let Some(alias) = selected {
        let matches: Vec<_> = servers
            .iter()
            .filter(|(name, _)| name.as_str() != "try" && name.eq_ignore_ascii_case(alias))
            .collect();
        return match matches.as_slice() {
            [] => ("NotConfigured", "RouteMissing", Some(alias.to_owned())),
            [(name, address)] => match endpoint(address) {
                Some(address) if address == backend => {
                    ("Match", "RouteMatches", Some((*name).clone()))
                }
                Some(_) => ("Mismatch", "RouteEndpointMismatch", Some((*name).clone())),
                None => (
                    "Unconfirmed",
                    "RouteEndpointUnsupported",
                    Some((*name).clone()),
                ),
            },
            _ => ("Unconfirmed", "SelectRoute", Some(alias.to_owned())),
        };
    }
    let mut servers = servers
        .iter()
        .filter(|(name, _)| name.as_str() != "try")
        .peekable();
    if servers.peek().is_none() {
        return ("NotConfigured", "RouteMissing", None);
    }
    let mut matching = Vec::new();
    let mut unsupported = false;
    for (name, value) in servers {
        match endpoint(value) {
            Some(address) if address == backend && label(name) => matching.push(name.clone()),
            Some(address) if address != backend => {}
            _ => unsupported = true,
        }
    }
    match matching.as_slice() {
        [name] if !unsupported => ("Match", "RouteMatches", Some(name.clone())),
        [] if unsupported => ("Unconfirmed", "RouteEndpointUnsupported", None),
        _ => ("Unconfirmed", "SelectRoute", None),
    }
}

fn configured_endpoint(item: &Instance) -> SocketAddrV4 {
    SocketAddrV4::new(instance_listen_address(item), item.port)
}

fn active(root: &Path, name: &str) -> Result<Instance> {
    if !token(name) {
        return Err("InvalidInstanceName".into());
    }
    child_directory(root, "registry")?;
    let item: Instance =
        serde_json::from_slice(&bounded(&record_path(root, name), CONFIG_LIMIT)?).map_err(fail)?;
    let registered = scan_policy::registered_path(root, name, item.directory.as_deref())?;
    if item.name != name
        || item.port == 0
        || item.removed
        || !scan_policy::admitted_path(root, name, &registered)?
    {
        return Err("InstanceExcludedOrInvalid".into());
    }
    Ok(item)
}

fn select_proxy(root: &Path, explicit: Option<&str>) -> Result<Option<Instance>> {
    if let Some(proxy) = explicit {
        let proxy = active(root, proxy)?;
        return if registered_velocity(&proxy).is_some() {
            Ok(Some(proxy))
        } else {
            Err("ProxyUnavailable".into())
        };
    }
    let mut selected = None;
    for entry in fs::read_dir(child_directory(root, "registry")?).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| token(name))
            .ok_or("ProxyUnavailable")?;
        let item: Instance =
            serde_json::from_slice(&bounded(&path, CONFIG_LIMIT)?).map_err(fail)?;
        if item.name != name || item.port == 0 {
            return Err("ProxyUnavailable".into());
        }
        let registered = scan_policy::registered_path(root, name, item.directory.as_deref())?;
        if !item.removed
            && registered_velocity(&item).is_some()
            && scan_policy::admitted_path(root, name, &registered)?
        {
            if selected.is_some() {
                return Err("SelectProxy".into());
            }
            selected = Some(item);
        }
    }
    Ok(selected)
}

struct ConfigEvidence {
    file: PathBuf,
    bytes: Vec<u8>,
    initial: Stamp,
}

impl ConfigEvidence {
    fn read(directory: &Path, name: &str) -> Result<Self> {
        let file = directory.join(name);
        let initial = current_stamp(&file)?;
        let bytes = bounded(&file, CONFIG_LIMIT)?;
        if current_stamp(&file)? != initial {
            return Err("DiagnosticEvidenceUnavailable".into());
        }
        Ok(Self {
            file,
            bytes,
            initial,
        })
    }

    fn unchanged(&self) -> bool {
        bounded(&self.file, CONFIG_LIMIT).is_ok_and(|bytes| bytes == self.bytes)
            && current_stamp(&self.file).ok() == Some(self.initial)
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Runtime {
    state: &'static str,
    run: Option<Run>,
}

fn runtime(root: &Path, item: &Instance) -> Runtime {
    observe_runtime(root, item).unwrap_or(Runtime {
        state: "Unconfirmed",
        run: None,
    })
}

fn observe_runtime(root: &Path, item: &Instance) -> Result<Runtime> {
    let runs = child_directory(root, "runs")?;
    let folder = runs.join(&item.name);
    let owner = ipv4_owner(instance_listen_address(item), item.port)?;
    match fs::symlink_metadata(&folder) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Runtime {
                state: if owner.is_none() {
                    "Stopped"
                } else {
                    "Unconfirmed"
                },
                run: None,
            })
        }
        Ok(_) => {
            no_redirect(&folder)?;
        }
        Err(error) => return Err(fail(error)),
    }
    let identity_file = folder.join("identity.json");
    let identity = open_local(&identity_file)?;
    let run_stamp = stamp(&identity)?;
    let run: Run = serde_json::from_slice(&bounded(&identity_file, CONFIG_LIMIT)?).map_err(fail)?;
    let ready = current_stamp(&folder.join("console-ready")).is_ok_and(|marker| {
        marker.created >= run.java.created && marker.changed >= run.java.created
    });
    if !label(&run.id)
        || run_stamp.created < run.java.created
        || run.host.created > run.java.created
        || current_stamp(&identity_file)? != run_stamp
    {
        return Ok(Runtime {
            state: "Unconfirmed",
            run: None,
        });
    }
    let state = match (alive(&run.host)?, alive(&run.java)?, owner) {
        (true, true, Some(pid)) if pid == run.java.pid && ready => "Ready",
        (true, true, Some(pid)) if pid == run.java.pid => "Starting",
        (true, true, None) => "Starting",
        (false, false, None) => "Stopped",
        _ => "Unconfirmed",
    };
    Ok(Runtime {
        state,
        run: matches!(state, "Ready" | "Starting").then_some(run),
    })
}

fn same_record(root: &Path, item: &Instance) -> bool {
    active(root, &item.name)
        .is_ok_and(|new| serde_json::to_value(&new).ok() == serde_json::to_value(item).ok())
}

struct PluginEvidence {
    path: PathBuf,
    identity: Stamp,
    name: String,
}

fn unique_plugin(jar: &PluginEvidence) -> bool {
    let Some(folder) = jar.path.parent() else {
        return false;
    };
    let Ok(entries) = fs::read_dir(folder) else {
        return false;
    };
    for (index, entry) in entries.enumerate() {
        let Ok(entry) = entry else { return false };
        if index >= 256 {
            return false;
        }
        let path = entry.path();
        if path == jar.path
            || !path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jar"))
        {
            continue;
        }
        let Ok(mut file) = open_local(&path) else {
            return false;
        };
        let Ok(identity) = stamp(&file) else {
            return false;
        };
        if identity.size > PLUGIN_LIMIT {
            return false;
        }
        let Some(name) = plugin_descriptor::name(&mut file) else {
            return false;
        };
        if name.eq_ignore_ascii_case(&jar.name) || current_stamp(&path).ok() != Some(identity) {
            return false;
        }
    }
    true
}

fn plugin_jar(
    item: &Instance,
    directory: &Path,
    explicit: Option<&str>,
) -> Result<Option<PluginEvidence>> {
    let selected = if let Some(jar) = explicit {
        Some((jar.to_owned(), None))
    } else {
        let receipt = directory.join("prepare.json");
        match fs::symlink_metadata(&receipt) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(fail(error)),
            Ok(_) => {
                let receipt: Value =
                    serde_json::from_slice(&bounded(&receipt, CONFIG_LIMIT)?).map_err(fail)?;
                receipt["pluginSha256"]
                    .as_str()
                    .map(|hash| ("project-plugin.jar".to_owned(), Some(hash.to_owned())))
                    .or_else(|| {
                        item.core_profile
                            .as_ref()
                            .and_then(test_permissions::diagnostic_plugin)
                            .filter(|(_, hash)| receipt["permissionSha256"] == *hash)
                            .map(|(name, hash)| (name, Some(hash)))
                    })
            }
        }
    };
    let Some((jar, expected)) = selected else {
        return Ok(None);
    };
    let path = child_directory(directory, "plugins")?.join(jar);
    let mut file = open_local(&path)?;
    let identity = stamp(&file)?;
    if identity.size > PLUGIN_LIMIT {
        return Err("PluginJarUnavailable".into());
    }
    let name = plugin_descriptor::name(&mut file).ok_or("PluginDescriptorUnconfirmed")?;
    if let Some(expected) = expected {
        let bytes = bounded(&path, PLUGIN_LIMIT)?;
        if content_revision::revision(&bytes)? != expected {
            return Err("PluginIdentityChanged".into());
        }
    }
    if current_stamp(&path)? != identity {
        return Err("PluginIdentityChanged".into());
    }
    Ok(Some(PluginEvidence {
        path,
        identity,
        name,
    }))
}

fn clean_log_line(raw: &str) -> Option<String> {
    let mut result = String::new();
    // ConPTY writes this exact prompt redraw before a standalone log record.
    // Other carriage returns or cursor moves do not delimit trusted records.
    let raw = raw.strip_prefix("> \r  \r").unwrap_or(raw);
    let mut chars = raw.trim_end_matches('\r').chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.next() == Some('[') {
            // ANSI SGR only: cursor movement can make separate records appear
            // contiguous, so reject those lines instead of joining their text.
            while chars
                .peek()
                .is_some_and(|ch| ch.is_ascii_digit() || *ch == ';')
            {
                chars.next();
            }
            if chars.next() != Some('m') {
                return None;
            }
        } else if ch.is_control() {
            return None;
        } else {
            result.push(ch);
        }
    }
    Some(result)
}

fn paper_message(line: &str) -> Option<(&str, &str)> {
    let (prefix, message) = line.strip_prefix('[')?.split_once("]: ")?;
    if let Some(level) = prefix.strip_prefix("Server thread/") {
        return Some((level, message));
    }
    let (time, level) = prefix.split_once(' ')?;
    is_log_time(time).then_some((level, message))
}

fn is_log_time(time: &str) -> bool {
    time.len() == 8
        && time.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 2 | 5) {
                c == b':'
            } else {
                c.is_ascii_digit()
            }
        })
}

fn plugin_log(bytes: &[u8], name: &str) -> Option<&'static str> {
    let tag = format!("[{name}] ");
    let enabling = format!("Enabling {name} v");
    let disabling = format!("Disabling {name} v");
    let error = format!("Error occurred while enabling {name} v");
    let failed = format!("Failed to enable {name} v");
    let mut state = None;
    for raw in bytes.split_inclusive(|byte| *byte == b'\n') {
        if raw.last() != Some(&b'\n') {
            break;
        }
        let Some(line) = std::str::from_utf8(&raw[..raw.len() - 1])
            .ok()
            .and_then(clean_log_line)
        else {
            continue;
        };
        let Some((level, message)) = paper_message(&line) else {
            continue;
        };
        let payload = message.strip_prefix(&tag).unwrap_or(message);
        if matches!(level, "ERROR" | "WARN")
            && (payload.starts_with(&error) || payload.starts_with(&failed))
        {
            state = Some("Failed");
        } else if level == "INFO" && payload.starts_with(&disabling) && state != Some("Failed") {
            state = Some("Disabled");
        } else if level == "INFO" && payload.starts_with(&enabling) {
            state = Some("Enabling");
        } else if level == "INFO"
            && state == Some("Enabling")
            && message.starts_with(&tag)
            && (payload == "Successfully enabled."
                || payload
                    .strip_prefix("Successfully enabled. (took ")
                    .and_then(|value| value.strip_suffix("ms)"))
                    .is_some_and(|duration| {
                        !duration.is_empty() && duration.bytes().all(|b| b.is_ascii_digit())
                    }))
        {
            state = Some("Enabled");
        }
    }
    state
}

fn log_tail(path: &Path, run: &Run) -> Result<Option<Vec<u8>>> {
    if fs::symlink_metadata(path).is_err_and(|error| error.kind() == io::ErrorKind::NotFound) {
        return Ok(None);
    }
    let mut file = open_local(path)?;
    let before = stamp(&file)?;
    if before.created < run.host.created || before.changed < run.java.created && before.size != 0 {
        return Err("RunLogUnconfirmed".into());
    }
    let offset = before.size.saturating_sub(LOG_LIMIT);
    file.seek(SeekFrom::Start(offset)).map_err(fail)?;
    let mut bytes = Vec::new();
    file.take(LOG_LIMIT).read_to_end(&mut bytes).map_err(fail)?;
    if bytes.len() as u64 != before.size - offset || current_stamp(path)? != before {
        return Err("RunLogUnconfirmed".into());
    }
    if offset > 0 {
        let first_line = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |index| index + 1);
        bytes.drain(..first_line);
    }
    Ok(Some(bytes))
}

fn plugin_observation(
    root: &Path,
    instance: &Instance,
    runtime: &Runtime,
    jar: &PluginEvidence,
) -> Finding {
    let Some(run) = &runtime.run else {
        return finding("Present", "PluginJar", "NoCurrentPluginRun");
    };
    if jar.identity.changed > run.java.created || jar.identity.created > run.java.created {
        return finding("Unconfirmed", "PluginJar", "PluginIdentityChanged");
    }
    if !unique_plugin(jar) {
        return finding("Unconfirmed", "PluginJar", "PluginDescriptorUnconfirmed");
    }
    let folder = root.join("runs").join(&instance.name);
    let stdout = folder.join("stdout.log");
    let stderr = folder.join("stderr.log");
    let read_logs = || -> Result<_> { Ok((log_tail(&stdout, run)?, log_tail(&stderr, run)?)) };
    let Ok(observed) = read_logs() else {
        return finding("Unconfirmed", "CurrentRunLog", "RunLogUnconfirmed");
    };
    if observed.0.is_none() || observed.1.is_none() || read_logs().as_ref() != Ok(&observed) {
        return finding("Unconfirmed", "CurrentRunLog", "RunLogUnconfirmed");
    }
    let primary = observed
        .0
        .as_deref()
        .and_then(|bytes| plugin_log(bytes, &jar.name));
    let secondary = observed
        .1
        .as_deref()
        .and_then(|bytes| plugin_log(bytes, &jar.name));
    let state = match (primary, secondary) {
        (Some("Failed"), _) | (_, Some("Failed")) => Some("Failed"),
        (Some("Disabled"), _) | (_, Some("Disabled")) => Some("Disabled"),
        (Some(a), Some(b)) if a != b => None,
        (Some(a), _) | (_, Some(a)) => Some(a),
        _ => None,
    };
    match state {
        Some(state) => finding(state, "CurrentRunLog", "PluginLifecycleObserved"),
        None => finding("Unconfirmed", "CurrentRunLog", "PluginEventUnconfirmed"),
    }
}

fn paper_player(message: &str) -> Option<&str> {
    let message = message.strip_prefix(": ")?;
    if let Some(player) = message.strip_suffix(" joined the game") {
        return Some(player);
    }
    // Paper 26.3 announces joins through system chat. Use the login
    // event with a socket address and entity ID, since chat can contain
    // user-controlled text.
    let (player, login) = message.split_once("[/")?;
    let (address, entity) = login.split_once("] logged in with entity id ")?;
    address.parse::<std::net::SocketAddr>().ok()?;
    let (id, location) = entity.split_once(" at (")?;
    id.parse::<u32>().ok()?;
    if location.strip_suffix(')')?.is_empty() {
        return None;
    }
    Some(player)
}

fn connection_events(bytes: &[u8], route: Option<&str>) -> Vec<(u32, String)> {
    let mut events = Vec::new();
    for raw in bytes.split_inclusive(|byte| *byte == b'\n') {
        let Some(raw) = raw.strip_suffix(b"\n") else {
            break;
        };
        let Some(line) = std::str::from_utf8(raw).ok().and_then(clean_log_line) else {
            continue;
        };
        let Some((prefix, message)) = line.strip_prefix('[').and_then(|line| line.split_once(']'))
        else {
            continue;
        };
        let Some(time) = prefix
            .strip_suffix(" INFO")
            .filter(|time| is_log_time(time))
        else {
            continue;
        };
        let player = if let Some(route) = route {
            message
                .strip_prefix(": [server connection] ")
                .or_else(|| message.strip_prefix(" [server connection] "))
                .and_then(|message| message.strip_suffix(" has connected"))
                .and_then(|message| message.rsplit_once(" -> "))
                .filter(|(_, actual)| actual.eq_ignore_ascii_case(route))
                .map(|(player, _)| player)
        } else {
            paper_player(message)
        };
        let Some(player) = player.filter(|player| {
            !player.is_empty()
                && player.len() <= 16
                && player
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        }) else {
            continue;
        };
        let mut components = time
            .split(':')
            .filter_map(|value| value.parse::<u32>().ok());
        let (Some(hour), Some(minute), Some(second)) =
            (components.next(), components.next(), components.next())
        else {
            continue;
        };
        if hour < 24 && minute < 60 && second < 60 {
            events.push((hour * 3600 + minute * 60 + second, player.to_owned()));
        }
    }
    events
}

fn linked_sockets(endpoint: SocketAddrV4, proxy: &Run, backend: &Run) -> bool {
    let mut size = 0;
    // TCP_TABLE_OWNER_PID_CONNECTIONS, AF_INET, no network traffic.
    unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, 2, 4, 0) };
    for _ in 0..3 {
        let mut table = vec![0u32; (size as usize).div_ceil(4).max(1)];
        let code = unsafe { GetExtendedTcpTable(table.as_mut_ptr().cast(), &mut size, 0, 2, 4, 0) };
        if code == 122 {
            continue;
        }
        if code != 0 {
            return false;
        }
        let Some(rows) = table.get(1..1 + table[0] as usize * 6) else {
            return false;
        };
        return rows.chunks_exact(6).any(|candidate| {
            // ESTABLISHED, current proxy PID, destination at this exact endpoint.
            if candidate[0] != 5
                || candidate[5] != proxy.java.pid
                || Ipv4Addr::from(candidate[3].to_ne_bytes()) != *endpoint.ip()
                || u16::from_be(candidate[4] as u16) != endpoint.port()
            {
                return false;
            }
            rows.chunks_exact(6).any(|peer| {
                peer[0] == 5
                    && peer[5] == backend.java.pid
                    && peer[1..3] == candidate[3..5]
                    && peer[3..5] == candidate[1..3]
            })
        });
    }
    false
}

fn paired_events(proxy_events: &[(u32, String)], backend_events: &[(u32, String)]) -> bool {
    proxy_events.iter().any(|(time, name)| {
        backend_events.iter().any(|(other, player)| {
            let difference = time.abs_diff(*other);
            name == player && (difference <= 10 || difference >= 86_390)
        })
    })
}

fn forwarding_observed(
    root: &Path,
    proxy: (&Instance, &Run),
    backend: (&Instance, &Run),
    route: &str,
) -> bool {
    let (proxy, proxy_run) = proxy;
    let (backend, backend_run) = backend;
    if !linked_sockets(configured_endpoint(backend), proxy_run, backend_run) {
        return false;
    }
    let proxy_log = root.join("runs").join(&proxy.name).join("stdout.log");
    let backend_log = root.join("runs").join(&backend.name).join("stdout.log");
    let (Ok(Some(left)), Ok(Some(right))) = (
        log_tail(&proxy_log, proxy_run),
        log_tail(&backend_log, backend_run),
    ) else {
        return false;
    };
    paired_events(
        &connection_events(&left, Some(route)),
        &connection_events(&right, None),
    ) && log_tail(&proxy_log, proxy_run).as_ref() == Ok(&Some(left))
        && log_tail(&backend_log, backend_run).as_ref() == Ok(&Some(right))
        && linked_sockets(configured_endpoint(backend), proxy_run, backend_run)
}

pub(super) fn inspect(root: &Path, args: &[String]) -> Result<Value> {
    let (name, options) = match args {
        [name] => (name, Options::default()),
        [name, raw] if raw.len() <= 1024 => (
            name,
            serde_json::from_str(raw).map_err(|_| "InvalidDiagnosticArguments")?,
        ),
        _ => return Err("InvalidDiagnosticArguments".into()),
    };
    if options.proxy.as_deref().is_some_and(|value| !token(value))
        || options.route.as_deref().is_some_and(|value| !label(value))
        || options
            .plugin
            .as_deref()
            .is_some_and(|value| !jar_basename(value))
    {
        return Err("InvalidDiagnosticArguments".into());
    }

    let item = active(root, name)?;
    let subject_is_proxy = registered_velocity(&item).is_some();
    let directory = managed_directory(root, name, item.directory.as_deref()).ok();
    let proxy_selection = if subject_is_proxy {
        Ok(None)
    } else {
        select_proxy(root, options.proxy.as_deref())
    };
    let proxy = proxy_selection.as_ref().ok().and_then(|item| item.as_ref());
    let subject_run = runtime(root, &item);
    let proxy_run = proxy.map(|item| runtime(root, item));
    let backend_file = directory
        .as_ref()
        .and_then(|dir| ConfigEvidence::read(dir, "server.properties").ok());
    let proxy_directory =
        proxy.and_then(|item| managed_directory(root, &item.name, item.directory.as_deref()).ok());
    let proxy_file = proxy_directory
        .as_ref()
        .and_then(|path| ConfigEvidence::read(path, "velocity.toml").ok());
    let core_stamp = |directory: Option<&Path>| {
        directory.and_then(|dir| current_stamp(&dir.join("server.jar")).ok())
    };
    let initial_subject_core = core_stamp(directory.as_deref());
    let initial_proxy_core = core_stamp(proxy_directory.as_deref());
    let backend = backend_file
        .as_ref()
        .and_then(|file| backend_endpoint(&file.bytes));
    let (state, reason, route) = match (subject_is_proxy, &proxy_selection, backend, &proxy_file) {
        (true, _, _, _) => ("NotApplicable", "SelectBackend", None),
        (_, Err(error), _, _) if error == "SelectProxy" => {
            ("Unconfirmed", "SelectProxy", options.route.clone())
        }
        (_, Err(_), _, _) => ("Unconfirmed", "ProxyUnavailable", options.route.clone()),
        (_, Ok(None), _, _) => ("NotConfigured", "NoProxy", options.route.clone()),
        (_, Ok(Some(_)), None, _) => (
            "Unconfirmed",
            "BackendEndpointUnconfirmed",
            options.route.clone(),
        ),
        (_, Ok(Some(_)), Some(address), _) if address != configured_endpoint(&item) => {
            ("Mismatch", "BackendBindingChanged", options.route.clone())
        }
        (_, Ok(Some(proxy)), Some(address), Some(file)) => route_finding(
            &file.bytes,
            configured_endpoint(proxy),
            address,
            options.route.as_deref(),
        ),
        _ => (
            "Unconfirmed",
            "VelocityConfigUnreadable",
            options.route.clone(),
        ),
    };

    let mut config = configuration(
        finding(
            state,
            if subject_is_proxy {
                "None"
            } else {
                "VelocityConfig"
            },
            reason,
        ),
        proxy.map(|item| item.name.clone()).or(options.proxy),
        route,
    );
    let jar = if subject_is_proxy {
        Ok(None)
    } else {
        directory
            .as_ref()
            .ok_or("PluginJarUnavailable".to_owned())
            .and_then(|dir| plugin_jar(&item, dir, options.plugin.as_deref()))
    };
    let mut plugin = match &jar {
        Ok(_) if subject_is_proxy => finding("NotApplicable", "None", "PluginNotApplicable"),
        Ok(None) => finding("Unconfirmed", "None", "SelectPlugin"),
        Ok(Some(jar)) => plugin_observation(root, &item, &subject_run, jar),
        Err(error) => finding(
            "Unconfirmed",
            "PluginJar",
            match error.as_str() {
                "PluginIdentityChanged" => "PluginIdentityChanged",
                "PluginDescriptorUnconfirmed" => "PluginDescriptorUnconfirmed",
                _ => "PluginJarUnavailable",
            },
        ),
    };
    let forwarding = config.finding.state == "Match"
        && subject_run.state == "Ready"
        && proxy_run.as_ref().is_some_and(|run| run.state == "Ready")
        && config
            .route
            .as_ref()
            .zip(subject_run.run.as_ref())
            .is_some_and(|(route, run)| {
                proxy
                    .zip(proxy_run.as_ref().and_then(|reading| reading.run.as_ref()))
                    .is_some_and(|(proxy, proxy_run)| {
                        // Paper can rewrite server.properties during startup. The
                        // exact reciprocal TCP sockets establish the live endpoint.
                        proxy_file
                            .as_ref()
                            .is_some_and(|file| file.initial.changed <= proxy_run.java.created)
                            && forwarding_observed(root, (proxy, proxy_run), (&item, run), route)
                    })
            });
    let current_subject = same_record(root, &item)
        && runtime(root, &item) == subject_run
        && core_stamp(directory.as_deref()) == initial_subject_core
        && subject_run.run.as_ref().is_none_or(|run| {
            initial_subject_core.is_some_and(|core| core.changed <= run.java.created)
        });
    let current_proxy = proxy.zip(proxy_run.as_ref()).is_none_or(|(item, before)| {
        same_record(root, item)
            && runtime(root, item) == *before
            && core_stamp(proxy_directory.as_deref()) == initial_proxy_core
            && before.run.as_ref().is_none_or(|run| {
                initial_proxy_core.is_some_and(|core| core.changed <= run.java.created)
            })
    });
    if !same_record(root, &item)
        || proxy.is_some_and(|item| !same_record(root, item))
        || backend_file.as_ref().is_some_and(|file| !file.unchanged())
        || proxy_file.as_ref().is_some_and(|file| !file.unchanged())
        || core_stamp(directory.as_deref()) != initial_subject_core
        || core_stamp(proxy_directory.as_deref()) != initial_proxy_core
    {
        config.finding = finding("Unconfirmed", "VelocityConfig", "EvidenceChanged");
    }
    if !subject_is_proxy
        && (!current_subject
            || jar
                .as_ref()
                .ok()
                .and_then(|jar| jar.as_ref())
                .is_some_and(|jar| current_stamp(&jar.path).ok() != Some(jar.identity)))
    {
        plugin = finding("Unconfirmed", "CurrentRunLog", "EvidenceChanged");
    }
    let (runtime_state, runtime_reason) = if subject_is_proxy {
        ("NotApplicable", "SelectBackend")
    } else if !current_subject || !current_proxy {
        ("Unconfirmed", "EvidenceChanged")
    } else if let Some(proxy_run) = &proxy_run {
        match (subject_run.state, proxy_run.state) {
            ("Ready", "Ready") => ("Ready", "BothRunsReady"),
            ("Unconfirmed", _) | (_, "Unconfirmed") => ("Unconfirmed", "RunUnconfirmed"),
            ("Starting", _) | (_, "Starting") => ("Starting", "RunStarting"),
            _ => ("Stopped", "RunStopped"),
        }
    } else {
        ("Unconfirmed", "ProxyUnavailable")
    };
    Ok(json!({
        "checkedAt": epoch_ms(),
        "instance": name,
        "configuration": config,
        "runtime": {
            "state": runtime_state, "source": if subject_is_proxy { "None" } else { "ManagedRun" },
            "reason": runtime_reason,
            "forwarding": if forwarding && runtime_state == "Ready" && config.finding.state == "Match" { "Observed" } else { "Unconfirmed" },
            "backendRun": current_subject.then(|| subject_run.run.as_ref().map(|run| &run.id)).flatten(),
            "proxyRun": current_proxy.then(|| proxy_run.as_ref().and_then(|reading| reading.run.as_ref()).map(|run| &run.id)).flatten()
        },
        "plugin": {
            "state": plugin.state, "source": plugin.source, "reason": plugin.reason,
            "name": jar.as_ref().ok().and_then(|jar| jar.as_ref()).map(|jar| &jar.name)
        },
        "player": finding("Unverified", "None", "PlayerEvidenceNotRecorded")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_uses_expected_endpoint_not_similar_names_and_fails_closed() {
        let proxy = "127.0.0.1:25577".parse().unwrap();
        let backend = "127.0.0.1:25565".parse().unwrap();
        let prefix = "bind = \"127.0.0.1:25577\"\n[servers]\ntry = [\"lobby\"]\n";
        let check = |rows: &str, selected| {
            route_finding(
                format!("{prefix}{rows}").as_bytes(),
                proxy,
                backend,
                selected,
            )
        };
        assert_eq!(
            check("lobby = '127.0.0.1:25565'", None),
            ("Match", "RouteMatches", Some("lobby".into()))
        );
        assert_eq!(check("lobby = '127.0.0.2:25565'", None).0, "Unconfirmed");
        assert_eq!(
            check("lobby = '127.0.0.2:25565'", Some("lobby")).0,
            "Mismatch"
        );
        assert_eq!(
            check("lobby = '127.0.0.1:25566'", Some("lobby")).0,
            "Mismatch"
        );
        assert_eq!(
            check("lobby = '127.0.0.1:25565'", Some("lobby-2")).0,
            "NotConfigured"
        );
        assert_eq!(
            check("lobby = '127.0.0.1:25565'\nother = '127.0.0.1:25565'", None).1,
            "SelectRoute"
        );
        assert_eq!(
            check("lobby = 'localhost:25565'", Some("lobby")).1,
            "RouteEndpointUnsupported"
        );
        assert_eq!(
            check("lobby = '127.0.0.1:25565'\nlobby = \"other\"", None).1,
            "VelocityConfigUnreadable"
        );
        assert_eq!(
            check(
                "lobby = '127.0.0.1:25565'\nLobby = '127.0.0.1:25565'",
                Some("lobby")
            )
            .1,
            "SelectRoute"
        );
        assert_eq!(check("", None).0, "NotConfigured");
        assert_eq!(
            route_finding(prefix.as_bytes(), backend, proxy, None).1,
            "ProxyBindingChanged"
        );
    }

    #[test]
    fn properties_require_specific_address_and_unique_keys() {
        assert_eq!(
            backend_endpoint(b"server-ip=127.0.0.1\nserver-port: 25565\n")
                .unwrap()
                .port(),
            25565
        );
        assert!(backend_endpoint(b"server-ip=\nserver-port=25565\n").is_none());
        assert!(backend_endpoint(
            b"server-ip=127.0.0.1\nserver-port=25565\nfoo=1\rserver-port=25566\n"
        )
        .is_none());
        assert!(backend_endpoint(b"server-ip=127.0.0.1\nserver-port==25565\n").is_none());
        assert!(
            backend_endpoint(b"server-ip=127.0.0.1\nserver-port=25565\nserver-port=25565\n")
                .is_none()
        );
        assert!(backend_endpoint(
            b"server-ip=127.0.0.1\nserver-port=25565\nserver\\u002dport=10\n"
        )
        .is_none());
    }

    #[test]
    fn plugin_requires_complete_current_logger_events() {
        let enabling = "[Server thread/INFO]: [LuckPerms] Enabling LuckPerms v5.5.85\n";
        let success =
            "[17:44:14 INFO]: \x1b[32m[LuckPerms] Successfully enabled. (took 526ms)\x1b[0m\n";
        let disabled = "[Server thread/INFO]: [LuckPerms] Disabling LuckPerms v5.5.85\n";
        let failed = "[Server thread/ERROR]: Error occurred while enabling LuckPerms v5.5.85\n";
        let state = |text: &str| plugin_log(text.as_bytes(), "LuckPerms");
        assert_eq!(state(enabling), Some("Enabling"));
        assert_eq!(state(success), None);
        assert_eq!(
            state(&format!("{enabling}{}", success.trim_end())),
            Some("Enabling")
        );
        assert_eq!(state(&format!("{enabling}{success}")), Some("Enabled"));
        assert_eq!(
            state(&format!("{enabling}> \r  \r{success}")),
            Some("Enabled")
        );
        assert_eq!(
            state(&format!("{enabling}{success}{disabled}")),
            Some("Disabled")
        );
        assert_eq!(
            state(&format!("{enabling}{success}{failed}{disabled}")),
            Some("Failed")
        );
        assert_eq!(
            state(&format!(
                "{enabling}[17:44:14 INFO]: <Player> [LuckPerms] Successfully enabled.\n"
            )),
            Some("Enabling")
        );
        assert_eq!(
            state(&format!(
                "{enabling}[17:44:14 INFO]: \x1b[2J[LuckPerms] Successfully enabled.\n"
            )),
            Some("Enabling")
        );
        assert!(!jar_basename("../secret.jar"));
        assert!(!jar_basename("C:secret.jar"));
        assert!(!jar_basename("NUL.jar"));
    }

    #[test]
    fn established_loopback_pair_must_match_the_two_socket_owners() {
        use std::net::{TcpListener, TcpStream};
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let endpoint = match listener.local_addr().unwrap() {
            std::net::SocketAddr::V4(address) => address,
            _ => unreachable!(),
        };
        let owner = identity(std::process::id()).unwrap().unwrap();
        let mut run = Run {
            id: "test-run".into(),
            host: owner.clone(),
            java: owner,
        };
        assert!(!linked_sockets(endpoint, &run, &run));
        let _client = TcpStream::connect(endpoint).unwrap();
        let _backend = listener.accept().unwrap();
        assert!(linked_sockets(endpoint, &run, &run));
        let verified_run = run.clone();
        run.java.pid = 0;
        assert!(!linked_sockets(endpoint, &verified_run, &run));
    }

    #[test]
    fn forwarding_requires_a_distinct_velocity_event_and_paper_join() {
        let velocity = b"[23:59:59 INFO] [server connection] Probe -> paper has connected\n";
        let paper = b"> \r  \r[00:00:01 INFO]: Probe joined the game\r\n";
        let parsed = connection_events(velocity, Some("paper"));
        assert!(paired_events(&parsed, &connection_events(paper, None)));
        let native_velocity =
            b"[23:59:59 INFO]: [server connection] Probe -> paper has connected\r\n";
        assert_eq!(connection_events(native_velocity, Some("paper")), parsed);
        assert!(connection_events(velocity, Some("different")).is_empty());
        assert!(!paired_events(
            &parsed,
            &connection_events(b"[00:00:12 INFO]: Probe joined the game\n", None)
        ));
        assert!(
            connection_events(b"[00:00:01 INFO]: <Probe> Probe joined the game\n", None).is_empty()
        );
        let paper_26 = b"> \r  \r[00:00:01 INFO]: Probe[/127.0.0.1:57221] logged in with entity id 1 at ([minecraft:overworld]-9.5, -60.0, 8.5)\r\n";
        assert!(paired_events(&parsed, &connection_events(paper_26, None)));
        assert!(connection_events(
            b"[00:00:01 INFO]: System chat: \x1b[93mProbe joined the game\x1b[0m\n",
            None
        )
        .is_empty());
        assert!(connection_events(
            b"[00:00:01 INFO]: Probe[/invalid] logged in with entity id 1 at (0, 0, 0)\n",
            None
        )
        .is_empty());
    }

    fn fingerprint(root: &Path) -> std::collections::BTreeMap<PathBuf, (Stamp, Option<String>)> {
        fn collect(
            path: &Path,
            entries: &mut std::collections::BTreeMap<PathBuf, (Stamp, Option<String>)>,
        ) {
            let metadata = fs::metadata(path).unwrap();
            let hash = metadata
                .is_file()
                .then(|| content_revision::file_revision(path).unwrap());
            entries.insert(
                path.to_owned(),
                (
                    Stamp {
                        created: metadata.creation_time(),
                        changed: metadata.last_write_time(),
                        size: metadata.len(),
                    },
                    hash,
                ),
            );
            if metadata.is_dir() {
                for entry in fs::read_dir(path).unwrap() {
                    collect(&entry.unwrap().path(), entries);
                }
            }
        }
        let mut result = std::collections::BTreeMap::new();
        collect(root, &mut result);
        result
    }

    #[test]
    fn workspace_diagnostics_preserve_bytes_timestamps_and_runtime() {
        let path =
            std::env::temp_dir().join(format!("ih-diagnostics-{}-{}", std::process::id(), clock()));
        let workspace = path.to_str().unwrap().to_owned();
        dispatch(&[workspace.clone(), "init".into()]).unwrap();
        let root = safe_directory(&path).unwrap();
        let register = |name: &str, port: u16, family: &str| {
            fs::create_dir(root.join("instances").join(name)).unwrap();
            let mut record = json!({"name":name,"java":"C:/isolated/java.exe","port":port});
            if family == "VELOCITY" {
                record["core_profile"] =
                    json!({"family":family, "minecraft":"26.3", "build":"38", "sha256":"fixture"});
            }
            fs::write(record_path(&root, name), record.to_string()).unwrap();
        };
        let external = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let backend_port = external.local_addr().unwrap().port();
        let proxy_reservation = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let proxy_port = proxy_reservation.local_addr().unwrap().port();
        drop(proxy_reservation);
        register("proxy", proxy_port, "VELOCITY");
        register("backend", backend_port, "PAPER");
        let configuration = root.join("instances/proxy/velocity.toml");
        let toml = format!(
            "bind = '127.0.0.1:{proxy_port}'\n[servers]\nother = '127.0.0.1:{backend_port}'\n"
        );
        fs::write(&configuration, &toml).unwrap();
        fs::write(
            root.join("instances/backend/server.properties"),
            format!("server-ip=127.0.0.1\nserver-port={backend_port}\n"),
        )
        .unwrap();
        let runs = root.join("runs/backend");
        fs::create_dir(&runs).unwrap();
        fs::write(
            runs.join("stdout.log"),
            "[09:00:00 INFO]: [LuckPerms] Successfully enabled.",
        )
        .unwrap();
        let inspect = |options: &str| {
            dispatch(&[
                workspace.clone(),
                "instance-diagnostics".into(),
                "backend".into(),
                options.into(),
            ])
            .unwrap()
        };

        let before = fingerprint(&root);
        let reported = inspect("{}");
        assert_eq!(reported["configuration"]["state"], "Match");
        assert_eq!(reported["configuration"]["route"], "other");
        assert_eq!(reported["runtime"]["state"], "Unconfirmed");
        assert!(reported["runtime"]["backendRun"].is_null());
        assert_eq!(reported["plugin"]["state"], "Unconfirmed");
        assert_eq!(reported["player"]["state"], "Unverified");
        assert_eq!(fingerprint(&root), before);
        assert_eq!(external.local_addr().unwrap().port(), backend_port);

        let evidence =
            ConfigEvidence::read(&root.join("instances/proxy"), "velocity.toml").unwrap();
        fs::write(&configuration, "bind = '\nsecret = 'excluded'").unwrap();
        assert!(!evidence.unchanged());
        let before = fingerprint(&root);
        let reported = inspect("{}");
        assert_eq!(reported["configuration"]["state"], "Unconfirmed");
        assert!(!reported.to_string().contains("excluded"));
        assert_eq!(fingerprint(&root), before);
        fs::write(&configuration, &toml).unwrap();
        register("proxy-extra", proxy_port, "VELOCITY");
        let before = fingerprint(&root);
        assert_eq!(inspect("{}")["configuration"]["reason"], "SelectProxy");
        assert_eq!(
            inspect(r#"{"proxy":"proxy","route":"other"}"#)["configuration"]["state"],
            "Match"
        );
        assert_eq!(fingerprint(&root), before);
        let link = root.join("instances/proxy/linked.toml");
        match std::os::windows::fs::symlink_file(&configuration, &link) {
            Ok(()) => assert!(open_local(&link).is_err()),
            Err(error) if error.raw_os_error() == Some(1314) => {}
            Err(error) => panic!("Could not create diagnostic link fixture: {error}"),
        }
        fs::remove_dir_all(path).unwrap();
    }
}
