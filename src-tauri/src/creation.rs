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
    /// Absent: grant unless the instance is under `instances/Agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    test_administrators: Option<bool>,
    /// Absent: add the parent folder when no management folder covers the target yet.
    #[serde(default, skip_serializing_if = "ManagementChoice::is_parent")]
    management_folder: ManagementChoice,
}

/// The management folder a creation adds when no management folder covers the target yet.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum ManagementChoice {
    #[default]
    Parent,
    None,
    /// The target folder itself or one of its parent folders.
    Folder { path: String },
}

impl ManagementChoice {
    fn is_parent(&self) -> bool {
        *self == Self::Parent
    }
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
    test_admins: TestAdmins,
    management: Management,
}

/// What the confirmation shows about management folders. Options stay empty
/// when an existing management folder already covers the target.
#[derive(Default)]
struct Management {
    folder: Option<PathBuf>,
    options: Vec<PathBuf>,
    candidates: Vec<Value>,
    complete: bool,
}

struct TestAdmins {
    requested: bool,
    configured: usize,
    grant: Option<test_admins::Grant>,
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

/// The folders a creation may add: the new folder itself, then each existing
/// parent folder this workspace may manage, nearest first.
fn management_options(root: &Path, parent: &Path, target: &Path) -> Vec<PathBuf> {
    let mut options = vec![target.to_path_buf()];
    let mut next = Some(parent.to_path_buf());
    while let Some(folder) = next {
        let verified = scan_policy::managed_path(root, &path_format::display(&folder));
        if !verified.is_ok_and(|path| scan_policy::contains(&path, &folder) && scan_policy::contains(&folder, &path)) {
            break;
        }
        next = folder.parent().map(Path::to_path_buf);
        options.push(folder);
    }
    options
}

fn chosen_folder(root: &Path, parent: &Path, target: &Path, choice: &ManagementChoice) -> Result<Option<PathBuf>> {
    match choice {
        ManagementChoice::Parent => Ok(Some(parent.to_path_buf())),
        ManagementChoice::None => Ok(None),
        ManagementChoice::Folder { path } => {
            let requested = scan_policy::rule_path(root, path)
                .map_err(|_| "CreationManagementFolderNotAncestor")?;
            management_options(root, parent, target)
                .into_iter()
                .find(|option| scan_policy::contains(option, &requested) && scan_policy::contains(&requested, option))
                .map(Some)
                .ok_or_else(|| "CreationManagementFolderNotAncestor".into())
        }
    }
}

fn proposed_scope(root: &Path, policy: &scan_policy::Policy, parent: &Path, target: &Path, folder: &Path) -> Result<scan_policy::Policy> {
    if folder == target {
        // The target does not exist before creation, so it is added as a new child.
        policy.clone().with_new_child(root, parent, target)
    } else {
        policy.clone().with_folder(root, folder, &[])
    }
}

fn inspect(root: &Path, draft: &Draft) -> Result<Inspection> {
    inspect_source(root, draft, true)
}

fn inspect_source(root: &Path, draft: &Draft, verify_latest: bool) -> Result<Inspection> {
    if !instance_name(&draft.name) {
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
    let mut management = Management { complete: true, ..Management::default() };
    let added_revision = if policy.manages_path(root, &target)? {
        None
    } else {
        management.options = management_options(root, &parent, &target);
        management.folder = chosen_folder(root, &parent, &target, &draft.management_folder)?;
        match &management.folder {
            None => None,
            Some(folder) => {
                let proposed = proposed_scope(root, &policy, &parent, &target, folder)?;
                if !proposed.matches(root, &draft.name, &target)?.1 {
                    return Err("CreationPathExcluded".into());
                }
                if folder != &target {
                    let (rows, reason) = scan_policy::discovered_changes(root, &policy, &proposed)?;
                    management.complete = reason.is_none();
                    management.candidates = rows.into_iter()
                        .filter(|row| row["state"] == "Ready")
                        .map(|row| {
                            let directory = row["directory"].as_str().ok_or("BatchImportEvidenceUnavailable")?;
                            let directory = path_format::display(&scan_policy::rule_path(root, directory)?);
                            Ok(json!({"name":row["name"],"directory":directory,"core":row["core"],
                                "kind":row["kind"],"port":row["port"],"listenAddress":row["listenAddress"]}))
                        })
                        .collect::<Result<_>>()?;
                }
                Some(scope_revision(&proposed)?)
            }
        }
    };
    if !scan_policy::admitted_path(root, &draft.name, &target)? && added_revision.is_none() {
        return Err("CreationPathExcluded".into());
    }
    port_unassigned(root, draft.port, &draft.name)?;
    port_free(draft.port)?;
    let (core, assets) = read_source(root, &draft.source, verify_latest)?;
    let requested = draft.test_administrators.unwrap_or_else(|| !test_admins::agent_path(root, &target));
    let (configured, grant) = test_admins::plan(root, &core, requested)?;
    let scope = Scope { original_revision, added_revision };
    let mut identity = json!({
        "draft":draft,"target":path_format::display(&target),"core":core,"scope":scope,
        "managementCandidates":management.candidates,"managementScanComplete":management.complete,
    });
    if let Some(grant) = &grant {
        identity["testAdministrators"] = grant.summary();
    }
    let revision = content_revision::revision(&serde_json::to_vec(&identity).map_err(fail)?)?;
    let test_admins = TestAdmins { requested, configured, grant };
    Ok(Inspection { parent, target, core, assets, scope, revision, test_admins, management })
}

fn report(inspection: &Inspection) -> Value {
    json!({
        "target":path_format::display(&inspection.target),
        "parent":path_format::display(&inspection.parent),
        "whitelist":inspection.scope.added_revision.as_ref().and(inspection.management.folder.as_deref()).map(path_format::display),
        "managementOptions":inspection.management.options.iter().map(|path| path_format::display(path)).collect::<Vec<_>>(),
        "managementCandidates":inspection.management.candidates,
        "managementScanComplete":inspection.management.complete,
        "core":inspection.core,"javaMajor":global_java::supported_major(&inspection.core),"revision":inspection.revision,
        "initializationDefaults":policy_settings::has_creation_defaults(&inspection.core),
        "testAdministrators":{
            "requested":inspection.test_admins.requested,"configured":inspection.test_admins.configured,
            "grant":inspection.test_admins.grant.as_ref().map(test_admins::Grant::summary),
        },
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
    if let Some(grant) = &inspection.test_admins.grant {
        test_admins::install(&stage.path, grant)?;
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
    let mut folder = chosen_folder(root, &parent, &target, &draft.management_folder)?;
    // 1.0.x requests have no management folder field and always added only the new folder.
    // The recorded revision tells which of the two scopes the confirmed preview showed.
    let legacy = draft.management_folder.is_parent().then(|| target.clone());
    let proposed = match need_scope {
        None => {
            if legacy.is_some() && publication.scope.added_revision.is_some()
                && !policy.manages_path(root, &parent)? && policy.manages_path(root, &target)?
            {
                folder = legacy;
            }
            None
        }
        Some(expected) => {
            let mut matched = None;
            for option in folder.iter().chain(legacy.iter()) {
                let proposed = proposed_scope(root, &policy, &parent, &target, option)?;
                if &scope_revision(&proposed)? == expected && proposed.matches(root, &draft.name, &target)?.1 {
                    matched = Some((option.clone(), proposed));
                    break;
                }
            }
            let (option, proposed) = matched.ok_or("CreationPolicyChanged")?;
            folder = Some(option);
            Some(proposed)
        }
    };

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
    let mut registration = (Value::Null, None);
    if let Some(proposed) = proposed {
        if scan_policy::managed_path(root, &path_format::display(&target))? != target {
            return Err("CreationRecoveryUnconfirmed".into());
        }
        scan_policy::persist(root, &proposed, &current)?;
        // Register the other servers the new folder covers now, instead of waiting for the
        // Manager's next scan. The instance itself is already created, so a failure here is
        // reported and left for that scan rather than undoing the creation.
        if let Some(folder) = folder.as_deref().filter(|folder| *folder != target) {
            registration = match batch_import::register_management_folder(root, folder) {
                Ok(value) => (value, None),
                Err(error) => (Value::Null, Some(error)),
            };
        }
    }
    let result = json!({
        "workId":work,"replayed":replayed,"instance":draft.name,"path":path_format::display(&target),
        "whitelist":publication.scope.added_revision.and(folder.as_deref()).map(path_format::display),
        "registration":registration.0,"registrationError":registration.1,
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
    if !instance_name(&draft.name) {
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
        // Changing only the management folder keeps the core the preview already checked
        // against the official source; confirmation never repeats that check either.
        ("gui-create-rescope", [raw]) => parse(raw).and_then(|draft| inspect_source(root, &draft, false)).map(|inspection| report(&inspection)),
        ("gui-create-confirm", [work, raw]) => confirm(root, work, raw, progress),
        ("gui-create-recover", [work]) => recover(root, work),
        ("gui-create-preview" | "gui-create-rescope" | "gui-create-confirm" | "gui-create-recover", _) => {
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
            test_administrators: None,
            management_folder: ManagementChoice::Parent,
        }
    }

    #[test]
    fn download_preview_proposes_scope_without_writes_and_rejects_stale_confirmation() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let draft = draft(Source::Download { family: "PAPER".into(), minecraft: "26.3".into() });
        let inspected = inspect(root, &draft).unwrap();
        let parent = root.join("instances").join("existing");
        assert_eq!(report(&inspected)["whitelist"], path_format::display(&parent));
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

    fn other_server(directory: &Path) {
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        fs::create_dir_all(directory.join("world")).unwrap();
        fs::write(directory.join("server.jar"), b"fixture").unwrap();
        fs::write(directory.join("server.properties"), format!("server-ip=127.0.0.1\nserver-port={port}\n")).unwrap();
        fs::write(directory.join("world").join("level.dat"), b"world").unwrap();
    }

    #[test]
    fn creation_adds_the_parent_folder_by_default_and_accepts_only_the_target_or_its_parents() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let parent = root.join("instances").join("existing");
        let target = parent.join("paper-new");
        let instances = root.join("instances");
        other_server(&parent.join("neighbor"));
        fs::create_dir(instances.join("elsewhere")).unwrap();
        let mut draft = draft(Source::Download { family: "PAPER".into(), minecraft: "26.3".into() });

        let default = report(&inspect(root, &draft).unwrap());
        assert_eq!(default["whitelist"], path_format::display(&parent));
        let options: Vec<_> = default["managementOptions"].as_array().unwrap().iter()
            .map(|value| value.as_str().unwrap().to_owned()).collect();
        assert_eq!(options, [path_format::display(&target), path_format::display(&parent), path_format::display(&instances)]);
        let candidates = default["managementCandidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 1, "{default}");
        assert_eq!(candidates[0]["directory"], path_format::display(&parent.join("neighbor")));
        assert_eq!(candidates[0]["name"], "neighbor");
        assert!(candidates[0]["kind"].is_string(), "{default}");
        assert!(candidates[0]["port"].is_u64(), "{default}");
        assert!(candidates[0].get("core").is_some(), "{default}");
        assert_eq!(default["managementScanComplete"], true);

        draft.management_folder = ManagementChoice::None;
        let none = report(&inspect(root, &draft).unwrap());
        assert_eq!(none["whitelist"], Value::Null);
        assert_eq!(none["managementCandidates"], json!([]));
        assert_ne!(none["revision"], default["revision"]);

        draft.management_folder = ManagementChoice::Folder { path: path_format::display(&target) };
        let own = report(&inspect(root, &draft).unwrap());
        assert_eq!(own["whitelist"], path_format::display(&target));
        assert_eq!(own["managementCandidates"], json!([]));

        draft.management_folder = ManagementChoice::Folder { path: path_format::display(&instances) };
        assert_eq!(report(&inspect(root, &draft).unwrap())["whitelist"], path_format::display(&instances));

        for refused in [instances.join("elsewhere"), parent.join("neighbor"), root.join("registry")] {
            draft.management_folder = ManagementChoice::Folder { path: path_format::display(&refused) };
            assert_eq!(
                inspect(root, &draft).err().as_deref(),
                Some("CreationManagementFolderNotAncestor"),
                "{}", refused.display()
            );
        }

        // A server folder that appears after the preview changes what will be registered.
        draft.management_folder = ManagementChoice::Parent;
        other_server(&parent.join("late"));
        let later = report(&inspect(root, &draft).unwrap());
        assert_eq!(later["managementCandidates"].as_array().unwrap().len(), 2);
        assert_ne!(later["revision"], default["revision"]);

        // A folder already covered by a management folder offers no choice and adds nothing.
        let (policy, revision) = scan_policy::load(root).unwrap();
        scan_policy::persist(root, &policy.with_folder(root, &instances, &[]).unwrap(), &revision).unwrap();
        for choice in [ManagementChoice::Parent, ManagementChoice::None] {
            draft.management_folder = choice;
            let covered = report(&inspect(root, &draft).unwrap());
            assert_eq!(covered["whitelist"], Value::Null);
            assert_eq!(covered["managementOptions"], json!([]));
            assert_eq!(covered["managementCandidates"], json!([]));
        }
    }

    #[test]
    fn changing_the_management_folder_rescopes_without_rechecking_the_official_build() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let parent = root.join("instances").join("existing");
        other_server(&parent.join("neighbor"));
        // Not the newest stable build: a full preview would query the official source.
        let mut draft = draft(Source::Official {
            family: "FOLIA".into(), minecraft: "26.3".into(), build: "26".into(), sha256: "a".repeat(64),
        });
        let rescope = |draft: &Draft| dispatch(root, "gui-create-rescope", &[serde_json::to_string(draft).unwrap()], &|_| {}).unwrap();
        let default = rescope(&draft).unwrap();
        assert_eq!(default["whitelist"], path_format::display(&parent));
        assert_eq!(default["managementCandidates"].as_array().unwrap().len(), 1);
        draft.management_folder = ManagementChoice::None;
        let none = rescope(&draft).unwrap();
        assert_eq!(none["whitelist"], Value::Null);
        assert_eq!(none["revision"], report(&inspect_source(root, &draft, false).unwrap())["revision"]);
        assert_eq!(
            dispatch(root, "gui-create-rescope", &[], &|_| {}).unwrap().unwrap_err(),
            "InvalidGuiCreationArguments"
        );
    }

    #[test]
    fn creation_registers_the_servers_the_added_folder_discovers_before_it_reports_done() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let core = root.join("stock.jar");
        fs::write(&core, "synthetic artifact").unwrap();
        let draft = draft(Source::Local { path: path_format::display(&core) });
        let parent = root.join("instances").join("existing");
        let target = parent.join(&draft.name);
        other_server(&parent.join("neighbor"));
        let (policy, original_revision) = scan_policy::load(root).unwrap();
        let added_revision = Some(scope_revision(&policy.with_folder(root, &parent, &[]).unwrap()).unwrap());
        let profile = CoreProfile { unverified: false, java: None,
            family: "SPIGOT".into(), minecraft: "1.8.8".into(), build: "fixture".into(),
            sha256: content_revision::file_revision(&core).unwrap(),
        };
        let inspection = Inspection {
            parent: parent.clone(), target: target.clone(), core: profile,
            assets: Assets::Local { core, mojang: None },
            scope: Scope { original_revision, added_revision },
            revision: "fixture".into(),
            test_admins: TestAdmins { requested: true, configured: 0, grant: None },
            management: Management::default(),
        };
        let work = "register-neighbors";
        let raw = json!({"draft":draft,"revision":inspection.revision}).to_string();
        let (mut staged, publication) = stage(root, &inspection, &draft, work, None).unwrap();
        let receipt = Receipt {
            arguments: vec!["gui-create-confirm".into(), draft.name.clone(), work.into(), raw.clone()],
            complete: false, output: Some(serde_json::to_value(&publication).unwrap()), created_at: None,
        };
        reserve_receipt(root, &root.join("jobs").join(format!("{work}.json")), &receipt).unwrap();
        staged.journaled = true;
        core_maintenance::write_durable(&pending_path(root, &draft.name), &work, false).unwrap();

        let result = confirm(root, work, &raw, &|_| {}).unwrap();
        assert_eq!(result["whitelist"], path_format::display(&parent));
        assert_eq!(result["registrationError"], Value::Null, "{result}");
        let neighbor = instance(root, "neighbor").unwrap();
        assert!(neighbor.management_discovered);
        assert!(!neighbor.removed);
        assert!(target.is_dir());
    }

    #[test]
    fn creation_without_a_management_folder_keeps_the_policy_and_the_instance_managed() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let core = root.join("stock.jar");
        fs::write(&core, "synthetic artifact").unwrap();
        let mut draft = draft(Source::Local { path: path_format::display(&core) });
        draft.management_folder = ManagementChoice::None;
        let parent = root.join("instances").join("existing");
        let target = parent.join(&draft.name);
        let (_, original_revision) = scan_policy::load(root).unwrap();
        let profile = CoreProfile { unverified: false, java: None,
            family: "SPIGOT".into(), minecraft: "1.8.8".into(), build: "fixture".into(),
            sha256: content_revision::file_revision(&core).unwrap(),
        };
        let inspection = Inspection {
            parent: parent.clone(), target: target.clone(), core: profile,
            assets: Assets::Local { core, mojang: None },
            scope: Scope { original_revision: original_revision.clone(), added_revision: None },
            revision: "fixture".into(),
            test_admins: TestAdmins { requested: true, configured: 0, grant: None },
            management: Management::default(),
        };
        let work = "no-folder";
        let raw = json!({"draft":draft,"revision":inspection.revision}).to_string();
        let (mut staged, publication) = stage(root, &inspection, &draft, work, None).unwrap();
        let receipt = Receipt {
            arguments: vec!["gui-create-confirm".into(), draft.name.clone(), work.into(), raw.clone()],
            complete: false, output: Some(serde_json::to_value(&publication).unwrap()), created_at: None,
        };
        reserve_receipt(root, &root.join("jobs").join(format!("{work}.json")), &receipt).unwrap();
        staged.journaled = true;
        core_maintenance::write_durable(&pending_path(root, &draft.name), &work, false).unwrap();

        let result = confirm(root, work, &raw, &|_| {}).unwrap();
        assert_eq!(result["whitelist"], Value::Null);
        assert!(target.is_dir());
        let (policy, revision) = scan_policy::load(root).unwrap();
        assert_eq!(revision, original_revision);
        assert!(policy.management_roots().is_empty());
        assert!(scan_policy::admitted(root, &draft.name).unwrap());
    }

    /// Stages a local creation that records `added_revision` and leaves it interrupted before
    /// publication, the way a crash after the receipt was written does.
    fn interrupted(root: &Path, draft: &Draft, raw: &str, work: &str, added_revision: Option<String>) {
        let parent = root.join("instances").join("existing");
        let core = root.join("stock.jar");
        let (_, original_revision) = scan_policy::load(root).unwrap();
        let profile = CoreProfile { unverified: false, java: None,
            family: "SPIGOT".into(), minecraft: "1.8.8".into(), build: "fixture".into(),
            sha256: content_revision::file_revision(&core).unwrap(),
        };
        let inspection = Inspection {
            parent: parent.clone(), target: parent.join(&draft.name), core: profile,
            assets: Assets::Local { core, mojang: None },
            scope: Scope { original_revision, added_revision },
            revision: "fixture".into(),
            test_admins: TestAdmins { requested: true, configured: 0, grant: None },
            management: Management::default(),
        };
        let (mut staged, publication) = stage(root, &inspection, draft, work, None).unwrap();
        let receipt = Receipt {
            arguments: vec!["gui-create-confirm".into(), draft.name.clone(), work.into(), raw.into()],
            complete: false, output: Some(serde_json::to_value(&publication).unwrap()), created_at: None,
        };
        reserve_receipt(root, &root.join("jobs").join(format!("{work}.json")), &receipt).unwrap();
        staged.journaled = true;
        core_maintenance::write_durable(&pending_path(root, &draft.name), &work, false).unwrap();
    }

    #[test]
    fn an_interrupted_1_0_creation_without_a_management_folder_resumes_with_the_target_scope() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let core = root.join("stock.jar");
        fs::write(&core, "synthetic artifact").unwrap();
        let draft = draft(Source::Local { path: path_format::display(&core) });
        let parent = root.join("instances").join("existing");
        let target = parent.join(&draft.name);
        other_server(&parent.join("neighbor"));
        // 1.0.x wrote this request: no `managementFolder`, and the scope it
        // recorded adds only the new folder itself (`with_new_child`).
        let raw = format!(
            r#"{{"draft":{{"name":"paper-new","parent":"instances/existing","port":{},"source":{{"kind":"local","path":{}}}}},"revision":"fixture"}}"#,
            draft.port, serde_json::to_string(&path_format::display(&core)).unwrap(),
        );
        let (policy, _) = scan_policy::load(root).unwrap();
        let legacy = scope_revision(&policy.with_new_child(root, &parent, &target).unwrap()).unwrap();
        interrupted(root, &draft, &raw, "legacy-draft", Some(legacy.clone()));

        let result = recover(root, "legacy-draft").unwrap();
        assert_eq!(result["whitelist"], path_format::display(&target), "{result}");
        assert_eq!(result["registration"], Value::Null, "{result}");
        assert!(target.is_dir());
        assert_eq!(scan_policy::load(root).unwrap().1, legacy);
        assert!(instance(root, "neighbor").is_err(), "the 1.0.x scope does not cover the neighbor");
    }

    #[test]
    fn an_interrupted_creation_with_the_default_folder_still_resumes_with_the_parent_scope() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let core = root.join("stock.jar");
        fs::write(&core, "synthetic artifact").unwrap();
        let draft = draft(Source::Local { path: path_format::display(&core) });
        let parent = root.join("instances").join("existing");
        let raw = json!({"draft":draft,"revision":"fixture"}).to_string();
        assert!(!raw.contains("managementFolder"), "the default choice is sent without the field");
        let (policy, _) = scan_policy::load(root).unwrap();
        let added = scope_revision(&policy.with_folder(root, &parent, &[]).unwrap()).unwrap();
        interrupted(root, &draft, &raw, "default-draft", Some(added));

        let result = recover(root, "default-draft").unwrap();
        assert_eq!(result["whitelist"], path_format::display(&parent), "{result}");
    }

    #[test]
    fn removing_a_created_instance_with_its_settings_keeps_it_out_of_the_management_folder_scan() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let core = root.join("stock.jar");
        fs::write(&core, "synthetic artifact").unwrap();
        let draft = draft(Source::Local { path: path_format::display(&core) });
        let parent = root.join("instances").join("existing");
        let target = parent.join(&draft.name);
        let raw = json!({"draft":draft,"revision":"fixture"}).to_string();
        let (policy, _) = scan_policy::load(root).unwrap();
        let added = scope_revision(&policy.with_folder(root, &parent, &[]).unwrap()).unwrap();
        interrupted(root, &draft, &raw, "created", Some(added));
        recover(root, "created").unwrap();
        // A started server has a world; the scan imports only such folders.
        fs::create_dir_all(target.join("world")).unwrap();
        fs::write(target.join("world").join("level.dat"), b"world").unwrap();
        assert!(!instance(root, &draft.name).unwrap().management_discovered);

        presentation::remove_management(root, &draft.name, "delete").unwrap();
        batch_import::sync_management_folders(root, 0).unwrap();

        let item = instance(root, &draft.name).unwrap();
        assert!(item.removed, "the scan registered the removed instance again");
        assert!(item.core_profile.is_none());
        assert!(target.is_dir());
    }

    #[test]
    fn removing_an_instance_whose_folder_is_gone_drops_its_record() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let core = root.join("stock.jar");
        fs::write(&core, "synthetic artifact").unwrap();
        let draft = draft(Source::Local { path: path_format::display(&core) });
        let target = root.join("instances").join("existing").join(&draft.name);
        let raw = json!({"draft":draft,"revision":"fixture"}).to_string();
        let parent = root.join("instances").join("existing");
        let (policy, _) = scan_policy::load(root).unwrap();
        let added = scope_revision(&policy.with_folder(root, &parent, &[]).unwrap()).unwrap();
        interrupted(root, &draft, &raw, "gone", Some(added));
        recover(root, "gone").unwrap();
        // Deleting the server moves the folder away before unregistering it.
        fs::remove_dir_all(&target).unwrap();

        presentation::remove_management(root, &draft.name, "delete").unwrap();
        assert!(!record_path(root, &draft.name).exists());
    }

