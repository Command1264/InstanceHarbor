use super::Property;

fn plain_entry(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once(':')?;
    let key = key.trim_end_matches(' ');
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || (!rest.is_empty() && !rest.starts_with(' '))
    {
        return None;
    }
    Some((key, rest.trim_start_matches(' ')))
}

fn comment_or_empty(rest: &str) -> bool {
    let trimmed = rest.trim_start_matches(' ');
    trimmed.is_empty() || (trimmed.len() != rest.len() && trimmed.starts_with('#'))
}

fn simple_value(value: &str) -> bool {
    let Some(first) = value.as_bytes().first().copied() else {
        return true;
    };
    match first {
        b'\'' | b'"' => {
            let mut bytes = value.bytes().enumerate().skip(1).peekable();
            while let Some((index, byte)) = bytes.next() {
                if first == b'"' && byte == b'\\' {
                    bytes.next();
                } else if byte == first {
                    if first == b'\'' && bytes.next_if(|(_, next)| *next == b'\'').is_some() {
                        continue;
                    }
                    return comment_or_empty(&value[index + 1..]);
                }
            }
            false
        }
        b'[' | b'{' => matches!(value.get(..2), Some("[]" | "{}")) && comment_or_empty(&value[2..]),
        b'&' | b'*' | b'!' | b'|' | b'>' => false,
        _ => true,
    }
}

/// Finds a plain two-level boolean in the layouts emitted by the supported cores.
/// Reject ambiguous mapping structures instead of rewriting or treating this as a YAML parser.
pub(super) fn boolean(text: &str, key: &str) -> Option<Property> {
    let (section, field) = key.split_once('.')?;
    if text.contains(['\u{0085}', '\u{2028}', '\u{2029}'])
        || text
            .bytes()
            .any(|byte| byte.is_ascii_control() && !matches!(byte, b'\n' | b'\r'))
    {
        return None;
    }
    let mut offset = 0;
    let mut section_seen = false;
    let mut active = false;
    let mut can_nest = false;
    let mut found = None;
    for physical in text.split_inclusive('\n') {
        let start = offset;
        offset += physical.len();
        let raw = physical
            .strip_suffix('\n')
            .map_or(physical, |line| line.strip_suffix('\r').unwrap_or(line));
        if raw.contains('\r') {
            return None;
        }
        let line = raw.trim_start_matches(' ');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match raw.len() - line.len() {
            0 => {
                let (name, value) = plain_entry(line)?;
                active = name == section;
                can_nest = false;
                if active {
                    if section_seen || (!value.is_empty() && !value.starts_with('#')) {
                        return None;
                    }
                    section_seen = true;
                } else if !simple_value(value) {
                    return None;
                }
            }
            1 if active => return None,
            2 if active => {
                let (name, value) = plain_entry(line)?;
                can_nest = value.is_empty() || value.starts_with('#');
                if name != field {
                    if !simple_value(value) {
                        return None;
                    }
                    continue;
                }
                if found.is_some() {
                    return None;
                }
                let token = value.split(' ').next()?;
                let canonical = match token {
                    "true" | "True" | "TRUE" => "true",
                    "false" | "False" | "FALSE" => "false",
                    _ => return None,
                };
                if !comment_or_empty(&value[token.len()..]) {
                    return None;
                }
                let start = start + raw.len() - value.len();
                found = Some(Property {
                    value: canonical.into(),
                    start,
                    end: start + token.len(),
                    editable: true,
                });
            }
            _ if active && !can_nest => return None,
            _ => {}
        }
    }
    found
}

