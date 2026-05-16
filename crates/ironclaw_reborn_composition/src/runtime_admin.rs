//! Admin surface of the assembled Reborn runtime.
//!
//! `RebornRuntime` exposes two surfaces: an **agent surface** (`new_conversation`,
//! `send_user_message`, `shutdown`) that channels and the CLI's interactive REPL
//! use, and an **admin surface** ([`RebornAdminClient`]) used by privileged
//! operations like blueprint apply / harness install / harness activation that
//! land in epic #3036.
//!
//! Why a separate handle:
//!
//! 1. **Privilege gating.** The admin handle is obtained from
//!    `RebornRuntime::admin(scope: RebornAdminScope)`. The scope is sealed —
//!    only the composition root and the CLI binary can construct it (via the
//!    fixed [`RebornAdminScope::for_cli_admin`] constructor today, replaced by
//!    a credential-aware constructor when the privilege/auth crate lands).
//!    Channels and remote handlers cannot escalate by accident.
//! 2. **Surface stability.** Sticking admin methods directly on
//!    `RebornRuntime` clutters the agent surface (the thing channels use).
//!    Separate handle keeps the agent surface to ~3 methods even as admin
//!    grows to a dozen.
//! 3. **Audit attribution.** Every admin method carries the `audit_reason`
//!    string from the scope into the eventual `CapabilityHost::invoke_json`
//!    so the audit trail records *why* the operator invoked it. This matches
//!    the v1 `src/tenant.rs::AdminScope` pattern called out in epic #3036.
//!
//! Today every admin method returns [`RebornAdminError::NotYetWired`] with a
//! clear pointer to the epic sub-issue gating it. The shape is locked
//! (boundary tests pin the public surface) so the substrate work can land
//! incrementally without breaking the CLI command tree.

use std::fmt;

use thiserror::Error;

use crate::runtime_input::RebornHarnessId;

/// Sealed admin-privilege witness. Obtaining one is restricted to the CLI
/// binary (and, in the future, an authenticated remote admin path that the
/// privilege/auth crate will provide).
#[derive(Debug, Clone)]
pub struct RebornAdminScope {
    audit_reason: String,
    /// Sealed marker — outside callers cannot construct this type because
    /// they cannot name the field. The two public constructors below are
    /// the only legitimate entry points.
    _sealed: SealedMarker,
}

#[derive(Debug, Clone)]
struct SealedMarker;

impl RebornAdminScope {
    /// Construct an admin scope for a CLI-driven invocation.
    ///
    /// The audit reason is logged on every admin write performed under
    /// this scope. This constructor is the **only** valid privilege gate
    /// in the standalone Reborn binary today. When the privilege/auth
    /// crate from epic #3036 lands, this signature will gain a credential
    /// parameter and remote admin paths will get their own constructor
    /// (`for_authenticated_admin(...)`).
    pub fn for_cli_admin(audit_reason: impl Into<String>) -> Self {
        Self {
            audit_reason: audit_reason.into(),
            _sealed: SealedMarker,
        }
    }

    pub fn audit_reason(&self) -> &str {
        &self.audit_reason
    }
}

/// Privileged operations on the assembled Reborn runtime. Obtain via
/// `RebornRuntime::admin(scope)`.
///
/// Every method is `NotYetWired` today; tracking issue
/// [#3036](https://github.com/nearai/ironclaw/issues/3036) drives the
/// substrate that makes them live.
#[derive(Debug, Clone)]
pub struct RebornAdminClient {
    scope: RebornAdminScope,
}

impl RebornAdminClient {
    /// Crate-private constructor. The agent-side `RebornRuntime::admin(scope)`
    /// method is the only legitimate caller. External crates cannot construct
    /// this directly because the constructor is `pub(crate)`.
    pub(crate) fn new(scope: RebornAdminScope) -> Self {
        Self { scope }
    }

    /// Audit reason recorded against this client's scope.
    pub fn audit_reason(&self) -> &str {
        self.scope.audit_reason()
    }

    /// Apply a parsed blueprint to the runtime's typed repos.
    ///
    /// Tracking: epic #3036 sub-issue "Blueprint apply service".
    pub async fn apply_blueprint(
        &self,
        request: ApplyBlueprintRequest,
    ) -> Result<ApplyReport, RebornAdminError> {
        let _ = request;
        Err(RebornAdminError::NotYetWired {
            operation: "config.apply",
            tracking_issue: "#3036",
            requires: "BlueprintParser + BlueprintApplyService + typed repos (Settings, Skill, Mission, Project)",
        })
    }

