use serde_json::{Map, Value};

const URL: &str = "serverProperties.resource-pack";
const SHA1: &str = "serverProperties.resource-pack-sha1";
const REQUIRED: &str = "serverProperties.require-resource-pack";
const ID: &str = "serverProperties.resource-pack-id";
const PROMPT: &str = "serverProperties.resource-pack-prompt";

fn value<'a>(values: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, &'static str> {
    values
        .get(key)
        .map(|value| value.as_str().ok_or("InvalidResourcePackValue"))
        .transpose()
}

fn valid_url(url: &str) -> bool {
    let authority = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|rest| rest.split(['/', '?', '#']).next());
    authority.is_some_and(|host| !host.is_empty() && !host.starts_with(':') && !host.contains('@'))
        && url.len() <= 2048
        && url.bytes().all(|byte| {
            byte.is_ascii_graphic() && !matches!(byte, b'\\' | b'<' | b'>' | b'"' | b'\'')
        })
}

fn valid_sha1(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// Validate the effective pack properties together after inherited and profile
/// settings have been combined. The caller remains responsible for version
/// support and for rejecting unsupported keys before persisting a profile.
pub(super) fn validate_group(values: &Map<String, Value>) -> Result<(), &'static str> {
    let url = value(values, URL)?.unwrap_or("");
    let sha1 = value(values, SHA1)?.unwrap_or("");
    let required = value(values, REQUIRED)?.unwrap_or("false");
    let id = value(values, ID)?.unwrap_or("");
    let prompt = value(values, PROMPT)?.unwrap_or("");

    if !matches!(required, "true" | "false") {
        return Err("InvalidResourcePackRequired");
    }
    if url.is_empty() {
        if !sha1.is_empty() || !id.is_empty() || !prompt.is_empty() || required == "true" {
            return Err("ResourcePackUrlRequired");
        }
        return Ok(());
    }
    if !valid_url(url) {
        return Err("InvalidResourcePackUrl");
    }
    if !valid_sha1(sha1) {
        return Err("ResourcePackSha1Required");
    }
    if !id.is_empty() && !valid_uuid(id) {
        return Err("InvalidResourcePackId");
    }
    if prompt.encode_utf16().count() > 256 || prompt.chars().any(char::is_control) {
        return Err("InvalidResourcePackPrompt");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn values(entries: Value) -> Map<String, Value> {
        entries.as_object().unwrap().clone()
    }

    #[test]
    fn accepts_an_absent_or_complete_pack() {
        assert!(validate_group(&Map::new()).is_ok());
        assert!(validate_group(&values(json!({
            "serverProperties.resource-pack": "https://example.org/packs/a.zip?build=3",
            "serverProperties.resource-pack-sha1": "1234567890abcdef1234567890abcdef12345678",
            "serverProperties.require-resource-pack": "true",
            "serverProperties.resource-pack-id": "12345678-1234-1234-1234-123456789abc",
            "serverProperties.resource-pack-prompt": "請先下載資源包"
        })))
        .is_ok());
    }

    #[test]
    fn rejects_incomplete_pack_and_credentials_in_url() {
        assert_eq!(
            validate_group(&values(
                json!({"serverProperties.resource-pack": "https://example.org/p.zip"})
            )),
            Err("ResourcePackSha1Required")
        );
        assert_eq!(
            validate_group(&values(
                json!({"serverProperties.require-resource-pack": "true"})
            )),
            Err("ResourcePackUrlRequired")
        );
        assert_eq!(
            validate_group(&values(
                json!({"serverProperties.resource-pack": "https://user@example.org/p.zip", "serverProperties.resource-pack-sha1": "1234567890abcdef1234567890abcdef12345678"})
            )),
            Err("InvalidResourcePackUrl")
        );
        assert_eq!(
            validate_group(&values(
                json!({"serverProperties.resource-pack": "file:///c:/private.zip", "serverProperties.resource-pack-sha1": "1234567890abcdef1234567890abcdef12345678"})
            )),
            Err("InvalidResourcePackUrl")
        );
    }

    #[test]
    fn rejects_invalid_hash_id_prompt_and_required_value() {
        let complete = values(
            json!({"serverProperties.resource-pack": "https://example.org/p.zip", "serverProperties.resource-pack-sha1": "1234567890abcdef1234567890abcdef12345678"}),
        );
        for (key, invalid, expected) in [
            (SHA1, "abcd", "ResourcePackSha1Required"),
            (ID, "1234", "InvalidResourcePackId"),
            (PROMPT, "first\nsecond", "InvalidResourcePackPrompt"),
            (REQUIRED, "yes", "InvalidResourcePackRequired"),
        ] {
            let mut case = complete.clone();
            case.insert(key.into(), json!(invalid));
            assert_eq!(validate_group(&case), Err(expected));
        }
    }
}
