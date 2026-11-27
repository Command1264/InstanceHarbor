#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(windows)]
mod about_links;
#[cfg(windows)]
mod folder_picker;
#[cfg(windows)]
mod workspace_binding;
#[cfg(windows)]
mod system_accent;
#[cfg(windows)]
mod webview_runtime;
mod startup_prefetch;
mod startup_theme;
mod startup_timing;
#[cfg(windows)]
mod startup_guard;
mod fast_open;
#[cfg(windows)]
mod webview_events;
#[cfg(windows)]
mod webview_start;
#[cfg(windows)]
mod exit_watchdog;
#[cfg(windows)]
mod manager_attach;
mod window_placement;
#[cfg(windows)]
use instanceharbor::desktop_channel;
use instanceharbor::diagnostics;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
static IPC_ID: AtomicU64 = AtomicU64::new(1);
static WINDOW_REVEALED: AtomicBool = AtomicBool::new(false);
static PREFETCH: startup_prefetch::Prefetch = startup_prefetch::Prefetch::new();
fn background_color(dark: bool) -> tauri::window::Color {
    let (red, green, blue) = startup_theme::background(dark);
    tauri::window::Color(red, green, blue, 255)
}
fn diagnostic_write(result: Result<(), String>) {
    if let Err(error) = result {
        eprintln!("GUI diagnostic write failed: {error}")
    }
}
/// Records the page's cold-start marks once, together with the native marks.
#[tauri::command]
fn report_startup_timing(marks: std::collections::BTreeMap<String, f64>) -> Result<(), String> {
    match startup_timing::report(&marks)? {
        Some(event) => diagnostics::global()?.record(event),
        None => Ok(()),
    }
}
#[tauri::command]
fn diagnostic_status() -> serde_json::Value {
    match diagnostics::global() {
        Ok(log) => log.status(),
        Err(error) => json!({"available":false,"error":error,"folder":diagnostics::log_folder()}),
    }
}
#[tauri::command]
fn about_info() -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        Ok(about_links::info())
    }
    #[cfg(not(windows))]
    {
        Err("AboutUnavailable".into())
    }
}
/// Opens one of the About section's fixed links; `link` is a name, never a URL.
#[tauri::command]
fn open_about_link(link: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        about_links::open(&link)
    }
    #[cfg(not(windows))]
    {
        let _ = link;
        Err("AboutUnavailable".into())
    }
}
/// Opens the GitHub page of one release; `tag` is checked against the version format (#65).
#[tauri::command]
fn open_update_release(tag: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        about_links::open_release(&tag)
    }
    #[cfg(not(windows))]
    {
        let _ = tag;
        Err("AboutUnavailable".into())
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
                hwnd.0,
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
    let first = !WINDOW_REVEALED.load(Ordering::SeqCst);
    if first {
        startup_timing::mark("present-window");
    }
    let theme = match theme.as_str() {
        "system" => None,
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => return Err("不支援的外觀設定。".into()),
    };
    window.set_theme(theme).map_err(|error| error.to_string())?;
    #[cfg(windows)]
    let dark = window.theme().map_err(|error| error.to_string())? == tauri::Theme::Dark;
    #[cfg(windows)]
    let caption = caption_color(&window, dark);
    // Keep what WebView2 shows before it draws in step with the page (see startup_theme).
    #[cfg(windows)]
    if let Err(error) = window.set_background_color(Some(background_color(dark))) {
        eprintln!("Could not set the window background color: {error}");
    }
    // Only the first call reveals the window. Later theme refreshes must not pull a
    // window the user hid in the notification area back onto the screen.
    if !WINDOW_REVEALED.load(Ordering::SeqCst)
        && !window.is_visible().map_err(|error| error.to_string())?
    {
        window.show().map_err(|error| error.to_string())?;
    }
    if first {
        startup_timing::mark("window-shown");
    }
    WINDOW_REVEALED.store(true, Ordering::SeqCst);
    #[cfg(windows)]
    startup_guard::mark_revealed();
    #[cfg(windows)]
    caption?;
    Ok(())
}
#[tauri::command]
async fn manage(
    arguments: Vec<String>,
    origin: Option<String>,
) -> Result<serde_json::Value, String> {
    if let Some(pending) = PREFETCH.take(&arguments, origin.as_deref()) {
        // The prefetch already recorded this read in the diagnostics log.
        if let Ok(Some(result)) = tauri::async_runtime::spawn_blocking(move || pending.finish()).await {
            return result;
        }
    }
    manage_recorded(arguments, origin, None).await
}

