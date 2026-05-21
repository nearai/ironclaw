use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_skills::SkillRegistry;

use crate::cli::hub_install::{hub_manifest_url_for_tag, validate_hub_name};
use crate::context::JobContext;
use crate::extensions::{EnsureReadyIntent, ExtensionKind, ExtensionManager};
use crate::registry::{
    HubInstallOutcome, HubInstaller, HubManifest, HubSkillEntry, HubToolEntry, Provenance,
};
use crate::tools::builtin::extension_tools::output_from_ensure_ready;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HubEntryKind {
    Tool,
    Skill,
}

impl HubEntryKind {
    fn as_str(self) -> &'static str {
        match self {
            HubEntryKind::Tool => "tool",
            HubEntryKind::Skill => "skill",
        }
    }

    fn from_param(s: &str) -> Result<Self, ToolError> {
        match s {
            "tool" => Ok(Self::Tool),
            "skill" => Ok(Self::Skill),
            other => Err(ToolError::InvalidParameters(format!(
                "kind must be 'tool' or 'skill', got '{other}'"
            ))),
        }
    }
}

#[derive(Clone)]
pub struct IronhubDeps {
    pub extension_manager: Arc<ExtensionManager>,
    pub skill_registry: Option<Arc<std::sync::RwLock<SkillRegistry>>>,
}

fn build_installer(
    release_tag: Option<&str>,
    skills_dir_override: Option<std::path::PathBuf>,
) -> Result<HubInstaller, ToolError> {
    let mut installer = HubInstaller::with_defaults();
    if let Some(dir) = skills_dir_override {
        installer = installer.with_skills_dir(dir);
    }
    if let Some(tag) = release_tag {
        let url = hub_manifest_url_for_tag(tag)
            .map_err(|e: anyhow::Error| ToolError::InvalidParameters(e.to_string()))?;
        installer = installer.with_manifest_url(url);
    }
    Ok(installer)
}

fn catalog_unavailable() -> ToolError {
    ToolError::ExternalService("IronHub catalog is temporarily unavailable".into())
}

fn classify_and_gate(
    manifest: &HubManifest,
    name: &str,
    hint: Option<HubEntryKind>,
    acknowledge_unverified: bool,
) -> Result<(HubEntryKind, Provenance), ToolError> {
    let kind = classify(manifest, name, hint)?;
    let provenance = match kind {
        HubEntryKind::Tool => manifest.find_tool(name).map(|t| t.provenance),
        HubEntryKind::Skill => manifest.find_skill(name).map(|s| s.provenance),
    }
    .unwrap_or_default();
    if provenance.is_community_unverified() && !acknowledge_unverified {
        return Err(ToolError::InvalidParameters(format!(
            "'{name}' is UNVERIFIED community content (trust tier: {}). \
             Not NEAR-vetted. Re-run with acknowledge_unverified=true to \
             install at your own risk.",
            provenance.as_wire()
        )));
    }
    Ok((kind, provenance))
}

fn classify(
    manifest: &HubManifest,
    name: &str,
    hint: Option<HubEntryKind>,
) -> Result<HubEntryKind, ToolError> {
    let in_tools = manifest.find_tool(name).is_some();
    let in_skills = manifest.find_skill(name).is_some();

    if let Some(HubEntryKind::Tool) = hint {
        if !in_tools {
            return Err(ToolError::InvalidParameters(format!(
                "'{name}' is not a tool in this IronHub release"
            )));
        }
        return Ok(HubEntryKind::Tool);
    }
    if let Some(HubEntryKind::Skill) = hint {
        if !in_skills {
            return Err(ToolError::InvalidParameters(format!(
                "'{name}' is not a skill in this IronHub release"
            )));
        }
        return Ok(HubEntryKind::Skill);
    }

    match (in_tools, in_skills) {
        (true, false) => Ok(HubEntryKind::Tool),
        (false, true) => Ok(HubEntryKind::Skill),
        (true, true) => Err(ToolError::InvalidParameters(format!(
            "'{name}' exists as both a tool and a skill in this release; pass kind='tool' or kind='skill' to disambiguate"
        ))),
        (false, false) => {
            let suggestions = nearest_matches(manifest, name);
            if suggestions.is_empty() {
                Err(ToolError::InvalidParameters(format!(
                    "'{name}' is not in this IronHub release"
                )))
            } else {
                Err(ToolError::InvalidParameters(format!(
                    "'{name}' is not in this IronHub release. Did you mean: {}?",
                    suggestions.join(", ")
                )))
            }
        }
    }
}

fn nearest_matches(manifest: &HubManifest, query: &str) -> Vec<String> {
    let q = query.to_ascii_lowercase();
    let mut out: Vec<String> = manifest
        .tools
        .iter()
        .map(|t| t.name.clone())
        .chain(manifest.skills.iter().map(|s| s.name.clone()))
        .filter(|n| {
            let nl = n.to_ascii_lowercase();
            nl.contains(&q) || q.contains(&nl)
        })
        .collect();
    out.sort();
    out.truncate(5);
    out
}

fn entry_matches(name: &str, description: &str, query_lower: &str) -> bool {
    name.to_ascii_lowercase().contains(query_lower)
        || description.to_ascii_lowercase().contains(query_lower)
}

