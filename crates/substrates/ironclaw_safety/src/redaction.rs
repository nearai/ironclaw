/// Replaces exact secret values in `text` with `[REDACTED]`.
///
/// Values are applied longest-first so encoded variants such as
/// `token%20value` are redacted before shorter raw substrings like `token`.
pub fn redact_exact_values(mut text: String, values: &[String]) -> String {
    if values.is_empty() {
        return text;
    }
    let mut values = values
        .iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for value in values {
        text = text.replace(value, "[REDACTED]");
    }
    text
}

/// Builds exact redaction candidates for a secret value.
///
/// The returned set includes the raw value plus URL-component encodings with
/// `%20` and `+` spaces, including lowercase percent-escape variants, so
/// callers can redact echoed query/path credentials before runtime visibility.
pub fn redaction_values_for_secret(value: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }
    let mut values = Vec::new();
    push_redaction_value(&mut values, value.to_string());
    let encoded = percent_encode_url_component(value.as_bytes(), SpaceEncoding::Percent20);
    push_redaction_value(&mut values, encoded.clone());
    push_redaction_value(&mut values, lowercase_percent_escapes(&encoded));
    let plus_encoded = percent_encode_url_component(value.as_bytes(), SpaceEncoding::Plus);
    push_redaction_value(&mut values, plus_encoded.clone());
    push_redaction_value(&mut values, lowercase_percent_escapes(&plus_encoded));
    values
}

#[derive(Clone, Copy)]
enum SpaceEncoding {
    Percent20,
    Plus,
}

fn percent_encode_url_component(bytes: &[u8], space_encoding: SpaceEncoding) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(bytes.len());
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(byte as char);
        } else if byte == b' ' && matches!(space_encoding, SpaceEncoding::Plus) {
            output.push('+');
        } else {
            output.push('%');
            output.push(HEX[(byte >> 4) as usize] as char);
            output.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    output
}

fn push_redaction_value(values: &mut Vec<String>, value: String) {
    if !value.is_empty() && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn lowercase_percent_escapes(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            output.push('%');
            output.push((bytes[index + 1] as char).to_ascii_lowercase());
            output.push((bytes[index + 2] as char).to_ascii_lowercase());
            index += 3;
            continue;
        }
        output.push(bytes[index] as char);
        index += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{redact_exact_values, redaction_values_for_secret};

    #[test]
    fn redaction_values_include_raw_and_encoded_variants() {
        let values = redaction_values_for_secret("secret value");

        assert!(values.contains(&"secret value".to_string()));
        assert!(values.contains(&"secret%20value".to_string()));
        assert!(values.contains(&"secret+value".to_string()));
    }

    #[test]
    fn redaction_values_include_lowercase_percent_escape_variants() {
        let values = redaction_values_for_secret("\u{ff}");

        assert!(values.contains(&"%C3%BF".to_string()));
        assert!(values.contains(&"%c3%bf".to_string()));
    }

    #[test]
    fn redaction_values_deduplicate_empty_and_repeated_variants() {
        assert!(redaction_values_for_secret("").is_empty());

        let values = redaction_values_for_secret("plain");
        let unique = values
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert_eq!(values.len(), unique);
    }

    #[test]
    fn redact_exact_values_replaces_all_non_empty_values() {
        let redacted = redact_exact_values(
            "raw token and encoded token%20value".to_string(),
            &[
                "token".to_string(),
                "token%20value".to_string(),
                String::new(),
            ],
        );

        assert_eq!(redacted, "raw [REDACTED] and encoded [REDACTED]");
    }

    #[test]
    fn redact_exact_values_prefers_longest_match_first() {
        let redacted = redact_exact_values(
            "raw secret%20value and secret".to_string(),
            &["secret".to_string(), "secret%20value".to_string()],
        );

        assert_eq!(redacted, "raw [REDACTED] and [REDACTED]");
        assert!(!redacted.contains("secret%20value"));
        assert!(!redacted.contains("secret"));
    }
}
