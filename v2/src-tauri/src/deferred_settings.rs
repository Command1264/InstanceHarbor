use super::*;
use serde::de::DeserializeOwned;

fn directory(root: &Path, create: bool) -> Result<Option<PathBuf>> {
    let path = root.join("deferred-settings");
    if create && !path.exists() {
        fs::create_dir(&path).map_err(|error| file_error("create-deferred-directory", &path, error))?;
    }
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(child_directory(root, "deferred-settings")?))
}

fn path(root: &Path, kind: &str, name: &str, create: bool) -> Result<Option<PathBuf>> {
    if !matches!(kind, "instance" | "world") || !token(name) {
        return Err("InvalidDeferredSettingsTarget".into());
    }
    Ok(directory(root, create)?.map(|folder| folder.join(format!("{kind}-{name}.json"))))
}

pub(super) fn load<T: DeserializeOwned>(root: &Path, kind: &str, name: &str) -> Result<Option<T>> {
    let Some(path) = path(root, kind, name, false)? else { return Ok(None) };
    if !path.exists() {
        return Ok(None);
    }
    no_redirect(&path)?;
    read(&path).map(Some)
}

pub(super) fn store(root: &Path, kind: &str, name: &str, value: &impl Serialize) -> Result<()> {
    let path = path(root, kind, name, true)?.ok_or("DeferredSettingsDirectoryMissing")?;
    if path.exists() {
        no_redirect(&path)?;
    }
    policy_settings::replace_file(&path, &serde_json::to_vec_pretty(value).map_err(fail)?)
}

pub(super) fn clear(root: &Path, kind: &str, name: &str) -> Result<()> {
    let Some(path) = path(root, kind, name, false)? else { return Ok(()) };
    if path.exists() {
        no_redirect(&path)?;
        fs::remove_file(&path).map_err(|error| file_error("clear-deferred-settings", &path, error))?;
    }
    Ok(())
}

pub(super) fn has_pending(root: &Path, name: &str) -> Result<bool> {
    for kind in ["world", "instance"] {
        if let Some(path) = path(root, kind, name, false)? {
            if path.exists() {
                no_redirect(&path)?;
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) fn state(root: &Path, kind: &str, name: &str) -> Result<Option<String>> {
    Ok(load::<Value>(root, kind, name)?
        .map(|pending| pending["state"].as_str().unwrap_or("Queued").to_owned()))
}

fn mark_conflict(root: &Path, kind: &str, name: &str, error: &str) -> Result<()> {
    let Some(mut pending) = load::<Value>(root, kind, name)? else { return Ok(()) };
    if pending["state"] == "Conflict" && pending["error"] == error { return Ok(()); }
    pending["state"] = json!("Conflict");
    pending["error"] = json!(error);
    store(root, kind, name, &pending)
}

// Call under instance-{name}.lock. A failed/conflicting intent remains durable,
// and a subsequent start must not bypass it.
pub(super) fn apply_locked(root: &Path, name: &str) -> Result<()> {
    agent_rebuild::assert_idle(root, name)?;
    if state(root, "world", name)?.as_deref() == Some("Conflict")
        || state(root, "instance", name)?.as_deref() == Some("Conflict") {
        return Err("DeferredSettingsConflict".into());
    }
    if let Err(error) = policy_settings::apply_deferred_world(root, name) {
        if matches!(error.as_str(), "DeferredWorldConflict" | "DeferredWorldCoreChanged"
            | "SettingsConflict") || error.starts_with("WorldSettingsPortUnconfirmed") {
            mark_conflict(root, "world", name, &error)?;
        }
        return Err(error);
    }
    if let Err(error) = presentation::apply_deferred_instance(root, name) {
        if matches!(error.as_str(), "DeferredInstanceConflict" | "SettingsConflict") {
            mark_conflict(root, "instance", name, &error)?;
        }
        return Err(error);
    }
    if state(root, "world", name)?.as_deref() == Some("Applying") {
        clear(root, "world", name)?;
    }
    Ok(())
}
