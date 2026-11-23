#[path = "console_viewer.rs"]
mod console_viewer;
#[path = "native_console.rs"]
mod native_console;
pub use console_viewer::ConsoleViewer;
pub use native_console::Request as ConsoleRequest;
#[path = "content_revision.rs"]
mod content_revision;
#[path = "deferred_settings.rs"]
mod deferred_settings;
#[path = "core_maintenance.rs"]
mod core_maintenance;
#[path = "compatibility_probe.rs"]
mod compatibility_probe;
#[path = "prepare.rs"]
mod prepare;
#[path = "creation.rs"]
mod creation;
pub use creation::CreationPhase;
#[path = "prepare_diagnostics.rs"]
mod prepare_diagnostics;
#[path = "instance_diagnostics.rs"]
mod instance_diagnostics;
#[path = "batch_import.rs"]
mod batch_import;
#[path = "plugin_descriptor.rs"]
mod plugin_descriptor;
#[path = "agent_profiles.rs"]
mod agent_profiles;
#[path = "agent_inventory.rs"]
mod agent_inventory;
#[path = "test_permissions.rs"]
mod test_permissions;
#[path = "core_batch.rs"]
mod core_batch;
#[path = "auto_restart.rs"]
mod auto_restart;
#[path = "core_source.rs"]
mod core_source;
#[path = "core_prefetch.rs"]
mod core_prefetch;
#[path = "agent_rebuild.rs"]
mod agent_rebuild;
#[path = "core_identity.rs"]
mod core_identity;
#[path = "core_artifact.rs"]
mod core_artifact;
#[path = "gui_singleton.rs"]
pub mod gui_singleton;
#[path = "host_pipe.rs"]
mod host_pipe;
#[path = "host_recovery.rs"]
mod host_recovery;
#[path = "legacy_preview.rs"]
mod legacy_preview;
#[path = "v1_handoff.rs"]
mod v1_handoff;
#[path = "path_format.rs"]
mod path_format;
#[path = "policy_settings.rs"]
mod policy_settings;
#[path = "instance_snapshot.rs"]
mod instance_snapshot;
#[path = "presentation.rs"]
mod presentation;
#[path = "recovery_keys.rs"]
mod recovery_keys;
#[path = "resident_manager.rs"]
mod resident_manager;
#[path = "scan_policy.rs"]
mod scan_policy;
#[path = "global_java.rs"]
mod global_java;
#[path = "stop_retry.rs"]
mod stop_retry;
#[path = "world_live.rs"]
mod world_live;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{Ipv4Addr, TcpListener},
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, String>;
const FORCE_EXIT: i32 = 0x4948;
const HIDDEN: u32 = 0x08000000;
/// Written by `init`. Change the number only for an incompatible workspace layout.
const SCHEMA: &str = "instanceharbor-workspace-1";
/// Workspaces created before 1.0 (rc2 and development builds) keep this value. They
/// open unchanged, so an older build can still open them.
const PREVIOUS_SCHEMAS: &[&str] = &["instanceharbor-v2-windows-preview-1"];

fn supported_schema(value: &str) -> bool {
    value == SCHEMA || PREVIOUS_SCHEMAS.contains(&value)
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Jvm {
    min_mib: Option<u32>,
    max_mib: Option<u32>,
    gc: Option<GarbageCollector>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    advanced: Vec<String>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum GarbageCollector {
    G1,
    Parallel,
    Serial,
}

impl Jvm {
    fn arguments(&self) -> Result<Vec<String>> {
        let min = self.min_mib.unwrap_or(256);
        let max = self.max_mib.unwrap_or(1024);
        if !(64..=1_048_576).contains(&min) || !(min..=1_048_576).contains(&max) {
            return Err("InvalidJvmMemory".into());
        }
        let mut args = vec![format!("-Xms{min}m"), format!("-Xmx{max}m")];
        if let Some(gc) = self.gc {
            args.push(match gc {
                GarbageCollector::G1 => "-XX:+UseG1GC",
                GarbageCollector::Parallel => "-XX:+UseParallelGC",
                GarbageCollector::Serial => "-XX:+UseSerialGC",
            }.to_owned());
        }
        if self.advanced.len() > 16 {
            return Err("InvalidJvmOption".into());
        }
        let mut seen = std::collections::HashSet::new();
        for option in &self.advanced {
            let (key, value) = option.strip_prefix("-XX:")
                .and_then(|value| value.split_once('='))
                .ok_or("InvalidJvmOption")?;
            let valid = match key {
                "ActiveProcessorCount" | "ParallelGCThreads" | "ConcGCThreads" =>
                    value.parse::<u16>().is_ok_and(|n| (1..=256).contains(&n)),
                "MaxGCPauseMillis" => value.parse::<u32>().is_ok_and(|n| (1..=60_000).contains(&n)),
                "InitiatingHeapOccupancyPercent" => value.parse::<u8>().is_ok_and(|n| n <= 100),
                "G1ReservePercent" => value.parse::<u8>().is_ok_and(|n| (0..=50).contains(&n)),
                _ => false,
            };
            if !valid || !value.bytes().all(|byte| byte.is_ascii_digit()) || !seen.insert(key) {
                return Err("InvalidJvmOption".into());
            }
            args.push(option.clone());
        }
        Ok(args)
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LifecycleTiming {
    stop_seconds: Option<u64>,
    ready_seconds: Option<u64>,
    restart_delay_ms: Option<u64>,
}

impl LifecycleTiming {
    fn validate(&self) -> Result<()> {
        if self.stop_seconds.is_some_and(|value| !(5..=300).contains(&value))
            || self.ready_seconds.is_some_and(|value| !(10..=600).contains(&value))
            || self.restart_delay_ms.is_some_and(|value| value > 5000) {
            return Err("InvalidLifecycleTiming".into());
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreProfile {
    family: String,
    minecraft: String,
    build: String,
    sha256: String,
}

fn verified_core_profile(jar: &Path) -> Result<Option<CoreProfile>> {
    Ok(pinned_core_profile(content_revision::file_revision(jar)?))
}

fn pinned_core_profile(sha256: String) -> Option<CoreProfile> {
    let (family, minecraft, build) = match sha256.as_str() {
        "338be77f5239c44cff3f80f5c107b5e61ac48fb39348bce7249303209201072a" => ("PAPER", "1.14", "17"),
        "bd8ec5cdb22370d37816a6de26798df3d2b0d6f9c7c96c88ca45a1303fea50e8" => ("PAPER", "1.14.4", "245"),
        "4b011f5adb5f6c72007686a223174fce82f31aeb4b34faf4652abc840b47e640" => ("PAPER", "1.20.6", "151"),
        "8de7c52c3b02403503d16fac58003f1efef7dd7a0256786843927fa92ee57f1e" => ("PAPER", "1.21.8", "60"),
        "1d70b1dab9cf4a6de615209a536f3a45a2186240253c428213ce2188ab95e5f7" => ("PAPER", "26.1.2", "74"),
        "a0a58f532f08054ea92eb916d3edb6ed3c86429d77175317304f6e995cce9968" => ("PAPER", "26.3", "205"),
        "bd3a58cf96874e5ea6643f5f6fe9b4f5bf9e34b795fa078c2f0ee8b98b2f907e" => ("PAPER", "26.2", "112"),
        "3cce0a4922baf79bd8e1275ee5ff9f073f24675770e09b0d4be9442d513e7a6b" => ("PAPER", "26.2", "134"),
        "607afd1c3320008e1ffd2eaee6780ace4419d5f8c527b75e79f259be79ebf57b" => ("FOLIA", "26.1.2", "8"),
        "6ed6046687ce9255228fe7bd31e097da49beff2a0c577e7be56d14f09753239c" => ("SPIGOT", "1.8.8", "582b"),
        "ca59a730f5b1c2e85a2abe907d9689cc2636be0200f333c1e1b87e2d620d8513" => ("SPIGOT", "1.12.2", "79a30d7"),
        "53633ffc9253a5e0c57e08f26ab37e5505edd76a6bb55ff71e18197d559d1f62" => ("SPIGOT", "1.14", "c8d4cef"),
        "c738dcb9ac8114228107bb4cda97bfec62a243c99fe0e7c6caf9d424bd1ef93c" => ("SPIGOT", "1.16.5", "3096a"),
        "27a348344d42ed2ba76aad6fceebb7a26365f09c9ac6eea4c0918ba83ba7e3dd" => ("SPIGOT", "26.2", "4648"),
        "db6be7e3ffe9114e9a5d0645fbea398bf4591276603718ffeedf4d2d8eac1aaa" => ("VELOCITY", "4.2.1-SNAPSHOT", "38"),
        "e42ffc9929c6aef74c78cd80b42494e0271c08561e238c17bd890662afe8d28d" => ("PAPER", "26.3", "184"),
        "f649771a2190214306fb0cfba22c7b2cbac5df6308b1158309593603ed07d488" => ("PAPER", "26.3", "222"),
        _ => return None,
    };
    Some(CoreProfile { family: family.into(), minecraft: minecraft.into(), build: build.into(), sha256 })
}

impl CoreProfile {
    fn mojang_sha256(&self) -> Option<&'static str> {
        match (self.family.as_str(), self.minecraft.as_str()) {
            ("PAPER", "1.14") => Some("671e3d334dd601c520bf1aeb96e49038145172bef16bc6c418e969fd8bf8ff6c"),
            ("PAPER", "1.14.4") => Some("5ecdedab3a6e129321a444490d0a467c25ea702a24a99cebe3b6aed41f8f5729"),
            ("PAPER", "1.20.6") => Some("c6d01d018ca782e506f0ec60652d47fd565078be9122b625c1681bc86c29c7ec"),
            ("PAPER", "1.21.8") => Some("2349d9a8f0d4be2c40e7692890ef46a4b07015e7955b075460d02793be7fbbe7"),
            ("PAPER", "26.1.2") => Some("cd47e7c38328f64768fd17af8fcd8b22496b40b63d4ffee81e71ae059fedcb42"),
            ("PAPER", "26.3") => Some("d052f14d7a173734fba553711e5b570162e2f2a313267ee31a21b975a679be64"),
            ("PAPER", "26.2") => Some("cdacdfb25898de5e4b4b0e5ddcc2722f77067e46605709c2d886c000ebb63ec5"),
            ("FOLIA", "26.1.2") => Some("cd47e7c38328f64768fd17af8fcd8b22496b40b63d4ffee81e71ae059fedcb42"),
            _ => None,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AutoRestart {
    on_crash: bool,
    on_unexpected_exit: bool,
    delay_seconds: u64,
    max_attempts: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retry_delays_seconds: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    window_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stable_seconds: Option<u64>,
}

impl AutoRestart {
    fn enabled(&self) -> bool {
        self.on_crash || self.on_unexpected_exit
    }

    fn advanced(&self) -> bool {
        self.retry_delays_seconds.is_some() || self.window_seconds.is_some() || self.stable_seconds.is_some()
    }

    fn valid(&self) -> bool {
        let max_attempts = if self.advanced() { 10 } else { 5 };
        (2..=600).contains(&self.delay_seconds)
            && (1..=max_attempts).contains(&self.max_attempts)
            && self.retry_delays_seconds.as_ref().is_none_or(|delays| {
                delays.len() == usize::from(self.max_attempts)
                    && delays.iter().all(|delay| (1..=300).contains(delay))
            })
            && self.window_seconds.is_none_or(|window| (60..=3600).contains(&window))
            && self.stable_seconds.is_none_or(|stable| (60..=3600).contains(&stable))
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Instance {
    name: String,
    #[serde(with = "path_format::stored_java")]
    java: PathBuf,
    port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    velocity_listen_address: Option<Ipv4Addr>,
    #[serde(default)]
    removed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    jvm: Option<Jvm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timing: Option<LifecycleTiming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_restart: Option<AutoRestart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    core_profile: Option<CoreProfile>,
    /// The core JAR's file name inside the instance folder; absent means `server.jar`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    core_file: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    core_local_create: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    management_discovered: bool,
}

impl Instance {
    fn can_inherit_java(&self) -> bool {
        self.core_profile.is_some() || self.management_discovered
    }
}

fn is_false(value: &bool) -> bool { !value }

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Identity {
    pid: u32,
    created: u64,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Run {
    id: String,
    host: Identity,
    java: Identity,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BatchStopIntent {
    request: String,
    deadline: u64,
}

#[derive(Serialize, Deserialize)]
struct BatchStopCompletion {
    run: Run,
    intent: BatchStopIntent,
    finished_at: u64,
}

fn epoch_ms() -> u64 {
    (clock() / 1_000_000) as u64
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartGuard {
    previous_run: Option<String>,
    stop_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    registration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_identity: Option<Run>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_attempt: Option<u8>,
    /// The user confirmed, for this start only, that a console window may be used.
    #[serde(default, skip_serializing_if = "is_false")]
    console_window: bool,
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    arguments: Vec<String>,
    complete: bool,
    #[serde(default)]
    output: Option<Value>,
    #[serde(default, rename = "createdAt")]
    created_at: Option<u64>,
}

#[repr(C)]
struct StartupInfo {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    geometry: [u32; 8],
    show: u16,
    reserved_size: u16,
    reserved_bytes: *mut u8,
    handles: [*mut c_void; 3],
}

#[repr(C)]
struct ProcessInfo {
    process: *mut c_void,
    thread: *mut c_void,
    pid: u32,
    thread_id: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn CreateProcessW(
        application: *const u16,
        command: *mut u16,
        process_attributes: *mut c_void,
        thread_attributes: *mut c_void,
        inherit: i32,
        flags: u32,
        environment: *mut c_void,
        directory: *const u16,
        startup: *mut StartupInfo,
        output: *mut ProcessInfo,
    ) -> i32;
    fn GetProcessTimes(
        handle: *mut c_void,
        created: *mut u64,
        exited: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
    fn TerminateProcess(handle: *mut c_void, exit_code: u32) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn QueryFullProcessImageNameW(
        handle: *mut c_void,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
}
#[link(name = "iphlpapi")]
unsafe extern "system" {
    fn GetExtendedTcpTable(
        table: *mut c_void,
        size: *mut u32,
        order: i32,
        family: u32,
        class: u32,
        reserved: u32,
    ) -> u32;
}

fn fail(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn file_error(stage: &str, path: &Path, error: io::Error) -> String {
    format!("FileAccess[{stage}] path={}: {error}", path.display())
}

fn clock() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("invalid system clock")
        .as_nanos()
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && !matches!(
            value,
            "con"
                | "prn"
                | "aux"
                | "nul"
                | "com1"
                | "com2"
                | "com3"
                | "com4"
                | "com5"
                | "com6"
                | "com7"
                | "com8"
                | "com9"
                | "lpt1"
                | "lpt2"
                | "lpt3"
                | "lpt4"
                | "lpt5"
                | "lpt6"
                | "lpt7"
                | "lpt8"
                | "lpt9"
        )
}

fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(fail)?;
    let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), clock()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| file_error("create-temp", &tmp, e))?;
    file.write_all(&bytes)
        .map_err(|e| file_error("write-temp", &tmp, e))?;
    file.sync_all()
        .map_err(|e| file_error("sync-temp", &tmp, e))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|e| file_error("replace", path, e))
}

fn publish_json(
    path: &Path,
    bytes: &[u8],
    replace: bool,
    verify: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), clock()));
    let mut file = File::create_new(&tmp).map_err(|error| file_error("create-temp", &tmp, error))?;
    let staged = file.write_all(bytes).map_err(|error| file_error("write-temp", &tmp, error))
        .and_then(|()| file.sync_all().map_err(|error| file_error("sync-temp", &tmp, error)));
    drop(file);
    let result = staged.and_then(|()| verify())
        .and_then(|()| core_maintenance::move_durable_with_stage(&tmp, path, replace, "replace"));
    if let Err(error) = result {
        if let Err(cleanup) = fs::remove_file(&tmp) {
            return Err(format!("{error}; {}", file_error("remove-temp", &tmp, cleanup)));
        }
        return Err(error);
    }
    Ok(())
}

fn retry_shared_file<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for delay in [20, 50, 100] {
        match operation() {
            Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => {
                thread::sleep(Duration::from_millis(delay))
            }
            result => return result,
        }
    }
    operation()
}

// Graceful and emergency work share request IDs, even while waiting independently.
fn reserve_receipt(root: &Path, path: &Path, receipt: &Receipt) -> Result<()> {
    let _lock = receipt_lock(root)?;
    if path.exists() {
        return Err("RequestConflict".into());
    }
    save(path, receipt)
}

fn receipt_lock(root: &Path) -> Result<File> {
    let lock_path = root.join("receipts.lock");
    if lock_path.exists() {
        no_redirect(&lock_path)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| file_error("open-receipt-lock", &lock_path, e))?;
    lock.lock().map_err(fail)?;
    Ok(lock)
}

/// Removes a receipt reserved by `reserve_receipt` when its work failed before
/// it changed anything. Keeps the original error; adds the removal error to it.
fn release_receipt_on_error<T>(root: &Path, path: &Path, result: Result<T>) -> Result<T> {
    let Err(error) = result else { return result };
    let released = receipt_lock(root).and_then(|_lock| {
        no_redirect(path)?;
        let receipt: Receipt = read(path)?;
        if receipt.complete {
            return Err("ReceiptAlreadyComplete".into());
        }
        fs::remove_file(path).map_err(|e| file_error("remove-receipt", path, e))
    });
    match released {
        Ok(()) => Err(error),
        Err(cleanup) => Err(format!("{error}; {cleanup}")),
    }
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(|error| file_error("read", path, error))?;
    serde_json::from_slice(&bytes).map_err(fail)
}

fn safe_directory(path: &Path) -> Result<PathBuf> {
    let absolute = fs::canonicalize(path).map_err(fail)?;
    if !absolute.is_dir() {
        return Err("ExpectedDirectory".into());
    }
    Ok(absolute)
}

fn child_directory(parent: &Path, name: &str) -> Result<PathBuf> {
    let expected = parent.join(name);
    let resolved = safe_directory(&expected)?;
    if resolved != expected {
        return Err("DirectoryRedirected".into());
    }
    Ok(resolved)
}

fn no_redirect(path: &Path) -> Result<()> {
    let resolved = fs::canonicalize(path).map_err(fail)?;
    if resolved != path {
        return Err("PathRedirected".into());
    }
    Ok(())
}

fn identity(pid: u32) -> io::Result<Option<Identity>> {
    let handle = unsafe { OpenProcess(0x1000, 0, pid) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(87) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    let (mut created, mut exited, mut kernel, mut user) = (0, 0, 0, 0);
    let ok = unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) };
    let error = io::Error::last_os_error();
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return Err(error);
    }
    Ok((exited == 0).then_some(Identity { pid, created }))
}

fn alive(process: &Identity) -> Result<bool> {
    Ok(identity(process.pid).map_err(fail)?.as_ref() == Some(process))
}

fn process_image(pid: u32) -> Option<PathBuf> {
    let handle = unsafe { OpenProcess(0x1000, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut name = vec![0u16; 32768];
    let mut size = name.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, name.as_mut_ptr(), &mut size) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return None;
    }
    let path = String::from_utf16(&name[..size as usize]).ok()?;
    Some(PathBuf::from(path))
}

fn process_name(pid: u32) -> Option<String> {
    process_image(pid)?.file_name()?.to_str().map(str::to_owned)
}
fn ipv4_owner(address: Ipv4Addr, port: u16) -> Result<Option<u32>> {
    let mut size = 0;
    unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, 2, 3, 0) };
    for _ in 0..3 {
        let mut table = vec![0u32; (size as usize).div_ceil(4).max(1)];
        let code = unsafe { GetExtendedTcpTable(table.as_mut_ptr().cast(), &mut size, 0, 2, 3, 0) };
        if code == 122 {
            continue;
        }
        if code != 0 {
            return Err(format!("PortOwnerUnconfirmed ({code})"));
        }
        let count = table[0] as usize;
        let rows = table.get(1..1 + count * 6).ok_or("InvalidTcpTable")?;
        return Ok(rows.chunks_exact(6).find_map(|row| {
            let actual = Ipv4Addr::from(row[1].to_ne_bytes());
            (u16::from_be(row[2] as u16) == port
                && (actual == address || actual.is_unspecified() || address.is_unspecified()))
            .then_some(row[5])
        }));
    }
    Err("PortOwnerUnconfirmed".into())
}

fn port_free(port: u16) -> Result<()> {
    port_free_at(Ipv4Addr::LOCALHOST, port)
}

fn instance_port_free(config: &Instance) -> Result<()> {
    port_free_at(instance_listen_address(config), config.port)
}

fn port_free_at(address: Ipv4Addr, port: u16) -> Result<()> {
    if let Some(owner) = ipv4_owner(address, port)? {
        return Err(format!("PortOccupied PID={owner}"));
    }
    TcpListener::bind((address, port))
        .map(|_| ())
        .map_err(|_| "PortOccupiedOrUnavailable".into())
}

fn java(path: &str) -> Result<PathBuf> {
    let path = java_file(path)?;
    probe_java(&path, &[])?;
    Ok(path)
}

fn launch_java(root: &Path, config: &Instance, directory: &Path) -> Result<PathBuf> {
    if !config.java.as_os_str().is_empty() {
        registered_java_present(&config.java)?;
        return java(config.java.to_str().ok_or("InvalidJavaPath")?);
    }
    if !config.can_inherit_java() {
        return Err("InvalidJavaExecutable".into());
    }
    let jar = core_artifact::jar(directory, config);
    no_redirect(&jar)?;
    let profile = core_maintenance::trusted_profile(config, content_revision::file_revision(&jar)?)
        .ok_or("AutoJavaUnavailable")?;
    global_java::resolve(root, &profile)
}

/// A registered Java can disappear after registration, for example when its
/// install folder is moved. Report that with a stable code instead of the raw
/// OS message, so the GUI can send the user to the instance's Java settings.
fn registered_java_present(path: &Path) -> Result<()> {
    match fs::metadata(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err("JavaExecutableMissing".into())
        }
        Err(error) => Err(format!("JavaExecutableUnreadable: {error}")),
    }
}

fn chosen_run_java(folder: &Path) -> Result<PathBuf> {
    let path = folder.join("launch-java.json");
    no_redirect(&path)?;
    let executable: PathBuf = read(&path)?;
    java_file(executable.to_str().ok_or("InvalidJavaPath")?)
}

fn java_file(path: &str) -> Result<PathBuf> {
    let shim = |path: &Path| {
        path.components().any(|part| {
            part.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("shims")
        })
    };
    if shim(Path::new(path)) {
        return Err("JavaShimNotAllowed".into());
    }
    let path = fs::canonicalize(path).map_err(fail)?;
    if shim(&path) {
        return Err("JavaShimNotAllowed".into());
    }
    if !path.is_file()
        || !path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("java.exe"))
    {
        return Err("InvalidJavaExecutable".into());
    }
    Ok(path)
}

fn probe_java(path: &Path, additional_arguments: &[String]) -> Result<()> {
    let mut probe = Command::new(path)
        .args(additional_arguments)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(HIDDEN)
        .spawn()
        .map_err(fail)?;
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = probe.try_wait().map_err(fail)? {
            return if status.success() {
                Ok(())
            } else {
                Err("JavaProbeFailed".into())
            };
        }
        if Instant::now() >= until {
            probe.kill().map_err(fail)?;
            probe.wait().map_err(fail)?;
            return Err("JavaProbeTimedOut".into());
        }
        thread::sleep(Duration::from_millis(30));
    }
}

fn record_path(root: &Path, name: &str) -> PathBuf {
    root.join("registry").join(format!("{name}.json"))
}

fn registration_revision(root: &Path, name: &str) -> Result<String> {
    use std::os::windows::fs::MetadataExt;
    let path = record_path(root, name);
    no_redirect(&path)?;
    let mut file = File::open(path).map_err(fail)?;
    let metadata = file.metadata().map_err(fail)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    if bytes.len() > 64 * 1024 {
        return Err("InvalidInstanceRecord".into());
    }
    bytes.extend_from_slice(&metadata.creation_time().to_le_bytes());
    bytes.extend_from_slice(&metadata.last_write_time().to_le_bytes());
    content_revision::revision(&bytes)
}

fn port_unassigned(root: &Path, port: u16, name: &str) -> Result<()> {
    for entry in fs::read_dir(root.join("registry")).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let other = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or("InvalidInstanceName")?;
        if !token(other) {
            return Err("InvalidInstanceName".into());
        }
        let config = instance(root, other)?;
        if !config.removed && other != name && config.port == port {
            return Err("PortAlreadyRegistered".into());
        }
    }
    Ok(())
}

