//! Hold a Windows session mutex from before WebView creation until GUI exit.
use super::process_image;
use std::{
    ffi::c_void,
    io,
    os::windows::io::{FromRawHandle, OwnedHandle},
    path::PathBuf,
};
type Handle = *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(attributes: Handle, owner: i32, name: *const u16) -> Handle;
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