    /// Compute drift between a parsed blueprint and the runtime's typed
    /// repos. Read-only; never writes.
    ///
    /// Tracking: epic #3036 sub-issue "Blueprint diff".
    pub async fn diff_blueprint(
        &self,
        request: DiffBlueprintRequest,
    ) -> Result<ApplyReport, RebornAdminError> {
        let _ = request;
        Err(RebornAdminError::NotYetWired {
            operation: "config.diff",
            tracking_issue: "#3036",
            requires: "BlueprintParser + BlueprintApplyService.diff()",
        })
    }

    /// Install a harness manifest into the typed harness repo.
    ///
    /// Tracking: epic #3036 sub-issue "HarnessManifest typed repo + lifecycle".
    pub async fn install_harness(
        &self,
        request: InstallHarnessRequest,
    ) -> Result<RebornHarnessId, RebornAdminError> {
        let _ = request;
        Err(RebornAdminError::NotYetWired {
            operation: "harness.install",
            tracking_issue: "#3036",
            requires: "HarnessManifest parser + HarnessRepo",
        })
    }

    /// Activate an installed harness for a scope (session/thread/project).
    ///
    /// Tracking: epic #3036 sub-issue "HarnessManifest typed repo + lifecycle"
    /// + "InstructionBundleAssembler with overlay".
    pub async fn activate_harness(
        &self,
        request: ActivateHarnessRequest,
    ) -> Result<(), RebornAdminError> {
        let _ = request;
        Err(RebornAdminError::NotYetWired {
            operation: "harness.activate",
            tracking_issue: "#3036",
            requires: "HarnessActivationService + InstructionBundleAssembler overlay path + capability-surface filter",
        })
    }

    /// Deactivate the active harness for a scope.
    ///
    /// Tracking: epic #3036 sub-issue "HarnessManifest typed repo + lifecycle".
    pub async fn deactivate_harness(
        &self,
        request: DeactivateHarnessRequest,
    ) -> Result<(), RebornAdminError> {
        let _ = request;
        Err(RebornAdminError::NotYetWired {
            operation: "harness.deactivate",
            tracking_issue: "#3036",
            requires: "HarnessActivationService",
        })
    }

    /// List installed harnesses.
    ///
    /// Tracking: epic #3036 sub-issue "HarnessManifest typed repo + lifecycle".
    pub async fn list_harnesses(&self) -> Result<Vec<HarnessDescriptor>, RebornAdminError> {
        Err(RebornAdminError::NotYetWired {
            operation: "harness.list",
            tracking_issue: "#3036",
            requires: "HarnessRepo",
        })
    }
}

// ─── Request / response DTOs ────────────────────────────────────────────────

/// Apply mode for [`RebornAdminClient::apply_blueprint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// Compute the report but perform no writes.
    DryRun,
    /// Compute and execute writes inside per-repo transactions.
    Apply,
}

/// Caller-supplied blueprint apply request. Held intentionally opaque
/// today — the parsed-blueprint DTO will live in the future
/// `ironclaw_blueprint` crate (epic #3036 sub-issue "Blueprint format +
/// parser"). For now this is a path/url placeholder so the admin surface
/// already takes a typed argument and won't break when the parser lands.
#[derive(Debug, Clone)]
pub struct ApplyBlueprintRequest {
    /// Path or git URL to the blueprint source. Will be replaced by a
    /// `ParsedBlueprint` value once the parser crate exists; the input
    /// to this method becomes the parsed value, and CLI does the parse
    /// step itself.
    pub source: BlueprintSource,
    pub mode: ApplyMode,
    /// Optional lockfile path. The apply service hashes file refs into
    /// this lockfile so re-applies are deterministic across machines.
    pub lockfile: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone)]
pub struct DiffBlueprintRequest {
    pub source: BlueprintSource,
}

#[derive(Debug, Clone)]
pub enum BlueprintSource {
    Path(std::path::PathBuf),
    GitUrl(String),
}

impl fmt::Display for BlueprintSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => write!(formatter, "{}", path.display()),
            Self::GitUrl(url) => formatter.write_str(url),
        }
    }
}

