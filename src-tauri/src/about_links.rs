//! Opens the About section's fixed links: the public repository pages in the
//! default browser, and the notice files beside the executable in Notepad.
use instanceharbor::about::{self, Link};
use serde_json::{json, Value};
use std::{os::windows::fs::MetadataExt, path::PathBuf, process::Command};

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        window: *mut std::ffi::c_void,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// A notice file beside the executable, when it is a plain file. Development
/// builds have none; the ZIP and Setup install both.
fn notice_file(name: &str) -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join(name);
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    (metadata.is_file() && metadata.file_attributes() & 0x400 == 0).then_some(path)
}

pub fn info() -> Value {
    json!({
        "version": about::VERSION,
        "commit": about::COMMIT,
        // The development workspace this GUI serves; null in the stable channel.
        "channel": instanceharbor::desktop_channel::TITLE,
        "workspace": instanceharbor::desktop_channel::workspace_name(),
        "author": about::AUTHOR,
        "repository": about::REPOSITORY,
        "issues": about::ISSUES,
        "license": notice_file(about::LICENSE_FILE).is_some(),
        "notices": notice_file(about::NOTICES_FILE).is_some(),
    })
}

fn notepad() -> Result<PathBuf, String> {
    let mut buffer = [0u16; 260];
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("SystemDirectoryUnavailable".into());
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..length])).join("notepad.exe"))
}

pub fn open(name: &str) -> Result<(), String> {
    let link = Link::parse(name).ok_or("AboutLinkUnknown")?;
    if let Some(url) = link.url() {
        let (verb, target) = (wide("open"), wide(url));
        let result = unsafe {
            ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), 1)
        };
        // ShellExecuteW reports success with a value greater than 32.
        return if result > 32 { Ok(()) } else { Err(format!("AboutLinkOpenFailed:{result}")) };
    }
    let file = link.file().and_then(notice_file).ok_or("AboutNoticeMissing")?;
    Command::new(notepad()?).arg(file).spawn().map(drop).map_err(|error| format!("AboutNoticeOpenFailed:{error}"))
}
