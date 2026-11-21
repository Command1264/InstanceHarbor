//! Read-only preflight for a separately prepared legacy-forwarding probe.
use super::*;
use std::collections::HashSet;

const MAX_MANIFEST_BYTES: usize = 4096;
const CONTRACT: &str = "v2-legacy-compatibility-preflight-1";
const VIA_PLUGINS: [(&str, &str); 3] = [
    (
        "ViaVersion-5.12.0.jar",
        "72c40a6a702d67f226fc9a0d8ad82aba1483fdabe2e6159bcdddb2dc070750b0",
    ),
    (
        "ViaBackwards-5.12.0.jar",
        "194e9250224632274d7b3c17e411e031a9223c1863c6f5138d53c721f07ab78d",
    ),
    (
        "ViaRewind-4.2.0.jar",
        "d6634ba57bb82d5161c68dfb393571cdf40511a0beb1b04b8c7ed794a3532c6a",
    ),
];
const VELOCITY_TEMPLATE: &str = include_str!("../../../config/compat-probe/velocity.toml");
const LOBBY_TEMPLATE: &str = include_str!("../../../config/compat-probe/lobby.properties");
const PAPER_TEMPLATE: &str = include_str!("../../../config/compat-probe/paper.properties");
const SPIGOT_TEMPLATE: &str = include_str!("../../../config/compat-probe/spigot.yml");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u8,
    probe_id: String,
    proxy: String,
    lobby: String,
    paper: String,
}

impl Manifest {
    fn parse(raw: &str) -> Result<Self> {
        if raw.len() > MAX_MANIFEST_BYTES {
            return Err("InvalidCompatibilityProbeManifest".into());
        }
        let value: Self =
            serde_json::from_str(raw).map_err(|_| "InvalidCompatibilityProbeManifest")?;
        let names = [&value.proxy, &value.lobby, &value.paper];
        let mut distinct = HashSet::new();
        if value.schema_version != 1
            || !token(&value.probe_id)
            || names
                .iter()
                .any(|name| !token(name) || !distinct.insert(name.to_ascii_lowercase()))
        {
            return Err("InvalidCompatibilityProbeManifest".into());
        }
        Ok(value)
    }
}

#[derive(Clone, Copy)]
enum Component {
    Proxy,
    Lobby,
    Paper,
}

impl Component {
    fn label(self) -> &'static str {
        match self {
            Self::Proxy => "proxy",
            Self::Lobby => "lobby",
            Self::Paper => "paper",
        }
    }

    fn supports(self, profile: &CoreProfile) -> bool {
        match self {
            Self::Proxy => profile.family == "VELOCITY" && profile.minecraft == "4.2.1-SNAPSHOT",
            Self::Lobby => profile.family == "SPIGOT" && profile.minecraft == "1.8.8",
            Self::Paper => profile.family == "PAPER" && profile.minecraft == "26.2",
        }
    }
}

struct Registered {
    role: Component,
    record: Instance,
    directory: PathBuf,
}

fn registered(root: &Path, probe: &Path, name: &str, role: Component) -> Result<Registered> {
    let record = instance(root, name)?;
    let expected = probe.join(role.label());
    let registered = scan_policy::registered_path(root, name, record.directory.as_deref())?;
    if record.removed || registered != expected || !admitted(root, name)? {
        return Err("CompatibilityProbeInstanceOutsideProbe".into());
    }
    let directory = child_directory(probe, role.label())?;
    Ok(Registered {
        role,
        record,
        directory,
    })
}

fn verify_text(path: &Path, expected: &str) -> Result<()> {
    no_redirect(path)?;
    let mut actual = String::new();
    File::open(path)
        .map_err(fail)?
        .take(16 * 1024)
        .read_to_string(&mut actual)
        .map_err(fail)?;
    // The preflight checks canonical fixtures and permits Windows line endings.
    // Arbitrary TOML, properties, or YAML cannot prove this narrow probe contract.
    if !actual.lines().eq(expected.lines()) {
        return Err("CompatibilityProbeConfigurationChanged".into());
    }
    Ok(())
}

