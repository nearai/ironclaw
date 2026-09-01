//! Progressive reply publication state on the outbound delivery attempt
//! aggregate.
//!
//! A one-shot send claims an attempt once (`Prepared -> Sending`) and settles
//! it once. Progressive publication reconciles a provider toward successive
//! desired revisions of one reply over time, so it does **not** reuse the
//! send claim as its lease: the persisted attempt row carries a
//! serde-defaulted [`ReplyPublicationState`] substate with its own ownership
//! (lease + monotonic fence), monotonic revision counters, the sink's
//! generation-pinned checkpoint, bounded provider evidence, and a one-way
//! settlement. The store exposes it only through the guarded operations on
//! [`crate::OutboundStateStorePort`] and the [`ReplyPublicationRecord`] view —
//! the public [`OutboundDeliveryAttempt`] shape is unchanged, and rows written
//! before the substate existed keep behaving as one-shot sends.
//!
//! Design record: `docs/internal/design/2026-08-31-progressive-reply-publication.md`
//! §5.

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use ironclaw_extension_contracts::channel::ReplyTransport;
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::reply::{
    ReplyAudience, ReplyOutcomeReason, ReplyProviderRefs, ReplySinkCheckpoint, ReplyThreadAnchor,
};
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::turn::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

use crate::{
    DeliveryFailureKind, OutboundDeliveryAttempt, OutboundDeliveryId, OutboundDeliveryStatus,
    OutboundError,
};

/// Byte bound shared by [`ReplyPublicationTargetKey`] and [`PublisherId`].
pub const REPLY_PUBLICATION_IDENTIFIER_MAX_BYTES: usize = 128;

