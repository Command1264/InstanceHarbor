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

pub fn acquire() -> io::Result<Option<OwnedHandle>> {
    acquire_named(if crate::desktop_channel::DEVELOPMENT {
        r"Local\InstanceHarbor-GUI-Dev"
    } else {
        r"Local\InstanceHarbor-GUI"
    })
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
    let mut target: (PathBuf, Handle) = (executable, std::ptr::null_mut());
    unsafe {
        EnumWindows(
            find_existing,
            (&mut target as *mut (PathBuf, Handle)) as isize,
        );
        let window = target.1;
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

unsafe extern "system" fn find_existing(window: Handle, context: isize) -> i32 {
    let mut title = [0u16; 64];
    let size = unsafe { GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32) };
    if size <= 0 || String::from_utf16_lossy(&title[..size as usize]) != crate::desktop_channel::TITLE {
        return 1;
    }
    let target = unsafe { &mut *(context as *mut (PathBuf, Handle)) };
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
    target.1 = window;
    0
}
