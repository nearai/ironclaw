//! Extensions/channels/tools converter
//! (v1 `wasm_tools` / `wasm_channels` / `tool_capabilities` → Reborn
//! `ExtensionInstallation`).
//!
//! Installed v1 WASM tools/channels are accumulated by validated
//! `ExtensionId`, then each group becomes one canonical Reborn
//! `ExtensionInstallation` with `installation_id == extension_id`. The
//! synthesized `InstalledLocal` manifest declares the
//! `ironclaw.capability_provider/v1` host API plus one placeholder,
//! approval-gated capability (a non-first-party manifest must declare a host API
//! or capability, and rejects top-level `[[capabilities]]`). Activation maps
//! from the v1 `status` column, failing closed when grouped source states
//! disagree; a tool's `tool_capabilities.allowed_secrets` become merged
//! `ExtensionCredentialBinding`s pointing at the migrated secrets. The store
//! itself is built by composition's `migration-support` seam
//! (`RebornTarget::extension_store`).
//!
//! Losses recorded (per installation): the manifest is a placeholder — the v1
//! tool's real capability contract and WASM binary are NOT carried over; tool
//! capability config beyond credential linkage (http_allowlist, rate limits,
//! workspace prefixes); and channel credential linkage (v1 has no explicit
//! channel→secret join; the secret *values* still migrate via the secrets
//! converter).

use std::collections::BTreeMap;
use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionActivationState, ExtensionCredentialBinding, ExtensionCredentialHandle,
    ExtensionInstallation, ExtensionInstallationError, ExtensionInstallationId,
    ExtensionManifestRecord, ExtensionManifestRef, HostApiContractRegistry, InstallationOwner,
    MANIFEST_SCHEMA_VERSION, ManifestSource, canonicalize_installation_rows,
};
use ironclaw_host_api::{ExtensionId, HostPortCatalog, SecretHandle, UserId};
use ironclaw_host_runtime::{default_host_api_contract_registry, default_host_port_catalog};

use crate::error::MigrationError;
use crate::legacy_snapshot::wasm_stores::{LibSqlWasmChannelStore, LibSqlWasmToolStore};
use crate::legacy_snapshot::wasm_stores::{PostgresWasmChannelStore, PostgresWasmToolStore};
use crate::legacy_snapshot::wasm_stores::{
    StoredWasmChannel, StoredWasmTool, ToolStatus, WasmChannelStore, WasmToolStore,
};
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;

pub(crate) async fn run(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let catalog = default_host_port_catalog().map_err(|e| MigrationError::WriteTarget {
        domain: "extension host-port catalog".into(),
        reason: e.to_string(),
    })?;
    let registry =
        default_host_api_contract_registry().map_err(|e| MigrationError::WriteTarget {
            domain: "extension host-api contract registry".into(),
            reason: e.to_string(),
        })?;

    let tool_store = build_tool_store(src);
    let channel_store = build_channel_store(src);
    let mut candidates_by_extension: BTreeMap<ExtensionId, Vec<InstallCandidate>> = BTreeMap::new();

    // Installed tools/channels are keyed by user_id; enumerate from both tables.
    let mut users: std::collections::BTreeSet<String> =
        src.distinct_users().await?.into_iter().collect();
    users.extend(src.distinct_user_ids_in("wasm_tools", "user_id").await?);
    users.extend(src.distinct_user_ids_in("wasm_channels", "user_id").await?);

    for user in users {
        if let Some(store) = tool_store.as_ref() {
            let tools = store
                .list(&user)
                .await
                .map_err(|e| MigrationError::ReadSource {
                    domain: "wasm_tools".into(),
                    reason: e.to_string(),
                })?;
            for tool in tools {
                let bindings = tool_credential_bindings(store.as_ref(), &tool, report).await?;
                collect_installation(
                    report,
                    &mut candidates_by_extension,
                    InstallInput {
                        owner: &user,
                        raw_name: &tool.name,
                        version: &tool.version,
                        description: &tool.description,
                        active: tool.status == ToolStatus::Active,
                        updated_at: tool.updated_at,
                        bindings,
                    },
                );
            }
        }

        if let Some(store) = channel_store.as_ref() {
            let channels = store
                .list(&user)
                .await
                .map_err(|e| MigrationError::ReadSource {
                    domain: "wasm_channels".into(),
                    reason: e.to_string(),
                })?;
            for channel in channels {
                collect_installation(
                    report,
                    &mut candidates_by_extension,
                    channel_input(&user, &channel),
                );
                report.record_loss(
                    Domain::Extension,
                    format!("channel:{}", channel.name),
                    "credential_binding",
                    LossReason::NoTargetField,
                    "v1 has no explicit channel→secret join; the credential value still \
                     migrates via the secrets converter, but the installation binding is \
                     not auto-linked"
                        .to_string(),
                );
            }
        }
    }

    for candidates in candidates_by_extension.into_values() {
        write_canonical_installation(tgt, options, report, &catalog, &registry, candidates).await?;
    }
    Ok(())
}

