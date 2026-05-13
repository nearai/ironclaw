//! Sinks — the trait surfaces hook authors receive when they're invoked.
//!
//! There are two sink surfaces per kind:
//!
//! - `Privileged*` — exposed to `Builtin` and `Trusted` hooks. Carries the
//!   full decision vocabulary including `Allow` for gate sinks and
//!   `add_trusted_snippet` for mutator sinks (no envelope required).
//! - `Restricted*` — exposed to `Installed` hooks. Does *not* expose `Allow`
//!   for gates and only accepts envelope-wrapped snippets for mutators. An
//!   `Installed` hook author literally cannot call `.allow()` — the method
//!   does not exist on this trait — so a malicious or buggy extension cannot
//!   override a more-restrictive prior decision.
//!
//! The framework adds one hook trait per (point, tier) pair so that the
//! signature an author writes against also carries the tier constraint at
//! compile time. The dispatcher routes through a `BoxedHook` enum that holds
//! either the privileged or restricted impl.

use async_trait::async_trait;

use crate::error::SanitizedReason;
use crate::kinds::gate::{BeforeCapabilityHookDecision, GateDecisionInner};
use crate::kinds::mutator::{HookPatch, PatchOrdinalHint};
use crate::kinds::observer::{NoteCategory, ObserverFact};
use crate::points::{BeforeCapabilityHookContext, BeforePromptHookContext, ObserverHookContext};
use crate::trust::HookTrustClass;

// ─── Gate sinks ─────────────────────────────────────────────────────────────