fn install_outcome_to_json(kind: HubEntryKind, outcome: &HubInstallOutcome) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "status".into(),
        serde_json::Value::String("installed".into()),
    );
    obj.insert(
        "kind".into(),
        serde_json::Value::String(kind.as_str().into()),
    );
    obj.insert(
        "name".into(),
        serde_json::Value::String(outcome.name.clone()),
    );
    obj.insert(
        "version".into(),
        serde_json::Value::String(outcome.version.clone()),
    );
    obj.insert(
        "release_tag".into(),
        serde_json::Value::String(outcome.release_tag.clone()),
    );
    obj.insert(
        "primary_path".into(),
        serde_json::Value::String(outcome.primary_path.display().to_string()),
    );
    if let Some(meta) = &outcome.metadata_path {
        obj.insert(
            "metadata_path".into(),
            serde_json::Value::String(meta.display().to_string()),
        );
    }
    obj.insert(
        "provenance".into(),
        serde_json::Value::String(outcome.provenance.as_wire().into()),
    );
    if outcome.provenance.is_community_unverified() {
        obj.insert("unverified".into(), serde_json::Value::Bool(true));
        obj.insert(
            "warning".into(),
            serde_json::Value::String(format!(
                "{} - not NEAR-vetted",
                outcome.provenance.trust_label()
            )),
        );
    }
    serde_json::Value::Object(obj)
}

fn tool_entry_json(entry: &HubToolEntry) -> serde_json::Value {
    serde_json::json!({
        "kind": "tool",
        "name": entry.name,
        "version": entry.version,
        "description": entry.description,
        "provenance": entry.provenance.as_wire(),
        "trust_label": entry.provenance.trust_label(),
    })
}

fn skill_entry_json(entry: &HubSkillEntry) -> serde_json::Value {
    serde_json::json!({
        "kind": "skill",
        "name": entry.name,
        "version": entry.version,
        "description": entry.description,
        "provenance": entry.provenance.as_wire(),
        "trust_label": entry.provenance.trust_label(),
    })
}

fn skill_install_dir(
    registry: &Option<Arc<std::sync::RwLock<SkillRegistry>>>,
) -> Option<std::path::PathBuf> {
    let registry = registry.as_ref()?;
    let guard = registry.read().unwrap_or_else(|poison| {
        tracing::error!(
            "skill registry RwLock was poisoned (a previous writer panicked); recovering"
        );
        poison.into_inner()
    });
    Some(
        guard
            .installed_dir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| guard.install_target_dir().to_path_buf()),
    )
}

pub struct IronhubInstallTool {
    deps: IronhubDeps,
}

impl IronhubInstallTool {
    pub fn new(deps: IronhubDeps) -> Self {
        Self { deps }
    }

    async fn install_from_manifest(
        &self,
        start: std::time::Instant,
        manifest: HubManifest,
        parsed: InstallParams,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let (kind, _provenance) = classify_and_gate(
            &manifest,
            &parsed.name,
            parsed.kind_hint,
            parsed.acknowledge_unverified,
        )?;

        let skills_dir = match kind {
            HubEntryKind::Skill => skill_install_dir(&self.deps.skill_registry),
            HubEntryKind::Tool => None,
        };
        let installer = build_installer(parsed.release_tag.as_deref(), skills_dir)?;

        match kind {
            HubEntryKind::Tool => {
                let outcome = installer
                    .install_tool_from_manifest(&manifest, &parsed.name, parsed.force)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
                let ready = self
                    .deps
                    .extension_manager
                    .ensure_extension_ready(
                        &parsed.name,
                        &ctx.user_id,
                        EnsureReadyIntent::PostInstall,
                    )
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
                let mut json = install_outcome_to_json(kind, &outcome);
                if let Some(obj) = json.as_object_mut() {
                    obj.insert("activation".into(), output_from_ensure_ready(ready));
                }
                Ok(ToolOutput::success(json, start.elapsed()))
            }
            HubEntryKind::Skill => {
                let outcome = installer
                    .install_skill_from_manifest(&manifest, &parsed.name, parsed.force)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
                if let Some(reg) = &self.deps.skill_registry {
                    let reg = Arc::clone(reg);
                    let handle = tokio::runtime::Handle::current();
                    tokio::task::spawn_blocking(move || {
                        let mut guard = reg.write().unwrap_or_else(|poison| {
                            tracing::error!(
                                "skill registry RwLock was poisoned (a previous writer panicked); recovering"
                            );
                            poison.into_inner()
                        });
                        handle.block_on(guard.reload());
                    })
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!("skill registry reload join: {e}"))
                    })?;
                }
                Ok(ToolOutput::success(
                    install_outcome_to_json(kind, &outcome),
                    start.elapsed(),
                ))
            }
        }
    }
}

struct InstallParams {
    name: String,
    kind_hint: Option<HubEntryKind>,
    release_tag: Option<String>,
    force: bool,
    acknowledge_unverified: bool,
}

impl InstallParams {
    fn from_json(params: &serde_json::Value) -> Result<Self, ToolError> {
        let name = require_str(params, "name")?.to_string();
        validate_hub_name(&name)
            .map_err(|e: anyhow::Error| ToolError::InvalidParameters(e.to_string()))?;
        let kind_hint = params
            .get("kind")
            .and_then(|v| v.as_str())
            .map(HubEntryKind::from_param)
            .transpose()?;
        let release_tag = params
            .get("release_tag")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let acknowledge_unverified = params
            .get("acknowledge_unverified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(Self {
            name,
            kind_hint,
            release_tag,
            force,
            acknowledge_unverified,
        })
    }
}

#[async_trait]
impl Tool for IronhubInstallTool {
    fn name(&self) -> &str {
        "ironhub_install"
    }

