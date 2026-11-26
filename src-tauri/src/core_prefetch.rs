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
    /// Purpur (#113): the official MD5 that names this candidate. Its SHA-256
    /// is known only after the first verified download, so `profile.sha256`
    /// stays empty here and the store records it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) md5: Option<String>,
}

impl Demand {
    /// The name of this candidate in the store: its SHA-256, or its MD5.
    fn key(&self) -> &str {
        self.md5.as_deref().unwrap_or(&self.profile.sha256)
    }
}

type Demands = BTreeMap<String, Demand>;
const PARTIAL_GRACE: Duration = Duration::from_secs(90);

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// A lowercase MD5, the name of a Purpur candidate before download (#113).
pub(super) fn md5_key(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn key(value: &str) -> bool {
    digest(value) || md5_key(value)
}

fn partial_hash(name: &str) -> Option<&str> {
    let (hash, suffix) = name.strip_suffix(".part")?.split_once('.')?;
    let (pid, timestamp) = suffix.split_once('.')?;
    (key(hash) && pid.parse::<u32>().is_ok() && timestamp.parse::<u64>().is_ok()).then_some(hash)
}

/// The file that records the SHA-256 of the verified download of an MD5.
fn record_name(md5: &str) -> String {
    format!("{md5}.sha256")
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

    /// Takes the lease of a candidate named by its SHA-256, or by its MD5.
    pub(super) fn lease(&self, hash: &str) -> Result<Lease> {
        if !key(hash) {
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
        let needed: BTreeSet<_> = demands.values().map(Demand::key).collect();
        self.collect_except(&needed)
    }

    /// The SHA-256 recorded for the verified download of `md5` (#113).
    pub(super) fn resolved(&self, md5: &str) -> Result<Option<String>> {
        if !md5_key(md5) {
            return Err("CoreCandidateUnverified".into());
        }
        let record = self.0.join(record_name(md5));
        if !present(&record)? {
            return Ok(None);
        }
        let hash: Option<String> = core_maintenance::read_optional(&record)?;
        Ok(hash.filter(|hash| digest(hash)))
    }

    /// Purpur (#113): the candidate named by its official MD5. Only the first
    /// download is compared with the MD5. Its SHA-256 is then recorded, and
    /// every later use checks the stored bytes against that SHA-256 only.
    /// Returns the lease of the stored bytes, their SHA-256, and whether they
    /// were already stored.
    pub(super) fn ensure_md5(
        &self,
        md5: &str,
        fetch: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<(Lease, String, bool)> {
        if !md5_key(md5) {
            return Err("CoreCandidateUnverified".into());
        }
        // The MD5 lease is held until the bytes and their record are stored,
        // so one MD5 is downloaded once even when several processes need it.
        let named = self.lease(md5)?;
        if let Some(hash) = self.resolved(md5)? {
            let lease = self.lease(&hash)?;
            if lease.ready(&hash)?.is_some() {
                return Ok((lease, hash, true));
            }
        }
        for entry in fs::read_dir(&self.0).map_err(fail)? {
            let entry = entry.map_err(fail)?;
            if entry.file_name().to_str().and_then(partial_hash) == Some(md5)
                && partial_active(&entry)?
            {
                return Err("CoreCandidateBusy".into());
            }
        }
        let part = named
            .path
            .with_extension(format!("{}.{}.part", std::process::id(), epoch_ms()));
        File::create_new(&part).map_err(fail)?;
        let result = (|| {
            fetch(&part)?;
            no_redirect(&part)?;
            if content_revision::file_md5(&part)? != md5 {
                return Err("CoreArtifactDigestMismatch".into());
            }
            OpenOptions::new()
                .write(true)
                .open(&part)
                .and_then(|file| file.sync_all())
                .map_err(fail)?;
            let hash = content_revision::file_revision(&part)?;
            let lease = self.lease(&hash)?;
            if lease.ready(&hash)?.is_some() {
                remove_owned(&part, &named)?;
            } else {
                core_maintenance::move_durable(&part, &lease.path, true)?;
            }
            let record = self.0.join(record_name(md5));
            core_maintenance::write_durable(&record, &hash, present(&record)?)?;
            Ok((lease, hash, false))
        })();
        if result.is_err() {
            remove_owned(&part, &named)?;
        }
        result
    }

    /// Removes candidates and stale partial downloads outside `needed`. A file
    /// whose lease is held elsewhere is left for a later pass. An MD5 in
    /// `needed` also keeps the bytes that its record names.
    fn collect_except(&self, needed: &BTreeSet<&str>) -> Result<()> {
        let mut kept: BTreeSet<String> = needed.iter().map(|key| (*key).to_owned()).collect();
        for md5 in needed.iter().filter(|key| md5_key(key)) {
            if let Some(hash) = self.resolved(md5)? {
                kept.insert(hash);
            }
        }
        for entry in fs::read_dir(&self.0).map_err(fail)? {
            let entry = entry.map_err(fail)?;
            let name = entry.file_name();
            let Some(filename) = name.to_str() else {
                continue;
            };
            let hash = if let Some(hash) = filename.strip_suffix(".jar") {
                if kept.contains(hash) {
                    continue;
                }
                hash
            } else if let Some(md5) = filename.strip_suffix(".sha256").filter(|md5| md5_key(md5)) {
                if kept.contains(md5) {
                    continue;
                }
                md5
            } else if let Some(hash) = partial_hash(filename) {
                if partial_active(&entry)? {
                    continue;
                }
                hash
            } else {
                continue;
            };
            if !key(hash) {
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
    // A Purpur candidate is named by its official MD5 (#113).
    let md5 = (row["family"] == "PURPUR")
        .then(|| candidate["md5"].as_str().map(str::to_owned))
        .flatten();
    let sha256 = match &md5 {
        Some(md5) if md5_key(md5) => String::new(),
        Some(_) => return None,
        None => candidate["sha256"].as_str().filter(|hash| digest(hash))?.to_owned(),
    };
    let current = field("sha256")?;
    let registration = field("registrationRevision")?;
    if row["status"] != "UpdateAvailable"
        || !row["family"].as_str().is_some_and(core_source::prefetch_family)
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
        md5,
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
    let Some(mut item) = store.demands()?.remove(name) else {
        return Ok(None);
    };
    if let Some(md5) = &item.md5 {
        match store.resolved(md5)? {
            Some(hash) => item.profile.sha256 = hash,
            None => return Ok(None),
        }
    }
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
            for item in demands.values().filter(|item| seen.insert(item.key())) {
                if !presentation::core_prefetch_enabled(root)? {
                    break;
                }
                if let Some(md5) = &item.md5 {
                    if let Err(error) = store.ensure_md5(md5, |path| {
                        core_source::download_purpur_prefetch(&item.url, path, md5, &item.profile)?;
                        if presentation::core_prefetch_enabled(root)? {
                            Ok(())
                        } else {
                            Err("CorePrefetchDisabled".into())
                        }
                    }) {
                        eprintln!("Core candidate preparation failed: {error}");
                    }
                    continue;
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
                md5: None,
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

    // #113: a Purpur candidate is named by its official MD5. Only bytes that
    // match it are stored, with their SHA-256 recorded. Later uses check that
    // SHA-256 and download nothing; collection follows the MD5 demand.
    #[test]
    fn purpur_candidate_is_named_by_md5_and_reused_by_its_recorded_sha256() {
        let tmp = std::env::temp_dir()
            .join(format!("ih-prefetch-md5-{}-{}", std::process::id(), epoch_ms()));
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
        let store = Store::open(&tmp, true).unwrap().unwrap();
        let bytes = b"purpur candidate";
        let probe = tmp.join("probe.jar");
        fs::write(&probe, bytes).unwrap();
        let md5 = content_revision::file_md5(&probe).unwrap();
        fs::remove_file(&probe).unwrap();
        let hash = content_revision::revision(bytes).unwrap();
        let downloads = std::cell::Cell::new(0);
        let fetch = |content: &'static [u8]| {
            let downloads = &downloads;
            move |path: &Path| {
                downloads.set(downloads.get() + 1);
                fs::write(path, content).map_err(fail)
            }
        };
        let names = || -> Vec<String> {
            fs::read_dir(&store.0).unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| !name.ends_with(".lock")).collect()
        };
        match store.ensure_md5(&md5, fetch(b"tampered candidate")) {
            Err(error) => assert_eq!(error, "CoreArtifactDigestMismatch"),
            Ok(_) => panic!("bytes that do not match the MD5 are never stored"),
        }
        assert!(store.resolved(&md5).unwrap().is_none());
        assert!(names().is_empty(), "a failed download leaves nothing behind: {:?}", names());
        let (lease, recorded, reused) = store.ensure_md5(&md5, fetch(bytes)).unwrap();
        assert_eq!((recorded.as_str(), reused), (hash.as_str(), false));
        assert_eq!(store.resolved(&md5).unwrap().as_deref(), Some(hash.as_str()));
        drop(lease);
        let (lease, _, reused) = store.ensure_md5(&md5, |_| panic!("must reuse stored bytes")).unwrap();
        assert!(reused);
        drop(lease);
        assert_eq!(downloads.get(), 2);
        // Bytes changed after the record are not used: they are downloaded again.
        let jar = store.0.join(format!("{hash}.jar"));
        fs::write(&jar, b"changed later").unwrap();
        let (lease, _, reused) = store.ensure_md5(&md5, fetch(bytes)).unwrap();
        assert!(!reused);
        assert_eq!(fs::read(&jar).unwrap(), bytes);
        drop(lease);
        assert!(store.ensure_md5(&md5.to_uppercase(), |_| panic!("not an MD5 key")).is_err());

        let row = json!({"name":"a","status":"UpdateAvailable","family":"PURPUR","minecraft":"26.3",
            "sha256":"1".repeat(64),"registrationRevision":"2".repeat(64),
            "candidate":{"build":"2660","md5":md5,"url":"https://api.purpurmc.org/v2/purpur/26.3/2660/download"}});
        let item = demand(&row).unwrap();
        assert_eq!((item.key(), item.profile.sha256.as_str(), item.profile.build.as_str()), (md5.as_str(), "", "2660"));
        let mut upper = row.clone();
        upper["candidate"]["md5"] = json!(md5.to_uppercase());
        assert!(demand(&upper).is_none());
        let mut demands = Demands::from([("a".into(), item)]);
        store.collect(&demands).unwrap();
        assert!(jar.exists() && store.0.join(record_name(&md5)).exists());
        demands.clear();
        store.collect(&demands).unwrap();
        assert!(names().is_empty(), "the bytes and their record go together: {:?}", names());
    }
}