fn proxy_config(proxy: u16, lobby: u16, paper: u16) -> String {
    VELOCITY_TEMPLATE
        .lines()
        .map(|line| {
            if line.starts_with("bind = ") {
                format!("bind = \"127.0.0.1:{proxy}\"")
            } else if line.starts_with("lobby = ") {
                format!("lobby = \"127.0.0.1:{lobby}\"")
            } else if line.starts_with("paper = ") {
                format!("paper = \"127.0.0.1:{paper}\"")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn server_config(template: &str, port: u16) -> String {
    template
        .lines()
        .map(|line| {
            if line.starts_with("server-port=") {
                format!("server-port={port}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn no_extra_files(directory: &Path, names: &[&str]) -> Result<()> {
    match fs::symlink_metadata(directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound && names.is_empty() => return Ok(()),
        Err(error) => return Err(fail(error)),
        Ok(_) => no_redirect(directory)?,
    }
    for entry in fs::read_dir(directory).map_err(fail)? {
        let name = entry.map_err(fail)?.file_name();
        if !name.to_str().is_some_and(|name| names.contains(&name)) {
            return Err("CompatibilityProbeUnexpectedPlugin".into());
        }
    }
    Ok(())
}

fn artifact_digest(path: &Path) -> Result<String> {
    no_redirect(path)?;
    if !path.is_file() {
        return Err("CompatibilityProbeArtifactMissing".into());
    }
    content_revision::file_revision(path)
}

fn check_component(root: &Path, target: &Registered) -> Result<Value> {
    let jar = target.directory.join("server.jar");
    let digest = artifact_digest(&jar)?;
    let profile = pinned_core_profile(digest)
        .filter(|profile| target.role.supports(profile))
        .ok_or("CompatibilityProbeArtifactUnverified")?;
    let registration_matches = target.record.core_profile.as_ref().map_or(
        !matches!(target.role, Component::Proxy),
        |saved| {
            saved.sha256 == profile.sha256
                && saved.family == profile.family
                && saved.minecraft == profile.minecraft
                && saved.build == profile.build
        },
    );
    if !registration_matches {
        return Err("CompatibilityProbeRegistrationChanged".into());
    }
    if status(root, &target.record)?["state"] != "Stopped" {
        return Err("CompatibilityProbeInstanceNotStopped".into());
    }
    Ok(
        json!({"role":target.role.label(),"name":target.record.name,"port":target.record.port,
        "configuredAddress":format!("127.0.0.1:{}",target.record.port),"profile":profile,"observedState":"Stopped"}),
    )
}

pub(super) fn check(root: &Path, raw: &str) -> Result<Value> {
    let manifest = Manifest::parse(raw)?;
    let parent = child_directory(&child_directory(root, "instances")?, "CompatProbe")?;
    let probe = child_directory(&parent, &manifest.probe_id)?;
    let proxy = registered(root, &probe, &manifest.proxy, Component::Proxy)?;
    let lobby = registered(root, &probe, &manifest.lobby, Component::Lobby)?;
    let paper = registered(root, &probe, &manifest.paper, Component::Paper)?;
    let ports = [proxy.record.port, lobby.record.port, paper.record.port];
    if ports.contains(&25565)
        || ports[0] == ports[1]
        || ports[0] == ports[2]
        || ports[1] == ports[2]
    {
        return Err("CompatibilityProbePortsNotIsolated".into());
    }
    verify_text(
        &proxy.directory.join("velocity.toml"),
        &proxy_config(ports[0], ports[1], ports[2]),
    )?;
    for (target, template) in [(&lobby, LOBBY_TEMPLATE), (&paper, PAPER_TEMPLATE)] {
        verify_text(
            &target.directory.join("server.properties"),
            &server_config(template, target.record.port),
        )?;
        verify_text(&target.directory.join("spigot.yml"), SPIGOT_TEMPLATE)?;
        verify_text(&target.directory.join("eula.txt"), "eula=true")?;
        no_extra_files(&target.directory.join("plugins"), &[])?;
    }
    // A generated or custom Paper config could override legacy forwarding.
    // Require the fresh fixture rather than interpreting unsupported YAML.
    match fs::symlink_metadata(paper.directory.join("config")) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(fail(error)),
        Ok(_) => return Err("CompatibilityProbeConfigurationChanged".into()),
    }
    let plugin_directory = proxy.directory.join("plugins");
    let names = VIA_PLUGINS.map(|(name, _)| name);
    no_extra_files(&plugin_directory, &names)?;
    for (name, expected) in VIA_PLUGINS {
        if artifact_digest(&plugin_directory.join(name))? != expected {
            return Err("CompatibilityProbePluginChanged".into());
        }
    }
    let components = [&proxy, &lobby, &paper]
        .iter()
        .map(|row| check_component(root, row))
        .collect::<Result<Vec<_>>>()?;
    let plugins = VIA_PLUGINS.map(|(name, sha256)| json!({"name":name,"sha256":sha256}));
    Ok(
        json!({"contract":CONTRACT,"probeId":manifest.probe_id,"state":"VerifiedConfiguration",
        "components":components,"plugins":plugins,"forwarding":"legacy",
        "processStartVerified":false,"clientConnectionVerified":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_rejects_duplicate_registrations_and_paths() {
        let raw = r#"{"schemaVersion":1,"probeId":"isolated","proxy":"probe-p","lobby":"probe-l","paper":"probe-b"}"#;
        assert!(Manifest::parse(raw).is_ok());
        assert!(Manifest::parse(&raw.replace("probe-l", "PROBE-P")).is_err());
        assert!(Manifest::parse(&raw.replace("isolated", "../Human")).is_err());
        assert!(Manifest::parse(&raw.replace("probe-b", "a/b")).is_err());
        assert!(Manifest::parse(&raw.replace("schemaVersion", "version")).is_err());
    }

    #[test]
    fn config_uses_exact_distinct_ports_without_chained_substitution() {
        let config = proxy_config(30566, 30567, 23951);
        assert_eq!(velocity_endpoint(&config).unwrap(), (Ipv4Addr::LOCALHOST, 30566));
        assert!(config.contains("lobby = \"127.0.0.1:30567\""));
        assert!(config.contains("paper = \"127.0.0.1:23951\""));
        assert!(config.contains("player-info-forwarding-mode = \"legacy\""));
        assert!(server_config(PAPER_TEMPLATE, 23951).contains("server-port=23951"));
    }
}