/// Report produced by `apply_blueprint` / `diff_blueprint`.
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    pub changes: Vec<ApplyChange>,
}

/// A single change row in an [`ApplyReport`]. Matches the shape epic #3036
/// specifies for `Change { domain, key, action, before_hash, after_hash }`.
#[derive(Debug, Clone)]
pub struct ApplyChange {
    pub domain: String,
    pub key: String,
    pub action: ApplyChangeAction,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApplyChangeAction {
    Create,
    Update,
    NoOp,
    DeleteDeferred,
}

#[derive(Debug, Clone)]
pub struct InstallHarnessRequest {
    pub manifest_source: BlueprintSource,
}

#[derive(Debug, Clone)]
pub struct ActivateHarnessRequest {
    pub harness_id: RebornHarnessId,
    pub scope: HarnessActivationScope,
}

#[derive(Debug, Clone)]
pub struct DeactivateHarnessRequest {
    pub scope: HarnessActivationScope,
}

/// Where to install/activate the harness. One-active-per-thread invariant
/// from epic #3036 is enforced by the activation service when it lands.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HarnessActivationScope {
    Thread { thread_id: String },
    Project { project_id: String },
    Tenant { tenant_id: String },
}

#[derive(Debug, Clone)]
pub struct HarnessDescriptor {
    pub id: RebornHarnessId,
    pub installed_revision: Option<String>,
    pub active_for_threads: Vec<String>,
}

// ─── Errors ─────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum RebornAdminError {
    /// Operation is recognized but its substrate hasn't landed yet. The
    /// payload identifies the tracking issue and the missing prerequisite
    /// so an operator hitting this gets a precise next-step pointer.
    #[error(
        "reborn admin operation `{operation}` is recognized but not yet wired; \
         tracking: {tracking_issue}; requires: {requires}"
    )]
    NotYetWired {
        operation: &'static str,
        tracking_issue: &'static str,
        requires: &'static str,
    },
    /// Reserved for the future privilege/auth crate — when admin-scope
    /// construction grows a credential parameter, callers passing an
    /// unauthenticated or expired credential get this.
    #[error("reborn admin operation rejected: {reason}")]
    Unauthorized { reason: String },
    /// Repo-side transactional failure surfaced verbatim.
    #[error("reborn admin repo error: {reason}")]
    Repo { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_scope_carries_audit_reason() {
        let scope = RebornAdminScope::for_cli_admin("smoke test");
        assert_eq!(scope.audit_reason(), "smoke test");
    }

    #[tokio::test]
    async fn every_admin_method_returns_not_yet_wired() {
        let client = RebornAdminClient::new(RebornAdminScope::for_cli_admin("unit"));

        let apply = client
            .apply_blueprint(ApplyBlueprintRequest {
                source: BlueprintSource::Path("/tmp/x".into()),
                mode: ApplyMode::DryRun,
                lockfile: None,
            })
            .await;
        assert!(matches!(apply, Err(RebornAdminError::NotYetWired { .. })));

        let diff = client
            .diff_blueprint(DiffBlueprintRequest {
                source: BlueprintSource::Path("/tmp/x".into()),
            })
            .await;
        assert!(matches!(diff, Err(RebornAdminError::NotYetWired { .. })));

        let install = client
            .install_harness(InstallHarnessRequest {
                manifest_source: BlueprintSource::Path("/tmp/x".into()),
            })
            .await;
        assert!(matches!(install, Err(RebornAdminError::NotYetWired { .. })));

        let activate = client
            .activate_harness(ActivateHarnessRequest {
                harness_id: RebornHarnessId::new("red-team").unwrap(),
                scope: HarnessActivationScope::Thread {
                    thread_id: "t".into(),
                },
            })
            .await;
        assert!(matches!(activate, Err(RebornAdminError::NotYetWired { .. })));

        let deactivate = client
            .deactivate_harness(DeactivateHarnessRequest {
                scope: HarnessActivationScope::Thread {
                    thread_id: "t".into(),
                },
            })
            .await;
        assert!(matches!(
            deactivate,
            Err(RebornAdminError::NotYetWired { .. })
        ));

        let list = client.list_harnesses().await;
        assert!(matches!(list, Err(RebornAdminError::NotYetWired { .. })));
    }
}
