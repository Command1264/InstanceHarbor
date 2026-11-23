//! Snapshot one explicitly selected 1.0 workspace. No setup, registration, or control verbs.
use super::*;
use std::{
    ffi::OsString,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Prefix},
};

const LIMIT: u64 = 512 * 1024;
const MAX_ENTRIES: usize = 1024;
const MAX_ROWS: usize = 128;
const SHARE_ALL: u32 = 7;
const OPEN_REPARSE: u32 = 0x00200000;
const BACKUP_SEMANTICS: u32 = 0x02000000;

#[link(name = "kernel32")]
extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
    fn GetFinalPathNameByHandleW(handle: *mut c_void, path: *mut u16, len: u32, flags: u32) -> u32;
    fn SystemTimeToFileTime(value: *const [u16; 8], result: *mut u64) -> i32;
}

fn same(a: &Path, b: &Path) -> bool {
    path_format::display(a).eq_ignore_ascii_case(&path_format::display(b))
}
fn normalized_input(value: &str) -> Result<PathBuf> {
    let native = value.replace('/', "\\");
    let path = Path::new(&native);
    let drive = match path.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(drive) => drive,
            _ => return Err("LegacyLocalPathRequired".into()),
        },
        _ => return Err("LegacyLocalPathRequired".into()),
    };
    if !path.is_absolute()
        || native[3..].split('\\').any(|part| {
            part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || part.contains([':', '*', '?', '"', '<', '>', '|'])
                || part.chars().any(char::is_control)
        })
    {
        return Err("LegacyLocalPathRequired".into());
    }
    let root = [drive as u16, 58, 92, 0];
    if unsafe { GetDriveTypeW(root.as_ptr()) } != 3 {
        return Err("LegacyLocalPathRequired".into());
    }
    Ok(path.components().collect())
}

fn checked_open(path: &Path, directory: bool) -> io::Result<File> {
    // Reject redirects in every component. Check the actual opened handle too,
    // before reading a byte, to catch replacement between validation and open.
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        if !matches!(component, Component::Prefix(_) | Component::RootDir)
            && fs::symlink_metadata(&prefix)?.file_attributes() & 0x400 != 0
        {
            return Err(io::Error::other("LegacyPathRedirected"));
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .share_mode(if directory { 3 } else { SHARE_ALL })
        .custom_flags(OPEN_REPARSE | if directory { BACKUP_SEMANTICS } else { 0 })
        .open(path)?;
    let metadata = file.metadata()?;
    if metadata.file_attributes() & 0x400 != 0 || metadata.is_dir() != directory {
        return Err(io::Error::other("LegacyUnexpectedFileType"));
    }
    let mut buffer = vec![0u16; 32768];
    let len = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    } as usize;
    if len == 0
        || len >= buffer.len()
        || !same(Path::new(&OsString::from_wide(&buffer[..len])), path)
    {
        return Err(io::Error::other("LegacyPathRedirected"));
    }
    Ok(file)
}

fn snapshot(path: &Path) -> io::Result<String> {
    let mut bytes = Vec::new();
    checked_open(path, false)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::other("LegacyFileTooLarge"));
    }
    String::from_utf8(bytes)
        .map(|s| s.trim_start_matches('\u{feff}').to_owned())
        .map_err(io::Error::other)
}
fn document(path: &Path) -> io::Result<Value> {
    serde_json::from_str(&snapshot(path)?).map_err(io::Error::other)
}
fn optional_document(path: &Path) -> Result<Option<Value>> {
    match document(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("LegacySettingsUnreadable".into()),
    }
}
fn children(path: &Path, budget: &mut usize) -> Result<Vec<fs::DirEntry>> {
    let _directory = checked_open(path, true).map_err(|_| "LegacyDirectoryUnreadable")?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|_| "LegacyDirectoryUnreadable")? {
        *budget = budget.checked_sub(1).ok_or("LegacyScanLimit")?;
        entries.push(entry.map_err(|_| "LegacyDirectoryUnreadable")?);
    }
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

