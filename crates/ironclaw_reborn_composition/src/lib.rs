//! Production composition root for IronClaw Reborn.
//!
//! The Reborn host service graph is intentionally split into many crates with
//! tight dependency boundaries (see `crates/ironclaw_architecture`). This crate
//! is the single place where those crates are wired together into a usable host
//! service graph, called by `src/app.rs::AppBuilder` when an explicit Reborn
//! profile is selected.
//!
//! # Profiles
//!
//! [`RebornProfile`] is an explicit four-state switch:
//!
//! - `Disabled` (default) — the legacy startup path runs. This crate is a
//!   no-op: nothing in `RebornProductionServices` is constructed and no
//!   substrate is wired.
//! - `LocalDev` — explicit dev/test profile. Allows in-memory and reference
//!   filesystem-backed stores so a developer can boot the full graph without
//!   provisioning a database. Readiness reports `LocalDev`, never
//!   "production-ready".
//! - `Production` — full fail-closed graph. Every required factory must be
//!   present and validate before the factory returns. Missing or invalid
//!   substrate is fatal at startup, never a silent in-memory fallback.
//! - `MigrationDryRun` — validates schemas/factories/config without serving
//!   traffic. Channels and loops are not exposed.
//!
//! # Substrate landing
//!
//! Reborn substrate crates land incrementally. This crate's factories are
//! permitted to fail with [`RebornBuildError::SubstrateNotImplemented`] when a
//! required service crate has not yet merged. That keeps the composition
//! root reviewable today without blocking on the full set of cutover-blocker
//! PRs (#3013, #3016, #3019, #3022 plus the secrets/network/memory/
//! capabilities/dispatcher crates that are still in flight).
//!
//! # What this crate is *not*
//!
//! It is not a replacement for `AppBuilder`. `AppBuilder` continues to own
//! database, secrets, LLM, tools, and channel orchestration. This crate
//! composes module-owned Reborn factories on top of what `AppBuilder` has
//! already initialised, and stashes the result on `AppComponents` so the rest
//! of the runtime can reach it through a single typed handle.

use std::sync::Arc;

use ironclaw_approvals::ApprovalResolutionError;
use ironclaw_authorization::CapabilityLeaseError;
use ironclaw_events::EventError;
use ironclaw_extensions::ExtensionError;
use ironclaw_filesystem::FilesystemError;
use ironclaw_host_api::HostApiError;
use ironclaw_resources::ResourceError;
use ironclaw_run_state::RunStateError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod factories;
mod profile;

pub use profile::{RebornProfile, RebornProfileParseError};

/// Inputs required to build a Reborn production service graph.
///
/// The struct is intentionally minimal — it carries only what the composition
/// root needs to call into module-owned factories. `AppBuilder` resolves these
/// from already-initialised state (config, DB, secrets) so this crate never
/// touches env/dotenv/keychain directly.
pub struct RebornBuildInput {
    /// Explicit profile selected by the operator. Construction of the service
    /// graph branches on this — see [`RebornProfile`] for the contract.
    pub profile: RebornProfile,
    /// Owner/admin scope used when reading typed settings. Mirrors
    /// `Config::owner_id`.
    pub owner_id: String,
}