fn instance(root: &Path, name: &str) -> Result<Instance> {
    let record = record_path(root, name);
    no_redirect(&record)?;
    let bytes = fs::read(&record).map_err(|error| file_error("read", &record, error))?;
    decode_instance(&record, name, &bytes)
}

fn decode_instance(record: &Path, name: &str, bytes: &[u8]) -> Result<Instance> {
    let value: Instance = serde_json::from_slice(bytes).map_err(|error| {
        format!("InvalidInstanceRecord: {}: {error}", record.display())
    })?;
    if value.name != name
        || value.port == 0
        || value.core_file.as_deref().is_some_and(|file| !core_artifact::valid_name(file))
    {
        return Err("InvalidInstanceRecord".into());
    }
    Ok(value)
}

fn instance_path(root: &Path, name: &str) -> Result<PathBuf> {
    let instance = instance(root, name)?;
    managed_directory(root, name, instance.directory.as_deref())
}

fn managed_directory(root: &Path, name: &str, directory: Option<&str>) -> Result<PathBuf> {
    child_directory(root, "instances")?;
    let target = scan_policy::registered_path(root, name, directory)?;
    let resolved = safe_directory(&target)?;
    if resolved != target {
        return Err("DirectoryRedirected".into());
    }
    Ok(resolved)
}

fn property_port(path: &Path) -> Result<u16> {
    let properties = path.join("server.properties");
    no_redirect(&properties)?;
    let text = fs::read_to_string(properties).map_err(fail)?;
    let mut port = None;
    let mut ip = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("server-ip=") {
            ip = Some(value);
        }
        if let Some(value) = line.strip_prefix("server-port=") {
            port = value.parse::<u16>().ok();
        }
    }
    if ip != Some("127.0.0.1") {
        return Err("ExpectedLoopbackServerAddress".into());
    }
    port.filter(|p| *p != 0).ok_or("InvalidServerPort".into())
}

fn velocity_endpoint(text: &str) -> Result<(Ipv4Addr, u16)> {
    let mut endpoint = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or("InvalidVelocityBind")?;
        let value = value.trim();
        // Velocity's generated global settings are single-line. Refuse a
        // multiline value where text resembling a top-level key could appear.
        if value.starts_with("\"\"\"") || value.starts_with("'''") {
            return Err("InvalidVelocityBind".into());
        }
        if !matches!(key.trim(), "bind" | "\"bind\"" | "'bind'") {
            continue;
        }
        let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))
            .ok_or("InvalidVelocityBind")?;
        let (address, tail) = value[1..].split_once(quote).ok_or("InvalidVelocityBind")?;
        if endpoint.is_some() || !(tail.trim().is_empty() || tail.trim_start().starts_with('#')) {
            return Err("InvalidVelocityBind".into());
        }
        let (host, value) = address.split_once(':').ok_or("InvalidVelocityBind")?;
        let host = valid_velocity_address(host)?;
        if !value.bytes().all(|digit| digit.is_ascii_digit()) {
            return Err("InvalidVelocityBind".into());
        }
        let value = value.parse::<u16>().ok().filter(|value| *value != 0)
            .ok_or("InvalidVelocityBind")?;
        endpoint = Some((host, value));
    }
    endpoint.ok_or_else(|| "InvalidVelocityBind".into())
}

fn valid_velocity_address(raw: &str) -> Result<Ipv4Addr> {
    let address = raw.parse::<Ipv4Addr>().map_err(|_| "InvalidVelocityBind")?;
    // 0.0.0.0 opens the proxy on every interface. Registration needs it confirmed
    // explicitly, like any other non-loopback address.
    if address.is_multicast() || address.is_broadcast() {
        return Err("VelocityRequiresSpecificAddress".into());
    }
    Ok(address)
}

fn confirmed_velocity_address(configured: Ipv4Addr, expected: Option<&str>) -> Result<Ipv4Addr> {
    match expected {
        Some(raw) if valid_velocity_address(raw)? == configured => Ok(configured),
        Some(_) => Err("VelocityListenAddressChanged".into()),
        None if configured == Ipv4Addr::LOCALHOST => Ok(configured),
        None => Err("VelocityListenAddressConfirmationRequired".into()),
    }
}