struct Policy {
    allow: Vec<PathBuf>,
    deny: Vec<PathBuf>,
}
fn within(path: &Path, scope: &Path) -> bool {
    path.ancestors().any(|p| same(p, scope))
}
impl Policy {
    fn read(path: &Path, instances: &Path) -> Result<Self> {
        let policy = document(path).map_err(|_| "LegacyPolicyUnreadable")?;
        if policy["schemaVersion"] != 1 {
            return Err("LegacyPolicyUnsupported".into());
        }
        let parse = |key: &str| -> Result<Vec<PathBuf>> {
            let items = policy[key].as_array().ok_or("LegacyPolicyUnsupported")?;
            if items.len() > MAX_ENTRIES {
                return Err("LegacyScanLimit".into());
            }
            items
                .iter()
                .map(|item| {
                    let raw = item
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or("LegacyPolicyUnsupported")?;
                    let relative = Path::new(raw);
                    let resolved = if relative.is_absolute() {
                        normalized_input(raw)?
                    } else {
                        normalized_input(
                            instances
                                .join(relative)
                                .to_str()
                                .ok_or("LegacyPolicyUnsupported")?,
                        )?
                    };
                    if !within(&resolved, instances) {
                        return Err("LegacyPolicyOutsideSource".into());
                    }
                    match checked_open(&resolved, true) {
                        Ok(_) => {}
                        Err(error) if key == "deny" && error.kind() == io::ErrorKind::NotFound => {}
                        Err(_) => return Err("LegacyPolicyUnreadable".into()),
                    }
                    Ok(resolved)
                })
                .collect()
        };
        let allow = parse("allow")?;
        if allow.is_empty()
            || allow.iter().any(|path| {
                let valid_audience = ["Human", "Agent"].iter().any(|name| {
                    let audience = instances.join(name);
                    path.ancestors().take(3).any(|p| same(p, &audience))
                });
                !valid_audience && !same(path, &instances.join("Velocity"))
            })
        {
            return Err("LegacyPolicyUnsupported".into());
        }
        Ok(Self {
            allow,
            deny: parse("deny")?,
        })
    }
    fn includes(&self, path: &Path) -> bool {
        self.allow.iter().any(|p| within(path, p)) && !self.deny.iter().any(|p| within(path, p))
    }
    fn intersects(&self, path: &Path) -> bool {
        self.allow
            .iter()
            .any(|p| within(p, path) || within(path, p))
            && !self.deny.iter().any(|p| within(path, p))
    }
}

fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut values = text.lines().filter_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then_some(value.trim())
    });
    let first = values.next()?;
    values.next().is_none().then_some(first)
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn configuration(
    path: &Path,
    kind: &str,
    project: &str,
    name: &str,
) -> Result<(u16, Option<String>)> {
    if kind == "Velocity" {
        let text = snapshot(&path.join("velocity.toml")).map_err(|_| "ConfigUnreadable")?;
        let bind = field(text.split("\n[").next().unwrap_or(""), "bind").ok_or("ConfigInvalid")?;
        let bind = bind
            .strip_prefix('"')
            .and_then(|v| v.split_once('"'))
            .filter(|(_, tail)| tail.trim().is_empty() || tail.trim_start().starts_with('#'))
            .map(|(v, _)| v)
            .ok_or("ConfigInvalid")?;
        let port = bind
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse::<u16>().ok())
            .filter(|p| *p > 0)
            .ok_or("ConfigInvalid")?;
        return Ok((port, None));
    }
    let metadata = document(&path.join("workspace.json")).map_err(|_| "ConfigUnreadable")?;
    checked_open(&path.join("server.jar"), false).map_err(|_| "ConfigUnreadable")?;
    let props = snapshot(&path.join("server.properties")).map_err(|_| "ConfigUnreadable")?;
    let eula = snapshot(&path.join("eula.txt")).map_err(|_| "ConfigUnreadable")?;
    let port = field(&props, "server-port")
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|p| *p > 0)
        .ok_or("ConfigInvalid")?;
    let alias = metadata["javaAlias"]
        .as_str()
        .filter(|s| identifier(s))
        .ok_or("ConfigInvalid")?;
    if metadata["schemaVersion"] != 2
        || metadata["audience"] != kind
        || metadata["project"] != project
        || metadata["instance"] != name
        || metadata["port"].as_u64() != Some(u64::from(port))
        || !identifier(project)
        || !identifier(name)
        || field(&props, "server-ip") != Some("127.0.0.1")
        || field(&props, "online-mode") != Some("false")
        || metadata["eulaAccepted"] != true
        || eula.trim() != "eula=true"
    {
        return Err("ConfigInvalid".into());
    }
    Ok((port, Some(alias.to_owned())))
}

