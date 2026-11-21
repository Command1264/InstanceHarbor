use super::*;
use std::os::windows::fs::FileExt;
use std::os::windows::{
    ffi::OsStrExt,
    io::{FromRawHandle, OwnedHandle},
};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

enum InputMessage {
    Interactive(Vec<u8>),
    Managed(Vec<u8>, mpsc::SyncSender<Result<()>>),
}

#[derive(Default)]
struct InputGate {
    interactive_started: bool,
    managed: bool,
    aborted: bool,
}

impl InputGate {
    fn reserve(&mut self) -> Result<()> {
        if self.interactive_started || self.managed || self.aborted {
            return Err("ConsoleManagedInputUnavailable".into());
        }
        self.managed = true;
        Ok(())
    }

    fn accept_interactive(&mut self) -> Result<()> {
        if self.managed || self.aborted {
            return Err("ConsoleManagedInputBusy".into());
        }
        self.interactive_started = true;
        Ok(())
    }
}

/// The Host keeps this guard until the corresponding output has been read back.
/// A lost write acknowledgement leaves the operation unconfirmed, never retried.
pub(super) struct ManagedInput {
    gate: Arc<Mutex<InputGate>>,
    input: mpsc::SyncSender<InputMessage>,
}

impl ManagedInput {
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
        {
            let gate = self.gate.lock().map_err(|_| "ConsoleInputUnavailable")?;
            if !gate.managed || gate.aborted {
                return Err("ConsoleManagedAborted".into());
            }
            self.input
                .try_send(InputMessage::Managed(command, sent))
                .map_err(|_| "ConsoleInputUnavailable")?;
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
    fn GetProcAddress(module: Handle, name: *const u8) -> Handle;
    fn WaitForSingleObject(process: Handle, millis: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
}

const DLL_HASH: &str = "feeef341d891643c62d30b6b07800bc70f0bc148f44cb8c3bee84aa557ae805a";
const CONSOLE_HASH: &str = "3d66b23d0a71bb8eed2b77edc8b9df9bf54ce6c8fb74c863a30e760997f80586";

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain([0]).collect()
}
fn launcher_path(path: &Path) -> Result<PathBuf> {
    // HotSpot uses the process image path to locate lib/modules. For Java
    // in ConPTY, pass the same ordinary path that Java's Windows launcher expects.
    let ordinary = PathBuf::from(path_format::display(path).replace('/', "\\"));
    if ordinary.as_os_str().encode_wide().count() >= 260
        || fs::canonicalize(&ordinary).map_err(fail)? != fs::canonicalize(path).map_err(fail)?
    {
        return Err("ConsolePathUnsupported".into());
    }
    Ok(ordinary)
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
    _library: Library,
}
// Closing an owned pseudoconsole is independent of the thread that opened it.
unsafe impl Send for PseudoConsole {}

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        unsafe { (self.close)(self.handle) };
    }
}
impl PseudoConsole {
    fn open(input: &OwnedHandle, output: &OwnedHandle) -> Result<Self> {
        let dir = std::env::current_exe()
            .and_then(|path| path.canonicalize())
            .map_err(fail)?;
        let dll = dir.with_file_name("conpty.dll");
        for (path, expected) in [
            (&dll, DLL_HASH),
            (&dir.with_file_name("OpenConsole.exe"), CONSOLE_HASH),
        ] {
            no_redirect(path)?;
            if content_revision::revision(&fs::read(path).map_err(fail)?)? != expected {
                return Err("ConsoleRuntimeMismatch".into());
            }
        }
        // DLL load directory and System32 are the only dependency lookup locations.
        let module =
            unsafe { LoadLibraryExW(wide(dll.as_os_str()).as_ptr(), std::ptr::null_mut(), 0x900) };
        if module.is_null() {
            return Err(system_error());
        }
        let library = Library(module);
        let create =
            unsafe { GetProcAddress(module, c"ConptyCreatePseudoConsole".as_ptr().cast()) };
        let close = unsafe { GetProcAddress(module, c"ConptyClosePseudoConsole".as_ptr().cast()) };
        // Use the same pinned provider for creating and resizing its handle.
        let resize = unsafe { GetProcAddress(module, c"ConptyResizePseudoConsole".as_ptr().cast()) };
        if create.is_null() || close.is_null() || resize.is_null() {
            return Err("ConsoleRuntimeMismatch".into());
        }
        let create: Create = unsafe { std::mem::transmute(create) };
        let close: Close = unsafe { std::mem::transmute(close) };
        let mut handle = std::ptr::null_mut();
        let result = unsafe {
            create(
                Coord { x: INITIAL_GRID.cols as i16, y: INITIAL_GRID.rows as i16 },
                input.as_raw_handle(),
                output.as_raw_handle(),
                8,
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
        let java = launcher_path(java)?;
        let directory = launcher_path(directory)?;
        let application = wide(java.as_os_str());
        let mut command = vec![b'"' as u16];
        command.extend(java.as_os_str().encode_wide());
        command.push(b'"' as u16);
        // The caller constructs these arguments from numbers and a GC enum.
        for argument in jvm {
            command.push(b' ' as u16);
            command.extend(argument.encode_utf16());
        }
        let jar = launcher_path(jar)?;
        command.extend(" -jar \"".encode_utf16());
        command.extend(jar.as_os_str().encode_wide());
        command.push(b'"' as u16);
        if nogui {
            command.extend(" nogui".encode_utf16());
        }
        command.push(0);
        let directory = wide(directory.as_os_str());
        let mut startup: ExtendedStartup = unsafe { std::mem::zeroed() };
        startup.base.size = size_of::<ExtendedStartup>() as u32;
        startup.attributes = attributes;
        let mut output: ProcessInfo = unsafe { std::mem::zeroed() };
        if unsafe {
            CreateProcessW(
                application.as_ptr(),
                command.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                0x00080000,
                std::ptr::null_mut(),
                directory.as_ptr(),
                &mut startup.base,
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
}

pub(super) struct Console {
    process: OwnedHandle,
    pid: u32,
    input: mpsc::SyncSender<InputMessage>,
    input_gate: Arc<Mutex<InputGate>>,
    output: Option<mpsc::Receiver<Result<()>>>,
    console: Arc<Mutex<Option<PseudoConsole>>>,
}
impl Console {
    pub(super) fn spawn(java: &Path, directory: &Path, jar: &Path, jvm: &[String], nogui: bool, mut log: File) -> Result<Self> {
        let (input_read, input_write) = pipe()?;
        let (output_read, output_write) = pipe()?;
        let console = PseudoConsole::open(&input_read, &output_write)?;
        let (drained, output) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut reader = File::from(output_read);
            let result = io::copy(&mut reader, &mut log).and_then(|_| log.sync_all()).map_err(fail);
            let _ = drained.send(result);
        });
        let (process, pid) = console.spawn(java, directory, jar, jvm, nogui)?;
        drop(input_read);
        drop(output_write);
        let (input, receiver) = mpsc::sync_channel::<InputMessage>(64);
        thread::spawn(move || {
            let mut writer = File::from(input_write);
            for message in receiver {
                let result = match message {
                    InputMessage::Interactive(bytes) => writer.write_all(&bytes).map_err(fail),
                    InputMessage::Managed(bytes, reply) => {
                        let result = writer.write_all(&bytes).map_err(fail);
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
        Ok(Self {
            process,
            pid,
            input,
            input_gate: Arc::new(Mutex::new(InputGate::default())),
            output: Some(output),
            console: Arc::new(Mutex::new(Some(console))),
        })
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
        let Some(console) = self.console.lock().map_err(fail)?.take() else { return Ok(()) };
        let output = self.output.take().ok_or("ConsoleDrainUnavailable")?;
        thread::spawn(move || drop(console));
        output.recv_timeout(Duration::from_secs(5)).map_err(|_| "ConsoleDrainUnconfirmed")?
    }
    pub(super) fn stop(&self, command: &str) -> Result<()> {
        // JLine: move to the start and clear the pending input before issuing stop.
        self.abort_managed_input()?;
        self.input
            .try_send(InputMessage::Interactive(format!("\x01\x0b{command}\r").into_bytes()))
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
    pub(super) fn serve(
        &self,
        run: &Run,
        log: File,
        recovery: Option<Arc<Mutex<super::host_recovery::Gate>>>,
    ) -> Result<()> {
        let exe = std::env::current_exe().map_err(fail)?;
        let gui = exe
            .with_file_name(crate::desktop_channel::GUI_EXECUTABLE)
            .canonicalize()
            .map_err(fail)?;
        let expected = [exe.canonicalize().map_err(fail)?, gui.clone()];
        let input = self.input.clone();
        let input_gate = Arc::clone(&self.input_gate);
        let console = Arc::clone(&self.console);
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
                        if gate.managed || gate.aborted {
                            return Err("ConsoleManagedInputBusy".into());
                        }
                        input
                            .try_send(InputMessage::Interactive(bytes))
                            .map_err(|_| "ConsoleInputUnavailable")?;
                        gate.accept_interactive()?;
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
    format!(r"\\.\pipe\instanceharbor-v2-console-{run}")
}

pub(super) fn call(root: &Path, name: &str, expected_run: &str, request: Request) -> Result<Value> {
    if !token(name) || !token(expected_run) || !admitted(root, name)? {
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

    #[test]
    fn managed_guard_can_move_to_the_host_worker() {
        fn assert_send<T: Send>() {}
        assert_send::<ManagedInput>();
    }

    #[test]
    fn interactive_bytes_permanently_preclude_managed_input_for_this_run() {
        let mut gate = InputGate::default();
        gate.reserve().unwrap();
        assert_eq!(gate.accept_interactive().unwrap_err(), "ConsoleManagedInputBusy");
        gate.managed = false;
        gate.accept_interactive().unwrap();
        assert_eq!(gate.reserve().unwrap_err(), "ConsoleManagedInputUnavailable");
    }

    #[test]
    fn managed_command_is_a_single_write_and_requires_acknowledgement() {
        let gate = Arc::new(Mutex::new(InputGate::default()));
        gate.lock().unwrap().reserve().unwrap();
        let (input, receiver) = mpsc::sync_channel(1);
        let managed = ManagedInput { gate: gate.clone(), input };
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
        gate.lock().unwrap().reserve().unwrap();
        let (input, receiver) = mpsc::sync_channel(1);
        let managed = ManagedInput { gate: gate.clone(), input };
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
        assert_eq!(gate.lock().unwrap().accept_interactive().unwrap_err(), "ConsoleManagedInputBusy");
    }

    #[test]
    fn accepted_stop_blocks_the_next_managed_command_and_pending_ack_is_not_success() {
        let gate = Arc::new(Mutex::new(InputGate::default()));
        gate.lock().unwrap().reserve().unwrap();
        let (input, receiver) = mpsc::sync_channel(1);
        let managed = ManagedInput { gate: gate.clone(), input };
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
