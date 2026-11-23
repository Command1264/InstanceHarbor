//! Read-only provisioning checks. Configured values and observed world data are separate evidence.
use super::*;

#[path = "prepare_diagnostics/nbt.rs"]
mod nbt;
pub(super) use nbt::observe as observe_saved_world;
pub(super) use nbt::write_rules as write_saved_rules;

const FIRST_READY_PLAN: &str = ".instanceharbor-first-ready.json";
const PREPARE_RECORD: &str = "prepare.json";
const MAX_METADATA_BYTES: u64 = 16 * 1024;

fn finding(findings: &mut Vec<Value>, check: &str, result: Result<()>) {
    if let Err(error) = result {
        findings.push(json!({"check":check,"reason":crate::diagnostics::safe_error(&error)}));
    }
}

fn read_metadata(path: &Path) -> Result<Option<Value>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(file_error("inspect-prepare-metadata", path, error)),
    };
    if !metadata.is_file() || metadata.len() > MAX_METADATA_BYTES {
        return Err("InvalidPrepareMetadata".into());
    }
    no_redirect(path)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(fail)?
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err("InvalidPrepareMetadata".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "InvalidPrepareMetadata".into())
}

fn initialization(root: &Path, core: Option<&CoreProfile>) -> Result<Value> {
    let args = core.map_or_else(Vec::new, |core| {
        vec![core.family.clone(), core.minecraft.clone()]
    });
    policy_settings::dispatch(root, "get-initialization", &args)
        .ok_or("InitializationCheckUnavailable")?
}

fn doctor(root: &Path, name: Option<&str>) -> Result<Value> {
    let mut findings = Vec::new();
    for directory in [
        "instances",
        "registry",
        "jobs",
        "runs",
        "history",
        "staging",
    ] {
        finding(
            &mut findings,
            directory,
            child_directory(root, directory).map(|_| ()),
        );
    }
    finding(
        &mut findings,
        "scanPolicy",
        scan_policy::dispatch(root, "scan-policy", &[])
            .ok_or_else(|| "ScanPolicyCheckUnavailable".to_owned())
            .and_then(|result| result.map(|_| ())),
    );
    finding(
        &mut findings,
        "initialization",
        initialization(root, None).map(|_| ()),
    );
    let mut observed = None;
    if let Some(name) = name {
        if !token(name) {
            return Err("InvalidInstanceName".into());
        }
        match instance(root, name) {
            Ok(config) => {
                let automatic_java = config.can_inherit_java() && config.java.as_os_str().is_empty();
                if !automatic_java {
                    finding(
                        &mut findings,
                        "registeredJavaPath",
                        (|| {
                            no_redirect(&config.java)?;
                            if !config.java.is_file()
                                || !config.java.file_name().is_some_and(|name| name.eq_ignore_ascii_case("java.exe"))
                            {
                                return Err("InvalidJavaExecutable".into());
                            }
                            Ok(())
                        })(),
                    );
                }
                finding(
                    &mut findings,
                    "scanAdmission",
                    admitted(root, name).and_then(|allowed| {
                        if allowed && !config.removed {
                            Ok(())
                        } else {
                            Err("InstanceExcludedOrInvalid".into())
                        }
                    }),
                );
                finding(
                    &mut findings,
                    "registeredCore",
                    (|| {
                        let jar = core_artifact::jar(&instance_path(root, name)?, &instance(root, name)?);
                        no_redirect(&jar)?;
                        let digest = content_revision::file_revision(&jar)?;
                        match &config.core_profile {
                            Some(core) if core.sha256 == digest => Ok(()),
                            Some(_) => Err("CoreIdentityChanged".into()),
                            None if verified_core_profile(&jar)?.is_some() => Ok(()),
                            None => Err("CoreIdentityUnverified".into()),
                        }
                    })(),
                );
                match status(root, &config) {
                    Ok(state) => {
                        let state_name = state["state"].clone();
                        finding(
                            &mut findings,
                            "port",
                            match state_name.as_str() {
                                Some("Stopped") => port_free(config.port),
                                Some("Listening" | "Starting" | "Stopping") => Ok(()),
                                _ => Err("InstanceStatusUnconfirmed".into()),
                            },
                        );
                        let mut row = json!({"state":state_name,"port":config.port});
                        if automatic_java {
                            row["javaSelection"] = json!("Automatic");
                        }
                        observed = Some(row);
                    }
                    Err(error) => finding(&mut findings, "status", Err(error)),
                }
            }
            Err(error) => finding(&mut findings, "registration", Err(error)),
        }
    }
    Ok(
        json!({"ok":findings.is_empty(),"findings":findings,"checkedAt":epoch_ms(),
        "instance":name,"observed":observed,"javaProbe":"pathOnly","remote":false}),
    )
}