/// Output of [`build_reborn_production_services`].
///
/// Each field is `Option` so partial graphs are explicit. Under
/// [`RebornProfile::Production`] every required field must be `Some` before
/// this struct is returned — incomplete graphs fail at construction with
/// [`RebornBuildError::SubstrateNotImplemented`] or
/// [`RebornBuildError::MissingRequired`].
///
/// Fields are added incrementally as substrate crates merge. The currently
/// populated fields cover the substrate already present in the workspace
/// (`ironclaw_authorization`, `ironclaw_run_state`, `ironclaw_approvals`,
/// `ironclaw_resources`, `ironclaw_events`, `ironclaw_filesystem`,
/// `ironclaw_extensions`). Capability host, process host, dispatcher,
/// turn coordinator, agent loop host, memory, secrets, and network services
/// are reserved for follow-up PRs and surface as
/// [`RebornBuildError::SubstrateNotImplemented`] when missing.
#[derive(Default)]
pub struct RebornProductionServices {
    /// Profile actually used to build the graph. Echoed back so the readiness
    /// surface in `AppComponents` can report it without re-reading config.
    pub profile: RebornProfile,
    /// Resource governor (already merged via `ironclaw_resources`).
    pub resource_governor: Option<Arc<dyn ironclaw_resources::ResourceGovernor>>,
    /// Authorization service for capability dispatch (`ironclaw_authorization`).
    pub authorization: Option<Arc<dyn ironclaw_authorization::CapabilityDispatchAuthorizer>>,
    /// Capability lease store shared with the approval resolver.
    pub capability_lease_store: Option<Arc<dyn ironclaw_authorization::CapabilityLeaseStore>>,
    /// Run-state store for thread/turn admission and durable lifecycle.
    pub run_state_store: Option<Arc<dyn ironclaw_run_state::RunStateStore>>,
    /// Approval request store shared with the approval resolver.
    pub approval_request_store: Option<Arc<dyn ironclaw_run_state::ApprovalRequestStore>>,
    /// Durable runtime event log (`ironclaw_events`).
    pub event_log: Option<Arc<dyn ironclaw_events::DurableEventLog>>,
    /// Durable audit log (`ironclaw_events`).
    pub audit_log: Option<Arc<dyn ironclaw_events::DurableAuditLog>>,
    /// Filesystem root used by scoped filesystem and run-state stores.
    pub filesystem_root: Option<Arc<dyn ironclaw_filesystem::RootFilesystem>>,
    /// Extension registry contracts (`ironclaw_extensions`).
    pub extension_registry: Option<Arc<ironclaw_extensions::ExtensionRegistry>>,
}

impl std::fmt::Debug for RebornProductionServices {
    // Trait objects in the slots don't all carry Debug, and a derived
    // impl would force every substrate trait to grow a Debug bound. The
    // hand-rolled version reports per-slot wired/unwired so test
    // assertions and operator diagnostics stay usable without leaking
    // the underlying handle's Debug — which could include host paths.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let wired = |slot: bool| if slot { "wired" } else { "unwired" };
        f.debug_struct("RebornProductionServices")
            .field("profile", &self.profile)
            .field(
                "resource_governor",
                &wired(self.resource_governor.is_some()),
            )
            .field("authorization", &wired(self.authorization.is_some()))
            .field(
                "capability_lease_store",
                &wired(self.capability_lease_store.is_some()),
            )
            .field("run_state_store", &wired(self.run_state_store.is_some()))
            .field(
                "approval_request_store",
                &wired(self.approval_request_store.is_some()),
            )
            .field("event_log", &wired(self.event_log.is_some()))
            .field("audit_log", &wired(self.audit_log.is_some()))
            .field("filesystem_root", &wired(self.filesystem_root.is_some()))
            .field(
                "extension_registry",
                &wired(self.extension_registry.is_some()),
            )
            .finish()
    }
}

impl RebornProductionServices {
    /// Sentinel value used by `AppBuilder` when [`RebornProfile::Disabled`] is
    /// selected. No substrate is wired and the legacy startup path remains
    /// authoritative.
    pub fn disabled() -> Self {
        Self {
            profile: RebornProfile::Disabled,
            ..Self::default()
        }
    }

    /// True when this graph was built under an explicit dev/test profile and
    /// must not be reported as production-ready by readiness/health surfaces.
    pub fn is_dev_only(&self) -> bool {
        matches!(
            self.profile,
            RebornProfile::LocalDev | RebornProfile::MigrationDryRun
        )
    }

