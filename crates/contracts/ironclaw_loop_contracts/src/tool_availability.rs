//! Tool availability port: can the acting user use a tool right now?
//!
//! A tool the model sees is already *authorized*: the host's visible surface
//! only lists capabilities that are registered, belong to an installed and
//! activated extension, and pass authorization for the run's scope. An
//! authorized tool can still be unusable, most often because its extension
//! needs an account the user has not connected. [`ToolAvailabilityPredicate`]
//! answers that last question for a batch of authorized tools.
//!
//! The loop host asks it when it chooses which tools to advertise (turn-start
//! tool selection today). A tool the predicate rules out is only left out of
//! the advertised list: it stays in the authorized catalog, reachable through
//! `tool_search` → `tool_call`, and calling it is what surfaces the
//! extension's connect/setup prompt. The predicate never grants or removes
//! authority.
//!
//! # Three answers, not two
//!
//! A lookup can fail or be slow. [`ToolAvailability::Unknown`] keeps "the
//! lookup did not answer" apart from "the tool is definitely unusable"
//! ([`ToolAvailability::Unavailable`]). Callers choose what each means: a
//! caller admitting new tools treats `Unknown` as unavailable (fail closed),
//! while a caller deciding whether to withdraw a tool it already advertised
//! keeps it on `Unknown`, so a transient outage never withdraws a tool for
//! good.

use std::{collections::BTreeMap, fmt};

use async_trait::async_trait;
use ironclaw_host_api::ids::CapabilityId;

use crate::LoopRunContext;

/// How long one availability lookup may take before its answer counts as
/// [`ToolAvailability::Unknown`].
pub const TOOL_AVAILABILITY_LOOKUP_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(300);

/// Whether the acting user can use one authorized tool right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAvailability {
    /// Usable: every mandatory credential is ready, or none is needed.
    Available,
    /// Definitely not usable right now.
    Unavailable(ToolUnavailableReason),
    /// The lookup failed or timed out; the answer is not known.
    Unknown,
}

/// Why a tool is definitely not usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolUnavailableReason {
    /// A mandatory credential has no configured account for the acting user.
    CredentialMissing,
}

impl ToolUnavailableReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CredentialMissing => "credential_missing",
        }
    }
}

/// Answers [`ToolAvailability`] for authorized tools. Composition supplies the
/// implementation; the loop host consumes it without knowing how credentials
/// or extensions are stored.
///
/// Contract:
///
/// - The result holds one entry per requested id. A missing entry counts as
///   [`ToolAvailability::Unknown`].
/// - Each lookup that needs I/O is bounded by
///   [`TOOL_AVAILABILITY_LOOKUP_TIMEOUT`] and reports `Unknown` when it runs
///   out, so one slow credential store only affects the tools that depend on
///   it.
/// - Implementations must not log credential material or account details.
#[async_trait]
pub trait ToolAvailabilityPredicate: Send + Sync + fmt::Debug {
    async fn availability(
        &self,
        run_context: &LoopRunContext,
        capability_ids: &[CapabilityId],
    ) -> BTreeMap<CapabilityId, ToolAvailability>;
}