/// Why a reply publication identifier was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplyPublicationIdentifierError {
    #[error("{field} must not be empty")]
    Empty { field: &'static str },
    #[error("{field} exceeds {max} bytes")]
    TooLong { field: &'static str, max: usize },
    #[error("{field} may contain only ASCII alphanumerics, '.', '_', ':' and '-'")]
    InvalidCharacter { field: &'static str },
}

fn validate_identifier(
    field: &'static str,
    value: &str,
) -> Result<(), ReplyPublicationIdentifierError> {
    if value.is_empty() {
        return Err(ReplyPublicationIdentifierError::Empty { field });
    }
    if value.len() > REPLY_PUBLICATION_IDENTIFIER_MAX_BYTES {
        return Err(ReplyPublicationIdentifierError::TooLong {
            field,
            max: REPLY_PUBLICATION_IDENTIFIER_MAX_BYTES,
        });
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(ReplyPublicationIdentifierError::InvalidCharacter { field });
    }
    Ok(())
}

/// Validated identifier newtype (`.claude/rules/types.md` template): 1..=128
/// bytes of ASCII alphanumerics plus `.`, `_`, `:` and `-`, validated at
/// construction and on the wire alike. Deliberately no `From<String>` /
/// `From<&str>` and no `Deref<Target = str>`.
macro_rules! publication_identifier {
    ($(#[$doc:meta])* $name:ident, $field:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            fn validate(value: &str) -> Result<(), ReplyPublicationIdentifierError> {
                validate_identifier($field, value)
            }

            pub fn new(raw: impl Into<String>) -> Result<Self, ReplyPublicationIdentifierError> {
                let value = raw.into();
                Self::validate(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ReplyPublicationIdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::validate(&value)?;
                Ok(Self(value))
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

publication_identifier!(
    /// The exact provider-neutral target a reply is published to (a channel
    /// conversation plus its thread anchor, a browser session, …), as the
    /// publication owner keys it. Two publications of one run to different
    /// targets are different aggregates; the same target under the same
    /// delivery id is one.
    ReplyPublicationTargetKey,
    "reply publication target key"
);

publication_identifier!(
    /// The identity of one publication worker instance holding (or asking
    /// for) a lease. Distinct from every actor/user identity: it names a
    /// process-local worker, never a person.
    PublisherId,
    "reply publisher id"
);

/// Identity of one publication: the reply's run plus the exact target.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReplyPublicationTarget {
    pub run_id: TurnRunId,
    pub key: ReplyPublicationTargetKey,
}

/// Everything a publisher on *any* node needs to address the target again:
/// the channel, the run's actor, the authorized reply-target binding, the
/// vendor conversation and thread anchor, the audience, and the declared
/// cadence. Persisted at open so a publication interrupted by a crash can be
/// resumed by whichever process observes the run's terminal commit — without
/// re-resolving anything from display strings or transport metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyPublicationTargetDescriptor {
    pub extension_id: ExtensionId,
    pub actor: TurnActor,
    pub reply_target: ReplyTargetBindingRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<ExternalConversationRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_anchor: Option<ReplyThreadAnchor>,
    pub audience: ReplyAudience,
    pub transport: ReplyTransport,
    /// The adapter's stored ingress reply context (ING-11) as it stood when
    /// the target was registered — snapshotted per run because the
    /// per-conversation store is latest-wins and a newer top-level DM in the
    /// same conversation would otherwise re-thread an older run's reply.
    /// Bounded to the seam's reply-context limit at registration. `None`
    /// means the target had no stored context when it was registered; a
    /// resume never re-reads the mutable store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_context: Option<Vec<u8>>,
}

/// Who may write publication state right now, and until when. The fence on
/// the enclosing [`ReplyPublicationState`] — not the clock — guards writes:
/// a lapsed lease nobody took over still belongs to its fenced owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyPublicationLease {
    pub owner: PublisherId,
    pub expires_at: DateTime<Utc>,
}

/// Provider evidence recorded with each advance. Bounded by construction
/// (`ReplyProviderRefs` is at most 32 × 256 bytes; the outcome reason folds
/// to 512 bytes).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyPublicationEvidence {
    #[serde(default)]
    pub provider_refs: ReplyProviderRefs,
    #[serde(default)]
    pub read_back_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<ReplyOutcomeReason>,
    /// The extension generation changed underneath the checkpoint since the
    /// previous advance.
    #[serde(default)]
    pub generation_changed: bool,
}

/// How a publication ended. `Delivered` is only reachable once the terminal
/// revision was applied; `Unknown` records an unverifiable terminal
/// reconcile; `Failed(kind)` a permanent, unauthorized, or abandoned one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "failure_kind")]
pub enum ReplyPublicationSettlement {
    Delivered,
    Unknown,
    Failed(DeliveryFailureKind),
}

impl ReplyPublicationSettlement {
    /// The attempt status this settlement writes on the aggregate.
    pub fn attempt_status(self) -> OutboundDeliveryStatus {
        match self {
            Self::Delivered => OutboundDeliveryStatus::Delivered,
            Self::Unknown => OutboundDeliveryStatus::Unknown,
            Self::Failed(_) => OutboundDeliveryStatus::Failed,
        }
    }

    /// The failure kind this settlement writes on the aggregate.
    pub fn failure_kind(self) -> Option<DeliveryFailureKind> {
        match self {
            Self::Delivered | Self::Unknown => None,
            Self::Failed(kind) => Some(kind),
        }
    }
}

/// Whether the publication still accepts advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "settlement")]
pub enum ReplyPublicationStatus {
    Active,
    Settled(ReplyPublicationSettlement),
}

impl ReplyPublicationStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// The publication substate persisted on one delivery attempt row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyPublicationState {
    pub target: ReplyPublicationTarget,
    /// How to address the target from any node (see the type). Absent only
    /// on rows a publisher opened without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<ReplyPublicationTargetDescriptor>,
    /// Monotonic ownership epoch; bumped on every successful claim by a new
    /// owner (a same-owner re-entry keeps it). Guarded writes carry it.
    pub fence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<ReplyPublicationLease>,
    /// The newest revision the projection wants published. Monotonic.
    pub desired_revision: u64,
    /// The newest revision the sink has applied. Monotonic and never above
    /// `desired_revision`.
    pub published_revision: u64,
    /// The desired revision that carries the terminal document. Set once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_revision: Option<u64>,
    /// The extension generation the checkpoint was minted under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<ReplySinkCheckpoint>,
    #[serde(default)]
    pub evidence: ReplyPublicationEvidence,
    pub status: ReplyPublicationStatus,
    pub updated_at: DateTime<Utc>,
}

impl ReplyPublicationState {
    /// A freshly opened publication: no owner yet, nothing published.
    pub fn opened(target: ReplyPublicationTarget, now: DateTime<Utc>) -> Self {
        Self {
            target,
            descriptor: None,
            fence: 0,
            lease: None,
            desired_revision: 0,
            published_revision: 0,
            terminal_revision: None,
            generation: None,
            checkpoint: None,
            evidence: ReplyPublicationEvidence::default(),
            status: ReplyPublicationStatus::Active,
            updated_at: now,
        }
    }

    /// The lease, if one exists and has not expired at `now`.
    pub fn live_lease(&self, now: DateTime<Utc>) -> Option<&ReplyPublicationLease> {
        self.lease.as_ref().filter(|lease| lease.expires_at > now)
    }

    /// True once the terminal revision is known and has been applied — the
    /// precondition for settling `Delivered`.
    pub fn terminal_applied(&self) -> bool {
        self.terminal_revision
            .is_some_and(|terminal| self.published_revision == terminal)
    }
}

/// One publication read back from the store: the attempt it rides on plus
/// its publication substate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyPublicationRecord {
    pub attempt: OutboundDeliveryAttempt,
    pub publication: ReplyPublicationState,
}

