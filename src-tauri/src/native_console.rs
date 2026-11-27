use super::*;
use std::os::windows::fs::FileExt;
use std::os::windows::{
    ffi::OsStrExt,
    io::{FromRawHandle, OwnedHandle},
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::io::{Seek, SeekFrom};
use std::time::Instant;

/// The console a run uses, in order of preference. The Host records the
/// chosen tier and every skipped tier with its reason in `console-tier.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Tier {
    /// The pinned `conpty.dll` and `OpenConsole.exe` next to the executable, shown in our viewer.
    Bundled,
    /// The pseudoconsole that ships with Windows (kernel32), shown in our viewer.
    System,
    /// A real console window. Closing that window stops the server.
    Window,
    /// No console. The Host owns standard input and output.
    Plain,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Skipped {
    pub(super) tier: Tier,
    pub(super) reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Choice {
    pub(super) tier: Tier,
    pub(super) skipped: Vec<Skipped>,
}

/// Development builds can lower the best tier to exercise each fallback.
/// Stable builds ignore the variable.
pub(super) const TIER_LIMIT_VARIABLE: &str = "IH_CONSOLE_TIER_LIMIT";

pub(super) fn tier_limit() -> Option<Tier> {
    if !crate::desktop_channel::DEVELOPMENT {
        return None;
    }
    match std::env::var(TIER_LIMIT_VARIABLE).ok()?.as_str() {
        "system" => Some(Tier::System),
        "window" => Some(Tier::Window),
        "plain" => Some(Tier::Plain),
        _ => None,
    }
}

fn rank(tier: Tier) -> u8 {
    match tier {
        Tier::Bundled => 0,
        Tier::System => 1,
        Tier::Window => 2,
        Tier::Plain => 3,
    }
}

/// What the start-time checks found before trying a tier.
pub(super) struct Facts {
    pub(super) paths: Result<()>,
    pub(super) bundled: Result<()>,
    pub(super) system: Result<()>,
    pub(super) window: Result<()>,
    pub(super) window_confirmed: bool,
    pub(super) limit: Option<Tier>,
}

/// Choose the first tier that the checks allow. A tier may still fail when
/// it starts; the caller then records that error and calls `choose` again.
pub(super) fn choose(facts: &Facts) -> Choice {
    let mut skipped = Vec::new();
    for tier in [Tier::Bundled, Tier::System, Tier::Window] {
        // Every server core gets a console. The pseudoconsole hands keystrokes
        // to the server's stdin, whether the core edits lines with JLine
        // (Bukkit, Spigot, Paper and its forks, Velocity) or reads plain lines.
        let blocked: Option<String> = if facts.limit.is_some_and(|limit| rank(tier) < rank(limit)) {
            Some("ConsoleTierLimited".into())
        } else if let Err(error) = &facts.paths {
            Some(error.clone())
        } else {
            match tier {
                Tier::Bundled => facts.bundled.as_ref().err().cloned(),
                Tier::System => facts.system.as_ref().err().cloned(),
                _ if !facts.window_confirmed => Some("ConsoleWindowNotConfirmed".into()),
                _ => facts.window.as_ref().err().cloned(),
            }
        };
        match blocked {
            Some(reason) => skipped.push(Skipped { tier, reason }),
            None => return Choice { tier, skipped },
        }
    }
    Choice { tier: Tier::Plain, skipped }
}

enum InputMessage {
    Interactive(Vec<u8>),
    Managed(Vec<u8>, mpsc::SyncSender<Result<()>>),
}

#[derive(Default)]
struct InputGate {
    managed: bool,
    aborted: bool,
    line: LineTracker,
}

impl InputGate {
    fn reserve(&mut self) -> Result<()> {
        if self.managed || self.aborted {
            return Err("ConsoleManagedInputUnavailable".into());
        }
        self.managed = true;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Escape {
    #[default]
    None,
    Start,
    Csi,
    Ss3,
    String,
}

/// The Host's model of the line the user is editing in the viewer. Managed
/// commands enter the input queue only while this line is known to be empty,
/// so they never join a partly typed user command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct LineTracker {
    text: String,
    /// Set after cursor keys, history, completion or other edits whose
    /// result the Host does not model. Only Enter clears it.
    unknown: bool,
    escape: Escape,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Submitted {
    Line(String),
    /// Enter after an edit the Host cannot read back, such as a history recall.
    Unverified,
    /// Ctrl+C, or Ctrl+D on an empty line. JLine stops the server for both.
    Interrupt,
}

impl LineTracker {
    pub(super) fn is_empty(&self) -> bool {
        !self.unknown && self.text.is_empty() && self.escape == Escape::None
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) -> Vec<Submitted> {
        let mut submitted = Vec::new();
        for ch in String::from_utf8_lossy(bytes).chars() {
            match self.escape {
                Escape::Start => {
                    self.escape = match ch {
                        '[' => Escape::Csi,
                        'O' => Escape::Ss3,
                        ']' | 'P' | '_' | '^' => Escape::String,
                        _ => Escape::None,
                    };
                    continue;
                }
                Escape::Csi => {
                    if ('\u{40}'..='\u{7e}').contains(&ch) {
                        self.escape = Escape::None;
                    }
                    continue;
                }
                Escape::Ss3 => {
                    self.escape = Escape::None;
                    continue;
                }
                Escape::String => {
                    if ch == '\u{7}' {
                        self.escape = Escape::None;
                    } else if ch == '\u{1b}' {
                        self.escape = Escape::Start;
                    }
                    continue;
                }
                Escape::None => {}
            }
            match ch {
                '\r' | '\n' => {
                    let text = std::mem::take(&mut self.text);
                    submitted.push(if self.unknown { Submitted::Unverified } else { Submitted::Line(text) });
                    self.unknown = false;
                }
                '\u{3}' => {
                    submitted.push(Submitted::Interrupt);
                    self.text.clear();
                    self.unknown = false;
                }
                '\u{4}' if self.is_empty() => submitted.push(Submitted::Interrupt),
                '\u{7f}' | '\u{8}' => {
                    if !self.unknown {
                        self.text.pop();
                    }
                }
                '\u{1b}' => {
                    self.unknown = true;
                    self.escape = Escape::Start;
                }
                ch if ch.is_control() => self.unknown = true,
                ch => {
                    if !self.unknown {
                        self.text.push(ch);
                    }
                }
            }
        }
        submitted
    }
}

/// Commands that stop Paper, Folia or Velocity when typed in the console.
pub(super) fn stops_server(line: &str) -> bool {
    let word = line.trim().trim_start_matches('/').split_whitespace().next().unwrap_or("");
    let word = word.rsplit(':').next().unwrap_or("").to_ascii_lowercase();
    matches!(word.as_str(), "stop" | "end" | "shutdown" | "restart")
}

/// Marker files in the run folder that `auto_restart` reads after an exit.
pub(super) const STOP_TYPED: &str = "console-stop-typed";
pub(super) const LINE_UNVERIFIED: &str = "console-line-unverified.json";

fn record_submissions(folder: &Path, submitted: &[Submitted]) {
    for item in submitted {
        match item {
            Submitted::Line(line) if stops_server(line) => mark_host_phase(&folder.join(STOP_TYPED)),
            Submitted::Interrupt => mark_host_phase(&folder.join(STOP_TYPED)),
            Submitted::Unverified => {
                if let Err(error) = save(&folder.join(LINE_UNVERIFIED), &epoch_ms()) {
                    eprintln!("Could not record unverified console input: {error}");
                }
            }
            Submitted::Line(_) => {}
        }
    }
}

/// The Host keeps this guard until the corresponding output has been read back.
/// A lost write acknowledgement leaves the operation unconfirmed, never retried.
pub(super) struct ManagedInput {
    gate: Arc<Mutex<InputGate>>,
    input: mpsc::SyncSender<InputMessage>,
}

const LINE_WAIT: Duration = Duration::from_millis(25);

impl ManagedInput {
    /// Waits until the user's line is empty, then queues one complete line.
    /// `ConsoleManagedInputBusy` means nothing was written.
    pub(super) fn write_command(&self, line: &str, deadline: Instant) -> Result<()> {
        if line.is_empty() || line.len() > 8191 || line.chars().any(char::is_control) {
            return Err("InvalidConsoleManagedCommand".into());
        }
        if deadline <= Instant::now() {
            return Err("ConsoleManagedWriteUnconfirmed".into());
        }
        let mut command = line.as_bytes().to_vec();
        command.push(b'\r');
        let (sent, written) = mpsc::sync_channel(1);
        let mut message = Some(InputMessage::Managed(command, sent));
        while let Some(pending) = message.take() {
            {
                let gate = self.gate.lock().map_err(|_| "ConsoleInputUnavailable")?;
                if !gate.managed || gate.aborted {
                    return Err("ConsoleManagedAborted".into());
                }
                if gate.line.is_empty() {
                    // The same lock orders this line against interactive bytes.
                    self.input
                        .try_send(pending)
                        .map_err(|_| "ConsoleInputUnavailable")?;
                    break;
                }
            }
            if Instant::now() + LINE_WAIT >= deadline {
                return Err("ConsoleManagedInputBusy".into());
            }
            message = Some(pending);
            thread::sleep(LINE_WAIT);
        }
        let result = deadline
            .checked_duration_since(Instant::now())
            .and_then(|wait| written.recv_timeout(wait).ok())
            .unwrap_or_else(|| Err("ConsoleManagedWriteUnconfirmed".into()));
        let mut gate = self.gate.lock().map_err(|_| "ConsoleInputUnavailable")?;
        if result.is_err() {
            // The writer may still hold a partial or queued command.
            gate.aborted = true;
        } else if gate.aborted {
            return Err("ConsoleManagedAborted".into());
        }
        result
    }
}

impl Drop for ManagedInput {
    fn drop(&mut self) {
        if let Ok(mut gate) = self.gate.lock() {
            gate.managed = false;
        }
    }
}

type Handle = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
struct Coord {
    x: i16,
    y: i16,
}
#[repr(C)]
struct ExtendedStartup {
    base: StartupInfo,
    attributes: Handle,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct KeyRecord {
    event_type: u16,
    _padding: u16,
    key_down: i32,
    repeat: u16,
    virtual_key: u16,
    scan_code: u16,
    unicode: u16,
    control_state: u32,
}
type Create = unsafe extern "system" fn(Coord, Handle, Handle, u32, *mut Handle) -> i32;
type Close = unsafe extern "system" fn(Handle);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreatePipe(read: *mut Handle, write: *mut Handle, security: Handle, size: u32) -> i32;
    fn InitializeProcThreadAttributeList(
        list: Handle,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: Handle,
        flags: u32,
        attribute: usize,
        value: Handle,
        size: usize,
        previous: Handle,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: Handle);
    fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
    fn FreeLibrary(module: Handle) -> i32;
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> Handle;
    fn WaitForSingleObject(process: Handle, millis: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn AttachConsole(pid: u32) -> i32;
    fn FreeConsole() -> i32;
    fn WriteConsoleInputW(input: Handle, records: *const KeyRecord, count: u32, written: *mut u32) -> i32;
}

/// Pinned identities of the bundled runtime. `packaging/console-runtime.json` must match.
pub(super) const RUNTIME_FILES: [(&str, &str); 2] = [
    ("conpty.dll", "feeef341d891643c62d30b6b07800bc70f0bc148f44cb8c3bee84aa557ae805a"),
    ("OpenConsole.exe", "3d66b23d0a71bb8eed2b77edc8b9df9bf54ce6c8fb74c863a30e760997f80586"),
];

type RuntimeKey = Vec<(u64, Option<std::time::SystemTime>)>;
static RUNTIME_CHECK: Mutex<Option<(PathBuf, RuntimeKey, Result<()>)>> = Mutex::new(None);

/// Verifies the bundled runtime next to the executable. Hashes are reused
/// until a file's length or modification time changes.
pub(super) fn bundled_runtime() -> Result<PathBuf> {
    let dir = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(fail)?;
    let mut key = Vec::new();
    for (name, _) in RUNTIME_FILES {
        let path = dir.with_file_name(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => key.push((metadata.len(), metadata.modified().ok())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Err("ConsoleRuntimeMissing".into()),
            Err(error) => return Err(fail(error)),
        }
    }
    let dll = dir.with_file_name(RUNTIME_FILES[0].0);
    let mut cache = RUNTIME_CHECK.lock().map_err(fail)?;
    if let Some((cached_dir, cached_key, result)) = cache.as_ref() {
        if *cached_dir == dir && *cached_key == key {
            return result.clone().map(|()| dll);
        }
    }
    let result = RUNTIME_FILES.iter().try_for_each(|(name, expected)| {
        let path = dir.with_file_name(name);
        no_redirect(&path)?;
        if content_revision::revision(&fs::read(&path).map_err(fail)?)? != *expected {
            return Err("ConsoleRuntimeMismatch".to_owned());
        }
        Ok(())
    });
    *cache = Some((dir, key, result.clone()));
    result.map(|()| dll)
}

fn system_functions() -> Result<(Handle, Handle, Handle)> {
    let module = unsafe { GetModuleHandleW(wide(std::ffi::OsStr::new("kernel32.dll")).as_ptr()) };
    if module.is_null() {
        return Err("ConsoleSystemUnavailable".into());
    }
    let create = unsafe { GetProcAddress(module, c"CreatePseudoConsole".as_ptr().cast()) };
    let close = unsafe { GetProcAddress(module, c"ClosePseudoConsole".as_ptr().cast()) };
    let resize = unsafe { GetProcAddress(module, c"ResizePseudoConsole".as_ptr().cast()) };
    if create.is_null() || close.is_null() || resize.is_null() {
        return Err("ConsoleSystemUnavailable".into());
    }
    Ok((create, close, resize))
}

/// Whether this Windows version provides a pseudoconsole.
pub(super) fn system_runtime() -> Result<()> {
    system_functions().map(|_| ())
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain([0]).collect()
}
fn launcher_path(path: &Path) -> Result<PathBuf> {
    // HotSpot uses the process image path to locate lib/modules. For Java
    // in a console, pass the same ordinary path that Java's Windows launcher expects.
    let ordinary = PathBuf::from(path_format::display(path).replace('/', "\\"));
    if ordinary.as_os_str().encode_wide().count() >= 260
        || fs::canonicalize(&ordinary).map_err(fail)? != fs::canonicalize(path).map_err(fail)?
    {
        return Err("ConsolePathUnsupported".into());
    }
    Ok(ordinary)
}

/// Every console tier starts Java with ordinary paths shorter than MAX_PATH.
pub(super) fn launch_paths(java: &Path, directory: &Path, jar: &Path) -> Result<()> {
    launcher_path(java)?;
    launcher_path(directory)?;
    launcher_path(jar).map(|_| ())
}

fn system_error() -> String {
    fail(io::Error::last_os_error())
}
fn pipe() -> Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
    if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null_mut(), 0) } == 0 {
        return Err(system_error());
    }
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    })
}
struct Library(Handle);
impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.0);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
struct Grid {
    cols: u16,
    rows: u16,
}

