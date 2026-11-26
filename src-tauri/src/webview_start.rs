//! Creates the main window's WebView2 environment here instead of inside wry, so the
//! diagnostics log can tell environment creation from controller creation, and watches
//! for WebView2 processes that end (#117).
//!
//! The options match what wry 0.55 uses when it creates the environment itself:
//! the same browser arguments, the user's UI language, the default scroll bars and
//! no browser extensions. Tauri passes none of its own for this window.
use crate::webview_events;
use std::path::Path;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2Environment,
    ICoreWebView2EnvironmentOptions, ICoreWebView2ProcessFailedEventArgs2,
    COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_REASON,
    COREWEBVIEW2_SCROLLBAR_STYLE_DEFAULT,
};
use webview2_com::{
    CoreWebView2EnvironmentOptions, CreateCoreWebView2EnvironmentCompletedHandler,
    ProcessFailedEventHandler,
};
use windows_core::{Interface, HSTRING, PCWSTR};

/// wry's default arguments: no mini menu, no PDF mini menu, no SmartScreen.
const BROWSER_ARGUMENTS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut std::ffi::c_void, model: u32) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
    fn LCIDToLocaleName(locale: u32, name: *mut u16, size: i32, flags: u32) -> i32;
}

fn ui_language() -> String {
    // LOCALE_ALLOW_NEUTRAL_NAMES, LOCALE_NAME_MAX_LENGTH
    let mut name = [0u16; 85];
    let size = unsafe {
        LCIDToLocaleName(
            u32::from(GetUserDefaultUILanguage()),
            name.as_mut_ptr(),
            85,
            0x0800_0000,
        )
    };
    String::from_utf16_lossy(&name[..(size.max(1) as usize - 1)])
}

/// Development builds only: how to imitate a WebView2 that fails (#117 acceptance).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Only `simulation()` of a development build constructs these.
#[cfg_attr(not(feature = "development-channel"), allow(dead_code))]
pub enum Simulate {
    /// Environment creation never completes.
    Hang,
    /// As `Hang`, and the GUI also ignores a takeover's quit request, so it must be ended.
    HangDeaf,
    /// The browser process is ended right after the controller is created.
    Exit,
}

/// Reads `IH_DEV_SIMULATE_WEBVIEW2` once and removes it, so the Manager this GUI starts,
/// and the GUIs that Manager opens, do not inherit it.
pub fn simulation() -> Option<Simulate> {
    #[cfg(feature = "development-channel")]
    {
        let value = std::env::var_os("IH_DEV_SIMULATE_WEBVIEW2")?;
        std::env::remove_var("IH_DEV_SIMULATE_WEBVIEW2");
        match value.to_str() {
            Some("hang") => Some(Simulate::Hang),
            Some("hang-deaf") => Some(Simulate::HangDeaf),
            Some("exit") => Some(Simulate::Exit),
            _ => None,
        }
    }
    #[cfg(not(feature = "development-channel"))]
    None
}

fn hresult(error: &windows_core::Error) -> i32 {
    error.code().0
}

/// Creates the environment on this (the main) thread, pumping messages while it waits.
pub fn environment(
    data_directory: &Path,
    simulate: Option<Simulate>,
) -> Result<ICoreWebView2Environment, String> {
    webview_events::step("environment", "started", None);
    // As wry does before it creates an environment: WebView2 needs an STA on this thread.
    // COINIT_APARTMENTTHREADED; S_FALSE (already initialized) is fine.
    let _ = unsafe { CoInitializeEx(std::ptr::null_mut(), 0x2) };
    let (sender, receiver) = std::sync::mpsc::channel();
    if matches!(simulate, Some(Simulate::Hang | Simulate::HangDeaf)) {
        // Never completes, as a WebView2 that does not answer; the startup deadline ends us.
        let _keep = sender;
        let _ = webview2_com::wait_with_pump::<windows_core::Result<ICoreWebView2Environment>>(
            receiver,
        );
        return Err("WebView2SimulatedHang".into());
    }
    let options = CoreWebView2EnvironmentOptions::default();
    unsafe {
        options.set_additional_browser_arguments(BROWSER_ARGUMENTS.into());
        options.set_are_browser_extensions_enabled(false);
        options.set_language(ui_language());
        options.set_scroll_bar_style(COREWEBVIEW2_SCROLLBAR_STYLE_DEFAULT);
    }
    let folder = HSTRING::from(data_directory);
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |code, environment| {
            let result = code.and_then(|_| {
                environment.ok_or_else(|| {
                    windows_core::Error::from_hresult(windows_core::HRESULT(0x8000_4003u32 as i32))
                })
            });
            let _ = sender.send(result);
            Ok(())
        },
    ));
    let started = unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &folder,
            &ICoreWebView2EnvironmentOptions::from(options),
            &handler,
        )
    };
    let result = started.map_err(|error| hresult(&error)).and_then(|_| {
        match webview2_com::wait_with_pump(receiver) {
            Ok(result) => result.map_err(|error| hresult(&error)),
            // E_ABORT: the message loop ended while waiting.
            Err(_) => Err(0x8000_4004u32 as i32),
        }
    });
    match result {
        Ok(environment) => {
            webview_events::step("environment", "created", None);
            Ok(environment)
        }
        Err(code) => {
            webview_events::step("environment", "failed", Some(code));
            Err(format!("WebView2EnvironmentFailed (0x{:08X})", code as u32))
        }
    }
}

/// Records ProcessFailed for the main window's WebView2. With `Simulate::Exit`, also
/// ends the browser process once, to show what that looks like.
pub fn watch(webview: tauri::webview::PlatformWebview, simulate: Option<Simulate>) {
    let result = (|| -> windows_core::Result<()> {
        let core = unsafe { webview.controller().CoreWebView2()? };
        let handler = ProcessFailedEventHandler::create(Box::new(|_, arguments| {
            let Some(arguments) = arguments else {
                return Ok(());
            };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            unsafe { arguments.ProcessFailedKind(&mut kind)? };
            let (mut reason, mut exit) = (None, None);
            if let Ok(details) = arguments.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                let mut value = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
                let mut code = 0;
                if unsafe { details.Reason(&mut value) }.is_ok() {
                    reason = Some(value.0);
                }
                if unsafe { details.ExitCode(&mut code) }.is_ok() {
                    exit = Some(code);
                }
            }
            webview_events::process_failed(kind.0, reason, exit);
            Ok(())
        }));
        let mut token = 0i64;
        unsafe { core.add_ProcessFailed(&handler, &mut token)? };
        if simulate == Some(Simulate::Exit) {
            let mut browser = 0u32;
            unsafe { core.BrowserProcessId(&mut browser)? };
            end_process(browser);
        }
        Ok(())
    })();
    if let Err(error) = result {
        webview_events::step("process-failed-watch", "failed", Some(hresult(&error)));
    }
}

fn end_process(pid: u32) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn TerminateProcess(process: *mut std::ffi::c_void, code: u32) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    // PROCESS_TERMINATE; only the browser process this GUI's WebView2 just started.
    let process = unsafe { OpenProcess(0x0001, 0, pid) };
    if !process.is_null() {
        unsafe {
            TerminateProcess(process, 1);
            CloseHandle(process);
        }
    }
}