/// Open (or re-open) a publication on a `Prepared` attempt. Idempotent for
/// the same delivery id and the same target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenReplyPublicationRequest {
    /// Must be `OutboundDeliveryStatus::Prepared`.
    pub attempt: OutboundDeliveryAttempt,
    pub target: ReplyPublicationTarget,
    /// Stored on first open; a re-open keeps the stored one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<ReplyPublicationTargetDescriptor>,
    pub now: DateTime<Utc>,
}

/// Acquire (or re-enter) the publication lease for `owner` until
/// `now + ttl`. Same-owner re-entry keeps the fence and extends the expiry —
/// it doubles as the heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimReplyPublicationLeaseRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub owner: PublisherId,
    /// Must be positive.
    pub ttl: Duration,
    pub now: DateTime<Utc>,
}

/// Outcome of a lease claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyPublicationClaim {
    /// The caller owns the lease; the record carries the fence to write with.
    Acquired(ReplyPublicationRecord),
    /// Another publisher holds a live lease.
    Held {
        owner: PublisherId,
        expires_at: DateTime<Utc>,
    },
    /// The publication already settled; nothing is left to publish.
    Settled(ReplyPublicationRecord),
}

/// Record progress under `fence`: revisions move forward only, the terminal
/// revision is set once, `checkpoint: None` keeps the previous checkpoint,
/// `generation` and `evidence` are stored as given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceReplyPublicationRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub fence: u64,
    pub desired_revision: u64,
    pub published_revision: u64,
    pub terminal_revision: Option<u64>,
    pub generation: Option<u64>,
    pub checkpoint: Option<ReplySinkCheckpoint>,
    pub evidence: ReplyPublicationEvidence,
    pub now: DateTime<Utc>,
}

/// End the publication under `fence`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettleReplyPublicationRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub fence: u64,
    pub settlement: ReplyPublicationSettlement,
    pub now: DateTime<Utc>,
}

/// Give the lease up under `fence` without settling; the fence is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseReplyPublicationLeaseRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub fence: u64,
}

