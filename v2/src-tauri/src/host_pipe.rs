use super::{alive, identity, process_image, Identity};
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::PathBuf,
    sync::mpsc::SyncSender,
    thread,
    time::{Duration, Instant},
};
type Handle = *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        access: u32,
        mode: u32,
        instances: u32,
        out_size: u32,
        in_size: u32,
        timeout: u32,
        security: Handle,
    ) -> Handle;
    fn ConnectNamedPipe(pipe: Handle, overlapped: Handle) -> i32;
    fn DisconnectNamedPipe(pipe: Handle) -> i32;
    fn GetNamedPipeClientProcessId(pipe: Handle, pid: *mut u32) -> i32;
    fn GetNamedPipeServerProcessId(pipe: Handle, pid: *mut u32) -> i32;
    fn LocalFree(memory: Handle) -> Handle;
    fn CreateEventW(
        attributes: Handle,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> Handle;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn ReadFile(
        handle: Handle,
        buffer: *mut u8,
        count: u32,
        bytes_read: *mut u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn WriteFile(
        handle: Handle,
        buffer: *const u8,
        count: u32,
        bytes_written: *mut u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn CancelIoEx(handle: Handle, overlapped: *mut Overlapped) -> i32;
    fn GetOverlappedResult(
        handle: Handle,
        overlapped: *mut Overlapped,
        transferred: *mut u32,
        wait: i32,
    ) -> i32;
    fn PeekNamedPipe(
        pipe: Handle,
        buffer: *mut u8,
        size: u32,
        read: *mut u32,
        available: *mut u32,
        remaining: *mut u32,
    ) -> i32;
    fn GetCurrentProcess() -> Handle;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut Handle,
        size: *mut u32,
    ) -> i32;
}

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: Handle,
    inherit: i32,
}

#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: [u32; 2],
    event: Handle,
}

const FRAME_LIMIT: usize = 1024;
const CONNECTION_IO_BUDGET: Duration = Duration::from_millis(450);