struct InstallInput<'a> {
    /// v1 owner user id. Candidates are merged into one private installation
    /// membership set after the source id has been validated.
    owner: &'a str,
    raw_name: &'a str,
    version: &'a str,
    description: &'a str,
    active: bool,
    updated_at: chrono::DateTime<chrono::Utc>,
    bindings: Vec<ExtensionCredentialBinding>,
}

struct InstallCandidate {
    extension_id: ExtensionId,
    owner: UserId,
    raw_name: String,
    version: String,
    description: String,
    active: bool,
    updated_at: chrono::DateTime<chrono::Utc>,
    bindings: Vec<ExtensionCredentialBinding>,
}

fn channel_input<'a>(owner: &'a str, channel: &'a StoredWasmChannel) -> InstallInput<'a> {
    InstallInput {
        owner,
        raw_name: &channel.name,
        version: &channel.version,
        description: &channel.description,
        active: channel.status == "active",
        updated_at: channel.updated_at,
        bindings: Vec::new(),
    }
}

fn collect_installation(
    report: &mut MigrationReport,
    candidates_by_extension: &mut BTreeMap<ExtensionId, Vec<InstallCandidate>>,
    input: InstallInput<'_>,
) {
    let source_id = format!("extension:{}:{}", input.owner, input.raw_name);
    let ext_id_str = sanitize_extension_id(input.raw_name);
    let extension_id = match ExtensionId::new(&ext_id_str) {
        Ok(id) => id,
        Err(e) => {
            report.record_loss(
                Domain::Extension,
                &source_id,
                "id",
                LossReason::Unparseable,
                format!("could not derive a valid Reborn extension id: {e}"),
            );
            return;
        }
    };

    // The synthesized manifest is a migration placeholder: v1 tools have no
    // Reborn capability contract and the WASM binary is not carried over, so a
    // single generic host-mediated capability stands in. Record that gap.
    report.record_loss(
        Domain::Extension,
        &source_id,
        "manifest_fidelity",
        LossReason::Degraded,
        "v1 tool capability contract + WASM binary are not migrated; a placeholder \
         capability_provider manifest is synthesized so the installation record + \
         activation + credential bindings carry over"
            .to_string(),
    );

    // v1 installs were per-user; carry each validated owner into the eventual
    // canonical private membership set under the #5459 P1 ownership model.
    let owner = match UserId::new(input.owner) {
        Ok(user_id) => user_id,
        Err(e) => {
            report.record_loss(
                Domain::Extension,
                &source_id,
                "owner",
                LossReason::Unparseable,
                format!("invalid owner user id: {e}"),
            );
            return;
        }
    };
    candidates_by_extension
        .entry(extension_id.clone())
        .or_default()
        .push(InstallCandidate {
            extension_id,
            owner,
            raw_name: input.raw_name.to_string(),
            version: input.version.to_string(),
            description: input.description.to_string(),
            active: input.active,
            updated_at: input.updated_at,
            bindings: input.bindings,
        });
}

