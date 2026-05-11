//! Skill context selection for the agent loop-support boundary.
//!
//! This module provides [`SkillContextService`] and the [`SkillContextSource`] trait,
//! which select model-visible skill context from a host-approved run snapshot.
//!
//! # Trust and Visibility Model
//!
//! Every installed skill in a run has two dimensions that gate what the model sees:
//!
//! - **Trust level** ([`SkillTrustLevel`]): determines how much content the model receives.
//!   `Trusted` skills include their full prompt content; `Installed` skills expose only
//!   a safe description.
//!
//! - **Visibility** ([`SkillVisibility`]): determines whether the model sees the skill at all.
//!   `Visible` skills appear in the context; `Hidden` and `Denied` skills are omitted entirely
//!   so the model has no knowledge of their existence.
//!
//! # Fail-closed semantics
//!
//! If trust or visibility data is missing, the snapshot version does not match entries,
//! model-visible fields contain unsafe internal markers, or prompt content exceeds configured
//! context budgets, the service returns an error rather than silently degrading. This ensures
//! that an unconfigured or corrupt snapshot never leaks capabilities to the model.
//!
//! # Determinism
//!
//! Output ordering is deterministic for the same [`SkillRunSnapshot`]: entries are sorted by
//! a total ordering rooted in [`InstalledSkillSnapshot::ordering_key`], and the snapshot
//! version is a deterministic SHA-256 content digest of all entry data. The digest verifies
//! snapshot consistency, not producer authenticity; host trust decisions remain authoritative.

use std::cmp::Ordering;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::LoopMessageRef;

use super::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopContextSnippet, LoopContextSnippetMetadata,
    stable_snippet_display_hash,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Error returned by [`SkillContextSource`] when skill context cannot be produced.
///
/// All variants are sanitized — no raw internals, file paths, or secret handles are leaked.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillContextError {
    /// Trust data is missing or the snapshot is in an inconsistent state.
    #[error("skill context: trust data missing")]
    TrustDataMissing,

    /// Visibility data is missing for one or more skills.
    #[error("skill context: visibility data missing")]
    VisibilityDataMissing,

    /// Snapshot version does not match the entry data.
    #[error("skill context: invalid snapshot version")]
    InvalidSnapshotVersion,

    /// Snapshot content is not safe to expose to the model.
    #[error("skill context: unsafe model-visible content")]
    UnsafeModelVisibleContent,

    /// Skill context budget configuration is invalid.
    #[error("skill context: budget misconfigured")]
    BudgetMisconfigured,

    /// Model-visible skill context exceeds configured context budgets.
    #[error("skill context: context budget exceeded")]
    ContextBudgetExceeded,

    /// An internal error that cannot be attributed to trust, visibility, or budget validation.
    #[error("skill context: internal error")]
    Internal,
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Host-approved visibility status for a skill in a run.
///
/// Controls whether the model is aware of the skill's existence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillVisibility {
    /// The skill is visible to the model and included in context.
    Visible,
    /// The skill exists but is hidden from the model — no mention in output.
    Hidden,
    /// The skill is explicitly denied — no mention in output.
    Denied,
}

/// Trust level for an installed skill, owned by this crate.
///
/// Mirrors the upstream `SkillTrust` enum without creating a production dependency
/// on `ironclaw_skills`.
///
/// - `Installed`: read-only context; the model sees only the safe description.
/// - `Trusted`: full context; the model sees description and prompt content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTrustLevel {
    /// Registry/external skill — description only, no prompt content.
    Installed,
    /// User-placed/trusted skill — description and prompt content.
    Trusted,
}

impl SkillTrustLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Trusted => "trusted",
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot types and context budgets
// ---------------------------------------------------------------------------

const EMPTY_SNAPSHOT_VERSION: &str = "empty";
const DEFAULT_MAX_SKILL_SNIPPET_BYTES: usize = 8 * 1024;
const DEFAULT_MAX_SKILL_CONTEXT_BYTES: usize = 32 * 1024;

/// Byte budgets for model-visible skill context produced by [`SkillContextService`].
///
/// Hosts can map a run's context profile to these limits via
/// [`SkillContextService::with_budget`]. Both limits fail closed when exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillContextBudget {
    /// Maximum bytes for one snippet summary.
    pub max_snippet_bytes: usize,
    /// Maximum aggregate bytes across emitted snippet refs and summaries.
    pub max_context_bytes: usize,
}

impl SkillContextBudget {
    /// Create explicit skill-context budget limits.
    pub const fn new(max_snippet_bytes: usize, max_context_bytes: usize) -> Self {
        Self {
            max_snippet_bytes,
            max_context_bytes,
        }
    }
}

