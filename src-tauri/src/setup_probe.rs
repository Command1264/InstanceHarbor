//! Helper that Setup runs from its own temporary folder while it updates an installation
//! (`ihctl setup-probe ...`). It reads server states and asks for exit only through the
//! installed `ihctl.exe`, so the installed version's own rules apply, and prints one short
//! word, because NSIS reads at most 1024 characters of output.
//!
//! Whether Setup may continue is never decided here: Setup waits until no process holds
//! the installation marker. The server count only explains why it waits.
use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `servers INSTALL WORKSPACE` prints how many servers are not known to be stopped, or
/// `unknown`. `close INSTALL WORKSPACE current|legacy` asks the installed version to end
/// its GUI and Manager and prints the outcome (see [`close_outcome`]).
pub fn run(arguments: &[String]) -> Result<String, String> {
    match arguments {
        [verb, install, workspace] if verb == "servers" => {
            let workspace = Path::new(workspace);
            if !workspace.exists() {
                return Ok("0".into());
            }
            let list = installed(Path::new(install), workspace, "list");
            Ok(match list.filter(|reply| reply.success).and_then(|reply| not_stopped(&reply.stdout)) {
                Some(count) => count.to_string(),
                None => "unknown".into(),
            })
        }
        [verb, install, workspace, generation] if verb == "close" => {
            let legacy = match generation.as_str() {
                "current" => false,
                "legacy" => true,
                _ => return Err("InvalidSetupProbe".into()),
            };
            if !Path::new(workspace).exists() {
                return Ok("none".into());
            }
            // 1.0.0 and 1.0.1 have no app-exit: they only end the Manager, and the user
            // closes the GUI window. Unknown verbs are never sent to an older version.
            let verb = if legacy { "manager-exit" } else { "app-exit" };
            let reply = installed(Path::new(install), Path::new(workspace), verb);
            Ok(close_outcome(reply.as_ref(), legacy).into())
        }
        _ => Err("InvalidSetupProbe".into()),
    }
}

struct Reply {
    success: bool,
    stdout: String,
    stderr: String,
}

fn installed(install: &Path, workspace: &Path, verb: &str) -> Option<Reply> {
    let output = Command::new(install.join("ihctl.exe"))
        .arg(workspace)
        .arg(verb)
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    Some(Reply {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Servers in `ihctl WORKSPACE list` output that are not `Stopped`, the rule tray Quit
/// uses: starting, running, undecided and unreadable states all count.
fn not_stopped(list: &str) -> Option<u32> {
    let rows: Value = serde_json::from_str(list.trim()).ok()?;
    Some(rows.as_array()?.iter().filter(|row| row["state"] != "Stopped").count() as u32)
}

/// `closed`/`legacy`: the Manager is ending (with its GUIs for `closed`). `none`/`legacy-none`:
/// no Manager runs for the workspace. `busy`: batch or core-update work blocks exit.
/// `failed`: anything else, including an installed CLI that could not run.
fn close_outcome(reply: Option<&Reply>, legacy: bool) -> &'static str {
    let Some(reply) = reply else { return "failed" };
    let code = reply.stderr.trim();
    match (reply.success, legacy) {
        (true, false) => "closed",
        (true, true) => "legacy",
        (false, _) if code.starts_with("ManagerHasActiveWork") => "busy",
        (false, false) if code.starts_with("ManagerUnavailable") => "none",
        (false, true) if code.starts_with("ManagerUnavailable") => "legacy-none",
        (false, _) => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(success: bool, stderr: &str) -> Reply {
        Reply { success, stdout: String::new(), stderr: stderr.into() }
    }

    #[test]
    fn counts_every_listed_server_that_is_not_stopped() {
        let list = r#"[{"name":"a","state":"Listening","run":{"id":"x"}},{"name":"b","state":"Stopped"},
            {"name":"c","state":"StateUnconfirmed","stateIssue":{"state":"Stopped"}},{"name":"d"}]"#;
        assert_eq!(not_stopped(list), Some(3));
        assert_eq!(not_stopped("[]\r\n"), Some(0));
        assert_eq!(not_stopped(r#"{"rows":[]}"#), None);
        assert_eq!(not_stopped("not json"), None);
    }

    #[test]
    fn close_reports_what_the_installed_version_did() {
        assert_eq!(close_outcome(Some(&reply(true, "")), false), "closed");
        assert_eq!(close_outcome(Some(&reply(true, "")), true), "legacy");
        assert_eq!(close_outcome(Some(&reply(false, "ManagerUnavailable\r\n")), false), "none");
        assert_eq!(close_outcome(Some(&reply(false, "ManagerUnavailable")), true), "legacy-none");
        assert_eq!(close_outcome(Some(&reply(false, "ManagerHasActiveWork")), false), "busy");
        assert_eq!(close_outcome(Some(&reply(false, "PeerMismatch")), false), "failed");
        assert_eq!(close_outcome(None, false), "failed");
    }

    #[test]
    fn rejects_unknown_requests_and_needs_no_workspace() {
        let missing = std::env::temp_dir().join(format!("ih-setup-probe-missing-{}", std::process::id()));
        let missing = missing.to_string_lossy().into_owned();
        let args = |list: &[&str]| list.iter().map(|value| value.to_string()).collect::<Vec<_>>();
        assert_eq!(run(&args(&["servers", "X:\\none", &missing])).unwrap(), "0");
        assert_eq!(run(&args(&["close", "X:\\none", &missing, "current"])).unwrap(), "none");
        assert!(run(&args(&["close", "X:\\none", &missing, "other"])).is_err());
        assert!(run(&args(&["stop-all", "X:\\none", &missing])).is_err());
    }
}