fn setting_runtime(document: Option<&Value>) -> Result<Option<&Value>> {
    let Some(doc) = document else {
        return Ok(None);
    };
    if doc["schemaVersion"] != 1 || !doc["settings"].is_object() {
        return Err("LegacySettingsUnsupported".into());
    }
    Ok(doc.pointer("/settings/java/runtime"))
}
fn java_preview(root: &Path, path: &Path, alias: Option<&str>) -> Result<Value> {
    let settings = root.join("settings");
    let local = optional_document(&path.join("instanceharbor-settings.json"))?;
    let global = optional_document(&settings.join("manager-settings.json"))?;
    let l = setting_runtime(local.as_ref())?;
    let g = setting_runtime(global.as_ref())?;
    let pins = if l.is_none() && g.is_some_and(|v| v["kind"] != "legacy") {
        optional_document(&settings.join("java-legacy-pins.json"))?
    } else {
        None
    };
    let mut pin = None;
    if let Some(pins) = &pins {
        let entries = pins["paths"]
            .as_object()
            .filter(|_| pins["schemaVersion"] == 1)
            .ok_or("LegacySettingsUnsupported")?;
        for (key, value) in entries {
            if same(Path::new(key), path) {
                if pin.replace(value).is_some() {
                    return Err("LegacySettingsUnsupported".into());
                }
            }
        }
    }
    let (choice, source) = match l {
        Some(value) if value["kind"] != "inherit" => (Some(value), "instance"),
        _ if pin.is_some() => (pin, "pin"),
        _ => (g, "global"),
    };
    let kind = choice
        .map_or(Some("legacy"), |c| c["kind"].as_str())
        .ok_or("LegacySettingsUnsupported")?;
    let registered;
    let velocity;
    let (selected, source, alias) = match kind {
        "path" | "mise" => (choice.and_then(|v| v["path"].as_str()), source, None),
        "legacy" => {
            let alias = if alias.is_some() {
                alias
            } else {
                velocity = optional_document(&settings.join("velocity-launch.json"))?;
                velocity.as_ref().and_then(|d| d["javaAlias"].as_str())
            };
            registered = optional_document(&settings.join("java-runtimes.json"))?;
            if registered
                .as_ref()
                .is_some_and(|v| v["schemaVersion"] != 1 || !v["paths"].is_object())
            {
                return Err("LegacySettingsUnsupported".into());
            }
            (
                alias.and_then(|a| registered.as_ref()?.get("paths")?.get(a)?.as_str()),
                "registry",
                alias,
            )
        }
        _ => return Err("LegacySettingsUnsupported".into()),
    };
    let selected = selected.map(normalized_input).transpose()?;
    let exists = selected.as_ref().is_some_and(|p| {
        p.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("java.exe"))
            && checked_open(p, false).is_ok()
    });
    Ok(json!({"kind":kind,"source":source,"alias":alias,
        "path":selected.as_deref().map(path_format::display), "fileVerified":exists}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    schema_version: u32,
    target_path: PathBuf,
    run_id: String,
    port: u16,
    mode: String,
    host_pid: u32,
    host_start_utc: String,
    child_pid: u32,
    child_start_utc: String,
    ready: bool,
}
fn hex_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn jar_name(root: &Path, kind: &str) -> Option<String> {
    if kind != "Velocity" {
        return Some("server.jar".into());
    }
    let value = document(&root.join("settings").join("velocity-launch.json")).ok()?;
    let name = value["jarName"].as_str()?;
    (value["schemaVersion"] == 1
        && Path::new(name).file_name().and_then(|s| s.to_str()) == Some(name)
        && !name.contains([':', '\\', '/', '*', '?'])
        && name.to_ascii_lowercase().ends_with(".jar"))
    .then(|| name.to_owned())
}
fn timestamp(value: &str) -> Option<u64> {
    let time = value.as_bytes();
    if time.len() != 28
        || !time.is_ascii()
        || time[27] != b'Z'
        || [4, 7, 10, 13, 16, 19].map(|i| time[i]) != *b"--T::."
    {
        return None;
    }
    let number = |start, end| value.get(start..end)?.parse::<u16>().ok();
    let date = [
        number(0, 4)?,
        number(5, 7)?,
        0,
        number(8, 10)?,
        number(11, 13)?,
        number(14, 16)?,
        number(17, 19)?,
        0,
    ];
    let mut ticks = 0;
    if unsafe { SystemTimeToFileTime(&date, &mut ticks) } == 0 {
        return None;
    }
    ticks.checked_add(value[20..27].parse().ok()?)
}
fn ticket(path: &Path, reply: &Status) -> Result<bool> {
    let directory = path.join("testlab-console");
    let mut budget = MAX_ENTRIES;
    let mut matched = 0;
    for entry in children(&directory, &mut budget)? {
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|s| s.strip_prefix("launch-"))
            .and_then(|s| s.strip_suffix(".json"))
            .filter(|s| hex_id(s))
        else {
            continue;
        };
        let item = document(&entry.path()).map_err(|_| "LegacyTicketUnreadable")?;
        if item["runId"] == reply.run_id {
            if item["schemaVersion"] != 1
                || item["state"] != "Accepted"
                || item["launchId"] != id
                || item["hostPid"] != reply.host_pid
                || item["hostStartUtc"] != reply.host_start_utc
                || item["mode"] != reply.mode
                || !item["id"]
                    .as_str()
                    .is_some_and(|s| same(Path::new(s), path))
            {
                return Ok(false);
            }
            matched += 1;
        }
    }
    Ok(matched == 1)
}