const INITIAL_GRID: Grid = Grid { cols: 100, rows: 35 };

impl Grid {
    fn valid(self) -> bool {
        (20..=500).contains(&self.cols) && (10..=200).contains(&self.rows)
    }
}

type Resize = unsafe extern "system" fn(Handle, Coord) -> i32;
struct PseudoConsole {
    handle: Handle,
    close: Close,
    resize: Resize,
    _library: Option<Library>,
}
// Closing an owned pseudoconsole is independent of the thread that opened it.
unsafe impl Send for PseudoConsole {}

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        unsafe { (self.close)(self.handle) };
    }
}

/// Java's command line. The caller constructs JVM arguments from numbers and a GC enum.
fn command_line(java: &Path, directory: &Path, jar: &Path, jvm: &[String], nogui: bool) -> Result<Vec<u16>> {
    let java = launcher_path(java)?;
    let mut command = vec![b'"' as u16];
    command.extend(java.as_os_str().encode_wide());
    command.push(b'"' as u16);
    for argument in jvm {
        command.push(b' ' as u16);
        command.extend(argument.encode_utf16());
    }
    // Legacy Java resolves the local server JAR through a file URI. A relative
    // name avoids the absolute-path ZipFS failure on old launchers.
    command.extend(" -jar \"".encode_utf16());
    if let Some(name) = jar
        .strip_prefix(directory)
        .ok()
        .filter(|name| name.components().count() == 1)
    {
        command.extend(name.as_os_str().encode_wide());
    } else {
        command.extend(launcher_path(jar)?.as_os_str().encode_wide());
    }
    command.push(b'"' as u16);
    if nogui {
        command.extend(" nogui".encode_utf16());
    }
    command.push(0);
    Ok(command)
}