fn velocity_binding(path: &Path) -> Result<((Ipv4Addr, u16), String)> {
    let file = path.join("velocity.toml");
    no_redirect(&file)?;
    let text = fs::read_to_string(file).map_err(fail)?;
    Ok((velocity_endpoint(&text)?, content_revision::revision(text.as_bytes())?))
}

fn registered_velocity(config: &Instance) -> Option<&CoreProfile> {
    config.core_profile.as_ref().filter(|profile| profile.family == "VELOCITY")
}

fn instance_listen_address(config: &Instance) -> Ipv4Addr {
    registered_velocity(config)
        .and(config.velocity_listen_address)
        .unwrap_or(Ipv4Addr::LOCALHOST)
}

fn validate_velocity_launch(path: &Path, profile: &CoreProfile, config: &Instance) -> Result<()> {
    let jar = core_artifact::jar(path, config);
    no_redirect(&jar)?;
    if content_revision::file_revision(&jar)? != profile.sha256 {
        return Err("CoreArtifactChanged".into());
    }
    if velocity_binding(path)?.0 != (instance_listen_address(config), config.port) {
        return Err("VelocityListenAddressChanged".into());
    }
    Ok(())
}

fn admitted(root: &Path, name: &str) -> Result<bool> {
    scan_policy::admitted(root, name)
}

// Excluding a folder must not remove control of a process already started here.
// Use the persisted run identity; a foreign listener alone grants no access.
fn stopping_outside_scope(root: &Path, config: &Instance) -> Result<bool> {
    if config.removed {
        return Ok(false);
    }
    let folder = root.join("runs").join(&config.name);
    let path = folder.join("identity.json");
    if !path.is_file() {
        return Ok(false);
    }
    no_redirect(&folder)?;
    no_redirect(&path)?;
    let run: Run = read(&path)?;
    Ok(alive(&run.host)? && alive(&run.java)?)
}

struct ImportSource {
    target: PathBuf,
    port: u16,
    retained: Option<Instance>,
    explicit_directory: bool,
    velocity: Option<CoreProfile>,
    core_file: String,
    address: Option<Ipv4Addr>,
    revision: Option<String>,
    _lease: std::os::windows::io::OwnedHandle,
}

fn registration_path_available(root: &Path, name: &str, target: &Path) -> Result<()> {
    for entry in fs::read_dir(root.join("registry")).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let other = path.file_stem().and_then(|stem| stem.to_str()).ok_or("InvalidInstanceName")?;
        if !token(other) {
            return Err("InvalidInstanceName".into());
        }
        if other != name {
            let record = instance(root, other)?;
            let location = scan_policy::registered_path(root, other, record.directory.as_deref())?;
            if scan_policy::contains(&location, target) || scan_policy::contains(target, &location) {
                return Err("ImportPathAlreadyRegistered".into());
            }
        }
    }
    Ok(())
}

fn checked_import(
    root: &Path,
    name: &str,
    requested: Option<&str>,
    listen_address: Option<&str>,
) -> Result<ImportSource> {
    checked_import_inner(root, name, requested, listen_address, None)
}

fn checked_import_with_policy(
    root: &Path,
    name: &str,
    requested: Option<&str>,
    listen_address: Option<&str>,
    policy: &scan_policy::Policy,
) -> Result<ImportSource> {
    checked_import_inner(root, name, requested, listen_address, Some(policy))
}

fn checked_import_inner(
    root: &Path,
    name: &str,
    requested: Option<&str>,
    listen_address: Option<&str>,
    policy: Option<&scan_policy::Policy>,
) -> Result<ImportSource> {
    if !token(name) {
        return Err("InvalidInstanceName".into());
    }
    let existing = if record_path(root, name).exists() {
        let existing = instance(root, name)?;
        if !existing.removed {
            return Err("AlreadyManaged".into());
        }
        if status(root, &existing)?["state"] != "Stopped" {
            return Err("InstanceMustBeStopped".into());
        }
        Some(existing)
    } else {
        None
    };
    let directory = requested.or_else(|| existing.as_ref().and_then(|item| item.directory.as_deref()));
    let target = managed_directory(root, name, directory)?;
    if let Some(existing) = &existing {
        if target != managed_directory(root, name, existing.directory.as_deref())? {
            return Err("ImportRegistrationPathChanged".into());
        }
    }
    registration_path_available(root, name, &target)?;
    let admitted = if let Some(policy) = policy {
        policy.matches(root, name, &target)?.1
    } else {
        scan_policy::admitted_path(root, name, &target)?
    };
    if !admitted {
        return Err("InstanceExcludedOrInvalid".into());
    }
    let proxy_config = target.join("velocity.toml");
    let proxy = match fs::symlink_metadata(&proxy_config) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(fail(error)),
    };
    let core_file = core_artifact::locate(&target, proxy)?.ok_or("MissingCoreArtifact")?;
    let core = target.join(&core_file);
    no_redirect(&core)?;
    if !core.is_file() {
        return Err("MissingCoreArtifact".into());
    }
    core_artifact::assert_unused(&core)?;
    let velocity = if proxy {
        Some(core_artifact::identify(&core)?
            .filter(|profile| profile.family == "VELOCITY")
            .ok_or("UnverifiedVelocityCore")?)
    } else {
        None
    };
    if existing.as_ref().and_then(registered_velocity).is_some() && velocity.is_none() {
        return Err("InvalidVelocityBind".into());
    }
    let (address, port, revision) = if let Some(profile) = &velocity {
        let ((configured, port), binding) = velocity_binding(&target)?;
        let confirmed = confirmed_velocity_address(configured, listen_address)?;
        (Some(confirmed), port, Some(content_revision::revision(format!("{}:{binding}", profile.sha256).as_bytes())?))
    } else {
        if listen_address.is_some() {
            return Err("VelocityListenAddressNotApplicable".into());
        }
        (None, property_port(&target)?, None)
    };
    port_free_at(address.unwrap_or(Ipv4Addr::LOCALHOST), port)?;
    port_unassigned(root, port, name)?;
    let lease = claim_runtime_directory(&target)?;
    scan_policy::verify_import_idle(&target)?;
    let explicit_directory = directory.is_some();
    Ok(ImportSource { target, port, retained: existing, explicit_directory, velocity, core_file, address, revision, _lease: lease })
}

fn provision(root: &Path, arguments: &[String]) -> Result<Value> {
    let created_at = Some((clock() / 1_000_000) as u64);
    let [verb, name, request, input, extra @ ..] = arguments else {
        return Err("Expected action, name, request ID and Java path".into());
    };
    if !token(name) || !token(request) {
        return Err("InstanceExcludedOrInvalid".into());
    }
    let mut prepared = if verb == "prepare" {
        if !extra.is_empty() {
            return Err("InvalidPrepareRequest".into());
        }
        Some(prepare::Request::parse(input)?)
    } else {
        None
    };
    let replacement = prepared.as_ref().filter(|plan| plan.replace);
    let work = root.join("jobs").join(format!("{request}.json"));
    if work.exists() {
        no_redirect(&work)?;
        let existing: Receipt = read(&work)?;
        if existing.arguments != arguments {
            return Err("RequestConflict".into());
        }
        if let Some(spec) = replacement {
            if agent_rebuild::pending(root, name)? {
                return agent_rebuild::finish(root, name, request, spec, existing);
            }
            if existing.complete {
                let mut result = existing.output.ok_or("ResultUnconfirmed")?;
                result["replayed"] = json!(true);
                return Ok(result);
            }
        }
        if let Some(spec) = prepared.as_ref().filter(|spec| spec.profile.is_some()) {
            return agent_profiles::replay(root, name, request, spec, &work, existing);
        }
        if !existing.complete {
            return Err("ResultUnconfirmed".into());
        }
        return Ok(json!({"workId":request,"replayed":true}));
    }
    agent_rebuild::assert_idle(root, name)?;
    if let Some(spec) = replacement {
        agent_rebuild::preflight(root, name, spec)?;
    }
    if let Some(spec) = prepared.as_mut().filter(|spec| spec.profile.is_some()) {
        agent_profiles::resolve(root, name, spec)?;
    }
    let replacement = prepared.as_ref().filter(|plan| plan.replace);
    let java = java(prepared.as_ref().map_or(input, |request| &request.java))?;
    let import = match (verb.as_str(), extra) {
        ("import", [directory, expected_port, ..]) if extra.len() <= 4 => {
            let directory = Some(directory.as_str()).filter(|value| !value.is_empty());
            let listen_address = extra.get(3).map(String::as_str);
            let info = checked_import(root, name, directory, listen_address)?;
            if expected_port.parse::<u16>().ok() != Some(info.port)
                || extra.get(2) != info.revision.as_ref()
            {
                return Err("ImportPreviewChanged".into());
            }
            Some(info)
        }
        ("import", [directory]) => Some(checked_import(root, name, Some(directory), None)?),
        ("import", []) => Some(checked_import(root, name, None, None)?),
        ("create" | "prepare", _) => None,
        _ => return Err("InvalidProvisionArguments".into()),
    };
    if import.is_none() && replacement.is_none() && record_path(root, name).exists() {
        return Err("AlreadyManaged".into());
    }
    let retained_jvm = import.as_ref()
        .and_then(|source| source.retained.as_ref())
        .and_then(|previous| previous.jvm.clone());
    let retained_timing = import.as_ref()
        .and_then(|source| source.retained.as_ref())
        .and_then(|previous| previous.timing.clone());
    let target = match (&import, &prepared) {
        (Some(source), _) => source.target.clone(),
        (None, Some(request)) => request.target(root),
        _ => root.join("instances").join(name),
    };
    if prepared.is_some() {
        registration_path_available(root, name, &target)?;
    }
    if import.is_none() && !scan_policy::admitted_path(root, name, &target)? {
        return Err("InstanceExcludedOrInvalid".into());
    }
    let create_args;
    let extra = if let Some(request) = &prepared {
        create_args = [request.port.to_string(), request.core_jar.clone(), request.mojang_jar.clone().unwrap_or_default()];
        &create_args[..]
    } else {
        extra
    };
    let (port, sources) = match (verb.as_str(), extra) {
        ("create" | "prepare", [port, core] | [port, core, _]) => {
            if replacement.is_none() && target.exists() {
                return Err("InstanceExists".into());
            }
            let port = port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or("InvalidServerPort")?;
            port_free(port)?;
            let core = if prepared.is_some() { prepare::local_jar(core)? } else { fs::canonicalize(core).map_err(fail)? };
            if !core.is_file() {
                return Err("MissingCoreArtifact".into());
            }
            let profile = verified_core_profile(&core)?.ok_or("UnverifiedCreationCore")?;
            if let Some(id) = prepared.as_ref().and_then(|request| request.profile.as_deref()) {
                agent_profiles::validate(id, &profile)?;
            }
            if profile.family == "VELOCITY" {
                return Err("UnsupportedCreationCore".into());
            }
            let bundled = extra.get(2).filter(|value| !value.is_empty());
            let mojang = match (profile.mojang_sha256(), bundled) {
                (Some(expected), Some(input)) => {
                    let input = if prepared.is_some() { prepare::local_jar(input)? } else { fs::canonicalize(input).map_err(fail)? };
                    if content_revision::file_revision(&input)? != expected {
                        return Err("MojangArtifactMismatch".into());
                    }
                    Some(input)
                }
                (Some(_), None) => return Err("MissingMojangArtifact".into()),
                (None, Some(_)) => return Err("UnexpectedMojangArtifact".into()),
                (None, None) => None,
            };
            (port, Some((core, mojang, profile)))
        }
        ("import", _) => (import.as_ref().ok_or("InvalidProvisionArguments")?.port, None),
        _ => return Err("InvalidProvisionArguments".into()),
    };
    port_unassigned(root, port, name)?;
    let prepared_assets = match (&prepared, &sources) {
        (Some(request), Some((_, _, profile))) => Some(request.verify(profile)?),
        _ => None,
    };
    reserve_receipt(
        root,
        &work,
        &Receipt {
            arguments: arguments.to_vec(),
            complete: false,
            output: None,
            created_at,
        },
    )?;
    let core_profile = if let Some((core, mojang, profile)) = sources {
        let stage = root.join("staging").join(request);
        fs::create_dir(&stage).map_err(fail)?;
        let staged_core = stage.join("server.jar");
        fs::copy(core, &staged_core).map_err(fail)?;
        if content_revision::file_revision(&staged_core)? != profile.sha256 {
            return Err("CoreArtifactChanged".into());
        }
        if let Some(mojang) = mojang {
            fs::create_dir(stage.join("cache")).map_err(fail)?;
            let output = stage.join("cache").join(format!("mojang_{}.jar", profile.minecraft));
            fs::copy(mojang, &output).map_err(fail)?;
            if Some(content_revision::file_revision(&output)?.as_str()) != profile.mojang_sha256() {
                return Err("MojangArtifactMismatch".into());
            }
        }
        fs::write(stage.join("eula.txt"), "eula=true\n").map_err(fail)?;
        if let Some((spec, assets)) = prepared.as_ref().zip(prepared_assets.as_ref()) {
            policy_settings::write_creation_settings_for(root, &stage, port, &profile, spec.human())?;
            spec.stage(&stage, name, &profile, assets)?;
            let _parent = spec.hold_parent(root)?;
            port_free(port)?;
            if !scan_policy::admitted_path(root, name, &target)? {
                return Err("InstanceExcludedOrInvalid".into());
            }
            if spec.replace {
                let mut next = agent_rebuild::preflight(root, name, spec)?;
                next.java = java;
                next.core_profile = Some(profile);
                agent_rebuild::stage(root, spec, request, &stage, next)?;
                return agent_rebuild::finish(root, name, request, spec, read(&work)?);
            }
            if let Some(profile_id) = spec.profile.as_deref() {
                return agent_profiles::publish(root, name, request, spec, &work, &stage, profile_id, java, profile);
            }
            core_maintenance::move_durable(&stage, &target, false)?;
        } else {
            policy_settings::write_creation_settings(root, &stage, port, &profile)?;
            fs::rename(stage, &target).map_err(fail)?;
        }
        Some(profile)
    } else {
        None
    };
    let core_local_create = core_profile.is_some();
    let core_profile = core_profile.or_else(|| import.as_ref().and_then(|source| source.velocity.clone()));
    save(
        &record_path(root, name),
        &Instance {
            name: name.clone(),
            java,
            port,
            velocity_listen_address: import.as_ref().and_then(|source| source.address).filter(|ip| *ip != Ipv4Addr::LOCALHOST),
            removed: false,
            jvm: retained_jvm,
            timing: retained_timing,
            auto_restart: import.as_ref().and_then(|source| source.retained.as_ref()).and_then(|item| item.auto_restart.clone()),
            core_local_create,
            core_file: None,
            core_profile,
            management_discovered: false,
            directory: (prepared.is_some() || import.is_some_and(|source| source.explicit_directory))
                .then(|| path_format::display(&target)),
        },
    )?;
    save(
        &work,
        &Receipt {
            arguments: arguments.to_vec(),
            complete: true,
            output: None,
            created_at,
        },
    )?;
    Ok(json!({"workId":request,"replayed":false}))
}