fn listeners(port: u16) -> Result<Vec<u32>> {
    let mut owners = Vec::new();
    for (family, stride, port_index) in [(2, 6, 2), (23, 14, 5)] {
        let mut size = 0;
        unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, family, 3, 0) };
        let mut done = false;
        for _ in 0..3 {
            if size > 16 * 1024 * 1024 {
                break;
            }
            let mut table = vec![0u32; (size as usize).div_ceil(4).max(1)];
            let code = unsafe {
                GetExtendedTcpTable(table.as_mut_ptr().cast(), &mut size, 0, family, 3, 0)
            };
            if code == 122 {
                continue;
            }
            if code != 0 {
                break;
            }
            let rows = table
                .get(1..1 + table[0] as usize * stride)
                .ok_or("ListenerUnconfirmed")?;
            for row in rows.chunks_exact(stride) {
                if u16::from_be(row[port_index] as u16) == port {
                    owners.push(row[stride - 1]);
                }
            }
            done = true;
            break;
        }
        if !done {
            return Err("ListenerUnconfirmed".into());
        }
    }
    owners.sort_unstable();
    owners.dedup();
    Ok(owners)
}

fn lock_is_held(path: &Path) -> Result<bool> {
    match checked_open(&path.join(".testlab-manager.lock"), false) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => Ok(true),
        Err(_) => Err("LockUnconfirmed".into()),
    }
}
fn launch_evidence(path: &Path) -> Result<(bool, Vec<Identity>)> {
    let mut active = Vec::new();
    let mut preparing = false;
    let directory = path.join("testlab-console");
    match checked_open(&directory, true) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((false, active)),
        Err(_) => return Err("LegacyDirectoryUnreadable".into()),
    }
    let mut budget = MAX_ENTRIES;
    for entry in children(&directory, &mut budget)? {
        let file = entry.file_name();
        if !file
            .to_str()
            .and_then(|n| n.strip_prefix("launch-"))
            .and_then(|n| n.strip_suffix(".json"))
            .is_some_and(hex_id)
        {
            continue;
        }
        let item = document(&entry.path()).map_err(|_| "LegacyTicketUnreadable")?;
        // Preparing has no process identity. A legacy launcher may yet be starting a child.
        if item["state"] == "Preparing" {
            preparing = true;
        }
        if item["state"] == "Accepted" {
            let pid = item["hostPid"]
                .as_u64()
                .and_then(|p| u32::try_from(p).ok())
                .filter(|p| *p > 0)
                .ok_or("HostUnconfirmed")?;
            let created = item["hostStartUtc"]
                .as_str()
                .and_then(timestamp)
                .ok_or("HostUnconfirmed")?;
            if let Some(who) = identity(pid)
                .map_err(|_| "HostUnconfirmed")?
                .filter(|who| who.created == created)
            {
                active.push(who);
            }
        }
    }
    Ok((preparing, active))
}
fn pipe_name(path: &Path) -> Result<String> {
    let path = path_format::display(path).replace('/', "\\");
    // .NET invariant uppercase never expands characters into multiple codepoints.
    // If Unicode versions disagree about case, avoid interpreting a missing pipe as stopped.
    if !path.is_ascii() {
        return Err("PipeNameUnsupported".into());
    }
    let hash = content_revision::revision(path.to_ascii_uppercase().as_bytes())?;
    Ok(format!(
        r"\\.\pipe\testlab-control-{}",
        hash[..32].to_ascii_uppercase()
    ))
}
fn observation(path: &Path, port: u16, host: Option<&Path>, deadline: Instant) -> Result<Value> {
    if Instant::now() >= deadline {
        return Err("PreviewDeadline".into());
    }
    let name = pipe_name(path)?;
    let locked = lock_is_held(path)?;
    let initial = listeners(port)?;
    let (preparing, evidence) = launch_evidence(path)?;
    if preparing {
        return Err("HostUnconfirmed".into());
    }
    if !locked && evidence.is_empty() {
        let pipe: Vec<u16> = std::ffi::OsStr::new(&name)
            .encode_wide()
            .chain([0])
            .collect();
        // Check absence without opening a connection. A legacy server can log
        // a broken pipe if a caller opens and disconnects without a status frame.
        let absent = || unsafe { WaitNamedPipeW(pipe.as_ptr(), 1) } == 0
            && io::Error::last_os_error().raw_os_error() == Some(2);
        if !absent() {
            return Err("HostUnconfirmed".into());
        }
        if lock_is_held(path)? || initial != listeners(port)? || !absent() {
            return Err("ObservationChanged".into());
        }
        return match initial.as_slice() {
            [] => Ok(json!({"state":"stopped","reason":"NoHostOrListener"})),
            [pid] if *pid > 0 => Ok(json!({"state":"external","reason":"IndependentListener",
                "pid":pid,"process":process_name(*pid)})),
            _ => Err("ListenerAmbiguous".into()),
        };
    }
    let executable = host.filter(|_| locked).ok_or("HostUnconfirmed")?;
    let [who] = evidence.as_slice() else {
        return Err("HostUnconfirmed".into());
    };
    if !process_image(who.pid).is_some_and(|image| same(&image, executable)) || !alive(who)? {
        return Err("HostUnconfirmed".into());
    }
    let (pid, body) = host_pipe::legacy_status(&name, |pid| Ok(pid == who.pid && alive(who)?))
        .map_err(|_| "HostUnconfirmed")?;
    let reply: Status =
        serde_json::from_str(body.strip_prefix("STATUS ").ok_or("HostUnconfirmed")?)
            .map_err(|_| "HostUnconfirmed")?;
    let child = identity(reply.child_pid)
        .map_err(|_| "HostUnconfirmed")?
        .ok_or("HostUnconfirmed")?;
    if !locked
        || !lock_is_held(path)?
        || !alive(who)?
        || !alive(&child)?
        || reply.schema_version != 1
        || !same(&reply.target_path, path)
        || reply.port != port
        || reply.host_pid != pid
        || reply.child_pid == pid
        || !hex_id(&reply.run_id)
        || !matches!(reply.mode.as_str(), "Detached" | "ConPty")
        || timestamp(&reply.host_start_utc) != Some(who.created)
        || timestamp(&reply.child_start_utc) != Some(child.created)
        || !ticket(path, &reply)?
    {
        return Err("HostUnconfirmed".into());
    }
    let owners = listeners(port)?;
    if (reply.ready && owners != [child.pid]) || (!owners.is_empty() && owners != [child.pid]) {
        return Err("ListenerMismatch".into());
    }
    Ok(
        json!({"state":"managed","reason":"VerifiedLegacyHost","hostPid":pid,
        "pid":child.pid,"runId":reply.run_id,"listening":owners == [child.pid]}),
    )
}