#[tauri::command]
async fn create_with_progress(
    work: String,
    draft: String,
    progress: tauri::ipc::Channel<instanceharbor::CreationPhase>,
) -> Result<serde_json::Value, String> {
    manage_recorded(vec!["gui-create-confirm".into(), work, draft], Some("manual".into()), Some(Progress::Creation(progress))).await
}

/// A core install or restore whose steps reach the GUI as they happen (#60).
#[tauri::command]
async fn core_maintenance_with_progress(
    arguments: Vec<String>,
    progress: tauri::ipc::Channel<instanceharbor::CoreUpdatePhase>,
) -> Result<serde_json::Value, String> {
    if !arguments.first().is_some_and(|verb| matches!(verb.as_str(), "core-install-latest" | "core-install" | "core-restore" | "core-install-version")) {
        return Err("InvalidCoreArguments".into());
    }
    manage_recorded(arguments, Some("manual".into()), Some(Progress::Core(progress))).await
}

/// Where a long operation reports its steps.
enum Progress {
    Creation(tauri::ipc::Channel<instanceharbor::CreationPhase>),
    Core(tauri::ipc::Channel<instanceharbor::CoreUpdatePhase>),
}

async fn manage_recorded(
    arguments: Vec<String>,
    origin: Option<String>,
    progress: Option<Progress>,
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
    progress: Option<Progress>,
) -> Result<serde_json::Value, String> {
    let root = std::env::var("IH_WORKSPACE")
        .map_err(|_| "MissingWorkspaceBinding: 未指定管理工作區。")?;
    let mut command = vec![root];
    if arguments.first().is_some_and(|verb| verb == "legacy-preview") {
        return Err("此版本僅提供一般匯入。".into());
    }
    if arguments.first().is_some_and(|verb| {
        matches!(
            verb.as_str(),
            "host" | "init" | "manager-serve" | "manager-exit" | "app-exit" | "gui-attach"
        )
    }) {
        return Err("不允許從 GUI 啟動 Host 或變更工作區。".into());
    }
    command.extend(arguments);
    tauri::async_runtime::spawn_blocking(move || match progress {
        Some(Progress::Creation(channel)) => instanceharbor::dispatch_with_creation_progress(&command, &|phase| {
            if let Err(error) = channel.send(phase) {
                eprintln!("GUI creation progress delivery failed: {error}");
            }
        }),
        Some(Progress::Core(channel)) => instanceharbor::dispatch_with_core_progress(&command, &|phase| {
            if let Err(error) = channel.send(phase) {
                eprintln!("GUI core progress delivery failed: {error}");
            }
        }),
        None => instanceharbor::dispatch(&command),
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
    let root = std::env::var("IH_WORKSPACE").map_err(|_| "MissingWorkspaceBinding")?;
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
        instanceharbor::dispatch_recovery_backup(&arguments, &password.0)
    }).await.map_err(|_| "RecoveryBackupUnconfirmed".to_owned())?
}
#[cfg(windows)]
#[tauri::command]
async fn terminal_io(
    app: tauri::AppHandle,
    run: String,
    request: instanceharbor::ConsoleRequest,
) -> Result<serde_json::Value, String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<instanceharbor::ConsoleViewer>()
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
    viewer: tauri::State<'_, instanceharbor::ConsoleViewer>,
) -> Result<serde_json::Value, String> {
    serde_json::to_value(viewer.pending()?).map_err(|error| error.to_string())
}

