use super::*;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

const FLAT_GENERATOR: &str = r#"{"layers":[{"block":"minecraft:bedrock","height":1},{"block":"minecraft:dirt","height":2},{"block":"minecraft:grass_block","height":1}],"biome":"minecraft:plains","features":false,"lakes":false}"#;
const PORT_REGISTRY: &str = include_str!("../../config/human-port-registry.json");

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
enum Audience {
    Agent,
    Human,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
enum WorldType {
    #[default]
    Flat,
    Normal,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Request {
    audience: Audience,
    project: String,
    instance: String,
    #[serde(default)]
    pub(super) java: String,
    #[serde(default)]
    pub(super) port: u16,
    #[serde(default)]
    pub(super) core_jar: String,
    pub(super) profile: Option<String>,
    pub(super) mojang_jar: Option<String>,
    plugin_jar: Option<String>,
    plugin_sha256: Option<String>,
    #[serde(default)]
    pub(super) permission_jar: String,
    #[serde(default)]
    world_type: WorldType,
    #[serde(default)]
    pub(super) replace: bool,
}

impl Request {
    pub(super) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 16 * 1024 {
            return Err("InvalidPrepareRequest".into());
        }
        let request: Self = serde_json::from_str(raw).map_err(|_| "InvalidPrepareRequest")?;
        if request.replace && request.human() {
            return Err("HumanInstancesCannotBeRebuilt".into());
        }
        if request.profile.is_some() {
            if request.human() || request.replace || !request.java.is_empty()
                || !request.core_jar.is_empty() || request.mojang_jar.is_some()
                || !request.permission_jar.is_empty()
            {
                return Err("InvalidAgentProfileRequest".into());
            }
        } else if request.port == 0 || request.java.is_empty() || request.core_jar.is_empty()
            || request.permission_jar.is_empty()
        {
            return Err("InvalidPrepareIdentity".into());
        }
        let instance = request.instance.to_ascii_lowercase();
        let portable = instance.replace(['.', '_'], "-");
        if !token(&request.project.to_ascii_lowercase())
            || !token(&portable)
            || !token(&instance.split('.').next().unwrap_or("").replace('_', "-"))
            || instance.ends_with('.')
        {
            return Err("InvalidPrepareIdentity".into());
        }
        Ok(request)
    }

    fn parent(&self, root: &Path) -> PathBuf {
        root.join("instances")
            .join(match self.audience {
                Audience::Agent => "Agent",
                Audience::Human => "Human",
            })
            .join(&self.project)
    }

    pub(super) fn target(&self, root: &Path) -> PathBuf {
        self.parent(root).join(&self.instance)
    }

    pub(super) fn matches_agent(&self, name: &str, metadata: &Value) -> bool {
        !self.human() && metadata["schemaVersion"] == 1 && metadata["audience"] == "Agent"
            && metadata["project"] == self.project && metadata["instance"] == self.instance
            && metadata["managedId"] == name
    }

    fn validate_human(&self, profile: &CoreProfile) -> Result<()> {
        let name = &self.instance;
        if self.audience == Audience::Agent {
            return Ok(());
        }
        let project_code = match self.project.as_str() {
            "Lobby" => 0,
            "Example" => 1,
            "ExampleTwo" => 2,
            "ExampleThree" => 3,
            _ => return Err("HumanRouteUnregistered".into()),
        };
        let core_code = match profile.family.as_str() {
            "SPIGOT" => 1,
            "PAPER" => 2,
            "FOLIA" => 3,
            _ => return Err("HumanRouteUnregistered".into()),
        };
        let prefix = format!(
            "{}-{}-{}",
            self.project.to_ascii_lowercase(),
            profile.family.to_ascii_lowercase(),
            profile.minecraft
        );
        let valid_name = name == &prefix
            || name.strip_prefix(&prefix).is_some_and(|tail| {
                tail.strip_prefix('-').is_some_and(|suffix| {
                    !suffix.is_empty()
                        && suffix.len() <= 16
                        && suffix
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                })
            });
        let valid = if self.project == "Lobby" {
            name == "lobby"
                && self.port == 10097
                && profile.family == "SPIGOT"
                && profile.minecraft == "1.8.8"
        } else {
            valid_name && self.port / 100 == core_code * 100 + project_code && self.port % 100 != 0
        };
        let registry: Value = serde_json::from_str(PORT_REGISTRY).map_err(fail)?;
        let entries = registry["entries"]
            .as_array()
            .ok_or("InvalidHumanPortRegistry")?;
        let port = json!(self.port);
        if !valid
            || registry["schemaVersion"] != 1
            || entries
                .iter()
                .filter(|entry| entry["route"] == name.as_str())
                .count()
                != 1
            || entries.iter().filter(|entry| entry["port"] == port).count() != 1
            || !entries
                .iter()
                .any(|entry| entry["port"] == port && entry["route"] == name.as_str())
        {
            return Err("HumanRouteUnregistered".into());
        }
        Ok(())
    }

