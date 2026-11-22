#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(windows)]
mod folder_picker;
#[cfg(windows)]
mod workspace_binding;
#[cfg(windows)]
mod system_accent;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
use instanceharbor_v2::desktop_channel;
use instanceharbor_v2::diagnostics;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
static IPC_ID: AtomicU64 = AtomicU64::new(1);
static WINDOW_REVEALED: AtomicBool = AtomicBool::new(false);
fn diagnostic_write(result: Result<(), String>) {
    if let Err(error) = result {
        eprintln!("GUI diagnostic write failed: {error}")
    }
}
#[tauri::command]
fn diagnostic_status() -> serde_json::Value {
    match diagnostics::global() {
        Ok(log) => log.status(),
        Err(error) => json!({"available":false,"error":error}),
    }
}
#[tauri::command]
fn system_accent_color() -> Result<String, String> {
    #[cfg(windows)]
    {
        system_accent::current()
    }
    #[cfg(not(windows))]
    {
        Err("SystemAccentUnavailable".into())
    }
}
#[tauri::command]
fn tray_status(app: tauri::AppHandle, running: u32, language: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        use tauri::Manager;
        let language = tray::Language::parse(&language)?;
        let tray = app.try_state::<tray::Tray>().ok_or("TrayUnavailable")?;
        tray.update(running, language).map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (app, running, language);
        Err("TrayUnavailable".into())
    }
}
#[tauri::command]
async fn pick_management_folder(window: tauri::WebviewWindow, current: String) -> Result<Option<String>, String> {
    pick_path(window, current, None).await
}

#[tauri::command]
async fn pick_file(window: tauri::WebviewWindow, current: String, kind: String) -> Result<Option<String>, String> {
    pick_path(window, current, Some(kind)).await
}

