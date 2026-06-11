//! Parse + validate harness-manifest source into a [`HarnessManifest`].
//!
//! Same pipeline shape as the blueprint parser: untyped parse → inline-secret
//! scan → typed deserialize (`deny_unknown_fields`) → semantic validation.

use crate::error::HarnessError;
use crate::schema::{HarnessKind, HarnessManifest};
use crate::secret_scan;

/// The api_version this build understands.
pub const SUPPORTED_API_VERSION: &str = "ironclaw.harness/v1";

/// Parse and fully validate harness-manifest source text.
pub fn parse(source: &str) -> Result<HarnessManifest, HarnessError> {
    let value: toml::Value = toml::from_str(source)?;
    secret_scan::scan(&value)?;
    let manifest: HarnessManifest = value.try_into()?;
    validate(&manifest)?;
    Ok(manifest)
}

fn validate(manifest: &HarnessManifest) -> Result<(), HarnessError> {
    validate_api_version(&manifest.api_version)?;
    match manifest.kind {
        HarnessKind::Harness => {}
    }

    validate_identifier("id", &manifest.id)?;
    if let Some(trust) = &manifest.trust {
        validate_identifier("trust", trust)?;
    }
    for (index, required) in manifest.required_extensions.iter().enumerate() {
        validate_identifier(&format!("required_extensions[{index}].id"), &required.id)?;
    }
    for (index, required) in manifest.required_skills.iter().enumerate() {
        validate_identifier(&format!("required_skills[{index}].id"), &required.id)?;
    }
    Ok(())
}

fn validate_api_version(found: &str) -> Result<(), HarnessError> {
    if found == SUPPORTED_API_VERSION {
        return Ok(());
    }
    let major_of = |s: &str| {
        s.rsplit("/v")
            .next()
            .and_then(|seg| seg.split('.').next())
            .unwrap_or(s)
            .to_string()
    };
    if found.starts_with("ironclaw.harness/v") && major_of(found) == major_of(SUPPORTED_API_VERSION)
    {
        Ok(())
    } else {
        Err(HarnessError::UnsupportedApiVersion {
            found: found.to_string(),
        })
    }
}

/// Identifiers: non-empty, bounded, no path separators or whitespace.
fn validate_identifier(path: &str, value: &str) -> Result<(), HarnessError> {
    let invalid = |reason: &str| HarnessError::InvalidIdentifier {
        path: path.to_string(),
        value: value.to_string(),
        reason: reason.to_string(),
    };
    if value.is_empty() {
        return Err(invalid("empty identifier"));
    }
    if value.len() > 128 {
        return Err(invalid("longer than 128 bytes"));
    }
    if value.contains("..") {
        return Err(invalid("contains `..`"));
    }
    for character in value.chars() {
        let ok = character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.');
        if !ok {
            return Err(invalid(
                "contains a character outside `a-zA-Z0-9_-.` (no spaces or slashes)",
            ));
        }
    }
    Ok(())
}
