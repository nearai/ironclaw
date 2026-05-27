//! Deterministic instruction/context bundle assembly for loop prompt ports.
//!
//! This module owns the host-side assembly step between a scoped loop context
//! snapshot and model-message refs. It does not fetch memory, skills, secrets,
//! capabilities, or provider data directly; callers pass already host-approved
//! context/services output in [`InstructionBundleRequest`].

use std::{collections::HashMap, sync::Mutex};

use sha2::{Digest, Sha256};

use crate::LoopMessageRef;

use super::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityDescriptorView, LoopContextBundle,
    LoopContextMessage, LoopContextSnippet, LoopInlineMessage, LoopInlineMessageRole,
    LoopModelMessage, LoopRunContext, PromptSkillContextMetadata, VisibleCapabilitySurface,
    skill_snippet_model_message_ref,
};

/// Stable fingerprint for an instruction bundle rebuild.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstructionBundleFingerprint(String);

impl InstructionBundleFingerprint {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err("instruction bundle fingerprint must start with sha256:".to_string());
        };
        if hex.len() != 64 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(
                "instruction bundle fingerprint must contain a SHA-256 hex digest".to_string(),
            );
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for InstructionBundleFingerprint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl serde::Serialize for InstructionBundleFingerprint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for InstructionBundleFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Model-safe safety policy context to include in prompt construction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstructionSafetyContext {
    pub policy_ref: String,
    pub safe_summary: String,
}

impl InstructionSafetyContext {
    pub fn new(
        policy_ref: impl Into<String>,
        safe_summary: impl Into<String>,
    ) -> Result<Self, AgentLoopHostError> {
        let policy_ref = validate_context_ref(policy_ref.into(), "safety policy ref")?;
        let safe_summary = validate_model_safe_text(safe_summary.into(), "safety policy summary")?;
        Ok(Self {
            policy_ref,
            safe_summary,
        })
    }
}

/// Inputs for a deterministic instruction bundle build.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstructionBundleRequest {
    pub context_bundle: LoopContextBundle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_surface: Option<VisibleCapabilitySurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_context: Option<InstructionSafetyContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_messages: Vec<LoopInlineMessage>,
}

/// Host-built instruction bundle materialized in memory for model-port resolution.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstructionBundleMaterializedMessage {
    pub role: String,
    pub content_ref: LoopMessageRef,
    pub safe_content: String,
}

/// Scoped store for host-owned prompt refs that are not durable transcript refs.
pub trait InstructionMaterializationStore: Send + Sync {
    fn put_materialized_messages(
        &self,
        context: &LoopRunContext,
        messages: Vec<InstructionBundleMaterializedMessage>,
    ) -> Result<(), AgentLoopHostError>;

    fn get_materialized_message(
        &self,
        context: &LoopRunContext,
        content_ref: &LoopMessageRef,
    ) -> Result<Option<InstructionBundleMaterializedMessage>, AgentLoopHostError>;
}

/// In-memory, per-process materialization store for model-visible safe context.
#[derive(Default)]
pub struct InMemoryInstructionMaterializationStore {
    messages: Mutex<HashMap<String, InstructionBundleMaterializedMessage>>,
}

impl InMemoryInstructionMaterializationStore {
    fn key(context: &LoopRunContext, content_ref: &LoopMessageRef) -> String {
        format!("{}:{}", context.run_id, content_ref.as_str())
    }
}

impl InstructionMaterializationStore for InMemoryInstructionMaterializationStore {
    fn put_materialized_messages(
        &self,
        context: &LoopRunContext,
        messages: Vec<InstructionBundleMaterializedMessage>,
    ) -> Result<(), AgentLoopHostError> {
        let mut stored = self.messages.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "instruction materialization store is unavailable",
            )
        })?;
        for message in messages {
            stored.insert(Self::key(context, &message.content_ref), message);
        }
        Ok(())
    }

    fn get_materialized_message(
        &self,
        context: &LoopRunContext,
        content_ref: &LoopMessageRef,
    ) -> Result<Option<InstructionBundleMaterializedMessage>, AgentLoopHostError> {
        self.messages
            .lock()
            .map(|messages| messages.get(&Self::key(context, content_ref)).cloned())
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "instruction materialization store is unavailable",
                )
            })
    }
}