async fn pick_path(window: tauri::WebviewWindow, current: String, kind: Option<String>) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        let owner = window.hwnd().map_err(|error| error.to_string())?.0 as isize;
        let pick = kind.as_deref().map(folder_picker::Pick::file).transpose()?.unwrap_or(folder_picker::Pick::Folder);
        let (sent, received) = std::sync::mpsc::sync_channel(1);
        window.run_on_main_thread(move || {
            let _ = sent.send(folder_picker::select(owner, &current, pick));
        }).map_err(|error| error.to_string())?;
        tauri::async_runtime::spawn_blocking(move || received.recv())
            .await.map_err(|error| error.to_string())?
            .map_err(|_| "FolderPickerUnavailable".to_owned())?
    }
    #[cfg(not(windows))]
    {
        let _ = (window, current, kind);
        Err("FolderPickerUnavailable".into())
    }
}
#[tauri::command]
fn report_gui_event(
    event: String,
    kind: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
) -> Result<(), String> {
    if !matches!(event.as_str(), "unhandled-error" | "unhandled-rejection") {
        return Err("InvalidDiagnosticEvent".into());
    }
    let kind = match kind.as_deref() {
        Some(
            "Error" | "TypeError" | "ReferenceError" | "SyntaxError" | "RangeError" | "URIError"
            | "EvalError" | "AggregateError",
        ) => kind.as_deref(),
        _ => Some("Unknown"),
    };
    diagnostics::global()?.record(
        json!({"type":"renderer-error","category":event,"kind":kind,"line":line,"column":column}),
    )
}
#[cfg(windows)]
fn caption_color(window: &tauri::WebviewWindow, dark: bool) -> Result<(), String> {
    use std::ffi::c_void;
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut c_void,
            attribute: u32,
            value: *const c_void,
            size: u32,
        ) -> i32;
    }
    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    // COLORREF is BGR; these match the web surface's --bg and --ink tokens.
    let colors: [(u32, u32); 2] = if dark {
        [(35, 0x002b1c12), (36, 0x00faf2ed)]
    } else {
        [(35, 0x00faf6f4), (36, 0x00402c20)]
    };
    for (attribute, color) in colors {
        let result = unsafe {
            DwmSetWindowAttribute(
                hwnd.0 as *mut c_void,
                attribute,
                &color as *const u32 as *const c_void,
                4,
            )
        };
        if result == 0x80070057u32 as i32 {
            // Windows before 11 does not support caption color. Keep native chrome.
            return Ok(());
        }
        if result < 0 {
            return Err(format!("無法套用標題列配色：{result:#x}"));
        }
    }
    Ok(())
}
#[tauri::command]
fn present_window(window: tauri::WebviewWindow, theme: String) -> Result<(), String> {
    let id = IPC_ID.fetch_add(1, Ordering::Relaxed);
    let started = std::time::Instant::now();
    if let Ok(log) = diagnostics::global() {
        diagnostic_write(log.begin(id, "present-window", "background"));
    }
    let result = present_window_inner(window, theme);
    if let Ok(log) = diagnostics::global() {
        let outcome = result
            .as_ref()
            .map(|_| serde_json::Value::Null)
            .map_err(Clone::clone);
        diagnostic_write(log.ipc(
            id,
            "present-window",
            "background",
            started.elapsed().as_millis(),
            &outcome,
        ));
    }
    result
}
fn present_window_inner(window: tauri::WebviewWindow, theme: String) -> Result<(), String> {
    let theme = match theme.as_str() {
        "system" => None,
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => return Err("不支援的外觀設定。".into()),
    };
    window.set_theme(theme).map_err(|error| error.to_string())?;
    #[cfg(windows)]
    let caption = caption_color(
        &window,
        window.theme().map_err(|error| error.to_string())? == tauri::Theme::Dark,
    );
    // Only the first call reveals the window. Later theme refreshes must not pull a
    // window the user hid in the notification area back onto the screen.
    if !WINDOW_REVEALED.load(Ordering::SeqCst)
        && !window.is_visible().map_err(|error| error.to_string())?
    {
        window.show().map_err(|error| error.to_string())?;
    }
    WINDOW_REVEALED.store(true, Ordering::SeqCst);
    #[cfg(windows)]
    caption?;
    Ok(())
}
#[tauri::command]
async fn manage(
    arguments: Vec<String>,
    origin: Option<String>,
) -> Result<serde_json::Value, String> {
    manage_recorded(arguments, origin, None).await
}

#[tauri::command]
async fn create_with_progress(
    work: String,
    draft: String,
    progress: tauri::ipc::Channel<instanceharbor_v2::CreationPhase>,
) -> Result<serde_json::Value, String> {
    manage_recorded(vec!["gui-create-confirm".into(), work, draft], Some("manual".into()), Some(progress)).await
}