    /// Operator-visible readiness summary.
    ///
    /// Issue #3026 acceptance criterion #14 requires that "health/
    /// readiness diagnostics expose mode/profile/backend readiness
    /// without leaking credentials or raw host paths." The
    /// [`RebornReadiness`] surface is the typed value an HTTP `/health`
    /// or status-CLI handler renders; it carries only profile, the
    /// readiness state, and a per-slot wired/unwired flag.
    ///
    /// The surface intentionally does not name the *concrete* backend
    /// (`Postgres`, `LibSql`, `InMemory`, host paths). Operators who
    /// need that detail should consult logs or a separate operator-only
    /// endpoint that gates on authentication. Splitting the public
    /// readiness from operator detail keeps this surface safe to expose
    /// to an unauthenticated `/health` route.
    pub fn readiness(&self) -> RebornReadiness {
        let state = if self.profile == RebornProfile::Disabled {
            RebornReadinessState::Disabled
        } else if self.is_dev_only() {
            RebornReadinessState::DevOnly
        } else {
            RebornReadinessState::ProductionReady
        };
        RebornReadiness {
            profile: self.profile,
            state,
            slots: RebornSlotReadiness {
                resource_governor: self.resource_governor.is_some(),
                authorization: self.authorization.is_some(),
                capability_lease_store: self.capability_lease_store.is_some(),
                run_state_store: self.run_state_store.is_some(),
                approval_request_store: self.approval_request_store.is_some(),
                event_log: self.event_log.is_some(),
                audit_log: self.audit_log.is_some(),
                filesystem_root: self.filesystem_root.is_some(),
                extension_registry: self.extension_registry.is_some(),
            },
        }
    }

    /// Cross-handle coupling and contract checks.
    ///
    /// Issue #3026 acceptance criterion #4 requires that "Reborn production
    /// mode validates the full required service graph before serving
    /// traffic." The full graph contract is only knowable once every
    /// substrate crate exists — most don't yet. What this method validates
    /// today is the subset of contracts that *can* be checked against the
    /// merged substrate:
    ///
    /// 1. `Disabled` → every slot must be empty. Catches a future bug
    ///    where a factory accidentally writes into the disabled graph and
    ///    leaks a partial Reborn island into the legacy startup path.
    /// 2. Authorization and capability lease store are coupled: the lease
    ///    store is what the lease-backed authorizer reads from, and the
    ///    approval resolver writes leases into it. Either both are present
    ///    or both are absent; one without the other is a wiring bug.
    /// 3. Run-state store and approval-request store are coupled for the
    ///    same reason: `ApprovalResolver` resolves requests by writing to
    ///    both, and `TurnCoordinator` (when it lands) shares both.
    /// 4. Event log and audit log are coupled: every audit-bearing flow
    ///    also emits runtime events, and replay is broken if only one
    ///    side is durable.
    /// 5. Filesystem root is required whenever extension registry is
    ///    populated, because `ExtensionDiscovery` resolves manifests
    ///    against a root.
    ///
    /// Future contracts (CapabilityHost ↔ ProcessHost shared store,
    /// TurnCoordinator ↔ AgentLoopHost shared scoped services, prompt
    /// write-safety hook present whenever filesystem is) plug in here as
    /// each substrate crate lands.
    ///
    /// Returns [`RebornBuildError::InvalidConfig`] when a coupling rule is
    /// violated. The reason string is redaction-safe by construction —
    /// it names rule labels, never substrate state.
    pub fn validate(&self) -> Result<(), RebornBuildError> {
        if self.profile == RebornProfile::Disabled {
            // Rule 1: a disabled graph must be entirely empty. Any wired
            // slot here means a factory ran when it shouldn't have.
            let any_wired = self.resource_governor.is_some()
                || self.authorization.is_some()
                || self.capability_lease_store.is_some()
                || self.run_state_store.is_some()
                || self.approval_request_store.is_some()
                || self.event_log.is_some()
                || self.audit_log.is_some()
                || self.filesystem_root.is_some()
                || self.extension_registry.is_some();
            if any_wired {
                return Err(RebornBuildError::InvalidConfig {
                    reason: "disabled profile produced a non-empty service graph".to_string(),
                });
            }
            return Ok(());
        }

        // Rule 2: authorization ↔ capability lease store coupling.
        if self.authorization.is_some() != self.capability_lease_store.is_some() {
            return Err(RebornBuildError::InvalidConfig {
                reason: "authorization and capability_lease_store must be wired together \
                         (lease-backed authorization needs the same lease store the resolver writes to)"
                    .to_string(),
            });
        }

        // Rule 3: run-state store ↔ approval-request store coupling.
        if self.run_state_store.is_some() != self.approval_request_store.is_some() {
            return Err(RebornBuildError::InvalidConfig {
                reason: "run_state_store and approval_request_store must be wired together \
                         (TurnCoordinator and ApprovalResolver share both)"
                    .to_string(),
            });
        }

        // Rule 4: event log ↔ audit log coupling.
        if self.event_log.is_some() != self.audit_log.is_some() {
            return Err(RebornBuildError::InvalidConfig {
                reason: "event_log and audit_log must be wired together \
                         (every audit-bearing flow also emits runtime events)"
                    .to_string(),
            });
        }

        // Rule 5: extension registry requires filesystem root.
        if self.extension_registry.is_some() && self.filesystem_root.is_none() {
            return Err(RebornBuildError::InvalidConfig {
                reason: "extension_registry requires filesystem_root \
                         (ExtensionDiscovery resolves manifests against a root)"
                    .to_string(),
            });
        }

        Ok(())
    }
}

