use super::*;

pub(super) const PROFILES: &[(&str, &[&str])] = &[
    (
        "PAPER",
        &[
            "1.14", "1.14.4", "1.20.6", "1.21.8", "26.1.2", "26.2", "26.3",
        ],
    ),
    ("SPIGOT", &["1.8.8", "1.12.2", "1.14", "1.16.5", "26.2"]),
    ("FOLIA", &["26.1.2"]),
];

#[derive(Clone, Copy)]
pub(super) struct Profile {
    pub(super) family: &'static str,
    pub(super) version: &'static str,
}

impl Profile {
    pub(super) fn resolve(family: &str, version: &str) -> Result<Self> {
        PROFILES
            .iter()
            .find_map(|(candidate, versions)| {
                candidate
                    .eq_ignore_ascii_case(family)
                    .then(|| {
                        versions
                            .iter()
                            .find(|known| **known == version)
                            .map(|version| Self {
                                family: candidate,
                                version,
                            })
                    })
                    .flatten()
            })
            .ok_or_else(|| format!("UnknownInitializationProfile: {family} {version}"))
    }

    pub(super) fn key(self) -> String {
        format!("{}@{}", self.family, self.version)
    }

    pub(super) fn current_storage(self) -> bool {
        self.version.starts_with("26.")
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Integer(i32, i32),
    Text(usize),
    Choice(&'static [&'static str]),
}
use Kind::*;

const DIFFICULTY: &[&str] = &["peaceful", "easy", "normal", "hard"];
const GAMEMODE: &[&str] = &["survival", "creative", "adventure", "spectator"];
const WORLD_TYPES: &[&str] = &["Flat", "Normal"];

pub(super) struct Field {
    pub(super) section: &'static str,
    pub(super) key: &'static str,
    label: &'static str,
    kind: Kind,
    common: bool,
}

macro_rules! fields {
    ($( $section:literal { $( $key:literal => $label:literal, $kind:expr, $common:literal; )* } )*) => {
        pub(super) const FIELDS: &[Field] = &[
            $( $( Field { section: $section, key: $key, label: $label, kind: $kind, common: $common }, )* )*
        ];
    };
}