    fn description(&self) -> &str {
        "Install a tool or skill from the IronHub catalog by name. \
         Auto-detects whether the name refers to a tool or skill from the published manifest; \
         pass kind='tool' or kind='skill' only when the same name exists in both. \
         Pin release_tag to install from a specific IronHub release (default: latest)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": {
                    "type": "string",
                    "pattern": "^[a-z0-9][a-z0-9_-]*$",
                    "minLength": 1,
                    "maxLength": 64,
                    "description": "IronHub entry name, e.g. 'clickup' or 'chief-of-staff'"
                },
                "kind": { "type": "string", "enum": ["tool", "skill"] },
                "release_tag": {
                    "type": "string",
                    "pattern": "^[A-Za-z0-9._-]+$",
                    "minLength": 1,
                    "maxLength": 128
                },
                "force": { "type": "boolean", "default": false },
                "acknowledge_unverified": { "type": "boolean", "default": false }
            },
            "required": ["name"]
        })
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig {
            requests_per_minute: 6,
            requests_per_hour: 30,
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let parsed = InstallParams::from_json(&params)?;
        let probe = build_installer(parsed.release_tag.as_deref(), None)?;
        let manifest = probe
            .fetch_manifest()
            .await
            .map_err(|_| catalog_unavailable())?;
        self.install_from_manifest(start, manifest, parsed, ctx)
            .await
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct IronhubSearchTool;

impl IronhubSearchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for IronhubSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for IronhubSearchTool {
    fn name(&self) -> &str {
        "ironhub_search"
    }

    fn description(&self) -> &str {
        "Search the IronHub catalog by substring against entry names and descriptions. \
         Returns matching tools and skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 128
                },
                "release_tag": {
                    "type": "string",
                    "pattern": "^[A-Za-z0-9._-]+$",
                    "minLength": 1,
                    "maxLength": 128
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let query = require_str(&params, "query")?;
        let release_tag = params
            .get("release_tag")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let installer = build_installer(release_tag.as_deref(), None)?;
        let manifest = installer
            .fetch_manifest()
            .await
            .map_err(|_| catalog_unavailable())?;

        let q = query.to_ascii_lowercase();
        let mut results: Vec<serde_json::Value> = manifest
            .tools
            .iter()
            .filter(|t| entry_matches(&t.name, &t.description, &q))
            .map(tool_entry_json)
            .collect();
        results.extend(
            manifest
                .skills
                .iter()
                .filter(|s| entry_matches(&s.name, &s.description, &q))
                .map(skill_entry_json),
        );

        let json = serde_json::json!({
            "query": query,
            "release_tag": manifest.release_tag,
            "count": results.len(),
            "results": results,
        });
        Ok(ToolOutput::success(json, start.elapsed()))
    }
}

pub struct IronhubListTool;

impl IronhubListTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for IronhubListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for IronhubListTool {
    fn name(&self) -> &str {
        "ironhub_list"
    }

    fn description(&self) -> &str {
        "List everything available in the IronHub catalog grouped by tools and skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "release_tag": {
                    "type": "string",
                    "pattern": "^[A-Za-z0-9._-]+$",
                    "minLength": 1,
                    "maxLength": 128
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let release_tag = params
            .get("release_tag")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let installer = build_installer(release_tag.as_deref(), None)?;
        let manifest = installer
            .fetch_manifest()
            .await
            .map_err(|_| catalog_unavailable())?;

        let tools: Vec<serde_json::Value> = manifest.tools.iter().map(tool_entry_json).collect();
        let skills: Vec<serde_json::Value> = manifest.skills.iter().map(skill_entry_json).collect();
        let json = serde_json::json!({
            "release_tag": manifest.release_tag,
            "repo": manifest.repo,
            "counts": {
                "tools": tools.len(),
                "skills": skills.len(),
            },
            "tools": tools,
            "skills": skills,
        });
        Ok(ToolOutput::success(json, start.elapsed()))
    }
}

pub struct IronhubInfoTool;

impl IronhubInfoTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for IronhubInfoTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for IronhubInfoTool {
    fn name(&self) -> &str {
        "ironhub_info"
    }

    fn description(&self) -> &str {
        "Show detailed metadata for one IronHub entry (tool or skill) including version, \
         description, artifact URLs, and SHA-256 checksums."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": {
                    "type": "string",
                    "pattern": "^[a-z0-9][a-z0-9_-]*$",
                    "minLength": 1,
                    "maxLength": 64
                },
                "release_tag": {
                    "type": "string",
                    "pattern": "^[A-Za-z0-9._-]+$",
                    "minLength": 1,
                    "maxLength": 128
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let name = require_str(&params, "name")?;
        validate_hub_name(name)
            .map_err(|e: anyhow::Error| ToolError::InvalidParameters(e.to_string()))?;
        let release_tag = params
            .get("release_tag")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let installer = build_installer(release_tag.as_deref(), None)?;
        let manifest = installer
            .fetch_manifest()
            .await
            .map_err(|_| catalog_unavailable())?;

        if let Some(t) = manifest.find_tool(name) {
            let json = serde_json::json!({
                "kind": "tool",
                "name": t.name,
                "crate_name": t.crate_name,
                "version": t.version,
                "description": t.description,
                "provenance": t.provenance.as_wire(),
                "release_tag": manifest.release_tag,
                "wasm": {
                    "url": t.wasm.url,
                    "size_bytes": t.wasm.size_bytes,
                    "sha256": t.wasm.sha256,
                },
                "capabilities": {
                    "url": t.capabilities.url,
                    "size_bytes": t.capabilities.size_bytes,
                    "sha256": t.capabilities.sha256,
                }
            });
            return Ok(ToolOutput::success(json, start.elapsed()));
        }
        if let Some(s) = manifest.find_skill(name) {
            let json = serde_json::json!({
                "kind": "skill",
                "name": s.name,
                "trunk": s.trunk,
                "version": s.version,
                "description": s.description,
                "provenance": s.provenance.as_wire(),
                "release_tag": manifest.release_tag,
                "skill_md": {
                    "url": s.skill_md.url,
                    "size_bytes": s.skill_md.size_bytes,
                    "sha256": s.skill_md.sha256,
                }
            });
            return Ok(ToolOutput::success(json, start.elapsed()));
        }

        let suggestions = nearest_matches(&manifest, name);
        if suggestions.is_empty() {
            Err(ToolError::InvalidParameters(format!(
                "'{name}' is not in this IronHub release"
            )))
        } else {
            Err(ToolError::InvalidParameters(format!(
                "'{name}' is not in this IronHub release. Did you mean: {}?",
                suggestions.join(", ")
            )))
        }
    }
}

pub struct IronhubRemoveTool {
    deps: IronhubDeps,
}

impl IronhubRemoveTool {
    pub fn new(deps: IronhubDeps) -> Self {
        Self { deps }
    }
}

#[async_trait]
impl Tool for IronhubRemoveTool {
    fn name(&self) -> &str {
        "ironhub_remove"
    }

    fn description(&self) -> &str {
        "Remove an installed IronHub tool by name. Deletes the tool's files and \
         unregisters it. Skills are removed with the skill_remove tool."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": {
                    "type": "string",
                    "pattern": "^[a-z0-9][a-z0-9_-]*$",
                    "minLength": 1,
                    "maxLength": 64
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let name = require_str(&params, "name")?;
        validate_hub_name(name)
            .map_err(|e: anyhow::Error| ToolError::InvalidParameters(e.to_string()))?;

        let message = self
            .deps
            .extension_manager
            .remove(name, &ctx.user_id)
            .await
            .map_err(|_| {
                ToolError::InvalidParameters(format!(
                    "'{name}' is not an installed IronHub tool. \
                     If it is a skill, remove it with the skill_remove tool."
                ))
            })?;

        let still_present = self
            .deps
            .extension_manager
            .list(Some(ExtensionKind::WasmTool), false, &ctx.user_id)
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(format!(
                    "could not verify '{name}' removal from installed extensions"
                ))
            })?
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(name));
        if still_present {
            return Err(ToolError::ExecutionFailed(format!(
                "'{name}' is still present after removal"
            )));
        }

        Ok(ToolOutput::success(
            serde_json::json!({
                "status": "removed",
                "name": name,
                "message": message,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{HubArtifact, HubSkillEntry, HubToolEntry, Provenance};

    fn art(name: &str, ext: &str) -> HubArtifact {
        HubArtifact {
            url: format!(
                "https://github.com/nearai/ironhub/releases/download/test/{}.{}",
                name, ext
            ),
            size_bytes: 1024,
            sha256: "a".repeat(64),
        }
    }

    fn manifest_with(tools: Vec<&str>, skills: Vec<&str>) -> HubManifest {
        HubManifest {
            version: "1".into(),
            generated_at: "2026-05-14T00:00:00Z".into(),
            release_tag: "release-test".into(),
            repo: "nearai/ironhub".into(),
            tools: tools
                .into_iter()
                .map(|n| HubToolEntry {
                    name: n.into(),
                    crate_name: format!("{}-tool", n),
                    version: "0.1.0".into(),
                    description: format!("{} tool", n),
                    provenance: Provenance::Official,
                    wasm: art(n, "wasm"),
                    capabilities: art(n, "capabilities.json"),
                })
                .collect(),
            skills: skills
                .into_iter()
                .map(|n| HubSkillEntry {
                    name: n.into(),
                    trunk: String::new(),
                    version: "0.1.0".into(),
                    description: format!("{} skill", n),
                    provenance: Provenance::Official,
                    skill_md: art(n, "SKILL.md"),
                })
                .collect(),
        }
    }

    fn outcome(name: &str, with_meta: bool) -> HubInstallOutcome {
        outcome_prov(name, with_meta, Provenance::Official)
    }

    fn outcome_prov(name: &str, with_meta: bool, provenance: Provenance) -> HubInstallOutcome {
        HubInstallOutcome {
            name: name.into(),
            version: "0.1.0".into(),
            release_tag: "release-test".into(),
            provenance,
            primary_path: std::path::PathBuf::from(format!("/install/{name}.wasm")),
            metadata_path: if with_meta {
                Some(std::path::PathBuf::from(format!(
                    "/install/{name}.capabilities.json"
                )))
            } else {
                None
            },
        }
    }

    #[test]
    fn search_schema_requires_query() {
        let tool = IronhubSearchTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"], serde_json::json!(["query"]));
    }

    #[test]
    fn list_schema_has_no_required_fields() {
        let tool = IronhubListTool::new();
        let schema = tool.parameters_schema();
        assert!(
            schema.get("required").is_none() || schema["required"].as_array().unwrap().is_empty()
        );
    }

    #[test]
    fn info_schema_requires_name() {
        let tool = IronhubInfoTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"], serde_json::json!(["name"]));
    }

    #[test]
    fn read_only_tools_default_to_never_approval() {
        let params = serde_json::json!({});
        assert!(matches!(
            IronhubSearchTool::new().requires_approval(&params),
            ApprovalRequirement::Never
        ));
        assert!(matches!(
            IronhubListTool::new().requires_approval(&params),
            ApprovalRequirement::Never
        ));
        assert!(matches!(
            IronhubInfoTool::new().requires_approval(&params),
            ApprovalRequirement::Never
        ));
    }

    #[test]
    fn classify_picks_tool_when_only_in_tools() {
        let m = manifest_with(vec!["clickup"], vec!["chief-of-staff"]);
        assert_eq!(classify(&m, "clickup", None).unwrap(), HubEntryKind::Tool);
    }

    #[test]
    fn classify_picks_skill_when_only_in_skills() {
        let m = manifest_with(vec!["clickup"], vec!["chief-of-staff"]);
        assert_eq!(
            classify(&m, "chief-of-staff", None).unwrap(),
            HubEntryKind::Skill
        );
    }

    #[test]
    fn classify_returns_invalid_parameters_for_ambiguous() {
        let m = manifest_with(vec!["overlap"], vec!["overlap"]);
        let err = classify(&m, "overlap", None).expect_err("must error");
        match err {
            ToolError::InvalidParameters(msg) => assert!(msg.contains("disambiguate")),
            other => panic!("expected InvalidParameters, got {other:?}"),
        }
    }

    #[test]
    fn classify_honors_kind_tool_override() {
        let m = manifest_with(vec!["overlap"], vec!["overlap"]);
        assert_eq!(
            classify(&m, "overlap", Some(HubEntryKind::Tool)).unwrap(),
            HubEntryKind::Tool
        );
    }

    #[test]
    fn classify_honors_kind_skill_override() {
        let m = manifest_with(vec!["overlap"], vec!["overlap"]);
        assert_eq!(
            classify(&m, "overlap", Some(HubEntryKind::Skill)).unwrap(),
            HubEntryKind::Skill
        );
    }

    #[test]
    fn classify_kind_tool_rejects_skill_only_name() {
        let m = manifest_with(vec![], vec!["chief-of-staff"]);
        let err = classify(&m, "chief-of-staff", Some(HubEntryKind::Tool)).expect_err("must error");
        match err {
            ToolError::InvalidParameters(msg) => assert!(msg.contains("not a tool")),
            other => panic!("expected InvalidParameters, got {other:?}"),
        }
    }

    #[test]
    fn classify_returns_invalid_parameters_with_suggestions_for_typos() {
        let m = manifest_with(vec!["clickup", "evm-rpc"], vec![]);
        let err = classify(&m, "click", None).expect_err("must error");
        match err {
            ToolError::InvalidParameters(msg) => {
                assert!(msg.contains("Did you mean"));
                assert!(msg.contains("clickup"));
            }
            other => panic!("expected InvalidParameters, got {other:?}"),
        }
    }

    #[test]
    fn kind_param_rejects_invalid_string() {
        let err = HubEntryKind::from_param("channel").expect_err("must error");
        match err {
            ToolError::InvalidParameters(msg) => assert!(msg.contains("kind must be")),
            other => panic!("expected InvalidParameters, got {other:?}"),
        }
    }

    #[test]
    fn kind_param_accepts_tool_and_skill() {
        assert_eq!(
            HubEntryKind::from_param("tool").unwrap(),
            HubEntryKind::Tool
        );
        assert_eq!(
            HubEntryKind::from_param("skill").unwrap(),
            HubEntryKind::Skill
        );
    }

    #[test]
    fn install_outcome_to_json_includes_required_fields() {
        let json = install_outcome_to_json(HubEntryKind::Tool, &outcome("clickup", true));
        assert_eq!(json["status"], "installed");
        assert_eq!(json["kind"], "tool");
        assert_eq!(json["name"], "clickup");
        assert_eq!(json["version"], "0.1.0");
        assert_eq!(json["release_tag"], "release-test");
        assert!(
            json["primary_path"]
                .as_str()
                .unwrap()
                .contains("clickup.wasm")
        );
        assert!(json["metadata_path"].as_str().is_some());
    }

    #[test]
    fn install_outcome_to_json_omits_metadata_path_when_none() {
        let json = install_outcome_to_json(HubEntryKind::Skill, &outcome("chief-of-staff", false));
        assert_eq!(json["kind"], "skill");
        assert!(json.get("metadata_path").is_none());
    }

    #[test]
    fn install_outcome_to_json_official_has_provenance_no_warning() {
        let json = install_outcome_to_json(HubEntryKind::Tool, &outcome("clickup", true));
        assert_eq!(json["provenance"], "official");
        assert!(json.get("warning").is_none());
        assert!(json.get("unverified").is_none());
    }

    #[test]
    fn install_outcome_to_json_new_provenance_warns_and_flags_unverified() {
        let json = install_outcome_to_json(
            HubEntryKind::Skill,
            &outcome_prov("indie-skill", false, Provenance::New),
        );
        assert_eq!(json["provenance"], "new");
        assert_eq!(json["unverified"], true);
        assert!(
            json["warning"]
                .as_str()
                .unwrap()
                .contains("not NEAR-vetted")
        );
    }

    #[test]
    fn tool_entry_json_surfaces_provenance_and_trust_label() {
        let entry = HubToolEntry {
            name: "indie-tool".into(),
            crate_name: "indie-tool".into(),
            version: "0.1.0".into(),
            description: "Community tool".into(),
            provenance: Provenance::New,
            wasm: art("indie-tool", "wasm"),
            capabilities: art("indie-tool", "capabilities.json"),
        };
        let json = tool_entry_json(&entry);
        assert_eq!(json["provenance"], "new");
        assert_eq!(json["trust_label"], Provenance::New.trust_label());
        assert_eq!(json["name"], "indie-tool");
        assert_eq!(json["kind"], "tool");
    }

    #[test]
    fn skill_entry_json_surfaces_provenance_and_trust_label() {
        let entry = HubSkillEntry {
            name: "indie-skill".into(),
            trunk: String::new(),
            version: "0.1.0".into(),
            description: "Community skill".into(),
            provenance: Provenance::Verified,
            skill_md: art("indie-skill", "SKILL.md"),
        };
        let json = skill_entry_json(&entry);
        assert_eq!(json["provenance"], "verified");
        assert_eq!(json["trust_label"], Provenance::Verified.trust_label());
        assert_eq!(json["name"], "indie-skill");
        assert_eq!(json["kind"], "skill");
    }

    fn manifest_with_provenance(
        tool_name: &str,
        tool_provenance: Provenance,
        skill_name: Option<&str>,
        skill_provenance: Option<Provenance>,
    ) -> HubManifest {
        let mut manifest = manifest_with(vec![tool_name], skill_name.into_iter().collect());
        if let Some(tool) = manifest.tools.first_mut() {
            tool.provenance = tool_provenance;
        }
        if let (Some(skill), Some(prov)) = (manifest.skills.first_mut(), skill_provenance) {
            skill.provenance = prov;
        }
        manifest
    }

    #[test]
    fn classify_and_gate_rejects_community_unverified_without_acknowledgement() {
        let manifest = manifest_with_provenance("indie-tool", Provenance::New, None, None);
        let err = classify_and_gate(&manifest, "indie-tool", None, false)
            .expect_err("community-unverified without ack must be rejected");
        match err {
            ToolError::InvalidParameters(msg) => {
                assert!(
                    msg.contains("UNVERIFIED") && msg.contains("acknowledge_unverified"),
                    "error must name the gate: {msg}"
                );
            }
            other => panic!("expected InvalidParameters, got {other:?}"),
        }
    }

    #[test]
    fn classify_and_gate_accepts_community_unverified_with_acknowledgement() {
        let manifest = manifest_with_provenance("indie-tool", Provenance::New, None, None);
        let (kind, provenance) = classify_and_gate(&manifest, "indie-tool", None, true)
            .expect("community-unverified with ack must pass the gate");
        assert_eq!(kind, HubEntryKind::Tool);
        assert_eq!(provenance, Provenance::New);
    }

    #[test]
    fn classify_and_gate_accepts_official_without_acknowledgement() {
        let manifest = manifest_with_provenance("clickup", Provenance::Official, None, None);
        let (kind, provenance) = classify_and_gate(&manifest, "clickup", None, false)
            .expect("official content must pass the gate without ack");
        assert_eq!(kind, HubEntryKind::Tool);
        assert_eq!(provenance, Provenance::Official);
    }

    #[test]
    fn classify_and_gate_rejects_community_unverified_skill_without_acknowledgement() {
        let manifest = manifest_with_provenance(
            "official-tool",
            Provenance::Official,
            Some("indie-skill"),
            Some(Provenance::New),
        );
        let err = classify_and_gate(&manifest, "indie-skill", None, false)
            .expect_err("community-unverified skill must be gated too");
        match err {
            ToolError::InvalidParameters(msg) => assert!(msg.contains("UNVERIFIED")),
            other => panic!("expected InvalidParameters, got {other:?}"),
        }
    }

    fn install_tool_with_ext_mgr() -> (IronhubInstallTool, tempfile::TempDir, tempfile::TempDir) {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let (ext_mgr, tools_dir, channels_dir) =
            crate::channels::web::test_helpers::test_ext_mgr(secrets);
        let tool = IronhubInstallTool::new(IronhubDeps {
            extension_manager: ext_mgr,
            skill_registry: None,
        });
        (tool, tools_dir, channels_dir)
    }

    #[tokio::test]
    async fn install_from_manifest_rejects_provenance_new_without_acknowledgement() {
        let (tool, _tools_dir, _channels_dir) = install_tool_with_ext_mgr();
        let manifest = manifest_with_provenance("indie-tool", Provenance::New, None, None);
        let parsed = InstallParams {
            name: "indie-tool".into(),
            kind_hint: None,
            release_tag: None,
            force: false,
            acknowledge_unverified: false,
        };
        let ctx = JobContext::with_user("test", "install gate test", "");
        let err = tool
            .install_from_manifest(std::time::Instant::now(), manifest, parsed, &ctx)
            .await
            .expect_err(
                "Provenance::New without ack must be rejected at the execute caller boundary",
            );
        match err {
            ToolError::InvalidParameters(msg) => {
                assert!(
                    msg.contains("UNVERIFIED") && msg.contains("acknowledge_unverified"),
                    "caller-level gate error must name the flag the user has to set, got: {msg}"
                );
            }
            other => panic!(
                "execute caller must surface the gate as InvalidParameters; got {other:?} \
                 (a different error type means the gate fired downstream of the install side effect)"
            ),
        }
    }

    #[tokio::test]
    async fn install_params_from_json_propagates_acknowledge_unverified() {
        let parsed = InstallParams::from_json(&serde_json::json!({
            "name": "indie-tool",
            "acknowledge_unverified": true,
        }))
        .expect("valid params");
        assert!(
            parsed.acknowledge_unverified,
            "ack flag must reach the install caller; without this, the UI ack flow is a no-op"
        );
        let default = InstallParams::from_json(&serde_json::json!({"name": "indie-tool"}))
            .expect("valid params");
        assert!(
            !default.acknowledge_unverified,
            "omitted ack must default to false so community content stays gated"
        );
    }

    #[test]
    fn catalog_unavailable_is_user_safe_external_service() {
        let err = catalog_unavailable();
        assert!(matches!(err, ToolError::ExternalService(_)), "got {err:?}");
        let msg = err.to_string();
        assert!(!msg.contains("http"));
        assert!(!msg.contains("hub.ironclaw.com"));
        assert!(!msg.contains('/'));
    }

    #[test]
    fn entry_matches_lowercases_name_against_already_lowercased_query() {
        assert!(entry_matches("ClickUp", "Task tracking", "clickup"));
        assert!(entry_matches("clickup", "Task tracking", "click"));
        assert!(!entry_matches("clickup", "Task tracking", "CLICK"));
    }

    #[test]
    fn entry_matches_searches_description() {
        assert!(entry_matches(
            "evm-rpc",
            "Ethereum RPC bindings",
            "ethereum"
        ));
        assert!(!entry_matches("evm-rpc", "Ethereum RPC bindings", "solana"));
    }

    #[test]
    fn nearest_matches_filters_by_substring_both_directions() {
        let m = manifest_with(vec!["clickup", "evm-rpc", "near-rpc"], vec![]);
        let hits = nearest_matches(&m, "rpc");
        assert!(hits.contains(&"evm-rpc".to_string()));
        assert!(hits.contains(&"near-rpc".to_string()));
        assert!(!hits.contains(&"clickup".to_string()));
    }

    #[test]
    fn install_schema_pattern_blocks_path_traversal() {
        let tool_schema = IronhubSearchTool::new().parameters_schema();
        let pattern = tool_schema["properties"]["release_tag"]["pattern"]
            .as_str()
            .expect("release_tag pattern");
        let re = regex::Regex::new(pattern).expect("valid regex");
        assert!(!re.is_match("../etc/passwd"));
        assert!(!re.is_match("release with space"));
        assert!(!re.is_match("release\nnewline"));
        assert!(re.is_match("release-2026-05-12-24"));
    }

    #[test]
    fn install_schema_declares_required_name() {
        let schema = IronhubInfoTool::new().parameters_schema();
        assert_eq!(schema["required"], serde_json::json!(["name"]));
        let name_pattern = schema["properties"]["name"]["pattern"]
            .as_str()
            .expect("name pattern");
        let re = regex::Regex::new(name_pattern).expect("valid regex");
        assert!(re.is_match("clickup"));
        assert!(re.is_match("chief-of-staff"));
        assert!(!re.is_match("../etc"));
        assert!(!re.is_match("Name"));
        assert!(!re.is_match(""));
    }

    #[test]
    fn schemas_reject_unknown_fields() {
        for schema in [
            IronhubSearchTool::new().parameters_schema(),
            IronhubListTool::new().parameters_schema(),
            IronhubInfoTool::new().parameters_schema(),
        ] {
            assert_eq!(
                schema["additionalProperties"],
                serde_json::Value::Bool(false),
                "additionalProperties: false required to reject LLM injection of unknown fields"
            );
        }
    }

    async fn dispatcher_with(tool: Arc<dyn Tool>) -> Arc<crate::tools::dispatch::ToolDispatcher> {
        use crate::config::SafetyConfig;
        use crate::db::Database;
        use crate::db::UserRecord;
        use crate::db::libsql::LibSqlBackend;
        use crate::tools::dispatch::ToolDispatcher;
        use crate::tools::registry::ToolRegistry;
        use ironclaw_safety::SafetyLayer;

        let dir = tempfile::tempdir().expect("tempdir");
        let backend = Arc::new(
            LibSqlBackend::new_local(&dir.path().join("test.db"))
                .await
                .expect("libsql backend"),
        );
        backend.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::clone(&backend) as Arc<dyn Database>;
        let now = chrono::Utc::now();
        db.create_user(&UserRecord {
            id: "tester".to_string(),
            email: None,
            display_name: "tester".to_string(),
            status: "active".to_string(),
            role: "admin".to_string(),
            created_at: now,
            updated_at: now,
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        })
        .await
        .expect("create user");

        let registry = Arc::new(ToolRegistry::new());
        registry.register(tool).await;
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 65_536,
            injection_check_enabled: false,
        }));
        std::mem::forget(dir);
        Arc::new(ToolDispatcher::new(registry, safety, db))
    }

    #[tokio::test]
    async fn dispatch_ironhub_search_rejects_empty_query() {
        let dispatcher = dispatcher_with(Arc::new(IronhubSearchTool::new())).await;
        let err = dispatcher
            .dispatch(
                "ironhub_search",
                serde_json::json!({ "query": "" }),
                "tester",
                crate::tools::dispatch::DispatchSource::Channel("gateway".into()),
            )
            .await
            .expect_err("empty query must fail schema validation");
        assert!(
            matches!(err, ToolError::InvalidParameters(_)),
            "expected InvalidParameters, got {err:?}"
        );
    }

    #[tokio::test]
    async fn dispatch_ironhub_search_rejects_unknown_field() {
        let dispatcher = dispatcher_with(Arc::new(IronhubSearchTool::new())).await;
        let err = dispatcher
            .dispatch(
                "ironhub_search",
                serde_json::json!({ "query": "rpc", "evil_extra_field": "exfil" }),
                "tester",
                crate::tools::dispatch::DispatchSource::Channel("gateway".into()),
            )
            .await
            .expect_err("unknown field must fail schema validation");
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn dispatch_ironhub_info_rejects_path_traversal_in_name() {
        let dispatcher = dispatcher_with(Arc::new(IronhubInfoTool::new())).await;
        let err = dispatcher
            .dispatch(
                "ironhub_info",
                serde_json::json!({ "name": "../etc/passwd" }),
                "tester",
                crate::tools::dispatch::DispatchSource::Channel("gateway".into()),
            )
            .await
            .expect_err("path traversal must fail schema validation");
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn dispatch_ironhub_info_rejects_missing_required_name() {
        let dispatcher = dispatcher_with(Arc::new(IronhubInfoTool::new())).await;
        let err = dispatcher
            .dispatch(
                "ironhub_info",
                serde_json::json!({}),
                "tester",
                crate::tools::dispatch::DispatchSource::Channel("gateway".into()),
            )
            .await
            .expect_err("missing required name must fail schema validation");
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn dispatch_ironhub_info_rejects_malformed_release_tag() {
        let dispatcher = dispatcher_with(Arc::new(IronhubInfoTool::new())).await;
        let err = dispatcher
            .dispatch(
                "ironhub_info",
                serde_json::json!({ "name": "clickup", "release_tag": "release with space" }),
                "tester",
                crate::tools::dispatch::DispatchSource::Channel("gateway".into()),
            )
            .await
            .expect_err("bad release_tag must fail schema validation");
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn skill_install_dir_prefers_installed_dir_over_user_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let installed = tmp.path().join("installed");
        let user = tmp.path().join("user");
        std::fs::create_dir_all(&installed).expect("mkdir installed");
        std::fs::create_dir_all(&user).expect("mkdir user");
        let mut registry = SkillRegistry::new(user.clone())
            .with_installed_dir(installed.clone())
            .with_max_scan_depth(2);
        registry.discover_all().await;
        let registry = Arc::new(std::sync::RwLock::new(registry));

        let resolved = skill_install_dir(&Some(registry.clone())).expect("resolved");
        assert_eq!(
            resolved, installed,
            "IronHub skill installs must land in the Installed bucket, not the Trusted user_dir"
        );
        assert_ne!(
            resolved, user,
            "regression: skill_install_dir must not return user_dir"
        );
    }

    fn write_fake_tool_on_disk(tools_dir: &std::path::Path, name: &str) {
        std::fs::create_dir_all(tools_dir).expect("tools dir");
        std::fs::write(tools_dir.join(format!("{name}.wasm")), b"not-a-real-wasm")
            .expect("write wasm stub");
        std::fs::write(
            tools_dir.join(format!("{name}.capabilities.json")),
            r#"{"description":"test stub"}"#,
        )
        .expect("write capabilities stub");
    }

    fn remove_tool_with_ext_mgr(ext_mgr: Arc<ExtensionManager>) -> IronhubRemoveTool {
        IronhubRemoveTool::new(IronhubDeps {
            extension_manager: ext_mgr,
            skill_registry: None,
        })
    }

    #[tokio::test]
    async fn ironhub_remove_execute_returns_removed_status_and_deletes_files() {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let (ext_mgr, tools_dir, _channels_dir) =
            crate::channels::web::test_helpers::test_ext_mgr(secrets);
        write_fake_tool_on_disk(tools_dir.path(), "test_remove_target");

        let tool = remove_tool_with_ext_mgr(Arc::clone(&ext_mgr));
        let ctx = JobContext::with_user("test", "remove test", "");

        let output = tool
            .execute(serde_json::json!({ "name": "test_remove_target" }), &ctx)
            .await
            .expect("remove of installed tool must succeed");

        assert_eq!(output.result["status"], "removed");
        assert_eq!(output.result["name"], "test_remove_target");
        assert!(
            output.result["message"].as_str().is_some(),
            "message field must be present so the agent surfaces the manager's report"
        );
        assert!(
            !tools_dir.path().join("test_remove_target.wasm").exists(),
            "remove must delete the .wasm artifact from disk"
        );
        assert!(
            !tools_dir
                .path()
                .join("test_remove_target.capabilities.json")
                .exists(),
            "remove must delete the .capabilities.json artifact from disk"
        );
    }

    #[tokio::test]
    async fn ironhub_remove_execute_rejects_invalid_name_before_touching_manager() {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let (ext_mgr, _tools_dir, _channels_dir) =
            crate::channels::web::test_helpers::test_ext_mgr(secrets);
        let tool = remove_tool_with_ext_mgr(ext_mgr);
        let ctx = JobContext::with_user("test", "remove test", "");

        let err = tool
            .execute(
                serde_json::json!({ "name": "Invalid Name With Spaces" }),
                &ctx,
            )
            .await
            .expect_err("malformed name must be rejected by validate_hub_name");

        match err {
            ToolError::InvalidParameters(msg) => assert!(
                msg.contains("not a valid IronHub name"),
                "validator message must surface to the caller, got: {msg}"
            ),
            other => panic!("expected InvalidParameters, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ironhub_remove_execute_maps_not_installed_to_actionable_error() {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let (ext_mgr, _tools_dir, _channels_dir) =
            crate::channels::web::test_helpers::test_ext_mgr(secrets);
        let tool = remove_tool_with_ext_mgr(ext_mgr);
        let ctx = JobContext::with_user("test", "remove test", "");

        let err = tool
            .execute(serde_json::json!({ "name": "never_installed_tool" }), &ctx)
            .await
            .expect_err("removing a tool that is not installed must surface as an error");

        match err {
            ToolError::InvalidParameters(msg) => {
                assert!(
                    msg.contains("not an installed IronHub tool"),
                    "error must point the caller at the right surface, got: {msg}"
                );
                assert!(
                    msg.contains("skill_remove"),
                    "error must hint at skill_remove for the wrong-tool case, got: {msg}"
                );
            }
            other => panic!("expected InvalidParameters, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ironhub_remove_execute_passes_post_remove_list_verification() {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let (ext_mgr, tools_dir, _channels_dir) =
            crate::channels::web::test_helpers::test_ext_mgr(secrets);
        write_fake_tool_on_disk(tools_dir.path(), "verifier_target");

        let pre_remove = ext_mgr
            .list(Some(ExtensionKind::WasmTool), false, "test")
            .await
            .expect("pre-remove list must succeed");
        assert!(
            pre_remove
                .iter()
                .any(|e| e.name.eq_ignore_ascii_case("verifier_target")),
            "fixture must seed verifier_target before remove runs"
        );

        let tool = remove_tool_with_ext_mgr(Arc::clone(&ext_mgr));
        let ctx = JobContext::with_user("test", "remove test", "");
        tool.execute(serde_json::json!({ "name": "verifier_target" }), &ctx)
            .await
            .expect("remove must succeed when the post-remove list is empty");

        let post_remove = ext_mgr
            .list(Some(ExtensionKind::WasmTool), false, "test")
            .await
            .expect("post-remove list must succeed");
        assert!(
            !post_remove
                .iter()
                .any(|e| e.name.eq_ignore_ascii_case("verifier_target")),
            "post-remove list must be empty so the still-present guard stays silent"
        );
    }

    #[tokio::test]
    async fn ironhub_remove_execute_surfaces_still_present_when_orphan_remains() {
        let secrets = crate::channels::web::test_helpers::test_secrets_store();
        let (ext_mgr, tools_dir, _channels_dir) =
            crate::channels::web::test_helpers::test_ext_mgr(secrets);
        write_fake_tool_on_disk(tools_dir.path(), "stuck_tool");
        std::fs::write(
            tools_dir.path().join("STUCK_TOOL.wasm"),
            b"orphan-uppercase-twin",
        )
        .expect("write orphan twin");

        let tool = remove_tool_with_ext_mgr(Arc::clone(&ext_mgr));
        let ctx = JobContext::with_user("test", "remove test", "");
        let err = tool
            .execute(serde_json::json!({ "name": "stuck_tool" }), &ctx)
            .await
            .expect_err("orphan twin under a different case must trip the still-present guard");

        match err {
            ToolError::ExecutionFailed(msg) => {
                assert!(
                    msg.contains("still present after removal"),
                    "still-present guard must surface a clear message, got: {msg}"
                );
                assert!(
                    msg.contains("stuck_tool"),
                    "error must name the offending tool, got: {msg}"
                );
            }
            other => panic!(
                "still-present must surface as ExecutionFailed, not the manager's not-installed mapping; got {other:?}"
            ),
        }
    }
}
