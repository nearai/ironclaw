//! Orphaned sandbox container reaper.
//!
//! Sandbox containers are launched per command invocation
//! (`RebornScopedSandboxCommandTransport::execute_in_container`) and normally
//! removed inline once the command finishes. A container can survive that
//! best-effort removal — host crash mid-command, killed process, a lost
//! connection to the daemon — and become an orphan: a running container whose
//! owning invocation is gone. [`SandboxReaper`] periodically lists containers
//! this transport created (via the `ironclaw.*` labels set in
//! `container_launch_config`) and removes the ones that are definitively
//! orphaned.
//!
//! **Deliberate fix over the legacy `src/orchestrator/reaper.rs`:** that
//! implementation treated "the run-state lookup failed" the same as "the run
//! is gone" and reaped in both cases. A transient backend/filesystem error is
//! not evidence of anything — reaping on it can kill a container whose
//! invocation is still very much alive. [`liveness_for`] treats a query error
//! as [`Liveness::Alive`]: the scan skips the container this tick and
//! re-evaluates it next tick, so only a *definitive* not-found or terminal
//! run status is ever grounds for removal.
//!
//! This module is NOT wired into composition yet — spawning [`SandboxReaper::run`]
//! as an owned background task is a later slice (composition/factory.rs).

use std::{collections::HashMap, sync::Arc, time::Duration};

use bollard::{
    Docker,
    container::{ListContainersOptions, RemoveContainerOptions, StopContainerOptions},
    models::ContainerSummary,
};
use chrono::{DateTime, Utc};
use ironclaw_host_api::{InvocationId, ResourceScope};
use ironclaw_run_state::{RunStateStore, RunStatus};

use crate::RuntimeProcessError;

use super::LABEL_PREFIX;

/// Docker label carrying the invocation ID that created the container.
pub(crate) fn label_invocation_id(prefix: &str) -> String {
    format!("{prefix}.invocation_id")
}

/// Docker label carrying the JSON-serialized [`ResourceScope`] the container
/// was launched under.
pub(crate) fn label_resource_scope(prefix: &str) -> String {
    format!("{prefix}.resource_scope")
}

/// Docker label carrying the container's RFC3339 creation timestamp, as
/// recorded by the transport (not Docker's own `Created` field), so orphan
/// age is always attributable to our own labeling scheme.
pub(crate) fn label_created_at(prefix: &str) -> String {
    format!("{prefix}.created_at")
}

/// Builds the `ironclaw.*` Docker labels for a container launched under
/// `scope`. Used by `container_launch_config` in the parent module.
///
/// The `resource_scope` label is the scope's existing `Serialize` impl
/// (`ResourceScope` derives `Serialize`/`Deserialize`) — never reconstructed
/// from ad-hoc string fields, so the reaper's parse side always matches
/// whatever shape `ResourceScope` actually has.
pub(crate) fn build_container_labels(
    scope: &ResourceScope,
) -> Result<HashMap<String, String>, RuntimeProcessError> {
    let resource_scope_json = serde_json::to_string(scope).map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox container labels could not serialize resource scope: {error}"
        ))
    })?;
    let mut labels = HashMap::with_capacity(3);
    labels.insert(
        label_invocation_id(LABEL_PREFIX),
        scope.invocation_id.to_string(),
    );
    labels.insert(label_resource_scope(LABEL_PREFIX), resource_scope_json);
    labels.insert(label_created_at(LABEL_PREFIX), Utc::now().to_rfc3339());
    Ok(labels)
}

/// A parsed, attributable sandbox container: the pieces of the `ironclaw.*`
/// labels the reaper needs to decide liveness and age.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReapCandidate {
    container_id: String,
    invocation_id: InvocationId,
    scope: ResourceScope,
    created_at: DateTime<Utc>,
}

