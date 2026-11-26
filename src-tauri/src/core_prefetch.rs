//! Workspace-local update candidates. A digest lease spans download, copy,
//! and collection, including when Manager and CLI run in separate processes.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub(super) struct Demand {
    pub(super) current: String,
    pub(super) registration: String,
    pub(super) profile: CoreProfile,
    pub(super) url: String,
}

type Demands = BTreeMap<String, Demand>;
const PARTIAL_GRACE: Duration = Duration::from_secs(90);

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn partial_hash(name: &str) -> Option<&str> {
    let (hash, suffix) = name.strip_suffix(".part")?.split_once('.')?;
    let (pid, timestamp) = suffix.split_once('.')?;
    (digest(hash) && pid.parse::<u32>().is_ok() && timestamp.parse::<u64>().is_ok()).then_some(hash)
}

fn partial_active(entry: &fs::DirEntry) -> Result<bool> {
    let metadata = entry.metadata().map_err(fail)?;
    Ok(metadata
        .modified()
        .map_err(fail)?
        .elapsed()
        .map_or(true, |age| age < PARTIAL_GRACE))
}

pub(super) struct Store(PathBuf);

/// Batch update candidates that the prefetch store does not hold (#60).
const BATCH_STORE: &str = "core-batch-candidates";

impl Store {
    fn open(root: &Path, create: bool) -> Result<Option<Self>> {
        Self::open_named(root, "core-prefetch", create)
    }

