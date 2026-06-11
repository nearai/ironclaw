//! Inline-secret scanning over the raw TOML tree.
//!
//! Same rule and detection as the blueprint parser: every string is checked
//! via the shared [`ironclaw_reborn_config::reject_inline_secret`] guard, and a
//! value that is exactly a `${secret:<name>}` handle is allowed (with its name
//! validated). The tree walk is small glue around that shared guard.

use ironclaw_reborn_config::reject_inline_secret;

use crate::error::HarnessError;

const HANDLE_PREFIX: &str = "${secret:";
const HANDLE_SUFFIX: &str = "}";

pub(crate) fn scan(root: &toml::Value) -> Result<(), HarnessError> {
    walk(root, &mut String::new())
}

fn walk(value: &toml::Value, path: &mut String) -> Result<(), HarnessError> {
    match value {
        toml::Value::String(text) => check_string(text, path),
        toml::Value::Table(table) => {
            for (key, child) in table {
                let len = path.len();
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(key);
                walk(child, path)?;
                path.truncate(len);
            }
            Ok(())
        }
        toml::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let len = path.len();
                path.push('[');
                path.push_str(&index.to_string());
                path.push(']');
                walk(child, path)?;
                path.truncate(len);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn check_string(text: &str, path: &str) -> Result<(), HarnessError> {
    if let Some(handle) = parse_secret_handle(text) {
        return validate_handle_name(handle, path);
    }
    reject_inline_secret(path.to_string(), text).map_err(|source| HarnessError::InlineSecret {
        path: path.to_string(),
        source,
    })
}

fn parse_secret_handle(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    trimmed
        .strip_prefix(HANDLE_PREFIX)?
        .strip_suffix(HANDLE_SUFFIX)
}

fn validate_handle_name(name: &str, path: &str) -> Result<(), HarnessError> {
    let invalid = |reason: &str| HarnessError::InvalidSecretHandle {
        path: path.to_string(),
        handle: name.to_string(),
        reason: reason.to_string(),
    };
    if name.is_empty() {
        return Err(invalid("empty name"));
    }
    if name.len() > 128 {
        return Err(invalid("longer than 128 bytes"));
    }
    if name.contains("..") {
        return Err(invalid("contains `..`"));
    }
    let first = name.chars().next().unwrap_or(' ');
    if !first.is_ascii_lowercase() {
        return Err(invalid("must start with a lowercase ASCII letter"));
    }
    for character in name.chars() {
        let ok = character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '_' | '-' | '.');
        if !ok {
            return Err(invalid("contains a character outside `a-z0-9_-.`"));
        }
    }
    Ok(())
}
