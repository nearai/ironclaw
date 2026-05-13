//! Mutator patches for the `before_prompt` / `before_context` hook points.
//!
//! Patches are **additive only**. No variant in [`HookPatchInner`] lets a hook
//! remove existing content, replace messages, or insert at the identity slot
//! (position 0). The byte budget is checked at dispatch time against the same
//! `MAX_TOTAL_SAFE_SUMMARY_BYTES` cap memory context uses (PR #3471), so a
//! hook cannot bypass the model's context window via mutator-flooding.
//!
//! Snippets emitted by `Installed`-tier hooks must already be wrapped in the
//! prompt envelope (see [`crate::sink::InstalledHookSink::add_envelope_snippet`]).
//! Builtin and Trusted tiers can submit pre-validated trusted snippets via a
//! different sink method, but the dispatcher converts both to a uniform
//! `HookPatch` shape before delivery.

use crate::error::SanitizedReason;
use crate::trust::HookTrustClass;

/// A bounded, typed patch the dispatcher applies between the prompt
/// composition and the model call. Sealed; constructable only via
/// [`crate::sink`] methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookPatch {
    pub(crate) inner: HookPatchInner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HookPatchInner {
    /// Append a safe-summary snippet to the prompt bundle's instruction-snippet
    /// list. The dispatcher (or a follow-up Reborn middleware) is responsible
    /// for routing this into `LoopContextSnippet` and pinning the source as
    /// `SnippetSourceKind::Hook { hook_id }`.
    AddSnippet {
        body: SnippetBody,
        ordinal_hint: PatchOrdinalHint,
        trust_class: HookTrustClass,
        byte_count: u32,
    },
    /// Attach typed metadata to the prompt-bundle milestone (telemetry only,
    /// not model-visible).
    AddMilestoneMetadata { key: MetadataKey, value: String },
}

/// Where in the snippet ordering the hook would like its patch placed. The
/// dispatcher honors hints subject to phase constraints (never position 0,
/// never before identity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOrdinalHint {
    /// Append at the end of the instruction-snippet list. Safe default.
    Last,
    /// Place near the top of the *non-identity* snippet region. The dispatcher
    /// clamps this to position 1 or later (identity owns position 0).
    NearTop,
}

/// Snippet body. Two flavors enforce that untrusted authors only contribute
/// envelope-wrapped content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SnippetBody {
    /// Already envelope-wrapped untrusted content produced by an Installed
    /// hook. The envelope helper (currently in
    /// `ironclaw_host_runtime::memory_context`; extraction tracked separately)
    /// is the only path that produces this variant.
    Enveloped { wrapped: String },
    /// Trusted content from a Builtin or Trusted hook. Bypasses envelope
    /// wrapping but goes through the safe-summary length and pattern checks.
    Trusted { text: String },
}

/// Sanitization-policy-checked metadata key for milestone attachments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataKey(pub(crate) String);