fn create_process(
    java: &Path,
    directory: &Path,
    mut command: Vec<u16>,
    flags: u32,
    startup: &mut StartupInfo,
) -> Result<(OwnedHandle, u32)> {
    let application = wide(launcher_path(java)?.as_os_str());
    let directory = wide(launcher_path(directory)?.as_os_str());
    let mut output: ProcessInfo = unsafe { std::mem::zeroed() };
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            flags,
            std::ptr::null_mut(),
            directory.as_ptr(),
            startup,
            &mut output,
        )
    } == 0
    {
        return Err(system_error());
    }
    let process = unsafe { OwnedHandle::from_raw_handle(output.process) };
    unsafe {
        CloseHandle(output.thread);
    }
    Ok((process, output.pid))
}

impl PseudoConsole {
    fn open(tier: Tier, input: &OwnedHandle, output: &OwnedHandle) -> Result<Self> {
        let (create, close, resize, library) = match tier {
            Tier::Bundled => {
                let dll = bundled_runtime()?;
                // DLL load directory and System32 are the only dependency lookup locations.
                let module = unsafe {
                    LoadLibraryExW(wide(dll.as_os_str()).as_ptr(), std::ptr::null_mut(), 0x900)
                };
                if module.is_null() {
                    return Err(system_error());
                }
                let library = Library(module);
                let create =
                    unsafe { GetProcAddress(module, c"ConptyCreatePseudoConsole".as_ptr().cast()) };
                let close =
                    unsafe { GetProcAddress(module, c"ConptyClosePseudoConsole".as_ptr().cast()) };
                // Use the same pinned provider for creating and resizing its handle.
                let resize =
                    unsafe { GetProcAddress(module, c"ConptyResizePseudoConsole".as_ptr().cast()) };
                if create.is_null() || close.is_null() || resize.is_null() {
                    return Err("ConsoleRuntimeMismatch".into());
                }
                (create, close, resize, Some(library))
            }
            Tier::System => {
                let (create, close, resize) = system_functions()?;
                (create, close, resize, None)
            }
            _ => return Err("ConsoleTierInvalid".into()),
        };
        let create: Create = unsafe { std::mem::transmute(create) };
        let close: Close = unsafe { std::mem::transmute(close) };
        let mut handle = std::ptr::null_mut();
        let result = unsafe {
            create(
                Coord { x: INITIAL_GRID.cols as i16, y: INITIAL_GRID.rows as i16 },
                input.as_raw_handle(),
                output.as_raw_handle(),
                // The pinned provider supports grapheme widths; Windows' own takes no flags.
                if tier == Tier::Bundled { 8 } else { 0 },
                &mut handle,
            )
        };
        if result != 0 {
            return Err(format!("ConsoleCreateFailed {result:#x}"));
        }
        Ok(Self {
            handle,
            close,
            resize: unsafe { std::mem::transmute::<Handle, Resize>(resize) },
            _library: library,
        })
    }