fn pipe_io(
    pipe: Handle,
    deadline: Option<Instant>,
    start: impl FnOnce(*mut Overlapped) -> i32,
) -> io::Result<u32> {
    let event = unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, std::ptr::null()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    let _event = unsafe { OwnedHandle::from_raw_handle(event) };
    let mut operation = Overlapped {
        internal: 0,
        internal_high: 0,
        offset: [0; 2],
        event,
    };
    let mut transferred = 0;
    if start(&mut operation) == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(997) {
            // ERROR_IO_PENDING
            return Err(error);
        }
        let timeout = deadline.map_or(u32::MAX, |end| {
            end.saturating_duration_since(Instant::now()).as_millis() as u32
        });
        let wait = unsafe { WaitForSingleObject(event, timeout) };
        if wait != 0 {
            let error = if wait == 258 {
                io::Error::new(io::ErrorKind::TimedOut, "Pipe I/O deadline exceeded")
            } else {
                io::Error::last_os_error()
            };
            // The event, buffer and OVERLAPPED must outlive even a cancellation race.
            unsafe {
                CancelIoEx(pipe, &mut operation);
                GetOverlappedResult(pipe, &mut operation, &mut transferred, 1);
            }
            return Err(error);
        }
    }
    if unsafe { GetOverlappedResult(pipe, &mut operation, &mut transferred, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(transferred)
}

fn read_frame(pipe: Handle, limit: usize, deadline: Instant) -> io::Result<String> {
    let mut frame = vec![0; limit];
    let mut used = 0;
    while used < frame.len() && Instant::now() < deadline {
        let remaining = &mut frame[used..];
        let count = pipe_io(pipe, Some(deadline), |operation| unsafe {
            ReadFile(
                pipe,
                remaining.as_mut_ptr(),
                remaining.len() as u32,
                std::ptr::null_mut(),
                operation,
            )
        })? as usize;
        if count == 0 {
            break;
        }
        used += count;
        if let Some(end) = frame[..used].iter().position(|byte| *byte == b'\n') {
            if end + 1 != used {
                break;
            }
            return std::str::from_utf8(&frame[..end])
                .map(|text| text.trim_end_matches('\r').to_owned())
                .map_err(io::Error::other);
        }
    }
    Err(io::Error::other("Incomplete or invalid pipe frame"))
}

fn send_reply(pipe: Handle, response: &str) -> io::Result<()> {
    let deadline = Instant::now() + CONNECTION_IO_BUDGET;
    let frame = format!("{response}\n");
    let count = pipe_io(pipe, Some(deadline), |operation| unsafe {
        WriteFile(
            pipe,
            frame.as_ptr(),
            frame.len() as u32,
            std::ptr::null_mut(),
            operation,
        )
    })?;
    if count as usize != frame.len() {
        return Err(io::Error::other("Incomplete pipe reply"));
    }
    // A cooperative client closes after reading. Bound the grace period for unread replies.
    let mut discarded = [0; FRAME_LIMIT];
    let mut used = 0;
    while used < discarded.len() && Instant::now() < deadline {
        let tail = &mut discarded[used..];
        match pipe_io(pipe, Some(deadline), |operation| unsafe {
            ReadFile(
                pipe,
                tail.as_mut_ptr(),
                tail.len() as u32,
                std::ptr::null_mut(),
                operation,
            )
        }) {
            Ok(count) if count > 0 => used += count as usize,
            _ => break,
        }
    }
    Ok(())
}

fn serve(
    name: &str,
    sid: Option<&str>,
    limit: usize,
    ready: impl FnOnce() -> io::Result<()>,
    mut respond: impl FnMut(&str, &File) -> String,
) -> io::Result<()> {
    let wide: Vec<_> = name.encode_utf16().chain([0]).collect();
    let mut descriptor = std::ptr::null_mut();
    if let Some(sid) = sid {
        if !sid.starts_with("S-1-") || !sid[4..].bytes().all(|b| b.is_ascii_digit() || b == b'-') {
            return Err(io::Error::other("Invalid SID"));
        }
        let sddl: Vec<_> = format!("D:P(A;;0x0012019b;;;{sid})")
            .encode_utf16()
            .chain([0])
            .collect();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    let mut security = SecurityAttributes {
        length: size_of::<SecurityAttributes>() as u32,
        descriptor,
        inherit: 0,
    };
    // Duplex, first instance only, byte mode, reject remote clients.
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            3 | 0x80000 | 0x40000000,
            8,
            1,
            4096,
            4096,
            0,
            if descriptor.is_null() {
                std::ptr::null_mut()
            } else {
                (&mut security as *mut SecurityAttributes).cast()
            },
        )
    };
    let error = io::Error::last_os_error();
    if !descriptor.is_null() {
        unsafe { LocalFree(descriptor) };
    }
    if handle as isize == -1 {
        return Err(error);
    }
    let pipe = unsafe { File::from_raw_handle(handle) };
    ready()?;
    loop {
        if let Err(error) = pipe_io(handle, None, |operation| unsafe {
            ConnectNamedPipe(handle, operation.cast())
        }) {
            if error.raw_os_error() != Some(535) {
                return Err(error);
            }
        }
        if let Ok(request) = read_frame(handle, limit, Instant::now() + CONNECTION_IO_BUDGET) {
            let _ = send_reply(handle, &respond(&request, &pipe));
        }
        if unsafe { DisconnectNamedPipe(handle) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(233) {
                return Err(error);
            }
        }
    }
}

fn read_line(pipe: &File) -> io::Result<String> {
    use std::io::Read;
    let mut line = String::new();
    BufReader::new(pipe.take(FRAME_LIMIT as u64)).read_line(&mut line)?;
    if !line.ends_with('\n') {
        return Err(io::Error::other("Incomplete pipe message"));
    }
    Ok(line.trim_end().to_owned())
}