async fn manage_recorded(
    arguments: Vec<String>,
    origin: Option<String>,
    progress: Option<tauri::ipc::Channel<instanceharbor_v2::CreationPhase>>,
) -> Result<serde_json::Value, String> {
    if arguments.first().is_some_and(|verb| matches!(
        verb.as_str(), "instance-diagnostics" | "import-batch-preview" | "import-batch-status"
    )) {
        return manage_inner(arguments, progress).await;
    }
    let action =
        diagnostics::safe_action(arguments.first().map(String::as_str).unwrap_or("")).to_owned();
    let origin = if origin.as_deref() == Some("manual") {
        "manual"
    } else {
        "background"
    };
    let id = IPC_ID.fetch_add(1, Ordering::Relaxed);
    let started = std::time::Instant::now();
    if let Ok(log) = diagnostics::global() {
        diagnostic_write(log.begin(id, &action, origin))
    }
    let result = manage_inner(arguments, progress).await;
    let recorded = diagnostics::global().and_then(|log| {
        log.ipc(id, &action, origin, started.elapsed().as_millis(), &result)
    });
    if let Err(log_error) = recorded {
        if let Err(operation_error) = &result {
            if matches!(action.as_str(), "preferences" | "save-preferences") {
                eprintln!("GUI IPC {id} {action}: {}; diagnostic sink: {log_error}", diagnostics::safe_error(operation_error));
            }
        }
    }
    result
}
async fn manage_inner(
    arguments: Vec<String>,
    progress: Option<tauri::ipc::Channel<instanceharbor_v2::CreationPhase>>,
) -> Result<serde_json::Value, String> {
    let root = std::env::var("IH_V2_WORKSPACE")
        .map_err(|_| "MissingWorkspaceBinding: 未指定管理工作區。")?;
    let mut command = vec![root];
    if arguments.first().is_some_and(|verb| verb == "legacy-preview") {
        return Err("此版本僅提供一般匯入。".into());
    }
    if arguments.first().is_some_and(|verb| {
        matches!(
            verb.as_str(),
            "host" | "init" | "manager-serve" | "manager-exit"
        )
    }) {
        return Err("不允許從 GUI 啟動 Host 或變更工作區。".into());
    }
    command.extend(arguments);
    tauri::async_runtime::spawn_blocking(move || match progress {
        Some(channel) => instanceharbor_v2::dispatch_with_creation_progress(&command, &|phase| {
            if let Err(error) = channel.send(phase) {
                eprintln!("GUI creation progress delivery failed: {error}");
            }
        }),
        None => instanceharbor_v2::dispatch(&command),
    })
        .await
        .map_err(|_| "管理操作中斷。".to_owned())?
}

#[cfg(windows)]
#[tauri::command]
async fn recovery_backup(
    action: String,
    path: String,
    password: String,
    generation: Option<u64>,
    public_key: Option<String>,
) -> Result<serde_json::Value, String> {
    struct Password(Vec<u8>);
    impl Drop for Password {
        fn drop(&mut self) {
            for byte in &mut self.0 {
                unsafe { std::ptr::write_volatile(byte, 0) };
            }
        }
    }
    let password = Password(password.into_bytes());
    if password.0.is_empty() || password.0.len() > 1024 {
        return Err("RecoveryPasswordInvalidLength".into());
    }
    let root = std::env::var("IH_V2_WORKSPACE").map_err(|_| "MissingWorkspaceBinding")?;
    let mut arguments = vec![root, format!("recovery-backup-{action}"), path];
    match (action.as_str(), generation, public_key) {
        ("export", Some(generation), None) => arguments.push(generation.to_string()),
        ("inspect", None, None) => (),
        ("restore", Some(generation), Some(public_key)) => {
            arguments.extend([generation.to_string(), public_key]);
        }
        _ => return Err("InvalidRecoveryArguments".into()),
    }
    // Keep the passphrase out of the generic IPC diagnostics and process arguments.
    tauri::async_runtime::spawn_blocking(move || {
        instanceharbor_v2::dispatch_recovery_backup(&arguments, &password.0)
    }).await.map_err(|_| "RecoveryBackupUnconfirmed".to_owned())?
}
#[cfg(windows)]
#[tauri::command]
async fn terminal_io(
    app: tauri::AppHandle,
    run: String,
    request: instanceharbor_v2::ConsoleRequest,
) -> Result<serde_json::Value, String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<instanceharbor_v2::ConsoleViewer>()
            .request(&run, request)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(windows)]