/// Top-level readiness state reported by [`RebornProductionServices::readiness`].
///
/// Three discrete values mirror the contract in #3026's "Explicit mode and
/// profile behavior" section. `Degraded` is intentionally absent today —
/// once durable backends exist, a partial-but-running production graph
/// will surface as a fourth variant rather than overloading these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RebornReadinessState {
    /// Reborn composition is off. Legacy startup path is authoritative.
    Disabled,
    /// Reborn composition is up but the profile is explicitly local/dev/
    /// test. Must never be reported as production-ready.
    DevOnly,
    /// Reborn composition is up under `Production` and every required
    /// service for the current build slate is wired. The "current build
    /// slate" expands as substrate crates land — this state does not by
    /// itself mean cutover-ready until every cutover-blocker substrate is
    /// in the workspace.
    ProductionReady,
}

/// Per-slot wired/unwired flags. Each field corresponds to a slot on
/// [`RebornProductionServices`].
///
/// Booleans only — no concrete backend name, no host path, no connection
/// string. Operators see "wired" or "unwired" per slot; the *what* of the
/// backend lives in operator-only logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornSlotReadiness {
    pub resource_governor: bool,
    pub authorization: bool,
    pub capability_lease_store: bool,
    pub run_state_store: bool,
    pub approval_request_store: bool,
    pub event_log: bool,
    pub audit_log: bool,
    pub filesystem_root: bool,
    pub extension_registry: bool,
}

/// Operator-visible readiness summary for the Reborn composition graph.
///
/// Returned by [`RebornProductionServices::readiness`] and rendered by
/// the HTTP `/health` / status-CLI handler. Redaction-safe by
/// construction: every field is a typed value that cannot carry secret
/// material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornReadiness {
    pub profile: RebornProfile,
    pub state: RebornReadinessState,
    pub slots: RebornSlotReadiness,
}