fn upstream(name: &str, request: &str, server: &Identity) -> String {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    // Individual read/write rights without FILE_CREATE_PIPE_INSTANCE, anonymous SQOS.
    options
        .access_mode(0x0012019b)
        .security_qos_flags(0x00100000);
    // The previous exchange may have replied before disconnecting. Retry only
    // the open, before any request bytes can reach the Host.
    let connect_deadline = Instant::now() + Duration::from_millis(250);
    let mut pipe = loop {
        match options.open(name) {
            Ok(pipe) => break pipe,
            Err(error) if error.raw_os_error() == Some(231) && Instant::now() < connect_deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.raw_os_error() == Some(231) => return "Timeout".into(),
            Err(_) => return "HostUnavailable".into(),
        }
    };
    let mut pid = 0;
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } == 0
        || pid != server.pid
        || !alive(server).unwrap_or(false)
    {
        return "PeerMismatch".into();
    }
    if writeln!(pipe, "{request}").is_err() {
        return "HostUnavailable".into();
    }
    // An incomplete upstream frame must not hold the sole worker indefinitely.
    let deadline = Instant::now() + Duration::from_millis(800);
    let mut bytes = [0u8; FRAME_LIMIT];
    while Instant::now() < deadline {
        let mut read = 0;
        let ok = unsafe {
            PeekNamedPipe(
                pipe.as_raw_handle(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                &mut read,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return "HostUnavailable".into();
        }
        if bytes[..read as usize].contains(&b'\n') {
            return read_line(&pipe).unwrap_or_else(|_| "HostUnavailable".into());
        }
        if read as usize == bytes.len() {
            return "InvalidReply".into();
        }
        thread::sleep(Duration::from_millis(10));
    }
    "Timeout".into()
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        buffer: Handle,
        length: u32,
        size: *mut u32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: Handle, output: *mut *mut u16) -> i32;
}

pub(super) fn current_sid() -> io::Result<String> {
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut size = 0;
    unsafe {
        GetTokenInformation(token, 1, std::ptr::null_mut(), 0, &mut size);
    }
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut data = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    if unsafe { GetTokenInformation(token, 1, data.as_mut_ptr().cast(), size, &mut size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(data[0] as Handle, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut length = 0;
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    let result = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) });
    unsafe {
        LocalFree(text.cast());
    }
    result.map_err(io::Error::other)
}

fn endpoint(run: &str) -> String {
    format!(r"\\.\pipe\instanceharbor-v2-{run}")
}

pub(super) fn host_control(
    run: String,
    expected: [PathBuf; 2],
    recovery: Option<(PathBuf, std::sync::Arc<std::sync::Mutex<super::host_recovery::Gate>>)>,
    stop: SyncSender<()>,
    force: SyncSender<()>,
    batch_stop: SyncSender<super::BatchStopIntent>,
    world: SyncSender<super::world_live::Command>,
) -> io::Result<()> {
    let sid = current_sid()?;
    serve(
        &endpoint(&run),
        Some(&sid),
        FRAME_LIMIT,
        || Ok(()),
        |request, pipe| {
            let mut pid = 0;
            if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid) } == 0 {
                return "PeerMismatch".into();
            }
            let Ok(Some(peer)) = identity(pid) else {
                return "PeerMismatch".into();
            };
            if !alive(&peer).unwrap_or(false) {
                return "PeerMismatch".into();
            }
            if let Some((root, gate)) = &recovery {
                let Ok(mut gate) = gate.lock() else { return "HostRecoveryUnconfirmed".into() };
                if request.starts_with("Rebind ")
                    || request.starts_with("RecoverChallenge ")
                    || request.starts_with("RecoverProof ")
                {
                    return gate.request(root, &peer, request).unwrap_or_else(|error| error);
                }
                if !gate.authorizes(&peer) {
                    return "HostManagerRequired".into();
                }
            } else {
                let image = process_image(pid).and_then(|path| path.canonicalize().ok());
                if !image.is_some_and(|path| expected.contains(&path)) {
                    return "PeerMismatch".into();
                }
            }
            if request == format!("Probe {run}") {
                return "RunVerified".into();
            }
            if let Some(arguments) = request.strip_prefix("World ") {
                let Some((target, line)) = arguments.split_once(' ') else {
                    return "InvalidWorldCommand".into();
                };
                if target != run {
                    return "RunMismatch".into();
                }
                return super::world_live::submit(&world, line).into();
            }
            if let ["BatchStop", target, id, deadline] = request
                .split_ascii_whitespace()
                .collect::<Vec<_>>()
                .as_slice()
            {
                if *target != run || !super::token(id) {
                    return "RunMismatch".into();
                }
                let Ok(deadline) = deadline.parse() else {
                    return "InvalidStopIntent".into();
                };
                let intent = super::BatchStopIntent {
                    request: (*id).into(),
                    deadline,
                };
                return match batch_stop.try_send(intent) {
                    Ok(()) => "StopSubmitted".into(),
                    Err(_) => "StopSubmissionUnconfirmed".into(),
                };
            }
            let channel = if request == format!("Stop {run}") {
                &stop
            } else if request == format!("ForceStop {run}") {
                &force
            } else {
                return "RunMismatch".into();
            };
            match channel.try_send(()) {
                Ok(()) | Err(std::sync::mpsc::TrySendError::Full(())) => "StopAccepted".into(),
                Err(std::sync::mpsc::TrySendError::Disconnected(())) => "HostUnavailable".into(),
            }
        },
    )
}