    pub(super) fn verify(&self, profile: &CoreProfile) -> Result<Assets> {
        self.validate_human(profile)?;
        let plugin = match (
            &self.plugin_jar,
            &self.plugin_sha256,
            self.project == "Lobby",
        ) {
            (None, None, true) => None,
            (Some(path), Some(expected), false) => {
                let source = local_jar(path)?;
                let expected = expected.to_ascii_lowercase();
                if content_revision::file_revision(&source)? != expected {
                    return Err("PreparePluginDigestMismatch".into());
                }
                Some((source, expected))
            }
            _ => return Err("PreparePluginRequired".into()),
        };
        let permission = test_permissions::preflight(&local_jar(&self.permission_jar)?, profile)?;
        Ok(Assets { plugin, permission })
    }

    pub(super) fn stage(
        &self,
        stage: &Path,
        name: &str,
        profile: &CoreProfile,
        assets: &Assets,
    ) -> Result<()> {
        test_permissions::stage(stage, &assets.permission)?;
        if let Some((source, expected)) = &assets.plugin {
            let path = stage.join("plugins").join("project-plugin.jar");
            core_maintenance::copy_verified(source, &path, expected)?;
        }
        let properties = stage.join("server.properties");
        let existing = fs::read_to_string(&properties).map_err(fail)?;
        let level = match (self.world_type, profile.minecraft.as_str()) {
            (WorldType::Flat, "1.8.8" | "1.12.2" | "1.14" | "1.14.4" | "1.16.5") => "FLAT",
            (WorldType::Flat, _) => "minecraft:flat",
            (WorldType::Normal, "1.8.8" | "1.12.2" | "1.14" | "1.14.4" | "1.16.5") => "DEFAULT",
            (WorldType::Normal, _) => "minecraft:normal",
        };
        let mut settings = existing
            .lines()
            .filter(|line| {
                !line.starts_with("level-type=") && !line.starts_with("generator-settings=")
            })
            .collect::<Vec<_>>()
            .join("\n");
        settings.push_str(&format!("\nlevel-type={level}\n"));
        if level == "minecraft:flat" {
            settings.push_str(&format!("generator-settings={FLAT_GENERATOR}\n"));
        }
        fs::write(properties, settings).map_err(fail)?;
        save(
            &stage.join("prepare.json"),
            &json!({
                "schemaVersion":1, "audience": self.audience, "project": self.project,
                "instance":self.instance, "managedId":name, "port": self.port, "core":profile,
                "worldType": self.world_type, "permissionSha256": assets.permission.sha256(),
                "pluginSha256": assets.plugin.as_ref().map(|(_,hash)| hash)
            }),
        )
    }

    pub(super) fn hold_parent(&self, root: &Path) -> Result<Vec<File>> {
        let mut guards = Vec::with_capacity(3);
        let parent = self.parent(root);
        for dir in [
            root.join("instances"),
            parent.parent().ok_or("InvalidPrepareIdentity")?.to_owned(),
            parent,
        ] {
            match fs::create_dir(&dir) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(fail(error)),
            }
            let handle = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x0220_0000)
                .open(&dir)
                .map_err(fail)?;
            let attributes = handle.metadata().map_err(fail)?;
            if !attributes.is_dir() || attributes.file_attributes() & 0x400 != 0 {
                return Err("PrepareDirectoryRedirected".into());
            }
            no_redirect(&dir)?;
            guards.push(handle);
        }
        Ok(guards)
    }

    pub(super) fn human(&self) -> bool {
        self.audience == Audience::Human
    }
}

pub(super) struct Assets {
    plugin: Option<(PathBuf, String)>,
    permission: test_permissions::PinnedArtifact,
}

pub(super) fn local_jar(raw: &str) -> Result<PathBuf> {
    let path = scan_policy::rule_path(Path::new(""), raw)?;
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jar"))
    {
        return Err("InvalidPrepareArtifact".into());
    }
    no_redirect(&path)?;
    if !fs::metadata(&path).map_err(fail)?.is_file() {
        return Err("InvalidPrepareArtifact".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(instance: &str, port: u16) -> Value {
        json!({
            "audience":"Human", "project":"Example", "instance":instance,
            "port":port, "java":"java.exe", "coreJar":"core.jar", "permissionJar":"permission.jar"
        })
    }

    #[test]
    fn prepare_keeps_registered_human_route_separate_and_refuses_traversal() {
        let root = Path::new("workspace");
        let request = Request::parse(&shape("example-paper-26.3", 20197).to_string()).unwrap();
        let paper = CoreProfile { unverified: false, java: None,
            family: "PAPER".into(),
            minecraft: "26.3".into(),
            build: "205".into(),
            sha256: "0".repeat(64),
        };
        assert!(request.validate_human(&paper).is_ok());
        assert_eq!(
            request.target(root),
            root.join("instances/Human/Example/example-paper-26.3")
        );
        let wrong = Request::parse(&shape("example-paper-26.3", 20198).to_string()).unwrap();
        assert!(wrong.validate_human(&paper).is_err());
        for unsafe_name in ["../stale", "con.jar", "name.", "other/child"] {
            assert!(Request::parse(&shape(unsafe_name, 20197).to_string()).is_err());
        }
    }
}