impl Default for SkillContextBudget {
    fn default() -> Self {
        Self {
            max_snippet_bytes: DEFAULT_MAX_SKILL_SNIPPET_BYTES,
            max_context_bytes: DEFAULT_MAX_SKILL_CONTEXT_BYTES,
        }
    }
}

/// Immutable, host-approved state of a single installed skill for a run.
///
/// Captures everything the service needs to decide what the model sees.
/// Must not contain raw file paths, capability IDs, secret handles, or
/// other internal metadata — only model-safe data.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InstalledSkillSnapshot {
    /// Human-readable name of the skill.
    pub name: String,
    /// Trust level — determines how much content the model receives.
    pub trust: SkillTrustLevel,
    /// Visibility — determines whether the model sees this skill at all.
    pub visibility: SkillVisibility,
    /// Full prompt content. Only included in model context when
    /// `trust == Trusted` and `visibility == Visible`.
    pub prompt_content: Option<String>,
    /// Sanitized description safe for model consumption.
    pub safe_description: String,
    /// Primary key used for deterministic sorting of output.
    pub ordering_key: String,
}

/// Complete set of installed skill snapshots for a run.
///
/// The `snapshot_version` is a deterministic SHA-256 content digest of all entries,
/// used to verify the service is reading the same entry data approved by the host.
/// It is not an authenticity proof; trusted hosts remain responsible for producing
/// approved snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRunSnapshot {
    /// All installed skill entries for this run.
    pub entries: Vec<InstalledSkillSnapshot>,
    /// Deterministic version string derived from entry data.
    /// An empty version indicates missing/corrupt trust data and triggers fail-closed behavior.
    pub snapshot_version: String,
}

impl SkillRunSnapshot {
    /// Create an empty snapshot for the no-skills case.
    ///
    /// Returns a stable, valid snapshot with an empty entry list and a fixed version string.
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            snapshot_version: EMPTY_SNAPSHOT_VERSION.to_string(),
        }
    }

    /// Build a snapshot from a list of entries with a deterministic version hash.
    ///
    /// Entries are total-order sorted before hashing so that insertion order and
    /// duplicate ordering keys do not affect the version.
    pub fn from_entries(mut entries: Vec<InstalledSkillSnapshot>) -> Self {
        if entries.is_empty() {
            return Self::empty();
        }

        entries.sort_by(compare_skill_entries);
        let version = compute_snapshot_version(&entries);
        Self {
            entries,
            snapshot_version: version,
        }
    }
}

/// Snippet data produced by [`SkillContextSource`], ready for conversion into
/// a [`LoopContextSnippet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillContextSnippet {
    /// Reference identifier, e.g. `skill:<name>`.
    pub snippet_ref: String,
    /// Sanitized summary containing only the safe description and optionally prompt content.
    pub safe_summary: String,
    /// Model-visible skill name used for telemetry, never for authority decisions.
    pub skill_name: String,
    /// Host-approved trust tier used for telemetry and downstream attenuation checks.
    pub trust: SkillTrustLevel,
}