#[tauri::command]
fn terminal_present(window: tauri::WebviewWindow) -> Result<(), String> {
    window
        .show()
        .and_then(|_| window.unminimize())
        .and_then(|_| window.set_focus())
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
#[tauri::command]
fn terminal_pending(
    viewer: tauri::State<'_, instanceharbor_v2::ConsoleViewer>,
) -> Result<serde_json::Value, String> {
    serde_json::to_value(viewer.pending()?).map_err(|error| error.to_string())
}

#[cfg(windows)]
#[tauri::command]
fn terminal_title(
    window: tauri::WebviewWindow,
    viewer: tauri::State<'_, instanceharbor_v2::ConsoleViewer>,
    run: String,
) -> Result<(), String> {
    window.set_title(&viewer.title(&run)?).map_err(|error| error.to_string())
}

/// Sets the translated window title while no tab supplies its own.
#[cfg(windows)]
#[tauri::command]
fn terminal_window_title(window: tauri::WebviewWindow, title: String) -> Result<(), String> {
    if title.trim().is_empty() || title.chars().count() > 256 || title.chars().any(char::is_control) {
        return Err("InvalidConsoleRequest".into());
    }
    window.set_title(&title).map_err(|error| error.to_string())
}

#[cfg(windows)]
#[tauri::command]
fn terminal_close(
    app: tauri::AppHandle,
    viewer: tauri::State<'_, instanceharbor_v2::ConsoleViewer>,
    run: String,
    version: u64,
) -> Result<bool, String> {
    let (removed, exit) = viewer.close(&run, version)?;
    if exit {
        app.exit(0);
    }
    Ok(removed)
}

#[cfg(windows)]
fn run_console(workspace: &str) -> Result<(), String> {
    desktop_channel::validate_workspace(std::path::Path::new(workspace), false)?;
    workspace_binding::configure_development_desktop()?;
    let Some(viewer) = instanceharbor_v2::ConsoleViewer::start(workspace)? else {
        return Ok(());
    };
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = desktop_channel::IDENTIFIER.into();
    context.config_mut().app.windows.clear();
    context.config_mut().app.security.csp = Some(serde_json::from_value(json!(
        "default-src 'self'; connect-src ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'"
    )).map_err(|error| error.to_string())?);
    let app = tauri::Builder::default()
        .manage(viewer)
        .invoke_handler(tauri::generate_handler![
            terminal_io,
            terminal_present,
            terminal_pending,
            terminal_title,
            terminal_window_title,
            terminal_close
        ])
        .setup(move |app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "console",
                tauri::WebviewUrl::App("console.html".into()),
            )
            // The viewer replaces this with its translated title before the window is shown.
            .title(desktop_channel::TITLE)
            .inner_size(1020.0, 820.0)
            .min_inner_size(460.0, 280.0)
            .resizable(true)
            .enable_clipboard_access()
            .devtools(false)
            .visible(false)
            .build()?;
            let window_handle = window.clone();
            window.on_window_event(move |event| match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    if let Err(error) = window_handle.hide() {
                        eprintln!("Could not hide native console: {error}");
                    }
                }
                tauri::WindowEvent::Focused(true) => {
                    if let Err(error) =
                        window_handle.eval("window.dispatchEvent(new Event('console-focus'))")
                    {
                        eprintln!("Could not focus native console: {error}");
                    }
                }
                _ => {}
            });
            Ok(())
        })
        .build(context)
        .map_err(|error| error.to_string())?;
    app.run(|_, _| {});
    Ok(())
}

