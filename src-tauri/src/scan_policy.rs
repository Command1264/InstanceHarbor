use super::*;
use std::path::Component;

const MAX_RULES: usize = 256;
const MAX_POLICY_BYTES: u64 = 256 * 1024;
const MAX_SCOPE_CANDIDATES: usize = 64;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Policy {
    // Historical registry names remain readable. New rules use explicit paths.
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    allow_paths: Vec<String>,
    #[serde(default)]
    exclude_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    managed_folders: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Edit {
    revision: String,
    allow_paths: Vec<String>,
    exclude_paths: Vec<String>,
    #[serde(default)]
    preview_revision: Option<String>,
}

#[derive(Serialize)]
struct PathIssue {
    kind: &'static str,
    index: usize,
    reason: String,
}

impl PathIssue {
    fn error(&self) -> String {
        format!(
            "InvalidManagementPath {} {} {}",
            self.kind, self.index, self.reason
        )
    }
}

struct ValidatedPaths {
    allowed: Vec<PathBuf>,
    excluded: Vec<PathBuf>,
    issues: Vec<PathIssue>,
}

impl ValidatedPaths {
    fn apply(self, policy: &mut Policy) {
        policy.allow_paths = self
            .allowed
            .iter()
            .map(|path| path_format::display(path))
            .collect();
        policy.exclude_paths = self
            .excluded
            .iter()
            .map(|path| path_format::display(path))
            .collect();
        policy.managed_folders = true;
    }
}

fn validate_paths(root: &Path, allows: &[String], excludes: &[String]) -> Result<ValidatedPaths> {
    if allows.len() + excludes.len() > MAX_RULES {
        return Err("ScanPolicyTooLarge".into());
    }
    let mut issues = Vec::new();
    let mut collect = |rules: &[String], kind| {
        let mut paths: Vec<(usize, PathBuf)> = Vec::new();
        for (index, rule) in rules.iter().enumerate() {
            let checked = if rule.trim().is_empty() {
                Err("ScanPathRequired".to_owned())
            } else {
                managed_path(root, rule)
            };
            match checked {
                Ok(path) if !paths.iter().any(|(_, existing)| same_path(existing, &path)) => {
                    paths.push((index, path));
                }
                Ok(_) => issues.push(PathIssue {
                    kind,
                    index,
                    reason: "DuplicateScanPath".into(),
                }),
                Err(reason) => issues.push(PathIssue {
                    kind,
                    index,
                    reason,
                }),
            }
        }
        paths
    };
    let allowed = collect(allows, "allow");
    let excluded = collect(excludes, "exclude");
    for (index, path) in &excluded {
        if !allowed
            .iter()
            .any(|(_, parent)| contains(parent, path) && !same_path(path, parent))
        {
            issues.push(PathIssue {
                kind: "exclude",
                index: *index,
                reason: "ScanExclusionOutsideWhitelist".into(),
            });
        }
    }
    issues.sort_by_key(|issue| (issue.kind, issue.index));
    Ok(ValidatedPaths {
        allowed: allowed.into_iter().map(|(_, path)| path).collect(),
        excluded: excluded.into_iter().map(|(_, path)| path).collect(),
        issues,
    })
}

fn same_path(left: &Path, right: &Path) -> bool {
    contains(left, right) && contains(right, left)
}

fn policy_path(root: &Path) -> PathBuf {
    root.join("scan-policy.json")
}

pub(super) fn load(root: &Path) -> Result<(Policy, String)> {
    let path = policy_path(root);
    let resolved = retry_shared_file(|| fs::canonicalize(&path))
        .map_err(|error| file_error("resolve", &path, error))?;
    if resolved != path {
        return Err("PathRedirected".into());
    }
    let mut bytes = Vec::new();
    retry_shared_file(|| File::open(&path))
        .map_err(|error| file_error("read", &path, error))?
        .take(MAX_POLICY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| file_error("read", &path, error))?;
    if bytes.len() as u64 > MAX_POLICY_BYTES {
        return Err("ScanPolicyTooLarge".into());
    }
    let policy = serde_json::from_slice(&bytes).map_err(|error| format!("InvalidScanPolicy: scan-policy.json: {error}"))?;
    Ok((policy, content_revision::revision(&bytes)?))
}

pub(super) fn rule_path(root: &Path, value: &str) -> Result<PathBuf> {
    let value = value.replace('/', "\\");
    let path = Path::new(&value);
    let drive = value.as_bytes();
    let absolute =
        drive.len() >= 3 && drive[0].is_ascii_alphabetic() && drive[1] == b':' && drive[2] == b'\\';
    let valid_components = value.split('\\').skip(usize::from(absolute)).all(|part| {
        part.is_empty()
            || (part == "."
                || (!part.ends_with(['.', ' '])
                    && !part.contains([':', '*', '?', '"', '<', '>', '|'])
                    && !part.chars().any(char::is_control)))
    });
    if value.is_empty()
        || !valid_components
        || (!absolute && (path.has_root() || !value.contains('\\')))
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("InvalidScanPath".into());
    }
    Ok(if absolute {
        PathBuf::from(format!(
            r"\\?\{}",
            path.components().collect::<PathBuf>().display()
        ))
    } else {
        root.join(
            path.components()
                .filter(|part| *part != Component::CurDir)
                .collect::<PathBuf>(),
        )
    })
}

