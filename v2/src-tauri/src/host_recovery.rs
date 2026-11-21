//! Recovery of an already-running Host. Control authorization and handoff share the same owner.
use super::*;
use std::{
    os::windows::fs::OpenOptionsExt,
    time::{Duration, Instant},
};

pub(super) const PIN_FILE: &str = "recovery-pin.json";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pin {
    pub(super) key: recovery_keys::PublicKey,
    pub(super) manager: Identity,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProofPayload {
    purpose: String,
    workspace: String,
    run: Run,
    manager: Identity,
    image: String,
    key_generation: u64,
    authorization_generation: u64,
    nonce: String,
}

struct Pending {
    payload: String,
    challenge: ProofPayload,
    expires: Instant,
}

pub(super) struct Gate {
    pin: Pin,
    workspace: String,
    run: Run,
    installation: Installation,
    authorization_generation: u64,
    pending: Option<Pending>,
}

pub(super) struct Installation {
    cli: PathBuf,
    gui: PathBuf,
    digest: String,
    _files: [File; 2],
}

impl Installation {
    pub(super) fn open(cli: &Path) -> Result<Self> {
        no_redirect(cli)?;
        let cli = cli.canonicalize().map_err(fail)?;
        let gui = cli.with_file_name(crate::desktop_channel::GUI_EXECUTABLE);
        no_redirect(&gui)?;
        let cli_file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&cli)
            .map_err(fail)?;
        let gui_file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&gui)
            .map_err(fail)?;
        let cli_hash = content_revision::file_revision(&cli)?;
        let gui_hash = content_revision::file_revision(&gui)?;
        let digest = content_revision::revision(
            &serde_json::to_vec(&(&cli, cli_hash, gui_hash)).map_err(fail)?,
        )?;
        Ok(Self {
            cli,
            gui,
            digest,
            _files: [cli_file, gui_file],
        })
    }

    fn for_process(peer: &Identity) -> Result<Self> {
        let image = process_image(peer.pid)
            .and_then(|path| path.canonicalize().ok())
            .ok_or("RecoveryCandidateImageUnavailable")?;
        if !image
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("ihctl.exe"))
            || !alive(peer)?
        {
            return Err("RecoveryCandidateUnconfirmed".into());
        }
        Self::open(&image)
    }
}

impl Gate {
    pub(super) fn new(root: &Path, run: Run, pin: Pin, installation: Installation) -> Result<Self> {
        Ok(Self {
            pin,
            workspace: recovery_workspace_id(root)?,
            run,
            installation,
            authorization_generation: 0,
            pending: None,
        })
    }

    pub(super) fn console_authorizes(&self, peer: &Identity, path: &Path) -> bool {
        (self.authorizes(peer) && path == self.installation.cli)
            || (path == self.installation.gui && alive(&self.pin.manager).unwrap_or(false))
    }

    pub(super) fn authorizes(&self, peer: &Identity) -> bool {
        self.pin.manager == *peer
    }

    fn candidate(&self, root: &Path, peer: &Identity) -> Result<Installation> {
        if alive(&self.pin.manager)? {
            return Err("RecoveryManagerActive".into());
        }
        if resident_manager::published_manager(root)?.as_ref() != Some(peer) {
            return Err("RecoveryCandidateUnconfirmed".into());
        }
        Installation::for_process(peer)
    }

    fn install(&mut self, peer: &Identity, installation: Installation) -> Result<()> {
        let generation = self
            .authorization_generation
            .checked_add(1)
            .ok_or("RecoveryGenerationExhausted")?;
        self.pin.manager = peer.clone();
        self.installation = installation;
        self.authorization_generation = generation;
        self.pending = None;
        Ok(())
    }

    pub(super) fn request(
        &mut self,
        root: &Path,
        peer: &Identity,
        request: &str,
    ) -> Result<String> {
        let fields: Vec<_> = request.split_ascii_whitespace().collect();
        match fields.as_slice() {
            ["Rebind", run] if *run == self.run.id => {
                let candidate = self.candidate(root, peer)?;
                if candidate.digest != self.installation.digest {
                    return Err("HostRecoveryRequired".into());
                }
                self.install(peer, candidate)?;
                Ok("RunVerified".into())
            }
            ["RecoverChallenge", run] if *run == self.run.id => {
                let candidate = self.candidate(root, peer)?;
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.expires > Instant::now())
                {
                    return Err("RecoveryChallengePending".into());
                }
                let challenge = ProofPayload {
                    purpose: "instanceharbor-host-recovery-v1".into(),
                    workspace: self.workspace.clone(),
                    run: self.run.clone(),
                    manager: peer.clone(),
                    image: candidate.digest,
                    key_generation: self.pin.key.generation,
                    authorization_generation: self.authorization_generation,
                    nonce: recovery_keys::nonce()?,
                };
                let payload = serde_json::to_string(&challenge).map_err(fail)?;
                self.pending = Some(Pending {
                    payload: payload.clone(),
                    challenge,
                    expires: Instant::now() + Duration::from_secs(5),
                });
                Ok(payload)
            }
            ["RecoverProof", run, nonce, signature] if *run == self.run.id => {
                let pending = consume_pending(&mut self.pending, peer, nonce)?;
                let candidate = self.candidate(root, peer)?;
                if pending.challenge.authorization_generation != self.authorization_generation
                    || pending.challenge.image != candidate.digest
                    || !recovery_keys::verify(&self.pin.key, pending.payload.as_bytes(), signature)?
                {
                    return Err("RecoveryProofRejected".into());
                }
                self.install(peer, candidate)?;
                Ok("RunRecovered".into())
            }
            _ => Err("RunMismatch".into()),
        }
    }
}