// Used by the already-authenticated resident Manager after it binds to this run.
pub(super) fn control_as_manager(run: &super::Run, request: &str) -> String {
    let first = upstream(&endpoint(&run.id), request, &run.host);
    if first != "HostManagerRequired" {
        return first;
    }
    let rebind = upstream(&endpoint(&run.id), &format!("Rebind {}", run.id), &run.host);
    if rebind == "RunVerified" {
        upstream(&endpoint(&run.id), request, &run.host)
    } else {
        rebind
    }
}

pub(super) fn recover(root: &std::path::Path, run: &super::Run) -> super::Result<()> {
    let address = endpoint(&run.id);
    let payload = upstream(&address, &format!("RecoverChallenge {}", run.id), &run.host);
    let (nonce, signature) = super::host_recovery::approve(root, run, &payload)?;
    let proof = format!("RecoverProof {} {nonce} {signature}", run.id);
    let response = upstream(&address, &proof, &run.host);
    if response != "RunRecovered" {
        return Err(response);
    }
    let probe = format!("Probe {}", run.id);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match upstream(&address, &probe, &run.host).as_str() {
            "RunVerified" => return Ok(()),
            "Timeout" if Instant::now() < deadline => thread::sleep(Duration::from_millis(30)),
            _ => return Err("HostRecoveryUnconfirmed".into()),
        }
    }
}

fn dispatch_control(
    root: &std::path::Path,
    name: &str,
    run: &super::Run,
    request: &str,
) -> super::Result<String> {
    let first = upstream(&endpoint(&run.id), request, &run.host);
    if first != "HostManagerRequired" {
        return Ok(first);
    }
    let reply = super::resident_manager::client(
        root,
        "host-control",
        &[name.into(), run.id.clone(), request.into()],
    )?;
    reply.as_str().map(str::to_owned).ok_or_else(|| "HostUnconfirmed".into())
}

pub(super) fn request_world_command(
    root: &std::path::Path, name: &str, run: &super::Run, line: &str
) -> super::Result<()> {
    let response = dispatch_control(root, name, run, &format!("World {} {line}", run.id))?;
    if response == "WorldCommandWritten" { Ok(()) } else { Err(response) }
}

pub(super) fn request_stop(
    root: &std::path::Path, name: &str, run: &super::Run, force: bool
) -> super::Result<()> {
    let action = if force { "ForceStop" } else { "Stop" };
    let response = dispatch_control(root, name, run, &format!("{action} {}", run.id))?;
    if response == "StopAccepted" { Ok(()) } else { Err(response) }
}

pub(super) fn submit_batch_stop(
    run: &super::Run,
    intent: &super::BatchStopIntent,
) -> super::Result<()> {
    let response = control_as_manager(run, &format!("BatchStop {} {} {}", run.id, intent.request, intent.deadline));
    if response == "StopSubmitted" { Ok(()) } else { Err(response) }
}

pub(super) fn verify_run(run: &super::Run) -> super::Result<()> {
    match control_as_manager(run, &format!("Probe {}", run.id)).as_str() {
        "RunVerified" => Ok(()),
        "Timeout" => Err("HostProbeTimeout".into()),
        "HostUnavailable" => Err("HostPipeUnavailable".into()),
        _ => Err("HostUnconfirmed".into()),
    }
}

pub(super) const MANAGER_FRAME_LIMIT: usize = 128 * 1024;

pub(super) fn serve_local(
    name: &str,
    expected: &[PathBuf],
    ready: impl FnOnce() -> io::Result<()>,
    respond: impl Fn(&str) -> String,
) -> io::Result<()> {
    serve_local_with(
        name,
        |_, image| expected.iter().any(|path| path == image),
        ready,
        respond,
    )
}

pub(super) fn serve_local_with(
    name: &str,
    authorized: impl Fn(&Identity, &std::path::Path) -> bool,
    ready: impl FnOnce() -> io::Result<()>,
    respond: impl Fn(&str) -> String,
) -> io::Result<()> {
    serve(
        name,
        Some(&current_sid()?),
        MANAGER_FRAME_LIMIT,
        ready,
        |request, pipe| {
            let mut pid = 0;
            let identified =
                unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid) } != 0;
            let verified = identified
                && identity(pid).ok().flatten().is_some_and(|peer| {
                    process_image(pid)
                        .and_then(|path| path.canonicalize().ok())
                        .is_some_and(|path| authorized(&peer, &path))
                        && alive(&peer).unwrap_or(false)
                });
            if verified {
                respond(request)
            } else {
                r#"{"error":"PeerMismatch"}"#.into()
            }
        },
    )
}

