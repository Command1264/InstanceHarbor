#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    close_to_tray, dispatch, dispatch_recovery_backup, dispatch_with_creation_progress, gui_singleton,
    ConsoleRequest, ConsoleViewer, CreationPhase,
};
pub mod diagnostics;

#[cfg(windows)]
pub mod installation;

#[cfg(windows)]
pub mod desktop_channel;