#[cfg(windows)]
#[tauri::command]
fn terminal_title(
    window: tauri::WebviewWindow,
    viewer: tauri::State<'_, instanceharbor::ConsoleViewer>,
    run: String,
) -> Result<(), String> {
    let title = desktop_channel::window_title(&viewer.title(&run)?, desktop_channel::workspace_name().as_deref());
    window.set_title(&title).map_err(|error| error.to_string())
}

/// Sets the translated window title while no tab supplies its own.
#[cfg(windows)]
#[tauri::command]
fn terminal_window_title(window: tauri::WebviewWindow, title: String) -> Result<(), String> {
    if title.trim().is_empty() || title.chars().count() > 256 || title.chars().any(char::is_control) {
        return Err("InvalidConsoleRequest".into());
    }
    let title = desktop_channel::window_title(&title, desktop_channel::workspace_name().as_deref());
    window.set_title(&title).map_err(|error| error.to_string())
}

#[cfg(windows)]
#[tauri::command]
fn terminal_close(
    app: tauri::AppHandle,
    viewer: tauri::State<'_, instanceharbor::ConsoleViewer>,
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
const CONSOLE_SIZE: window_placement::LogicalSize = window_placement::LogicalSize {
    width: 1020.0,
    height: 820.0,
};
#[cfg(windows)]
const CONSOLE_MIN_SIZE: window_placement::LogicalSize = window_placement::LogicalSize {
    width: 460.0,
    height: 280.0,
};

#[cfg(windows)]
fn run_console(workspace: &str) -> Result<(), String> {
    desktop_channel::validate_workspace(std::path::Path::new(workspace), false)?;
    desktop_channel::bind_workspace(std::path::Path::new(workspace));
    workspace_binding::configure_desktop()?;
    let Some(viewer) = instanceharbor::ConsoleViewer::start(workspace)? else {
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
            use tauri::Manager;
            // Tray Quit and `app-exit` end the viewer with the GUI and the Manager (#114).
            let quit = app.state::<instanceharbor::ConsoleViewer>().quit_requests()?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if quit.recv().is_ok() {
                    handle.exit(0);
                }
            });
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "console",
                tauri::WebviewUrl::App("console.html".into()),
            )
            // The viewer replaces this with its translated title before the window is shown.
            .title(desktop_channel::product_title(desktop_channel::workspace_name().as_deref()))
            .inner_size(CONSOLE_SIZE.width, CONSOLE_SIZE.height)
            .min_inner_size(CONSOLE_MIN_SIZE.width, CONSOLE_MIN_SIZE.height)
            .resizable(true)
            .enable_clipboard_access()
            .devtools(false)
            .visible(false);
            let window = match workspace_binding::webview_data_directory() {
                Some(folder) => window.data_directory(folder),
                None => window,
            }
            .build()?;
            if let Err(error) =
                window_placement::fit_window(&window, CONSOLE_SIZE, CONSOLE_MIN_SIZE)
            {
                eprintln!("Could not fit native console to the work area: {error}");
            }
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