fields! {
    "serverProperties" {
        "allow-nether" => "啟用地獄維度", Bool, true;
        "generate-structures" => "生成結構", Bool, false;
        "spawn-protection" => "重生點保護半徑", Integer(0, 256), true;
        "max-players" => "玩家上限", Integer(1, 1000), true;
        "view-distance" => "視距", Integer(2, 32), true;
        "simulation-distance" => "模擬距離", Integer(2, 32), true;
        "motd" => "伺服器訊息", Text(120), true;
        "difficulty" => "難度", Choice(DIFFICULTY), true;
        "gamemode" => "遊戲模式", Choice(GAMEMODE), true;
        "hardcore" => "極限模式", Bool, false;
        "pvp" => "玩家互相傷害", Bool, false;
        "spawn-animals" => "生成動物", Bool, false;
        "spawn-monsters" => "生成怪物", Bool, false;
        "spawn-npcs" => "生成 NPC", Bool, false;
        "white-list" => "啟用玩家白名單", Bool, false;
        "enable-command-block" => "允許命令方塊", Bool, false;
        "player-idle-timeout" => "閒置踢出分鐘數", Integer(0, 100000), false;
        "max-world-size" => "世界大小上限", Integer(1, 29999984), false;
        "network-compression-threshold" => "封包壓縮門檻", Integer(-1, 65535), false;
        "use-native-transport" => "使用原生網路傳輸", Bool, false;
        "allow-flight" => "允許飛行", Bool, false;
        "force-gamemode" => "強制預設遊戲模式", Bool, false;
        "max-tick-time" => "單刻逾時（毫秒）", Integer(1000, 3600000), false;
        "sync-chunk-writes" => "同步寫入區塊", Bool, false;
        "entity-broadcast-range-percentage" => "實體廣播範圍（%）", Integer(10, 1000), false;
        "enable-status" => "提供伺服器狀態", Bool, false;
        "enforce-whitelist" => "強制白名單", Bool, false;
        "function-permission-level" => "函數權限等級", Integer(1, 4), false;
        "rate-limit" => "封包速率限制", Integer(0, 100000), false;
        "level-seed" => "世界種子", Text(128), false;
        "resource-pack" => "資源包網址", Text(2048), false;
        "resource-pack-sha1" => "資源包 SHA-1", Text(40), false;
        "require-resource-pack" => "要求使用資源包", Bool, false;
        "resource-pack-id" => "資源包 UUID", Text(36), false;
        "resource-pack-prompt" => "資源包提示", Text(256), false;
    }
    "spigot" {
        "view-distance" => "Spigot 視距覆寫", Integer(2, 32), false;
        "simulation-distance" => "Spigot 模擬距離覆寫", Integer(2, 32), false;
        "mob-spawn-range" => "生物生成範圍", Integer(1, 32), false;
        "item-despawn-rate" => "掉落物消失刻數", Integer(1, 1000000), false;
        "arrow-despawn-rate" => "箭矢消失刻數", Integer(1, 1000000), false;
        "trident-despawn-rate" => "三叉戟消失刻數", Integer(1, 1000000), false;
        "max-tnt-per-tick" => "每刻 TNT 上限", Integer(1, 100000), false;
    }
    "gamerules" {
        "advance_time" => "時間流動", Bool, true;
        "spawn_mobs" => "自然生成生物", Bool, true;
        "command_block_output" => "命令方塊輸出", Bool, true;
        "max_entity_cramming" => "實體擠壓上限", Integer(0, 9999), true;
        "allow_entering_nether_using_portals" => "允許經地獄傳送門進入", Bool, true;
    }
    "" {
        "worldType" => "世界類型", Choice(WORLD_TYPES), true;
        "initialTime" => "世界時間", Integer(0, 23999), true;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Binding {
    Properties,
    Yaml(&'static str, &'static str),
    Rule(&'static str),
    WorldType,
    Time,
}

impl Field {
    pub(super) fn id(&self) -> String {
        if self.section.is_empty() {
            self.key.to_owned()
        } else {
            format!("{}.{}", self.section, self.key)
        }
    }

    pub(super) fn binding(&self, profile: Profile) -> Binding {
        match (self.section, self.key, profile.current_storage()) {
            ("spigot", key, _) => Binding::Yaml("spigot.yml", key),
            ("serverProperties", "allow-nether", true) if profile.family == "SPIGOT" => {
                Binding::Yaml("bukkit.yml", "settings.allow-nether")
            }
            ("serverProperties", "allow-nether", true) => {
                Binding::Yaml("config/paper-global.yml", "misc.enable-nether")
            }
            ("serverProperties", "spawn-monsters", true) => {
                Binding::Rule("minecraft:spawn_monsters")
            }
            ("serverProperties", "enable-command-block", true) => {
                Binding::Rule("minecraft:command_blocks_work")
            }
            ("serverProperties", "pvp", true) => Binding::Rule("minecraft:pvp"),
            ("gamerules", "advance_time", false) => Binding::Rule("doDaylightCycle"),
            ("gamerules", "spawn_mobs", false) => Binding::Rule("doMobSpawning"),
            ("gamerules", "command_block_output", false) => Binding::Rule("commandBlockOutput"),
            ("gamerules", "max_entity_cramming", false) => Binding::Rule("maxEntityCramming"),
            ("gamerules", "advance_time", _) => Binding::Rule("minecraft:advance_time"),
            ("gamerules", "spawn_mobs", _) => Binding::Rule("minecraft:spawn_mobs"),
            ("gamerules", "command_block_output", _) => {
                Binding::Rule("minecraft:command_block_output")
            }
            ("gamerules", "max_entity_cramming", _) => {
                Binding::Rule("minecraft:max_entity_cramming")
            }
            ("gamerules", _, _) => Binding::Rule("minecraft:allow_entering_nether_using_portals"),
            ("", "worldType", _) => Binding::WorldType,
            ("", "initialTime", _) => Binding::Time,
            _ => Binding::Properties,
        }
    }

    pub(super) fn gate(&self, profile: Profile, existing: bool) -> (&'static str, &'static str) {
        let pre114 = matches!(profile.version, "1.8.8" | "1.12.2");
        let pre116 = pre114 || matches!(profile.version, "1.14" | "1.14.4");
        let pre120 = pre116 || profile.version == "1.16.5";
        if existing && matches!(self.key, "worldType" | "level-seed" | "generate-structures") {
            return ("unsupported", "此項只供新建世界設定。");
        }
        let unsupported = match self.key {
            "simulation-distance"
            | "require-resource-pack"
            | "resource-pack-prompt"
            | "rate-limit" => pre120,
            "trident-despawn-rate" | "enforce-whitelist" => pre114,
            "entity-broadcast-range-percentage" | "enable-status" | "sync-chunk-writes" => pre116,
            "max_entity_cramming" | "function-permission-level" => profile.version == "1.8.8",
            "allow_entering_nether_using_portals" => !profile.current_storage(),
            "spawn-animals" | "spawn-npcs" => {
                profile.current_storage() || profile.version == "1.21.8"
            }
            "resource-pack-id" => !profile.current_storage() && profile.version != "1.21.8",
            _ => false,
        };
        if unsupported {
            return ("unsupported", "此核心版本沒有對應的欄位或規則。");
        }
        if self.key == "sync-chunk-writes" && profile.family != "SPIGOT" {
            return ("unverified", "需要另外確認核心的 JVM 旗標。");
        }
        if (self.key.starts_with("resource-pack") || self.key == "require-resource-pack")
            && !profile.current_storage()
        {
            return ("unverified", "資源包網址與驗證資訊需要成組核對。");
        }
        match self.binding(profile) {
            Binding::Yaml(_, _) if self.key == "allow-nether" => ("available", ""),
            Binding::Yaml("spigot.yml", _)
                if matches!(profile.version, "1.14" | "1.14.4")
                    && matches!(self.key, "view-distance" | "trident-despawn-rate") =>
            {
                ("unverified", "未能從此版本的設定檔核對可寫的覆寫值。")
            }
            Binding::Yaml("spigot.yml", _)
                if profile.family != "SPIGOT"
                    && matches!(self.key, "view-distance" | "simulation-distance") =>
            {
                ("unverified", "需核對核心的有效距離後才能修改此覆寫值。")
            }
            Binding::Yaml("spigot.yml", _) => ("available", ""),
            Binding::Yaml(_, _) => ("unverified", "尚未接入 YAML 安全寫入與讀回。"),
            Binding::Rule(_) if !existing && world_live::first_ready::supports(profile.version, self.section, self.key) => ("available", ""),
            Binding::Rule(_) => ("unverified", "尚未接入已核對工作階段的遊戲規則讀回。"),
            Binding::Time if !existing => ("available", ""),
            Binding::Time => ("unverified", "時間需要獨立確認及工作階段讀回。"),
            _ => ("available", ""),
        }
    }

    pub(super) fn describe(&self, profile: Option<Profile>, existing: bool) -> Value {
        let binding = profile.map(|profile| self.binding(profile));
        let (status, reason) = profile.map_or(
            ("unverified", "無法核對核心與 Minecraft 版本。"),
            |profile| self.gate(profile, existing),
        );
        let source = match binding {
            Some(Binding::Properties) => format!("server.properties · {}", self.key),
            Some(Binding::Yaml("spigot.yml", key)) => {
                format!("spigot.yml · world-settings.default.{key}")
            }
            Some(Binding::Yaml(path, key)) => format!("{path} · {key}"),
            Some(Binding::Rule(key)) => format!("遊戲規則 · {key}"),
            Some(Binding::WorldType) => {
                "server.properties · level-type / generator-settings".into()
            }
            Some(Binding::Time) => "世界時間（獨立操作）".into(),
            None => String::new(),
        };
        let category = match binding {
            Some(Binding::Yaml("config/paper-global.yml", _)) => "Paper",
            Some(Binding::Yaml(_, _)) => "Spigot／Bukkit",
            Some(Binding::Rule(_)) => "遊戲規則",
            _ if self.section == "gamerules" => "遊戲規則",
            _ if self.section == "spigot" => "Spigot／Bukkit",
            _ if self.section.is_empty()
                || matches!(
                    self.key,
                    "level-seed" | "generate-structures" | "max-world-size" | "allow-nether"
                ) =>
            {
                "世界"
            }
            _ => "伺服器",
        };
        let (kind, min, max, choices) = match self.kind {
            Bool => ("boolean", None, None, &["true", "false"][..]),
            Integer(min, max) => ("integer", Some(min), Some(max), &[][..]),
            Text(max) => ("text", None, Some(max as i32), &[][..]),
            Choice(options) => ("choice", None, None, options),
        };
        let evidence = if status == "unsupported" {
            "unsupported"
        } else {
            "unverified"
        };
        json!({"key": self.id(), "label": self.label, "category": category, "source": source,
            "kind": kind, "min": min, "max": max, "choices": choices, "common": self.common,
            "status": status, "evidence": evidence, "reason": reason})
    }

    pub(super) fn normalize(&self, value: &str, profile: Profile) -> Result<String> {
        let invalid = || format!("InvalidPolicyValue: {}", self.id());
        if value.chars().any(char::is_control) {
            return Err(invalid());
        }
        match self.kind {
            Bool if matches!(value, "true" | "false") => Ok(value.into()),
            Integer(min, max) => value
                .parse::<i32>()
                .ok()
                .filter(|n| (min..=max).contains(n))
                .map(|n| n.to_string())
                .ok_or_else(invalid),
            Text(max) if value.encode_utf16().count() <= max => Ok(value.into()),
            Choice(options) if options.contains(&value) => Ok(value.into()),
            Choice(options) if profile.version == "1.8.8" || profile.version == "1.12.2" => value
                .parse::<usize>()
                .ok()
                .and_then(|index| options.get(index))
                .filter(|_| self.section == "serverProperties")
                .map(|value| (*value).into())
                .ok_or_else(invalid),
            _ => Err(invalid()),
        }
    }

    pub(super) fn encode(&self, value: &str, profile: Profile) -> String {
        if matches!(profile.version, "1.8.8" | "1.12.2")
            && matches!(self.key, "difficulty" | "gamemode")
        {
            let Choice(options) = self.kind else {
                unreachable!()
            };
            return options
                .iter()
                .position(|option| *option == value)
                .expect("validated choice")
                .to_string();
        }
        super::encode_property(value)
    }
}

#[cfg(test)]
mod yaml_version_tests {
    use super::*;

    #[test]
    fn real_core_yaml_matrix_keeps_unsupported_and_inherited_values_closed() {
        for (family, version, key, status) in [
            ("SPIGOT", "1.8.8", "item-despawn-rate", "available"),
            ("SPIGOT", "1.8.8", "view-distance", "available"),
            ("SPIGOT", "1.8.8", "simulation-distance", "unsupported"),
            ("SPIGOT", "1.8.8", "trident-despawn-rate", "unsupported"),
            ("PAPER", "26.2", "item-despawn-rate", "available"),
            ("PAPER", "26.3", "item-despawn-rate", "available"),
            ("FOLIA", "26.1.2", "item-despawn-rate", "available"),
            ("PAPER", "26.3", "view-distance", "unverified"),
            ("FOLIA", "26.1.2", "simulation-distance", "unverified"),
            ("SPIGOT", "1.12.2", "item-despawn-rate", "available"),
            ("SPIGOT", "1.14", "view-distance", "unverified"),
            ("SPIGOT", "1.14", "trident-despawn-rate", "unverified"),
            ("SPIGOT", "1.16.5", "item-despawn-rate", "available"),
            ("SPIGOT", "1.16.5", "trident-despawn-rate", "available"),
            ("PAPER", "1.14", "item-despawn-rate", "available"),
            ("PAPER", "1.14.4", "item-despawn-rate", "available"),
            ("PAPER", "1.14.4", "trident-despawn-rate", "unverified"),
            ("PAPER", "1.14.4", "view-distance", "unverified"),
            ("PAPER", "1.20.6", "item-despawn-rate", "available"),
            ("PAPER", "1.20.6", "simulation-distance", "unverified"),
            ("PAPER", "1.21.8", "item-despawn-rate", "available"),
            ("PAPER", "26.1.2", "item-despawn-rate", "available"),
        ] {
            let profile = Profile::resolve(family, version).unwrap();
            let field = FIELDS
                .iter()
                .find(|field| field.section == "spigot" && field.key == key)
                .unwrap();
            for existing in [false, true] {
                assert_eq!(field.gate(profile, existing).0, status, "{family} {version} {key}");
            }
        }
    }
}