/// Host-built instruction bundle suitable for model invocation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstructionBundle {
    pub fingerprint: InstructionBundleFingerprint,
    pub messages: Vec<LoopModelMessage>,
    #[serde(default, skip)]
    pub materialized_messages: Vec<InstructionBundleMaterializedMessage>,
    #[serde(default, skip)]
    pub requires_materialization_store: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skill_context: Vec<PromptSkillContextMetadata>,
}

/// Deterministic host-owned instruction bundle builder.
#[derive(Debug, Clone)]
pub struct InstructionBundleBuilder {
    context: LoopRunContext,
}

impl InstructionBundleBuilder {
    pub fn new(context: LoopRunContext) -> Self {
        Self { context }
    }

    pub fn build(
        &self,
        request: InstructionBundleRequest,
    ) -> Result<InstructionBundle, AgentLoopHostError> {
        let mut messages = Vec::new();
        let mut materialized_messages = Vec::new();
        let mut skill_context = Vec::new();
        let mut requires_materialization_store = false;
        let mut synthetic_refs = SyntheticMessageRefRegistry::default();
        let mut fingerprint = Sha256::new();

        feed_field(
            &mut fingerprint,
            b"run",
            self.context.run_id.to_string().as_bytes(),
        );
        feed_field(
            &mut fingerprint,
            b"profile",
            self.context
                .resolved_run_profile
                .profile_id
                .as_str()
                .as_bytes(),
        );

        if !request.inline_messages.is_empty() {
            requires_materialization_store = true;
        }
        for (ordinal, message) in request.inline_messages.into_iter().enumerate() {
            push_inline_message(
                &mut messages,
                &mut materialized_messages,
                &mut fingerprint,
                ordinal,
                message,
                &mut synthetic_refs,
            )?;
        }

        if !request.context_bundle.identity_messages.is_empty() {
            requires_materialization_store = true;
        }
        for (ordinal, message) in request
            .context_bundle
            .identity_messages
            .into_iter()
            .enumerate()
        {
            push_context_message(
                &mut messages,
                &mut materialized_messages,
                &mut fingerprint,
                ContextMessageOptions {
                    section: "identity",
                    ordinal,
                    force_materialize: true,
                },
                &mut synthetic_refs,
                message,
            )?;
        }

        let mut instruction_snippets = request.context_bundle.instruction_snippets;
        sort_instruction_snippets_for_prompt(&mut instruction_snippets);
        let mut skill_ordinal = 0usize;
        for snippet in instruction_snippets {
            if snippet.snippet_ref.starts_with("skill:") {
                let content_ref = skill_snippet_model_message_ref(
                    &snippet.snippet_ref,
                    &snippet.safe_summary,
                    skill_ordinal,
                )?;
                let Some(metadata) = snippet.metadata.as_ref() else {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "skill instruction snippet metadata is missing",
                    ));
                };
                push_snippet_message(
                    &mut messages,
                    &mut materialized_messages,
                    &mut fingerprint,
                    "skill",
                    skill_ordinal,
                    content_ref,
                    &snippet,
                )?;
                skill_context.push(PromptSkillContextMetadata {
                    ordinal: skill_ordinal,
                    source_name: metadata.source_name.clone(),
                    trust_level: metadata.trust_level.clone(),
                });
                skill_ordinal += 1;
            } else {
                requires_materialization_store = true;
                let ordinal = messages.len();
                let content_ref =
                    snippet_message_ref("instruction", &snippet, ordinal, &mut synthetic_refs)?;
                push_snippet_message(
                    &mut messages,
                    &mut materialized_messages,
                    &mut fingerprint,
                    "instruction",
                    ordinal,
                    content_ref,
                    &snippet,
                )?;
            }
        }

        let mut memory_snippets = request.context_bundle.memory_snippets;
        if !memory_snippets.is_empty() {
            requires_materialization_store = true;
        }
        memory_snippets.sort_by(compare_snippet_refs);
        for (ordinal, snippet) in memory_snippets.into_iter().enumerate() {
            let content_ref =
                snippet_message_ref("memory", &snippet, ordinal, &mut synthetic_refs)?;
            push_snippet_message(
                &mut messages,
                &mut materialized_messages,
                &mut fingerprint,
                "memory",
                ordinal,
                content_ref,
                &snippet,
            )?;
        }

        if let Some(safety_context) = request.safety_context {
            requires_materialization_store = true;
            push_safety_context(
                &mut messages,
                &mut materialized_messages,
                &mut fingerprint,
                safety_context,
                &mut synthetic_refs,
            )?;
        }

        if let Some(surface) = request
            .visible_surface
            .filter(|surface| !surface.descriptors.is_empty())
        {
            requires_materialization_store = true;
            push_visible_surface(
                &mut messages,
                &mut materialized_messages,
                &mut fingerprint,
                surface,
                &mut synthetic_refs,
            )?;
        }

        for (ordinal, message) in request.context_bundle.messages.into_iter().enumerate() {
            requires_materialization_store |= push_context_message(
                &mut messages,
                &mut materialized_messages,
                &mut fingerprint,
                ContextMessageOptions {
                    section: "thread",
                    ordinal,
                    force_materialize: false,
                },
                &mut synthetic_refs,
                message,
            )?;
        }

        let fingerprint = InstructionBundleFingerprint::new(format!(
            "sha256:{}",
            hex::encode(fingerprint.finalize())
        ))
        .map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "instruction bundle fingerprint could not be represented",
            )
        })?;

        Ok(InstructionBundle {
            fingerprint,
            messages,
            materialized_messages,
            requires_materialization_store,
            skill_context,
        })
    }
}

