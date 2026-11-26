//! What the About section and `ihctl --version` report about this build.

pub use crate::build_commit::{is_label as is_commit_label, UNKNOWN as UNKNOWN_COMMIT};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Written by `build.rs`; see [`crate::build_commit`].
pub const COMMIT: &str = env!("IH_BUILD_COMMIT");
pub const AUTHOR: &str = "Command1264";
pub const REPOSITORY: &str = "https://github.com/Command1264/InstanceHarbor";
pub const ISSUES: &str = "https://github.com/Command1264/InstanceHarbor/issues";
/// SpigotMC's official BuildTools guide. Spigot has no official download, so
/// "Core maintenance" points here instead of offering an update (#75).
pub const BUILDTOOLS: &str = "https://www.spigotmc.org/wiki/buildtools/";
/// The notice files that the ZIP and Setup place beside the executables.
pub const LICENSE_FILE: &str = "LICENSE";
pub const NOTICES_FILE: &str = "THIRD-PARTY-NOTICES.txt";

/// The first line of `ihctl --version`, for example `InstanceHarbor CLI 1.0.0 (1a2b3c4)`.
pub fn cli_version(title: &str) -> String {
    format!("{title} CLI {VERSION} ({COMMIT})")
}

/// A fixed place the GUI may open: the About section's links and the BuildTools
/// guide in "Core maintenance". The GUI accepts only these names, never a URL or
/// path from the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Repository,
    Issues,
    BuildTools,
    License,
    Notices,
}

impl Link {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "repository" => Some(Self::Repository),
            "issues" => Some(Self::Issues),
            "buildtools" => Some(Self::BuildTools),
            "license" => Some(Self::License),
            "notices" => Some(Self::Notices),
            _ => None,
        }
    }

    /// The web address for links that open in the browser.
    pub fn url(self) -> Option<&'static str> {
        match self {
            Self::Repository => Some(REPOSITORY),
            Self::Issues => Some(ISSUES),
            Self::BuildTools => Some(BUILDTOOLS),
            Self::License | Self::Notices => None,
        }
    }

    /// The notice file name, beside the executable, for links that open a file.
    pub fn file(self) -> Option<&'static str> {
        match self {
            Self::License => Some(LICENSE_FILE),
            Self::Notices => Some(NOTICES_FILE),
            Self::Repository | Self::Issues | Self::BuildTools => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_commit::label;

    #[test]
    fn commit_label_is_the_first_seven_hex_digits() {
        let hash = "33dfd35b0c1e2f3a4b5c6d7e8f90123456789abc";
        assert_eq!(label(hash, false).as_deref(), Some("33dfd35"));
        assert_eq!(label(&format!("{hash}\n"), true).as_deref(), Some("33dfd35-dirty"));
        assert_eq!(label("33dfd35", false).as_deref(), Some("33dfd35"));
    }

    #[test]
    fn commit_label_rejects_what_is_not_a_hash() {
        for value in ["", "33dfd3", "33DFD35", "33dfd3g", "fatal: not a git repository"] {
            assert_eq!(label(value, false), None, "{value}");
        }
    }

    #[test]
    fn accepted_labels_match_what_build_commit_produces() {
        for value in ["33dfd35", "33dfd35-dirty", UNKNOWN_COMMIT] {
            assert!(is_commit_label(value), "{value}");
        }
        for value in ["", "33dfd3", "33dfd35b", "33dfd35-", "33dfd35-dirty-dirty", "-dirty", "unknown-dirty", "33DFD35"] {
            assert!(!is_commit_label(value), "{value}");
        }
    }

    #[test]
    fn this_build_has_a_valid_commit_label() {
        assert!(is_commit_label(COMMIT), "{COMMIT}");
    }

    #[test]
    fn cli_version_keeps_the_prefix_scripts_match() {
        let stable = cli_version("InstanceHarbor");
        assert!(stable.starts_with(&format!("InstanceHarbor CLI {VERSION} (")), "{stable}");
        assert!(stable.ends_with(&format!("({COMMIT})")), "{stable}");
        // launch-development.ps1 checks `-like 'InstanceHarbor Dev CLI *'`.
        assert!(cli_version("InstanceHarbor Dev").starts_with("InstanceHarbor Dev CLI "));
    }

    #[test]
    fn links_are_fixed_to_the_public_repository() {
        assert_eq!(Link::parse("repository").and_then(Link::url), Some("https://github.com/Command1264/InstanceHarbor"));
        assert_eq!(Link::parse("issues").and_then(Link::url), Some("https://github.com/Command1264/InstanceHarbor/issues"));
        assert_eq!(Link::parse("buildtools").and_then(Link::url), Some("https://www.spigotmc.org/wiki/buildtools/"));
        assert_eq!(Link::parse("license").and_then(Link::file), Some("LICENSE"));
        assert_eq!(Link::parse("notices").and_then(Link::file), Some("THIRD-PARTY-NOTICES.txt"));
        for name in ["", "https://example.com", "Repository", "../LICENSE", "file:///C:/Windows"] {
            assert_eq!(Link::parse(name), None, "{name}");
        }
        for link in [Link::Repository, Link::Issues, Link::BuildTools, Link::License, Link::Notices] {
            assert!(link.url().is_some() != link.file().is_some());
            if let Some(url) = link.url() {
                assert!(url.starts_with("https://github.com/Command1264/InstanceHarbor")
                    || url == "https://www.spigotmc.org/wiki/buildtools/");
            }
        }
    }
}
