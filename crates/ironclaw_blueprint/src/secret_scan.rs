//! Inline-secret scanning over the raw TOML tree.
//!
//! Walking the untyped [`toml::Value`] (rather than the typed AST) means every
//! string in the document is checked — values *and* table keys, including ones
//! that would later be rejected as unknown keys — so a pasted credential fails
//! closed no matter where it lands. Keys matter as much as values: inside an
//! opaque `extensions[].config` table `deny_unknown_fields` cannot see them,
//! so a credential pasted as a key would otherwise sail through. Each finding
//! carries the dotted/indexed path to the offending string so the operator can
//! fix the exact line.
//!
//! The one legitimate way to name a secret in a blueprint is a
//! `${secret:<name>}` handle (per `docs/reborn/contracts/secrets.md`). A value
//! that is exactly such a handle is allowed — its name validated by
//! constructing the real [`ironclaw_host_api::SecretHandle`], so the grammar
//! accepted here is exactly the grammar the secrets layer accepts. Anything
//! else runs through the shared
//! [`ironclaw_reborn_config::reject_inline_secret`] guard.

use ironclaw_host_api::SecretHandle;
use ironclaw_reborn_config::reject_inline_secret;

use crate::error::{BlueprintError, host_api_reason};

const HANDLE_PREFIX: &str = "${secret:";
const HANDLE_SUFFIX: &str = "}";

/// Walk the whole document, rejecting inline secret material.
pub(crate) fn scan(root: &toml::Value) -> Result<(), BlueprintError> {
    walk(root, &mut String::new())
}

fn walk(value: &toml::Value, path: &mut String) -> Result<(), BlueprintError> {
    match value {
        toml::Value::String(text) => check_string(text, path),
        toml::Value::Table(table) => {
            for (key, child) in table {
                let len = path.len();
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(key);
                // Keys are operator-typed strings too; scan them so a
                // credential pasted as a key (not just as a value) is caught.
                check_string(key, path)?;
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
        // Integers, floats, booleans, datetimes cannot carry pasted
        // credentials in the shapes the guard detects.
        _ => Ok(()),
    }
}

fn check_string(text: &str, path: &str) -> Result<(), BlueprintError> {
    if let Some(handle) = parse_secret_handle(text) {
        return validate_handle_name(handle, path);
    }
    reject_inline_secret(path.to_string(), text).map_err(|source| BlueprintError::InlineSecret {
        path: path.to_string(),
        source,
    })
}

/// Returns the inner name if `text` is exactly a `${secret:<name>}` handle.
/// A string that merely *contains* the marker is not a handle — it falls
/// through to the inline-secret guard, which is the safe default.
fn parse_secret_handle(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let inner = trimmed
        .strip_prefix(HANDLE_PREFIX)?
        .strip_suffix(HANDLE_SUFFIX)?;
    Some(inner)
}

/// Validate a secret-handle name by constructing the real
/// [`SecretHandle`] — the same constructor the secrets layer uses — so a
/// handle accepted at parse time can never be rejected at resolve time, and
/// vice versa. Re-implementing the grammar here is how it drifted before.
fn validate_handle_name(name: &str, path: &str) -> Result<(), BlueprintError> {
    match SecretHandle::new(name) {
        Ok(_) => Ok(()),
        Err(err) => Err(BlueprintError::InvalidSecretHandle {
            path: path.to_string(),
            handle: name.to_string(),
            reason: host_api_reason(err),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_str(toml_src: &str) -> Result<(), BlueprintError> {
        let value: toml::Value = toml::from_str(toml_src).expect("valid toml");
        scan(&value)
    }

    #[test]
    fn allows_secret_handle() {
        scan_str(r#"api_key = "${secret:anthropic_api_key}""#).expect("handle allowed");
    }

    #[test]
    fn rejects_inline_credential_with_path() {
        let err =
            scan_str("[providers.anthropic]\napi_key = \"sk-proj-abcdef1234567890abcdef1234\"\n")
                .expect_err("inline secret rejected");
        match err {
            BlueprintError::InlineSecret { path, .. } => {
                assert_eq!(path, "providers.anthropic.api_key");
            }
            other => panic!("expected InlineSecret, got {other:?}"),
        }
    }

    #[test]
    fn reports_array_index_in_path() {
        let err = scan_str(
            "[[missions]]\nid = \"a\"\n[[missions]]\nid = \"sk-proj-abcdef1234567890abcdef1234\"\n",
        )
        .expect_err("inline secret rejected");
        match err {
            BlueprintError::InlineSecret { path, .. } => assert_eq!(path, "missions[1].id"),
            other => panic!("expected InlineSecret, got {other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_handle_name() {
        let err = scan_str(r#"api_key = "${secret:Bad Name}""#).expect_err("bad handle rejected");
        assert!(matches!(err, BlueprintError::InvalidSecretHandle { .. }));
    }

    #[test]
    fn rejects_handle_name_traversal() {
        let err = scan_str(r#"api_key = "${secret:../escape}""#).expect_err("traversal rejected");
        assert!(matches!(err, BlueprintError::InvalidSecretHandle { .. }));
    }

    /// Pins grammar parity with `ironclaw_host_api::SecretHandle`: digit-start
    /// names are valid there and must be valid here.
    #[test]
    fn allows_digit_start_handle_name() {
        scan_str(r#"api_key = "${secret:1password_token}""#).expect("digit start allowed");
    }

    /// Pins grammar parity the other way: empty dot segments are rejected by
    /// `SecretHandle` and must be rejected here too.
    #[test]
    fn rejects_empty_dot_segment_handle_name() {
        let err = scan_str(r#"api_key = "${secret:a.}""#).expect_err("empty dot segment rejected");
        assert!(matches!(err, BlueprintError::InvalidSecretHandle { .. }));
    }

    /// A credential pasted as a table KEY (not a value) must be caught too —
    /// inside an opaque config table, `deny_unknown_fields` never sees it.
    #[test]
    fn rejects_inline_credential_in_table_key() {
        let err = scan_str("[config]\n\"sk-proj-abcdef1234567890abcdef1234\" = true\n")
            .expect_err("secret-as-key rejected");
        match err {
            BlueprintError::InlineSecret { path, .. } => {
                assert_eq!(path, "config.sk-proj-abcdef1234567890abcdef1234");
            }
            other => panic!("expected InlineSecret, got {other:?}"),
        }
    }
}
