//! Hold a Windows session mutex from before WebView creation until GUI exit.
use super::process_image;
use std::{
    ffi::c_void,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::PathBuf,
};
type Handle = *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(attributes: Handle, owner: i32, name: *const u16) -> Handle;
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn CreateEventW(attributes: Handle, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn EnumWindows(
        callback: unsafe extern "system" fn(Handle, isize) -> i32,
        context: isize,
    ) -> i32;
    fn GetWindowThreadProcessId(window: Handle, pid: *mut u32) -> u32;
    fn GetWindowTextW(window: Handle, title: *mut u16, capacity: i32) -> i32;
    fn IsIconic(window: Handle) -> i32;
    fn IsWindowVisible(window: Handle) -> i32;
    fn ShowWindowAsync(window: Handle, command: i32) -> i32;
    fn SetForegroundWindow(window: Handle) -> i32;
    fn FlashWindow(window: Handle, invert: i32) -> i32;
}

/// One GUI per channel; the development channel allows one per workspace.
pub fn acquire() -> io::Result<Option<OwnedHandle>> {
    let workspace = crate::desktop_channel::workspace_name();
    acquire_named(&crate::desktop_channel::gui_lock_name(workspace.as_deref()))
}

pub(super) fn acquire_named(name: &str) -> io::Result<Option<OwnedHandle>> {
    let name: Vec<_> = name.encode_utf16().chain([0]).collect();
    let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    let error = io::Error::last_os_error();
    if handle.is_null() {
        return Err(error);
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    if error.raw_os_error() == Some(183) {
        return Ok(None);
    }
    Ok(Some(handle))
}

/// Whether some process holds `name` now. Unlike `acquire_named`, it never creates the mutex,
/// so a process starting at the same moment is not turned away.
pub(super) fn exists_named(name: &str) -> io::Result<bool> {
    let name: Vec<_> = name.encode_utf16().chain([0]).collect();
    // SYNCHRONIZE
    let handle = unsafe { OpenMutexW(0x0010_0000, 0, name.as_ptr()) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        // ERROR_FILE_NOT_FOUND: no process holds it.
        return if error.raw_os_error() == Some(2) { Ok(false) } else { Err(error) };
    }
    drop(unsafe { OwnedHandle::from_raw_handle(handle) });
    Ok(true)
}

/// The session event a GUI process waits on for the Manager's Quit (#56). Named by the
/// process id: the Manager has already verified that process before it signals.
fn exit_event_name(pid: u32) -> Vec<u16> {
    format!(r"Local\InstanceHarbor-GUI-Exit-{pid}").encode_utf16().chain([0]).collect()
}

/// Creates this process's exit event; manual reset, so a late waiter still sees it.
pub fn exit_event() -> io::Result<OwnedHandle> {
    let name = exit_event_name(std::process::id());
    let handle = unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// Asks the GUI process `pid` to exit. False when it has no exit event (an older GUI,
/// or one that already ended).
pub fn signal_exit(pid: u32) -> bool {
    let name = exit_event_name(pid);
    // EVENT_MODIFY_STATE
    let handle = unsafe { OpenEventW(0x0002, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    unsafe { SetEvent(handle.as_raw_handle()) != 0 }
}

/// Waits up to `milliseconds` (u32::MAX: forever) for the event; true once it is set.
pub fn wait(event: &OwnedHandle, milliseconds: u32) -> bool {
    unsafe { WaitForSingleObject(event.as_raw_handle(), milliseconds) == 0 }
}

pub fn signaled(event: &OwnedHandle) -> bool {
    wait(event, 0)
}

pub fn focus_existing() -> bool {
    let Ok(executable) = std::env::current_exe().and_then(|path| path.canonicalize()) else {
        return false;
    };
    // The main window title names the workspace, so only this workspace's GUI matches.
    let title = crate::desktop_channel::product_title(crate::desktop_channel::workspace_name().as_deref());
    let mut target: Target = (executable, title, std::ptr::null_mut());
    unsafe {
        EnumWindows(find_existing, (&mut target as *mut Target) as isize);
        let window = target.2;
        if window.is_null() {
            return false;
        }
        if IsIconic(window) != 0 {
            ShowWindowAsync(window, 9);
        } else if IsWindowVisible(window) == 0 {
            // The window may be hidden in the notification area; SW_SHOW restores it.
            ShowWindowAsync(window, 5);
        }
        if SetForegroundWindow(window) == 0 {
            FlashWindow(window, 1);
        }
    }
    true
}

/// The executable, the expected main window title, and the window found.
type Target = (PathBuf, String, Handle);

unsafe extern "system" fn find_existing(window: Handle, context: isize) -> i32 {
    let target = unsafe { &mut *(context as *mut Target) };
    let mut title = [0u16; 512];
    let size = unsafe { GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32) };
    if size <= 0 || String::from_utf16_lossy(&title[..size as usize]) != target.1 {
        return 1;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    if pid == std::process::id()
        || process_image(pid)
            .and_then(|image| image.canonicalize().ok())
            .as_ref()
            != Some(&target.0)
    {
        return 1;
    }
    target.2 = window;
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exit_request_reaches_the_event_of_that_process_only() {
        // No GUI with this id listens: nothing to signal.
        assert!(!signal_exit(u32::MAX - 1));
        let event = exit_event().unwrap();
        assert!(!signaled(&event));
        assert!(signal_exit(std::process::id()));
        assert!(signaled(&event));
    }

    #[test]
    fn exists_named_sees_a_held_lease_without_creating_one() {
        let name = format!(r"Local\instanceharbor-test-lease-{}", std::process::id());
        assert!(!exists_named(&name).unwrap());
        // Checking did not create it: the first acquire still wins.
        let lease = acquire_named(&name).unwrap().expect("no one holds the lease yet");
        assert!(exists_named(&name).unwrap());
        drop(lease);
        assert!(!exists_named(&name).unwrap());
    }
}
