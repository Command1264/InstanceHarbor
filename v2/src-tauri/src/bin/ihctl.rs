use instanceharbor_v2::{dispatch, dispatch_recovery_backup};
use std::io::{self, IsTerminal, Read};

struct Password(Vec<u8>);

impl Drop for Password {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}

fn with_password(arguments: &[String]) -> Result<serde_json::Value, String> {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err("RecoveryPasswordNeedsSecurePrompt".into());
    }
    let mut password = Password(Vec::with_capacity(1025));
    stdin.lock().take(1025).read_to_end(&mut password.0).map_err(|_| "RecoveryPasswordReadFailed")?;
    if password.0.is_empty() || password.0.len() > 1024 {
        return Err("RecoveryPasswordInvalidLength".into());
    }
    dispatch_recovery_backup(arguments, &password.0)
}

fn main() {
    #[cfg(windows)]
    let _installation = match instanceharbor_v2::installation::acquire() {
        Ok(lease) => lease,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let mut arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.as_slice() == ["--version"] {
        println!("{} CLI {}", instanceharbor_v2::desktop_channel::TITLE, env!("CARGO_PKG_VERSION"));
        return;
    }
    let plain_list = arguments.len() == 2 && arguments[1] == "list";
    if plain_list {
        arguments.push("--details".into());
    }
    let result = if arguments.get(1).is_some_and(|verb| matches!(
        verb.as_str(), "recovery-backup-export" | "recovery-backup-inspect" | "recovery-backup-restore"
    )) {
        with_password(&arguments)
    } else {
        dispatch(&arguments)
    };
    match result {
        Ok(result) => {
            if plain_list {
                if let Some(issues) = result["issues"].as_array() {
                    for issue in issues {
                        eprintln!("Instance registration unavailable: {issue}");
                    }
                }
                println!("{}", result["rows"]);
            } else {
                println!("{result}");
            }
            let verdict = match arguments.get(1).map(String::as_str) {
                Some("doctor" | "defaults-diff") => Some("ok"),
                Some("admin-verify") => Some("verified"),
                _ => None,
            };
            if verdict.is_some_and(|key| result[key] != true) {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