impl ReapCandidate {
    /// Parses a [`ContainerSummary`] into a candidate. Returns `None` when
    /// the container has no ID or its labels are missing/unparseable — such
    /// a container cannot be attributed to an invocation, so the reaper must
    /// leave it alone rather than guess.
    fn from_summary(container: &ContainerSummary, label_prefix: &str) -> Option<Self> {
        let container_id = container.id.clone()?;
        let labels = container.labels.as_ref()?;

        let invocation_id = labels
            .get(&label_invocation_id(label_prefix))
            .and_then(|value| InvocationId::parse(value).ok())?;
        let scope = labels
            .get(&label_resource_scope(label_prefix))
            .and_then(|value| serde_json::from_str::<ResourceScope>(value).ok())?;
        let created_at = labels
            .get(&label_created_at(label_prefix))
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))?;

        Some(Self {
            container_id,
            invocation_id,
            scope,
            created_at,
        })
    }

    fn age(&self, now: DateTime<Utc>) -> Duration {
        (now - self.created_at).to_std().unwrap_or(Duration::ZERO)
    }
}

/// Whether a sandbox container's owning invocation is still alive, as far as
/// the reaper can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Liveness {
    /// The run is still running/blocked, OR the liveness query itself failed
    /// (backend/filesystem error) — never conflate "I don't know" with
    /// "it's gone".
    Alive,
    /// The run-state store definitively has no record for this invocation,
    /// or the record is in a terminal status (`Completed`/`Failed`).
    Orphaned,
}

/// Determines liveness for one invocation by querying `run_state`.
///
/// This is a free function (not a `SandboxReaper` method) so it can be unit
/// tested against a fake [`RunStateStore`] without a Docker connection.
pub(crate) async fn liveness_for(
    run_state: &dyn RunStateStore,
    scope: &ResourceScope,
    invocation_id: InvocationId,
) -> Liveness {
    match run_state.get(scope, invocation_id).await {
        Ok(Some(record)) => match record.status {
            RunStatus::Running | RunStatus::BlockedApproval | RunStatus::BlockedAuth => {
                Liveness::Alive
            }
            RunStatus::Completed | RunStatus::Failed => Liveness::Orphaned,
        },
        // Definitively no record for this invocation: orphan.
        Ok(None) => Liveness::Orphaned,
        // Transient query error: never reap on uncertainty. Skip this scan;
        // the next tick re-evaluates from scratch.
        Err(_) => Liveness::Alive,
    }
}

/// Configuration for [`SandboxReaper`].
#[derive(Debug, Clone)]
pub struct SandboxReaperConfig {
    /// How often the reaper lists containers and evaluates them.
    pub scan_interval: Duration,
    /// Minimum container age (by the `ironclaw.created_at` label) before it
    /// is eligible for reaping, even if definitively orphaned. Guards
    /// against racing a container that is mid-create.
    pub orphan_threshold: Duration,
    /// Prefix for the `ironclaw.*` Docker labels this reaper looks for.
    pub label_prefix: String,
}

impl Default for SandboxReaperConfig {
    fn default() -> Self {
        Self {
            scan_interval: Duration::from_secs(300),
            orphan_threshold: Duration::from_secs(600),
            label_prefix: LABEL_PREFIX.to_string(),
        }
    }
}

/// Summary of one `scan_and_reap` pass, for logging/tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReapSummary {
    pub considered: usize,
    pub reaped: usize,
}

/// Periodically removes orphaned sandbox containers.
///
/// Not wired into composition yet: constructing and spawning `run()` as an
/// owned background task is a later slice.
pub struct SandboxReaper {
    docker: Docker,
    run_state: Arc<dyn RunStateStore>,
    config: SandboxReaperConfig,
}

impl SandboxReaper {
    pub fn new(
        docker: Docker,
        run_state: Arc<dyn RunStateStore>,
        config: SandboxReaperConfig,
    ) -> Self {
        Self {
            docker,
            run_state,
            config,
        }
    }

