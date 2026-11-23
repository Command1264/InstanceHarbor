//! Preview and journaled publication of servers created by the GUI.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum CreationPhase {
    Checking,
    DownloadingCore,
    DownloadingMojang,
    CoreBytes { received: u64, total: u64 },
    MojangBytes { received: u64, total: u64 },
    Creating,
}

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_NEIGHBORS: usize = 512;
const MAX_NEIGHBOR_JARS: usize = 32;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Draft {
    name: String,
    parent: String,
    port: u16,
    source: Source,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Source {
    Local { path: String },
    Download { family: String, minecraft: String },
    Official { family: String, minecraft: String, build: String, sha256: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    draft: Draft,
    revision: String,
}

enum Assets {
    Local { core: PathBuf, mojang: Option<PathBuf> },
    Download { mojang_url: &'static str },
    Official,
}

struct Inspection {
    parent: PathBuf,
    target: PathBuf,
    core: CoreProfile,
    assets: Assets,
    scope: Scope,
    revision: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Scope {
    original_revision: String,
    added_revision: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    stage: String,
    config: Instance,
    scope: Scope,
    hashes: BTreeMap<String, String>,
}

fn parse<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T> {
    if raw.len() > MAX_REQUEST_BYTES {
        return Err("InvalidGuiCreationArguments".into());
    }
    serde_json::from_str(raw).map_err(|_| "InvalidGuiCreationArguments".into())
}

fn entry_present(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(fail(error)),
    }
}

fn matching_jar(candidate: &Path, expected: &str) -> bool {
    let Ok(local) = prepare::local_jar(&path_format::display(candidate)) else { return false };
    content_revision::file_revision(&local).is_ok_and(|hash| hash == expected)
}

fn local_mojang(root: &Path, core: &Path, profile: &CoreProfile) -> Result<Option<PathBuf>> {
    let Some(expected) = profile.mojang_sha256() else { return Ok(None) };
    let parent = core.parent().ok_or("InvalidLocalCreationCore")?;
    let named = format!("mojang_{}.jar", profile.minecraft);
    let known = [
        parent.join(&named),
        parent.join("mojang.jar"),
        parent.join("cache").join(&named),
        core_source::download_directory(root, profile)?.join("mojang.jar"),
        root.join("profile-assets")
            .join(format!("{}-{}", profile.family.to_ascii_lowercase(), profile.minecraft))
            .join("mojang.jar"),
    ];
    if let Some(path) = known.iter().find(|path| *path != core && matching_jar(path, expected)) {
        return Ok(Some(path.clone()));
    }
    let mut jars = 0;
    for (index, candidate) in fs::read_dir(parent).map_err(fail)?.enumerate() {
        if index >= MAX_NEIGHBORS {
            break;
        }
        let candidate = candidate.map_err(fail)?;
        let path = candidate.path();
        if path == core || !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("jar"))
            || !candidate.file_type().map_err(fail)?.is_file() || known.contains(&path)
        {
            continue;
        }
        jars += 1;
        if jars > MAX_NEIGHBOR_JARS {
            break;
        }
        if matching_jar(&path, expected) {
            return Ok(Some(path));
        }
    }
    Err("MissingMojangArtifact".into())
}

fn read_source(root: &Path, source: &Source, verify_latest: bool) -> Result<(CoreProfile, Assets)> {
    match source {
        Source::Local { path } => {
            let core = prepare::local_jar(path).map_err(|_| "InvalidLocalCreationCore")?;
            let profile = verified_core_profile(&core)?.ok_or("UnverifiedCreationCore")?;
            if profile.family == "VELOCITY" {
                return Err("UnsupportedCreationCore".into());
            }
            let mojang = local_mojang(root, &core, &profile)?;
            Ok((profile, Assets::Local { core, mojang }))
        }
        Source::Download { family, minecraft } => {
            let (core, mojang_url) = core_source::fixed_creation_target(family, minecraft)?;
            Ok((core, Assets::Download { mojang_url }))
        }
        Source::Official { family, minecraft, build, sha256 } => {
            let build_number = build.parse::<u64>().ok().filter(|id| *id != 0 && id.to_string() == *build);
            if !matches!(family.as_str(), "PAPER" | "FOLIA") || minecraft.len() > 32
                || minecraft.split('.').count() < 2
                || !minecraft.split('.').all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                || build_number.is_none() || sha256.len() != 64
                || !sha256.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err("InvalidGuiCreationArguments".into());
            }
            let core = CoreProfile { unverified: false, java: None, family: family.clone(), minecraft: minecraft.clone(), build: build.clone(), sha256: sha256.clone() };
            if verify_latest && core_source::latest_creation_core(family, minecraft)? != core {
                return Err("CreationPreviewChanged".into());
            }
            Ok((core, Assets::Official))
        }
    }
}

fn scope_revision(policy: &scan_policy::Policy) -> Result<String> {
    content_revision::revision(&serde_json::to_vec_pretty(policy).map_err(fail)?)
}

fn covered(root: &Path, policy: &scan_policy::Policy, target: &Path) -> Result<bool> {
    for rule in policy.management_roots() {
        if scan_policy::contains(&scan_policy::rule_path(root, rule)?, target) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn inspect(root: &Path, draft: &Draft) -> Result<Inspection> {
    inspect_source(root, draft, true)
}

fn inspect_source(root: &Path, draft: &Draft, verify_latest: bool) -> Result<Inspection> {
    if !token(&draft.name) {
        return Err("InvalidCreationName".into());
    }
    if draft.port == 0 || draft.parent.len() > 4096 {
        return Err("InvalidGuiCreationArguments".into());
    }
    assert_idle(root, &draft.name)?;
    let parent = scan_policy::managed_path(root, &draft.parent)?;
    let _parents = batch_import::hold_directories(root, &parent)?;
    let target = parent.join(&draft.name);
    if entry_present(&record_path(root, &draft.name))? || entry_present(&target)? {
        return Err("InstanceExists".into());
    }
    registration_path_available(root, &draft.name, &target)?;
    let (policy, original_revision) = scan_policy::load(root)?;
    if policy.excludes_path(root, &target)? {
        return Err("CreationPathExcluded".into());
    }
    let added_revision = if covered(root, &policy, &target)? {
        None
    } else {
        let proposed = policy.with_new_child(root, &parent, &target)?;
        if !proposed.matches(root, &draft.name, &target)?.1 {
            return Err("CreationPathExcluded".into());
        }
        Some(scope_revision(&proposed)?)
    };
    if !scan_policy::admitted_path(root, &draft.name, &target)? && added_revision.is_none() {
        return Err("CreationPathExcluded".into());
    }
    port_unassigned(root, draft.port, &draft.name)?;
    port_free(draft.port)?;
    let (core, assets) = read_source(root, &draft.source, verify_latest)?;
    let scope = Scope { original_revision, added_revision };
    let revision = content_revision::revision(&serde_json::to_vec(&json!({
        "draft":draft,"target":path_format::display(&target),"core":core,"scope":scope
    })).map_err(fail)?)?;
    Ok(Inspection { parent, target, core, assets, scope, revision })
}

fn report(inspection: &Inspection) -> Value {
    json!({
        "target":path_format::display(&inspection.target),
        "parent":path_format::display(&inspection.parent),
        "whitelist":inspection.scope.added_revision.as_ref().map(|_| path_format::display(&inspection.target)),
        "core":inspection.core,"javaMajor":global_java::supported_major(&inspection.core),"revision":inspection.revision,
        "initializationDefaults":policy_settings::has_creation_defaults(&inspection.core),
    })
}

fn pending_path(root: &Path, name: &str) -> PathBuf {
    root.join("staging").join(format!("gui-creation-{name}.json"))
}

fn read_pending(root: &Path, name: &str) -> Result<Option<String>> {
    let path = pending_path(root, name);
    if !entry_present(&path)? {
        return Ok(None);
    }
    no_redirect(&path)?;
    let work: String = read(&path)?;
    if !token(&work) {
        return Err("CreationRecoveryUnconfirmed".into());
    }
    Ok(Some(work))
}

pub(super) fn assert_idle(root: &Path, name: &str) -> Result<()> {
    if let Some(work) = read_pending(root, name)? {
        let path = root.join("jobs").join(format!("{work}.json"));
        no_redirect(&path)?;
        let receipt: Receipt = read(&path)?;
        if !receipt.complete
            || receipt.arguments.first().map(String::as_str) != Some("gui-create-confirm")
            || receipt.arguments.get(1).map(String::as_str) != Some(name)
        {
            return Err("CreationRecoveryPending".into());
        }
    }
    Ok(())
}

struct Stage {
    path: PathBuf,
    journaled: bool,
}

impl Drop for Stage {
    fn drop(&mut self) {
        if self.journaled {
            return;
        }
        // This is a newly created, task-owned staging directory. The Windows
        // recursive removal implementation does not traverse junctions.
        if let Err(error) = no_redirect(&self.path).and_then(|()| fs::remove_dir_all(&self.path).map_err(fail)) {
            eprintln!("Creation staging cleanup: {error}");
        }
    }
}

type PreparedCore = (PathBuf, Option<(PathBuf, String)>);

fn stage(root: &Path, inspection: &Inspection, draft: &Draft, work: &str, prepared: Option<PreparedCore>) -> Result<(Stage, Publication)> {
    let stage_name = format!("gui-create-{work}-{}-{}", std::process::id(), clock());
    let path = child_directory(root, "staging")?.join(&stage_name);
    fs::create_dir(&path).map_err(fail)?;
    let profile = &inspection.core;
    let stage = Stage { path, journaled: false };
    let _staging = batch_import::hold_directories(root, &stage.path)?;
    let (core, mojang) = match &inspection.assets {
        Assets::Local { core, mojang } => (core.clone(), mojang.as_ref().zip(profile.mojang_sha256())
            .map(|(path, hash)| (path.clone(), hash.to_owned()))),
        Assets::Download { .. } => {
            let cache = core_source::download_directory(root, profile)?;
            let hash = profile.mojang_sha256().ok_or("MissingMojangArtifact")?;
            (cache.join("server.jar"), Some((cache.join("mojang.jar"), hash.to_owned())))
        }
        Assets::Official => prepared.ok_or("CoreDownloadUnverified")?,
    };
    core_maintenance::copy_verified(&core, &stage.path.join("server.jar"), &profile.sha256)?;
    if profile.mojang_sha256().is_some_and(|expected| {
        !mojang.as_ref().is_some_and(|(_, hash)| expected == hash)
    }) {
        return Err("MissingMojangArtifact".into());
    }
    if let Some((source, expected)) = mojang {
        let directory = stage.path.join("cache");
        fs::create_dir(&directory).map_err(fail)?;
        let name = format!("mojang_{}.jar", profile.minecraft);
        core_maintenance::copy_verified(&source, &directory.join(name), &expected)?;
    }
    fs::write(stage.path.join("eula.txt"), "eula=true\n").map_err(fail)?;
    if matches!(inspection.assets, Assets::Official)
        && !policy_settings::has_creation_defaults(profile)
    {
        policy_settings::write_basic_creation_settings(&stage.path, draft.port)?;
    } else {
        policy_settings::write_creation_settings(root, &stage.path, draft.port, profile)?;
    }
    let hashes = stage_hashes(&stage.path, profile)?;
    let config = Instance {
        name: draft.name.clone(), java: PathBuf::new(), port: draft.port,
        velocity_listen_address: None, removed: false,
        directory: Some(path_format::display(&inspection.target)),
        jvm: None, timing: None, auto_restart: None, core_profile: Some(profile.clone()),
        core_file: None, core_local_create: true, management_discovered: false,
    };
    let scope = Scope {
        original_revision: inspection.scope.original_revision.clone(),
        added_revision: inspection.scope.added_revision.clone(),
    };
    Ok((stage, Publication { stage: stage_name, config, scope, hashes }))
}

fn stage_hashes(directory: &Path, profile: &CoreProfile) -> Result<BTreeMap<String, String>> {
    let mojang = format!("cache/mojang_{}.jar", profile.minecraft);
    let mut folders = vec![directory.to_path_buf()];
    let mut hashes = BTreeMap::new();
    let mut count = 0;
    while let Some(folder) = folders.pop() {
        no_redirect(&folder)?;
        for entry in fs::read_dir(folder).map_err(fail)? {
            let entry = entry.map_err(fail)?;
            count += 1;
            if count > 64 {
                return Err("CreationRecoveryUnconfirmed".into());
            }
            let path = entry.path();
            let key = path.strip_prefix(directory).map_err(fail)?.to_str()
                .ok_or("CreationRecoveryUnconfirmed")?.replace('\\', "/");
            let kind = entry.file_type().map_err(fail)?;
            if kind.is_dir() {
                folders.push(path);
            } else if kind.is_file() {
                if key != "server.jar" && (key != mojang || profile.mojang_sha256().is_none()) {
                    hashes.insert(key, content_revision::file_revision(&path)?);
                }
            } else {
                return Err("CreationRecoveryUnconfirmed".into());
            }
        }
    }
    if !hashes.contains_key("server.properties") || !hashes.contains_key("eula.txt") {
        return Err("CreationRecoveryUnconfirmed".into());
    }
    Ok(hashes)
}

fn verified_publication(directory: &Path, profile: &CoreProfile, expected: &BTreeMap<String, String>) -> Result<()> {
    no_redirect(directory)?;
    if &stage_hashes(directory, profile)? != expected
        || content_revision::file_revision(&directory.join("server.jar"))? != profile.sha256
    {
        return Err("CreationRecoveryUnconfirmed".into());
    }
    if let Some(expected) = profile.mojang_sha256() {
        let cache = child_directory(directory, "cache")?;
        let mojang = cache.join(format!("mojang_{}.jar", profile.minecraft));
        if content_revision::file_revision(&mojang)? != expected {
            return Err("CreationRecoveryUnconfirmed".into());
        }
    }
    Ok(())
}

fn finish(
    root: &Path, work: &str, receipt_path: &Path, mut receipt: Receipt,
    draft: &Draft, replayed: bool,
) -> Result<Value> {
    let snapshot = receipt.output.take().ok_or("CreationRecoveryUnconfirmed")?;
    let publication: Publication = serde_json::from_value(snapshot).map_err(fail)?;
    let parent = scan_policy::managed_path(root, &draft.parent)?;
    let target = parent.join(&draft.name);
    let config = &publication.config;
    let profile = config.core_profile.as_ref().ok_or("CreationRecoveryUnconfirmed")?;
    if config.name != draft.name || config.port != draft.port
        || config.directory.as_deref() != Some(path_format::display(&target).as_str())
        || !config.core_local_create || !config.java.as_os_str().is_empty()
        || !publication.stage.starts_with(&format!("gui-create-{work}-"))
        || !publication.stage.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'-')
    {
        return Err("CreationRecoveryUnconfirmed".into());
    }
    let _parents = batch_import::hold_directories(root, &parent)?;
    let stage = child_directory(root, "staging")?.join(&publication.stage);
    let staged = entry_present(&stage)?;
    if staged == entry_present(&target)? {
        return Err("CreationRecoveryUnconfirmed".into());
    }
    let record = record_path(root, &draft.name);
    let registered = entry_present(&record)?;
    if !staged && !registered {
        return Err("CreationRecoveryUnconfirmed".into());
    }
    if registered && serde_json::to_value(instance(root, &draft.name)?).map_err(fail)?
        != serde_json::to_value(config).map_err(fail)?
    {
        return Err("CreationRecoveryUnconfirmed".into());
    }
    verified_publication(if staged { &stage } else { &target }, profile, &publication.hashes)?;
    let (policy, current) = scan_policy::load(root)?;
    let need_scope = if current == publication.scope.original_revision {
        publication.scope.added_revision.as_ref()
    } else if !staged && publication.scope.added_revision.as_ref() == Some(&current) {
        None
    } else {
        return Err("CreationPolicyChanged".into());
    };
    let proposed = need_scope.map(|expected| -> Result<scan_policy::Policy> {
        let proposed = policy.with_new_child(root, &parent, &target)?;
        if &scope_revision(&proposed)? != expected || !proposed.matches(root, &draft.name, &target)?.1 {
            return Err("CreationPolicyChanged".into());
        }
        Ok(proposed)
    }).transpose()?;

    registration_path_available(root, &draft.name, &target)?;
    port_unassigned(root, draft.port, &draft.name)?;
    port_free(draft.port)?;
    let pending = pending_path(root, &draft.name);
    match read_pending(root, &draft.name)?.as_deref() {
        Some(owner) if owner == work => {}
        None if staged && !registered => core_maintenance::write_durable(&pending, &work, false)?,
        _ => return Err("CreationRecoveryUnconfirmed".into()),
    }
    if !registered {
        core_maintenance::write_durable(&record, config, false)?;
    }
    if staged {
        core_maintenance::move_durable(&stage, &target, false)?;
        verified_publication(&target, profile, &publication.hashes)?;
    }
    if let Some(proposed) = proposed {
        if scan_policy::managed_path(root, &path_format::display(&target))? != target {
            return Err("CreationRecoveryUnconfirmed".into());
        }
        scan_policy::persist(root, &proposed, &current)?;
    }
    let result = json!({
        "workId":work,"replayed":replayed,"instance":draft.name,"path":path_format::display(&target),
        "whitelist":publication.scope.added_revision.map(|_| path_format::display(&target)),
    });
    receipt.complete = true;
    receipt.output = Some(result.clone());
    core_maintenance::write_durable(receipt_path, &receipt, true)?;
    if let Err(error) = fs::remove_file(pending) {
        eprintln!("Completed creation marker cleanup: {error}");
    }
    Ok(result)
}

fn confirm(root: &Path, work: &str, raw: &str, progress: &dyn Fn(CreationPhase)) -> Result<Value> {
    progress(CreationPhase::Checking);
    let confirmation: Confirmation = parse(raw)?;
    let Confirmation { draft, revision } = confirmation;
    if !token(&draft.name) {
        return Err("InvalidCreationName".into());
    }
    if !token(work) || draft.port == 0 {
        return Err("InvalidGuiCreationArguments".into());
    }
    let receipt_path = child_directory(root, "jobs")?.join(format!("{work}.json"));
    let arguments = vec!["gui-create-confirm".into(), draft.name.clone(), work.to_owned(), raw.to_owned()];
    // Downloading may take longer than workspace operations. No rule or
    // registration is published until the preview is rechecked under the lock.
    let mut prepared = None;
    if !matches!(&draft.source, Source::Local { .. }) && !entry_present(&receipt_path)? {
        let checked = inspect_source(root, &draft, false)?;
        if checked.revision != revision {
            return Err("CreationPreviewChanged".into());
        }
        match checked.assets {
            Assets::Download { mojang_url } => {
                progress(CreationPhase::DownloadingCore);
                core_source::publish_downloads(root, &checked.core, mojang_url)?;
            }
            Assets::Official => {
                global_java::resolve(root, &checked.core)?;
                prepared = Some(core_source::prepare_creation_download(root, &checked.core, progress)?);
            }
            Assets::Local { .. } => {}
        }
    }
    let _ports = command_lock(root, "port-registration.lock", false)?;
    let _instance = instance_writer_lock(root, &draft.name)?;
    if entry_present(&receipt_path)? {
        no_redirect(&receipt_path)?;
        let receipt: Receipt = read(&receipt_path)?;
        if receipt.arguments != arguments {
            return Err("RequestConflict".into());
        }
        if receipt.complete {
            let mut result = receipt.output.ok_or("CreationRecoveryUnconfirmed")?;
            result["replayed"] = json!(true);
            if read_pending(root, &draft.name)?.as_deref() == Some(work) {
                if let Err(error) = fs::remove_file(pending_path(root, &draft.name)) {
                    eprintln!("Completed creation marker cleanup: {error}");
                }
            }
            return Ok(result);
        }
        progress(CreationPhase::Creating);
        return finish(root, work, &receipt_path, receipt, &draft, true);
    }
    let inspection = inspect_source(root, &draft, false)?;
    if inspection.revision != revision {
        return Err("CreationPreviewChanged".into());
    }
    progress(CreationPhase::Creating);
    let (mut staged, publication) = stage(root, &inspection, &draft, work, prepared)?;
    let receipt = Receipt {
        arguments,
        complete: false,
        output: Some(serde_json::to_value(publication).map_err(fail)?),
        created_at: Some((clock() / 1_000_000) as u64),
    };
    reserve_receipt(root, &receipt_path, &receipt)?;
    staged.journaled = true;
    finish(root, work, &receipt_path, receipt, &draft, false)
}

fn recover(root: &Path, work: &str) -> Result<Value> {
    if !token(work) {
        return Err("InvalidGuiCreationArguments".into());
    }
    let receipt = child_directory(root, "jobs")?.join(format!("{work}.json"));
    if !entry_present(&receipt)? {
        return Err("CreationRecoveryNotFound".into());
    }
    no_redirect(&receipt)?;
    let receipt: Receipt = read(&receipt)?;
    let [action, _name, id, raw] = receipt.arguments.as_slice() else {
        return Err("CreationRecoveryUnconfirmed".into());
    };
    if action != "gui-create-confirm" || id != work {
        return Err("CreationRecoveryUnconfirmed".into());
    }
    confirm(root, work, raw, &|_| {})
}

pub(super) fn dispatch(
    root: &Path,
    verb: &str,
    args: &[String],
    progress: &dyn Fn(CreationPhase),
) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("gui-create-preview", [raw]) => parse(raw).and_then(|draft| inspect(root, &draft)).map(|inspection| report(&inspection)),
        ("gui-create-confirm", [work, raw]) => confirm(root, work, raw, progress),
        ("gui-create-recover", [work]) => recover(root, work),
        ("gui-create-preview" | "gui-create-confirm" | "gui-create-recover", _) => {
            Err("InvalidGuiCreationArguments".into())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ih-gui-create-{}-{}-{}", std::process::id(), clock(), NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            super::super::dispatch(&[path.to_string_lossy().into_owned(), "init".into()]).unwrap();
            let root = safe_directory(&path).unwrap();
            fs::create_dir(root.join("instances").join("existing")).unwrap();
            Self(root)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn draft(source: Source) -> Draft {
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap().local_addr().unwrap().port();
        Draft {
            name: "paper-new".into(),
            parent: "instances/existing".into(),
            port,
            source,
        }
    }

    #[test]
    fn download_preview_proposes_scope_without_writes_and_rejects_stale_confirmation() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let draft = draft(Source::Download { family: "PAPER".into(), minecraft: "26.3".into() });
        let inspected = inspect(root, &draft).unwrap();
        let parent = root.join("instances").join("existing");
        assert_eq!(report(&inspected)["whitelist"], path_format::display(&parent.join("paper-new")));
        assert_eq!(report(&inspected)["target"], path_format::display(&parent.join("paper-new")));
        let (policy, revision) = scan_policy::load(root).unwrap();
        assert!(policy.management_roots().is_empty());
        assert_eq!(fs::read_dir(root.join("staging")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.join("jobs")).unwrap().count(), 0);
        let policy = policy.with_folder(root, &parent, &[]).unwrap();
        scan_policy::persist(root, &policy, &revision).unwrap();
        let error = confirm(root, "new-paper", &json!({"draft":draft,"revision":inspected.revision}).to_string(), &|_| {}).unwrap_err();
        assert_eq!(error, "CreationPreviewChanged");
        assert_eq!(report(&inspect(root, &draft).unwrap())["whitelist"], Value::Null);
        assert!(!parent.join("paper-new").exists());
    }

    #[test]
    fn official_download_stages_only_verified_assets_and_binds_unmapped_versions_to_loopback() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let core = root.join("downloaded.jar");
        let mojang = root.join("mojang.jar");
        fs::write(&core, b"official download fixture").unwrap();
        fs::write(&mojang, b"launcher artifact fixture").unwrap();
        let hash = content_revision::file_revision(&core).unwrap();
        let mojang_hash = content_revision::file_revision(&mojang).unwrap();
        let draft = draft(Source::Official {
            family: "FOLIA".into(), minecraft: "26.3".into(), build: "26".into(), sha256: hash,
        });
        let inspected = inspect_source(root, &draft, false).unwrap();
        assert_eq!(report(&inspected)["initializationDefaults"], false);
        assert!(!inspected.target.exists());
        let phases = std::cell::RefCell::new(Vec::new());
        let raw = json!({"draft":draft,"revision":"stale"}).to_string();
        assert_eq!(
            confirm(root, "not-current", &raw, &|phase| phases.borrow_mut().push(phase)).unwrap_err(),
            "CreationPreviewChanged"
        );
        assert_eq!(phases.into_inner(), [CreationPhase::Checking]);

        let (stage, publication) = stage(root, &inspected, &draft, "checked-assets", Some((core, Some((mojang, mojang_hash.clone()))))).unwrap();
        assert_eq!(publication.hashes["cache/mojang_26.3.jar"], mojang_hash);
        assert!(fs::read_to_string(stage.path.join("server.properties")).unwrap().contains("server-ip=127.0.0.1"));
        fs::write(stage.path.join("cache/mojang_26.3.jar"), b"changed").unwrap();
        assert!(verified_publication(&stage.path, &inspected.core, &publication.hashes).is_err());
        assert!(!record_path(root, &draft.name).exists());
    }

    #[test]
    fn retry_completes_interrupted_creation_without_duplicate_registry_or_scope() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let core = root.join("stock.jar");
        fs::write(&core, "synthetic artifact").unwrap();
        let draft = draft(Source::Local { path: path_format::display(&core) });
        let parent = root.join("instances").join("existing");
        let target = parent.join(&draft.name);
        let (policy, original_revision) = scan_policy::load(root).unwrap();
        let proposed = policy.with_new_child(root, &parent, &target).unwrap();
        let added_revision = Some(scope_revision(&proposed).unwrap());
        let profile = CoreProfile { unverified: false, java: None,
            family: "SPIGOT".into(), minecraft: "1.8.8".into(), build: "fixture".into(),
            sha256: content_revision::file_revision(&core).unwrap(),
        };
        let inspection = Inspection {
            parent: parent.clone(), target: target.clone(), core: profile,
            assets: Assets::Local { core, mojang: None },
            scope: Scope { original_revision, added_revision },
            revision: "fixture".into(),
        };
        let work = "resume-creation";
        let raw = json!({"draft":draft,"revision":inspection.revision}).to_string();
        let (mut staged, publication) = stage(root, &inspection, &draft, work, None).unwrap();
        let receipt = Receipt {
            arguments: vec!["gui-create-confirm".into(), draft.name.clone(), work.into(), raw.clone()],
            complete: false, output: Some(serde_json::to_value(&publication).unwrap()), created_at: None,
        };
        let path = root.join("jobs").join(format!("{work}.json"));
        reserve_receipt(root, &path, &receipt).unwrap();
        staged.journaled = true;

        // Script the point after publishing the record and moving the staged
        // directory where a terminated manager must still block writes/starts.
        core_maintenance::write_durable(&pending_path(root, &draft.name), &work, false).unwrap();
        core_maintenance::write_durable(&record_path(root, &draft.name), &publication.config, false).unwrap();
        core_maintenance::move_durable(&staged.path, &target, false).unwrap();
        assert_eq!(assert_idle(root, &draft.name).unwrap_err(), "CreationRecoveryPending");
        assert!(scan_policy::load(root).unwrap().0.management_roots().is_empty());

        let phases = std::cell::RefCell::new(Vec::new());
        let result = confirm(root, work, &raw, &|phase| phases.borrow_mut().push(phase)).unwrap();
        assert_eq!(phases.into_inner(), [CreationPhase::Checking, CreationPhase::Creating]);
        assert_eq!(result["replayed"], true);
        assert_eq!(result["path"], path_format::display(&target));
        assert_eq!(result["whitelist"], path_format::display(&target));
        assert!(instance(root, &draft.name).unwrap().java.as_os_str().is_empty());
        assert_idle(root, &draft.name).unwrap();
        assert_eq!(scan_policy::load(root).unwrap().0.management_roots(), &[path_format::display(&target)]);
        let phases = std::cell::RefCell::new(Vec::new());
        assert_eq!(confirm(root, work, &raw, &|phase| phases.borrow_mut().push(phase)).unwrap(), result);
        assert_eq!(phases.into_inner(), [CreationPhase::Checking]);
        assert_eq!(recover(root, work).unwrap(), result);
        assert_eq!(fs::read_dir(root.join("registry")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    }
}