fn first_ready_expected(directory: &Path, core: &CoreProfile) -> Result<Option<Value>> {
    let Some(plan) = read_metadata(&directory.join(FIRST_READY_PLAN))? else {
        return Ok(None);
    };
    let core_hash = plan["sha256"]
        .as_str()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("InvalidFirstReadyRecord")?;
    let rows = plan["rules"].as_array().ok_or("InvalidFirstReadyRecord")?;
    let mut values = serde_json::Map::new();
    for row in rows {
        let pair = row
            .as_array()
            .filter(|row| row.len() == 2)
            .ok_or("InvalidFirstReadyRecord")?;
        let key = pair[0].as_str().ok_or("InvalidFirstReadyRecord")?;
        let value = pair[1].as_str().ok_or("InvalidFirstReadyRecord")?;
        let (section, rule) = key.rsplit_once('.').unwrap_or(("", key));
        if !world_live::first_ready::supports(&core.minecraft, section, rule)
            || values.insert(key.to_owned(), json!(value)).is_some()
        {
            return Err("InvalidFirstReadyRecord".into());
        }
    }
    Ok(Some(
        json!({"values":values,"originalCore":core_hash,"source":"FirstReadyPlan"}),
    ))
}


fn configured_drift(
    defaults: &Value,
    snapshot: &Value,
    expected_rules: Option<&Value>,
    configured_world_type: Option<&str>,
    modern: bool,
) -> Result<Vec<Value>> {
    let desired = defaults["rows"]
        .as_array()
        .ok_or("DefaultsBaselineUnavailable")?;
    let observed = snapshot["rows"]
        .as_array()
        .ok_or("WorldSettingsReadbackUnavailable")?;
    let mut rows = Vec::new();
    for field in desired {
        let Some(key) = field["key"].as_str() else {
            return Err("DefaultsBaselineUnavailable".into());
        };
        let frozen = expected_rules.and_then(|plan| plan["values"][key].as_str());
        let expected = if let Some(frozen) = frozen {
            Some(frozen)
        } else if key == "worldType" {
            configured_world_type.or_else(|| field["inherited"].as_str())
        } else {
            field["inherited"].as_str()
        };
        let Some(expected) = expected else { continue };
        if field["status"] == "unsupported" {
            continue;
        }
        let file_readback = !nbt::persisted_field(key, modern);
        let actual = file_readback
            .then(|| {
                observed
                    .iter()
                    .find(|row| row["key"] == key && row["editable"] == true)
            })
            .flatten()
            .and_then(|row| row["value"].as_str());
        let state = match actual {
            Some(value) if expected == value => "Match",
            Some(_) => "Drift",
            None => "Unconfirmed",
        };
        rows.push(json!({
            "setting":key,"expected":expected,"observed":actual,"state":state,
            "expectedSource":if frozen.is_some() {"FirstReadyPlan"} else if key == "worldType" && configured_world_type.is_some() {"PrepareRecord"} else {"CurrentInitialization"},
            "observedSource":if file_readback {"ConfigurationFile"} else {"PersistedWorld"},
        }));
    }
    Ok(rows)
}

