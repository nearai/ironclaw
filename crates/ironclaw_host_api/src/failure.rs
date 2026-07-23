//! Slice-C kernel vocabulary — the host-failure channel.
//!
//! Part of the capability-path result collapse described in
//! `docs/ironclaw/2026-07-17-architecture-simplification-dto-dyn-local.md` (§3,
//! §5.3.2). It resolves the *infrastructure-error* half of §1.2's core footgun:
//! today a recoverable `Ok(CapabilityOutcome::Failed)` and a run-terminating
//! `Err(HostRuntimeError)` are "structurally identical — a footgun the
//! loop-capability contract docs record shipping three times." Under the target
//! model those are **two distinct channels**:
//!
//! - recoverable, model-visible failure → `Resolution::Done` with a
//!   recoverable `ToolVerdict` (a later slice), and
//! - infrastructure failure that makes the run unable to continue → this
//!   [`HostFailure`], the `Err` arm of `resolve`/`authorize`/`dispatch`/`abort`
//!   (§3).
//!
//! `HostFailure` is introduced additively ahead of wiring (§9): nothing returns
//! it yet. It is deliberately *not* the recoverable path — a lane-correctable or
//! model-correctable error stays a recoverable outcome (`tools.md`), never a
//! `HostFailure`.

use serde::{Deserialize, Serialize};

use crate::ErrRef;

/// An infrastructure/host failure — the `Err` arm of the kernel capability fold
/// (§3). The variant is the **sanitized recoverability class**; the raw cause is
/// stored host-side and correlated through the carried [`ErrRef`]
/// (error-handling.md: sanitized category plus opaque correlation id, never a
/// raw backend string across the boundary).
///
/// The three classes are recovery-relevant, not cosmetic:
///
/// - [`HostFailure::Transient`] — retryable infra fault (backend unavailable,
///   lock contention); the scheduler may re-run.
/// - [`HostFailure::Permanent`] — a fault that will not resolve on retry
///   (corrupt state, contract violation); fail closed.
/// - [`HostFailure::Uncertain`] — a crash **between dispatch start and outcome
///   record** (§5.3.2): whether the side effect happened is unknown, so recovery
///   must treat it as at-most-once and reconcile from durable evidence rather
///   than blindly retrying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum HostFailure {
    /// Retryable infrastructure fault.
    #[error("transient host failure (ref {0})")]
    Transient(ErrRef),
    /// Non-retryable host fault; fail closed.
    #[error("permanent host failure (ref {0})")]
    Permanent(ErrRef),
    /// Crash between dispatch start and outcome record — side effect unknown
    /// (§5.3.2); reconcile, do not blindly retry.
    #[error("uncertain host failure (ref {0})")]
    Uncertain(ErrRef),
}

impl HostFailure {
    /// The correlation handle to the stored raw cause, regardless of class.
    pub fn err_ref(&self) -> &ErrRef {
        match self {
            HostFailure::Transient(r) | HostFailure::Permanent(r) | HostFailure::Uncertain(r) => r,
        }
    }

    /// Stable recoverability-class discriminant (matches the serde tag) for logs
    /// and recovery routing, without matching on the variant.
    pub fn recoverability(&self) -> &'static str {
        match self {
            HostFailure::Transient(_) => "transient",
            HostFailure::Permanent(_) => "permanent",
            HostFailure::Uncertain(_) => "uncertain",
        }
    }

    /// Whether the scheduler may retry without reconciliation. Only `Transient`
    /// is safe to blindly retry — `Uncertain` requires at-most-once
    /// reconciliation (§5.3.2), `Permanent` never resolves on retry.
    pub fn is_blindly_retryable(&self) -> bool {
        matches!(self, HostFailure::Transient(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_ref() -> ErrRef {
        ErrRef::parse("01890a5d-ac96-774b-bcce-b302099a8057").unwrap()
    }

    #[test]
    fn host_failure_serde_is_snake_case_tagged_and_roundtrips() {
        let failure = HostFailure::Uncertain(err_ref());
        let json = serde_json::to_value(&failure).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "uncertain": "01890a5d-ac96-774b-bcce-b302099a8057" })
        );
        let back: HostFailure = serde_json::from_value(json).unwrap();
        assert_eq!(back, failure);
    }

    #[test]
    fn recoverability_discriminant_matches_serde_tag() {
        for (failure, tag) in [
            (HostFailure::Transient(err_ref()), "transient"),
            (HostFailure::Permanent(err_ref()), "permanent"),
            (HostFailure::Uncertain(err_ref()), "uncertain"),
        ] {
            let wire = serde_json::to_value(&failure).unwrap();
            let tag_on_wire = wire.as_object().unwrap().keys().next().unwrap().clone();
            assert_eq!(failure.recoverability(), tag);
            assert_eq!(tag_on_wire, tag);
        }
    }

    #[test]
    fn only_transient_is_blindly_retryable() {
        // §5.3.2: Uncertain must NOT be blindly retried (side effect unknown),
        // and Permanent never resolves on retry — this pins that contract.
        assert!(HostFailure::Transient(err_ref()).is_blindly_retryable());
        assert!(!HostFailure::Uncertain(err_ref()).is_blindly_retryable());
        assert!(!HostFailure::Permanent(err_ref()).is_blindly_retryable());
    }

    #[test]
    fn err_ref_is_reachable_from_every_class() {
        let r = err_ref();
        assert_eq!(HostFailure::Transient(r).err_ref(), &r);
        assert_eq!(HostFailure::Permanent(r).err_ref(), &r);
        assert_eq!(HostFailure::Uncertain(r).err_ref(), &r);
    }

    #[test]
    fn display_carries_class_and_ref_but_not_raw_cause() {
        // The Display is a sanitized boundary string: class + opaque ref only.
        let shown = HostFailure::Permanent(err_ref()).to_string();
        assert!(shown.contains("permanent"));
        assert!(shown.contains("01890a5d-ac96-774b-bcce-b302099a8057"));
    }

    #[test]
    fn err_ref_structurally_rejects_raw_error_text() {
        // Regression (review finding on the C.2 slice): `HostFailure` serializes
        // and Displays the ref across the sanitized error boundary, so `ErrRef`
        // must be constructible only as a UUID — a call site cannot smuggle a
        // raw backend error string (or a secret) through it.
        for raw in [
            "connection refused: postgres://user:hunter2@db:5432",
            "thread 'main' panicked at src/lib.rs:42",
            "inv-01HZ0000000000000000000000",
            "",
        ] {
            assert!(
                ErrRef::parse(raw).is_err(),
                "raw text must not become an ErrRef: {raw:?}"
            );
        }
        // The sanctioned constructions: host-minted, or carrying an existing
        // host id's UUID verbatim.
        let minted = ErrRef::new();
        assert_eq!(ErrRef::parse(&minted.to_string()).unwrap(), minted);
        let inv = crate::InvocationId::new();
        assert_eq!(ErrRef::from_uuid(inv.as_uuid()).as_uuid(), inv.as_uuid());
    }
}