impl SkillContextSnippet {
    /// Convert into the loop-layer [`LoopContextSnippet`] type.
    pub fn into_loop_snippet(self) -> LoopContextSnippet {
        LoopContextSnippet {
            snippet_ref: self.snippet_ref,
            safe_summary: self.safe_summary,
            metadata: Some(LoopContextSnippetMetadata {
                source_name: self.skill_name,
                trust_level: self.trust.as_str().to_string(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Port for selecting model-visible skill context from a host-approved run snapshot.
///
/// Implementations must be deterministic for the same inputs, trust-aware, and fail-closed
/// when trust or visibility data is missing. They must never grant authority or make
/// hidden/denied capabilities invokable.
#[async_trait]
pub trait SkillContextSource: Send + Sync {
    /// Produce skill context snippets for the given run snapshot.
    async fn skill_snippets(
        &self,
        run_snapshot: &SkillRunSnapshot,
    ) -> Result<Vec<SkillContextSnippet>, SkillContextError>;
}

// ---------------------------------------------------------------------------
// Service implementation
// ---------------------------------------------------------------------------

/// Deterministic, trust-aware skill context service.
///
/// Holds a [`SkillRunSnapshot`] and produces model-visible context snippets
/// following the trust/visibility rules documented at the module level.
///
/// The held snapshot is used as a convenience default via
/// [`skill_snippets_from_held`](Self::skill_snippets_from_held). The trait
/// method [`SkillContextSource::skill_snippets`] accepts any snapshot.
pub struct SkillContextService {
    snapshot: SkillRunSnapshot,
    budget: SkillContextBudget,
}

impl SkillContextService {
    /// Create a new service from a host-approved run snapshot with default context limits.
    pub fn new(snapshot: SkillRunSnapshot) -> Self {
        Self::with_budget(snapshot, SkillContextBudget::default())
    }

    /// Create a new service from a host-approved run snapshot with explicit context limits.
    pub fn with_budget(snapshot: SkillRunSnapshot, budget: SkillContextBudget) -> Self {
        Self { snapshot, budget }
    }

    /// Convenience: produce snippets from the held snapshot.
    pub async fn skill_snippets_from_held(
        &self,
    ) -> Result<Vec<SkillContextSnippet>, SkillContextError> {
        self.skill_snippets(&self.snapshot).await
    }
}

#[async_trait]
impl SkillContextSource for SkillContextService {
    async fn skill_snippets(
        &self,
        run_snapshot: &SkillRunSnapshot,
    ) -> Result<Vec<SkillContextSnippet>, SkillContextError> {
        validate_snapshot(run_snapshot)?;
        validate_budget(self.budget)?;

        let mut visible: Vec<&InstalledSkillSnapshot> = run_snapshot
            .entries
            .iter()
            .filter(|entry| entry.visibility == SkillVisibility::Visible)
            .collect();

        // Re-sort here even though `from_entries` sorts, because snapshots may
        // have been constructed manually. Use total-order sorting so duplicate
        // ordering keys cannot make output depend on input order.
        visible.sort_by(compare_visible_skill_entries);

        let mut snippets = Vec::with_capacity(visible.len());
        let mut total_bytes = 0usize;

        for entry in visible {
            let safe_summary = match entry.trust {
                SkillTrustLevel::Trusted => {
                    if let Some(ref content) = entry.prompt_content {
                        format!("{}\n\n{}", entry.safe_description, content)
                    } else {
                        entry.safe_description.clone()
                    }
                }
                SkillTrustLevel::Installed => entry.safe_description.clone(),
            };

            if safe_summary.len() > self.budget.max_snippet_bytes {
                return Err(SkillContextError::ContextBudgetExceeded);
            }

            validate_model_visible_skill_name(&entry.name)?;
            validate_model_visible_text(&safe_summary)?;

            let snippet_ref = format!("skill:{}", entry.name);
            total_bytes = checked_context_total_bytes(
                total_bytes,
                snippet_ref.len(),
                safe_summary.len(),
                self.budget.max_context_bytes,
            )?;

            snippets.push(SkillContextSnippet {
                snippet_ref,
                safe_summary,
                skill_name: entry.name.clone(),
                trust: entry.trust,
            });
        }

        Ok(snippets)
    }
}

// ---------------------------------------------------------------------------
// Noop implementation
// ---------------------------------------------------------------------------

/// A no-op implementation of [`SkillContextSource`] that always returns an empty list.
///
/// Useful for composition and testing when no skill context is needed.
pub struct NoopSkillContextSource;

#[async_trait]
impl SkillContextSource for NoopSkillContextSource {
    async fn skill_snippets(
        &self,
        _run_snapshot: &SkillRunSnapshot,
    ) -> Result<Vec<SkillContextSnippet>, SkillContextError> {
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the model-message ref for a skill snippet.
///
/// Prompt construction and model-message resolution both use this exact helper
/// so source/ordering drift fails closed instead of producing mismatched refs.
pub fn skill_snippet_model_message_ref(
    snippet_ref: &str,
    safe_summary: &str,
    ordinal: usize,
) -> Result<LoopMessageRef, AgentLoopHostError> {
    let slug = sanitize_ref_suffix(snippet_ref);
    let ordinal = ordinal.to_string();
    let hash = stable_snippet_display_hash([snippet_ref, safe_summary, &ordinal]);
    LoopMessageRef::new(format!("msg:snippet.{slug}.{ordinal}.{hash:016x}")).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "skill context snippet reference could not be represented",
        )
    })
}

pub fn is_skill_snippet_model_message_ref(content_ref: &LoopMessageRef) -> bool {
    content_ref.as_str().starts_with("msg:snippet.")
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

fn validate_snapshot(snapshot: &SkillRunSnapshot) -> Result<(), SkillContextError> {
    if snapshot.snapshot_version.is_empty() {
        return Err(SkillContextError::TrustDataMissing);
    }

    if snapshot.entries.is_empty() {
        if snapshot.snapshot_version == EMPTY_SNAPSHOT_VERSION {
            return Ok(());
        }
        return Err(SkillContextError::InvalidSnapshotVersion);
    }

    let mut sorted_entries = snapshot.entries.clone();
    sorted_entries.sort_by(compare_skill_entries);
    let expected_version = compute_snapshot_version(&sorted_entries);
    if snapshot.snapshot_version != expected_version {
        return Err(SkillContextError::InvalidSnapshotVersion);
    }

    Ok(())
}

fn validate_budget(budget: SkillContextBudget) -> Result<(), SkillContextError> {
    if budget.max_snippet_bytes == 0
        || budget.max_context_bytes == 0
        || budget.max_snippet_bytes > budget.max_context_bytes
    {
        return Err(SkillContextError::BudgetMisconfigured);
    }

    Ok(())
}

fn compare_visible_skill_entries(
    a: &&InstalledSkillSnapshot,
    b: &&InstalledSkillSnapshot,
) -> Ordering {
    compare_skill_entries(a, b)
}

fn compare_skill_entries(a: &InstalledSkillSnapshot, b: &InstalledSkillSnapshot) -> Ordering {
    a.ordering_key
        .cmp(&b.ordering_key)
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| trust_rank(a.trust).cmp(&trust_rank(b.trust)))
        .then_with(|| visibility_rank(a.visibility).cmp(&visibility_rank(b.visibility)))
        .then_with(|| a.safe_description.cmp(&b.safe_description))
        .then_with(|| a.prompt_content.cmp(&b.prompt_content))
}

const fn trust_rank(trust: SkillTrustLevel) -> u8 {
    match trust {
        SkillTrustLevel::Installed => 0,
        SkillTrustLevel::Trusted => 1,
    }
}

const fn visibility_rank(visibility: SkillVisibility) -> u8 {
    match visibility {
        SkillVisibility::Visible => 0,
        SkillVisibility::Hidden => 1,
        SkillVisibility::Denied => 2,
    }
}

fn validate_model_visible_skill_name(name: &str) -> Result<(), SkillContextError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(SkillContextError::UnsafeModelVisibleContent);
    };

    if !first.is_ascii_alphanumeric()
        || name.len() > 64
        || chars.any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')))
    {
        return Err(SkillContextError::UnsafeModelVisibleContent);
    }

    Ok(())
}

fn validate_model_visible_text(text: &str) -> Result<(), SkillContextError> {
    if text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
        || contains_raw_host_path(text)
        || contains_internal_handle_marker(text)
    {
        return Err(SkillContextError::UnsafeModelVisibleContent);
    }

    Ok(())
}

fn contains_raw_host_path(text: &str) -> bool {
    text.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
            )
    })
    .any(|token| {
        token.starts_with("/Users/")
            || token.starts_with("/home/")
            || token.starts_with("/private/")
            || token.starts_with("/tmp/") // safety: this is a blocked host-path prefix pattern, not a test temp path.
            || token.starts_with("/var/")
            || token.starts_with("/etc/")
            || token.as_bytes().get(0..3).is_some_and(|prefix| {
                prefix[0].is_ascii_alphabetic() && prefix[1] == b':' && prefix[2] == b'\\'
            })
    })
}