/// Gate sink surface for Builtin + Trusted hooks. Includes `allow`.
///
/// Reasons accepted by the deny/pause methods are `&'static str` so the
/// authored content goes through the rustc literal table — no dynamic
/// `format!`-built strings can leak through this seam. Hooks needing
/// parameterized user-facing reasons should ship them via the manifest
/// predicate path (which is validated at install time) rather than minting
/// reasons at hook-time.
pub trait PrivilegedGateSink: Send {
    fn allow(&mut self);
    fn deny(&mut self, reason: &'static str);
    fn pause_approval(&mut self, reason: &'static str);
    fn pause_auth(&mut self, reason: &'static str);
}

/// Gate sink surface for Installed hooks. Deliberately omits `allow`; an
/// Installed-tier hook can only restrict, never relax, prior decisions.
pub trait RestrictedGateSink: Send {
    fn deny(&mut self, reason: &'static str);
    fn pause_approval(&mut self, reason: &'static str);
    fn pause_auth(&mut self, reason: &'static str);
}

/// Dispatcher-internal sink implementation that records the decision a hook
/// minted. Implements both privileged and restricted traits because the
/// dispatcher uses one concrete type behind whichever trait pointer it hands
/// the hook.
pub(crate) struct RecordingGateSink {
    pub(crate) decision: Option<BeforeCapabilityHookDecision>,
}

impl RecordingGateSink {
    pub(crate) fn new() -> Self {
        Self { decision: None }
    }
}

impl PrivilegedGateSink for RecordingGateSink {
    fn allow(&mut self) {
        self.decision = Some(BeforeCapabilityHookDecision::allow());
    }

    fn deny(&mut self, reason: &'static str) {
        self.decision = Some(BeforeCapabilityHookDecision::deny(
            SanitizedReason::from_static(reason),
        ));
    }

    fn pause_approval(&mut self, reason: &'static str) {
        self.decision = Some(BeforeCapabilityHookDecision::pause_approval(
            SanitizedReason::from_static(reason),
        ));
    }

    fn pause_auth(&mut self, reason: &'static str) {
        self.decision = Some(BeforeCapabilityHookDecision::pause_auth(
            SanitizedReason::from_static(reason),
        ));
    }
}

impl RestrictedGateSink for RecordingGateSink {
    fn deny(&mut self, reason: &'static str) {
        self.decision = Some(BeforeCapabilityHookDecision::deny(
            SanitizedReason::from_static(reason),
        ));
    }

    fn pause_approval(&mut self, reason: &'static str) {
        self.decision = Some(BeforeCapabilityHookDecision::pause_approval(
            SanitizedReason::from_static(reason),
        ));
    }

    fn pause_auth(&mut self, reason: &'static str) {
        self.decision = Some(BeforeCapabilityHookDecision::pause_auth(
            SanitizedReason::from_static(reason),
        ));
    }
}

// ─── Mutator sinks ──────────────────────────────────────────────────────────

/// Mutator sink for Builtin + Trusted hooks. Accepts both trusted (raw text)
/// and enveloped snippets.
pub trait PrivilegedMutatorSink: Send {
    /// Append a trusted snippet (no envelope wrapping). Reserved for
    /// host-authored content.
    fn add_trusted_snippet(
        &mut self,
        text: String,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<(), SanitizedReason>;

    /// Append an envelope-wrapped untrusted snippet. The wrapping is the
    /// caller's responsibility; the dispatcher validates the envelope marker
    /// at a higher layer (follow-up: tie this to the shared `prompt_envelope`
    /// helper).
    fn add_envelope_snippet(
        &mut self,
        wrapped: String,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<(), SanitizedReason>;

    /// Attach typed metadata to the prompt-bundle milestone (telemetry only).
    fn add_milestone_metadata(&mut self, key: &'static str, value: String);
}

/// Mutator sink for Installed hooks. Only accepts envelope-wrapped snippets;
/// the raw-text path is not exposed.
pub trait RestrictedMutatorSink: Send {
    fn add_envelope_snippet(
        &mut self,
        wrapped: String,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<(), SanitizedReason>;

    fn add_milestone_metadata(&mut self, key: &'static str, value: String);
}

pub(crate) struct RecordingMutatorSink {
    pub(crate) trust_class: HookTrustClass,
    pub(crate) patches: Vec<HookPatch>,
}

impl RecordingMutatorSink {
    pub(crate) fn new(trust_class: HookTrustClass) -> Self {
        Self {
            trust_class,
            patches: Vec::new(),
        }
    }
}

impl PrivilegedMutatorSink for RecordingMutatorSink {
    fn add_trusted_snippet(
        &mut self,
        text: String,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<(), SanitizedReason> {
        let patch = HookPatch::add_trusted_snippet(text, self.trust_class, ordinal_hint)?;
        self.patches.push(patch);
        Ok(())
    }

    fn add_envelope_snippet(
        &mut self,
        wrapped: String,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<(), SanitizedReason> {
        let patch = HookPatch::add_enveloped_snippet(wrapped, self.trust_class, ordinal_hint)?;
        self.patches.push(patch);
        Ok(())
    }

    fn add_milestone_metadata(&mut self, key: &'static str, value: String) {
        let patch = HookPatch::add_milestone_metadata(
            crate::kinds::mutator::MetadataKey::from_static(key),
            value,
        );
        self.patches.push(patch);
    }
}

impl RestrictedMutatorSink for RecordingMutatorSink {
    fn add_envelope_snippet(
        &mut self,
        wrapped: String,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<(), SanitizedReason> {
        let patch = HookPatch::add_enveloped_snippet(wrapped, self.trust_class, ordinal_hint)?;
        self.patches.push(patch);
        Ok(())
    }

    fn add_milestone_metadata(&mut self, key: &'static str, value: String) {
        let patch = HookPatch::add_milestone_metadata(
            crate::kinds::mutator::MetadataKey::from_static(key),
            value,
        );
        self.patches.push(patch);
    }
}

// ─── Observer sink ──────────────────────────────────────────────────────────

/// Observer sink — same surface for all trust tiers because observers cannot
/// alter outcomes. The dispatcher still scopes attribution by trust class so
/// audit consumers can distinguish "Builtin observer fired" from "Installed
/// observer fired."
pub trait ObserverSink: Send {
    fn note(&mut self, category: NoteCategory, summary: &'static str);
}

pub(crate) struct RecordingObserverSink {
    pub(crate) facts: Vec<ObserverFact>,
}

impl RecordingObserverSink {
    pub(crate) fn new() -> Self {
        Self { facts: Vec::new() }
    }
}

impl ObserverSink for RecordingObserverSink {
    fn note(&mut self, category: NoteCategory, summary: &'static str) {
        self.facts.push(ObserverFact::note(
            category,
            SanitizedReason::from_static(summary),
        ));
    }
}

// ─── Hook author traits (per point × tier) ─────────────────────────────────

/// A `before_capability` hook supplied by a Builtin or Trusted source.
#[async_trait]
pub trait PrivilegedBeforeCapabilityHook: Send + Sync {
    async fn evaluate(&self, ctx: &BeforeCapabilityHookContext, sink: &mut dyn PrivilegedGateSink);
}

/// A `before_capability` hook supplied by an Installed source. The sink
/// surface omits `.allow()` so this hook cannot mint a permissive override.
#[async_trait]
pub trait RestrictedBeforeCapabilityHook: Send + Sync {
    async fn evaluate(&self, ctx: &BeforeCapabilityHookContext, sink: &mut dyn RestrictedGateSink);
}

/// A `before_prompt` mutator supplied by a Builtin or Trusted source.
#[async_trait]
pub trait PrivilegedBeforePromptHook: Send + Sync {
    async fn evaluate(&self, ctx: &BeforePromptHookContext, sink: &mut dyn PrivilegedMutatorSink);
}

/// A `before_prompt` mutator supplied by an Installed source.
#[async_trait]
pub trait RestrictedBeforePromptHook: Send + Sync {
    async fn evaluate(&self, ctx: &BeforePromptHookContext, sink: &mut dyn RestrictedMutatorSink);
}

/// An observer hook. Same surface for all tiers because observers do not
/// affect outcomes.
#[async_trait]
pub trait ObserverHook: Send + Sync {
    async fn observe(&self, ctx: &ObserverHookContext, sink: &mut dyn ObserverSink);
}

// ─── Dispatcher access to the internal decision ────────────────────────────

impl BeforeCapabilityHookDecision {
    /// Internal accessor used by the dispatcher to inspect the decision's
    /// inner shape without going through the public `view()` projection (which
    /// allocates lifetimes the dispatcher does not need).
    pub(crate) fn inner(&self) -> &GateDecisionInner {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn restricted_gate_sink_cannot_allow_at_type_level() {
        // Compile-time check: `RestrictedGateSink` has no `allow` method.
        // The fact that this trait function compiles is the proof — if we
        // could write `sink.allow();` here, the line would compile against a
        // `&mut dyn RestrictedGateSink` and the trust property would be
        // broken. We verify deny still works.
        struct DenyOnly;
        #[async_trait]
        impl RestrictedBeforeCapabilityHook for DenyOnly {
            async fn evaluate(
                &self,
                _ctx: &BeforeCapabilityHookContext,
                sink: &mut dyn RestrictedGateSink,
            ) {
                sink.deny("blocked");
            }
        }

        let mut recording = RecordingGateSink::new();
        let ctx = BeforeCapabilityHookContext::new(
            ironclaw_host_api::TenantId::new("t".to_string()).expect("valid tenant"),
            "cap.x".to_string(),
            [0u8; 32],
        );
        DenyOnly
            .evaluate(&ctx, &mut recording as &mut dyn RestrictedGateSink)
            .await;
        assert!(!recording.decision.as_ref().unwrap().permits());
    }

    #[tokio::test]
    async fn privileged_gate_sink_can_allow() {
        struct AllowOnly;
        #[async_trait]
        impl PrivilegedBeforeCapabilityHook for AllowOnly {
            async fn evaluate(
                &self,
                _ctx: &BeforeCapabilityHookContext,
                sink: &mut dyn PrivilegedGateSink,
            ) {
                sink.allow();
            }
        }

        let mut recording = RecordingGateSink::new();
        let ctx = BeforeCapabilityHookContext::new(
            ironclaw_host_api::TenantId::new("t".to_string()).expect("valid tenant"),
            "cap.x".to_string(),
            [0u8; 32],
        );
        AllowOnly
            .evaluate(&ctx, &mut recording as &mut dyn PrivilegedGateSink)
            .await;
        assert!(recording.decision.as_ref().unwrap().permits());
    }

    #[tokio::test]
    async fn installed_mutator_path_only_envelopes() {
        struct EnvelopeOnly;
        #[async_trait]
        impl RestrictedBeforePromptHook for EnvelopeOnly {
            async fn evaluate(
                &self,
                _ctx: &BeforePromptHookContext,
                sink: &mut dyn RestrictedMutatorSink,
            ) {
                sink.add_envelope_snippet(
                    "Untrusted hook content: hi".to_string(),
                    PatchOrdinalHint::Last,
                )
                .expect("ok");
            }
        }

        let mut recording = RecordingMutatorSink::new(HookTrustClass::Installed);
        let ctx = BeforePromptHookContext::new(
            ironclaw_host_api::TenantId::new("t".to_string()).expect("valid tenant"),
            4096,
        );
        EnvelopeOnly
            .evaluate(&ctx, &mut recording as &mut dyn RestrictedMutatorSink)
            .await;
        assert_eq!(recording.patches.len(), 1);
        assert_eq!(recording.patches[0].snippet_byte_count(), 26);
    }
}
