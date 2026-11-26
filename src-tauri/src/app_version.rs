//! InstanceHarbor version numbers as GitHub Release tags carry them (#65).
//!
//! `1.1.0`, `v1.1.0`, `1.1.0-rc.1` and the older `1.0.0-rc2` all parse. The order
//! follows SemVer, except that a prerelease identifier mixing letters and digits
//! compares its digit runs as numbers, so `rc2` < `rc10`.
use std::cmp::Ordering;

const MAX_LENGTH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    pre: Vec<String>,
}

impl Version {
    /// Parses `MAJOR.MINOR.PATCH[-PRERELEASE]`, with an optional leading `v`.
    /// Build metadata (`+...`) is not accepted: release tags never carry it.
    pub fn parse(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > MAX_LENGTH {
            return None;
        }
        let text = text.strip_prefix('v').unwrap_or(text);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut parts = core.split('.');
        let mut numbers = [0u64; 3];
        for number in &mut numbers {
            let part = parts.next()?;
            if part.is_empty() || part.len() > 9 || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            *number = part.parse().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => vec![],
            Some(pre) => {
                let identifiers: Vec<String> = pre.split('.').map(str::to_owned).collect();
                if identifiers.iter().any(|identifier| {
                    identifier.is_empty()
                        || !identifier.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                }) {
                    return None;
                }
                identifiers
            }
        };
        Some(Self { core: numbers, pre })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [major, minor, patch] = self.core;
        write!(formatter, "{major}.{minor}.{patch}")?;
        if !self.pre.is_empty() {
            write!(formatter, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

/// Compares runs of digits as numbers and other runs as text; digits sort first.
fn natural(left: &str, right: &str) -> Ordering {
    fn runs(text: &str) -> Vec<&str> {
        let mut runs = vec![];
        let mut start = 0;
        let bytes = text.as_bytes();
        for index in 1..=bytes.len() {
            if index == bytes.len() || bytes[index].is_ascii_digit() != bytes[start].is_ascii_digit() {
                runs.push(&text[start..index]);
                start = index;
            }
        }
        runs
    }
    let (left, right) = (runs(left), runs(right));
    for (a, b) in left.iter().zip(&right) {
        let order = match (a.as_bytes()[0].is_ascii_digit(), b.as_bytes()[0].is_ascii_digit()) {
            (true, true) => {
                let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
                a.len().cmp(&b.len()).then_with(|| a.cmp(b))
            }
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => a.cmp(b),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            // A release is newer than any of its prereleases.
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let order = natural(a, b);
                    if order != Ordering::Equal {
                        return order;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The GitHub page of one release, or None when `tag` is not a version tag.
/// The page sends only the tag; the address is built here.
pub fn release_page(tag: &str) -> Option<String> {
    Version::parse(tag)?;
    Some(format!("{}/releases/tag/{tag}", crate::about::REPOSITORY))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap_or_else(|| panic!("{text}"))
    }

    #[test]
    fn parses_release_tags_and_prereleases() {
        assert_eq!(v("v1.1.0").to_string(), "1.1.0");
        assert_eq!(v("1.1.0-rc.1").to_string(), "1.1.0-rc.1");
        assert_eq!(v("v1.0.0-rc2").to_string(), "1.0.0-rc2");
        assert!(v("1.0.0-rc2").is_prerelease());
        assert!(!v("1.0.0").is_prerelease());
        for bad in ["", "1", "1.1", "1.1.0.0", "1.x.0", "1.1.0-", "1.1.0-rc..1", "1.1.0+build", "1.1.0-rc/1",
            " 1.1.0", "vv1.1.0", "1.1.0-rc_1", "1234567890.0.0", &format!("1.1.0-{}", "a".repeat(64))] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn orders_versions_like_semver_with_natural_prerelease_numbers() {
        let ordered = ["0.9.9", "1.0.0-rc1", "1.0.0-rc2", "1.0.0-rc10", "1.0.0", "1.0.1",
            "1.1.0-alpha", "1.1.0-alpha.1", "1.1.0-beta", "1.1.0-rc.1", "1.1.0-rc.2", "1.1.0-rc.10", "1.1.0", "1.10.0", "2.0.0"];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(v("v1.1.0").cmp(&v("1.1.0")), Ordering::Equal);
        assert_eq!(v("1.1.0-rc.01").cmp(&v("1.1.0-rc.1")), Ordering::Equal);
    }

    #[test]
    fn release_pages_stay_on_the_public_repository() {
        assert_eq!(release_page("v1.2.0").as_deref(), Some("https://github.com/Command1264/InstanceHarbor/releases/tag/v1.2.0"));
        assert_eq!(release_page("v1.2.0-rc.1").as_deref(), Some("https://github.com/Command1264/InstanceHarbor/releases/tag/v1.2.0-rc.1"));
        for bad in ["", "latest", "../v1.2.0", "v1.2.0/../../x", "https://example.com", "v1.2.0?x=1", "v1.2.0#x"] {
            assert_eq!(release_page(bad), None, "{bad}");
        }
    }
}
