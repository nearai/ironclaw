//! Parse + validate blueprint source into a [`Blueprint`] AST.
//!
//! The pipeline is ordered to fail loud on the highest-signal problem first:
//!
//! 1. parse to an untyped [`toml::Value`] (syntax errors);
//! 2. scan every string for inline secrets (catches credentials even in keys
//!    that step 3 would reject as unknown);
//! 3. deserialize into the typed AST (`deny_unknown_fields` → unknown keys);
//! 4. semantic validation (api_version major, identifiers, harness shape).
//!
//! Identifier validation delegates to the typed IDs in `ironclaw_host_api` —
//! the same constructors the typed repos use downstream — so the grammar this
//! parser accepts is the grammar the rest of Reborn accepts, by construction.

use ironclaw_host_api::{
    AgentId, ExtensionId, HostApiError, MissionId, ProjectId, TenantId, UserId,
};

use crate::error::{BlueprintError, host_api_reason};
use crate::schema::{AppliesTo, Blueprint, BlueprintKind, Scope};
use crate::secret_scan;

/// The api_version this build understands. A `parse` of any other major is a
/// hard error — schema majors are forever and require a migration path.
pub const SUPPORTED_API_VERSION: &str = "ironclaw.config/v1";

// `pub(crate)`: `json_schema.rs` mirrors the version rule into the exported
// schema's `pattern`, built from these same constants.
pub(crate) const API_VERSION_PREFIX: &str = "ironclaw.config/v";
pub(crate) const SUPPORTED_MAJOR: &str = "1";

/// Parse and fully validate blueprint source text.
pub fn parse(source: &str) -> Result<Blueprint, BlueprintError> {
    let value: toml::Value = toml::from_str(source)?;
    secret_scan::scan(&value)?;
    let blueprint: Blueprint = value.try_into()?;
    validate(&blueprint)?;
    Ok(blueprint)
}

fn validate(blueprint: &Blueprint) -> Result<(), BlueprintError> {
    validate_api_version(&blueprint.api_version)?;
    // `kind` is enforced by the `BlueprintKind` enum during deserialization;
    // the explicit match keeps the intent legible and survives new variants.
    match blueprint.kind {
        BlueprintKind::Blueprint => {}
    }

    validate_scope(&blueprint.scope)?;

    if let Some(prompt) = &blueprint.system_prompt
        && let Some(applies) = &prompt.applies_to
    {
        validate_applies_to(applies)?;
    }

    if let Some(providers) = &blueprint.providers {
        if let Some(default_llm) = &providers.default_llm {
            validate_name_id("providers.default_llm", default_llm)?;
        }
        for name in providers.entries.keys() {
            validate_name_id(&format!("providers.{name}"), name)?;
        }
    }

    for (index, extension) in blueprint.extensions.iter().enumerate() {
        validate_name_id(&format!("extensions[{index}].id"), &extension.id)?;
        if let Some(version) = &extension.version {
            semver::VersionReq::parse(version).map_err(|e| BlueprintError::InvalidVersionReq {
                path: format!("extensions[{index}].version"),
                value: version.clone(),
                reason: e.to_string(),
            })?;
        }
    }
    for (index, skill) in blueprint.skills.iter().enumerate() {
        validate_name_id(&format!("skills[{index}].id"), &skill.id)?;
    }
    for (index, mission) in blueprint.missions.iter().enumerate() {
        validate_id(
            &format!("missions[{index}].id"),
            &mission.id,
            MissionId::new,
        )?;
    }
    for (index, project) in blueprint.projects.iter().enumerate() {
        validate_id(
            &format!("projects[{index}].id"),
            &project.id,
            ProjectId::new,
        )?;
    }

    if let Some(harness) = &blueprint.harness {
        if harness.id.is_some() && harness.inline.is_some() {
            return Err(BlueprintError::InvalidIdentifier {
                path: "harness".to_string(),
                value: "id + inline".to_string(),
                reason: "bind a registered harness by `id` or define one `inline`, not both"
                    .to_string(),
            });
        }
        if let Some(id) = &harness.id {
            validate_name_id("harness.id", id)?;
        }
        if let Some(inline) = &harness.inline {
            validate_name_id("harness.inline.id", &inline.id)?;
            for (index, required) in inline.required_extensions.iter().enumerate() {
                validate_name_id(
                    &format!("harness.inline.required_extensions[{index}].id"),
                    &required.id,
                )?;
            }
            for (index, required) in inline.required_skills.iter().enumerate() {
                validate_name_id(
                    &format!("harness.inline.required_skills[{index}].id"),
                    &required.id,
                )?;
            }
        }
    }

    Ok(())
}