// arch-exempt: too_many_args, migration converter scope + catalog + registry + group, plan #5459
#[allow(clippy::too_many_arguments)]
async fn write_canonical_installation(
    tgt: &RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
    catalog: &HostPortCatalog,
    registry: &HostApiContractRegistry,
    candidates: Vec<InstallCandidate>,
) -> Result<(), MigrationError> {
    let Some((first, rest)) = candidates.split_first() else {
        return Ok(());
    };
    let source_id = format!("extension:{}", first.extension_id);

    if rest.iter().any(|candidate| {
        candidate.raw_name != first.raw_name
            || candidate.version != first.version
            || candidate.description != first.description
    }) {
        report.record_loss(
            Domain::Extension,
            &source_id,
            "canonicalization",
            LossReason::Unparseable,
            "source names or manifest metadata sanitize to the same ExtensionId but disagree; \
             the group was skipped and no partial canonical installation was written"
                .to_string(),
        );
        return Ok(());
    }

    let manifest_toml = build_manifest_toml(
        first.extension_id.as_str(),
        &first.raw_name,
        &first.version,
        &first.description,
    );
    let manifest = match ExtensionManifestRecord::from_toml_with_contracts(
        manifest_toml,
        ManifestSource::InstalledLocal,
        catalog,
        None,
        registry,
    ) {
        Ok(manifest) => manifest,
        Err(e) => {
            report.record_loss(
                Domain::Extension,
                &source_id,
                "manifest",
                LossReason::Unparseable,
                format!("synthesized manifest did not validate: {e}"),
            );
            return Ok(());
        }
    };

    // Build one typed row per source install, then let the generic extension
    // reducer own canonical ids, ownership, activation, credentials, and
    // timestamps. The source-derived id exists only as a deterministic health
    // tie-break; the reducer replaces it with `extension_id`.
    let installations = candidates
        .into_iter()
        .map(|candidate| {
            let installation_id = ExtensionInstallationId::new(format!(
                "{}:{}",
                candidate.extension_id.as_str(),
                candidate.owner.as_str()
            ))?;
            ExtensionInstallation::new(
                installation_id,
                candidate.extension_id.clone(),
                if candidate.active {
                    ExtensionActivationState::Enabled
                } else {
                    ExtensionActivationState::Disabled
                },
                ExtensionManifestRef::new(candidate.extension_id.clone(), None),
                candidate.bindings,
                candidate.updated_at,
                InstallationOwner::user(candidate.owner),
            )
        })
        .collect::<Result<Vec<_>, ExtensionInstallationError>>();
    let installations = match installations {
        Ok(installations) => installations,
        Err(error) => {
            report.record_loss(
                Domain::Extension,
                &source_id,
                "installation",
                LossReason::Unparseable,
                format!(
                    "could not build source installation rows: {error}; the group was skipped and no partial canonical installation was written"
                ),
            );
            return Ok(());
        }
    };
    let mut canonical = match canonicalize_installation_rows(installations) {
        Ok(canonical) => canonical,
        Err(ExtensionInstallationError::ConflictingCredentialBinding { handle, .. }) => {
            report.record_loss(
                Domain::Extension,
                &source_id,
                "credential_bindings",
                LossReason::Unparseable,
                format!(
                    "credential handle '{handle}' maps to conflicting secret handles; the group was skipped and no partial canonical installation was written"
                ),
            );
            return Ok(());
        }
        Err(error) => {
            report.record_loss(
                Domain::Extension,
                &source_id,
                "canonicalization",
                LossReason::Unparseable,
                format!(
                    "could not reduce source installation rows: {error}; the group was skipped and no partial canonical installation was written"
                ),
            );
            return Ok(());
        }
    };
    let Some(installation) = canonical.pop() else {
        report.record_loss(
            Domain::Extension,
            &source_id,
            "canonicalization",
            LossReason::Unparseable,
            "no canonical installation row was produced; the group was skipped and no partial canonical installation was written".to_string(),
        );
        return Ok(());
    };

    if !options.dry_run {
        tgt.extension_store
            .upsert_manifest_and_installation(manifest, installation)
            .await
            .map_err(|e| MigrationError::WriteTarget {
                domain: format!("extension {source_id}"),
                reason: e.to_string(),
            })?;
    }
    report.stats.extensions += 1;
    Ok(())
}

