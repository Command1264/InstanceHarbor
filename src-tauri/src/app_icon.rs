//! The application icon each channel embeds at build time.
//!
//! `build.rs` includes this file with `#[path]`, so it must not use anything from
//! the crate. The icon becomes Windows resource 32512 of every executable, which
//! the Explorer, the taskbar, Alt+Tab, the window title bar and the Manager tray
//! icon all read. The product channel keeps the blue icon; the development channel
//! uses the red copy that `generate-development-icons.py` derives from it.

/// The blue icon of the product channel, relative to the crate folder.
pub const PRODUCT: &str = "icons/icon.ico";
/// The red icon of the development channel, relative to the crate folder.
pub const DEVELOPMENT: &str = "icons/icon-dev.ico";
/// The compile-time variable through which `build.rs` reports the embedded icon.
pub const VARIABLE: &str = "IH_APP_ICON";

/// The icon a build embeds, by whether the `development-channel` feature is on.
pub const fn for_channel(development: bool) -> &'static str {
    if development {
        DEVELOPMENT
    } else {
        PRODUCT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Width and height of each image in an ICO directory.
    fn sizes(path: &str) -> Vec<(u8, u8)> {
        let data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        assert_eq!(&data[..4], &[0, 0, 1, 0], "{path} is not an ICO file");
        let count = u16::from_le_bytes([data[4], data[5]]) as usize;
        (0..count).map(|index| (data[6 + 16 * index], data[7 + 16 * index])).collect()
    }

    #[test]
    fn build_embeds_the_icon_of_its_channel() {
        assert_eq!(env!("IH_APP_ICON"), for_channel(cfg!(feature = "development-channel")));
    }

    #[cfg(not(feature = "development-channel"))]
    #[test]
    fn product_build_embeds_the_blue_icon() {
        assert_eq!(env!("IH_APP_ICON"), "icons/icon.ico");
    }

    #[cfg(feature = "development-channel")]
    #[test]
    fn development_build_embeds_the_red_icon() {
        assert_eq!(env!("IH_APP_ICON"), "icons/icon-dev.ico");
    }

    #[test]
    fn channel_icons_differ_but_share_every_size() {
        let product = sizes(PRODUCT);
        assert!(product.len() > 1, "the product icon should hold several sizes");
        assert_eq!(product, sizes(DEVELOPMENT));
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_ne!(
            std::fs::read(root.join(PRODUCT)).unwrap(),
            std::fs::read(root.join(DEVELOPMENT)).unwrap(),
            "the development icon must not be a copy of the product icon"
        );
    }
}
