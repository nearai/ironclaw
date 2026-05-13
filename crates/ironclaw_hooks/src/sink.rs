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
    /// Record that the hook evaluated the context and has no opinion. The
    /// dispatcher treats this as "this hook contributes nothing to the
    /// composed decision" — distinct from "the hook returned without calling
    /// any sink method," which is treated as a protocol violation and
    /// fails closed.
    fn pass(&mut self);
}

/// Gate sink surface for Installed hooks. Deliberately omits `allow`; an
/// Installed-tier hook can only restrict, never relax, prior decisions.
pub trait RestrictedGateSink: Send {
    fn deny(&mut self, reason: &'static str);
    fn pause_approval(&mut self, reason: &'static str);
    fn pause_auth(&mut self, reason: &'static str);
    /// Record that the hook evaluated the context and has no opinion. See
    /// [`PrivilegedGateSink::pass`] for the full semantics.
    fn pass(&mut self);
}

/// State recorded by [`RecordingGateSink`] as the hook calls sink methods.
/// The dispatcher consumes this to distinguish "hook called nothing"
/// (`Unset` → Malformed → fail-closed) from "hook explicitly passed"
/// (`Passed` → no-opinion → composed decision unchanged) from "hook minted
/// a decision" (`Decided` → compose normally).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GateSinkState {
    Unset,
    Passed,
    Decided(BeforeCapabilityHookDecision),
}

/// Dispatcher-internal sink implementation that records the outcome a hook
/// minted. Implements both privileged and restricted traits because the
/// dispatcher uses one concrete type behind whichever trait pointer it hands
/// the hook.
pub(crate) struct RecordingGateSink {
    pub(crate) state: GateSinkState,
}

impl RecordingGateSink {
    pub(crate) fn new() -> Self {
        Self {
            state: GateSinkState::Unset,
        }
    }

    /// Test/dispatcher accessor: the decision the hook minted, if any.
    /// Returns `None` for both `Unset` and `Passed` — callers that need to
    /// distinguish should inspect [`Self::state`] directly.
    #[cfg(test)]
    pub(crate) fn decision(&self) -> Option<&BeforeCapabilityHookDecision> {
        match &self.state {
            GateSinkState::Decided(d) => Some(d),
            _ => None,
        }
    }
}

impl PrivilegedGateSink for RecordingGateSink {
    fn allow(&mut self) {
        self.state = GateSinkState::Decided(BeforeCapabilityHookDecision::allow());
    }

    fn deny(&mut self, reason: &'static str) {
        self.state = GateSinkState::Decided(BeforeCapabilityHookDecision::deny(
            SanitizedReason::from_static(reason),
        ));
    }

    fn pause_approval(&mut self, reason: &'static str) {
        self.state = GateSinkState::Decided(BeforeCapabilityHookDecision::pause_approval(
            SanitizedReason::from_static(reason),
        ));
    }

    fn pause_auth(&mut self, reason: &'static str) {
        self.state = GateSinkState::Decided(BeforeCapabilityHookDecision::pause_auth(
            SanitizedReason::from_static(reason),
        ));
    }

    fn pass(&mut self) {
        self.state = GateSinkState::Passed;
    }
}

impl RestrictedGateSink for RecordingGateSink {
    fn deny(&mut self, reason: &'static str) {
        self.state = GateSinkState::Decided(BeforeCapabilityHookDecision::deny(
            SanitizedReason::from_static(reason),
        ));
    }

    fn pause_approval(&mut self, reason: &'static str) {
        self.state = GateSinkState::Decided(BeforeCapabilityHookDecision::pause_approval(
            SanitizedReason::from_static(reason),
        ));
    }

    fn pause_auth(&mut self, reason: &'static str) {
        self.state = GateSinkState::Decided(BeforeCapabilityHookDecision::pause_auth(
            SanitizedReason::from_static(reason),
        ));
    }

    fn pass(&mut self) {
        self.state = GateSinkState::Passed;
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

    /// Append an envelope-wrapped untrusted snippet. The caller passes the
    /// raw body; `ironclaw_prompt_envelope::wrap_untrusted` performs the
    /// wrapping, hijack-marker checks, and byte-budget enforcement.
    fn add_envelope_snippet(
        &mut self,
        body: String,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<(), SanitizedReason>;

    /// Attach typed metadata to the prompt-bundle milestone (telemetry only).
    fn add_milestone_metadata(&mut self, key: &'static str, value: String);
}

/// Mutator sink for Installed hooks. Only accepts envelope-wrapped snippets;
/// the raw-text path is not exposed.
pub trait RestrictedMutatorSink: Send {
    /// Append an envelope-wrapped untrusted snippet. The caller passes the
    /// raw body; `ironclaw_prompt_envelope::wrap_untrusted` performs the
    /// wrapping, hijack-marker checks, and byte-budget enforcement.
    fn add_envelope_snippet(
        &mut self,
        body: String,
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
        body: String,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<(), SanitizedReason> {
        let patch = HookPatch::add_enveloped_snippet(body, self.trust_class, ordinal_hint)?;
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
        body: String,
        ordinal_hint: PatchOrdinalHint,
    ) -> Result<(), SanitizedReason> {
        let patch = HookPatch::add_enveloped_snippet(body, self.trust_class, ordinal_hint)?;
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
        let ctx = BeforeCapabilityHookContext::new_unresolved(
            ironclaw_host_api::TenantId::new("t".to_string()).expect("valid tenant"),
            "cap.x".to_string(),
            [0u8; 32],
        );
        DenyOnly
            .evaluate(&ctx, &mut recording as &mut dyn RestrictedGateSink)
            .await;
        assert!(!recording.decision().expect("decision recorded").permits());
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
        let ctx = BeforeCapabilityHookContext::new_unresolved(
            ironclaw_host_api::TenantId::new("t".to_string()).expect("valid tenant"),
            "cap.x".to_string(),
            [0u8; 32],
        );
        AllowOnly
            .evaluate(&ctx, &mut recording as &mut dyn PrivilegedGateSink)
            .await;
        assert!(recording.decision().expect("decision recorded").permits());
    }

    #[tokio::test]
    async fn pass_does_not_record_decision() {
        struct PassingHook;
        #[async_trait]
        impl RestrictedBeforeCapabilityHook for PassingHook {
            async fn evaluate(
                &self,
                _ctx: &BeforeCapabilityHookContext,
                sink: &mut dyn RestrictedGateSink,
            ) {
                sink.pass();
            }
        }

        let mut recording = RecordingGateSink::new();
        let ctx = BeforeCapabilityHookContext::new_unresolved(
            ironclaw_host_api::TenantId::new("t".to_string()).expect("valid tenant"),
            "cap.x".to_string(),
            [0u8; 32],
        );
        PassingHook
            .evaluate(&ctx, &mut recording as &mut dyn RestrictedGateSink)
            .await;
        assert!(recording.decision().is_none());
        assert_eq!(recording.state, GateSinkState::Passed);
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
                sink.add_envelope_snippet("hi".to_string(), PatchOrdinalHint::Last)
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