/// Accept the exact supported version plus any minor/patch within the major
/// (`ironclaw.config/v1.2`); reject everything else. Strict prefix parsing
/// plus a digits-and-dots shape check means malformed strings like
/// `ironclaw.config/v2/v1` or `ironclaw.config/v1x` cannot sneak through,
/// and the major is compared exactly, so `v10` does not match `v1`.
fn validate_api_version(found: &str) -> Result<(), BlueprintError> {
    let unsupported = || BlueprintError::UnsupportedApiVersion {
        found: found.to_string(),
    };
    let Some(rest) = found.strip_prefix(API_VERSION_PREFIX) else {
        return Err(unsupported());
    };
    let well_formed = !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
        && !rest.starts_with('.')
        && !rest.ends_with('.')
        && !rest.contains("..");
    let major = rest.split('.').next().unwrap_or(rest);
    if well_formed && major == SUPPORTED_MAJOR {
        Ok(())
    } else {
        Err(unsupported())
    }
}

fn validate_scope(scope: &Scope) -> Result<(), BlueprintError> {
    if let Some(tenant) = &scope.tenant {
        validate_id("scope.tenant", tenant, TenantId::new)?;
    }
    if let Some(user) = &scope.user {
        validate_id("scope.user", user, UserId::new)?;
    }
    if let Some(project) = &scope.project {
        validate_id("scope.project", project, ProjectId::new)?;
    }
    if let Some(agent) = &scope.agent {
        validate_id("scope.agent", agent, AgentId::new)?;
    }
    Ok(())
}

/// `applies_to` narrows where a scoped setting lands. The epic's examples use
/// `"*"` as an explicit wildcard; anything else must be a valid scope id.
fn validate_applies_to(applies: &AppliesTo) -> Result<(), BlueprintError> {
    if let Some(project) = &applies.project
        && project != "*"
    {
        validate_id("system_prompt.applies_to.project", project, ProjectId::new)?;
    }
    if let Some(agent) = &applies.agent
        && agent != "*"
    {
        validate_id("system_prompt.applies_to.agent", agent, AgentId::new)?;
    }
    if let Some(user) = &applies.user
        && user != "*"
    {
        validate_id("system_prompt.applies_to.user", user, UserId::new)?;
    }
    Ok(())
}

/// Validate `value` with a typed-ID constructor from `ironclaw_host_api`,
/// mapping a failure into a path-bearing blueprint error. Only the upstream
/// `reason` is reused — the `kind` label is dropped so a skill validated via
/// the extension grammar never reports itself as an "extension".
fn validate_id<T>(
    path: &str,
    value: &str,
    construct: impl FnOnce(String) -> Result<T, HostApiError>,
) -> Result<(), BlueprintError> {
    match construct(value.to_string()) {
        Ok(_) => Ok(()),
        Err(err) => Err(BlueprintError::InvalidIdentifier {
            path: path.to_string(),
            value: value.to_string(),
            reason: host_api_reason(err),
        }),
    }
}

/// Registry-style names (extensions, skills, providers, harnesses) share the
/// host-api name-segment grammar. Skills, providers, and harnesses have no
/// typed ID upstream yet; `ExtensionId` carries the identical grammar.
fn validate_name_id(path: &str, value: &str) -> Result<(), BlueprintError> {
    validate_id(path, value, ExtensionId::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_version_constant_matches_prefix_and_major() {
        assert_eq!(
            SUPPORTED_API_VERSION,
            format!("{API_VERSION_PREFIX}{SUPPORTED_MAJOR}"),
            "SUPPORTED_API_VERSION must stay in sync with the prefix/major parts"
        );
    }

    #[test]
    fn api_version_accepts_minor_within_major() {
        validate_api_version("ironclaw.config/v1").expect("exact");
        validate_api_version("ironclaw.config/v1.2").expect("minor");
        validate_api_version("ironclaw.config/v1.2.3").expect("patch");
    }

    #[test]
    fn api_version_rejects_malformed_and_other_majors() {
        for bad in [
            "ironclaw.config/v2",
            "ironclaw.config/v10", // exact major compare, not prefix match
            "ironclaw.config/v2/v1",
            "ironclaw.config/v1x",
            "ironclaw.config/v",
            "ironclaw.config/v1.",
            "ironclaw.config/v.1",
            "ironclaw.config/v1..2",
            "bogus/v1",
            "",
        ] {
            assert!(
                validate_api_version(bad).is_err(),
                "`{bad}` must be rejected"
            );
        }
    }
}