struct ContextMessageOptions {
    section: &'static str,
    ordinal: usize,
    force_materialize: bool,
}

fn push_context_message(
    messages: &mut Vec<LoopModelMessage>,
    materialized_messages: &mut Vec<InstructionBundleMaterializedMessage>,
    fingerprint: &mut Sha256,
    options: ContextMessageOptions,
    synthetic_refs: &mut SyntheticMessageRefRegistry,
    message: LoopContextMessage,
) -> Result<bool, AgentLoopHostError> {
    let safe_summary = validate_model_safe_text(message.safe_summary, "context message summary")?;
    validate_model_role(&message.role)?;
    let source_ref = message.message_ref;
    let (content_ref, summary_only) = match source_ref {
        Some(content_ref) => {
            feed_field(fingerprint, b"ref", content_ref.as_str().as_bytes());
            (content_ref, false)
        }
        None => {
            let summary_section = if options.section == "identity" {
                "identity-summary"
            } else {
                "context-summary"
            };
            let content_ref = synthetic_message_ref(
                summary_section,
                &message.role,
                &safe_summary,
                options.ordinal,
                synthetic_refs,
            )?;
            feed_field(fingerprint, b"ref", content_ref.as_str().as_bytes());
            feed_field(fingerprint, b"summary", safe_summary.as_bytes());
            (content_ref, true)
        }
    };
    feed_field(fingerprint, b"section", options.section.as_bytes());
    feed_field(fingerprint, b"role", message.role.as_bytes());
    if options.force_materialize || summary_only {
        materialized_messages.push(InstructionBundleMaterializedMessage {
            role: message.role.clone(),
            content_ref: content_ref.clone(),
            safe_content: safe_summary,
        });
    }
    messages.push(LoopModelMessage {
        role: message.role,
        content_ref,
    });
    Ok(summary_only)
}

fn push_snippet_message(
    messages: &mut Vec<LoopModelMessage>,
    materialized_messages: &mut Vec<InstructionBundleMaterializedMessage>,
    fingerprint: &mut Sha256,
    section: &'static str,
    ordinal: usize,
    content_ref: LoopMessageRef,
    snippet: &LoopContextSnippet,
) -> Result<(), AgentLoopHostError> {
    validate_context_ref(snippet.snippet_ref.clone(), "context snippet ref")?;
    let safe_summary =
        validate_model_safe_text(snippet.safe_summary.clone(), "context snippet summary").map_err(
            |error| {
                tracing::debug!(
                    section,
                    ordinal,
                    snippet_ref = %snippet.snippet_ref,
                    content_ref = %content_ref.as_str(),
                    safe_summary_bytes = snippet.safe_summary.len(),
                    error_kind = ?error.kind,
                    error_safe_summary = %error.safe_summary,
                    "instruction bundle rejected context snippet safe summary"
                );
                error
            },
        )?;
    feed_field(fingerprint, b"section", section.as_bytes());
    feed_field(fingerprint, b"ref", content_ref.as_str().as_bytes());
    feed_field(fingerprint, b"source", snippet.snippet_ref.as_bytes());
    feed_field(fingerprint, b"summary", snippet.safe_summary.as_bytes());
    materialized_messages.push(InstructionBundleMaterializedMessage {
        role: "system".to_string(),
        content_ref: content_ref.clone(),
        safe_content: safe_summary,
    });
    messages.push(LoopModelMessage {
        role: "system".to_string(),
        content_ref,
    });
    Ok(())
}