fn status(root: &Path, config: &Instance) -> Result<Value> {
    let folder = root.join("runs").join(&config.name);
    if folder.exists() {
        no_redirect(&folder)?;
    }
    let path = folder.join("identity.json");
    let owner = ipv4_owner(instance_listen_address(config), config.port)?;
    let (state, run) = if path.exists() {
        no_redirect(&path)?;
        let run: Run = read(&path)?;
        let host = alive(&run.host)?;
        let java = alive(&run.java)?;
        let state = match (host, java, owner) {
            (true, true, Some(pid)) if pid == run.java.pid => "Listening",
            (true, true, None) => "Starting",
            (_, false, Some(_)) | (true, true, Some(_)) => "PortOccupied",
            (false, false, None) => "Stopped",
            _ => "RunUnconfirmed",
        };
        (state, Some(run))
    } else {
        (
            if folder.exists() {
                "RunUnconfirmed"
            } else if owner.is_some() {
                "PortOccupied"
            } else {
                "Stopped"
            },
            None,
        )
    };
    let stop_phase = if folder.join("stop-sent").is_file() {
        Some("Sent")
    } else if folder.join("stop-queued").is_file() {
        Some("Queued")
    } else if folder.join(native_console::STOP_TYPED).is_file() {
        // The user typed a stop command in the console; the GUI shows "Stopping".
        Some("Typed")
    } else {
        None
    };
    let stop_token = cancellation_token(&folder)?;
    let auto_restart = config.auto_restart.as_ref()
        .map(|policy| auto_restart::state_for_status(root, &folder, policy, run.as_ref(), state == "Stopped"));
    // The tier this run chose and every tier it skipped, for the GUI to explain.
    let console = run.as_ref().and_then(|_| {
        let path = folder.join(CONSOLE_TIER);
        no_redirect(&path).ok().and_then(|()| read::<native_console::Choice>(&path).ok())
    });
    Ok(
        json!({"console":console,"name":config.name,"port":config.port,"listenAddress":registered_velocity(config).map(|_| instance_listen_address(config)),"java":config.java,"state":state,"ownerPid":owner,"ownerName":owner.and_then(process_name),"consoleReady":folder.join("console-ready").is_file(),"nativeConsole":folder.join("native-console").is_file(),"recoveryPinned":folder.join(host_recovery::PIN_FILE).is_file(),"stopPhase":stop_phase,"stopToken":stop_token,"run":run,"autoRestartStatus":auto_restart}),
    )
}

fn launch_detached(exe: &Path, root: &Path, arguments: &str) -> Result<u32> {
    use std::os::windows::ffi::OsStrExt;
    let app: Vec<_> = exe.as_os_str().encode_wide().chain([0]).collect();
    let exe = exe.to_str().ok_or("InvalidHostPath")?;
    let root = root.to_str().ok_or("InvalidWorkspacePath")?;
    if exe.contains('"') || root.contains('"') || root.ends_with('\\') {
        return Err("InvalidHostArgument".into());
    }
    let mut command: Vec<_> = format!("\"{exe}\" \"{root}\" {arguments}")
        .encode_utf16()
        .chain([0])
        .collect();
    // The detached process must not inherit redirected CLI streams or workspace locks.
    let mut startup: StartupInfo = unsafe { std::mem::zeroed() };
    startup.size = size_of::<StartupInfo>() as u32;
    let mut process: ProcessInfo = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            app.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            0x00000008 | 0x00000200,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut startup,
            &mut process,
        )
    };
    if ok == 0 {
        return Err(fail(io::Error::last_os_error()));
    }
    unsafe {
        CloseHandle(process.thread);
        CloseHandle(process.process);
    }
    Ok(process.pid)
}
fn launch_host(exe: &Path, root: &Path, name: &str, run: &str) -> Result<Identity> {
    identity(launch_detached(exe, root, &format!("host {name} {run}"))?)
        .map_err(fail)?
        .ok_or("HostStartupFailed".into())
}
/// Run-folder record of the console tier the Host chose.
const CONSOLE_TIER: &str = "console-tier.json";
/// Written by a user-confirmed start. Only then may the Host open a console window.
const CONSOLE_WINDOW_ALLOWED: &str = "console-window-allowed";

fn console_family(config: &Instance, jar: &Path) -> String {
    match &config.core_profile {
        Some(profile) => profile.family.clone(),
        None => core_identity::read_identity(jar)
            .ok()
            .flatten()
            .map(|identity| identity.family.to_owned())
            .unwrap_or_default(),
    }
}

/// Predicts the console tier of the next start, so the GUI can ask before a
/// start would need a console window. The Host repeats every check at start.
fn console_plan(root: &Path, config: &Instance) -> Result<Value> {
    let path = instance_path(root, &config.name)?;
    let jar = core_artifact::jar(&path, config);
    let paths = native_console::launch_paths(&jar, &path, &jar).and_then(|()| {
        if config.java.as_os_str().is_empty() {
            Ok(())
        } else {
            native_console::launch_paths(&config.java, &path, &jar)
        }
    });
    let choice = native_console::choose(&native_console::Facts {
        supported_core: native_console::supported_core(&console_family(config, &jar)),
        paths,
        bundled: native_console::bundled_runtime().map(|_| ()),
        system: native_console::system_runtime(),
        window: Ok(()),
        window_confirmed: true,
        limit: native_console::tier_limit(),
    });
    let mut value = serde_json::to_value(&choice).map_err(fail)?;
    value["needsConfirmation"] = json!(choice.tier == native_console::Tier::Window);
    Ok(value)
}

/// The checks a start passes before it changes any run state. A failure here
/// launched nothing, so the caller may release the request's receipt.
struct StartPlan {
    config: Instance,
    executable: PathBuf,
    yaml_preflight: Option<Value>,
    recovery_lock: File,
    recovery_pin: Option<host_recovery::Pin>,
}

fn plan_start(root: &Path, config: &Instance) -> Result<StartPlan> {
    creation::assert_idle(root, &config.name)?;
    agent_rebuild::assert_idle(root, &config.name)?;
    if !admitted(root, &config.name)? {
        return Err("InstanceExcludedOrInvalid".into());
    }
    // An unreadable registration can hide a conflicting port or directory.
    // Stops keep using the target's verified run and need no namespace scan.
    port_unassigned(root, config.port, &config.name).map_err(|error| {
        if error == "PortAlreadyRegistered" { error } else { format!("RegistryConflictUnconfirmed: {error}") }
    })?;
    // The caller holds instance-{name}.lock. Recover interrupted world commands
    // and commit durable file edits before a Host can rotate the old run.
    world_live::recover_saved_locked(root, &config.name)?;
    deferred_settings::apply_locked(root, &config.name)?;
    let refreshed = instance(root, &config.name)?;
    // Follow a core replaced under another file name before any check reads it.
    let refreshed = core_artifact::adopt(root, &instance_path(root, &config.name)?, refreshed)?;
    let config = &refreshed;
    if !admitted(root, &config.name)? {
        return Err("InstanceExcludedOrInvalid".into());
    }
    if status(root, config)?["state"] != "Stopped" {
        return Err("InstanceNotConfirmedStopped".into());
    }
    if config.auto_restart.as_ref().is_some_and(|policy| !policy.valid()) {
        return Err("InvalidAutoRestartPolicy".into());
    }
    let path = instance_path(root, &config.name)?;
    // The Host takes the same lease before starting Java.
    let runtime_lease = claim_runtime_directory(&path)?;
    core_maintenance::assert_start_allowed(&path)?;
    let proxy_profile = registered_velocity(config);
    if let Some(profile) = proxy_profile {
        validate_velocity_launch(&path, profile, config)?;
    } else if property_port(&path)? != config.port {
        return Err("PortChanged".into());
    }
    no_redirect(&core_artifact::jar(&path, config))?;
    instance_port_free(config)?;
    let executable = launch_java(root, config, &path)?;
    if let Some(jvm) = &config.jvm {
        probe_java(&executable, &jvm.arguments()?).map_err(|error| format!("JvmProbeFailed:{error}"))?;
    }
    let yaml_preflight = if proxy_profile.is_none() {
        policy_settings::yaml_preflight::check_start(root, &config.name)
    } else {
        None
    };
    drop(runtime_lease);
    let recovery_lock = command_lock(root, "recovery-key.lock", true)?;
    recovery_lock.try_lock_shared().map_err(|_| "ManagerBusy".to_owned())?;
    let recovery_pin = if let Some(key) = recovery_keys::current(root, &recovery_workspace_id(root)?)? {
        let manager = resident_manager::initial_host_manager(root)?;
        Some(host_recovery::Pin { key, manager })
    } else { None };
    Ok(StartPlan {
        config: refreshed,
        executable,
        yaml_preflight,
        recovery_lock,
        recovery_pin,
    })
}