    #[test]
    fn a_name_with_an_inner_period_is_created_under_its_own_folder_and_a_trailing_period_is_refused() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let mut draft = draft(Source::Download { family: "PAPER".into(), minecraft: "26.3".into() });
        draft.name = "paper-1.14.4".into();
        let inspected = inspect(root, &draft).unwrap();
        let parent = root.join("instances").join("existing");
        assert_eq!(report(&inspected)["target"], path_format::display(&parent.join("paper-1.14.4")));
        for refused in ["paper.", ".", "..", "con.1"] {
            draft.name = refused.into();
            assert_eq!(inspect(root, &draft).err().as_deref(), Some("InvalidCreationName"), "{refused}");
        }
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
    fn test_administrators_follow_audience_default_and_stop_on_missing_cache() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let mut draft = draft(Source::Download { family: "PAPER".into(), minecraft: "26.3".into() });
        let empty = report(&inspect_source(root, &draft, false).unwrap());
        assert_eq!(empty["testAdministrators"], json!({"requested":true,"configured":0,"grant":null}));
        test_admins::tests::save_fake_administrators(root, "");
        assert_eq!(inspect_source(root, &draft, false).err().unwrap(), "TestPermissionArtifactMissing");
        draft.test_administrators = Some(false);
        let declined = report(&inspect_source(root, &draft, false).unwrap());
        assert_eq!(declined["testAdministrators"], json!({"requested":false,"configured":2,"grant":null}));
        fs::create_dir_all(root.join("instances").join("Agent").join("Lobby")).unwrap();
        draft.test_administrators = None;
        draft.parent = "instances/Agent/Lobby".into();
        let agent = report(&inspect_source(root, &draft, false).unwrap());
        assert_eq!(agent["testAdministrators"]["requested"], false);
        draft.test_administrators = Some(true);
        assert_eq!(inspect_source(root, &draft, false).err().unwrap(), "TestPermissionArtifactMissing");
    }

    #[test]
    fn staged_creation_installs_verified_luckperms_and_seeded_identities() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let core = root.join("downloaded.jar");
        let mojang = root.join("mojang.jar");
        fs::write(&core, b"official download fixture").unwrap();
        fs::write(&mojang, b"launcher artifact fixture").unwrap();
        let mojang_hash = content_revision::file_revision(&mojang).unwrap();
        let draft = draft(Source::Official {
            family: "FOLIA".into(), minecraft: "26.3".into(), build: "26".into(),
            sha256: content_revision::file_revision(&core).unwrap(),
        });
        let mut inspected = inspect_source(root, &draft, false).unwrap();
        inspected.test_admins.grant = Some(test_admins::tests::stand_in_grant(root, test_admins::Platform::Bukkit));
        let (stage, publication) = stage(root, &inspected, &draft, "seeded", Some((core, Some((mojang, mojang_hash))))).unwrap();
        let jar = "plugins/LuckPerms-Bukkit-5.5.85.jar";
        assert!(publication.hashes.contains_key(jar));
        assert!(publication.hashes.contains_key("plugins/LuckPerms/config.yml"));
        assert_eq!(
            publication.hashes.keys().filter(|key| key.starts_with("plugins/LuckPerms/json-storage/users/")).count(),
            2
        );
        assert!(!stage.path.join("ops.json").exists());
        verified_publication(&stage.path, &inspected.core, &publication.hashes).unwrap();
        fs::write(stage.path.join(jar), b"changed").unwrap();
        assert!(verified_publication(&stage.path, &inspected.core, &publication.hashes).is_err());
    }

    #[test]
    fn staged_creation_with_op_writes_ops_json_beside_the_luckperms_data() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let core = root.join("downloaded.jar");
        let mojang = root.join("mojang.jar");
        fs::write(&core, b"official download fixture").unwrap();
        fs::write(&mojang, b"launcher artifact fixture").unwrap();
        let mojang_hash = content_revision::file_revision(&mojang).unwrap();
        let draft = draft(Source::Official {
            family: "FOLIA".into(), minecraft: "26.3".into(), build: "26".into(),
            sha256: content_revision::file_revision(&core).unwrap(),
        });
        let mut inspected = inspect_source(root, &draft, false).unwrap();
        let grant = test_admins::tests::with_op(test_admins::tests::stand_in_grant(root, test_admins::Platform::Bukkit));
        inspected.test_admins.grant = Some(grant);
        let (stage, publication) = stage(root, &inspected, &draft, "seeded-op", Some((core, Some((mojang, mojang_hash))))).unwrap();
        let ops: Value = serde_json::from_slice(&fs::read(stage.path.join("ops.json")).unwrap()).unwrap();
        assert_eq!(ops.as_array().unwrap().len(), 2);
        assert_eq!(ops[0]["level"], 4);
        assert!(publication.hashes.contains_key("ops.json"));
        verified_publication(&stage.path, &inspected.core, &publication.hashes).unwrap();
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
        let proposed = policy.with_folder(root, &parent, &[]).unwrap();
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
            test_admins: TestAdmins { requested: true, configured: 0, grant: None },
            management: Management::default(),
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
        assert_eq!(result["whitelist"], path_format::display(&parent));
        assert!(instance(root, &draft.name).unwrap().java.as_os_str().is_empty());
        assert_idle(root, &draft.name).unwrap();
        assert_eq!(scan_policy::load(root).unwrap().0.management_roots(), &[path_format::display(&parent)]);
        let phases = std::cell::RefCell::new(Vec::new());
        assert_eq!(confirm(root, work, &raw, &|phase| phases.borrow_mut().push(phase)).unwrap(), result);
        assert_eq!(phases.into_inner(), [CreationPhase::Checking]);
        assert_eq!(recover(root, work).unwrap(), result);
        assert_eq!(fs::read_dir(root.join("registry")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    }
}