fn consume_pending(pending: &mut Option<Pending>, peer: &Identity, nonce: &str) -> Result<Pending> {
    let challenge = pending
        .as_ref()
        .filter(|p| p.challenge.nonce == nonce)
        .ok_or("RecoveryChallengeUnknown")?;
    if challenge.challenge.manager != *peer {
        return Err("RecoveryCandidateUnconfirmed".into());
    }
    let claimed = pending.take().ok_or("RecoveryChallengeUnknown")?;
    if claimed.expires <= Instant::now() {
        return Err("RecoveryChallengeExpired".into());
    }
    Ok(claimed)
}

pub(super) fn approve(root: &Path, run: &Run, payload: &str) -> Result<(String, String)> {
    let challenge: ProofPayload = serde_json::from_str(payload).map_err(|_| match payload {
        "RecoveryManagerActive"
        | "RecoveryCandidateUnconfirmed"
        | "RecoveryCandidateImageUnavailable"
        | "RecoveryChallengePending"
        | "HostUnavailable"
        | "Timeout"
        | "PeerMismatch"
        | "RunMismatch" => payload,
        _ => "RecoveryChallengeUnavailable",
    })?;
    let me = identity(std::process::id())
        .map_err(fail)?
        .ok_or("ManagerIdentityUnavailable")?;
    let workspace = recovery_workspace_id(root)?;
    if challenge.purpose != "instanceharbor-host-recovery-v1"
        || challenge.workspace != workspace
        || challenge.run != *run
        || challenge.manager != me
        || challenge.image != Installation::for_process(&me)?.digest
        || challenge.nonce.len() != 64
        || !challenge.nonce.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("RecoveryChallengeUnconfirmed".into());
    }
    let key = recovery_keys::current(root, &workspace)?.ok_or("RecoveryKeyNotFound")?;
    if key.generation != challenge.key_generation {
        return Err("RecoveryKeyGenerationChanged".into());
    }
    let signature = recovery_keys::sign(root, &workspace, &key, payload.as_bytes())?;
    Ok((challenge.nonce, signature))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(peer: &Identity, expires: Instant) -> Option<Pending> {
        let challenge = ProofPayload {
            purpose: "fixture".into(),
            workspace: "fixture".into(),
            run: Run {
                id: "fixture".into(),
                host: peer.clone(),
                java: peer.clone(),
            },
            manager: peer.clone(),
            image: "fixture".into(),
            key_generation: 1,
            authorization_generation: 0,
            nonce: "nonce".into(),
        };
        Some(Pending {
            payload: String::new(),
            challenge,
            expires,
        })
    }

    #[test]
    fn same_pid_with_a_new_creation_time_cannot_claim_or_consume_proof() {
        let peer = Identity {
            pid: 42,
            created: 100,
        };
        let other = Identity {
            pid: 42,
            created: 200,
        };
        let mut challenge = pending(&peer, Instant::now() + Duration::from_secs(5));
        assert_eq!(
            consume_pending(&mut challenge, &other, "nonce")
                .err()
                .as_deref(),
            Some("RecoveryCandidateUnconfirmed")
        );
        assert!(consume_pending(&mut challenge, &peer, "nonce").is_ok());
        assert_eq!(
            consume_pending(&mut challenge, &peer, "nonce")
                .err()
                .as_deref(),
            Some("RecoveryChallengeUnknown")
        );
    }

    #[test]
    fn an_expired_proof_is_consumed_without_granting_control() {
        let peer = Identity {
            pid: 42,
            created: 100,
        };
        let mut challenge = pending(&peer, Instant::now());
        assert_eq!(
            consume_pending(&mut challenge, &peer, "nonce")
                .err()
                .as_deref(),
            Some("RecoveryChallengeExpired")
        );
        assert!(challenge.is_none());
    }
}