fn launch_start(root: &Path, plan: StartPlan, attempt: u8, console_window: bool) -> Result<Value> {
    let StartPlan { config, executable, yaml_preflight, recovery_lock, recovery_pin } = plan;
    let config = &config;
    let run = format!("{}-{}", std::process::id(), clock());
    let folder = root.join("runs").join(&config.name);
    let budget = config.auto_restart.as_ref()
        .filter(|policy| attempt > 0 && policy.advanced())
        .map(|policy| auto_restart::inherit(&folder, policy, attempt, epoch_ms()))
        .transpose()?;
    if folder.exists() {
        fs::rename(&folder, root.join("history").join(&run)).map_err(fail)?;
    }
    fs::create_dir(&folder).map_err(fail)?;
    if config.java.as_os_str().is_empty() {
        save(&folder.join("launch-java.json"), &executable)?;
    }
    if let Some(pin) = recovery_pin {
        save(&folder.join(host_recovery::PIN_FILE), &pin)?;
    }
    drop(recovery_lock);
    if attempt > 0 {
        save(&folder.join("auto-restart-attempt.json"), &attempt)?;
    } else if console_window {
        mark_host_phase(&folder.join(CONSOLE_WINDOW_ALLOWED));
    }
    if let Some(publish) = budget {
        publish(&folder)?;
    }
    if let Some(report) = &yaml_preflight {
        policy_settings::yaml_preflight::store_run_report(&folder, &run, report)?;
    }
    let executable = std::env::current_exe().map_err(fail)?;
    let cli = executable.with_file_name("ihctl.exe");
    let host = launch_host(&cli, root, &config.name, &run)?;
    let until = Instant::now() + Duration::from_secs(8);
    while Instant::now() < until {
        if !alive(&host)? {
            return Err("HostStartupFailed".into());
        }
        if folder.join("identity.json").is_file() {
            let mut output = status(root, config)?;
            if let Some(report) = yaml_preflight {
                output["yamlPreflight"] = report;
            }
            return Ok(output);
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err("RunUnconfirmed".into())
}

fn stop(root: &Path, config: &Instance, expected: &str, force: bool) -> Result<Value> {
    let _transition = if force {
        Some(transition_guard(root, &config.name)?)
    } else {
        None
    };
    let folder = child_directory(&child_directory(root, "runs")?, &config.name)?;
    let identity_path = folder.join("identity.json");
    no_redirect(&identity_path)?;
    let run: Run = read(&identity_path)?;
    if run.id != expected {
        return Err("RunUnconfirmed".into());
    }
    let current = status(root, config)?;
    if current["run"]["id"].as_str() != Some(expected) {
        return Err("RunUnconfirmed".into());
    }
    if force
        && matches!(
            current["state"].as_str(),
            Some("Stopped" | "Listening" | "Starting")
        )
    {
        let cancelled = folder.join("restart-cancelled.json");
        if cancelled.exists() {
            no_redirect(&cancelled)?;
        }
        save(
            &cancelled,
            &json!({"run":expected,"intent":clock().to_string()}),
        )?;
    }
    if force && current["state"] == "Stopped" {
        return Ok(current);
    }
    if !matches!(current["state"].as_str(), Some("Listening" | "Starting")) {
        return Err("RunUnconfirmed".into());
    }
    if let Err(error) = host_pipe::request_stop(root, &config.name, &run, force) {
        if !force || !matches!(error.as_str(), "HostUnavailable" | "Timeout") {
            return Err(error);
        }
        // The Host can exit after accepting force stop but before replying.
        // Only the verified exit below can resolve a lost acknowledgement.
    }
    let wait = if force {
        Duration::from_secs(5)
    } else {
        Duration::from_millis(normal_stop_wait_ms(root, config))
    };
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if !alive(&run.java)? && !alive(&run.host)? {
            instance_port_free(config)?;
            let exit: Option<i32> = read(&folder.join("exit.json"))?;
            if force {
                let mut result = status(root, config)?;
                if result["run"]["id"].as_str() != Some(expected)
                    || !matches!(exit, Some(0 | FORCE_EXIT))
                {
                    return Err("StopResultUnconfirmed".into());
                }
                result["forced"] = json!(exit == Some(FORCE_EXIT));
                return Ok(result);
            }
            if exit != Some(0) {
                return Err("StopResultUnconfirmed".into());
            }
            // Out-of-scope runs finish their own normal save. The Manager
            // defers additional configuration edits until scope is restored.
            match admitted(root, &config.name) {
                Ok(true) => (),
                Ok(false) => return status(root, config),
                Err(error) => {
                    eprintln!("Deferred settings scope unavailable for {}: {error}", config.name);
                    return status(root, config);
                }
            }
            // A settings conflict must not turn a verified stop into a failed stop.
            // Keep the intent for the Manager sweep or next-start preflight.
            if let Err(error) = world_live::recover_saved_locked(root, &config.name) {
                eprintln!("Deferred world command remains pending for {}: {error}", config.name);
            }
            if let Err(error) = deferred_settings::apply_locked(root, &config.name) {
                eprintln!("Deferred settings remain pending for {}: {error}", config.name);
            } else if registered_velocity(config).is_none() {
                if let Err(error) = policy_settings::yaml_preflight::record_stop(root, &config.name) {
                    eprintln!("YAML readback could not be verified for {}: {error}", config.name);
                }
            }
            return status(root, config);
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("StopTimeout".into())
}

fn normal_stop_wait_ms(root: &Path, config: &Instance) -> u64 {
    // Isolated debug verifiers can exercise expiry without changing Host retries.
    if cfg!(debug_assertions)
        && root
            .ancestors()
            .skip(1)
            .take(2)
            .any(|path| path.file_name().is_some_and(|name| name == ".evidence"))
    {
        if let Some(ms) = std::env::var("IH_TEST_STOP_WAIT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|ms| (1..45_000).contains(ms))
        {
            return ms;
        }
    }
    config.timing.as_ref().and_then(|timing| timing.stop_seconds).unwrap_or(45) * 1000
}

fn instance_writer_lock(root: &Path, name: &str) -> Result<File> {
    command_lock(root, &format!("instance-{name}.lock"), false)
}

fn command_lock(root: &Path, name: &str, read_only: bool) -> Result<File> {
    let path = root.join(name);
    if path.exists() {
        no_redirect(&path)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|error| file_error("open-command-lock", &path, error))?;
    if !read_only {
        lock.try_lock().map_err(|_| "ManagerBusy".to_owned())?;
    }
    Ok(lock)
}

fn transition_guard(root: &Path, name: &str) -> Result<File> {
    let path = root.join(format!("transition-{name}.lock"));
    if path.exists() {
        no_redirect(&path)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(fail)?;
    lock.try_lock().map_err(|_| "ManagerBusy".to_owned())?;
    Ok(lock)
}

fn restart_permitted(folder: &Path, expected: &str) -> Result<()> {
    let cancelled = folder.join("restart-cancelled.json");
    if cancelled.exists() {
        no_redirect(&cancelled)?;
        let intent: Value = read(&cancelled)?;
        let run = intent
            .as_str()
            .or_else(|| intent["run"].as_str())
            .ok_or("RunUnconfirmed")?;
        return if run == expected {
            Err("RestartCancelledByEmergency".into())
        } else {
            Err("RunUnconfirmed".into())
        };
    }
    Ok(())
}

fn cancellation_token(folder: &Path) -> Result<Option<String>> {
    let emergency = folder.join("restart-cancelled.json");
    let normal = folder.join("newer-stop-intent.json");
    let mut tokens = Vec::new();
    for path in [emergency, normal] {
        if path.exists() {
            no_redirect(&path)?;
            tokens.push(content_revision::revision(&fs::read(path).map_err(fail)?)?);
        }
    }
    if tokens.len() <= 1 {
        return Ok(tokens.pop());
    }
    Ok(Some(content_revision::revision(
        tokens.join(":").as_bytes(),
    )?))
}

fn verify_start_guard(observed: &Value, guard: &StartGuard) -> Result<()> {
    if observed["state"] != "Stopped"
        || observed["run"]["id"].as_str() != guard.previous_run.as_deref()
        || observed["stopToken"].as_str() != guard.stop_token.as_deref()
        || guard.previous_identity.as_ref().is_some_and(|expected| {
            serde_json::from_value::<Run>(observed["run"].clone())
                .as_ref()
                .ok()
                != Some(expected)
        })
    {
        return Err("StartIntentChanged".into());
    }
    Ok(())
}

fn mark_host_phase(path: &Path) {
    if let Err(error) = File::create_new(path) {
        if error.kind() != io::ErrorKind::AlreadyExists {
            eprintln!("Could not record Host phase at {}: {error}", path.display());
        }
    }
}

fn verified_launch_jar(root: &Path, config: &Instance, local: &Path) -> Result<(PathBuf, Option<File>)> {
    use std::os::windows::fs::OpenOptionsExt;
    let local = core_artifact::jar(local, config);
    let _requested = match fs::symlink_metadata(root.join("shared-core.enabled")) {
        Ok(_) => { no_redirect(&root.join("shared-core.enabled"))?; true }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((local, None)),
        Err(error) => return Err(fail(error)),
    };
    let profile = core_maintenance::trusted_profile(config, content_revision::file_revision(&local)?)
        .ok_or("SharedCoreUnknown")?;
    if !matches!(profile.family.as_str(), "PAPER" | "FOLIA") {
        return Err("SharedCoreUnsupported".into());
    }
    let home = root.join("shared-core");
    if !home.is_dir() {
        match fs::create_dir(&home) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(fail(error)),
        }
    }
    let home = child_directory(root, "shared-core")?;
    let jar = home.join(format!("{}.jar", profile.sha256));
    if !jar.exists() {
        let _publisher = command_lock(root, "shared-core.lock", false)?;
        if !jar.exists() {
            let stage = home.join(format!("stage-{}-{}.tmp", std::process::id(), clock()));
            let copied = (|| {
                let mut out = File::create_new(&stage).map_err(fail)?;
                io::copy(&mut File::open(&local).map_err(fail)?, &mut out).map_err(fail)?;
                out.sync_all().map_err(fail)?;
                drop(out);
                if content_revision::file_revision(&stage)? != profile.sha256 {
                    return Err("CoreArtifactChanged".into());
                }
                fs::rename(&stage, &jar).map_err(fail)
            })();
            if copied.is_err() && stage.is_file() {
                fs::remove_file(stage).map_err(fail)?;
            }
            copied?;
        }
    }
    no_redirect(&jar)?;
    // Deny writes and removal while Java is reading this published revision.
    let guard = OpenOptions::new().read(true).share_mode(1).open(&jar).map_err(fail)?;
    if content_revision::file_revision(&jar)? != profile.sha256 {
        return Err("SharedCoreDigestMismatch".into());
    }
    if let Some(expected) = &config.core_profile {
        if expected.sha256 != profile.sha256 {
            return Err("CoreArtifactChanged".into());
        }
    }
    Ok((jar, Some(guard)))
}

fn claim_runtime_directory(path: &Path) -> Result<std::os::windows::io::OwnedHandle> {
    let identity = path.to_string_lossy().to_lowercase();
    let digest = content_revision::revision(identity.as_bytes())?;
    gui_singleton::acquire_named(&format!(r"Global\InstanceHarborData-{digest}"))
        .map_err(|error| file_error("claim-runtime-directory", path, error))?
        .ok_or_else(|| "InstanceDirectoryInUse".to_owned())
}

fn record_host_error(folder: &Path, stage: &str, error: &str) {
    let diagnostic = folder.join("host-diagnostic.json");
    if let Err(failure) = save(&diagnostic, &json!({"stage":stage,"error":crate::diagnostics::safe_error(error)})) {
        eprintln!("Host diagnostic unavailable: {failure}");
    }
}

fn host(root: &Path, config: &Instance, run_id: &str) -> Result<Value> {
    agent_rebuild::assert_idle(root, &config.name)?;
    let folder = child_directory(&child_directory(root, "runs")?, &config.name)?;
    let java = if config.java.as_os_str().is_empty() && config.can_inherit_java() {
        chosen_run_java(&folder)?
    } else {
        config.java.clone()
    };
    let path = instance_path(root, &config.name)?;
    let _claimed_directory = claim_runtime_directory(&path)?;
    core_maintenance::assert_start_allowed(&path)?;
    let velocity = registered_velocity(config);
    if let Some(profile) = velocity {
        validate_velocity_launch(&path, profile, config)?;
    }
    let (jar, _artifact_guard) = if velocity.is_some() {
        (core_artifact::jar(&path, config), None)
    } else {
        verified_launch_jar(root, config, &path)?
    };
    let stop_command = if velocity.is_some() { "shutdown" } else { "stop" };
    let mut first_plan = world_live::first_ready::pending(&path, config)?;
    let mut queued_world = match world_live::queued::has_queued(root, &config.name, run_id) {
        Ok(pending) => pending,
        Err(error) => {
            record_host_error(&folder, "queued-world", &error);
            false
        }
    };
    let this = std::env::current_exe().map_err(fail)?;
    let host_image = this.canonicalize().map_err(fail)?;
    let gui_image = this.with_file_name(crate::desktop_channel::GUI_EXECUTABLE).canonicalize().map_err(fail)?;
    let expected = [host_image, gui_image];
    let pin_path = folder.join(host_recovery::PIN_FILE);
    let recovery = match fs::symlink_metadata(&pin_path) {
        Ok(_) => {
            no_redirect(&pin_path)?;
            let pin: host_recovery::Pin = read(&pin_path)?;
            let installation = host_recovery::Installation::open(&expected[0])?;
            Some((pin, installation))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(fail(error)),
    };
    let args = config.jvm.clone().unwrap_or_default().arguments()?;
    let stdout = File::create_new(folder.join("stdout.log")).map_err(fail)?;
    let stderr = File::create_new(folder.join("stderr.log")).map_err(fail)?;
    let mut startup_log = File::open(folder.join("stdout.log")).map_err(fail)?;
    let mut facts = native_console::Facts {
        supported_core: native_console::supported_core(&console_family(config, &jar)),
        paths: native_console::launch_paths(&java, &path, &jar),
        bundled: native_console::bundled_runtime().map(|_| ()),
        system: native_console::system_runtime(),
        window: Ok(()),
        window_confirmed: folder.join(CONSOLE_WINDOW_ALLOWED).is_file(),
        limit: native_console::tier_limit(),
    };
    let title = presentation::console_window_title(root, &config.name);
    let launch = native_console::Launch {
        java: &java,
        directory: &path,
        jar: &jar,
        jvm: &args,
        nogui: velocity.is_none(),
        folder: &folder,
        title: &title,
    };
    let mut choice = native_console::choose(&facts);
    let mut console = None;
    while choice.tier != native_console::Tier::Plain {
        // A tier that fails here has not started Java, so the next tier may.
        let log = stdout.try_clone().map_err(fail)?;
        match native_console::Console::spawn(choice.tier, &launch, log) {
            Ok(started) => {
                console = Some(started);
                break;
            }
            Err(error) => {
                eprintln!("Console tier {:?} unavailable: {error}", choice.tier);
                match choice.tier {
                    native_console::Tier::Bundled => facts.bundled = Err(error),
                    native_console::Tier::System => facts.system = Err(error),
                    _ => facts.window = Err(error),
                }
                choice = native_console::choose(&facts);
            }
        }
    }
    save(&folder.join(CONSOLE_TIER), &choice)?;
    let mut child = if console.is_some() {
        None
    } else {
        // Legacy Java resolves the local server JAR through a file URI. Passing
        // a verbatim Windows absolute path can make that URI invalid for ZipFS.
        let java_jar = match jar.strip_prefix(&path) {
            Ok(name) if name.components().count() == 1 => name,
            _ => &jar,
        };
        let mut command = Command::new(&java);
        command.args(&args).arg("-jar").arg(java_jar);
        if velocity.is_none() {
            command.arg("nogui");
        }
        let child = command
            .current_dir(&path)
            .stdin(Stdio::piped())
            .stdout(stdout)
            .stderr(stderr)
            .creation_flags(HIDDEN)
            .spawn()
            .map_err(fail)?;
        Some(child)
    };
    let pid = child
        .as_ref()
        .map(std::process::Child::id)
        .or_else(|| console.as_ref().map(native_console::Console::pid))
        .ok_or("MissingJava")?;
    let run = Run {
        id: run_id.to_owned(),
        host: identity(std::process::id())
            .map_err(fail)?
            .ok_or("MissingHost")?,
        java: identity(pid).map_err(fail)?.ok_or("MissingJava")?,
    };
    save(&folder.join("identity.json"), &run)?;
    let recovery = recovery
        .map(|(pin, installation)| host_recovery::Gate::new(root, run.clone(), pin, installation))
        .transpose()?
        .map(|gate| std::sync::Arc::new(std::sync::Mutex::new(gate)));
    let host_gate = recovery.as_ref().map(|gate| (root.to_owned(), gate.clone()));
    let mut world_console = world_live::ConsoleWindow::default();
    let mut first_console = if first_plan.is_some() || queued_world {
        console.as_ref().map(native_console::Console::begin_managed_input).transpose()?
    } else {
        None
    };
    if let Some(console) = console.as_ref().filter(|console| console.tier() != native_console::Tier::Window) {
        match console.serve(&run, File::open(folder.join("stdout.log")).map_err(fail)?, recovery) {
            Ok(()) => mark_host_phase(&folder.join("native-console")),
            Err(error) => eprintln!("Native console is unavailable: {error}"),
        }
    }
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let (batch_send, batch_receive) = std::sync::mpsc::sync_channel::<BatchStopIntent>(1);
    let (force_send, force_receive) = std::sync::mpsc::sync_channel(1);
    let (world_send, world_receive) = std::sync::mpsc::sync_channel::<world_live::Command>(1);
    let (first_send, first_receive) = std::sync::mpsc::sync_channel::<world_live::Command>(1);
    let first_active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let endpoint_run = run.id.clone();
    let diagnostics = folder.join("control-error.txt");
    thread::spawn(move || {
        if let Err(error) =
            host_pipe::host_control(endpoint_run, expected, host_gate, send, force_send, batch_send, world_send)
        {
            if let Err(log_error) = fs::write(diagnostics, error.to_string()) {
                eprintln!("Host control failed: {error}; diagnostics unavailable: {log_error}");
            }
        }
    });
    let mut stop_retry = stop_retry::StopRetry::default();
    let mut batch_intent = None;
    let mut batch_stop_sent = false;
    let stop_clock = Instant::now();
    let mut ready = false;
    let mut ready_at = None;
    let mut tail = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut read_error_reported = false;
    let mut live_retry_clock = Instant::now();
    loop {
        let exited = match &mut child {
            Some(child) => child
                .try_wait()
                .map_err(fail)?
                .map(|status| status.code().unwrap_or(-1)),
            None => console.as_ref().ok_or("MissingJava")?.exit()?,
        };
        if let Some(code) = exited {
            let drained = match &mut console {
                Some(console) => console.close_and_drain(),
                None => ["stdout.log","stderr.log"].iter().try_for_each(|file| {
                    OpenOptions::new().write(true).open(folder.join(file)).and_then(|log| log.sync_all()).map_err(fail)
                }),
            };
            if let Err(error) = &drained {
                record_host_error(&folder, "drain", error);
            }
            let observed_at = epoch_ms();
            if let Some(policy) = config.auto_restart.as_ref()
                .filter(|policy| drained.is_ok() && policy.enabled() && policy.valid())
            {
                if let Err(error) = auto_restart::record_exit(
                    &folder, policy, &run, code, observed_at, ready_at, console.as_ref().map(native_console::Console::tier),
                    velocity.is_some(),
                ) {
                    record_host_error(&folder, "auto-restart-exit", &error);
                }
            }
            if code == 0 && drained.is_ok() {
                save(&folder.join("run-release.json"), &json!({"run":run,"exit":code}))?;
            }
            if code == 0 && batch_stop_sent {
                if let Some(intent) = batch_intent.take() {
                    if let Err(error) = save(
                        &folder.join("batch-stop-completion.json"),
                        &BatchStopCompletion {
                            run: run.clone(),
                            intent,
                            finished_at: epoch_ms(),
                        },
                    ) {
                        eprintln!("Could not record verified batch stop completion: {error}");
                    }
                }
            }
            save(&folder.join("exit.json"), &Some(code))?;
            save(&folder.join("exit-observed-at.json"), &observed_at)?;
            return Ok(json!({"exit": code}));
        }
        if force_receive.try_recv().is_ok() {
            // The retained child handle cannot be redirected by PID reuse.
            let handle = child
                .as_ref()
                .map(AsRawHandle::as_raw_handle)
                .or_else(|| console.as_ref().map(native_console::Console::handle))
                .ok_or("MissingJava")?;
            if unsafe { TerminateProcess(handle, FORCE_EXIT as u32) } != 0 {
                if let Some(child) = &mut child {
                    let status = child.wait().map_err(fail)?;
                    save(&folder.join("exit.json"), &status.code())?;
                    return Ok(json!({"exit": status.code()}));
                }
                // Wait for the ConPTY child handle to signal before recording exit.
                thread::sleep(Duration::from_millis(25));
                continue;
            }
            eprintln!(
                "Could not force stop the managed Java process: {}",
                io::Error::last_os_error()
            );
        }
        let abort_managed_input = || {
            if let Some(console) = &console {
                if let Err(error) = console.abort_managed_input() {
                    eprintln!("Could not cancel managed console input: {error}");
                }
            }
        };
        if let Ok(intent) = batch_receive.try_recv() {
            // Once the Host has recorded this request it owns graceful shutdown,
            // even if the Manager disconnects or reaches its observation deadline.
            if batch_intent.is_none()
                && epoch_ms() < intent.deadline
                && !folder.join("stop-sent").exists()
            {
                match save(&folder.join("batch-stop-accepted.json"), &intent) {
                    Ok(()) => {
                        abort_managed_input();
                        batch_intent = Some(intent);
                        stop_retry.request();
                        mark_host_phase(&folder.join("stop-queued"));
                    }
                    Err(error) => eprintln!("Could not record batch stop intent: {error}"),
                }
            }
        }
        if receive.try_recv().is_ok() {
            abort_managed_input();
            stop_retry.request();
            mark_host_phase(&folder.join("stop-queued"));
        }
        if !ready {
            let count = match startup_log.read(&mut chunk) {
                Ok(count) => {
                    read_error_reported = false;
                    count
                }
                Err(error) => {
                    if !read_error_reported {
                        eprintln!("Could not read Java startup log: {error}");
                        read_error_reported = true;
                    }
                    0
                }
            };
            tail.extend_from_slice(&chunk[..count]);
            ready = tail
                .split(|byte| matches!(byte, b'\r' | b'\n'))
                .any(|line| {
                    let text = String::from_utf8_lossy(line);
                    if velocity.is_some() {
                        return text.contains("Done (") && text.contains(")!");
                    }
                    let marker = if console.is_some() {
                        "Done ("
                    } else {
                        "INFO]: Done ("
                    };
                    text.contains(marker) && text.contains(")! For help")
                });
            if ready {
                ready_at = Some(epoch_ms());
                mark_host_phase(&folder.join("console-ready"));
            }
            // Preserve split markers while bounding the amount of console output held in memory.
            if tail.len() > 1024 {
                tail.drain(..tail.len() - 1024);
            }
        }
        if stop_retry.due(stop_clock.elapsed(), ready) {
            if let Some(console) = &console {
                if let Err(error) = console.stop(stop_command) {
                    eprintln!("Could not queue native console stop: {error}");
                    continue;
                }
            } else {
                child
                    .as_mut()
                    .and_then(|child| child.stdin.as_mut())
                    .ok_or("MissingJavaInput")?
                    .write_all(format!("{stop_command}\n").as_bytes())
                    .map_err(fail)?;
            }
            batch_stop_sent = batch_intent.is_some();
            mark_host_phase(&folder.join("stop-sent"));
        }
        let commands_ready = ready
            && !folder.join("stop-queued").exists()
            && !folder.join("stop-sent").exists();
        use std::sync::atomic::Ordering;
        // Retry live-first rules this run could not send yet, for example while the
        // user was typing in the console. The managed writer waits for an empty line.
        let mut live_retry = false;
        if commands_ready
            && first_plan.is_none()
            && !queued_world
            && !first_active.load(Ordering::Acquire)
            && live_retry_clock.elapsed() >= Duration::from_secs(2)
        {
            live_retry_clock = Instant::now();
            live_retry = world_live::queued::has_live_retry(root, &config.name, run_id).unwrap_or_else(|error| {
                record_host_error(&folder, "live-retry", &error);
                false
            });
            if live_retry {
                // Another managed writer, such as a world command window, keeps its turn.
                match console.as_ref().map(native_console::Console::begin_managed_input).transpose() {
                    Ok(managed) => first_console = managed,
                    Err(_) => live_retry = false,
                }
            }
        }
        if commands_ready && (first_plan.is_some() || queued_world || live_retry) {
                let plan = first_plan.take();
                let apply_queued = std::mem::take(&mut queued_world);
                first_active.store(true, Ordering::Release);
                let (root, path, folder) = (root.to_owned(), path.clone(), folder.clone());
                let (config, run) = (config.clone(), run.clone());
                let (send, active) = (first_send.clone(), first_active.clone());
                let managed = first_console.take();
                thread::spawn(move || {
                    let write = |line: &str, deadline| match managed.as_ref() {
                        Some(input) => input.write_command(line, deadline),
                        None => match world_live::submit(&send, line) {
                            "WorldCommandWritten" => Ok(()),
                            error => Err(error.into()),
                        },
                    };
                    if let Some(plan) = plan {
                        if let Err(error) =
                            world_live::first_ready::apply(&root, &path, &config, &run, plan, &write)
                        {
                            record_host_error(&folder, "first-ready", &error);
                        }
                    }
                    if apply_queued {
                        if let Err(error) = world_live::queued::apply_queued(&root, &config, &run, &write) {
                            record_host_error(&folder, "queued-world", &error);
                        }
                    }
                    if live_retry {
                        if let Err(error) = world_live::queued::apply_live_retry(&root, &config, &run, &write) {
                            // Busy console lines stay due and are retried on a later pass.
                            if error != "WorldConsoleInputBusy" && error != "ConsoleManagedInputBusy" {
                                record_host_error(&folder, "live-retry", &error);
                            }
                        }
                    }
                    drop(managed);
                    active.store(false, Ordering::Release);
                });
        }
        let active = first_active.load(Ordering::Acquire);
        world_live::deliver(
            &first_receive,
            commands_ready && active,
            child.as_mut().and_then(|child| child.stdin.as_mut()),
            None,
        );
        world_live::deliver(
            &world_receive,
            commands_ready && !active && first_plan.is_none(),
            child.as_mut().and_then(|child| child.stdin.as_mut()),
            console.as_ref().map(|console| (console, &mut world_console)),
        );
        thread::sleep(Duration::from_millis(75));
    }
}

fn lifecycle(
    root: &Path,
    config: &Instance,
    verb: &str,
    request: &str,
    run: Option<&str>,
    start_guard: Option<&StartGuard>,
) -> Result<Value> {
    let created_at = Some((clock() / 1_000_000) as u64);
    if !token(request) {
        return Err("InvalidRequestId".into());
    }
    let mut arguments = vec![
        verb.to_owned(),
        config.name.clone(),
        request.to_owned(),
        run.unwrap_or_default().to_owned(),
    ];
    if let Some(guard) = start_guard {
        arguments.push(serde_json::to_string(guard).map_err(fail)?);
    }
    let path = root.join("jobs").join(format!("{request}.json"));
    if path.exists() {
        no_redirect(&path)?;
        let prior: Receipt = read(&path)?;
        if prior.arguments != arguments {
            return Err("RequestConflict".into());
        }
        if !prior.complete {
            return Err("ResultUnconfirmed".into());
        }
        let mut output = prior.output.ok_or("MissingWorkResult")?;
        output["replayed"] = json!(true);
        return Ok(output);
    }
    // Hold this through publication of the new run. A force request committed
    // after the GUI snapshot invalidates even an already queued start IPC.
    let _guarded_transition = if start_guard.is_some() {
        Some(transition_guard(root, &config.name)?)
    } else {
        None
    };
    let before = status(root, config)?;
    if matches!(verb, "stop" | "restart") && run.is_some() && before["run"]["id"].as_str() == run {
        let _transition = transition_guard(root, &config.name)?;
        let path = root
            .join("runs")
            .join(&config.name)
            .join("newer-stop-intent.json");
        if path.exists() {
            no_redirect(&path)?;
        }
        save(&path, &json!({"run":run, "request":request}))?;
    }
    if let Some(guard) = start_guard {
        verify_start_guard(&before, guard)?;
        if let Some(expected) = &guard.registration {
            if registration_revision(root, &config.name)? != *expected {
                return Err("StartIntentChanged".into());
            }
        }
    }
    if run.is_none() && before["state"] != "Stopped" {
        return Err("InstanceNotConfirmedStopped".into());
    }
    if let Some(expected) = run {
        if before["run"]["id"].as_str() != Some(expected)
            || !(matches!(before["state"].as_str(), Some("Listening" | "Starting"))
                || (verb == "force-stop" && before["state"] == "Stopped"))
        {
            return Err("RunUnconfirmed".into());
        }
    }
    reserve_receipt(
        root,
        &path,
        &Receipt {
            arguments: arguments.clone(),
            complete: false,
            output: None,
            created_at,
        },
    )?;
    // A start that fails before launch changed no run state, so the same
    // request may be sent again instead of turning into ResultUnconfirmed.
    let preflight = |result| release_receipt_on_error(root, &path, result);
    let mut output = match (verb, run) {
        ("start-if-current", None) if start_guard.is_some() => {
            let attempt = start_guard.and_then(|guard| guard.auto_attempt).unwrap_or(0);
            let console_window = start_guard.is_some_and(|guard| guard.console_window);
            let plan = preflight(plan_start(root, config))?;
            launch_start(root, plan, attempt, console_window)?
        }
        ("start" | "restart", None) => {
            let _transition = release_receipt_on_error(root, &path, transition_guard(root, &config.name))?;
            let plan = preflight(plan_start(root, config))?;
            launch_start(root, plan, 0, false)?
        }
        ("stop", Some(run)) => stop(root, config, run, false)?,
        ("force-stop", Some(run)) => stop(root, config, run, true)?,
        ("restart", Some(run)) => {
            stop(root, config, run, false)?;
            let delay = config.timing.as_ref().and_then(|timing| timing.restart_delay_ms).unwrap_or(0);
            if delay > 0 {
                thread::sleep(Duration::from_millis(delay));
            }
            let _transition = transition_guard(root, &config.name)?;
            let current = status(root, config)?;
            if current["state"] != "Stopped" || current["run"]["id"].as_str() != Some(run) {
                return Err("RunUnconfirmed".into());
            }
            restart_permitted(
                &child_directory(&child_directory(root, "runs")?, &config.name)?,
                run,
            )?;
            // The stop already happened, so this receipt stays unconfirmed on failure.
            launch_start(root, plan_start(root, config)?, 0, false)?
        }
        _ => return Err("InvalidOperationOrState".into()),
    };
    output["workId"] = json!(request);
    output["replayed"] = json!(false);
    save(
        &path,
        &Receipt {
            arguments,
            complete: true,
            output: Some(output.clone()),
            created_at,
        },
    )?;
    Ok(output)
}

fn jobs(root: &Path) -> Result<Value> {
    let mut rows = batch_import::activity(root)?;
    for entry in fs::read_dir(root.join("jobs")).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        no_redirect(&path)?;
        let receipt: Receipt = read(&path)?;
        let [action, instance, ..] = receipt.arguments.as_slice() else {
            return Err("InvalidWorkReceipt".into());
        };
        if !admitted(root, instance)? {
            continue;
        }
        let mut row = json!({"id":path.file_stem().and_then(|name| name.to_str()), "action":action, "instance":instance,"createdAt":receipt.created_at,
            "state":if receipt.complete {"Completed"} else {"Unconfirmed"}});
        if let Some(report) = receipt.output.as_ref().and_then(|output| output.get("yamlPreflight")) {
            if policy_settings::yaml_preflight::has_changes(report) {
                row["message"] = json!("偵測到 YAML 外部變更，可到世界設定查看。");
            } else if report["state"] == "Unverified" {
                row["message"] = json!("YAML 啟動前差異尚未確認。");
            }
        }
        rows.push(row);
    }
    for entry in fs::read_dir(root.join("registry")).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let name = path.file_stem().and_then(|name| name.to_str()).ok_or("InvalidInstanceName")?;
        if token(name) && admitted(root, name)? && !instance(root, name)?.removed {
            rows.extend(core_maintenance::activity(root, name)?);
            rows.extend(world_live::activity(root, name)?);
            if !instance(root, name)?.removed {
                rows.extend(world_live::first_ready::activity(&instance_path(root, name)?, name)?);
            }
        }
    }
    rows.sort_by(|a, b| {
        b["createdAt"]
            .as_u64()
            .cmp(&a["createdAt"].as_u64())
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
    });
    Ok(json!(rows))
}
fn recovery_workspace_id(root: &Path) -> Result<String> {
    let hash = content_revision::revision(root.to_string_lossy().to_lowercase().as_bytes())?;
    Ok(hash[..32].to_owned())
}

fn ensure_recovery_rotation_safe(root: &Path) -> Result<()> {
    let runs = child_directory(root, "runs")?;
    for entry in fs::read_dir(&runs).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        if !entry.file_type().map_err(fail)?.is_dir() {
            continue;
        }
        let folder = entry.path();
        no_redirect(&folder)?;
        let pin = folder.join(host_recovery::PIN_FILE);
        match fs::symlink_metadata(&pin) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(fail(error)),
            Ok(_) => no_redirect(&pin)?,
        }
        let identity = folder.join("identity.json");
        no_redirect(&identity).map_err(|_| "RecoveryHostUnconfirmed")?;
        let run: Run = read(&identity).map_err(|_| "RecoveryHostUnconfirmed")?;
        if alive(&run.host)? {
            return Err("RecoveryHostActive".into());
        }
    }
    Ok(())
}

fn recovery_backup_path(input: &str) -> Result<PathBuf> {
    let target = Path::new(input);
    let name = target.file_name().and_then(|file| file.to_str()).ok_or("InvalidRecoveryBackupPath")?;
    let parent = target.parent().ok_or("InvalidRecoveryBackupPath")?;
    if !target.is_absolute() || target.extension().and_then(|ext| ext.to_str()) != Some("ihrecovery")
        || name.contains([':', '*', '?', '"', '<', '>', '|'])
    {
        return Err("InvalidRecoveryBackupPath".into());
    }
    let parent_path = safe_directory(parent)?;
    let lexical = path_format::display(parent);
    let canonical = path_format::display(&parent_path);
    if !lexical.trim_end_matches('/').eq_ignore_ascii_case(canonical.trim_end_matches('/')) {
        return Err("RecoveryBackupPathRedirected".into());
    }
    Ok(parent_path.join(name))
}

pub fn dispatch_recovery_backup(arguments: &[String], password: &[u8]) -> Result<Value> {
    let [workspace, verb, file, args @ ..] = arguments else {
        return Err("InvalidRecoveryArguments".into());
    };
    let root_path = Path::new(workspace);
    if !root_path.is_absolute() {
        return Err("ExpectedAbsoluteWorkspace".into());
    }
    crate::desktop_channel::validate_workspace(root_path, false)?;
    let root = safe_directory(root_path)?;
    no_redirect(&root.join("workspace.json"))?;
    if !supported_schema(&read::<String>(&root.join("workspace.json"))?) {
        return Err("UnsupportedWorkspace".into());
    }
    let id = recovery_workspace_id(&root)?;
    let path = recovery_backup_path(file)?;
    let generation = |value: &str| value.parse::<u64>().map_err(|_| "InvalidRecoveryGeneration".to_owned());
    if let ("recovery-backup-export", [expected]) = (verb.as_str(), args) {
        let _guard = command_lock(&root, "recovery-key.lock", false)?;
        let expected = generation(expected)?;
        let bytes = recovery_keys::backup(&root, &id, expected, password)?;
        let mut output = OpenOptions::new().write(true).create_new(true).open(&path)
            .map_err(|error| file_error("create-recovery-backup", &path, error))?;
        output.write_all(&bytes).and_then(|_| output.sync_all())
            .map_err(|error| file_error("write-recovery-backup", &path, error))?;
        return Ok(json!({"generation":expected,"backup":path_format::display(&path)}));
    }
    no_redirect(&path)?;
    let mut bytes = Vec::new();
    File::open(&path).map_err(fail)?
        .take(recovery_keys::PORTABLE_BACKUP_BYTES as u64 + 1)
        .read_to_end(&mut bytes).map_err(fail)?;
    if bytes.len() > recovery_keys::PORTABLE_BACKUP_BYTES {
        return Err("InvalidRecoveryBackupSize".into());
    }
    let key = match (verb.as_str(), args) {
        ("recovery-backup-inspect", []) => recovery_keys::inspect_backup(&id, &bytes, password)?,
        ("recovery-backup-restore", [expected, public]) => {
            let _guard = command_lock(&root, "recovery-key.lock", false)?;
            let key = recovery_keys::PublicKey {generation:generation(expected)?,public_key:public.clone()};
            recovery_keys::restore_backup(&root, &id, &key, &bytes, password)?
        }
        _ => return Err("InvalidRecoveryArguments".into()),
    };
    Ok(json!({"key":key,"hostTakeoverAvailable":true}))
}

pub fn dispatch(arguments: &[String]) -> Result<Value> {
    dispatch_with_creation_progress(arguments, &|_| {})
}

/// Whether closing the main window should keep the GUI in the notification area.
pub fn close_to_tray(workspace: &Path) -> Result<bool> {
    crate::desktop_channel::validate_workspace(workspace, false)?;
    presentation::close_to_tray_enabled(workspace)
}

pub fn dispatch_with_creation_progress(
    arguments: &[String],
    progress: &dyn Fn(CreationPhase),
) -> Result<Value> {
    let [workspace, verb, args @ ..] = arguments else {
        return Err("Usage: ihctl WORKSPACE ACTION ...".into());
    };
    let path = Path::new(workspace);
    if !path.is_absolute() {
        return Err("ExpectedAbsoluteWorkspace".into());
    }
    let initializing = verb == "init" && args.is_empty();
    crate::desktop_channel::validate_workspace(path, initializing)?;
    if initializing {
        fs::create_dir(path).map_err(fail)?;
        for entry in [
            "instances",
            "registry",
            "jobs",
            "runs",
            "history",
            "staging",
        ] {
            fs::create_dir(path.join(entry)).map_err(fail)?;
        }
        save(
            &path.join("scan-policy.json"),
            &scan_policy::Policy::default(),
        )?;
        save(&path.join("workspace.json"), &SCHEMA)?;
        return Ok(json!({"workspace":path}));
    }
    let root = safe_directory(path)?;
    no_redirect(&root.join("workspace.json"))?;
    if !supported_schema(&read::<String>(&root.join("workspace.json"))?) {
        return Err("UnsupportedWorkspace".into());
    }
    if let Some(result) = global_java::dispatch(&root, verb, args) {
        return result;
    }
    if let Some(result) = prepare_diagnostics::dispatch(&root, verb, args) {
        return result;
    }
    if verb == "instance-diagnostics" {
        return instance_diagnostics::inspect(&root, args);
    }
    if verb == "legacy-preview" {
        return legacy_preview::inspect(args);
    }
    if matches!(verb.as_str(), "preferences" | "preferences-snapshot" | "java-check") {
        return presentation::dispatch(&root, verb, args).ok_or("InvalidPresentationArguments")?;
    }
    for entry in [
        "instances",
        "registry",
        "jobs",
        "runs",
        "history",
        "staging",
    ] {
        child_directory(&root, entry)?;
    }
    if let Some(result) = batch_import::dispatch(&root, verb, args) {
        return result;
    }
    if let Some(result) = v1_handoff::dispatch_verb(&root, verb, args) {
        return result;
    }
    if let Some(result) = creation::dispatch(&root, verb, args, progress) {
        return result;
    }
    if matches!(verb.as_str(), "recovery-key-status" | "recovery-key-rotate") {
        let id = recovery_workspace_id(&root)?;
        let workspace = id.as_str();
        let key = match (verb.as_str(), args) {
            ("recovery-key-status", []) => recovery_keys::current(&root, workspace)?,
            ("recovery-key-rotate", [generation]) => {
                let _guard = command_lock(&root, "recovery-key.lock", false)?;
                ensure_recovery_rotation_safe(&root)?;
                let expected = if generation == "none" { None } else {
                    Some(generation.parse::<u64>().map_err(|_| "InvalidRecoveryGeneration")?)
                };
                Some(recovery_keys::rotate(&root, workspace, expected)?)
            }
            _ => return Err("InvalidRecoveryArguments".into()),
        };
        return Ok(json!({"key": key, "hostTakeoverAvailable": false}));
    }
    if verb == "compatibility-probe-check" {
        let [manifest] = args else { return Err("InvalidCompatibilityProbeManifest".into()); };
        return compatibility_probe::check(&root, manifest);
    }
    if verb == "core-state" {
        return core_maintenance::dispatch(&root, verb, args).ok_or("InvalidCoreArguments")?;
    }
    if verb == "core-install-latest" {
        let [name, request, current, candidate] = args else { return Err("InvalidCoreArguments".into()); };
        return core_source::install_latest(&root, name, request, current, candidate);
    }
    if verb == "core-catalog-versions" {
        let [family] = args else { return Err("InvalidCoreArguments".into()); };
        return core_source::creation_versions(family);
    }
    if verb == "core-catalog-build" {
        let [family, version] = args else { return Err("InvalidCoreArguments".into()); };
        let core = core_source::latest_creation_core(family, version)?;
        return serde_json::to_value(core).map_err(fail);
    }
    if verb == "core-check" {
        return core_source::check(&root, args);
    }
    if matches!(verb.as_str(), "core-batch-submit" | "core-batch-list" | "core-batch-status" | "core-batch-cancel") {
        return core_batch::client(&root, verb, args);
    }
    if verb == "create-download" {
        return core_source::create_download(&root, args);
    }
    if let Some(result) = test_permissions::dispatch(verb) {
        return result;
    }
    if let Some(result) = world_live::dispatch(&root, verb, args) {
        return result;
    }
    if matches!(verb.as_str(), "get-initialization" | "world-settings" | "world-batch-preview") {
        return policy_settings::dispatch(&root, verb, args).ok_or("InvalidSettingsArguments")?;
    }
    if let Some(result) = scan_policy::dispatch(&root, verb, args) {
        return result;
    }
    if verb == "import-preview" {
        let [name, java_path, directory, rest @ ..] = args else {
            return Err("InvalidProvisionArguments".into());
        };
        if rest.len() > 1 {
            return Err("InvalidProvisionArguments".into());
        }
        let source = checked_import(
            &root, name, Some(directory.as_str()).filter(|value| !value.is_empty()),
            rest.first().map(String::as_str).filter(|value| !value.is_empty()),
        )?;
        let java = java(java_path)?;
        return Ok(json!({
            "path":path_format::display(&source.target),
            "java":path_format::display(&java),
            "port":source.port,
            "listenAddress":source.address,
            "revision":source.revision,
            "retained":source.retained.is_some()
        }));
    }
    if verb == "host" {
        let [name, run] = args else {
            return Err("InvalidHostArguments".into());
        };
        if !token(name) || !token(run) {
            return Err("InvalidHostIdentity".into());
        }
        let result = host(&root, &instance(&root, name)?, run);
        if let Err(error) = &result {
            if let Ok(folder) = child_directory(&root.join("runs"), name) {
                record_host_error(&folder, "host", error);
            }
        }
        return result;
    }
    if verb == "manager-serve" && args.is_empty() {
        return resident_manager::run(&root);
    }
    if matches!(
        verb.as_str(),
        "batch-start"
            | "batch-stop"
            | "batch-restart"
            | "batch-list"
            | "batch-connect"
            | "manager-connect"
            | "agent-list"
            | "batch-cancel"
            | "manager-exit"
            | "console-open"
            | "host-recover"
    ) {
        return resident_manager::client(&root, verb, args);
    }
    match (verb.as_str(), args) {
        ("list", []) => return instance_snapshot::query(&root, false).map(|snapshot| snapshot["rows"].clone()),
        ("list", [option]) if option == "--details" => return instance_snapshot::query(&root, false),
        ("instance-snapshot", []) => return instance_snapshot::query(&root, true),
        _ => {}
    }
    // Configuration writes reserve the workspace-wide port namespace together.
    // Lifecycle waits never hold this lock, and readers never acquire it.
    let _port_lock = if matches!(
        verb.as_str(),
        "create" | "prepare" | "import" | "adopt"
    ) {
        let path = root.join("port-registration.lock");
        if path.exists() {
            no_redirect(&path)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(fail)?;
        lock.try_lock().map_err(|_| "ManagerBusy".to_owned())?;
        Some(lock)
    } else {
        None
    };
    // A pending graceful stop holds the regular lock while waiting. Force stop
    // only targets an existing Host and never waits for the regular operation lock.
    // Reads preserve the last committed snapshot while another instance stops.
    // Per-instance writers serialize with settings/provisioning for that name;
    // emergency requests retain their separate run-verified path.
    let read_only = matches!(
        verb.as_str(),
        "jobs"
            | "status"
            | "presentation"
            | "presentation-metadata"
            | "instance-settings"
            | "discover-java"
            | "logs"
            | "world-settings"
            | "get-initialization"
            | "console-plan"
            | "console-io"
    );
    let instance_writer = matches!(
        verb.as_str(),
        "create"
            | "prepare"
            | "import"
            | "start"
            | "start-if-current"
            | "stop"
            | "restart"
            | "detach"
            | "remove-management"
            | "adopt"
            | "save-instance-settings"
            | "save-world-settings"
            | "core-install"
            | "core-restore"
            | "core-reconcile"
    );
    let lock_name = if verb == "force-stop" {
        "force-stop.lock".to_owned()
    } else if instance_writer {
        let name = args
            .first()
            .filter(|name| token(name))
            .ok_or("InvalidInstanceName")?;
        format!("instance-{name}.lock")
    } else {
        "manager.lock".to_owned()
    };
    let _lock = if read_only { None } else { Some(command_lock(&root, &lock_name, false)?) };
    if instance_writer {
        if let Some(name) = args.first() {
            creation::assert_idle(&root, name)?;
        }
    }
    if instance_writer && verb != "prepare" {
        if let Some(name) = args.first() {
            agent_rebuild::assert_idle(&root, name)?;
        }
    }
    if matches!(verb.as_str(), "save-instance-settings" | "save-world-settings" | "remove-management" | "adopt") {
        if let Some(name) = args.first().filter(|name| token(name) && record_path(&root, name).exists()) {
            core_maintenance::assert_start_allowed(&instance_path(&root, name)?)?;
        }
    }
    if let Some(result) = core_maintenance::dispatch(&root, verb, args) {
        return result;
    }
    if let Some(result) = presentation::dispatch(&root, verb, args) {
        return result;
    }
    if let Some(result) = policy_settings::dispatch(&root, verb, args) {
        return result;
    }
    if matches!(verb.as_str(), "create" | "prepare" | "import") {
        return provision(&root, &arguments[1..]);
    }
    if verb == "jobs" && args.is_empty() {
        return jobs(&root);
    }
    let [name, rest @ ..] = args else {
        return Err("MissingInstanceName".into());
    };
    if !token(name) {
        return Err("InstanceExcludedOrInvalid".into());
    }
    let config = instance(&root, name)?;
    if !admitted(&root, name)? && !matches!(verb.as_str(), "stop" | "force-stop" | "status") {
        return Err("InstanceExcludedOrInvalid".into());
    }
    if config.removed {
        return Err("InstanceRemovedFromManagement".into());
    }
    if matches!(verb.as_str(), "start" | "restart") && config.auto_restart.as_ref().is_some_and(|policy| policy.on_crash || policy.on_unexpected_exit) {
        resident_manager::client(&root, "manager-connect", &[])?;
    }
    match (verb.as_str(), rest) {
        ("status", []) => status(&root, &config),
        ("console-plan", []) => console_plan(&root, &config),
        // Development builds only: drive a run's console pipe like the viewer does,
        // so typing can be verified without the GUI. `text` is a JSON string.
        ("console-io", [run, op, text]) if crate::desktop_channel::DEVELOPMENT => {
            let request = match op.as_str() {
                "input" => native_console::Request::Input {
                    bytes: serde_json::from_str::<String>(text).map_err(fail)?.into_bytes(),
                },
                "read" => native_console::Request::Read { offset: text.parse().map_err(fail)? },
                _ => return Err("InvalidConsoleRequest".into()),
            };
            native_console::call(&root, name, run, request)
        }
        ("start" | "restart", [request]) => lifecycle(&root, &config, verb, request, None, None),
        ("start-if-current", [request, input]) => {
            let guard: StartGuard = serde_json::from_str(input).map_err(fail)?;
            if guard.auto_attempt.is_some() {
                return Err("InvalidStartGuard".into());
            }
            lifecycle(&root, &config, verb, request, None, Some(&guard))
        }
        ("stop" | "restart" | "force-stop", [request, run]) => {
            lifecycle(&root, &config, verb, request, Some(run), None)
        }
        ("detach", []) if status(&root, &config)?["state"] == "Stopped" => {
            fs::remove_file(record_path(&root, name)).map_err(fail)?;
            Ok(json!({"detached":name}))
        }
        _ => Err("InvalidOperationOrState".into()),
    }
}

#[cfg(test)]
mod transition_tests {
    use super::*;

    #[test]
    fn velocity_import_requires_explicit_single_bound_address() {
        assert_eq!(velocity_endpoint("# proxy\r\nbind = \"127.0.0.1:16001\" # local\r\n[servers]\r\nbind = \"ignored\"").unwrap(), (Ipv4Addr::LOCALHOST,16001));
        assert_eq!(velocity_endpoint("'bind' = '127.0.0.1:16002'").unwrap(), (Ipv4Addr::LOCALHOST,16002));
        let address = velocity_endpoint("bind = \"192.168.56.2:16001\"").unwrap().0;
        assert!(confirmed_velocity_address(address, None).is_err());
        assert!(confirmed_velocity_address(address, Some("192.168.56.1")).is_err());
        assert_eq!(confirmed_velocity_address(address, Some("192.168.56.2")).unwrap(), address);
        assert!(velocity_endpoint("bind = \"127.0.0.1:0\"").is_err());
        assert!(velocity_endpoint("bind = \"127.0.0.1:+16001\"").is_err());
        assert!(velocity_endpoint("[servers]\nbind = \"127.0.0.1:16001\"").is_err());
        assert!(velocity_endpoint("bind = \"127.0.0.1:16001\"\n\"bind\" = \"127.0.0.1:16002\"").is_err());
        assert!(velocity_endpoint("motd = '''\nbind = \"127.0.0.1:16001\"\n'''").is_err());
    }

    #[test]
    fn a_proxy_bound_to_every_interface_is_accepted_after_explicit_confirmation() {
        let all = velocity_endpoint("bind = \"0.0.0.0:16001\"").unwrap();
        assert_eq!(all, (Ipv4Addr::UNSPECIFIED, 16001));
        assert_eq!(confirmed_velocity_address(all.0, None).unwrap_err(), "VelocityListenAddressConfirmationRequired");
        assert_eq!(confirmed_velocity_address(all.0, Some("127.0.0.1")).unwrap_err(), "VelocityListenAddressChanged");
        assert_eq!(confirmed_velocity_address(all.0, Some("0.0.0.0")).unwrap(), Ipv4Addr::UNSPECIFIED);
        assert!(velocity_endpoint("bind = \"255.255.255.255:16001\"").is_err());
        assert!(velocity_endpoint("bind = \"224.0.0.1:16001\"").is_err());
    }

    #[test]
    fn a_port_query_on_every_interface_finds_a_listener_on_any_address() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_eq!(ipv4_owner(Ipv4Addr::UNSPECIFIED, port).unwrap(), Some(std::process::id()));
    }

    #[test]
    fn jvm_defaults_and_memory_validation() {
        assert_eq!(Jvm::default().arguments().unwrap(), ["-Xms256m", "-Xmx1024m"]);
        let configured = Jvm {
            min_mib: Some(128),
            max_mib: Some(512),
            gc: Some(GarbageCollector::Parallel),
            advanced: vec![],
        };
        assert_eq!(configured.arguments().unwrap(), ["-Xms128m", "-Xmx512m", "-XX:+UseParallelGC"]);
        assert_eq!(Jvm { min_mib: Some(512), max_mib: Some(128), gc: None, advanced: vec![] }.arguments().unwrap_err(), "InvalidJvmMemory");
    }

    #[test]
    fn runtime_directory_lease_rejects_second_holder_and_releases() {
        let path = std::env::temp_dir().join(format!("ih-runtime-lease-{}-{}", std::process::id(), clock()));
        let first = claim_runtime_directory(&path).unwrap();
        assert_eq!(claim_runtime_directory(&path).unwrap_err(), "InstanceDirectoryInUse");
        drop(first);
        assert!(claim_runtime_directory(&path).is_ok());
    }

    #[test]
    fn guarded_start_rejects_force_received_after_the_snapshot() {
        let observed = json!({"state":"Stopped","run":{"id":"run-1"},"stopToken":"before"});
        let guard = StartGuard {
            previous_run: Some("run-1".into()),
            stop_token: Some("before".into()),
            registration: None,
            previous_identity: None,
            auto_attempt: None,
            console_window: false,
        };
        verify_start_guard(&observed, &guard).unwrap();
        let newer_force = json!({"state":"Stopped","run":{"id":"run-1"},"stopToken":"after"});
        assert_eq!(
            verify_start_guard(&newer_force, &guard).unwrap_err(),
            "StartIntentChanged"
        );
        let fresh_user_intent = StartGuard {
            previous_run: Some("run-1".into()),
            stop_token: Some("after".into()),
            registration: None,
            previous_identity: None,
            auto_attempt: None,
            console_window: false,
        };
        verify_start_guard(&newer_force, &fresh_user_intent).unwrap();
    }

    #[test]
    fn emergency_intent_blocks_restart_even_after_normal_exit() {
        let root =
            std::env::temp_dir().join(format!("ih-transition-{}-{}", std::process::id(), clock()));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let held = transition_guard(&root, "agent").unwrap();
        assert!(matches!(transition_guard(&root, "agent"), Err(error) if error == "ManagerBusy"));
        save(&root.join("restart-cancelled.json"), &"run-1").unwrap();
        assert_eq!(
            restart_permitted(&root, "run-1").unwrap_err(),
            "RestartCancelledByEmergency"
        );
        assert_eq!(
            restart_permitted(&root, "run-2").unwrap_err(),
            "RunUnconfirmed"
        );
        drop(held);
        drop(transition_guard(&root, "agent").unwrap());
        fs::remove_file(root.join("restart-cancelled.json")).unwrap();
        fs::remove_file(root.join("transition-agent.lock")).unwrap();
        fs::remove_dir(&root).unwrap();
    }
}

#[cfg(test)]
mod workspace_schema_tests {
    use super::*;

    #[test]
    fn new_workspaces_write_the_1_0_schema_and_earlier_workspaces_still_open() {
        let parent = std::env::temp_dir().join(format!("ih-schema-{}-{}", std::process::id(), clock()));
        fs::create_dir(&parent).unwrap();
        let workspace = parent.join("workspace");
        let path = workspace.to_string_lossy().into_owned();
        dispatch(&[path.clone(), "init".into()]).unwrap();
        let file = workspace.join("workspace.json");
        assert_eq!(read::<String>(&file).unwrap(), "instanceharbor-workspace-1");
        assert!(dispatch(&[path.clone(), "list".into()]).is_ok());

        // rc2 and development builds wrote the earlier value; it opens and stays unchanged.
        save(&file, &"instanceharbor-v2-windows-preview-1").unwrap();
        assert!(dispatch(&[path.clone(), "list".into()]).is_ok());
        assert_eq!(read::<String>(&file).unwrap(), "instanceharbor-v2-windows-preview-1");

        save(&file, &"instanceharbor-workspace-2").unwrap();
        assert_eq!(dispatch(&[path, "list".into()]).unwrap_err(), "UnsupportedWorkspace");
        fs::remove_dir_all(&parent).unwrap();
    }
}

#[cfg(test)]
mod start_preflight_tests {
    use super::*;

    fn free_port() -> u16 {
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
    }

    #[test]
    fn a_stop_typed_in_the_console_is_reported_as_a_stop_phase() {
        let (parent, root, config) = workspace_with_missing_java();
        let folder = root.join("runs").join(&config.name);
        fs::create_dir_all(&folder).unwrap();
        assert!(status(&root, &config).unwrap()["stopPhase"].is_null());
        fs::write(folder.join(native_console::STOP_TYPED), b"").unwrap();
        assert_eq!(status(&root, &config).unwrap()["stopPhase"], "Typed");
        fs::write(folder.join("stop-sent"), b"").unwrap();
        assert_eq!(status(&root, &config).unwrap()["stopPhase"], "Sent");
        fs::remove_dir_all(&parent).unwrap();
    }

    /// An isolated workspace with one instance whose registered Java was removed.
    fn workspace_with_missing_java() -> (PathBuf, PathBuf, Instance) {
        let parent = std::env::temp_dir().join(format!("ih-start-preflight-{}-{}", std::process::id(), clock()));
        fs::create_dir(&parent).unwrap();
        let workspace = parent.join("workspace");
        dispatch(&[workspace.to_string_lossy().into_owned(), "init".into()]).unwrap();
        let root = workspace.canonicalize().unwrap();
        let port = free_port();
        let directory = root.join("instances").join("agent");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("server.jar"), b"not a real jar").unwrap();
        fs::write(directory.join("server.properties"), format!("server-ip=127.0.0.1\nserver-port={port}\n")).unwrap();
        let config = Instance { core_file: None,
            name: "agent".into(),
            java: parent.join("removed-jdk").join("bin").join("java.exe"),
            port,
            velocity_listen_address: None,
            removed: false,
            directory: None,
            jvm: None,
            timing: None,
            auto_restart: None,
            core_profile: None,
            core_local_create: false,
            management_discovered: false,
        };
        save(&record_path(&root, "agent"), &config).unwrap();
        (parent, root, config)
    }

    #[test]
    fn registered_java_reports_a_missing_file_with_a_stable_code() {
        let parent = std::env::temp_dir().join(format!("ih-java-present-{}-{}", std::process::id(), clock()));
        fs::create_dir(&parent).unwrap();
        assert_eq!(
            registered_java_present(&parent.join("gone").join("bin").join("java.exe")).unwrap_err(),
            "JavaExecutableMissing"
        );
        assert_eq!(registered_java_present(&parent.join("java.exe")).unwrap_err(), "JavaExecutableMissing");
        fs::write(parent.join("java.exe"), b"").unwrap();
        registered_java_present(&parent.join("java.exe")).unwrap();
        fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn start_with_missing_java_fails_in_preflight_and_can_be_sent_again() {
        let (parent, root, config) = workspace_with_missing_java();
        let receipt = root.join("jobs").join("start-1.json");
        for _ in 0..2 {
            assert_eq!(
                lifecycle(&root, &config, "start", "start-1", None, None).unwrap_err(),
                "JavaExecutableMissing"
            );
            assert!(!receipt.exists(), "a preflight failure must not leave an unconfirmed receipt");
        }

        let before = status(&root, &config).unwrap();
        let guard = StartGuard {
            previous_run: before["run"]["id"].as_str().map(str::to_owned),
            stop_token: before["stopToken"].as_str().map(str::to_owned),
            registration: None,
            previous_identity: None,
            auto_attempt: None,
            console_window: false,
        };
        for _ in 0..2 {
            assert_eq!(
                lifecycle(&root, &config, "start-if-current", "start-2", None, Some(&guard)).unwrap_err(),
                "JavaExecutableMissing"
            );
            assert!(!root.join("jobs").join("start-2.json").exists());
        }

        // The GUI keeps sending the same request ID until a start succeeds. Once the
        // user puts a file back at the registered path, that ID must run preflight
        // again and get past the Java check instead of replaying ResultUnconfirmed.
        fs::create_dir_all(config.java.parent().unwrap()).unwrap();
        fs::write(&config.java, b"not an executable").unwrap();
        let error = lifecycle(&root, &config, "start-if-current", "start-2", None, Some(&guard)).unwrap_err();
        assert!(
            !error.contains("JavaExecutableMissing") && !error.contains("ResultUnconfirmed"),
            "the retry did not run preflight again: {error}"
        );
        assert!(!root.join("jobs").join("start-2.json").exists());
        assert!(!root.join("runs").join("agent").join("identity.json").exists());
        fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn releasing_keeps_completed_receipts_and_reports_why() {
        let root = std::env::temp_dir().join(format!("ih-receipt-release-{}-{}", std::process::id(), clock()));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join("done.json");
        let complete = Receipt { arguments: vec![], complete: true, output: Some(json!({})), created_at: None };
        save(&path, &complete).unwrap();
        let error = release_receipt_on_error::<()>(&root, &path, Err("PortChanged".into())).unwrap_err();
        assert!(error.starts_with("PortChanged; ReceiptAlreadyComplete"), "{error}");
        assert!(path.exists());

        let pending = root.join("pending.json");
        reserve_receipt(&root, &pending, &Receipt { arguments: vec![], complete: false, output: None, created_at: None }).unwrap();
        assert_eq!(release_receipt_on_error::<()>(&root, &pending, Err("PortChanged".into())).unwrap_err(), "PortChanged");
        assert!(!pending.exists());
        assert_eq!(release_receipt_on_error(&root, &pending, Ok(7)).unwrap(), 7);
        fs::remove_dir_all(&root).unwrap();
    }
}