fn push_safety_context(
    messages: &mut Vec<LoopModelMessage>,
    materialized_messages: &mut Vec<InstructionBundleMaterializedMessage>,
    fingerprint: &mut Sha256,
    safety_context: InstructionSafetyContext,
    synthetic_refs: &mut SyntheticMessageRefRegistry,
) -> Result<(), AgentLoopHostError> {
    let content_ref = synthetic_message_ref(
        "safety",
        &safety_context.policy_ref,
        &safety_context.safe_summary,
        0,
        synthetic_refs,
    )?;
    feed_field(fingerprint, b"section", b"safety");
    feed_field(fingerprint, b"ref", content_ref.as_str().as_bytes());
    feed_field(fingerprint, b"source", safety_context.policy_ref.as_bytes());
    feed_field(
        fingerprint,
        b"summary",
        safety_context.safe_summary.as_bytes(),
    );
    materialized_messages.push(InstructionBundleMaterializedMessage {
        role: "system".to_string(),
        content_ref: content_ref.clone(),
        safe_content: safety_context.safe_summary,
    });
    messages.push(LoopModelMessage {
        role: "system".to_string(),
        content_ref,
    });
    Ok(())
}

fn push_inline_message(
    messages: &mut Vec<LoopModelMessage>,
    materialized_messages: &mut Vec<InstructionBundleMaterializedMessage>,
    fingerprint: &mut Sha256,
    ordinal: usize,
    message: LoopInlineMessage,
    synthetic_refs: &mut SyntheticMessageRefRegistry,
) -> Result<(), AgentLoopHostError> {
    let role = inline_role(message.role).to_string();
    let safe_body =
        validate_model_safe_text(message.safe_body.as_str().to_string(), "inline prompt body")?;
    let content_ref = synthetic_message_ref("inline", &role, &safe_body, ordinal, synthetic_refs)?;
    feed_field(fingerprint, b"section", b"inline");
    feed_field(fingerprint, b"ref", content_ref.as_str().as_bytes());
    feed_field(fingerprint, b"role", role.as_bytes());
    feed_field(fingerprint, b"body", safe_body.as_bytes());
    materialized_messages.push(InstructionBundleMaterializedMessage {
        role: role.clone(),
        content_ref: content_ref.clone(),
        safe_content: safe_body,
    });
    messages.push(LoopModelMessage { role, content_ref });
    Ok(())
}

fn inline_role(role: LoopInlineMessageRole) -> &'static str {
    match role {
        LoopInlineMessageRole::System => "system",
        LoopInlineMessageRole::User => "user",
        LoopInlineMessageRole::Assistant => "assistant",
    }
}

fn push_visible_surface(
    messages: &mut Vec<LoopModelMessage>,
    materialized_messages: &mut Vec<InstructionBundleMaterializedMessage>,
    fingerprint: &mut Sha256,
    mut surface: VisibleCapabilitySurface,
    synthetic_refs: &mut SyntheticMessageRefRegistry,
) -> Result<(), AgentLoopHostError> {
    surface
        .descriptors
        .sort_by(|a, b| a.capability_id.cmp(&b.capability_id));
    let mut summary = format!("surface {}", surface.version.as_str());
    for descriptor in &surface.descriptors {
        validate_surface_descriptor(descriptor)?;
        summary.push('|');
        summary.push_str(descriptor.capability_id.as_str());
        summary.push('|');
        summary.push_str(&descriptor.safe_name);
        summary.push('|');
        summary.push_str(&descriptor.safe_description);
    }
    let content_ref = synthetic_message_ref(
        "surface",
        surface.version.as_str(),
        &summary,
        0,
        synthetic_refs,
    )?;
    feed_field(fingerprint, b"section", b"surface");
    feed_field(fingerprint, b"ref", content_ref.as_str().as_bytes());
    feed_field(fingerprint, b"version", surface.version.as_str().as_bytes());
    for descriptor in &surface.descriptors {
        feed_field(
            fingerprint,
            b"capability",
            descriptor.capability_id.as_str().as_bytes(),
        );
        feed_field(fingerprint, b"name", descriptor.safe_name.as_bytes());
        feed_field(
            fingerprint,
            b"description",
            descriptor.safe_description.as_bytes(),
        );
    }
    materialized_messages.push(InstructionBundleMaterializedMessage {
        role: "system".to_string(),
        content_ref: content_ref.clone(),
        safe_content: summary,
    });
    messages.push(LoopModelMessage {
        role: "system".to_string(),
        content_ref,
    });
    Ok(())
}

