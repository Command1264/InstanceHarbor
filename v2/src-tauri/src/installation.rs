use std::{env, fs::{self, File, OpenOptions}, io, os::windows::fs::{MetadataExt, OpenOptionsExt}};

// The per-user installer holds this file exclusively during removal.
// A shared read handle travels with each GUI, CLI, Manager and Host process.
pub fn acquire() -> Result<Option<File>, String> {
    let executable = env::current_exe().map_err(|error| error.to_string())?;
    let marker = executable.with_file_name("instanceharbor.install");
    match fs::symlink_metadata(&marker) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Ok(meta) if meta.is_file() && meta.file_attributes() & 0x400 == 0 => {}
        Ok(_) => return Err("InvalidInstallationMarker".into()),
        Err(error) => return Err(format!("InstallationMarkerUnavailable: {error}")),
    }
    let file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000) // FILE_FLAG_OPEN_REPARSE_POINT
        .open(marker)
        .map_err(|error| format!("InstallationInUse: {error}"))?;
    let meta = file.metadata().map_err(|error| error.to_string())?;
    if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
        return Err("InvalidInstallationMarker".into());
    }
    Ok(Some(file))
}