fn defaults_diff(root: &Path, name: &str) -> Result<Value> {
    if !token(name) || !admitted(root, name)? {
        return Err("InstanceExcludedOrInvalid".into());
    }
    let config = instance(root, name)?;
    if config.removed || registered_velocity(&config).is_some() {
        return Err("InstanceUnavailableForDefaultsDiff".into());
    }
    let directory = instance_path(root, name)?;
    let (snapshot, core) = {
        let source = core_artifact::jar(&directory, &config);
        no_redirect(&source)?;
        let digest = content_revision::file_revision(&source)?;
        if config
            .core_profile
            .as_ref()
            .is_some_and(|saved| saved.sha256 != digest)
        {
            return Err("CoreIdentityChanged".into());
        }
        let core =
            core_maintenance::trusted_profile(&config, digest).ok_or("CoreIdentityUnverified")?;
        let snapshot = policy_settings::diagnostic_snapshot(root, name, &core)?;
        (snapshot, core)
    };
    let defaults = initialization(root, Some(&core))?;
    let plan = first_ready_expected(&directory, &core)?;
    let prepared = read_metadata(&directory.join(PREPARE_RECORD))?;
    let world_type = prepared
        .as_ref()
        .and_then(|record| record["worldType"].as_str());
    let mut rows = configured_drift(
        &defaults,
        &snapshot,
        plan.as_ref(),
        world_type,
        core.minecraft.starts_with("26."),
    )?;
    let state = status(root, &config)?;
    let wanted: Vec<_> = rows
        .iter()
        .filter(|row| row["observedSource"] == "PersistedWorld")
        .filter_map(|row| row["setting"].as_str())
        .collect();
    let requested = wanted.len();
    let mut saved = std::collections::BTreeMap::new();
    if requested > 0 && state["state"] == "Stopped" {
        if let Ok(_lease) = claim_runtime_directory(&directory) {
            let revision = registration_revision(root, name)?;
            if status(root, &config)?["state"] == "Stopped" {
                saved = observe_saved_world(&directory, &core, &wanted);
                if registration_revision(root, name)? != revision
                    || status(root, &config)?["state"] != "Stopped"
                {
                    saved.clear();
                }
            }
        }
    }
    for row in &mut rows {
        if let Some(observed) = row["setting"].as_str().and_then(|key| saved.get(key)) {
            let state = if row["expected"] == observed.as_str() {
                "Match"
            } else {
                "Drift"
            };
            row["observed"] = json!(observed);
            row["state"] = json!(state);
        }
    }
    let readback = match (requested, saved.len()) {
        (0, _) => "NotRequested",
        (_, 0) => "Unconfirmed",
        (requested, actual) if requested == actual => "Observed",
        _ => "Partial",
    };
    let ok = !rows.is_empty() && rows.iter().all(|row| row["state"] == "Match");
    Ok(
        json!({"ok":ok,"instance":name,"instanceState":state["state"],"checkedAt":epoch_ms(),
        "rows":rows,"hasBaseline":!rows.is_empty(),
        "persistedWorldReadback":readback,"firstReadyPlan":plan}),
    )
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("doctor", []) => doctor(root, None),
        ("doctor", [name]) => doctor(root, Some(name)),
        ("defaults-diff", [name]) => defaults_diff(root, name),
        ("doctor" | "defaults-diff", _) => Err("InvalidPrepareDiagnosticArguments".into()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_report_a_missing_directory_without_creating_it() {
        let workspace = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "ih-prepare-doctor-{}-{}",
            std::process::id(),
            clock()
        ));
        super::super::dispatch(&[workspace.to_string_lossy().into_owned(), "init".into()]).unwrap();
        assert_eq!(doctor(&workspace, None).unwrap()["ok"], true);
        fs::remove_dir(workspace.join("jobs")).unwrap();
        let result = doctor(&workspace, None).unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["findings"][0]["check"], "jobs");
        assert!(!workspace.join("jobs").exists());
        assert_eq!(doctor(&workspace, Some("missing")).unwrap()["ok"], false);
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn configuration_match_never_substitutes_for_persisted_world_readback() {
        let defaults = json!({"rows":[
            {"key":"serverProperties.view-distance","inherited":"7"},
            {"key":"serverProperties.allow-nether","inherited":"true"},
            {"key":"worldType","inherited":"Normal"},
            {"key":"gamerules.spawn_mobs","inherited":"true"},
        ]});
        let snapshot = json!({"rows":[
            {"key":"serverProperties.view-distance","editable":true,"value":"7"},
            {"key":"serverProperties.allow-nether","editable":true,"value":"false"},
            {"key":"worldType","editable":true,"value":"Flat"},
            {"key":"gamerules.spawn_mobs","editable":true,"value":"false"},
        ]});
        let plan = json!({"values":{"gamerules.spawn_mobs":"false"}});
        let rows = configured_drift(&defaults, &snapshot, Some(&plan), Some("Flat"), true).unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row["state"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["Match", "Drift", "Unconfirmed", "Unconfirmed"]
        );
        assert_eq!(rows[2]["observed"], Value::Null);
        assert_eq!(rows[3]["expected"], "false");
        assert_eq!(rows[3]["expectedSource"], "FirstReadyPlan");
        assert_eq!(rows[3]["observed"], Value::Null);
    }
}