    fn open_named(root: &Path, name: &str, create: bool) -> Result<Option<Self>> {
        let stage = child_directory(root, "staging")?;
        let directory = stage.join(name);
        if create {
            match fs::create_dir(&directory) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(fail(error)),
            }
        } else if !present(&directory)? {
            return Ok(None);
        }
        Ok(Some(Self(child_directory(&stage, name)?)))
    }

    /// The candidate store of batch updates. Each SHA-256 is downloaded once and
    /// reused by later items, until `collect_batch` finds no active item needs it.
    pub(super) fn batch(root: &Path) -> Result<Self> {
        Self::open_named(root, BATCH_STORE, true)?.ok_or_else(|| "CoreDownloadCacheInvalid".into())
    }

    pub(super) fn enabled(root: &Path) -> Result<Option<Self>> {
        if presentation::core_prefetch_enabled(root)? {
            Self::open(root, true)
        } else {
            Ok(None)
        }
    }

    fn demands(&self) -> Result<Demands> {
        let path = self.0.join("demands.json");
        if present(&path)? {
            read(&path)
        } else {
            Ok(BTreeMap::new())
        }
    }

    fn save_demands(&self, demands: &Demands) -> Result<()> {
        let path = self.0.join("demands.json");
        core_maintenance::write_durable(&path, demands, present(&path)?)
    }

    pub(super) fn lease(&self, hash: &str) -> Result<Lease> {
        if !digest(hash) {
            return Err("CoreCandidateUnverified".into());
        }
        let guard = command_lock(&self.0, &format!("{hash}.lock"), false).map_err(|error| {
            if error == "ManagerBusy" {
                "CoreCandidateBusy".into()
            } else {
                error
            }
        })?;
        Ok(Lease {
            path: self.0.join(format!("{hash}.jar")),
            _guard: guard,
        })
    }

    pub(super) fn prepared(
        &self,
        name: &str,
        current: &str,
        registration: &str,
        hash: &str,
    ) -> Result<Option<Demand>> {
        Ok(self.demands()?.remove(name).filter(|demand| {
            demand.current == current
                && demand.registration == registration
                && demand.profile.sha256 == hash
        }))
    }

    fn collect(&self, demands: &Demands) -> Result<()> {
        let needed: BTreeSet<_> = demands
            .values()
            .map(|item| item.profile.sha256.as_str())
            .collect();
        self.collect_except(&needed)
    }

    /// Removes candidates and stale partial downloads outside `needed`. A file
    /// whose lease is held elsewhere is left for a later pass.
    fn collect_except(&self, needed: &BTreeSet<&str>) -> Result<()> {
        for entry in fs::read_dir(&self.0).map_err(fail)? {
            let entry = entry.map_err(fail)?;
            let name = entry.file_name();
            let Some(filename) = name.to_str() else {
                continue;
            };
            let hash = if let Some(hash) = filename.strip_suffix(".jar") {
                if needed.contains(hash) {
                    continue;
                }
                hash
            } else if let Some(hash) = partial_hash(filename) {
                if partial_active(&entry)? {
                    continue;
                }
                hash
            } else {
                continue;
            };
            if !digest(hash) {
                continue;
            }
            match self.lease(hash) {
                Ok(lease) => remove_owned(&self.0.join(name), &lease)?,
                Err(error) if error == "CoreCandidateBusy" => (),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

/// Removes batch candidates that no active batch item still needs.
pub(super) fn collect_batch(root: &Path, needed: &BTreeSet<&str>) -> Result<()> {
    match Store::open_named(root, BATCH_STORE, false)? {
        Some(store) => store.collect_except(needed),
        None => Ok(()),
    }
}

fn present(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            no_redirect(path)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(fail(error)),
    }
}

pub(super) struct Lease {
    path: PathBuf,
    _guard: File,
}

fn remove_owned(path: &Path, _lease: &Lease) -> Result<()> {
    if present(path)? {
        fs::remove_file(path).map_err(fail)?;
    }
    Ok(())
}

impl Lease {
    pub(super) fn ready(&self, hash: &str) -> Result<Option<&Path>> {
        if present(&self.path)? && content_revision::file_revision(&self.path)? == hash {
            Ok(Some(&self.path))
        } else {
            Ok(None)
        }
    }

    pub(super) fn ensure(
        &self,
        hash: &str,
        fetch: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<&Path> {
        if self.ready(hash)?.is_some() {
            return Ok(&self.path);
        }
        let directory = self.path.parent().ok_or("CoreDownloadCacheInvalid")?;
        for entry in fs::read_dir(directory).map_err(fail)? {
            let entry = entry.map_err(fail)?;
            if entry.file_name().to_str().and_then(partial_hash) == Some(hash)
                && partial_active(&entry)?
            {
                return Err("CoreCandidateBusy".into());
            }
        }
        // A downloader can outlive a killed Manager. A replacement Manager
        // never reuses a filename that the previous downloader could write.
        let part = self
            .path
            .with_extension(format!("{}.{}.part", std::process::id(), epoch_ms()));
        File::create_new(&part).map_err(fail)?;
        let result = fetch(&part).and_then(|()| {
            no_redirect(&part)?;
            if content_revision::file_revision(&part)? != hash {
                return Err("CoreArtifactDigestMismatch".into());
            }
            OpenOptions::new()
                .write(true)
                .open(&part)
                .and_then(|file| file.sync_all())
                .map_err(fail)?;
            core_maintenance::move_durable(&part, &self.path, true)
        });
        if result.is_err() {
            remove_owned(&part, self)?;
        }
        result.map(|()| self.path.as_path())
    }
}

fn demand(row: &Value) -> Option<Demand> {
    let field = |key| row[key].as_str().map(str::to_owned);
    let candidate = &row["candidate"];
    let sha256 = candidate["sha256"].as_str()?.to_owned();
    let current = field("sha256")?;
    let registration = field("registrationRevision")?;
    if row["status"] != "UpdateAvailable"
        || !row["family"].as_str().is_some_and(core_source::prefetch_family)
        || !digest(&sha256)
        || !digest(&current)
        || !digest(&registration)
    {
        return None;
    }
    Some(Demand {
        current,
        registration,
        profile: CoreProfile { unverified: false, java: None,
            family: field("family")?,
            minecraft: field("minecraft")?,
            build: candidate["build"].as_str()?.to_owned(),
            sha256,
        },
        url: candidate["url"].as_str()?.to_owned(),
    })
}

fn refresh(root: &Path, names: &[String], demands: &mut Demands) -> Result<()> {
    for chunk in names.chunks(core_source::MAX_TARGETS) {
        let reply = core_source::check(root, chunk)?;
        for row in reply["rows"].as_array().ok_or("CoreSourceUnavailable")? {
            let Some(name) = row["name"].as_str() else {
                continue;
            };
            if let Some(candidate) = demand(row) {
                demands.insert(name.into(), candidate);
            } else if matches!(row["status"].as_str(), Some("Current" | "NoStableBuild")) {
                demands.remove(name);
            }
        }
    }
    Ok(())
}

fn outstanding(root: &Path, config: &Instance, item: &Demand) -> bool {
    if config
        .core_profile
        .as_ref()
        .is_none_or(|profile| profile.sha256 == item.current)
    {
        return true;
    }
    // A failed read is not proof that an instance no longer needs a candidate.
    let Ok(directory) = instance_path(root, &config.name) else {
        return true;
    };
    let jar = core_artifact::jar(&directory, config);
    if no_redirect(&jar).is_err() {
        return true;
    }
    let Ok(hash) = content_revision::file_revision(&jar) else {
        return true;
    };
    if hash == item.profile.sha256 {
        return false;
    }
    let installed = core_maintenance::trusted_profile(config, hash);
    !installed.is_some_and(|current| {
        current.family == item.profile.family
            && current.minecraft == item.profile.minecraft
            && current
                .build
                .parse::<u64>()
                .ok()
                .zip(item.profile.build.parse::<u64>().ok())
                .is_some_and(|(current, candidate)| current >= candidate)
    })
}

pub(super) fn status(root: &Path, name: &str, current: &str) -> Result<Option<CoreProfile>> {
    if !presentation::core_prefetch_enabled(root)? {
        return Ok(None);
    }
    let Some(store) = Store::open(root, false)? else {
        return Ok(None);
    };
    let Some(item) = store.demands()?.remove(name) else {
        return Ok(None);
    };
    if !digest(&item.profile.sha256)
        || item.current != current
        || registration_revision(root, name)? != item.registration
    {
        return Ok(None);
    }
    let path = store.0.join(format!("{}.jar", item.profile.sha256));
    // Presentation is a hint. Installation always takes the lease and hashes the bytes.
    Ok(present(&path)?.then_some(item.profile))
}

pub(super) fn supervise(root: &Path) {
    let mut next_check = None;
    loop {
        if let Err(error) = cycle(root, &mut next_check) {
            eprintln!("Core prefetch: {error}");
        }
        thread::sleep(Duration::from_secs(3));
    }
}

/// How long the next source check waits: the regular interval, or a shorter but still
/// spaced retry after a failed check. Without a deadline after a failure, every 3 s cycle
/// hashed each core and contacted the sources again (#118).
const CHECK_INTERVAL: Duration = Duration::from_secs(120);
const CHECK_RETRY: Duration = Duration::from_secs(30);

fn schedule_check(
    next_check: &mut Option<(Instant, Vec<String>)>,
    names: Vec<String>,
    outcome: Result<()>,
    now: Instant,
) -> Result<()> {
    let wait = if outcome.is_ok() { CHECK_INTERVAL } else { CHECK_RETRY };
    *next_check = Some((now + wait, names));
    outcome
}

fn cycle(root: &Path, next_check: &mut Option<(Instant, Vec<String>)>) -> Result<()> {
    let enabled = presentation::core_prefetch_enabled(root)?;
    if !enabled {
        *next_check = None;
    }
    let Some(store) = Store::open(root, enabled)? else {
        return Ok(());
    };
    let mut demands = store.demands()?;
    let previous = demands.clone();
    if enabled {
        let mut active = BTreeMap::new();
        for config in presentation::registry_records(root)? {
            if !config.removed && admitted(root, &config.name)? {
                active.insert(config.name.clone(), config);
            }
        }
        let names: Vec<_> = active.keys().cloned().collect();
        let due = next_check
            .as_ref()
            .is_none_or(|(deadline, previous)| Instant::now() >= *deadline || previous != &names);
        if due {
            let outcome = refresh(root, &names, &mut demands);
            schedule_check(next_check, names, outcome, Instant::now())?;
        }
        demands.retain(|name, item| {
            active
                .get(name)
                .is_some_and(|config| outstanding(root, config, item))
        });
        if demands != previous {
            store.save_demands(&demands)?;
        }
        store.collect(&demands)?;
        // Retry missing candidates on the source-check interval. Each successful
        // digest is downloaded once even when many instances request it.
        if due {
            let mut seen = BTreeSet::new();
            for item in demands
                .values()
                .filter(|item| seen.insert(&item.profile.sha256))
            {
                if !presentation::core_prefetch_enabled(root)? {
                    break;
                }
                let Ok(lease) = store.lease(&item.profile.sha256) else {
                    continue;
                };
                if let Err(error) = lease.ensure(&item.profile.sha256, |path| {
                    core_source::download_prefetch(&item.url, path, &item.profile.sha256)?;
                    if presentation::core_prefetch_enabled(root)? {
                        Ok(())
                    } else {
                        Err("CorePrefetchDisabled".into())
                    }
                }) {
                    eprintln!("Core candidate preparation failed: {error}");
                }
            }
        }
    } else {
        demands.clear();
        if !previous.is_empty() {
            store.save_demands(&demands)?;
        }
        store.collect(&demands)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_source_check_waits_before_the_next_attempt() {
        let now = Instant::now();
        let names = vec!["agent".to_owned()];
        let mut next = None;
        assert!(schedule_check(&mut next, names.clone(), Err("CoreSourceUnavailable".into()), now).is_err());
        assert_eq!(next, Some((now + CHECK_RETRY, names.clone())), "a failure still sets a deadline");
        schedule_check(&mut next, names.clone(), Ok(()), now).unwrap();
        assert_eq!(next, Some((now + CHECK_INTERVAL, names)));
    }

    #[test]
    fn candidate_is_shared_across_reopens_and_collected_only_after_last_reader() {
        let tmp =
            std::env::temp_dir().join(format!("ih-prefetch-{}-{}", std::process::id(), epoch_ms()));
        fs::create_dir(&tmp).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(tmp.clone());
        fs::create_dir(tmp.join("staging")).unwrap();
        let tmp = safe_directory(&tmp).unwrap();
        assert!(Store::enabled(&tmp).unwrap().is_none());
        assert!(!tmp.join("staging/core-prefetch").exists());
        let store = Store::open(&tmp, true).unwrap().unwrap();
        let bytes = b"verified candidate";
        let hash = content_revision::revision(bytes).unwrap();
        let item = Demand {
            current: "1".repeat(64),
            registration: "2".repeat(64),
            profile: CoreProfile { unverified: false, java: None,
                family: "PAPER".into(),
                minecraft: "26.3".into(),
                build: "253".into(),
                sha256: hash.clone(),
            },
            url: String::new(),
        };
        let mut demands = Demands::from([("a".into(), item.clone()), ("b".into(), item)]);
        store.save_demands(&demands).unwrap();
        let lease = store.lease(&hash).unwrap();
        assert!(lease
            .ensure(&hash, |p| {
                fs::write(p, b"incomplete").unwrap();
                Err("DownloadInterrupted".into())
            })
            .is_err());
        assert!(lease.ready(&hash).unwrap().is_none());
        let jar = lease
            .ensure(&hash, |p| fs::write(p, bytes).map_err(fail))
            .unwrap()
            .to_owned();
        drop(lease);
        let reopened = Store::open(&tmp, false).unwrap().unwrap();
        assert_eq!(reopened.demands().unwrap().len(), 2);
        let lease = reopened.lease(&hash).unwrap();
        lease
            .ensure(&hash, |_| panic!("must reuse prepared bytes"))
            .unwrap();
        demands.remove("a");
        reopened.collect(&demands).unwrap();
        assert!(jar.exists());
        demands.remove("b");
        reopened.collect(&demands).unwrap();
        assert!(jar.exists(), "open install lease protects the bytes");
        drop(lease);
        reopened.collect(&demands).unwrap();
        assert!(!jar.exists());
    }
}