    fn resize(&self, grid: Grid) -> Result<()> {
        let code = unsafe { (self.resize)(self.handle, Coord { x: grid.cols as i16, y: grid.rows as i16 }) };
        if code == 0 {
            Ok(())
        } else {
            Err(format!("ConsoleResizeFailed {code:#x}"))
        }
    }

    fn spawn(&self, java: &Path, directory: &Path, jar: &Path, jvm: &[String], nogui: bool) -> Result<(OwnedHandle, u32)> {
        let mut size = 0;
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size);
        }
        if size == 0 {
            return Err(system_error());
        }
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
        let attributes = storage.as_mut_ptr().cast();
        if unsafe { InitializeProcThreadAttributeList(attributes, 1, 0, &mut size) } == 0 {
            return Err(system_error());
        }
        struct Attributes(Handle);
        impl Drop for Attributes {
            fn drop(&mut self) {
                unsafe { DeleteProcThreadAttributeList(self.0) };
            }
        }
        let _attributes = Attributes(attributes);
        if unsafe {
            UpdateProcThreadAttribute(
                attributes,
                0,
                0x00020016,
                self.handle,
                size_of::<Handle>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(system_error());
        }
        let command = command_line(java, directory, jar, jvm, nogui)?;
        let mut startup: ExtendedStartup = unsafe { std::mem::zeroed() };
        startup.base.size = size_of::<ExtendedStartup>() as u32;
        startup.attributes = attributes;
        create_process(java, directory, command, 0x00080000, &mut startup.base)
    }
}

/// Copies `logs/latest.log` of a console-window run into `stdout.log`, so
/// readiness, the Logs dialog and diagnostics keep working.
struct LogMirror {
    stop: Arc<AtomicBool>,
}

impl LogMirror {
    fn start(source: PathBuf, mut log: File, drained: mpsc::SyncSender<Result<()>>) -> Self {
        // A previous run's file stays in place until the server rotates it.
        let previous = head(&source);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        thread::spawn(move || {
            let mut offset = 0u64;
            let mut fresh = previous.is_none();
            let result = loop {
                let last = stopping.load(Ordering::Acquire);
                if !fresh {
                    fresh = match (&previous, head(&source)) {
                        (Some((old_len, old)), Some((len, now))) => len < *old_len || now != *old,
                        _ => false,
                    };
                }
                if fresh {
                    if let Err(error) = copy_from(&source, &mut offset, &mut log) {
                        break Err(error);
                    }
                }
                if last {
                    break log.sync_all().map_err(fail);
                }
                thread::sleep(Duration::from_millis(200));
            };
            let _ = drained.send(result);
        });
        Self { stop }
    }
}

/// Length and first bytes of a log file, used to tell a new file from the old one.
fn head(path: &Path) -> Option<(u64, Vec<u8>)> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut bytes = Vec::new();
    (&mut file).take(512).read_to_end(&mut bytes).ok()?;
    Some((len, bytes))
}

fn copy_from(source: &Path, offset: &mut u64, log: &mut File) -> Result<()> {
    // Open by name each time so the server can rotate or compress the file.
    let Ok(mut file) = File::open(source) else { return Ok(()) };
    let len = file.metadata().map_err(fail)?.len();
    if len < *offset {
        *offset = 0;
    }
    file.seek(SeekFrom::Start(*offset)).map_err(fail)?;
    *offset += io::copy(&mut file, log).map_err(fail)?;
    Ok(())
}

/// Sends input to a console window as key events. The Host attaches to the
/// window only for the duration of one write.
fn write_window(pid: u32, bytes: &[u8]) -> Result<()> {
    let mut records = Vec::new();
    for unit in String::from_utf8_lossy(bytes).encode_utf16() {
        let (virtual_key, control_state) = match unit {
            0x0d => (0x0d, 0),
            // Ctrl+letter: JLine reads the character, Windows the key.
            0x01..=0x1a => (0x40 + unit, 0x0008),
            _ => (0, 0),
        };
        for key_down in [1, 0] {
            records.push(KeyRecord {
                event_type: 1,
                _padding: 0,
                key_down,
                repeat: 1,
                virtual_key,
                scan_code: 0,
                unicode: unit,
                control_state,
            });
        }
    }
    if unsafe { AttachConsole(pid) } == 0 {
        return Err(system_error());
    }
    struct Detach;
    impl Drop for Detach {
        fn drop(&mut self) {
            unsafe { FreeConsole() };
        }
    }
    let _detach = Detach;
    let input = OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONIN$")
        .map_err(fail)?;
    let mut written = 0;
    if unsafe {
        WriteConsoleInputW(input.as_raw_handle(), records.as_ptr(), records.len() as u32, &mut written)
    } == 0
        || written as usize != records.len()
    {
        return Err(system_error());
    }
    Ok(())
}

enum Output {
    Pty(Arc<Mutex<Option<PseudoConsole>>>),
    Window(LogMirror),
}

pub(super) struct Console {
    tier: Tier,
    process: OwnedHandle,
    pid: u32,
    input: mpsc::SyncSender<InputMessage>,
    input_gate: Arc<Mutex<InputGate>>,
    drained: Option<mpsc::Receiver<Result<()>>>,
    output: Option<Output>,
    folder: PathBuf,
}

fn writer(receiver: mpsc::Receiver<InputMessage>, mut write: impl FnMut(&[u8]) -> Result<()> + Send + 'static) {
    thread::spawn(move || {
        for message in receiver {
            let result = match message {
                InputMessage::Interactive(bytes) => write(&bytes),
                InputMessage::Managed(bytes, reply) => {
                    let result = write(&bytes);
                    let failed = result.is_err();
                    let _ = reply.send(result);
                    if failed {
                        break;
                    }
                    continue;
                }
            };
            if result.is_err() {
                break;
            }
        }
    });
}

