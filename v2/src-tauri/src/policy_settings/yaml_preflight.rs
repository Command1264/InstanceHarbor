use super::*;

const RUN_REPORT: &str = "yaml-preflight.json";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Observation {
    file: String,
    value: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Baseline {
    directory: PathBuf,
    core_sha256: String,
    fields: BTreeMap<String, Observation>,
    #[serde(default)]
    file_hashes: BTreeMap<String, Option<String>>,
}

impl Baseline {
    fn from_snapshot(snapshot: &WorldSnapshot) -> Result<Option<Self>> {
        let (Some(profile), Some(core_sha256), Some(directory)) = (
            snapshot.profile,
            snapshot.core_sha256.as_ref(),
            snapshot.path.parent(),
        ) else {
            return Ok(None);
        };
        let fields = FIELDS
            .iter()
            .filter_map(|field| {
                let Binding::Yaml(file, _) = field.binding(profile) else {
                    return None;
                };
                if field.gate(profile, true).0 != "available" {
                    return None;
                }
                let value = snapshot
                    .observe(field)
                    .filter(|property| property.editable)
                    .and_then(|property| logical_value(field, &property.value, profile).ok());
                Some((
                    field.id(),
                    Observation {
                        file: file.into(),
                        value,
                    },
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let mut file_hashes = BTreeMap::new();
        for field in FIELDS {
            let Binding::Yaml(file, _) = field.binding(profile) else {
                continue;
            };
            if file_hashes.contains_key(file) {
                continue;
            }
            let hash = snapshot
                .yaml(file)
                .map(|yaml| content_revision::revision(yaml.text.as_bytes()))
                .transpose()?;
            file_hashes.insert(file.into(), hash);
        }
        Ok(Some(Self {
            directory: directory.to_owned(),
            core_sha256: core_sha256.clone(),
            fields,
            file_hashes,
        }))
    }

    fn same_source(&self, other: &Self) -> bool {
        self.directory == other.directory && self.core_sha256 == other.core_sha256
    }

    fn has_readback(&self) -> bool {
        self.file_hashes.values().any(Option::is_some)
    }

    fn differences(&self, observed: &Self) -> Vec<Value> {
        self.fields
            .iter()
            .filter_map(|(key, before)| {
                let after = observed.fields.get(key)?;
                (before.file == after.file && before.value != after.value).then(|| {
                    json!({"key":key,"file":before.file,"before":before.value,"after":after.value})
                })
            })
            .collect()
    }

    fn changed_files(&self, observed: &Self) -> Vec<&str> {
        self.file_hashes
            .iter()
            .filter_map(|(file, before)| {
                observed
                    .file_hashes
                    .get(file)
                    .filter(|after| *after != before)
                    .map(|_| file.as_str())
            })
            .collect()
    }

    fn record_saved(
        &mut self,
        observed: &Self,
        keys: &BTreeSet<&str>,
        original_hashes: &BTreeMap<&str, String>,
    ) {
        for (key, field) in &observed.fields {
            if keys.contains(key.as_str()) {
                self.fields.insert(key.clone(), field.clone());
            }
        }
        for (file, original) in original_hashes {
            let before = self.file_hashes.get(*file);
            let after = observed.file_hashes.get(*file);
            if let Some(after) = after {
                if before.is_none_or(|hash| hash.as_ref() == Some(original)) {
                    self.file_hashes.insert((*file).into(), after.clone());
                }
            }
        }
    }
}

fn baseline_path(root: &Path, name: &str) -> Result<PathBuf> {
    if !token(name) {
        return Err("InvalidYamlPreflightTarget".into());
    }
    Ok(child_directory(root, "history")?.join(format!("yaml-preflight-{name}.json")))
}

fn optional<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            no_redirect(path)?;
            read(path).map(Some)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(file_error("read-yaml-preflight", path, error)),
    }
}

pub(super) fn record_save(
    root: &Path,
    name: &str,
    snapshot: &WorldSnapshot,
    keys: &BTreeSet<&str>,
    originals: &BTreeMap<&str, String>,
) -> Result<()> {
    let Some(observed) = Baseline::from_snapshot(snapshot)? else {
        return Ok(());
    };
    let path = baseline_path(root, name)?;
    let replace = path.try_exists().map_err(fail)?;
    if let Some(mut previous) =
        optional::<Baseline>(&path)?.filter(|base| base.same_source(&observed))
    {
        previous.record_saved(&observed, keys, originals);
        return core_maintenance::write_durable(&path, &previous, true);
    }
    core_maintenance::write_durable(&path, &observed, replace)
}

pub(in crate::windows) fn record_stop(root: &Path, name: &str) -> Result<()> {
    let directory = instance_path(root, name)?;
    let _lease = claim_runtime_directory(&directory)?;
    let snapshot = WorldSnapshot::read(root, name)?;
    if let Some(observed) = Baseline::from_snapshot(&snapshot)?.filter(Baseline::has_readback) {
        let path = baseline_path(root, name)?;
        core_maintenance::write_durable(&path, &observed, path.try_exists().map_err(fail)?)?;
    }
    Ok(())
}

fn unverified(source: &str) -> Value {
    json!({"state":"Unverified","changes":[],"filesChanged":[],"checkedAt":epoch_ms(),"source":source})
}

pub(in crate::windows) fn has_changes(report: &Value) -> bool {
    ["changes", "filesChanged"].iter().any(|name| {
        report[*name]
            .as_array()
            .is_some_and(|changes| !changes.is_empty())
    })
}

fn compare(
    root: &Path,
    name: &str,
    snapshot: &WorldSnapshot,
    source: &str,
) -> Result<Option<Value>> {
    let previous: Option<Baseline> = optional(&baseline_path(root, name)?)?;
    Ok(compare_observations(
        previous,
        Baseline::from_snapshot(snapshot)?,
        source,
    ))
}

fn compare_observations(
    previous: Option<Baseline>,
    observed: Option<Baseline>,
    source: &str,
) -> Option<Value> {
    let Some(observed) = observed else {
        return previous.map(|_| unverified(source));
    };
    let Some(previous) = previous.filter(|base| base.same_source(&observed)) else {
        return Some(unverified(source));
    };
    Some(json!({
        "state":"Compared","changes":previous.differences(&observed),
        "filesChanged":previous.changed_files(&observed),"checkedAt":epoch_ms(),"source":source
    }))
}

pub(in crate::windows) fn check_start(root: &Path, name: &str) -> Option<Value> {
    match WorldSnapshot::read(root, name)
        .and_then(|snapshot| compare(root, name, &snapshot, "Launch"))
    {
        Ok(report) => report,
        Err(error) => {
            eprintln!("YAML preflight could not be verified for {name}: {error}");
            Some(unverified("Launch"))
        }
    }
}

pub(in crate::windows) fn store_run_report(folder: &Path, run: &str, report: &Value) -> Result<()> {
    let record = json!({"run":run,"report":report});
    core_maintenance::write_durable(&folder.join(RUN_REPORT), &record, false)
}

fn run_report(folder: &Path, run: &str) -> Result<Option<Value>> {
    no_redirect(folder)?;
    optional::<Value>(&folder.join(RUN_REPORT)).map(|record| {
        record
            .filter(|record| record["run"] == run)
            .and_then(|record| record.get("report").cloned())
    })
}

fn response_report(root: &Path, name: &str, snapshot: &WorldSnapshot) -> Result<Option<Value>> {
    let observation = status(root, &instance(root, name)?)?;
    let run = observation["run"]["id"].as_str();
    let folder = root.join("runs").join(name);
    if observation["state"] != "Stopped" {
        return run.map_or(Ok(None), |run| run_report(&folder, run));
    }
    let current = compare(root, name, snapshot, "StoppedReadback")?;
    if current
        .as_ref()
        .is_some_and(|report| report["state"] == "Compared" && !has_changes(report))
    {
        if let Some(launch) = run.map_or(Ok(None), |run| run_report(&folder, run))? {
            if has_changes(&launch) {
                return Ok(Some(launch));
            }
        }
    }
    Ok(current)
}

pub(super) fn add_to_response(
    root: &Path,
    name: &str,
    snapshot: &WorldSnapshot,
    response: &mut Value,
) {
    match response_report(root, name, snapshot) {
        Ok(Some(report)) => response["yamlPreflight"] = report,
        Ok(None) => (),
        Err(error) => {
            eprintln!("YAML preflight could not be read for {name}: {error}");
            response["yamlPreflight"] = unverified("StoppedReadback");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline(mobs: &str, despawn: Option<&str>) -> Baseline {
        let observation = |value: Option<&str>| Observation {
            file: "spigot.yml".into(),
            value: value.map(str::to_owned),
        };
        Baseline {
            directory: PathBuf::from("world"),
            core_sha256: "core".into(),
            fields: BTreeMap::from([
                ("spigot.mob-spawn-range".into(), observation(Some(mobs))),
                ("spigot.item-despawn-rate".into(), observation(despawn)),
            ]),
            file_hashes: BTreeMap::from([("spigot.yml".into(), Some("old".into()))]),
        }
    }

    #[test]
    fn saving_one_yaml_field_retains_external_changes_to_other_fields() {
        let mut previous = baseline("8", Some("6000"));
        let readback = baseline("4", Some("9000"));
        previous.record_saved(
            &readback,
            &BTreeSet::from(["spigot.mob-spawn-range"]),
            &BTreeMap::new(),
        );
        assert_eq!(
            previous.differences(&readback),
            [json!({
                "key":"spigot.item-despawn-rate","file":"spigot.yml","before":"6000","after":"9000"
            })]
        );
        assert_eq!(
            previous.differences(&baseline("4", None)),
            [json!({
                "key":"spigot.item-despawn-rate","file":"spigot.yml","before":"6000","after":null
            })]
        );
    }

    #[test]
    fn unmanaged_yaml_edits_report_only_safe_file_names_and_survive_an_app_save() {
        let mut saved = baseline("8", Some("6000"));
        let mut edited = baseline("8", Some("6000"));
        edited
            .file_hashes
            .insert("spigot.yml".into(), Some("external".into()));
        assert!(saved.differences(&edited).is_empty());
        assert_eq!(saved.changed_files(&edited), ["spigot.yml"]);

        let mut app_readback = baseline("8", Some("7000"));
        app_readback
            .file_hashes
            .insert("spigot.yml".into(), Some("after-app-save".into()));
        let app_edits = BTreeSet::from(["spigot.item-despawn-rate"]);
        let mut clean = baseline("8", Some("6000"));
        clean.record_saved(
            &app_readback,
            &app_edits,
            &BTreeMap::from([("spigot.yml", "old".into())]),
        );
        assert!(clean.changed_files(&app_readback).is_empty());

        saved.record_saved(
            &app_readback,
            &app_edits,
            &BTreeMap::from([("spigot.yml", "external".into())]),
        );
        assert!(saved.differences(&app_readback).is_empty());
        assert_eq!(saved.changed_files(&app_readback), ["spigot.yml"]);
    }

    #[test]
    fn first_launch_has_no_comparable_baseline() {
        let report =
            compare_observations(None, Some(baseline("8", Some("6000"))), "Launch").unwrap();
        assert_eq!(report["state"], "Unverified");
        assert_eq!(report["source"], "Launch");
        assert!(!has_changes(&report));
    }

    #[test]
    fn launch_report_cannot_overwrite_or_leak_to_a_different_run() {
        struct TestDirectory(PathBuf);
        impl Drop for TestDirectory {
            fn drop(&mut self) {
                for name in ["current", "old"] {
                    let folder = self.0.join(name);
                    let _ = fs::remove_file(folder.join(RUN_REPORT));
                    let _ = fs::remove_dir(folder);
                }
                let _ = fs::remove_dir(&self.0);
            }
        }
        let directory = TestDirectory(std::env::temp_dir().canonicalize().unwrap().join(format!(
            "ih-yaml-preflight-{}-{}",
            std::process::id(),
            clock()
        )));
        fs::create_dir(&directory.0).unwrap();
        let current = directory.0.join("current");
        fs::create_dir(&current).unwrap();
        let first = json!({"filesChanged":["spigot.yml"]});
        store_run_report(&current, "run-a", &first).unwrap();
        assert!(store_run_report(&current, "run-b", &json!({})).is_err());
        assert_eq!(run_report(&current, "run-a").unwrap(), Some(first.clone()));
        assert_eq!(run_report(&current, "run-b").unwrap(), None);

        let archived = directory.0.join("old");
        fs::rename(&current, &archived).unwrap();
        fs::create_dir(&current).unwrap();
        let next = json!({"filesChanged":[]});
        store_run_report(&current, "run-b", &next).unwrap();
        assert_eq!(run_report(&current, "run-a").unwrap(), None);
        assert_eq!(run_report(&current, "run-b").unwrap(), Some(next));
        assert_eq!(run_report(&archived, "run-a").unwrap(), Some(first));
    }
}