/// Keeps this GUI registered with its workspace's Manager, so the Manager's tray
/// icon shows while the GUI runs and its Quit closes this GUI too. Repeats in case
/// the Manager is restarted while the window stays open.
#[cfg(windows)]
fn attach_to_manager() {
    let Some(root) = std::env::var_os("IH_WORKSPACE").and_then(|root| root.into_string().ok()) else {
        return;
    };
    let pid = std::process::id().to_string();
    let mut last_error: Option<String> = None;
    let mut failures = 0u32;
    loop {
        // A closing GUI must not start the Manager again: the user may have just quit it.
        if exit_watchdog::closing() {
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        match instanceharbor::dispatch(&[root.clone(), "gui-attach".into(), pid.clone()]) {
            Ok(_) => {
                last_error = None;
                failures = 0;
            }
            // A Manager that is quitting is about to close this GUI.
            Err(error) if error == "ManagerExiting" => {}
            Err(error) => {
                failures = failures.saturating_add(1);
                if last_error.as_deref() != Some(error.as_str()) {
                    eprintln!("Could not attach to the Manager: {error}");
                    if let Ok(log) = diagnostics::global() {
                        diagnostic_write(log.record(json!({"type":"gui-attach","error":diagnostics::safe_error(&error)})));
                    }
                    last_error = Some(error);
                }
            }
        }
        std::thread::sleep(manager_attach::retry_delay(failures));
    }
}

fn main() {
    startup_timing::mark("main");
    // Before any thread or child process starts: read once and drop from the environment.
    #[cfg(windows)]
    let simulate = webview_start::simulation();
    #[cfg(windows)]
    fast_open::configure();
    #[cfg(windows)]
    let _installation = match instanceharbor::installation::acquire() {
        Ok(lease) => lease,
        Err(error) => {
            workspace_binding::show_startup_error(&error);
            std::process::exit(1);
        }
    };
    // Before the single-instance lock: a GUI that cannot create its window must
    // not keep running and block the next launch.
    startup_timing::mark("installation-lease");
    #[cfg(windows)]
    if !webview_runtime::ensure_available() {
        std::process::exit(1);
    }
    startup_timing::mark("webview-runtime-checked");
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
        std::env::remove_var("IH_WORKSPACE");
        std::env::remove_var("WEBVIEW2_USER_DATA_FOLDER");
    }
    #[cfg(windows)]
    if desktop_channel::DEVELOPMENT {
        if let Err(error) = workspace_binding::development_workspace()
            .map(|workspace| desktop_channel::bind_workspace(&workspace))
            .and_then(|_| workspace_binding::configure_desktop())
        {
            workspace_binding::show_startup_error(&error);
            std::process::exit(1);
        }
    } else {
        if let Some(workspace) = std::env::var_os("IH_WORKSPACE") {
            if let Err(error) = desktop_channel::validate_workspace(std::path::Path::new(&workspace), false) {
                workspace_binding::show_startup_error(&error);
                std::process::exit(1);
            }
        }
        if let Err(error) = workspace_binding::configure_desktop() {
            workspace_binding::show_startup_error(&error);
            std::process::exit(1);
        }
    }
    startup_timing::mark("workspace-validated");
    #[cfg(windows)]
    let _gui_lease = match instanceharbor::gui_singleton::acquire() {
        Ok(Some(lease)) => lease,
        Ok(None) => match replace_stuck_gui() {
            Ok(lease) => lease,
            Err(holder) => {
                // A GUI in fast open mode shows its hidden window and resumes its page (#56).
                startup_guard::request_show(&desktop_channel::gui_lock_name(desktop_channel::workspace_name().as_deref()));
                let existing_window_found = instanceharbor::gui_singleton::focus_existing();
                if let Ok(log) = diagnostics::global() {
                    diagnostic_write(log.record(json!({
                        "type":"gui-duplicate",
                        "existingWindowFound":existing_window_found,
                        "holder":holder,
                    })));
                }
                return;
            }
        },
        Err(error) => {
            eprintln!("InstanceHarbor GUI instance check failed: {error}");
            std::process::exit(1);
        }
    };
    startup_timing::mark("gui-singleton");
    #[cfg(windows)]
    if std::env::var_os("IH_WORKSPACE").is_none() {
        match workspace_binding::default_workspace() {
            // Bind once before the webview and its worker threads start.
            Ok(workspace) => std::env::set_var("IH_WORKSPACE", workspace),
            Err(error) => {
                workspace_binding::show_startup_error(&error);
                std::process::exit(1);
            }
        }
    }
    // Publish this GUI as the lock holder, so a later launch can replace it if it never
    // shows its window, and end it if that window is not shown in time (#117).
    #[cfg(windows)]
    {
        let lock = desktop_channel::gui_lock_name(desktop_channel::workspace_name().as_deref());
        if let Err(error) = startup_guard::publish(
            &lock,
            startup_guard::current_workspace(|| None),
            simulate != Some(webview_start::Simulate::HangDeaf),
        ) {
            eprintln!("GUI owner record unavailable: {error}");
            if let Ok(log) = diagnostics::global() {
                diagnostic_write(log.record(json!({"type":"gui-owner-record","error":diagnostics::safe_error(&error)})));
            }
        }
        startup_guard::start_deadline(startup_timing::since_start);
    }
    match diagnostics::global(){
        Ok(log)=>diagnostic_write(log.record(json!({"type":"gui-start","pid":std::process::id(),"version":env!("CARGO_PKG_VERSION"),"commit":instanceharbor::about::COMMIT,"executableFingerprint":std::env::current_exe().ok().map(|p|diagnostics::path_fingerprint(&p)),"workspaceFingerprint":std::env::var("IH_WORKSPACE").ok().map(|p|diagnostics::path_fingerprint(std::path::Path::new(&p)))}))),
        Err(error)=>eprintln!("GUI diagnostics unavailable: {error}"),
    }
    startup_timing::mark("diagnostics-open");
    // Settings may have scheduled this WebView2 folder for deletion; WebView2 has not opened it yet.
    #[cfg(windows)]
    if let Some(result) = instanceharbor::clear_scheduled_webview() {
        if let Ok(log) = diagnostics::global() {
            diagnostic_write(log.record(match &result {
                Ok(()) => json!({"type":"webview-cache-cleared"}),
                Err(error) => json!({"type":"webview-cache-clear-failed","error":diagnostics::safe_error(error)}),
            }));
        }
        if let Err(error) = result {
            eprintln!("Scheduled WebView2 cache clear failed: {error}");
        }
    }
    // The workspace is bound; read what the first screen needs while WebView2 starts.
    #[cfg(windows)]
    for verb in startup_prefetch::VERBS {
        PREFETCH.start(verb, move || {
            tauri::async_runtime::block_on(manage_recorded(vec![verb.into()], None, None))
        });
    }
    let mut context = tauri::generate_context!();
    // Acceptance uses the debug profile, so set this before Tauri creates windows.
    let enable_devtools = cfg!(any(debug_assertions, feature = "development-channel"))
        && !cfg!(feature = "acceptance-ui");
    #[cfg(windows)]
    {
        context.config_mut().identifier = desktop_channel::IDENTIFIER.into();
        // The development channel names the workspace in the title and taskbar.
        let title = desktop_channel::product_title(desktop_channel::workspace_name().as_deref());
        context.config_mut().product_name = Some(title.clone());
        for window in &mut context.config_mut().app.windows {
            window.title = title.clone();
        }
    }
    for window in &mut context.config_mut().app.windows {
        window.devtools = Some(enable_devtools);
    }
    // Create the main window in setup, so it can name its WebView2 folder.
    #[cfg(windows)]
    let main_window = std::mem::take(&mut context.config_mut().app.windows);
    let application = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            manage,
            create_with_progress,
            core_maintenance_with_progress,
            recovery_backup,
            present_window,
            system_accent_color,
            pick_management_folder,
            pick_file,
            diagnostic_status,
            report_gui_event,
            about_info,
            open_about_link,
            open_update_release,
            report_startup_timing
        ])
        .setup(move |app| {
            startup_timing::mark("setup");
            #[cfg(windows)]
            for config in &main_window {
                // The page's first frame follows the window's theme, and WebView2 shows this
                // color until it draws (see startup_theme).
                let saved = std::env::var_os("IH_WORKSPACE")
                    .and_then(|workspace| startup_theme::saved(std::path::Path::new(&workspace)));
                let dark = startup_theme::dark(saved, startup_theme::windows_dark);
                startup_timing::mark("window-theme");
                let window = tauri::WebviewWindowBuilder::from_config(app.handle(), config)?
                    .theme(Some(if dark { tauri::Theme::Dark } else { tauri::Theme::Light }))
                    .background_color(background_color(dark));
                // configure_desktop always names the profile folder. Without one, Tauri picks
                // its own default, so wry creates the environment as before.
                let window = match workspace_binding::webview_data_directory() {
                    Some(folder) => {
                        startup_timing::mark("webview-environment");
                        let environment = webview_start::environment(&folder, simulate)?;
                        window.data_directory(folder).with_environment(environment)
                    }
                    None => window,
                };
                startup_timing::mark("webview-controller");
                webview_events::step("controller", "started", None);
                let window = window.build().inspect_err(|_| webview_events::step("controller", "failed", None))?;
                webview_events::step("controller", "created", None);
                startup_timing::mark("window-built");
                if let Err(error) = window.with_webview(move |webview| webview_start::watch(webview, simulate)) {
                    eprintln!("Could not watch the WebView2 processes: {error}");
                }
                let size = window_placement::LogicalSize {
                    width: config.width,
                    height: config.height,
                };
                let min_size = window_placement::LogicalSize {
                    width: config.min_width.unwrap_or(1.0),
                    height: config.min_height.unwrap_or(1.0),
                };
                if let Err(error) = window_placement::fit_window(&window, size, min_size) {
                    eprintln!("Could not fit the main window to the work area: {error}");
                }
                match window.hwnd() {
                    Ok(handle) => {
                        if let Err(error) = workspace_binding::name_taskbar_window(handle.0 as _) {
                            eprintln!("Could not name the taskbar Jump List: {error}");
                        }
                        exit_watchdog::watch(handle.0 as isize)
                    }
                    Err(error) => eprintln!("GUI exit watchdog cannot see the main window: {error}"),
                }
                fast_open::install(&window);
                let main = window.clone();
                window.on_window_event(move |event| match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        let revealed = WINDOW_REVEALED.load(Ordering::SeqCst);
                        let (enabled, quitting) = (fast_open::enabled(), fast_open::quitting());
                        if let Ok(log) = diagnostics::global() {
                            diagnostic_write(log.record(json!({
                                "type":"gui-close-requested","fastOpen":enabled,"revealed":revealed,"quitting":quitting,
                            })));
                        }
                        if fast_open::on_close(enabled, quitting, revealed) == fast_open::Close::Hide {
                            api.prevent_close();
                            fast_open::hide(&main);
                        } else {
                            exit_watchdog::arm();
                        }
                    }
                    tauri::WindowEvent::Destroyed => exit_watchdog::arm(),
                    _ => {}
                });
            }
            // The Manager owns the notification-area icon; attaching starts it if needed.
            #[cfg(windows)]
            std::thread::spawn(attach_to_manager);
            #[cfg(not(windows))]
            let _ = app;
            startup_timing::mark("setup-done");
            Ok(())
        })
        .build(context)
        .map_err(|error| error.to_string());
    startup_timing::mark("app-built");
    match application {
        Ok(app) => app.run(|_, event| {
            #[cfg(windows)]
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                exit_watchdog::arm();
            }
            // Tauri exits the process after this callback; code after run is not
            // a reliable place to record a normal Windows GUI shutdown.
            if matches!(event, tauri::RunEvent::Exit) {
                if let Ok(log) = diagnostics::global() {
                    diagnostic_write(log.runtime_result(&Ok(())));
                }
                // Development builds only: stay alive with no window, as the stuck GUI
                // did, so the exit watchdog can be verified.
                #[cfg(feature = "development-channel")]
                if std::env::var_os("IH_DEV_HANG_ON_EXIT").is_some() {
                    loop {
                        std::thread::park();
                    }
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

/// The GUI lock is held by a GUI of this installation and workspace that started more
/// than 15 seconds ago and never showed its window: end it and take the lock (#117).
/// Otherwise returns what was seen about the holder, for the `gui-duplicate` record.
#[cfg(windows)]
fn replace_stuck_gui() -> Result<std::os::windows::io::OwnedHandle, serde_json::Value> {
    let lock = desktop_channel::gui_lock_name(desktop_channel::workspace_name().as_deref());
    let owner = startup_guard::read(&lock);
    let holder = owner.as_ref().and_then(startup_guard::open_verified);
    let workspace = startup_guard::current_workspace(|| workspace_binding::default_workspace().ok());
    let now = startup_guard::now();
    let seen = owner.map(|owner| json!({
        "verified":holder.is_some(),
        "sameWorkspace":owner.workspace == workspace,
        "revealed":owner.revealed,
        "ageMs":now.saturating_sub(owner.created) / 10_000,
    }));
    let (startup_guard::Takeover::Replace(owner), Some(holder)) =
        (startup_guard::decide(owner, now, workspace, holder.is_some()), holder.as_ref())
    else {
        return Err(seen.unwrap_or(serde_json::Value::Null));
    };
    let ended = startup_guard::end(&lock, holder);
    // The lock is released when the holder's process ends; give Windows a moment.
    let mut lease = None;
    if matches!(ended, startup_guard::Ended::Quit | startup_guard::Ended::Terminated) {
        for _ in 0..20 {
            match instanceharbor::gui_singleton::acquire() {
                Ok(Some(acquired)) => {
                    lease = Some(acquired);
                    break;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(error) => {
                    eprintln!("InstanceHarbor GUI instance check failed: {error}");
                    break;
                }
            }
        }
    }
    if let Ok(log) = diagnostics::global() {
        let age = startup_guard::now().saturating_sub(owner.created) / 10_000;
        diagnostic_write(log.record(json!({
            "type":"gui-takeover",
            "holderPid":owner.pid,
            "holderAgeMs":age,
            "ended":ended.name(),
            "lockTaken":lease.is_some(),
        })));
    }
    lease.ok_or_else(|| json!({"takeover":ended.name()}))
}

#[cfg(test)]
mod console_viewer_exit_tests {
    fn body(source: &str, start: &str) -> String {
        let rest = source.split(start).nth(1).unwrap_or_else(|| panic!("{start} exists"));
        rest.split("\n}\n").next().unwrap().to_owned()
    }

    // Production code only, with Windows line endings removed: these tests name the same calls.
    fn source() -> String {
        let source = include_str!("main.rs").replace("\r\n", "\n");
        let production = source.split("#[cfg(test)]\nmod console_viewer_exit_tests").next().unwrap();
        assert!(production.len() < source.len(), "the test module is split off");
        production.to_owned()
    }

    #[test]
    fn closing_the_last_tab_ends_the_viewer_process() {
        // ConsoleViewer::close decides whether this was the last tab (console_viewer tests).
        let close = body(&source(), "fn terminal_close(");
        assert!(close.contains("let (removed, exit) = viewer.close(&run, version)?;"));
        assert!(close.contains("if exit {\n        app.exit(0);\n    }"));
    }

    #[test]
    fn the_x_button_hides_the_viewer_and_a_manager_quit_ends_it() {
        let console = body(&source(), "fn run_console(");
        assert!(console.contains("api.prevent_close();") && console.contains("window_handle.hide()"));
        assert!(console.contains("let quit = app.state::<instanceharbor::ConsoleViewer>().quit_requests()?;"));
        assert!(console.contains("if quit.recv().is_ok() {\n                    handle.exit(0);"));
    }
}
