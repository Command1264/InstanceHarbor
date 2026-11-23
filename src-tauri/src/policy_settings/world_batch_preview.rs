use super::*;

const FIELDS_TO_COMPARE: &[&str] = &[
    "serverProperties.motd",
    "serverProperties.max-players",
    "serverProperties.spawn-protection",
    "serverProperties.view-distance",
    "serverProperties.simulation-distance",
    "serverProperties.difficulty",
    "serverProperties.gamemode",
    "serverProperties.allow-nether",
    "spigot.item-despawn-rate",
    "spigot.mob-spawn-range",
];

fn target_source(
    settings: &Value,
    saved: &BTreeSet<String>,
    profile: Profile,
    field: &Field,
) -> &'static str {
    if settings["profiles"][profile.key()]
        .get(field.id())
        .is_some()
    {
        "VersionOverride"
    } else if field.section == "serverProperties"
        && (settings["serverProperties"].get(field.key).is_some()
            || BASE_FIELDS
                .iter()
                .any(|(key, property, _)| *property == field.key && saved.contains(*key)))
    {
        "WorkspaceDefault"
    } else {
        "BuiltInDefault"
    }
}

fn compare(
    snapshot: &WorldSnapshot,
    settings: &Value,
    saved: &BTreeSet<String>,
) -> Result<Vec<Value>> {
    let targets = snapshot
        .profile
        .map(|profile| selected_defaults(settings, profile))
        .transpose()?;
    FIELDS_TO_COMPARE
        .iter()
        .map(|id| {
            let field = field(id)?;
            let mut row = field.describe(snapshot.profile, true);
            let Some(profile) = snapshot.profile else {
                row["status"] = json!("Unverified");
                return Ok(row);
            };
            if row["status"] == "unsupported" {
                row["status"] = json!("Unsupported");
                return Ok(row);
            }

            let reason = snapshot.reason(field);
            let current = snapshot
                .observe(field)
                .and_then(|property| logical_value(field, &property.value, profile).ok());
            let desired = targets
                .as_ref()
                .and_then(|values| values.get(*id))
                .and_then(Value::as_str);
            let target = desired.and_then(|value| field.normalize(value, profile).ok());
            let source = desired.map(|_| target_source(settings, saved, profile, field));
            let (state, explanation) = match (current.as_deref(), target.as_deref()) {
                _ if !reason.is_empty() => ("Unverified", reason),
                (Some(current), Some(target)) if current == target => ("Match", ""),
                (Some(_), Some(_)) => ("Different", ""),
                (Some(_), None) if desired.is_none() => ("NoTarget", "初始化預設尚未指定此欄位。"),
                (None, _) => ("Unverified", "無法核對此實例的目前值。"),
                _ => ("Unverified", "無法核對此版本的預設值。"),
            };
            Ok(json!({
                "key":row["key"], "label":row["label"], "source":row["source"],
                "current":current, "target":target, "targetSource":source,
                "status":state, "reason":explanation
            }))
        })
        .collect()
}

fn describe_instance(
    root: &Path,
    name: &str,
    settings: &Value,
    saved: &BTreeSet<String>,
) -> Result<Value> {
    let snapshot = WorldSnapshot::read(root, name)?;
    let config = instance(root, name)?;
    let state = status(root, &config)
        .ok()
        .and_then(|value| value["state"].as_str().map(str::to_owned));
    let pending = deferred_settings::has_pending(root, name).ok();
    Ok(json!({
        "name":name, "profile":snapshot.profile.map(profile_json), "state":state, "pending":pending,
        "revision":snapshot.revision, "fields":compare(&snapshot, settings, saved)?
    }))
}

pub(super) fn preview(root: &Path, request: &str) -> Result<Value> {
    if request.len() > 8 * 1024 {
        return Err("InvalidWorldPreviewTargets".into());
    }
    let names: Vec<String> =
        serde_json::from_str(request).map_err(|_| "InvalidWorldPreviewTargets")?;
    let mut unique = BTreeSet::new();
    if names.is_empty()
        || names.len() > 32
        || names
            .iter()
            .any(|name| !token(name) || !unique.insert(name))
    {
        return Err("InvalidWorldPreviewTargets".into());
    }
    let (settings, saved) = initialization_with_sources(root)?;
    let source_revision =
        content_revision::revision(&serde_json::to_vec(&(&settings, &saved)).map_err(fail)?)?;
    let rows: Vec<_> = names
        .iter()
        .map(|name| {
            describe_instance(root, name, &settings, &saved).unwrap_or_else(|error| {
                let reason = if matches!(
                    error.as_str(),
                    "InstanceExcludedOrInvalid" | "InstanceRemoved"
                ) {
                    "此實例已移除、被掃描政策排除或無法識別。"
                } else {
                    "無法讀取此實例的設定。請核對登記及設定檔後重新讀取。"
                };
                json!({"name":name,"error":reason,"fields":[]})
            })
        })
        .collect();
    Ok(json!({"instances":rows,"sourceRevision":source_revision}))
}