fn validate_surface_descriptor(
    descriptor: &CapabilityDescriptorView,
) -> Result<(), AgentLoopHostError> {
    validate_model_safe_text(descriptor.safe_name.clone(), "capability safe name")?;
    validate_model_safe_text(
        descriptor.safe_description.clone(),
        "capability safe description",
    )?;
    Ok(())
}

fn snippet_message_ref(
    section: &'static str,
    snippet: &LoopContextSnippet,
    ordinal: usize,
    synthetic_refs: &mut SyntheticMessageRefRegistry,
) -> Result<LoopMessageRef, AgentLoopHostError> {
    synthetic_message_ref(
        section,
        &snippet.snippet_ref,
        &snippet.safe_summary,
        ordinal,
        synthetic_refs,
    )
}

fn synthetic_message_ref(
    section: &'static str,
    source_ref: &str,
    safe_summary: &str,
    ordinal: usize,
    synthetic_refs: &mut SyntheticMessageRefRegistry,
) -> Result<LoopMessageRef, AgentLoopHostError> {
    let slug = sanitize_ref_suffix(source_ref);
    let hash = stable_ref_hash(section, source_ref, safe_summary, ordinal);
    let content_ref = LoopMessageRef::new(format!("msg:{section}.{slug}.{ordinal}.{hash:016x}"))
        .map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "instruction bundle message reference could not be represented",
            )
        })?;
    synthetic_refs.record(
        content_ref,
        SyntheticMessageRefInput::new(section, source_ref, safe_summary, ordinal),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SyntheticMessageRefInput {
    section: &'static str,
    source_ref: String,
    safe_summary: String,
    ordinal: usize,
}

impl SyntheticMessageRefInput {
    fn new(
        section: &'static str,
        source_ref: impl Into<String>,
        safe_summary: impl Into<String>,
        ordinal: usize,
    ) -> Self {
        Self {
            section,
            source_ref: source_ref.into(),
            safe_summary: safe_summary.into(),
            ordinal,
        }
    }
}

#[derive(Debug, Default)]
struct SyntheticMessageRefRegistry {
    inputs_by_ref: HashMap<String, SyntheticMessageRefInput>,
}

impl SyntheticMessageRefRegistry {
    fn record(
        &mut self,
        content_ref: LoopMessageRef,
        input: SyntheticMessageRefInput,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        let key = content_ref.as_str().to_string();
        if let Some(existing) = self.inputs_by_ref.get(&key) {
            if existing != &input {
                tracing::debug!(
                    content_ref = %content_ref.as_str(),
                    existing_section = existing.section,
                    new_section = input.section,
                    new_ordinal = input.ordinal,
                    "instruction bundle synthetic message ref collision detected"
                );
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "instruction bundle message reference collision detected",
                ));
            }
        } else {
            self.inputs_by_ref.insert(key, input);
        }
        Ok(content_ref)
    }
}

/// Sorts instruction snippets in the same order used for prompt construction.
///
/// Skill snippet model refs include their prompt ordinal, so any resolver that
/// recreates those refs must use this ordering before assigning ordinals.
pub fn sort_instruction_snippets_for_prompt(snippets: &mut [LoopContextSnippet]) {
    snippets.sort_by(compare_instruction_snippets);
}

fn compare_instruction_snippets(
    a: &LoopContextSnippet,
    b: &LoopContextSnippet,
) -> std::cmp::Ordering {
    instruction_rank(&a.snippet_ref)
        .cmp(&instruction_rank(&b.snippet_ref))
        .then_with(|| compare_snippet_refs(a, b))
}

fn compare_snippet_refs(a: &LoopContextSnippet, b: &LoopContextSnippet) -> std::cmp::Ordering {
    a.snippet_ref
        .cmp(&b.snippet_ref)
        .then_with(|| a.safe_summary.cmp(&b.safe_summary))
}

fn instruction_rank(snippet_ref: &str) -> u8 {
    if snippet_ref.starts_with("instruction:system") {
        0
    } else if snippet_ref.starts_with("instruction:user") {
        1
    } else if snippet_ref.starts_with("instruction:agent") {
        2
    } else if snippet_ref.starts_with("instruction:project") {
        3
    } else if snippet_ref.starts_with("skill:") {
        4
    } else {
        5
    }
}

