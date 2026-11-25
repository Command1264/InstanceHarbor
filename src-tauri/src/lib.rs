#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    dispatch, dispatch_recovery_backup, dispatch_with_creation_progress, gui_singleton,
    ConsoleRequest, ConsoleViewer, CreationPhase,
};
pub mod about;
pub mod build_commit;
pub mod diagnostics;

#[cfg(windows)]
pub mod installation;

#[cfg(windows)]
pub mod setup_probe;

#[cfg(windows)]
pub mod desktop_channel;