pub(super) fn local_request(name: &str, peer: &Identity, request: &str) -> super::Result<String> {
    local_request_matching(name, request, |pid| Ok(pid == peer.pid && alive(peer)?))
        .map(|(_, response)| response)
}

pub(super) fn local_request_executable(
    name: &str,
    executable: &std::path::Path,
    request: &str,
) -> super::Result<(u32, String)> {
    local_request_matching(name, request, |pid| {
        let Some(peer) = identity(pid).map_err(super::fail)? else {
            return Ok(false);
        };
        Ok(process_image(pid)
            .and_then(|image| image.canonicalize().ok())
            .as_deref()
            == Some(executable)
            && alive(&peer)?)
    })
}

pub(super) fn legacy_status(
    name: &str,
    verify_peer: impl FnOnce(u32) -> super::Result<bool>,
) -> super::Result<(u32, String)> {
    local_request_matching(name, "status", verify_peer)
}

fn local_request_matching(
    name: &str,
    request: &str,
    verify_peer: impl FnOnce(u32) -> super::Result<bool>,
) -> super::Result<(u32, String)> {
    if request.contains('\n') || request.len() + 1 > MANAGER_FRAME_LIMIT {
        return Err("ManagerFrameTooLarge".into());
    }
    let pipe = OpenOptions::new()
        .access_mode(0x0012019b)
        .security_qos_flags(0x00100000)
        .custom_flags(0x40000000)
        .open(name)
        .map_err(|_| "ManagerPipeUnavailable")?;
    let handle = pipe.as_raw_handle();
    let mut pid = 0;
    if unsafe { GetNamedPipeServerProcessId(handle, &mut pid) } == 0 || !verify_peer(pid)? {
        return Err("PeerMismatch".into());
    }
    let frame = format!("{request}\n");
    let deadline = Instant::now() + Duration::from_secs(3);
    let written = pipe_io(handle, Some(deadline), |operation| unsafe {
        WriteFile(
            handle,
            frame.as_ptr(),
            frame.len() as u32,
            std::ptr::null_mut(),
            operation,
        )
    })
    .map_err(|_| "ManagerResultUnconfirmed")?;
    if written as usize != frame.len() {
        return Err("ManagerResultUnconfirmed".into());
    }
    read_frame(handle, MANAGER_FRAME_LIMIT, deadline)
        .map(|response| (pid, response))
        .map_err(|_| "ManagerResultUnconfirmed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn local_pipe_recovers_from_partial_disconnect_and_held_connection() {
        let name = format!(
            r"\\.\pipe\ih-manager-test-{}-{}",
            std::process::id(),
            super::super::clock()
        );
        let server_name = name.clone();
        let image = std::env::current_exe().unwrap().canonicalize().unwrap();
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = serve_local(
                &server_name,
                &[image],
                || send.send(()).map_err(io::Error::other),
                |request| format!("echo {request}"),
            );
        });
        receive.recv_timeout(Duration::from_secs(2)).unwrap();
        let peer = identity(std::process::id()).unwrap().unwrap();
        let connect = || {
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                if let Ok(pipe) = OpenOptions::new()
                    .access_mode(0x0012019b)
                    .security_qos_flags(0x00100000)
                    .open(&name)
                {
                    return pipe;
                }
                assert!(Instant::now() < until, "isolated pipe unavailable");
                thread::sleep(Duration::from_millis(20));
            }
        };
        let echo = || {
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                match local_request(&name, &peer, "status") {
                    Ok(reply) => {
                        assert_eq!(reply, "echo status");
                        return;
                    }
                    Err(error) if error == "ManagerPipeUnavailable" && Instant::now() < until => {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Err(error) => panic!("healthy request did not recover: {error}"),
                }
            }
        };
        let mut partial = connect();
        partial.write_all(b"part").unwrap();
        drop(partial);
        echo();
        let mut stalled = connect();
        stalled.write_all(b"part").unwrap();
        echo();
        drop(stalled);
    }
}