impl MetadataKey {
    /// Construct from a known-safe static key. Keys are part of the
    /// observability schema and the static-only constraint mirrors the
    /// schema-as-code convention.
    pub(crate) fn from_static(key: &'static str) -> Self {
        Self(key.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl HookPatch {
    pub(crate) fn add_enveloped_snippet(
        wrapped: String,
        trust_class: HookTrustClass,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<Self, SanitizedReason> {
        let byte_count = u32::try_from(wrapped.len()).map_err(|_| {
            SanitizedReason::from_static("hook snippet exceeds 4 GiB; refusing to construct")
        })?;
        Ok(Self {
            inner: HookPatchInner::AddSnippet {
                body: SnippetBody::Enveloped { wrapped },
                ordinal_hint,
                trust_class,
                byte_count,
            },
        })
    }

    pub(crate) fn add_trusted_snippet(
        text: String,
        trust_class: HookTrustClass,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<Self, SanitizedReason> {
        debug_assert!(
            matches!(
                trust_class,
                HookTrustClass::Builtin | HookTrustClass::Trusted
            ),
            "trusted snippet body requires Builtin or Trusted tier"
        );
        let byte_count = u32::try_from(text.len()).map_err(|_| {
            SanitizedReason::from_static("hook snippet exceeds 4 GiB; refusing to construct")
        })?;
        Ok(Self {
            inner: HookPatchInner::AddSnippet {
                body: SnippetBody::Trusted { text },
                ordinal_hint,
                trust_class,
                byte_count,
            },
        })
    }

    pub(crate) fn add_milestone_metadata(key: MetadataKey, value: String) -> Self {
        Self {
            inner: HookPatchInner::AddMilestoneMetadata { key, value },
        }
    }

    /// Public read-only view for the dispatcher and downstream consumers.
    pub fn view(&self) -> HookPatchView<'_> {
        match &self.inner {
            HookPatchInner::AddSnippet {
                body,
                ordinal_hint,
                trust_class,
                byte_count,
            } => HookPatchView::AddSnippet {
                body: body.view(),
                ordinal_hint: *ordinal_hint,
                trust_class: *trust_class,
                byte_count: *byte_count,
            },
            HookPatchInner::AddMilestoneMetadata { key, value } => {
                HookPatchView::AddMilestoneMetadata { key, value }
            }
        }
    }

    /// Byte cost of this patch toward the prompt-bundle byte budget. Metadata
    /// attachments cost zero because they don't reach the model.
    pub fn snippet_byte_count(&self) -> u32 {
        match &self.inner {
            HookPatchInner::AddSnippet { byte_count, .. } => *byte_count,
            HookPatchInner::AddMilestoneMetadata { .. } => 0,
        }
    }
}

impl SnippetBody {
    fn view(&self) -> SnippetBodyView<'_> {
        match self {
            Self::Enveloped { wrapped } => SnippetBodyView::Enveloped { wrapped },
            Self::Trusted { text } => SnippetBodyView::Trusted { text },
        }
    }
}

/// Read-only projection of [`HookPatch`].
#[derive(Debug)]
pub enum HookPatchView<'a> {
    AddSnippet {
        body: SnippetBodyView<'a>,
        ordinal_hint: PatchOrdinalHint,
        trust_class: HookTrustClass,
        byte_count: u32,
    },
    AddMilestoneMetadata {
        key: &'a MetadataKey,
        value: &'a String,
    },
}

/// Read-only projection of a snippet body.
#[derive(Debug)]
pub enum SnippetBodyView<'a> {
    Enveloped { wrapped: &'a str },
    Trusted { text: &'a str },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enveloped_snippet_records_byte_count() {
        let patch = HookPatch::add_enveloped_snippet(
            "Untrusted hook content: hi".to_string(),
            HookTrustClass::Installed,
            PatchOrdinalHint::Last,
        )
        .expect("construct");
        assert_eq!(patch.snippet_byte_count(), 26);
    }

    #[test]
    fn trusted_snippet_carries_text_body() {
        let patch = HookPatch::add_trusted_snippet(
            "Safety reminder: do not".to_string(),
            HookTrustClass::Builtin,
            PatchOrdinalHint::NearTop,
        )
        .expect("construct");
        match patch.view() {
            HookPatchView::AddSnippet {
                body: SnippetBodyView::Trusted { text },
                trust_class,
                ordinal_hint,
                ..
            } => {
                assert_eq!(text, "Safety reminder: do not");
                assert_eq!(trust_class, HookTrustClass::Builtin);
                assert_eq!(ordinal_hint, PatchOrdinalHint::NearTop);
            }
            other => panic!("unexpected view: {other:?}"),
        }
    }

    #[test]
    fn milestone_metadata_does_not_count_toward_byte_budget() {
        let patch = HookPatch::add_milestone_metadata(
            MetadataKey::from_static("hook.fired"),
            "some-id".to_string(),
        );
        assert_eq!(patch.snippet_byte_count(), 0);
    }
}
