//! First-start rules are bound to the created core and consumed before any write.
use super::*;
use serde_json::Map;

const PLAN: &str = ".instanceharbor-first-ready.json";
const CLAIM: &str = ".instanceharbor-first-ready-claim.json";
const RESULT: &str = ".instanceharbor-first-ready-result.json";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(in crate::windows) struct Plan {
    sha256: String,
    rules: Vec<(String, String)>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Claim {
    run: String,
    created_at: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Outcome {
    run: String,
    state: String,
    checked: usize,
    error: Option<Value>,
}

pub(in crate::windows) fn supports(version: &str, section: &str, key: &str) -> bool {
    let id = if section.is_empty() { key.to_owned() } else { format!("{section}.{key}") };
    Target::field_for(version, &id).is_ok()
}

fn validated_rules(version: &str, rules: &[(String, String)]) -> Result<Vec<(Field, String)>> {
    if rules.is_empty() || rules.len() > RULE_KEYS.len() + 1 {
        return Err("InvalidFirstReadyRules".into());
    }
    let mut keys = std::collections::BTreeSet::new();
    let mut fields = rules
        .iter()
        .map(|(key, value)| {
            let field = Target::field_for(version, key)?;
            if !field.valid(value) || !keys.insert(key) {
                return Err("InvalidFirstReadyRules".into());
            }
            Ok((field, value.clone()))
        })
        .collect::<Result<Vec<_>>>()?;
    // Apply the clock setting after the gamerule governing its movement.
    fields.sort_by_key(|(field, _)| field.is_time());
    Ok(fields)
}

pub(in crate::windows) fn stage(
    directory: &Path,
    core: &CoreProfile,
    values: &Map<String, Value>,
) -> Result<()> {
    let rules = values
        .iter()
        .filter(|(key, _)| {
            Target::field_for(&core.minecraft, key).is_ok()
        })
        .map(|(key, value)| {
            Ok((
                key.clone(),
                value.as_str().ok_or("InvalidFirstReadyRules")?.to_owned(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    if rules.is_empty() {
        return Ok(());
    }
    validated_rules(&core.minecraft, &rules)?;
    save(
        &directory.join(PLAN),
        &Plan {
            sha256: core.sha256.clone(),
            rules,
        },
    )
}

fn optional<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(file_error("inspect-first-ready", path, error)),
        Ok(_) => no_redirect(path)?,
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(fail)?
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    if bytes.len() > 4096 {
        return Err("InvalidFirstReadyRecord".into());
    }
    serde_json::from_slice(&bytes).map(Some).map_err(fail)
}

pub(in crate::windows) fn pending(directory: &Path, instance: &Instance) -> Result<Option<Plan>> {
    if !instance.core_local_create || optional::<Claim>(&directory.join(CLAIM))?.is_some() {
        return Ok(None);
    }
    optional(&directory.join(PLAN))
}

fn claim(directory: &Path, run: &Run) -> Result<Claim> {
    let claim = Claim {
        run: run.id.clone(),
        created_at: epoch_ms(),
    };
    let path = directory.join(CLAIM);
    no_redirect(directory)?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(file_error("inspect-first-ready-claim", &path, error)),
        Ok(_) => return Err("FirstReadyAlreadyClaimed".into()),
    }
    // The caller holds the directory's runtime lease throughout this operation.
    save(&path, &claim)?;
    Ok(claim)
}

pub(in crate::windows) fn apply(
    root: &Path,
    directory: &Path,
    config: &Instance,
    run: &Run,
    plan: Plan,
    write: impl Fn(&str, Instant) -> Result<()>,
) -> Result<()> {
    let claim = claim(directory, run)?;
    let mut checked = 0;
    let result = (|| {
        let core = verified_core_profile(&directory.join("server.jar"))?
            .filter(|core| core.sha256 == plan.sha256)
            .filter(|core| {
                config
                    .core_profile
                    .as_ref()
                    .is_some_and(|saved| saved.sha256 == core.sha256)
            })
            .ok_or("FirstReadyCoreChanged")?;
        apply_rules(root, directory, config, run, &core, &plan.rules, write, || checked += 1)
    })();
    save(
        &directory.join(RESULT),
        &Outcome {
            run: claim.run,
            checked,
            state: if result.is_ok() {
                "AwaitingNormalStop"
            } else {
                "Unconfirmed"
            }
            .into(),
            error: result
                .as_ref()
                .err()
                .map(|e: &String| crate::diagnostics::safe_error(e)),
        },
    )?;
    result
}

pub(super) fn apply_rules(
    root: &Path,
    directory: &Path,
    config: &Instance,
    run: &Run,
    core: &CoreProfile,
    requested: &[(String, String)],
    write: impl Fn(&str, Instant) -> Result<()>,
    mut checked: impl FnMut(),
) -> Result<()> {
        let rules = validated_rules(&core.minecraft, requested)?;
        let output = child_directory(directory, "logs")?.join("latest.log");
        let assert_run = || {
            let observation = status(root, config)?;
            if observation["state"] != "Listening"
                || observation["consoleReady"] != true
                || !observation["stopPhase"].is_null()
                || !observation["stopToken"].is_null()
                || serde_json::from_value::<Run>(observation["run"].clone()).map_err(fail)? != *run
            {
                return Err("FirstReadyRunChanged".into());
            }
            Ok::<(), String>(())
        };
        let command = |line: &str, parse: &dyn Fn(&str) -> Option<String>| -> Result<String> {
            assert_run()?;
            if !allowed_line(line) {
                return Err("InvalidWorldCommand".into());
            }
            let mut log = RunOutput::open(&output)?;
            write(line, Instant::now() + Duration::from_millis(500))?;
            // The server can still be busy loading chunks after announcing readiness.
            // Give its reply time to reach the log without sending the command again.
            let expiry = Instant::now() + Duration::from_secs(12);
            while Instant::now() < expiry {
                assert_run()?;
                if let Some(reply) = log.reply(&parse)? {
                    assert_run()?;
                    return Ok(reply);
                }
                thread::sleep(Duration::from_millis(75));
            }
            Err("WorldReplyUnconfirmed".into())
        };
        for (field, target) in rules {
            let moving = if field.is_time() {
                let flow = Target::field_for(&core.minecraft, FLOW_RULE)?;
                command(&flow.query_line(), &|text| flow.query_reply(text))? == "true"
            } else {
                false
            };
            let query = || command(&field.query_line(), &|text| field.query_reply(text));
            if query()? != target {
                command(&field.set_line(&target), &|text| {
                    field.set_reply(text, &target).then(String::new)
                })?;
                if !matches_readback(&query()?, &target, moving) {
                    return Err("WorldReadbackMismatch".into());
                }
            }
            checked();
        }
        Ok(())
}

pub(in crate::windows) fn activity(directory: &Path, name: &str) -> Result<Option<Value>> {
    let Some(claim) = optional::<Claim>(&directory.join(CLAIM))? else {
        return Ok(None);
    };
    if !token(&claim.run) {
        return Err("InvalidFirstReadyRecord".into());
    }
    let result = optional::<Outcome>(&directory.join(RESULT))?;
    let state = result
        .filter(|result| result.run == claim.run && result.state == "AwaitingNormalStop")
        .map_or("Unconfirmed", |_| "VerifiedLive");
    Ok(Some(json!({
        "id": format!("first-ready-{}", claim.run),
        "action": "world-first-ready",
        "instance": name,
        "createdAt": claim.created_at,
        "state": state,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_ready_accepts_valid_time_and_rejects_unavailable_legacy_rules() {
        assert!(supports("26.3", "gamerules", "spawn_mobs"));
        assert!(supports("1.8.8", "gamerules", "spawn_mobs"));
        assert!(!supports("1.8.8", "gamerules", "max_entity_cramming"));
        assert!(supports("26.3", "", "initialTime"));
        assert!(validated_rules("26.3", &[("initialTime".into(), "30".into())]).is_ok());
        assert!(validated_rules("26.3", &[("initialTime".into(), "24000".into())]).is_err());
        assert!(validated_rules(
            "26.3",
            &[("gamerules.spawn_mobs".into(), "false\nstop".into())]
        )
        .is_err());
        assert!(validated_rules("26.3", &[("gamerules.spawn_mobs".into(), "true".into())]).is_ok());
    }
}