/// Build credential bindings from a tool's `allowed_secrets`, recording the
/// capability config that has no Reborn target.
async fn tool_credential_bindings(
    store: &dyn WasmToolStore,
    tool: &StoredWasmTool,
    report: &mut MigrationReport,
) -> Result<Vec<ExtensionCredentialBinding>, MigrationError> {
    // A read *error* is a real infrastructure failure and aborts the run; a
    // legitimate "no capabilities row" (`Ok(None)`) just yields no bindings.
    let capabilities = match store.get_capabilities(tool.id).await {
        Ok(Some(capabilities)) => capabilities,
        Ok(None) => return Ok(Vec::new()),
        Err(e) => {
            return Err(MigrationError::ReadSource {
                domain: "tool_capabilities".into(),
                reason: e.to_string(),
            });
        }
    };
    report.record_loss(
        Domain::Extension,
        format!("tool:{}", tool.name),
        "capabilities",
        LossReason::NoTargetField,
        "tool http_allowlist / rate limits / workspace prefixes have no Reborn \
         installation field"
            .to_string(),
    );
    let mut bindings_by_handle = BTreeMap::new();
    for secret_name in capabilities.allowed_secrets {
        match (
            ExtensionCredentialHandle::new(secret_name.clone()),
            SecretHandle::new(&secret_name),
        ) {
            (Ok(handle), Ok(secret_handle)) => {
                bindings_by_handle.entry(handle).or_insert(secret_handle);
            }
            // An unconvertible secret name is recorded, not dropped silently.
            _ => report.record_loss(
                Domain::Extension,
                format!("tool:{}", tool.name),
                "allowed_secret",
                LossReason::Unparseable,
                format!("secret name '{secret_name}' is not a valid Reborn credential binding"),
            ),
        }
    }
    Ok(bindings_by_handle
        .into_iter()
        .map(|(handle, secret_handle)| ExtensionCredentialBinding::new(handle, secret_handle))
        .collect())
}

fn build_tool_store(src: &V1Source) -> Option<Arc<dyn WasmToolStore>> {
    if let Some(db) = src.handles.libsql_db.as_ref() {
        return Some(Arc::new(LibSqlWasmToolStore::new(db.clone())));
    }
    if let Some(pool) = src.handles.pg_pool.as_ref() {
        return Some(Arc::new(PostgresWasmToolStore::new(pool.clone())));
    }
    None
}

fn build_channel_store(src: &V1Source) -> Option<Arc<dyn WasmChannelStore>> {
    if let Some(db) = src.handles.libsql_db.as_ref() {
        return Some(Arc::new(LibSqlWasmChannelStore::new(db.clone())));
    }
    if let Some(pool) = src.handles.pg_pool.as_ref() {
        return Some(Arc::new(PostgresWasmChannelStore::new(pool.clone())));
    }
    None
}

/// Sanitize a v1 tool/channel name into a valid Reborn `ExtensionId`
/// (`validate_name_segment`: lowercase, starts alnum, `[a-z0-9._-]`, ≤128).
fn sanitize_extension_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || matches!(lower, '_' | '-' | '.') {
            out.push(lower);
        } else {
            out.push('_');
        }
    }
    // Must start with an alphanumeric.
    if !out
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        out.insert(0, 'x');
    }
    out.truncate(128);
    if out.is_empty() {
        out.push_str("ext");
    }
    out
}

fn build_manifest_toml(ext_id: &str, name: &str, version: &str, description: &str) -> String {
    // A valid non-first-party manifest declares the capability_provider host API
    // and at least one namespaced, host-mediated capability (empty manifests and
    // top-level `[[capabilities]]` are both rejected). `ask` permission keeps the
    // migrated tool approval-gated.
    format!(
        r#"schema_version = "{schema}"
id = "{ext_id}"
name = "{name}"
version = "{version}"
description = "{description}"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{ext_id}.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[[capability_provider.tools.capabilities]]
id = "{ext_id}.invoke"
description = "Migrated v1 tool capability (placeholder)."
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/{ext_id}/invoke.input.v1.json"
output_schema_ref = "schemas/{ext_id}/invoke.output.v1.json"
prompt_doc_ref = "prompts/{ext_id}/invoke.md"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
        name = toml_escape(name),
        version = toml_escape(normalize_version(version)),
        description = toml_escape(description),
    )
}

fn normalize_version(version: &str) -> &str {
    if version.trim().is_empty() {
        "0.1.0"
    } else {
        version
    }
}

/// Escape a value for a TOML basic string: backslash + quote escaped, control
/// characters (incl. newlines) dropped so the synthesized manifest stays valid.
fn toml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}