/// Failures from [`build_reborn_production_services`].
///
/// `Display` is intentionally redaction-safe: variants name the missing
/// service or reason without leaking host paths, connection strings,
/// credentials, or raw secret material. Operators get an actionable name;
/// detailed lower-level errors are emitted through `tracing` separately.
#[derive(Debug, Error)]
pub enum RebornBuildError {
    /// Production profile was selected but a required substrate crate has
    /// not yet merged. The string names the missing service so operators
    /// can correlate with the cutover-blocker tracking issues.
    #[error("reborn production substrate not yet implemented: {service}")]
    SubstrateNotImplemented { service: &'static str },

    /// A required service exists in this build but its factory could not
    /// produce a usable instance under the selected profile.
    #[error("reborn required service '{service}' could not be built: {reason}")]
    MissingRequired {
        service: &'static str,
        reason: String,
    },

    /// Configuration combination is invalid for the selected profile.
    /// Examples: `Production` + in-memory event backend, or `LocalDev`
    /// without an explicit allowance.
    #[error("invalid reborn configuration: {reason}")]
    InvalidConfig { reason: String },

    /// Underlying host-api contract violation surfaced from one of the
    /// substrate crates.
    #[error(transparent)]
    HostApi(#[from] HostApiError),

    #[error(transparent)]
    Filesystem(#[from] FilesystemError),

    #[error(transparent)]
    Resource(#[from] ResourceError),

    #[error(transparent)]
    Event(#[from] EventError),

    #[error(transparent)]
    Extension(#[from] ExtensionError),

    #[error(transparent)]
    RunState(#[from] RunStateError),

    #[error(transparent)]
    CapabilityLease(#[from] CapabilityLeaseError),

    #[error(transparent)]
    Approval(#[from] ApprovalResolutionError),
}

/// Build the Reborn production service graph for the given input.
///
/// Behavior by [`RebornProfile`]:
///
/// - [`RebornProfile::Disabled`] — returns [`RebornProductionServices::disabled`]
///   immediately. No substrate is wired.
/// - [`RebornProfile::LocalDev`] — wires already-merged substrate using
///   in-memory / reference filesystem backends. Future substrate crates that
///   have not yet merged surface as
///   [`RebornBuildError::SubstrateNotImplemented`] only when their corresponding
///   feature is explicitly required by the input.
/// - [`RebornProfile::Production`] — wires already-merged substrate with
///   durable backends. Any required service whose substrate has not yet
///   merged fails with [`RebornBuildError::SubstrateNotImplemented`]. This is
///   the fail-closed default once Reborn is the production path.
/// - [`RebornProfile::MigrationDryRun`] — validates factories without exposing
///   any traffic-serving surface. Same construction as `Production` but the
///   readiness surface explicitly reports dry-run state.
pub async fn build_reborn_production_services(
    input: RebornBuildInput,
) -> Result<RebornProductionServices, RebornBuildError> {
    if input.profile == RebornProfile::Disabled {
        return Ok(RebornProductionServices::disabled());
    }

    tracing::info!(
        profile = %input.profile,
        owner_id = %input.owner_id,
        "Building Reborn production service graph"
    );

    let mut services = RebornProductionServices {
        profile: input.profile,
        ..RebornProductionServices::default()
    };

    factories::resources::build(&input, &mut services)?;
    factories::events::build(&input, &mut services)?;
    factories::filesystem::build(&input, &mut services)?;
    factories::run_state::build(&input, &mut services)?;
    factories::auth::build(&input, &mut services)?;
    factories::extensions::build(&input, &mut services)?;

    // Substrate gates: required production services that are not yet merged.
    // Each call returns Ok(()) under LocalDev/MigrationDryRun (the dev/test
    // profiles tolerate partial graphs by design) and
    // SubstrateNotImplemented under Production. Once the corresponding crate
    // lands, the factory module replaces the gate with a real builder.
    factories::capabilities::build(&input, &mut services)?;
    factories::processes::build(&input, &mut services)?;
    factories::dispatcher::build(&input, &mut services)?;
    factories::secrets::build(&input, &mut services)?;
    factories::network::build(&input, &mut services)?;
    factories::memory::build(&input, &mut services)?;
    factories::trust::build(&input, &mut services)?;
    factories::turns::build(&input, &mut services)?;
    factories::agent_loop::build(&input, &mut services)?;
    factories::prompt_safety::build(&input, &mut services)?;

    // Cross-handle coupling validation. Every successful build runs this
    // before returning — issue #3026 AC #4 requires the full required
    // service graph be validated before serving traffic. Rules expand as
    // substrate lands; today they cover the merged substrate's contracts.
    services.validate()?;

    Ok(services)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(profile: RebornProfile) -> RebornBuildInput {
        RebornBuildInput {
            profile,
            owner_id: "test-owner".to_string(),
        }
    }

    #[tokio::test]
    async fn disabled_profile_returns_empty_services() {
        let services = build_reborn_production_services(input(RebornProfile::Disabled))
            .await
            .expect("disabled must succeed");
        assert_eq!(services.profile, RebornProfile::Disabled);
        assert!(services.resource_governor.is_none());
        assert!(services.authorization.is_none());
        assert!(services.event_log.is_none());
    }

    #[tokio::test]
    async fn local_dev_wires_already_merged_substrate() {
        let services = build_reborn_production_services(input(RebornProfile::LocalDev))
            .await
            .expect("local-dev must succeed with merged substrate only");
        assert_eq!(services.profile, RebornProfile::LocalDev);
        assert!(services.resource_governor.is_some(), "resources merged");
        assert!(services.event_log.is_some(), "events merged");
        assert!(services.audit_log.is_some(), "audit log merged");
        assert!(services.filesystem_root.is_some(), "filesystem merged");
        assert!(services.run_state_store.is_some(), "run state store merged");
        assert!(
            services.approval_request_store.is_some(),
            "approval request store merged"
        );
        assert!(services.authorization.is_some(), "authorization merged");
        assert!(
            services.capability_lease_store.is_some(),
            "capability lease store merged"
        );
        assert!(
            services.extension_registry.is_some(),
            "extension registry merged"
        );
        assert!(services.is_dev_only());
    }

    #[tokio::test]
    async fn production_fails_on_missing_substrate() {
        let err = build_reborn_production_services(input(RebornProfile::Production))
            .await
            .expect_err("production must fail closed when substrate is missing");
        match err {
            RebornBuildError::SubstrateNotImplemented { service } => {
                // Any of the not-yet-merged substrate crates is acceptable.
                // The first gate to fail wins, so we just assert that one of
                // the documented gates triggered.
                // Whichever gate fires first wins. The list mirrors every
                // currently-gated service so adding a new gate above will
                // not silently break this test.
                assert!(
                    [
                        "durable_event_backend",
                        "durable_run_state_backend",
                        "ironclaw_capabilities",
                        "ironclaw_processes",
                        "ironclaw_dispatcher",
                        "ironclaw_secrets",
                        "ironclaw_network",
                        "ironclaw_memory",
                        "trust_class_policy",
                        "turn_coordinator",
                        "agent_loop_host",
                        "prompt_write_safety_policy",
                    ]
                    .contains(&service),
                    "unexpected missing service: {service}"
                );
            }
            other => panic!("expected SubstrateNotImplemented, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn migration_dry_run_fails_closed_like_production() {
        // MigrationDryRun shares Production's full-graph requirement so
        // that a config destined for production fails the same way under
        // dry run before it ever reaches a live deployment. The dev-only
        // reporting bit only matters once a build actually succeeds (which
        // requires every substrate to be wired) — covered by integration
        // tests added once the gates flip into real builders.
        let err = build_reborn_production_services(input(RebornProfile::MigrationDryRun))
            .await
            .expect_err("dry run must fail closed when substrate is missing");
        assert!(matches!(
            err,
            RebornBuildError::SubstrateNotImplemented { .. }
        ));
    }

    #[test]
    fn build_error_display_does_not_leak_sensitive_detail() {
        let err = RebornBuildError::SubstrateNotImplemented {
            service: "ironclaw_secrets",
        };
        let rendered = err.to_string();
        // The Display string must name the missing service. It must not
        // contain anything that resembles a credential, host path, or
        // connection string — the variant only carries a static service id.
        assert!(rendered.contains("ironclaw_secrets"));
        assert!(!rendered.contains("/Users/"));
        assert!(!rendered.contains("postgres://"));
    }

    // ── validate() coupling rules (AC #4) ────────────────────────────────

    fn local_dev_services() -> RebornProductionServices {
        // A graph identical to what `build_reborn_production_services`
        // produces under LocalDev, used as the baseline against which we
        // test the coupling rules.
        RebornProductionServices {
            profile: RebornProfile::LocalDev,
            resource_governor: Some(Arc::new(ironclaw_resources::InMemoryResourceGovernor::new())),
            authorization: Some(Arc::new(ironclaw_authorization::GrantAuthorizer::new())),
            capability_lease_store: Some(Arc::new(
                ironclaw_authorization::InMemoryCapabilityLeaseStore::new(),
            )),
            run_state_store: Some(Arc::new(ironclaw_run_state::InMemoryRunStateStore::new())),
            approval_request_store: Some(Arc::new(
                ironclaw_run_state::InMemoryApprovalRequestStore::new(),
            )),
            event_log: Some(Arc::new(ironclaw_events::InMemoryDurableEventLog::new())),
            audit_log: Some(Arc::new(ironclaw_events::InMemoryDurableAuditLog::new())),
            filesystem_root: Some(Arc::new(ironclaw_filesystem::CompositeRootFilesystem::new())),
            extension_registry: Some(Arc::new(ironclaw_extensions::ExtensionRegistry::new())),
        }
    }

    #[test]
    fn validate_accepts_disabled_empty_graph() {
        let services = RebornProductionServices::disabled();
        services.validate().expect("disabled empty graph must pass");
    }

    #[test]
    fn validate_rejects_disabled_with_wired_slot() {
        // Rule 1: any wired slot under Disabled is a factory bug.
        let services = RebornProductionServices {
            profile: RebornProfile::Disabled,
            resource_governor: Some(Arc::new(ironclaw_resources::InMemoryResourceGovernor::new())),
            ..RebornProductionServices::default()
        };
        let err = services
            .validate()
            .expect_err("disabled with a wired slot must fail");
        match err {
            RebornBuildError::InvalidConfig { reason } => {
                assert!(reason.contains("disabled profile"));
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn validate_accepts_local_dev_full_graph() {
        let services = local_dev_services();
        services
            .validate()
            .expect("local-dev full graph must pass coupling rules");
    }

    #[test]
    fn validate_rejects_authorization_without_lease_store() {
        // Rule 2: authorization + capability_lease_store must be wired
        // together. Lease-backed authorization needs the same store the
        // resolver writes leases into.
        let mut services = local_dev_services();
        services.capability_lease_store = None;
        let err = services
            .validate()
            .expect_err("authorization without lease store must fail");
        match err {
            RebornBuildError::InvalidConfig { reason } => {
                assert!(reason.contains("authorization"));
                assert!(reason.contains("capability_lease_store"));
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_run_state_without_approval_request() {
        // Rule 3: run_state_store + approval_request_store must be wired
        // together (TurnCoordinator + ApprovalResolver share both).
        let mut services = local_dev_services();
        services.approval_request_store = None;
        let err = services
            .validate()
            .expect_err("run state without approval store must fail");
        match err {
            RebornBuildError::InvalidConfig { reason } => {
                assert!(reason.contains("run_state_store"));
                assert!(reason.contains("approval_request_store"));
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_event_without_audit() {
        // Rule 4: event_log + audit_log coupling.
        let mut services = local_dev_services();
        services.audit_log = None;
        let err = services
            .validate()
            .expect_err("event without audit must fail");
        match err {
            RebornBuildError::InvalidConfig { reason } => {
                assert!(reason.contains("event_log"));
                assert!(reason.contains("audit_log"));
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_extension_registry_without_filesystem() {
        // Rule 5: extension_registry requires filesystem_root.
        let mut services = local_dev_services();
        services.filesystem_root = None;
        let err = services
            .validate()
            .expect_err("extensions without filesystem must fail");
        match err {
            RebornBuildError::InvalidConfig { reason } => {
                assert!(reason.contains("extension_registry"));
                assert!(reason.contains("filesystem_root"));
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    // ── Readiness surface (AC #14) ───────────────────────────────────────

    #[test]
    fn readiness_for_disabled_reports_disabled_state_and_empty_slots() {
        let r = RebornProductionServices::disabled().readiness();
        assert_eq!(r.profile, RebornProfile::Disabled);
        assert_eq!(r.state, RebornReadinessState::Disabled);
        assert!(!r.slots.resource_governor);
        assert!(!r.slots.event_log);
        assert!(!r.slots.extension_registry);
    }

    #[test]
    fn readiness_for_local_dev_reports_dev_only() {
        let services = local_dev_services();
        let r = services.readiness();
        assert_eq!(r.profile, RebornProfile::LocalDev);
        assert_eq!(
            r.state,
            RebornReadinessState::DevOnly,
            "LocalDev must never be reported as production-ready"
        );
        // Every slot is wired in the LocalDev baseline.
        assert!(r.slots.resource_governor);
        assert!(r.slots.authorization);
        assert!(r.slots.capability_lease_store);
        assert!(r.slots.run_state_store);
        assert!(r.slots.approval_request_store);
        assert!(r.slots.event_log);
        assert!(r.slots.audit_log);
        assert!(r.slots.filesystem_root);
        assert!(r.slots.extension_registry);
    }

    #[test]
    fn readiness_serialization_does_not_leak_sensitive_fields() {
        // The JSON rendering of the readiness surface is what an HTTP
        // /health handler emits. Confirm field names cannot match
        // anything that suggests credential material — a future
        // refactor that added an operator-only field with the wrong
        // name would fail this test.
        let r = local_dev_services().readiness();
        let rendered = serde_json::to_string(&r).expect("serialize readiness");
        let lc = rendered.to_ascii_lowercase();
        for forbidden in ["api_key", "secret", "password", "token", "credential"] {
            assert!(
                !lc.contains(forbidden),
                "readiness serialization contains forbidden token '{forbidden}': {rendered}"
            );
        }
        assert!(
            !rendered.contains("/Users/"),
            "readiness leaked a host path: {rendered}"
        );
        assert!(
            !rendered.contains("postgres://"),
            "readiness leaked a connection string: {rendered}"
        );
    }

    #[test]
    fn readiness_state_serializes_in_kebab_case() {
        // Wire-stable serialization. The HTTP /health surface is part
        // of the operator contract; renaming a variant would break the
        // dashboard.
        assert_eq!(
            serde_json::to_string(&RebornReadinessState::Disabled).unwrap(),
            "\"disabled\""
        );
        assert_eq!(
            serde_json::to_string(&RebornReadinessState::DevOnly).unwrap(),
            "\"dev-only\""
        );
        assert_eq!(
            serde_json::to_string(&RebornReadinessState::ProductionReady).unwrap(),
            "\"production-ready\""
        );
    }

    #[test]
    fn validate_diagnostics_are_redaction_safe() {
        // Every InvalidConfig variant the rules can emit goes through a
        // static reason string. Confirm none of them carry anything that
        // could leak host paths, credentials, or connection strings.
        let cases: Vec<RebornProductionServices> = vec![
            RebornProductionServices {
                profile: RebornProfile::Disabled,
                resource_governor: Some(Arc::new(
                    ironclaw_resources::InMemoryResourceGovernor::new(),
                )),
                ..RebornProductionServices::default()
            },
            {
                let mut s = local_dev_services();
                s.capability_lease_store = None;
                s
            },
            {
                let mut s = local_dev_services();
                s.approval_request_store = None;
                s
            },
            {
                let mut s = local_dev_services();
                s.audit_log = None;
                s
            },
            {
                let mut s = local_dev_services();
                s.filesystem_root = None;
                s
            },
        ];
        for services in cases {
            let err = services
                .validate()
                .expect_err("test cases are intentional violations");
            let rendered = err.to_string();
            assert!(!rendered.contains("/Users/"), "leaked host path: {rendered}");
            assert!(
                !rendered.contains("postgres://"),
                "leaked connection string: {rendered}"
            );
            assert!(
                !rendered.to_ascii_lowercase().contains("api_key"),
                "leaked credential token: {rendered}"
            );
        }
    }
}