/// Extracts only a plain integer from Spigot's default world map.
pub(super) fn spigot_integer(text: &str, field: &str) -> Option<Property> {
    if text.contains(['\u{0085}', '\u{2028}', '\u{2029}'])
        || text
            .bytes()
            .any(|byte| byte.is_ascii_control() && !matches!(byte, b'\n' | b'\r'))
    {
        return None;
    }
    let mut offset = 0;
    let mut in_worlds = false;
    let mut worlds_seen = false;
    let mut default_seen = false;
    let mut can_nest = false;
    let mut found = None;
    for physical in text.split_inclusive('\n') {
        let start = offset;
        offset += physical.len();
        let raw = physical.strip_suffix('\n').unwrap_or(physical);
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if raw.contains('\r') {
            return None;
        }
        let line = raw.trim_start_matches(' ');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match raw.len() - line.len() {
            0 => {
                let (name, value) = plain_entry(line)?;
                in_worlds = name == "world-settings";
                if in_worlds {
                    if worlds_seen || !(value.is_empty() || value.starts_with('#')) {
                        return None;
                    }
                    worlds_seen = true;
                } else if !simple_value(value) {
                    return None;
                }
            }
            _ if !in_worlds => {}
            2 => {
                let (name, value) = plain_entry(line)?;
                // A named group may change the effective value for one world.
                if default_seen
                    || name != "default"
                    || !(value.is_empty() || value.starts_with('#'))
                {
                    return None;
                }
                default_seen = true;
            }
            4 if default_seen => {
                let (name, value) = plain_entry(line)?;
                if !simple_value(value) {
                    return None;
                }
                can_nest = value.is_empty() || value.starts_with('#');
                if name == field {
                    let token = value.split(' ').next()?;
                    let parsed = token.parse::<i32>().ok()?;
                    if found.is_some()
                        || parsed.to_string() != token
                        || !comment_or_empty(&value[token.len()..])
                    {
                        return None;
                    }
                    let start = start + raw.len() - value.len();
                    found = Some(Property {
                        value: token.into(),
                        start,
                        end: start + token.len(),
                        editable: true,
                    });
                }
            }
            indentation if indentation > 4 && can_nest => {}
            _ => return None,
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spigot_integer_requires_a_unique_default_without_world_overrides() {
        let source = "world-settings:\r\n  default:\r\n    item-despawn-rate: 6000  # preserve\r\n    nested:\r\n      item-despawn-rate: 10\r\n";
        let value = spigot_integer(source, "item-despawn-rate").unwrap();
        assert_eq!(&source[value.start..value.end], "6000");
        for suffix in [
            "    item-despawn-rate: 7000\r\n",
            "  world:\r\n    item-despawn-rate: 7000\r\n",
            "    <<: *defaults\r\n",
            "'world-settings': {}\r\n",
        ] {
            assert!(spigot_integer(&format!("{source}{suffix}"), "item-despawn-rate").is_none());
        }
        assert!(spigot_integer(&source.replace("6000", "06000"), "item-despawn-rate").is_none());
    }

    #[test]
    fn replaces_only_the_selected_scalar_preserving_other_bytes() {
        let original = "# 使用者設定\r\nsettings: # group\r\n  allow-end: true\r\n  allow-nether:  TRUE  # Keep\r\nworld:\r\n  allow-nether: true";
        let result = boolean(original, "settings.allow-nether").unwrap();
        assert_eq!(result.value, "true");
        assert!(result.editable);
        assert_eq!(&original[result.start..result.end], "TRUE");
        let mut updated = original.to_owned();
        updated.replace_range(result.start..result.end, "false");
        assert_eq!(updated, original.replace("TRUE  #", "false  #"));
        assert_eq!(
            boolean(&updated, "settings.allow-nether").unwrap().value,
            "false"
        );
    }

    #[test]
    fn ignores_nested_keys_and_handles_single_line_siblings() {
        let text = "misc:\n  sibling: 'a''b' # comment\n  quoted: \"a\\\"b\"\n  empty: []\n  nested:\n    enable-nether: false\n  enable-nether: true\n";
        assert_eq!(boolean(text, "misc.enable-nether").unwrap().value, "true");
    }

    #[test]
    fn refuses_ambiguous_or_complex_values() {
        let good = "misc:\n  enable-nether: true\n";
        for text in [
            format!("{good}  enable-nether: false\n"),
            format!("{good}\"misc\":\n  enable-nether: false\n"),
            format!("{good}  'enable-nether': false\n"),
            format!("{good}{good}"),
            format!("{good}  <<: *defaults\n"),
            format!("{good}\tother: false\n"),
            format!("{good}    continuation\n"),
            format!("{good}---\n"),
            "misc: {enable-nether: true}".into(),
            "misc: &m\n  enable-nether: true".into(),
            "misc:\n  enable-nether: true#not-a-comment".into(),
            "misc:\n  enable-nether: 'true'".into(),
            "misc:\n  enable-nether: false extra".into(),
            "misc:\n  enable-nether: false\u{0085}other: false".into(),
            "misc:\n  other: 'first line\n  enable-nether: false\n  last line'".into(),
            "misc:\n    other: false\n  enable-nether: true".into(),
            "misc:\r\r\n  enable-nether: true".into(),
        ] {
            assert!(boolean(&text, "misc.enable-nether").is_none(), "{text}");
        }
    }
}