pub(super) fn contains(parent: &Path, child: &Path) -> bool {
    let mut child = child.components();
    parent.components().all(|part| {
        child.next().is_some_and(|candidate| {
            part.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&candidate.as_os_str().to_string_lossy())
        })
    })
}

pub(super) fn managed_path(root: &Path, value: &str) -> Result<PathBuf> {
    let base = root.join("instances");
    let path = if value.eq_ignore_ascii_case("instances") {
        base.clone()
    } else {
        rule_path(root, value)?
    };
    if !contains(&base, &path) {
        return Err("BatchImportDirectoryOutsideScope".into());
    }
    let canonical = safe_directory(&path).map_err(|_| {
        if path.is_file() {
            "ExpectedDirectory"
        } else {
            "ScanDirectoryUnavailable"
        }
    })?;
    if !contains(&path, &canonical) || !contains(&canonical, &path) {
        return Err("DirectoryRedirected".into());
    }
    Ok(canonical)
}

impl Policy {
    pub(super) fn management_roots(&self) -> &[String] {
        if self.managed_folders {
            &self.allow_paths
        } else {
            &[]
        }
    }

    pub(super) fn excludes_path(&self, root: &Path, path: &Path) -> Result<bool> {
        for rule in &self.exclude_paths {
            if contains(&rule_path(root, rule)?, path) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn matches(
        &self,
        root: &Path,
        name: &str,
        path: &Path,
    ) -> Result<(&'static str, bool)> {
        let matches_paths = |paths: &[String]| -> Result<bool> {
            for rule in paths {
                if contains(&rule_path(root, rule)?, path) {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        if self.exclude.iter().any(|entry| entry == name) || matches_paths(&self.exclude_paths)? {
            return Ok(("Excluded", false));
        }
        if self.managed_folders {
            return Ok(if matches_paths(&self.allow_paths)? {
                ("Allowed", true)
            } else {
                ("NotAllowed", false)
            });
        }
        if self.allow.is_empty() && self.allow_paths.is_empty() {
            return Ok(("Default", true));
        }
        let allowed =
            self.allow.iter().any(|entry| entry == name) || matches_paths(&self.allow_paths)?;
        Ok(if allowed {
            ("Allowed", true)
        } else {
            ("NotAllowed", false)
        })
    }

    fn instance_matches(
        &self,
        root: &Path,
        name: &str,
        path: &Path,
        discovered: bool,
    ) -> Result<(&'static str, bool)> {
        if self.managed_folders && !discovered {
            // Explicit creation and pre-existing registrations have their own
            // lifecycle and must not silently disappear during policy upgrade.
            let excluded =
                self.exclude.iter().any(|entry| entry == name) || self.excludes_path(root, path)?;
            return Ok(if excluded {
                ("Excluded", false)
            } else {
                ("Default", true)
            });
        }
        self.matches(root, name, path)
    }

    fn replace_paths(
        &mut self,
        root: &Path,
        allows: Vec<String>,
        excludes: Vec<String>,
    ) -> Result<()> {
        let validation = validate_paths(root, &allows, &excludes)?;
        if let Some(issue) = validation.issues.first() {
            return Err(issue.error());
        }
        validation.apply(self);
        Ok(())
    }

    pub(super) fn with_folder(
        mut self,
        root: &Path,
        directory: &Path,
        rejected: &[String],
    ) -> Result<Self> {
        let mut allows = self.allow_paths.clone();
        if !allows
            .iter()
            .any(|rule| managed_path(root, rule).is_ok_and(|path| path == directory))
        {
            allows.push(path_format::display(directory));
        }
        let mut excludes = self.exclude_paths.clone();
        excludes.extend_from_slice(rejected);
        self.replace_paths(root, allows, excludes)?;
        Ok(self)
    }

    pub(super) fn with_new_child(
        mut self,
        root: &Path,
        parent: &Path,
        target: &Path,
    ) -> Result<Self> {
        let canonical = managed_path(root, &path_format::display(parent))?;
        if !target.parent().is_some_and(|path| same_path(path, &canonical))
            || !target.file_name().and_then(|name| name.to_str()).is_some_and(token)
        {
            return Err("InvalidInstancePath".into());
        }
        match fs::symlink_metadata(target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(fail(error)),
            Ok(_) => {
                let verified = managed_path(root, &path_format::display(target))?;
                if !same_path(&verified, target) {
                    return Err("DirectoryRedirected".into());
                }
            }
        }
        if self.excludes_path(root, target)? {
            return Err("InstanceExcludedOrInvalid".into());
        }
        let allow = self.allow_paths.clone();
        let exclude = self.exclude_paths.clone();
        self.replace_paths(root, allow, exclude)?;
        if !self
            .allow_paths
            .iter()
            .any(|path| rule_path(root, path).is_ok_and(|path| same_path(&path, target)))
        {
            if self.allow_paths.len() + self.exclude_paths.len() >= MAX_RULES {
                return Err("ScanPolicyTooLarge".into());
            }
            self.allow_paths.push(path_format::display(target));
        }
        Ok(self)
    }
}

pub(super) fn admits_record(root: &Path, policy: &Policy, record: &Instance) -> Result<bool> {
    let path = registered_path(root, &record.name, record.directory.as_deref())?;
    Ok(policy.instance_matches(root, &record.name, &path, record.management_discovered)?.1)
}

pub(super) fn admitted(root: &Path, name: &str) -> Result<bool> {
    let record = match fs::symlink_metadata(record_path(root, name)) {
        Ok(_) => Some(instance(root, name)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(fail(error)),
    };
    let path = registered_path(
        root,
        name,
        record.as_ref().and_then(|item| item.directory.as_deref()),
    )?;
    let (policy, _) = load(root)?;
    let discovered = record.is_some_and(|item| item.management_discovered);
    Ok(policy.instance_matches(root, name, &path, discovered)?.1)
}

pub(super) fn admitted_path(root: &Path, name: &str, path: &Path) -> Result<bool> {
    let (policy, _) = load(root)?;
    let discovered = match fs::symlink_metadata(record_path(root, name)) {
        Ok(_) => instance(root, name)?.management_discovered,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(fail(error)),
    };
    Ok(policy.instance_matches(root, name, path, discovered)?.1)
}

pub(super) fn registered_path(root: &Path, name: &str, directory: Option<&str>) -> Result<PathBuf> {
    let base = root.join("instances");
    let Some(directory) = directory else {
        return Ok(base.join(name));
    };
    let target = rule_path(root, directory)?;
    if contains(&target, &base) || !contains(&base, &target) {
        return Err("ImportOutsideInstanceRoot".into());
    }
    Ok(target)
}

pub(super) fn verify_import_idle(directory: &Path) -> Result<()> {
    // A legacy-managed directory requires an ownership handoff. Ordinary
    // import never interprets or changes legacy metadata.
    for marker in ["workspace.json", ".testlab-manager.lock", "testlab-console"] {
        match fs::symlink_metadata(directory.join(marker)) {
            Ok(_) => return Err("ImportManagementUnconfirmed".into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(fail(error)),
        }
    }
    // Java can still be starting before it takes its port. An existing
    // world lock provides independent evidence that the directory is in use.
    let mut folders = vec![directory.to_path_buf()];
    let mut visited = 0;
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).map_err(fail)? {
            let entry = entry.map_err(fail)?;
            visited += 1;
            if visited > 32_768 {
                return Err("ImportTreeTooLarge".into());
            }
            use std::os::windows::fs::MetadataExt;
            if fs::symlink_metadata(entry.path())
                .map_err(fail)?
                .file_attributes()
                & 0x400
                != 0
            {
                return Err("DirectoryRedirected".into());
            }
            if entry.file_type().map_err(fail)?.is_dir() {
                folders.push(entry.path());
            } else if entry.file_name() == "session.lock" {
                use std::os::windows::fs::OpenOptionsExt;
                let file = OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(entry.path())
                    .map_err(|_| "ImportManagementUnconfirmed".to_owned())?;
                file.try_lock_shared()
                    .map_err(|_| "ImportManagementUnconfirmed".to_owned())?;
            }
        }
    }
    Ok(())
}

pub(super) fn preview(root: &Path, current: &Policy, proposed: &Policy) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    for entry in fs::read_dir(root.join("registry")).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| token(name))
            .ok_or("InvalidInstanceName")?;
        let item = instance(root, name)?;
        if item.removed {
            continue;
        }
        let server = registered_path(root, name, item.directory.as_deref())?;
        let (_, was_visible) =
            current.instance_matches(root, name, &server, item.management_discovered)?;
        let (reason, visible) =
            proposed.instance_matches(root, name, &server, item.management_discovered)?;
        rows.push(
            json!({"name":name,"path":path_format::display(&server),"visible":visible,
            "changed":was_visible != visible,"reason":reason}),
        );
    }
    rows.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(rows)
}

fn changed_discovery_roots(
    root: &Path,
    current: &Policy,
    proposed: &Policy,
) -> Result<Vec<PathBuf>> {
    let old_roots = current
        .management_roots()
        .iter()
        .map(|path| rule_path(root, path))
        .collect::<Result<Vec<_>>>()?;
    let mut roots = Vec::new();
    for path in proposed.management_roots() {
        let directory = rule_path(root, path)?;
        if proposed.excludes_path(root, &directory)? {
            continue;
        }
        let previously_covered = old_roots.iter().any(|old| contains(old, &directory))
            && !current.excludes_path(root, &directory)?;
        let mut lifts_exclusion = false;
        if previously_covered {
            for path in &current.exclude_paths {
                let old = rule_path(root, path)?;
                if contains(&directory, &old) && !proposed.excludes_path(root, &old)? {
                    lifts_exclusion = true;
                    break;
                }
            }
        }
        if !previously_covered || lifts_exclusion {
            roots.push(directory);
        }
    }
    roots.sort_by_key(|path| path.components().count());
    let mut outer: Vec<PathBuf> = Vec::new();
    for path in roots {
        if !outer.iter().any(|parent| contains(parent, &path)) {
            outer.push(path);
        }
    }
    Ok(outer)
}

fn discovered_changes(
    root: &Path,
    current: &Policy,
    proposed: &Policy,
) -> Result<(Vec<Value>, Option<&'static str>)> {
    let mut candidates = Vec::new();
    let mut reason = None;
    for directory in changed_discovery_roots(root, current, proposed)? {
        let view = batch_import::discover_folder(root, &directory, proposed)?;
        if view["complete"] != true {
            reason = Some("ManagementScopeScanIncomplete");
        }
        let rows = view["rows"]
            .as_array()
            .ok_or("BatchImportEvidenceUnavailable")?;
        for row in rows {
            if row["reason"] == "AlreadyManaged" || row["reason"] == "InstanceExcludedOrInvalid" {
                continue;
            }
            let path = rule_path(
                root,
                row["directory"]
                    .as_str()
                    .ok_or("BatchImportEvidenceUnavailable")?,
            )?;
            if current.managed_folders
                && current
                    .matches(root, row["name"].as_str().unwrap_or_default(), &path)?
                    .1
            {
                continue;
            }
            if candidates.len() == MAX_SCOPE_CANDIDATES {
                return Ok((candidates, Some("ManagementScopeScanIncomplete")));
            }
            candidates.push(row.clone());
        }
    }
    candidates.sort_by(|a, b| a["directory"].as_str().cmp(&b["directory"].as_str()));
    // Individual scans already validate their own entries. A shared port in
    // different roots must not be assigned to whichever root the watcher scans first.
    let mut ports = std::collections::BTreeMap::new();
    let mut names = std::collections::BTreeMap::new();
    for row in candidates.iter().filter(|row| row["state"] == "Ready") {
        *ports.entry(row["port"].as_u64()).or_insert(0) += 1;
        *names
            .entry(
                row["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            )
            .or_insert(0) += 1;
    }
    for row in candidates.iter_mut().filter(|row| row["state"] == "Ready") {
        if ports[&row["port"].as_u64()] > 1
            || names[&row["name"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase()]
                > 1
        {
            row["state"] = json!("Blocked");
            row["reason"] = json!("BatchImportSelectionConflict");
            reason.get_or_insert("ManagementScopeCandidateConflict");
        }
    }
    Ok((candidates, reason))
}

fn scope_preview(
    root: &Path,
    current: &Policy,
    proposed: &Policy,
    revision: &str,
    mut rows: Vec<Value>,
) -> Result<Value> {
    for row in &mut rows {
        if row["changed"] != true || row["visible"] != false {
            continue;
        }
        let name = row["name"].as_str().ok_or("InvalidInstanceName")?;
        let config = instance(root, name)?;
        let active = stopping_outside_scope(root, &config)?;
        if active {
            let observed = status(root, &config)?;
            row["runId"] = observed["run"]["id"].clone();
        }
        row["active"] = json!(active);
    }
    let (candidates, reason) = discovered_changes(root, current, proposed)?;
    let mut result = json!({
        "policy":proposed, "revision":revision, "issues":[], "rows":rows,
        "candidates":candidates, "complete":reason.is_none(), "reason":reason
    });
    let snapshot = serde_json::to_vec(&result).map_err(fail)?;
    result["previewRevision"] = json!(content_revision::revision(&snapshot)?);
    Ok(result)
}

pub(super) fn persist(root: &Path, policy: &Policy, expected_revision: &str) -> Result<String> {
    let bytes = serde_json::to_vec_pretty(policy).map_err(fail)?;
    if bytes.len() as u64 > MAX_POLICY_BYTES {
        return Err("ScanPolicyTooLarge".into());
    }
    publish_json(&policy_path(root), &bytes, true, || {
        if load(root)?.1 != expected_revision {
            return Err("SettingsConflict".into());
        }
        Ok(())
    })?;
    content_revision::revision(&bytes)
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    if !matches!(
        verb,
        "scan-policy"
            | "preview-scan-policy"
            | "save-scan-policy"
            | "validate-scan-policy"
            | "preview-management-scope"
            | "save-management-scope"
    ) {
        return None;
    }
    Some((|| {
        if verb == "scan-policy" && args.is_empty() {
            let (policy, revision) = load(root)?;
            return Ok(
                json!({"policy":policy,"revision":revision,"base":path_format::display(root),
                "path":path_format::display(&policy_path(root))}),
            );
        }
        let [raw] = args else {
            return Err("InvalidScanPolicyArguments".into());
        };
        if verb == "scan-policy" || raw.len() as u64 > MAX_POLICY_BYTES {
            return Err("InvalidScanPolicyArguments".into());
        }
        let _writer = if matches!(verb, "save-scan-policy" | "save-management-scope") {
            Some(command_lock(root, "port-registration.lock", false)?)
        } else {
            None
        };
        let (current, revision) = load(root)?;
        let edit: Edit = serde_json::from_str(raw).map_err(fail)?;
        if edit.preview_revision.is_some() != (verb == "save-management-scope") {
            return Err("InvalidScanPolicyArguments".into());
        }
        if edit.revision != revision {
            return Err("SettingsConflict".into());
        }
        let validation = validate_paths(root, &edit.allow_paths, &edit.exclude_paths)?;
        if verb == "validate-scan-policy" {
            return Ok(json!({"revision":revision, "issues":validation.issues}));
        }
        if let Some(issue) = validation.issues.first() {
            if verb == "preview-management-scope" {
                return Ok(json!({"revision":revision, "issues":validation.issues,
                    "rows":[], "candidates":[], "complete":false, "previewRevision":null}));
            }
            if verb == "save-management-scope" {
                return Err("ManagementScopePreviewChanged".into());
            }
            return Err(issue.error());
        }
        let mut policy = current.clone();
        validation.apply(&mut policy);
        if matches!(verb, "preview-management-scope" | "save-management-scope") {
            let impacts = preview(root, &current, &policy)?;
            let _stopping_guard = if verb == "save-management-scope" {
                impacts
                    .iter()
                    .filter(|row| row["changed"] == true && row["visible"] == false)
                    .map(|row| {
                        transition_guard(root, row["name"].as_str().ok_or("InvalidInstanceName")?)
                    })
                    .collect::<Result<Vec<_>>>()?
            } else {
                Vec::new()
            };
            let mut result = scope_preview(root, &current, &policy, &revision, impacts)?;
            let Some(expected) = edit.preview_revision.as_deref() else {
                return Ok(result);
            };
            if result["complete"] != true || result["previewRevision"].as_str() != Some(expected) {
                return Err("ManagementScopePreviewChanged".into());
            }
            result["revision"] = json!(persist(root, &policy, &revision)?);
            return Ok(result);
        }
        let rows = preview(root, &current, &policy)?;
        let revision = if verb == "save-scan-policy" {
            persist(root, &policy, &revision)?
        } else {
            revision
        };
        Ok(json!({"policy":policy,"revision":revision,"rows":rows}))
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(policy: &Policy) -> Self {
            let folder = std::env::temp_dir().join(format!(
                "ih-scan-policy-{}-{}",
                std::process::id(),
                clock()
            ));
            fs::create_dir(&folder).unwrap();
            let fixture = Self(folder.canonicalize().unwrap());
            for name in [
                "instances/scope/child",
                "instances/old",
                "instances/other",
                "registry",
                "runs",
            ] {
                fs::create_dir_all(fixture.0.join(name)).unwrap();
            }
            save(&policy_path(&fixture.0), policy).unwrap();
            fixture
        }

        fn call(&self, verb: &str, arguments: &Value) -> Result<Value> {
            dispatch(&self.0, verb, &[arguments.to_string()]).unwrap()
        }

        fn draft(&self, allowed: &[&str], excluded: &[&str]) -> Value {
            json!({"revision":load(&self.0).unwrap().1, "allowPaths":allowed, "excludePaths":excluded})
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove temporary scope validation fixture");
        }
    }

    #[test]
    fn matches_full_components_and_prioritizes_exclusion() {
        let root = Path::new(r"\\?\D:\fixture");
        let policy = Policy {
            allow_paths: vec!["./instances".into()],
            exclude_paths: vec!["D:/fixture/instances/hidden".into()],
            ..Policy::default()
        };
        assert_eq!(
            policy
                .matches(root, "one", &root.join("instances/hidden"))
                .unwrap(),
            ("Excluded", false)
        );
        assert_eq!(
            policy
                .matches(root, "two", &root.join("instances/hidden-extra"))
                .unwrap(),
            ("Allowed", true)
        );
        assert!(rule_path(root, "../outside").is_err());
        assert!(rule_path(root, "Human").is_err());
        assert!(rule_path(root, "//server/share").is_err());
    }

    #[test]
    fn validates_all_invalid_path_rows_together_without_writing() {
        let f = Fixture::new(&Policy::default());
        fs::write(f.0.join("instances/scope/file.txt"), "fixture").unwrap();
        let before = fs::read(policy_path(&f.0)).unwrap();
        let edit = f.draft(
            &[
                "",
                "instances/missing",
                "instances/scope",
                "INSTANCES/SCOPE",
            ],
            &[
                "instances/scope",
                "instances/other",
                "instances/scope/child",
                "instances/scope/file.txt",
            ],
        );
        let reply = f.call("validate-scan-policy", &edit).unwrap();
        assert_eq!(
            reply["issues"],
            json!([
                {"kind":"allow","index":0,"reason":"ScanPathRequired"},
                {"kind":"allow","index":1,"reason":"ScanDirectoryUnavailable"},
                {"kind":"allow","index":3,"reason":"DuplicateScanPath"},
                {"kind":"exclude","index":0,"reason":"ScanExclusionOutsideWhitelist"},
                {"kind":"exclude","index":1,"reason":"ScanExclusionOutsideWhitelist"},
                {"kind":"exclude","index":3,"reason":"ExpectedDirectory"}
            ])
        );
        let scope = f.call("preview-management-scope", &edit).unwrap();
        assert_eq!(scope["issues"], reply["issues"]);
        assert!(scope["previewRevision"].is_null());
        assert_eq!(scope["complete"], false);
        assert_eq!(fs::read(policy_path(&f.0)).unwrap(), before);
        assert!(!f.0.join("port-registration.lock").exists());
    }

    #[test]
    fn stale_scope_cannot_save_after_candidate_observation_changes() {
        let policy = Policy {
            allow_paths: vec!["instances/old".into()],
            managed_folders: true,
            ..Policy::default()
        };
        let f = Fixture::new(&policy);
        save(
            &record_path(&f.0, "old"),
            &json!({
                "name":"old", "port":25010, "java":"", "management_discovered":true
            }),
        )
        .unwrap();
        let source = f.0.join("instances/scope/child");
        fs::write(source.join("server.jar"), "an unverified fixture").unwrap();
        let config = source.join("server.properties");
        fs::write(&config, "server-ip=127.0.0.1\nserver-port=25011\n").unwrap();
        let before = fs::read(policy_path(&f.0)).unwrap();
        let mut edit = f.draft(&["instances/scope"], &[]);
        let initial = f.call("preview-management-scope", &edit).unwrap();
        assert_eq!(initial["rows"][0]["name"], "old");
        assert_eq!(initial["rows"][0]["active"], false);
        assert_eq!(initial["rows"][0]["visible"], false);
        assert_eq!(initial["candidates"][0]["name"], "child");
        assert_eq!(initial["candidates"][0]["port"], 25011);
        assert_eq!(initial["candidates"][0]["state"], "Blocked");
        assert_eq!(initial["complete"], true);
        assert_eq!(fs::read(policy_path(&f.0)).unwrap(), before);
        edit["previewRevision"] = initial["previewRevision"].clone();
        fs::write(&config, "server-ip=127.0.0.1\nserver-port=25012\n").unwrap();
        assert_eq!(
            f.call("save-management-scope", &edit).unwrap_err(),
            "ManagementScopePreviewChanged"
        );
        assert_eq!(fs::read(policy_path(&f.0)).unwrap(), before);
        edit.as_object_mut().unwrap().remove("previewRevision");
        let current = f.call("preview-management-scope", &edit).unwrap();
        assert_ne!(current["previewRevision"], initial["previewRevision"]);
        assert_eq!(current["candidates"][0]["port"], 25012);
        edit["previewRevision"] = current["previewRevision"].clone();
        let saved = f.call("save-management-scope", &edit).unwrap();
        assert_ne!(saved["revision"], edit["revision"]);
        assert_eq!(saved["revision"], load(&f.0).unwrap().1);
        assert_eq!(fs::read_dir(f.0.join("registry")).unwrap().count(), 1);
    }

    #[test]
    fn creation_whitelist_adds_only_the_server_child_and_replays_without_duplicate() {
        let f = Fixture::new(&Policy::default());
        let parent = f.0.join("instances/scope");
        let target = parent.join("new-server");
        let sibling = parent.join("sibling");
        let policy = Policy::default()
            .with_new_child(&f.0, &parent, &target)
            .unwrap();
        assert!(!target.exists());
        assert_eq!(policy.allow_paths, vec![path_format::display(&target)]);
        assert!(policy.matches(&f.0, "new-server", &target).unwrap().1);
        assert!(!policy.matches(&f.0, "sibling", &sibling).unwrap().1);
        assert!(policy
            .clone()
            .with_new_child(&f.0, &parent, &sibling.join("nested"))
            .is_err());
        fs::create_dir(&target).unwrap();
        let replay = policy.with_new_child(&f.0, &parent, &target).unwrap();
        assert_eq!(replay.allow_paths, vec![path_format::display(&target)]);
    }
}