pub(super) struct Launch<'a> {
    pub(super) java: &'a Path,
    pub(super) directory: &'a Path,
    pub(super) jar: &'a Path,
    pub(super) jvm: &'a [String],
    pub(super) nogui: bool,
    pub(super) folder: &'a Path,
    pub(super) title: &'a str,
}

impl Console {
    /// Starts Java in a pseudoconsole (tier 1 or 2) or a console window (tier 3).
    pub(super) fn spawn(tier: Tier, launch: &Launch, mut log: File) -> Result<Self> {
        let (input, receiver) = mpsc::sync_channel::<InputMessage>(64);
        let (done, drained) = mpsc::sync_channel(1);
        let (process, pid, output) = match tier {
            Tier::Bundled | Tier::System => {
                let (input_read, input_write) = pipe()?;
                let (output_read, output_write) = pipe()?;
                let console = PseudoConsole::open(tier, &input_read, &output_write)?;
                thread::spawn(move || {
                    let mut reader = File::from(output_read);
                    let result = io::copy(&mut reader, &mut log).and_then(|_| log.sync_all()).map_err(fail);
                    let _ = done.send(result);
                });
                let (process, pid) = console.spawn(launch.java, launch.directory, launch.jar, launch.jvm, launch.nogui)?;
                drop(input_read);
                drop(output_write);
                let mut pipe = File::from(input_write);
                writer(receiver, move |bytes| pipe.write_all(bytes).map_err(fail));
                (process, pid, Output::Pty(Arc::new(Mutex::new(Some(console)))))
            }
            Tier::Window => {
                let command = command_line(launch.java, launch.directory, launch.jar, launch.jvm, launch.nogui)?;
                let mut title = wide(std::ffi::OsStr::new(launch.title));
                let mut startup: StartupInfo = unsafe { std::mem::zeroed() };
                startup.size = size_of::<StartupInfo>() as u32;
                startup.title = title.as_mut_ptr();
                let mirror = LogMirror::start(launch.directory.join("logs").join("latest.log"), log, done);
                // CREATE_NEW_CONSOLE: Windows opens conhost or the default terminal.
                let (process, pid) = match create_process(launch.java, launch.directory, command, 0x00000010, &mut startup) {
                    Ok(started) => started,
                    Err(error) => {
                        mirror.stop.store(true, Ordering::Release);
                        return Err(error);
                    }
                };
                writer(receiver, move |bytes| write_window(pid, bytes));
                (process, pid, Output::Window(mirror))
            }
            Tier::Plain => return Err("ConsoleTierInvalid".into()),
        };
        Ok(Self {
            tier,
            process,
            pid,
            input,
            input_gate: Arc::new(Mutex::new(InputGate::default())),
            drained: Some(drained),
            output: Some(output),
            folder: launch.folder.to_owned(),
        })
    }
    pub(super) fn tier(&self) -> Tier {
        self.tier
    }
    pub(super) fn pid(&self) -> u32 {
        self.pid
    }
    pub(super) fn handle(&self) -> Handle {
        self.process.as_raw_handle()
    }
    pub(super) fn exit(&self) -> Result<Option<i32>> {
        match unsafe { WaitForSingleObject(self.handle(), 0) } {
            258 => Ok(None),
            0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.handle(), &mut code) } == 0 {
                    return Err(system_error());
                }
                Ok(Some(code as i32))
            }
            _ => Err(system_error()),
        }
    }
    pub(super) fn close_and_drain(&mut self) -> Result<()> {
        let Some(output) = self.output.take() else { return Ok(()) };
        match output {
            Output::Pty(console) => {
                let Some(console) = console.lock().map_err(fail)?.take() else { return Ok(()) };
                thread::spawn(move || drop(console));
            }
            Output::Window(mirror) => mirror.stop.store(true, Ordering::Release),
        }
        let drained = self.drained.take().ok_or("ConsoleDrainUnavailable")?;
        drained.recv_timeout(Duration::from_secs(5)).map_err(|_| "ConsoleDrainUnconfirmed")?
    }
    pub(super) fn stop(&self, command: &str) -> Result<()> {
        self.abort_managed_input()?;
        // JLine: move to the start and clear the pending input before issuing stop.
        // A console window may read cooked input, where these keys are literal.
        let line = if self.tier == Tier::Window {
            format!("{command}\r")
        } else {
            format!("\x01\x0b{command}\r")
        };
        self.input
            .try_send(InputMessage::Interactive(line.into_bytes()))
            .map_err(|_| "ConsoleInputUnavailable".into())
    }

    /// Called as soon as the Host accepts a stop intent, before it records a
    /// stop marker. No later managed command may enter the writer queue.
    pub(super) fn abort_managed_input(&self) -> Result<()> {
        self.input_gate
            .lock()
            .map_err(|_| "ConsoleInputUnavailable")?
            .aborted = true;
        Ok(())
    }

    pub(super) fn begin_managed_input(&self) -> Result<ManagedInput> {
        self.input_gate
            .lock()
            .map_err(|_| "ConsoleInputUnavailable")?
            .reserve()?;
        Ok(ManagedInput {
            gate: Arc::clone(&self.input_gate),
            input: self.input.clone(),
        })
    }
    /// Serves the viewer pipe of a pseudoconsole run. A console window has no viewer.
    pub(super) fn serve(
        &self,
        run: &Run,
        log: File,
        recovery: Option<Arc<Mutex<super::host_recovery::Gate>>>,
    ) -> Result<()> {
        let Some(Output::Pty(console)) = &self.output else {
            return Err("ConsoleViewerUnsupported".into());
        };
        let console = Arc::clone(console);
        let exe = std::env::current_exe().map_err(fail)?;
        let gui = exe
            .with_file_name(crate::desktop_channel::GUI_EXECUTABLE)
            .canonicalize()
            .map_err(fail)?;
        let expected = [exe.canonicalize().map_err(fail)?, gui.clone()];
        let input = self.input.clone();
        let input_gate = Arc::clone(&self.input_gate);
        let folder = self.folder.clone();
        let pipe_name = endpoint(&run.id);
        let (ready, started) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let grids = Mutex::new(vec![(0, INITIAL_GRID)]);
            let respond = |request: &str| -> Result<Value> {
                let command: Request =
                    serde_json::from_str(request).map_err(|_| "InvalidConsoleRequest")?;
                match command {
                    Request::Probe => Ok(Value::Null),
                    Request::Read { offset } => {
                        let mut bytes = [0u8; 8192];
                        let grids = grids.lock().map_err(fail)?;
                        let (grid, limit) = grid_at(&grids, offset, bytes.len());
                        let size = log.seek_read(&mut bytes[..limit], offset).map_err(fail)?;
                        Ok(json!({"bytes":&bytes[..size],"grid":grid}))
                    }
                    Request::Resize { cols, rows, offset } if (Grid { cols, rows }).valid() => {
                        let current = log.metadata().map_err(fail)?.len();
                        if current != offset {
                            return Ok(json!({"caughtUp":false}));
                        }
                        let grid = Grid { cols, rows };
                        let mut grids = grids.lock().map_err(fail)?;
                        let last = grids.last().expect("initial console grid");
                        if last.1 != grid {
                            if last.0 != offset && grids.len() >= 4096 {
                                return Err("ConsoleResizeHistoryLimit".into());
                            }
                            console.lock().map_err(fail)?.as_ref().ok_or("ConsoleRunEnded")?.resize(grid)?;
                            if grids.last().is_some_and(|last| last.0 == offset) {
                                grids.pop();
                            }
                            grids.push((offset, grid));
                        }
                        Ok(json!({"caughtUp":true,"grid":grid}))
                    }
                    Request::Input { bytes } if !bytes.is_empty() && bytes.len() <= 8192 => {
                        let mut gate = input_gate.lock().map_err(|_| "ConsoleInputUnavailable")?;
                        if gate.aborted {
                            return Err("ConsoleInputUnavailable".into());
                        }
                        let mut line = gate.line.clone();
                        let submitted = line.feed(&bytes);
                        // Record a typed stop before Java can read it.
                        record_submissions(&folder, &submitted);
                        input
                            .try_send(InputMessage::Interactive(bytes))
                            .map_err(|_| "ConsoleInputUnavailable")?;
                        gate.line = line;
                        Ok(Value::Null)
                    }
                    _ => Err("InvalidConsoleRequest".into()),
                }
            };
            let result = host_pipe::serve_local_with(
                &pipe_name,
                |peer, image| match &recovery {
                    Some(gate) => gate.lock().is_ok_and(|gate| gate.console_authorizes(peer, image)),
                    None => expected.iter().any(|path| path == image),
                },
                || ready.send(()).map_err(io::Error::other),
                |request| match respond(request) {
                    Ok(value) => json!({"result":value}).to_string(),
                    Err(error) => json!({"error":error}).to_string(),
                },
            );
            if let Err(error) = result {
                eprintln!("Native console pipe: {error}");
            }
        });
        started
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "ConsoleHostUnavailable".into())
    }
}
impl Drop for Console {
    fn drop(&mut self) {
        if let Err(error) = self.close_and_drain() {
            eprintln!("Native console output: {error}");
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    Probe,
    Read { offset: u64 },
    Resize { cols: u16, rows: u16, offset: u64 },
    Input { bytes: Vec<u8> },
}

/// Keep reads within one grid interval so a newly opened viewer can apply
/// earlier sizes before replaying the VT bytes produced at each size.
fn grid_at(grids: &[(u64, Grid)], offset: u64, capacity: usize) -> (Grid, usize) {
    let index = grids.partition_point(|(start, _)| *start <= offset);
    let grid = grids[index - 1].1;
    let limit = grids.get(index).map_or(capacity, |(next, _)| {
        (*next - offset).min(capacity as u64) as usize
    });
    (grid, limit)
}
fn endpoint(run: &str) -> String {
    format!(r"\\.\pipe\instanceharbor-console-{run}")
}

pub(super) fn call(root: &Path, name: &str, expected_run: &str, request: Request) -> Result<Value> {
    if !instance_name(name) || !token(expected_run) || !admitted(root, name)? {
        return Err("ConsoleRunChanged".into());
    }
    let config = instance(root, name)?;
    if config.removed {
        return Err("ConsoleRunChanged".into());
    }
    let current = status(root, &config)?;
    if current["run"]["id"] != expected_run {
        return Err("ConsoleRunChanged".into());
    }
    if !matches!(current["state"].as_str(), Some("Starting" | "Listening")) {
        return Err("ConsoleRunEnded".into());
    }
    if current["nativeConsole"] != true {
        return Err("ConsoleUnavailable".into());
    }
    let run: Run = serde_json::from_value(current["run"].clone()).map_err(fail)?;
    let expected = std::env::current_exe()
        .map_err(fail)?
        .with_file_name("ihctl.exe")
        .canonicalize()
        .map_err(fail)?;
    if process_image(run.host.pid).and_then(|path| path.canonicalize().ok()) != Some(expected) {
        return Err("ConsoleHostMismatch".into());
    }
    let frame = serde_json::to_string(&request).map_err(fail)?;
    let endpoint = endpoint(&run.id);
    let mut response = host_pipe::local_request(&endpoint, &run.host, &frame);
    // Only retry failure to open the pipe. A request with a lost reply may
    // already have delivered input and must never be sent a second time.
    for _ in 0..8 {
        if response.as_ref().err().map(String::as_str) != Some("ManagerPipeUnavailable") {
            break;
        }
        thread::sleep(Duration::from_millis(15));
        response = host_pipe::local_request(&endpoint, &run.host, &frame);
    }
    let reply = response?;
    let message: Value = serde_json::from_str(&reply).map_err(fail)?;
    if let Some(error) = message["error"].as_str() {
        return Err(error.into());
    }
    message
        .get("result")
        .cloned()
        .ok_or("ConsoleResultUnconfirmed".into())
}


#[cfg(test)]
mod managed_input_tests {
    use super::*;

    fn managed(gate: &Arc<Mutex<InputGate>>) -> (ManagedInput, mpsc::Receiver<InputMessage>) {
        gate.lock().unwrap().reserve().unwrap();
        let (input, receiver) = mpsc::sync_channel(1);
        (ManagedInput { gate: gate.clone(), input }, receiver)
    }

    #[test]
    fn managed_guard_can_move_to_the_host_worker() {
        fn assert_send<T: Send>() {}
        assert_send::<ManagedInput>();
    }

    #[test]
    fn typing_no_longer_blocks_later_managed_commands() {
        let gate = Arc::new(Mutex::new(InputGate::default()));
        gate.lock().unwrap().line.feed(b"list\r");
        let (managed, receiver) = managed(&gate);
        let writer = thread::spawn(move || {
            let InputMessage::Managed(bytes, acknowledge) = receiver.recv().unwrap() else {
                panic!("unexpected interactive bytes")
            };
            assert_eq!(bytes, b"gamerule doDaylightCycle false\r");
            acknowledge.send(Ok(())).unwrap();
        });
        managed.write_command("gamerule doDaylightCycle false", Instant::now() + Duration::from_secs(1)).unwrap();
        writer.join().unwrap();
        assert_eq!(gate.lock().unwrap().reserve().unwrap_err(), "ConsoleManagedInputUnavailable");
        drop(managed);
        gate.lock().unwrap().reserve().unwrap();
    }

    #[test]
    fn a_partly_typed_line_holds_the_managed_command_until_enter() {
        let gate = Arc::new(Mutex::new(InputGate::default()));
        gate.lock().unwrap().line.feed(b"say hel");
        let (managed, receiver) = managed(&gate);
        let typist = gate.clone();
        let user = thread::spawn(move || {
            thread::sleep(Duration::from_millis(120));
            assert!(receiver.try_recv().is_err(), "managed bytes joined the partial line");
            typist.lock().unwrap().line.feed(b"lo\r");
            let InputMessage::Managed(bytes, acknowledge) = receiver.recv().unwrap() else {
                panic!("unexpected interactive bytes")
            };
            assert_eq!(bytes, b"gamerule keepInventory true\r");
            acknowledge.send(Ok(())).unwrap();
        });
        managed.write_command("gamerule keepInventory true", Instant::now() + Duration::from_secs(2)).unwrap();
        user.join().unwrap();
    }

    #[test]
    fn a_line_left_unfinished_times_out_without_writing() {
        let gate = Arc::new(Mutex::new(InputGate::default()));
        gate.lock().unwrap().line.feed(b"op Ste");
        let (managed, receiver) = managed(&gate);
        assert_eq!(
            managed.write_command("gamerule keepInventory true", Instant::now() + Duration::from_millis(150)).unwrap_err(),
            "ConsoleManagedInputBusy"
        );
        assert!(receiver.try_recv().is_err());
        // Nothing was written, so the run keeps accepting managed commands.
        assert!(!gate.lock().unwrap().aborted);
    }

    #[test]
    fn managed_command_is_a_single_write_and_requires_acknowledgement() {
        let gate = Arc::new(Mutex::new(InputGate::default()));
        let (managed, receiver) = managed(&gate);
        let writer = thread::spawn(move || {
            let InputMessage::Managed(bytes, acknowledge) = receiver.recv().unwrap() else {
                panic!("unexpected interactive bytes")
            };
            assert_eq!(bytes, b"gamerule doDaylightCycle false\r");
            acknowledge.send(Ok(())).unwrap();
            assert!(receiver.try_recv().is_err());
        });
        managed.write_command("gamerule doDaylightCycle false", Instant::now() + Duration::from_secs(1)).unwrap();
        writer.join().unwrap();
        assert_eq!(managed.write_command("gamerule x\nstop", Instant::now() + Duration::from_secs(1)).unwrap_err(), "InvalidConsoleManagedCommand");
        gate.lock().unwrap().aborted = true;
        assert_eq!(managed.write_command("gamerule doDaylightCycle true", Instant::now() + Duration::from_secs(1)).unwrap_err(), "ConsoleManagedAborted");
        drop(managed);
        assert!(!gate.lock().unwrap().managed);
    }

    #[test]
    fn lost_acknowledgement_is_unconfirmed_and_input_is_not_replayed() {
        let gate = Arc::new(Mutex::new(InputGate::default()));
        let (managed, receiver) = managed(&gate);
        let writer = thread::spawn(move || {
            let InputMessage::Managed(_, reply) = receiver.recv().unwrap() else {
                panic!("unexpected interactive bytes")
            };
            drop(reply);
            assert!(receiver.try_recv().is_err());
        });
        assert_eq!(managed.write_command("gamerule doDaylightCycle false", Instant::now() + Duration::from_secs(1)).unwrap_err(), "ConsoleManagedWriteUnconfirmed");
        writer.join().unwrap();
        drop(managed);
        assert!(gate.lock().unwrap().aborted);
        assert_eq!(gate.lock().unwrap().reserve().unwrap_err(), "ConsoleManagedInputUnavailable");
    }

    #[test]
    fn accepted_stop_blocks_the_next_managed_command_and_pending_ack_is_not_success() {
        let gate = Arc::new(Mutex::new(InputGate::default()));
        let (managed, receiver) = managed(&gate);
        let stop_gate = gate.clone();
        let writer = thread::spawn(move || {
            let InputMessage::Managed(_, acknowledge) = receiver.recv().unwrap() else {
                panic!("unexpected interactive bytes")
            };
            stop_gate.lock().unwrap().aborted = true;
            acknowledge.send(Ok(())).unwrap();
            assert!(receiver.try_recv().is_err());
        });
        assert_eq!(
            managed
                .write_command("gamerule doDaylightCycle false", Instant::now() + Duration::from_secs(1))
                .unwrap_err(),
            "ConsoleManagedAborted"
        );
        writer.join().unwrap();
        assert_eq!(
            managed
                .write_command("gamerule doDaylightCycle true", Instant::now() + Duration::from_secs(1))
                .unwrap_err(),
            "ConsoleManagedAborted"
        );
    }
}

#[cfg(test)]
mod line_tests {
    use super::*;

    #[test]
    fn typed_lines_are_read_back_with_backspace() {
        let mut line = LineTracker::default();
        assert!(line.is_empty());
        assert!(line.feed("stoq\u{7f}p".as_bytes()).is_empty());
        assert!(!line.is_empty());
        assert_eq!(line.feed(b"\r"), [Submitted::Line("stop".into())]);
        assert!(line.is_empty());
        assert_eq!(line.feed("say 你好\r".as_bytes()), [Submitted::Line("say 你好".into())]);
    }

    #[test]
    fn history_and_cursor_keys_make_the_line_unverified_until_enter() {
        let mut line = LineTracker::default();
        line.feed(b"\x1b[A");
        assert!(!line.is_empty());
        line.feed(b"\x7f\x7f");
        assert!(!line.is_empty(), "backspace cannot prove an unknown line is empty");
        assert_eq!(line.feed(b"\r"), [Submitted::Unverified]);
        assert!(line.is_empty());
        line.feed(b"li\x1bOD\x1b]0;t\x07");
        assert_eq!(line.feed(b"\r"), [Submitted::Unverified]);
        line.feed(b"\t");
        assert_eq!(line.feed(b"\r"), [Submitted::Unverified]);
    }

    #[test]
    fn interrupts_and_pasted_lines() {
        let mut line = LineTracker::default();
        assert_eq!(line.feed(b"ab\x03"), [Submitted::Interrupt]);
        assert!(line.is_empty());
        assert_eq!(line.feed(b"\x04"), [Submitted::Interrupt]);
        assert_eq!(line.feed(b"x\x04"), Vec::<Submitted>::new());
        assert!(!line.is_empty());
        let mut line = LineTracker::default();
        assert_eq!(
            line.feed(b"list\r\nsay hi\r"),
            [Submitted::Line("list".into()), Submitted::Line(String::new()), Submitted::Line("say hi".into())]
        );
    }

    #[test]
    fn stop_commands_for_paper_and_velocity() {
        for line in ["stop", " STOP ", "/stop", "minecraft:stop", "restart", "shutdown", "end", "velocity:shutdown now"] {
            assert!(stops_server(line), "{line}");
        }
        for line in ["", "list", "stopwatch", "say stop", "gamerule stop"] {
            assert!(!stops_server(line), "{line}");
        }
    }

    #[test]
    fn typed_stop_and_unverified_lines_leave_markers_for_auto_restart() {
        let folder = std::env::temp_dir().join(format!("ih-console-markers-{}-{}", std::process::id(), clock()));
        fs::create_dir(&folder).unwrap();
        record_submissions(&folder, &[Submitted::Line("list".into())]);
        assert!(!folder.join(STOP_TYPED).exists() && !folder.join(LINE_UNVERIFIED).exists());
        record_submissions(&folder, &[Submitted::Unverified]);
        assert!(folder.join(LINE_UNVERIFIED).is_file() && !folder.join(STOP_TYPED).exists());
        record_submissions(&folder, &[Submitted::Line("minecraft:stop".into())]);
        assert!(folder.join(STOP_TYPED).is_file());
        fs::remove_dir_all(&folder).unwrap();
    }
}

#[cfg(test)]
mod tier_tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            paths: Ok(()),
            bundled: Ok(()),
            system: Ok(()),
            window: Ok(()),
            window_confirmed: false,
            limit: None,
        }
    }

    fn reasons(choice: &Choice) -> Vec<(Tier, &str)> {
        choice.skipped.iter().map(|item| (item.tier, item.reason.as_str())).collect()
    }

    #[test]
    fn bundled_runtime_is_preferred_then_windows_then_a_confirmed_window_then_plain() {
        let mut facts = facts();
        assert_eq!(choose(&facts), Choice { tier: Tier::Bundled, skipped: vec![] });
        facts.bundled = Err("ConsoleRuntimeMissing".into());
        let choice = choose(&facts);
        assert_eq!(choice.tier, Tier::System);
        assert_eq!(reasons(&choice), [(Tier::Bundled, "ConsoleRuntimeMissing")]);
        facts.system = Err("ConsoleSystemUnavailable".into());
        let choice = choose(&facts);
        assert_eq!(choice.tier, Tier::Plain);
        assert_eq!(choice.skipped[2], Skipped { tier: Tier::Window, reason: "ConsoleWindowNotConfirmed".into() });
        facts.window_confirmed = true;
        assert_eq!(choose(&facts).tier, Tier::Window);
    }

    #[test]
    fn only_long_paths_or_missing_tiers_run_without_a_console() {
        let mut facts = facts();
        facts.window_confirmed = true;
        facts.paths = Err("ConsolePathUnsupported".into());
        let choice = choose(&facts);
        assert_eq!(choice.tier, Tier::Plain);
        assert!(choice.skipped.iter().all(|item| item.reason == "ConsolePathUnsupported"));
        // No core family blocks a tier: the facts no longer carry one.
        facts.paths = Ok(());
        assert_eq!(choose(&facts), Choice { tier: Tier::Bundled, skipped: vec![] });
    }

    #[test]
    fn a_development_limit_skips_better_tiers() {
        let mut facts = facts();
        facts.window_confirmed = true;
        for (limit, expected) in [(Tier::System, Tier::System), (Tier::Window, Tier::Window), (Tier::Plain, Tier::Plain)] {
            facts.limit = Some(limit);
            let choice = choose(&facts);
            assert_eq!(choice.tier, expected);
            assert!(choice.skipped.iter().all(|item| item.reason == "ConsoleTierLimited"));
        }
    }

    #[test]
    fn this_windows_provides_a_pseudoconsole() {
        system_runtime().unwrap();
    }

    #[test]
    fn runtime_pins_match_the_distribution_manifest() {
        let manifest: Value = serde_json::from_str(include_str!("../../packaging/console-runtime.json")).unwrap();
        for (name, hash) in RUNTIME_FILES {
            assert_eq!(manifest["files"][name]["sha256"], hash, "{name}");
        }
        assert_eq!(manifest["files"].as_object().unwrap().len(), RUNTIME_FILES.len());
    }

    #[test]
    fn short_local_server_jars_are_passed_by_relative_name() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let java = std::env::current_exe().unwrap();
        let command = command_line(&java, &dir, &dir.join("server.jar"), &["-Xmx1G".into()], true).unwrap();
        let text = String::from_utf16_lossy(&command[..command.len() - 1]);
        assert!(text.ends_with(" -Xmx1G -jar \"server.jar\" nogui"), "{text}");
    }
}

#[cfg(test)]
mod viewport_tests {
    use super::*;

    #[test]
    fn replay_reads_do_not_cross_grid_boundaries() {
        let wide = Grid { cols: 180, rows: 48 };
        let grids = [(0, INITIAL_GRID), (17, wide), (39, INITIAL_GRID)];
        let (grid, limit) = grid_at(&grids, 13, 8192);
        assert!(grid == INITIAL_GRID);
        assert_eq!(limit, 4);
        let (grid, limit) = grid_at(&grids, 17, 8192);
        assert!(grid == wide);
        assert_eq!(limit, 22);
        let (grid, limit) = grid_at(&grids, 39, 8192);
        assert!(grid == INITIAL_GRID);
        assert_eq!(limit, 8192);
    }

    #[test]
    fn dimensions_are_bounded_before_converting_to_win32_coordinates() {
        assert!(Grid { cols: 20, rows: 10 }.valid());
        assert!(Grid { cols: 500, rows: 200 }.valid());
        assert!(!Grid { cols: 501, rows: 200 }.valid());
        assert!(!Grid { cols: 20, rows: 0 }.valid());
    }
}