/// The expiry a lease claimed or renewed at `now` for `ttl` gets. A zero or
/// unrepresentable `ttl` is a caller bug, refused before any store I/O.
pub(crate) fn lease_expires_at(
    now: DateTime<Utc>,
    ttl: Duration,
) -> Result<DateTime<Utc>, OutboundError> {
    if ttl.is_zero() {
        return Err(OutboundError::InvalidRequest {
            reason: "reply publication lease ttl must be positive",
        });
    }
    let ttl = chrono::Duration::from_std(ttl).map_err(|error| {
        tracing::debug!(error = %error, "reply publication lease ttl is out of range");
        OutboundError::InvalidRequest {
            reason: "reply publication lease ttl is out of range",
        }
    })?;
    now.checked_add_signed(ttl)
        .ok_or(OutboundError::InvalidRequest {
            reason: "reply publication lease expiry is out of range",
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_hold_the_grammar_by_construction_and_on_the_wire() {
        for valid in [
            "a",
            "slack:C123:1712.0001",
            "web_session-42.x",
            &"k".repeat(128),
        ] {
            assert!(
                ReplyPublicationTargetKey::new(valid).is_ok(),
                "{valid:?} is a valid key"
            );
            assert!(PublisherId::new(valid).is_ok(), "{valid:?} is a valid id");
        }
        for (invalid, expected) in [
            ("", "must not be empty"),
            (&"k".repeat(129), "exceeds 128 bytes"),
            ("has space", "may contain only"),
            ("slash/", "may contain only"),
            ("ünïcode", "may contain only"),
        ] {
            let error = ReplyPublicationTargetKey::new(invalid).unwrap_err();
            assert!(error.to_string().contains(expected), "{invalid:?}: {error}");
            let wire = serde_json::to_string(invalid).unwrap();
            assert!(
                serde_json::from_str::<PublisherId>(&wire).is_err(),
                "{invalid:?} must be refused on the wire"
            );
        }
        let key = ReplyPublicationTargetKey::new("slack:C1:1.2").unwrap();
        let wire = serde_json::to_string(&key).unwrap();
        assert_eq!(wire, "\"slack:C1:1.2\"");
        assert_eq!(
            serde_json::from_str::<ReplyPublicationTargetKey>(&wire).unwrap(),
            key
        );
    }

    #[test]
    fn lease_expiry_requires_a_positive_representable_ttl() {
        let now = Utc::now();
        assert!(matches!(
            lease_expires_at(now, Duration::ZERO),
            Err(OutboundError::InvalidRequest { .. })
        ));
        assert!(matches!(
            lease_expires_at(now, Duration::MAX),
            Err(OutboundError::InvalidRequest { .. })
        ));
        assert_eq!(
            lease_expires_at(now, Duration::from_secs(30)).unwrap(),
            now + chrono::Duration::seconds(30)
        );
    }

    #[test]
    fn settlement_and_status_have_a_stable_wire_shape() {
        let failed = ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Failed(
            DeliveryFailureKind::RateLimited,
        ));
        let json = serde_json::to_value(failed).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "state": "settled",
                "settlement": { "kind": "failed", "failure_kind": "rate_limited" }
            })
        );
        assert_eq!(
            serde_json::from_value::<ReplyPublicationStatus>(json).unwrap(),
            failed
        );
        assert_eq!(
            serde_json::to_value(ReplyPublicationStatus::Active).unwrap(),
            serde_json::json!({ "state": "active" })
        );
        assert_eq!(
            serde_json::to_value(ReplyPublicationStatus::Settled(
                ReplyPublicationSettlement::Delivered
            ))
            .unwrap(),
            serde_json::json!({ "state": "settled", "settlement": { "kind": "delivered" } })
        );
        assert_eq!(
            ReplyPublicationSettlement::Failed(DeliveryFailureKind::Rejected).attempt_status(),
            OutboundDeliveryStatus::Failed
        );
        assert_eq!(
            ReplyPublicationSettlement::Failed(DeliveryFailureKind::Rejected).failure_kind(),
            Some(DeliveryFailureKind::Rejected)
        );
        assert_eq!(
            ReplyPublicationSettlement::Unknown.attempt_status(),
            OutboundDeliveryStatus::Unknown
        );
        assert_eq!(ReplyPublicationSettlement::Delivered.failure_kind(), None);
    }

    #[test]
    fn opened_state_round_trips_and_omits_absent_optionals() {
        let now = Utc::now();
        let state = ReplyPublicationState::opened(
            ReplyPublicationTarget {
                run_id: TurnRunId::new(),
                key: ReplyPublicationTargetKey::new("target").unwrap(),
            },
            now,
        );
        assert!(state.status.is_active());
        assert!(state.live_lease(now).is_none());
        assert!(!state.terminal_applied());
        let json = serde_json::to_value(&state).unwrap();
        for absent in ["lease", "terminal_revision", "generation", "checkpoint"] {
            assert!(json.get(absent).is_none(), "{absent} must be omitted");
        }
        assert_eq!(
            serde_json::from_value::<ReplyPublicationState>(json).unwrap(),
            state
        );
    }
}
