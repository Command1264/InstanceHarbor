//! Windows path representations at the JSON boundary. Operational paths stay
//! verbatim so long paths keep their Windows semantics.
use std::path::{Path, PathBuf};

fn physical(path: &str) -> Option<String> {
    let ordinary = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        if !is_drive_absolute(rest) {
            return None; // Device namespaces are not physical drive paths.
        }
        rest.to_owned()
    } else if path.starts_with(r"\\.\") {
        return None;
    } else {
        path.to_owned()
    };
    Some(ordinary.replace('\u{5c}', "/"))
}

fn is_drive_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

/// Fixed, forward-slash transport representation for user-facing paths.
/// The UI may render a native separator preference without changing this value.
pub(super) fn display(path: &Path) -> String {
    let raw = path.to_string_lossy();
    physical(&raw).unwrap_or_else(|| raw.into_owned())
}

/// The registry stores portable drive/UNC spelling. On read, restore the
/// verbatim Windows namespace for normalized paths so long-path Java launch
/// behavior does not depend on the process-wide Win32 long-path setting.
pub(super) mod stored_java {
    use super::*;
    use serde::{ser::Error, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(path: &PathBuf, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let raw = path
            .to_str()
            .ok_or_else(|| S::Error::custom("Java path is not valid Unicode"))?;
        serializer.serialize_str(&physical(raw).unwrap_or_else(|| raw.to_owned()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        // Old registry values with backslashes, including verbatim paths,
        // remain accepted as-is. New forward paths regain verbatim semantics.
        if is_drive_absolute(&value) && value.as_bytes()[2] == b'/' {
            return Ok(PathBuf::from(format!(r"\\?\{}", value.replace('/', r"\"))));
        }
        if let Some(rest) = value.strip_prefix("//") {
            if !rest.starts_with('?') && !rest.starts_with('.') {
                return Ok(PathBuf::from(format!(
                    r"\\?\UNC\{}",
                    rest.replace('/', r"\")
                )));
            }
        }
        Ok(PathBuf::from(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct JavaPath {
        #[serde(with = "stored_java")]
        java: PathBuf,
    }

    #[test]
    fn display_normalizes_drive_and_unc_paths() {
        assert_eq!(
            display(Path::new(r"\\?\C:\Tools\jdk\bin\java.exe")),
            "C:/Tools/jdk/bin/java.exe"
        );
        assert_eq!(
            display(Path::new(r"C:\Tools\jdk\bin\java.exe")),
            "C:/Tools/jdk/bin/java.exe"
        );
        assert_eq!(
            display(Path::new(r"\\?\UNC\host\share\jdk\java.exe")),
            "//host/share/jdk/java.exe"
        );
        assert_eq!(
            display(Path::new(r"\\host\share\jdk\java.exe")),
            "//host/share/jdk/java.exe"
        );
    }

    #[test]
    fn device_namespace_is_not_reinterpreted() {
        for value in [
            r"\\?\GLOBALROOT\Device\HarddiskVolume1",
            r"\\.\PhysicalDrive0",
        ] {
            assert_eq!(display(Path::new(value)), value);
        }
    }

    #[test]
    fn stored_java_is_forward_and_restores_verbatim_runtime_path() {
        for (runtime, stored) in [
            (
                r"\\?\C:\Tools\jdk\bin\java.exe",
                "C:/Tools/jdk/bin/java.exe",
            ),
            (
                r"\\?\UNC\host\share\jdk\java.exe",
                "//host/share/jdk/java.exe",
            ),
        ] {
            let encoded = serde_json::to_string(&JavaPath {
                java: PathBuf::from(runtime),
            })
            .unwrap();
            assert_eq!(encoded, serde_json::json!({"java":stored}).to_string());
            let decoded: JavaPath = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded.java, PathBuf::from(runtime));
        }
        let old: JavaPath =
            serde_json::from_str(r#"{"java":"\\\\?\\C:\\Tools\\java.exe"}"#).unwrap();
        assert_eq!(old.java, PathBuf::from(r"\\?\C:\Tools\java.exe"));

        let long = format!(r"\\?\C:\{}\java.exe", "nested".repeat(50));
        let encoded = serde_json::to_string(&JavaPath {
            java: PathBuf::from(&long),
        })
        .unwrap();
        assert!(!encoded.contains(r"\\\\?\\"));
        let decoded: JavaPath = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.java, PathBuf::from(long));
    }

    #[test]
    fn json_escaping_preserves_special_characters() {
        let input = JavaPath {
            java: PathBuf::from("C:\\Program Files\\A\"B\\java.exe"),
        };
        let encoded = serde_json::to_string(&input).unwrap();
        assert_eq!(encoded, r#"{"java":"C:/Program Files/A\"B/java.exe"}"#);
    }
}