fn contains_internal_handle_marker(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("cap_") || lower.contains("secret://") || lower.contains("secret:")
}

fn checked_context_total_bytes(
    current_total: usize,
    snippet_ref_bytes: usize,
    safe_summary_bytes: usize,
    max_context_bytes: usize,
) -> Result<usize, SkillContextError> {
    let next_total = current_total
        .checked_add(snippet_ref_bytes)
        .and_then(|total| total.checked_add(safe_summary_bytes))
        .ok_or(SkillContextError::ContextBudgetExceeded)?;

    if next_total > max_context_bytes {
        return Err(SkillContextError::ContextBudgetExceeded);
    }

    Ok(next_total)
}

/// Compute a deterministic version string from sorted snapshot entries.
///
/// Uses a SHA-256 digest over length-prefixed field data. The digest is collision-resistant
/// for consistency checks, but is not an authenticity proof or authorization decision.
fn compute_snapshot_version(sorted_entries: &[InstalledSkillSnapshot]) -> String {
    let mut digest = Sha256::new();

    for entry in sorted_entries {
        feed_digest_field(&mut digest, entry.name.as_bytes());
        feed_digest_field(
            &mut digest,
            match entry.trust {
                SkillTrustLevel::Installed => b"installed",
                SkillTrustLevel::Trusted => b"trusted",
            },
        );
        feed_digest_field(
            &mut digest,
            match entry.visibility {
                SkillVisibility::Visible => b"visible",
                SkillVisibility::Hidden => b"hidden",
                SkillVisibility::Denied => b"denied",
            },
        );
        match entry.prompt_content {
            Some(ref content) => {
                digest.update([1]);
                feed_digest_field(&mut digest, content.as_bytes());
            }
            None => digest.update([0]),
        }
        feed_digest_field(&mut digest, entry.safe_description.as_bytes());
        feed_digest_field(&mut digest, entry.ordering_key.as_bytes());
        digest.update([0xFE]);
    }

    format!("sha256:{}", hex::encode(digest.finalize()))
}

fn feed_digest_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_byte_accumulator_reports_arithmetic_overflow() {
        let err = checked_context_total_bytes(usize::MAX, 1, 0, usize::MAX).unwrap_err();
        assert_eq!(err, SkillContextError::ContextBudgetExceeded);
    }
}
