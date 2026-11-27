//! The icons of each channel: the one place the Rust code learns which files they are.
//!
//! `src-tauri/icons/channels.json` pairs each channel with its files. Vite, the build
//! checks and the generator of the red files read the same manifest (#116). The
//! product channel keeps the blue icon; the development channel uses the red copy that
//! `packaging/generate-development-icons.py` derives from it.
//!
//! `build.rs` includes this file with `#[path]`, so it must not use anything from the
//! crate. With the channel's ICO, `build.rs`
//! - embeds it as the application icon resource of every executable, which the Explorer,
//!   shortcuts and the Manager tray icon read ([`resource_id`]), and
//! - names it as Tauri's `bundle.icon`, from which `generate_context!` takes the default
//!   window icon that the title bar, the taskbar and Alt+Tab show.

/// The manifest, relative to the crate folder.
pub const MANIFEST: &str = "icons/channels.json";
/// The compile-time variable through which `build.rs` reports the embedded icon.
pub const VARIABLE: &str = "IH_APP_ICON";
const MANIFEST_TEXT: &str = include_str!("../icons/channels.json");

/// The icon files of one channel, relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelIcons {
    /// The ICO of the executables and windows.
    pub app: String,
    /// The sidebar brand image (chosen by `vite.config.ts`).
    pub brand: String,
}

impl ChannelIcons {
    /// The ICO relative to the crate folder, as `build.rs` and Tauri's config resolve it.
    pub fn app_in_crate(&self) -> String {
        format!("../{}", self.app)
    }
}

fn manifest() -> serde_json::Value {
    serde_json::from_str(MANIFEST_TEXT).expect("icons/channels.json is valid JSON")
}

/// The manifest key of a channel, by whether the `development-channel` feature is on.
pub const fn channel(development: bool) -> &'static str {
    if development {
        "development"
    } else {
        "product"
    }
}

/// The icons a build uses, by whether the `development-channel` feature is on.
pub fn for_channel(development: bool) -> ChannelIcons {
    let manifest = manifest();
    let entry = &manifest[channel(development)];
    let path = |kind: &str| {
        entry[kind]
            .as_str()
            .unwrap_or_else(|| panic!("icons/channels.json names the {kind} icon of {}", channel(development)))
            .to_owned()
    };
    ChannelIcons { app: path("app"), brand: path("brand") }
}

/// The resource number under which tauri-build embeds the application icon.
pub fn resource_id() -> u16 {
    manifest()["resource"]
        .as_u64()
        .and_then(|id| u16::try_from(id).ok())
        .expect("icons/channels.json names the icon resource")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn read(path: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
    }

    /// Width and height of each image in an ICO directory.
    fn sizes(path: &str) -> Vec<(u8, u8)> {
        let data = read(path);
        assert_eq!(&data[..4], &[0, 0, 1, 0], "{path} is not an ICO file");
        let count = u16::from_le_bytes([data[4], data[5]]) as usize;
        (0..count).map(|index| (data[6 + 16 * index], data[7 + 16 * index])).collect()
    }

    #[test]
    fn build_embeds_the_icon_of_its_channel() {
        let development = cfg!(feature = "development-channel");
        assert_eq!(env!("IH_APP_ICON"), for_channel(development).app_in_crate());
    }

    #[test]
    fn tauri_config_names_the_icon_of_its_channel() {
        // generate_context! reads the default window icon from bundle.icon (#116).
        let config: serde_json::Value = serde_json::from_str(env!("TAURI_CONFIG")).expect("build.rs sets TAURI_CONFIG");
        let icon = for_channel(cfg!(feature = "development-channel")).app_in_crate();
        assert_eq!(config["bundle"]["icon"], serde_json::json!([icon]));
    }

    #[test]
    fn channel_icons_differ_but_share_every_size() {
        let (product, development) = (for_channel(false), for_channel(true));
        let (product, development) = (product.app_in_crate(), development.app_in_crate());
        let sizes_of_product = sizes(&product);
        assert!(sizes_of_product.len() > 1, "the product icon should hold several sizes");
        assert_eq!(sizes_of_product, sizes(&development));
        assert_ne!(read(&product), read(&development), "the development icon must not be a copy of the product icon");
    }

    #[test]
    fn the_manifest_names_the_icon_resource() {
        // src/channel-icons.test.mjs checks the number that tauri-build uses.
        assert!(resource_id() > 0);
    }
}
