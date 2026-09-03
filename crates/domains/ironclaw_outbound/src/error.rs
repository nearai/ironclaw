use thiserror::Error;

#[derive(Debug, Error)]
pub enum OutboundError {
    #[error("outbound state backend unavailable")]
    Backend,
    #[error("outbound state serialization failed")]
    Serialization,
    #[error("outbound state request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    /// The creator's communication preference does not include a delivery
    /// target for the requested notification kind. Kept channel-neutral:
    /// callers map this to transport-specific handling.
    #[error("communication preference target is missing: {kind}")]
    PreferenceTargetMissing { kind: &'static str },
    #[error("subscription cursor scope mismatch")]
    SubscriptionScopeMismatch,
    #[error("outbound access denied")]
    AccessDenied,
    #[error("outbound delivery not found")]
    DeliveryNotFound,
    #[error("reply attachment intents are already sealed for this run")]
    ReplyAttachmentIntentsSealed,
    #[error("reply attachment metadata conflicts with an existing path")]
    ReplyAttachmentIntentConflict,
    #[error("reply attachment intent budget exceeded")]
    ReplyAttachmentIntentLimitExceeded,
    /// Compare-and-swap precondition failed on the underlying filesystem. The
    /// caller observed a stale `RecordVersion`; a bounded retry loop should
    /// re-read the current entry and re-apply the transformation. Distinct
    /// from [`OutboundError::Backend`] so retry loops can match on the typed
    /// variant rather than collapsing transient races into a permanent
    /// failure. Stays internal to the crate — converted to
    /// [`OutboundError::Backend`] before returning to a caller once the
    /// bounded retry budget is exhausted.
    #[error("outbound state compare-and-swap conflict")]
    CasConflict,
    /// A guarded reply publication write carried a fence other than the
    /// current one: another publisher claimed the lease since. `expected_fence`
    /// is the fence the caller wrote with; `actual_fence` the stored one.
    #[error(
        "reply publisher is stale: wrote with fence {expected_fence}, current fence is {actual_fence}"
    )]
    StaleReplyPublisher {
        expected_fence: u64,
        actual_fence: u64,
    },
    /// The publication already settled; settlement is one-way.
    #[error("reply publication is already settled")]
    ReplyPublicationSettled,
    /// A revision counter would move backwards or cross: `published` is the
    /// durable (or limiting) value, `requested` the offending one.
    #[error("reply publication revision regressed: durable {published}, requested {requested}")]
    ReplyPublicationRevisionRegressed { published: u64, requested: u64 },
    /// No publication substate exists under that delivery id.
    #[error("reply publication not found")]
    ReplyPublicationNotFound,
    /// The delivery id already carries a publication for another target, or a
    /// plain one-shot attempt that cannot be adopted.
    #[error("reply publication target does not match the existing attempt")]
    ReplyPublicationTargetMismatch,
    /// `Delivered` was requested before the terminal revision was applied.
    #[error("reply publication has not applied its terminal revision")]
    ReplyPublicationNotTerminal,
    /// A guarded write needs a lease (claim first).
    #[error("reply publication lease is required")]
    ReplyPublicationLeaseRequired,
}