fn validate_model_role(role: &str) -> Result<(), AgentLoopHostError> {
    if matches!(
        role,
        "system" | "user" | "assistant" | "tool" | "tool_result_reference"
    ) {
        return Ok(());
    }
    Err(AgentLoopHostError::new(
        AgentLoopHostErrorKind::PolicyDenied,
        "context message role is not model-safe",
    ))
}

fn validate_context_ref(value: String, label: &'static str) -> Result<String, AgentLoopHostError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-' | '.'))
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::PolicyDenied,
            format!("{label} is not model-safe"),
        ));
    }
    reject_sensitive_text(&value, label)?;
    Ok(value)
}

fn validate_model_safe_text(
    value: String,
    label: &'static str,
) -> Result<String, AgentLoopHostError> {
    if value.is_empty() || value.len() > 4096 {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::PolicyDenied,
            format!("{label} is not model-safe"),
        ));
    }
    if value
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::PolicyDenied,
            format!("{label} contains control characters"),
        ));
    }
    reject_sensitive_text(&value, label)?;
    Ok(value)
}

fn reject_sensitive_text(value: &str, label: &'static str) -> Result<(), AgentLoopHostError> {
    let lower = value.to_ascii_lowercase();
    for forbidden_path in [
        "/users/",
        "/home/",
        "/private/",
        "/tmp/", // safety: model-safety denylist literal, not a filesystem temp path.
        "/var/",
        "/etc/",
    ] {
        if lower.contains(forbidden_path) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::PolicyDenied,
                format!("{label} contains non-model-safe content"),
            ));
        }
    }
    for forbidden_phrase in [
        "access token",
        "api key",
        "api_key",
        "api secret",
        "authorization",
        "bearer",
        "client secret",
        "invalid api key",
        "password",
        "passwd",
        "secret key",
        "secret-key",
        "secret token",
        "secret_token",
        "shared secret",
    ] {
        if contains_token_phrase(&lower, forbidden_phrase) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::PolicyDenied,
                format!("{label} contains non-model-safe content"),
            ));
        }
    }
    if lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .any(|token| token.starts_with("sk-"))
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::PolicyDenied,
            format!("{label} contains non-model-safe content"),
        ));
    }
    Ok(())
}

fn contains_token_phrase(value: &str, phrase: &str) -> bool {
    value.match_indices(phrase).any(|(start, matched)| {
        let end = start + matched.len();
        is_token_boundary(char_before(value, start)) && is_token_boundary(char_at(value, end))
    })
}

fn char_before(value: &str, byte_index: usize) -> Option<char> {
    value
        .char_indices()
        .take_while(|(index, _)| *index < byte_index)
        .last()
        .map(|(_, character)| character)
}

fn char_at(value: &str, byte_index: usize) -> Option<char> {
    value
        .char_indices()
        .find(|(index, _)| *index == byte_index)
        .map(|(_, character)| character)
}

fn is_token_boundary(character: Option<char>) -> bool {
    match character {
        Some(character) => !character.is_ascii_alphanumeric() && character != '_',
        None => true,
    }
}

fn sanitize_ref_suffix(value: &str) -> String {
    let mut suffix = String::with_capacity(value.len().min(96));
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
            suffix.push(character);
        } else {
            suffix.push('.');
        }
        if suffix.len() >= 96 {
            break;
        }
    }
    let suffix = suffix.trim_matches('.');
    if suffix.is_empty() {
        "context".to_string()
    } else {
        suffix.to_string()
    }
}

fn stable_ref_hash(section: &str, source_ref: &str, safe_summary: &str, ordinal: usize) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;
    let mut hash = FNV_OFFSET;
    for bytes in [
        section.as_bytes(),
        &[0xFF],
        source_ref.as_bytes(),
        &[0xFF],
        safe_summary.as_bytes(),
        &[0xFF],
        ordinal.to_string().as_bytes(),
    ] {
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

fn feed_field(digest: &mut Sha256, label: &[u8], value: &[u8]) {
    digest.update((label.len() as u64).to_le_bytes());
    digest.update(label);
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_ref_registry_rejects_mismatched_duplicate_refs() {
        let mut registry = SyntheticMessageRefRegistry::default();
        let content_ref = LoopMessageRef::new("msg:instruction.source.0.deadbeefdeadbeef").unwrap();

        registry
            .record(
                content_ref.clone(),
                SyntheticMessageRefInput::new("instruction", "source-a", "summary-a", 0),
            )
            .unwrap();
        let error = registry
            .record(
                content_ref,
                SyntheticMessageRefInput::new("instruction", "source-b", "summary-b", 0),
            )
            .unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    }
}