    /// Runs the scan loop until `shutdown` reports `true`. Composition owns
    /// spawning this as a task and driving `shutdown` — this method itself
    /// has no opinion on how it is scheduled.
    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(self.config.scan_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(error) = self.scan_and_reap().await {
                        tracing::debug!(?error, "sandbox reaper scan failed");
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    }

    /// Lists `ironclaw`-labeled containers, evaluates each against
    /// `orphan_threshold` and [`liveness_for`], and reaps the definitively
    /// orphaned ones. Errors listing containers are propagated (a Docker
    /// connectivity failure means the scan learned nothing, not that
    /// everything is fine); errors reaping an individual container are
    /// best-effort and logged.
    pub async fn scan_and_reap(&self) -> Result<ReapSummary, RuntimeProcessError> {
        let containers = self.list_ironclaw_containers().await?;
        let now = Utc::now();
        let mut summary = ReapSummary::default();

        for container in &containers {
            let Some(candidate) = ReapCandidate::from_summary(container, &self.config.label_prefix)
            else {
                // Unattributable container (missing/unparseable labels): not
                // ours to manage, leave it alone.
                continue;
            };
            summary.considered += 1;

            if candidate.age(now) < self.config.orphan_threshold {
                continue;
            }

            let liveness = liveness_for(
                self.run_state.as_ref(),
                &candidate.scope,
                candidate.invocation_id,
            )
            .await;
            if liveness == Liveness::Orphaned {
                self.reap_container(&candidate.container_id).await;
                summary.reaped += 1;
            }
        }

        tracing::debug!(
            considered = summary.considered,
            reaped = summary.reaped,
            "sandbox reaper scan complete"
        );
        Ok(summary)
    }

    async fn list_ironclaw_containers(&self) -> Result<Vec<ContainerSummary>, RuntimeProcessError> {
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![label_invocation_id(&self.config.label_prefix)],
        );
        self.docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters,
                ..Default::default()
            }))
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox reaper container list failed: {error}"
                ))
            })
    }

    /// Best-effort stop-then-remove, mirroring the removal style already
    /// used inline in `execute_in_container`: never fail the scan over a
    /// single container's teardown error, just log at `debug!` (never
    /// `info!` — background tasks must not write to the REPL surface).
    async fn reap_container(&self, container_id: &str) {
        if let Err(error) = self
            .docker
            .stop_container(container_id, Some(StopContainerOptions { t: 0 }))
            .await
        {
            tracing::debug!(
                ?error,
                container_id,
                "sandbox reaper best-effort stop failed"
            );
        }
        if let Err(error) = self
            .docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            tracing::debug!(
                ?error,
                container_id,
                "sandbox reaper best-effort removal failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_host_api::{CapabilityId, InvocationId, UserId};
    use ironclaw_run_state::{RunRecord, RunStart, RunStateError};

    /// A fake [`RunStateStore`] whose `get` returns one of three canned
    /// answers, so `liveness_for` can be tested without a real filesystem or
    /// Docker daemon. Every other trait method is unreachable from these
    /// tests — the reaper only ever calls `get`.
    enum FixedResponse {
        Found(RunStatus),
        NotFound,
        Error,
    }

    struct FixedRunStateStore(FixedResponse);

    #[async_trait]
    impl RunStateStore for FixedRunStateStore {
        async fn start(&self, _start: RunStart) -> Result<RunRecord, RunStateError> {
            unreachable!("reaper liveness tests never call start()")
        }

        async fn block_approval(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
            _approval: ironclaw_host_api::ApprovalRequest,
        ) -> Result<RunRecord, RunStateError> {
            unreachable!("reaper liveness tests never call block_approval()")
        }

        async fn block_auth(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
            _error_kind: String,
        ) -> Result<RunRecord, RunStateError> {
            unreachable!("reaper liveness tests never call block_auth()")
        }

        async fn complete(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
        ) -> Result<RunRecord, RunStateError> {
            unreachable!("reaper liveness tests never call complete()")
        }

        async fn fail(
            &self,
            _scope: &ResourceScope,
            _invocation_id: InvocationId,
            _error_kind: String,
        ) -> Result<RunRecord, RunStateError> {
            unreachable!("reaper liveness tests never call fail()")
        }

        async fn get(
            &self,
            scope: &ResourceScope,
            invocation_id: InvocationId,
        ) -> Result<Option<RunRecord>, RunStateError> {
            match &self.0 {
                FixedResponse::Found(status) => Ok(Some(RunRecord {
                    invocation_id,
                    capability_id: CapabilityId::new("builtin.shell").unwrap(),
                    scope: scope.clone(),
                    authenticated_actor_user_id: None,
                    status: *status,
                    approval_request_id: None,
                    error_kind: None,
                })),
                FixedResponse::NotFound => Ok(None),
                FixedResponse::Error => Err(RunStateError::Backend(
                    "transient backend failure".to_string(),
                )),
            }
        }

        async fn records_for_scope(
            &self,
            _scope: &ResourceScope,
        ) -> Result<Vec<RunRecord>, RunStateError> {
            unreachable!("reaper liveness tests never call records_for_scope()")
        }
    }

    fn test_scope() -> ResourceScope {
        ResourceScope::local_default(
            UserId::new("reaper-test-user").unwrap(),
            InvocationId::new(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn is_container_live_treats_transient_query_error_as_alive() {
        // HEADLINE regression: a backend/filesystem error from the run-state
        // store must never be treated as "the run is gone". Reaping here
        // would kill a container whose invocation is still alive but whose
        // liveness query merely failed this tick.
        let store = FixedRunStateStore(FixedResponse::Error);
        let scope = test_scope();

        let liveness = liveness_for(&store, &scope, scope.invocation_id).await;

        assert_eq!(liveness, Liveness::Alive);
    }

    #[tokio::test]
    async fn is_container_live_reaps_when_run_definitively_absent() {
        let store = FixedRunStateStore(FixedResponse::NotFound);
        let scope = test_scope();

        let liveness = liveness_for(&store, &scope, scope.invocation_id).await;

        assert_eq!(liveness, Liveness::Orphaned);
    }

    #[tokio::test]
    async fn is_container_live_reaps_when_run_is_terminal() {
        for status in [RunStatus::Completed, RunStatus::Failed] {
            let store = FixedRunStateStore(FixedResponse::Found(status));
            let scope = test_scope();

            let liveness = liveness_for(&store, &scope, scope.invocation_id).await;

            assert_eq!(liveness, Liveness::Orphaned, "status: {status:?}");
        }
    }

    #[tokio::test]
    async fn is_container_live_stays_alive_while_running_or_blocked() {
        for status in [
            RunStatus::Running,
            RunStatus::BlockedApproval,
            RunStatus::BlockedAuth,
        ] {
            let store = FixedRunStateStore(FixedResponse::Found(status));
            let scope = test_scope();

            let liveness = liveness_for(&store, &scope, scope.invocation_id).await;

            assert_eq!(liveness, Liveness::Alive, "status: {status:?}");
        }
    }

    #[test]
    fn list_ironclaw_containers_parses_scope_and_invocation_labels() {
        let scope = test_scope();
        let labels = build_container_labels(&scope).unwrap();
        let container = ContainerSummary {
            id: Some("abc123".to_string()),
            labels: Some(labels),
            ..Default::default()
        };

        let candidate = ReapCandidate::from_summary(&container, LABEL_PREFIX)
            .expect("round-tripped labels must parse back into a candidate");

        assert_eq!(candidate.container_id, "abc123");
        assert_eq!(candidate.invocation_id, scope.invocation_id);
        assert_eq!(candidate.scope, scope);
    }

    #[test]
    fn missing_labels_do_not_parse_into_a_candidate() {
        let container = ContainerSummary {
            id: Some("no-labels".to_string()),
            labels: None,
            ..Default::default()
        };

        assert!(ReapCandidate::from_summary(&container, LABEL_PREFIX).is_none());
    }

    #[test]
    fn unparseable_resource_scope_label_does_not_parse_into_a_candidate() {
        let scope = test_scope();
        let container = ContainerSummary {
            id: Some("bad-scope".to_string()),
            labels: Some(HashMap::from([
                (
                    label_invocation_id(LABEL_PREFIX),
                    scope.invocation_id.to_string(),
                ),
                (
                    label_resource_scope(LABEL_PREFIX),
                    "not valid json".to_string(),
                ),
                (label_created_at(LABEL_PREFIX), Utc::now().to_rfc3339()),
            ])),
            ..Default::default()
        };

        assert!(ReapCandidate::from_summary(&container, LABEL_PREFIX).is_none());
    }

    #[test]
    fn default_config_matches_plan_thresholds() {
        let config = SandboxReaperConfig::default();

        assert_eq!(config.scan_interval, Duration::from_secs(300));
        assert_eq!(config.orphan_threshold, Duration::from_secs(600));
        assert_eq!(config.label_prefix, "ironclaw");
    }
}