fn main() {
    #[cfg(windows)]
    let _installation = match instanceharbor_v2::installation::acquire() {
        Ok(lease) => lease,
        Err(error) => {
            workspace_binding::show_startup_error(&error);
            std::process::exit(1);
        }
    };
    #[cfg(windows)]
    if let [_, workspace, mode] = std::env::args().collect::<Vec<_>>().as_slice() {
        if mode == "native-console" {
            if let Err(error) = run_console(workspace) {
                eprintln!("InstanceHarbor server console failed to start: {error}");
                std::process::exit(1);
            }
            return;
        }
    }
    #[cfg(windows)]
    if !desktop_channel::DEVELOPMENT
        && std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--stable-default"))
    {
        // The installed Start menu entry always selects the stable user's default profile.
        std::env::remove_var("IH_V2_WORKSPACE");
        std::env::remove_var("WEBVIEW2_USER_DATA_FOLDER");
    }
    #[cfg(windows)]
    if desktop_channel::DEVELOPMENT {
        if let Err(error) = workspace_binding::development_workspace()
            .and_then(|_| workspace_binding::configure_development_desktop())
        {
            workspace_binding::show_startup_error(&error);
            std::process::exit(1);
        }
    } else if let Some(workspace) = std::env::var_os("IH_V2_WORKSPACE") {
        if let Err(error) = desktop_channel::validate_workspace(std::path::Path::new(&workspace), false) {
            workspace_binding::show_startup_error(&error);
            std::process::exit(1);
        }
    }
    #[cfg(windows)]
    let _gui_lease = match instanceharbor_v2::gui_singleton::acquire() {
        Ok(Some(lease)) => lease,
        Ok(None) => {
            let existing_window_found = instanceharbor_v2::gui_singleton::focus_existing();
            if let Ok(log) = diagnostics::global() {
                diagnostic_write(log.record(
                    json!({"type":"gui-duplicate","existingWindowFound":existing_window_found}),
                ));
            }
            return;
        }
        Err(error) => {
            eprintln!("InstanceHarbor GUI instance check failed: {error}");
            std::process::exit(1);
        }
    };
    #[cfg(windows)]
    if std::env::var_os("IH_V2_WORKSPACE").is_none() {
        match workspace_binding::default_workspace() {
            // Bind once before the webview and its worker threads start.
            Ok(workspace) => std::env::set_var("IH_V2_WORKSPACE", workspace),
            Err(error) => {
                workspace_binding::show_startup_error(&error);
                std::process::exit(1);
            }
        }
    }
    match diagnostics::global(){
        Ok(log)=>diagnostic_write(log.record(json!({"type":"gui-start","pid":std::process::id(),"version":env!("CARGO_PKG_VERSION"),"executableFingerprint":std::env::current_exe().ok().map(|p|diagnostics::path_fingerprint(&p)),"workspaceFingerprint":std::env::var("IH_V2_WORKSPACE").ok().map(|p|diagnostics::path_fingerprint(std::path::Path::new(&p)))}))),
        Err(error)=>eprintln!("GUI diagnostics unavailable: {error}"),
    }
    let mut context = tauri::generate_context!();
    // Acceptance uses the debug profile, so set this before Tauri creates windows.
    let enable_devtools = cfg!(any(debug_assertions, feature = "development-channel"))
        && !cfg!(feature = "acceptance-ui");
    #[cfg(windows)]
    {
        context.config_mut().identifier = desktop_channel::IDENTIFIER.into();
        context.config_mut().product_name = Some(desktop_channel::TITLE.into());
        for window in &mut context.config_mut().app.windows {
            window.title = desktop_channel::TITLE.into();
        }
    }
    for window in &mut context.config_mut().app.windows {
        window.devtools = Some(enable_devtools);
    }
    let application = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            manage,
            create_with_progress,
            recovery_backup,
            present_window,
            system_accent_color,
            pick_management_folder,
            pick_file,
            diagnostic_status,
            report_gui_event,
            tray_status
        ])
        .setup(|app| {
            #[cfg(windows)]
            tray::install(app.handle())?;
            #[cfg(not(windows))]
            let _ = app;
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(windows)]
            tray::window_event(window, event);
            #[cfg(not(windows))]
            let _ = (window, event);
        })
        .build(context)
        .map_err(|error| error.to_string());
    match application {
        Ok(app) => app.run(|_, event| {
            // Tauri exits the process after this callback; code after run is not
            // a reliable place to record a normal Windows GUI shutdown.
            if matches!(event, tauri::RunEvent::Exit) {
                if let Ok(log) = diagnostics::global() {
                    diagnostic_write(log.runtime_result(&Ok(())));
                }
            }
        }),
        Err(error) => {
            if let Ok(log) = diagnostics::global() {
                diagnostic_write(log.runtime_result(&Err(error.clone())));
            }
            eprintln!("InstanceHarbor 啟動失敗：{error}");
            std::process::exit(1)
        }
    }
}
