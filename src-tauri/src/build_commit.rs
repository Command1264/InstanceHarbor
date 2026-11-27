//! The Git commit label written into the executables at build time.
//!
//! `build.rs` includes this file with `#[path]`, so it must not use anything from
//! the crate. A label is the first seven hex digits of `HEAD`, with `-dirty` when
//! the working tree had uncommitted changes, or [`UNKNOWN`] when no Git checkout
//! or Git executable was available.

/// The label of a build made without Git, for example from a source archive.
pub const UNKNOWN: &str = "unknown";
/// The environment variable that `packaging/build-product.ps1` sets and `build.rs` reads.
pub const VARIABLE: &str = "IH_BUILD_COMMIT";
const DIRTY: &str = "-dirty";
const LENGTH: usize = 7;

/// The label for a full or abbreviated commit hash. Returns `None` when the hash
/// is shorter than seven characters or is not lowercase hexadecimal.
pub fn label(hash: &str, dirty: bool) -> Option<String> {
    let hash = hash.trim();
    let short = hash.get(..LENGTH)?;
    if !hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return None;
    }
    Some(if dirty { format!("{short}{DIRTY}") } else { short.to_owned() })
}

/// Whether `value` is a label this module produces.
pub fn is_label(value: &str) -> bool {
    if value == UNKNOWN {
        return true;
    }
    let short = value.strip_suffix(DIRTY).unwrap_or(value);
    short.len() == LENGTH && label(short, false).as_deref() == Some(short)
}