/// Evidence that V1 no longer runs the server in `path`: its manager lock is
/// free, no launch is preparing or held by a live V1 Host, its control pipe is
/// absent and nothing listens on the port. Used before a V1 handoff.
pub(super) fn v1_stopped(path: &Path, port: u16) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(6);
    match observation(path, port, None, deadline) {
        Ok(state) if state["state"] == "stopped" => Ok(()),
        Ok(state) if state["state"] == "external" => Err("V1ServerListening".into()),
        Ok(_) => Err("V1HostRunning".into()),
        Err(error) if error == "HostUnconfirmed" => Err("V1HostRunning".into()),
        Err(error) => Err(error),
    }
}

fn is_redirect(entry: &fs::DirEntry) -> Result<bool> {
    Ok(entry
        .metadata()
        .map_err(|_| "LegacyDirectoryUnreadable")?
        .file_attributes()
        & 0x400
        != 0)
}
fn unknown(reason: &str) -> Value {
    json!({"state":"unknown","reason":reason})
}

pub(super) fn inspect(args: &[String]) -> Result<Value> {
    let [root, policy, host] = args else {
        return Err("LegacyPreviewArguments".into());
    };
    let root = normalized_input(root)?;
    let instances = root.join("instances");
    checked_open(&instances, true).map_err(|_| "LegacyInstancesUnreadable")?;
    let policy_path = if policy.is_empty() {
        root.join("config").join("managed-scan.json")
    } else {
        normalized_input(policy)?
    };
    let policy = Policy::read(&policy_path, &instances)?;
    let host = if host.is_empty() {
        None
    } else {
        let path = normalized_input(host)?;
        if !path
            .extension()
            .is_some_and(|n| n.eq_ignore_ascii_case("exe"))
        {
            return Err("LegacyHostExecutableRequired".into());
        }
        checked_open(&path, false).map_err(|_| "LegacyHostExecutableRequired")?;
        Some(path)
    };
    let mut budget = MAX_ENTRIES;
    let mut skipped = 0;
    let mut targets = Vec::new();
    let velocity = instances.join("Velocity");
    if policy.includes(&velocity) {
        targets.push((velocity, "Velocity", String::new(), "Velocity".into()));
    }
    for kind in ["Human", "Agent"] {
        let directory = instances.join(kind);
        if !policy.intersects(&directory) {
            continue;
        }
        for project in children(&directory, &mut budget)? {
            if !policy.intersects(&project.path()) {
                continue;
            }
            if is_redirect(&project)? {
                skipped += 1;
                continue;
            }
            if !project.file_type().map_err(fail)?.is_dir() {
                continue;
            }
            for entry in children(&project.path(), &mut budget)? {
                if !policy.includes(&entry.path()) {
                    continue;
                }
                if is_redirect(&entry)? {
                    skipped += 1;
                    continue;
                }
                if !entry.file_type().map_err(fail)?.is_dir() {
                    continue;
                }
                if targets.len() >= MAX_ROWS {
                    return Err("LegacyScanLimit".into());
                }
                targets.push((
                    entry.path(),
                    kind,
                    project.file_name().to_string_lossy().into_owned(),
                    entry.file_name().to_string_lossy().into_owned(),
                ));
            }
        }
    }
    let config: Vec<_> = targets
        .iter()
        .map(|(path, kind, project, name)| configuration(path, kind, project, name))
        .collect();
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut rows = Vec::new();
    for ((path, kind, project, name), conf) in targets.iter().zip(&config) {
        let jar_name = jar_name(&root, kind);
        let core = jar_name
            .as_ref()
            .and_then(|name| {
                let jar = path.join(name);
                let file = checked_open(&jar, false).ok()?;
                core_identity::read_open_identity(&jar, file).ok().flatten()
            })
            .map_or_else(
                || json!({"state":"unknown","source":jar_name}),
                |identity| json!({"state":"known","identity":identity,"source":jar_name}),
            );
        let (port, java, observed) = match conf {
            Ok((port, alias)) => {
                let java = java_preview(&root, path, alias.as_deref())
                    .unwrap_or_else(|_| json!({"source":"unknown"}));
                let duplicates = config
                    .iter()
                    .filter(|entry| matches!(entry, Ok((p, _)) if p == port))
                    .count()
                    > 1;
                let state = if duplicates {
                    unknown("DuplicatePort")
                } else {
                    observation(path, *port, host.as_deref(), deadline)
                        .unwrap_or_else(|reason| unknown(&reason))
                };
                (Some(*port), java, state)
            }
            Err(reason) => (None, json!({"source":"unknown"}), unknown(reason)),
        };
        rows.push(
            json!({"name":name,"project":project,"kind":kind,"path":path_format::display(path),
            "port":port,"core":core,"java":java,"observation":observed}),
        );
    }
    Ok(
        json!({"source":path_format::display(&root),"policy":path_format::display(&policy_path),
        "schemaVersion":1,"skippedLinks":skipped,"rows":rows}),
    )
}
