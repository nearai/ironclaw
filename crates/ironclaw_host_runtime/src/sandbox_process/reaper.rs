//! Persistent per-user sandbox container reaper.
//!
//! The sandboxed profile now runs one long-lived container per `{tenant,
//! user}` pair (see `super::exec_transport`), reused across every shell
//! invocation rather than recreated per command. A container therefore
//! belongs to a *user*, not an invocation, and its lifecycle is a two-stage
//! idle/retention policy instead of invocation-liveness:
//!
//! 1. **Idle-stop**: a *running* container with no recorded activity for
//!    `idle_stop_after` is stopped (not removed) — the user's workspace and
//!    container state are preserved for the next command.
//! 2. **Retention-remove**: a *stopped* container whose `finished_at` is
//!    older than `remove_stopped_after` is removed outright.
//!
//! A **forced recycle** age (`forced_recycle_after`, measured from the
//! container's `created_at` label) overrides both stages as a janitor
//! backstop: past that age a running container is stopped and a stopped one
//! is removed immediately, regardless of idle/retention windows.
//!
//! [`decide_reap_action`] is the pure decision function driving all of this
//! — deliberately free of I/O so it can be exhaustively unit tested on a
//! fake clock. **Never reap on uncertainty**: an unknown idle duration (no
//! activity record — e.g. after a process restart lost the in-memory map)
//! or an unknown `finished_at` (can't attribute a stop time) always yields
//! [`ReapAction::None`]. Containers whose `ironclaw.tenant`/`ironclaw.user`
//! labels are missing or fail to parse are skipped entirely by
//! [`SandboxReaper::scan_and_reap`] — never reaped — since they cannot be
//! attributed to a user record at all.
//!
//! "Orphan sweep" (a user has no live record at all) is deliberately not
//! this module's job: in the fixed-owner single-tenant profile users never
//! disappear, so that sweep has no trigger yet — it is a named follow-up
//! for a future multi-user profile.
//!
//! **Idle-stop veto (live background jobs).** The activity registry only
//! records *foreground* exec activity, so a `background:true` dev server
//! started once and then left running never touches it again — a naive
//! idle-stop would hard-kill a live server out from under the user. Before
//! [`SandboxReaper::scan_and_reap`] acts on an idle-stop verdict it runs a
//! live `ps` probe against the container (see
//! [`SandboxReaper::probe_live_background`]); a surviving non-PID-1 process
//! vetoes the stop for this scan. The probe never gates forced recycle,
//! which stays unconditional — nothing runs past `forced_recycle_after`
//! regardless of what is live inside it. A probe failure counts as "live"
//! (never reap on uncertainty).
//!
//! **Remove debounce.** `docker rm` is destructive and irreversible, so
//! [`ReapAction::Remove`] only executes once the *same* container id has
//! produced a `Remove` verdict on two consecutive scans (see
//! [`RemoveDebounce`]) — one bad sweep (a label race, a transient inspect
//! failure shape) marks the container pending instead of removing it
//! outright. `Stop` is not debounced: a stopped container is trivially
//! restartable, so there is no comparable risk to guard against.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use bollard::{
    Docker,
    container::{ListContainersOptions, RemoveContainerOptions, StopContainerOptions},
    models::ContainerSummary,
};
use chrono::{DateTime, Utc};
use ironclaw_host_api::{TenantId, UserId};

use crate::RuntimeProcessError;

use super::ContainerWorkdir;
use super::LABEL_PREFIX;
use super::exec_transport;
use super::registry::{self, SandboxActivityRegistry, UserContainerCandidate};
use super::user_key::RebornSandboxUserKey;

/// Overrides [`SandboxReaperConfig::idle_stop_after`]. Unset, empty,
/// non-numeric, or zero falls back to the default.
const IDLE_STOP_AFTER_ENV: &str = "IRONCLAW_SANDBOX_IDLE_STOP_SECS";
/// Overrides [`SandboxReaperConfig::remove_stopped_after`].
const REMOVE_STOPPED_AFTER_ENV: &str = "IRONCLAW_SANDBOX_REMOVE_STOPPED_SECS";
/// Overrides [`SandboxReaperConfig::forced_recycle_after`].
const FORCED_RECYCLE_AFTER_ENV: &str = "IRONCLAW_SANDBOX_FORCED_RECYCLE_SECS";

const DEFAULT_IDLE_STOP_AFTER_SECS: u64 = 15 * 60;
const DEFAULT_REMOVE_STOPPED_AFTER_SECS: u64 = 7 * 24 * 3600;
const DEFAULT_FORCED_RECYCLE_AFTER_SECS: u64 = 7 * 24 * 3600;

/// Pure resolution of a duration-in-seconds config knob from an
/// already-read raw env value, falling back to `default_secs` when the
/// value is absent, empty, non-numeric, or zero. Kept separate from the env
/// read itself (mirrors `sandbox_quota::resolve_sandbox_max_concurrent_from_raw`)
/// so tests can drive every branch with an explicit `Some`/`None` input
/// instead of mutating process-global env — this crate is used from a
/// `#![forbid(unsafe_code)]` composition crate that bans `std::env::set_var`,
/// and raw env mutation is flaky under parallel test execution regardless.
pub(crate) fn resolve_duration_secs_from_raw(raw: Option<String>, default_secs: u64) -> Duration {
    let secs = raw
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default_secs);
    Duration::from_secs(secs)
}

fn idle_stop_after_from_env() -> Duration {
    resolve_duration_secs_from_raw(
        std::env::var(IDLE_STOP_AFTER_ENV).ok(),
        DEFAULT_IDLE_STOP_AFTER_SECS,
    )
}

fn remove_stopped_after_from_env() -> Duration {
    resolve_duration_secs_from_raw(
        std::env::var(REMOVE_STOPPED_AFTER_ENV).ok(),
        DEFAULT_REMOVE_STOPPED_AFTER_SECS,
    )
}

fn forced_recycle_after_from_env() -> Duration {
    resolve_duration_secs_from_raw(
        std::env::var(FORCED_RECYCLE_AFTER_ENV).ok(),
        DEFAULT_FORCED_RECYCLE_AFTER_SECS,
    )
}

/// Configuration for [`SandboxReaper`].
#[derive(Debug, Clone)]
pub struct SandboxReaperConfig {
    /// How often the reaper lists containers and evaluates them.
    pub scan_interval: Duration,
    /// How long a *running* container may go with no recorded activity
    /// before it is stopped (not removed). Default 15 minutes, overridable
    /// via [`IDLE_STOP_AFTER_ENV`].
    pub idle_stop_after: Duration,
    /// How long a *stopped* container is retained before it is removed.
    /// Default 7 days, overridable via [`REMOVE_STOPPED_AFTER_ENV`].
    pub remove_stopped_after: Duration,
    /// Forced-recycle age (from the container's `created_at` label) past
    /// which a running container is stopped and a stopped one is removed
    /// immediately, regardless of the idle/retention windows above.
    /// Default 7 days, overridable via [`FORCED_RECYCLE_AFTER_ENV`].
    pub forced_recycle_after: Duration,
    /// Prefix for the `ironclaw.*` Docker labels this reaper looks for.
    pub label_prefix: String,
}

impl Default for SandboxReaperConfig {
    fn default() -> Self {
        Self {
            scan_interval: Duration::from_secs(300),
            idle_stop_after: idle_stop_after_from_env(),
            remove_stopped_after: remove_stopped_after_from_env(),
            forced_recycle_after: forced_recycle_after_from_env(),
            label_prefix: LABEL_PREFIX.to_string(),
        }
    }
}

/// Pure helper shared by [`decide_reap_action`] and
/// [`SandboxReaper::scan_and_reap`] (the latter needs it to decide whether a
/// container is even a candidate for the idle-stop live probe) so the two
/// never compute "how old is this container" in two places that could
/// drift apart.
fn age_since(now: DateTime<Utc>, created_at: DateTime<Utc>) -> Duration {
    (now - created_at).to_std().unwrap_or(Duration::ZERO)
}

/// What [`decide_reap_action`] concluded a container should have done to
/// it. Pure decision output — [`SandboxReaper::scan_and_reap`] is the only
/// caller that turns this into a Docker call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReapAction {
    None,
    Stop,
    Remove,
}

/// Pure two-stage idle/retention decision, with a forced-recycle backstop.
/// No I/O — every input is a value the caller already read from Docker/the
/// activity registry, so this is exhaustively unit-testable on a fake
/// clock (see `decision_tests` below).
///
/// **Never reap on uncertainty**: `idle: None` (no activity record for a
/// running container) and `finished_at: None` (no attributable stop time
/// for a stopped container) both yield [`ReapAction::None`] rather than
/// treating "unknown" as "eligible" — this holds even when the container's
/// age has crossed `forced_recycle_after`: a stopped container with an
/// unparseable/absent `finished_at` is left alone, not force-removed,
/// because "unknown" must never be reinterpreted as "eligible" just because
/// another signal (age) happens to be known.
///
/// `idle_stop_veto` is the caller's live-probe result (module doc's
/// "Idle-stop veto" section): when `true`, an idle-stop that would
/// otherwise fire is suppressed for this scan (`Stop` becomes `None`). The
/// veto applies only to the idle-stop branch — it never suppresses forced
/// recycle, which stays unconditional regardless of what is live inside
/// the container.
pub(crate) fn decide_reap_action(
    now: DateTime<Utc>,
    created_at: DateTime<Utc>,
    running: bool,
    finished_at: Option<DateTime<Utc>>,
    idle: Option<Duration>,
    idle_stop_veto: bool,
    config: &SandboxReaperConfig,
) -> ReapAction {
    let age = age_since(now, created_at);
    let past_forced_recycle_age = age >= config.forced_recycle_after;

    if running {
        // Forced recycle overrides the idle window (and the veto) outright:
        // a running container's age is always known (it is the container's
        // own `created_at` label), so there is no uncertainty to defer to
        // here, and nothing — including a live background job — runs past
        // this backstop.
        if past_forced_recycle_age {
            return ReapAction::Stop;
        }
        return match idle {
            Some(idle) if idle >= config.idle_stop_after => {
                if idle_stop_veto {
                    // A live-probe found a surviving background process:
                    // suppress the stop for this scan rather than kill a
                    // running dev server out from under the user.
                    ReapAction::None
                } else {
                    ReapAction::Stop
                }
            }
            _ => ReapAction::None, // no activity record: never reap on uncertainty
        };
    }

    match finished_at {
        Some(finished_at) => {
            if past_forced_recycle_age {
                return ReapAction::Remove;
            }
            let stopped_for = (now - finished_at).to_std().unwrap_or(Duration::ZERO);
            if stopped_for >= config.remove_stopped_after {
                ReapAction::Remove
            } else {
                ReapAction::None
            }
        }
        // Can't attribute a stop time: never reap on uncertainty, even if
        // the container's age alone would otherwise justify forced recycle.
        None => ReapAction::None,
    }
}

/// Summary of one `scan_and_reap` pass, for logging/tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReapSummary {
    pub considered: usize,
    pub reaped: usize,
}

/// In-memory debounce for [`ReapAction::Remove`] (module doc's "Remove
/// debounce" section): `docker rm` only fires once the same container id
/// has produced a `Remove` verdict on two *consecutive* [`observe`] calls.
/// Any call for that id whose verdict is not `Remove` clears the pending
/// mark, so the debounce never carries a stale "one Remove sighting" across
/// an unrelated intervening scan. Interior mutability (a `Mutex`, mirroring
/// [`SandboxActivityRegistry`]'s own pattern) because `SandboxReaper` only
/// ever hands out `&self`.
///
/// [`observe`]: RemoveDebounce::observe
#[derive(Default)]
struct RemoveDebounce {
    pending: Mutex<HashSet<String>>,
}

impl RemoveDebounce {
    /// Records this sweep's verdict for `container_id` and returns whether
    /// the caller should act on it now.
    ///
    /// - `is_remove_verdict == false`: clears any pending mark for this id
    ///   and returns `false`.
    /// - `is_remove_verdict == true`, first sighting: marks the id pending
    ///   and returns `false` (do not act yet).
    /// - `is_remove_verdict == true`, second *consecutive* sighting: clears
    ///   the mark and returns `true` (act now).
    fn observe(&self, container_id: &str, is_remove_verdict: bool) -> bool {
        let mut pending = match self.pending.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !is_remove_verdict {
            pending.remove(container_id);
            return false;
        }
        if pending.remove(container_id) {
            true
        } else {
            pending.insert(container_id.to_string());
            false
        }
    }
}

/// Periodically stops idle and removes retention-expired persistent
/// per-user sandbox containers.
pub struct SandboxReaper {
    docker: Docker,
    activity: Arc<SandboxActivityRegistry>,
    config: SandboxReaperConfig,
    remove_debounce: RemoveDebounce,
}

impl SandboxReaper {
    pub fn new(
        docker: Docker,
        activity: Arc<SandboxActivityRegistry>,
        config: SandboxReaperConfig,
    ) -> Self {
        Self {
            docker,
            activity,
            config,
            remove_debounce: RemoveDebounce::default(),
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

    /// Lists every persistent sandbox container (any container carrying the
    /// `ironclaw.created_at` label), evaluates each against
    /// [`decide_reap_action`], and acts on the result. Errors listing
    /// containers are propagated (a Docker connectivity failure means the
    /// scan learned nothing, not that everything is fine); errors reaping
    /// an individual container are best-effort and logged.
    pub async fn scan_and_reap(&self) -> Result<ReapSummary, RuntimeProcessError> {
        let containers = self.list_persistent_containers().await?;
        let now = Utc::now();
        let mut summary = ReapSummary::default();

        for container in &containers {
            let Some(candidate) =
                UserContainerCandidate::from_summary(container, &self.config.label_prefix)
            else {
                // Unattributable container (missing/unparseable created_at
                // label): not ours to manage, leave it alone.
                continue;
            };
            let Some((tenant_id, user_id)) =
                parse_tenant_user_labels(container, &self.config.label_prefix)
            else {
                // Missing/unparseable tenant or user label: never reap on
                // uncertainty — this is the "unparseable/foreign labels"
                // half of dropping orphan-sweep duty.
                continue;
            };
            summary.considered += 1;

            let key = RebornSandboxUserKey::from_tenant_user(&tenant_id, &user_id);
            let running = container.state.as_deref() == Some("running");
            let idle = self.activity.idle_for(&key, std::time::Instant::now());
            let finished_at = if running {
                None
            } else {
                self.finished_at(&candidate.container_id).await
            };

            // Only spend a live probe when the pure decision would *idle*-stop
            // this container — forced recycle is unconditional and must never
            // be gated behind a probe, and a container that isn't headed for
            // idle-stop has nothing for the veto to suppress.
            let age = age_since(now, candidate.created_at);
            let past_forced_recycle_age = age >= self.config.forced_recycle_after;
            let would_idle_stop = running
                && !past_forced_recycle_age
                && idle.is_some_and(|idle| idle >= self.config.idle_stop_after);
            let idle_stop_veto = if would_idle_stop {
                self.probe_live_background(&candidate.container_id).await
            } else {
                false
            };

            let action = decide_reap_action(
                now,
                candidate.created_at,
                running,
                finished_at,
                idle,
                idle_stop_veto,
                &self.config,
            );

            match action {
                ReapAction::None => {
                    self.remove_debounce.observe(&candidate.container_id, false);
                }
                ReapAction::Stop => {
                    self.remove_debounce.observe(&candidate.container_id, false);
                    self.stop_container(&candidate.container_id).await;
                    summary.reaped += 1;
                }
                ReapAction::Remove => {
                    // Debounced: only act once the same container id has
                    // produced a `Remove` verdict on two consecutive sweeps
                    // (see the module doc's "Remove debounce" section).
                    if self.remove_debounce.observe(&candidate.container_id, true) {
                        self.remove_container(&candidate.container_id).await;
                        self.activity.forget(&key);
                        summary.reaped += 1;
                    }
                }
            }
        }

        tracing::debug!(
            considered = summary.considered,
            reaped = summary.reaped,
            "sandbox reaper scan complete"
        );
        Ok(summary)
    }

    async fn list_persistent_containers(
        &self,
    ) -> Result<Vec<ContainerSummary>, RuntimeProcessError> {
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![registry::label_created_at(&self.config.label_prefix)],
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

    /// Inspects `container_id` for its `State.FinishedAt` timestamp.
    /// Returns `None` on any inspect failure or unparseable/absent value —
    /// callers treat that the same as "can't attribute a stop time", which
    /// [`decide_reap_action`] already leaves alone rather than reaping.
    async fn finished_at(&self, container_id: &str) -> Option<DateTime<Utc>> {
        let inspected = self
            .docker
            .inspect_container(
                container_id,
                None::<bollard::container::InspectContainerOptions>,
            )
            .await
            .ok()?;
        let raw = inspected.state?.finished_at?;
        // Docker reports the zero value ("0001-01-01T00:00:00Z") for a
        // container that has never exited; that is not a real stop time.
        let parsed = DateTime::parse_from_rfc3339(&raw).ok()?;
        if parsed.timestamp() <= 0 {
            return None;
        }
        Some(parsed.with_timezone(&Utc))
    }

    /// Live-probes `container_id` for a surviving background job to decide
    /// whether an idle-stop should be vetoed (module doc's "Idle-stop veto"
    /// section). Only called when the pure decision would otherwise
    /// idle-stop this container.
    ///
    /// The persistent container's PID 1 is always `sleep infinity` (see
    /// `exec_transport::user_container_launch_config`), and by scan time
    /// every *foreground* shell invocation's own `setsid`-wrapped exec
    /// session has exited (`exec_in_container` waits for it, and this probe
    /// only runs between commands — see
    /// `RebornScopedSandboxCommandTransport::reconcile_background_jobs` for
    /// the sibling in-band sweep that trims dead pids from the footer). So
    /// any pid other than PID 1 that this `ps` snapshot observes is
    /// necessarily a detached, model-launched background process (e.g. a
    /// `background:true` dev server) — the same shape
    /// `reconcile_background_jobs` sweeps, just probed here without a
    /// tracked-pid list to check against.
    ///
    /// Probe failure (exec error, container race, timeout) returns `true`
    /// (veto/"live") — never reap on uncertainty.
    async fn probe_live_background(&self, container_id: &str) -> bool {
        let probed = exec_transport::exec_in_container(
            &self.docker,
            container_id,
            ContainerWorkdir::workspace_root(),
            Vec::new(),
            "ps -o pid= --no-headers".to_string(),
            Duration::from_secs(5),
            4096,
        )
        .await;
        let Ok(probed) = probed else {
            return true;
        };
        probed
            .output
            .split_whitespace()
            .filter_map(|token| token.trim().parse::<u32>().ok())
            .any(|pid| pid != 1)
    }

    /// Best-effort stop: never fail the scan over a single container's
    /// teardown error, just log at `debug!` (never `info!` — background
    /// tasks must not write to the REPL surface).
    async fn stop_container(&self, container_id: &str) {
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
    }

    /// Best-effort forced removal, mirroring [`Self::stop_container`]'s
    /// error handling.
    async fn remove_container(&self, container_id: &str) {
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

/// Parses the `ironclaw.tenant`/`ironclaw.user` labels off a
/// [`ContainerSummary`] into a `{TenantId, UserId}` pair. `None` when
/// either label is missing or fails to parse into a valid id — the
/// container cannot be attributed to a user record, so the caller must
/// leave it alone rather than guess.
fn parse_tenant_user_labels(
    container: &ContainerSummary,
    label_prefix: &str,
) -> Option<(TenantId, UserId)> {
    let labels = container.labels.as_ref()?;
    let tenant_id = labels
        .get(&registry::label_tenant(label_prefix))
        .and_then(|value| TenantId::new(value).ok())?;
    let user_id = labels
        .get(&registry::label_user(label_prefix))
        .and_then(|value| UserId::new(value).ok())?;
    Some((tenant_id, user_id))
}

#[cfg(test)]
mod decision_tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    fn config() -> SandboxReaperConfig {
        SandboxReaperConfig {
            scan_interval: Duration::from_secs(300),
            idle_stop_after: Duration::from_secs(900),
            remove_stopped_after: Duration::from_secs(7 * 24 * 3600),
            forced_recycle_after: Duration::from_secs(7 * 24 * 3600),
            label_prefix: LABEL_PREFIX.to_string(),
        }
    }

    #[test]
    fn running_container_under_idle_threshold_is_left_alone() {
        let now = Utc::now();
        let action = decide_reap_action(
            now,
            now,
            true,
            None,
            Some(Duration::from_secs(60)),
            false,
            &config(),
        );
        assert_eq!(action, ReapAction::None);
    }

    #[test]
    fn running_container_past_idle_threshold_is_stopped() {
        let now = Utc::now();
        let action = decide_reap_action(
            now,
            now,
            true,
            None,
            Some(Duration::from_secs(1_000)),
            false,
            &config(),
        );
        assert_eq!(action, ReapAction::Stop);
    }

    #[test]
    fn running_container_past_idle_threshold_with_live_background_veto_is_left_alone() {
        // A `background:true` dev server only touches the activity registry
        // once at launch, so idle crosses the threshold while it is still
        // running. A live `ps` probe finding it suppresses the idle-stop.
        let now = Utc::now();
        let action = decide_reap_action(
            now,
            now,
            true,
            None,
            Some(Duration::from_secs(1_000)),
            true,
            &config(),
        );
        assert_eq!(action, ReapAction::None);
    }

    #[test]
    fn running_container_with_no_activity_record_is_left_alone_not_stopped() {
        // A process restart loses the in-memory activity map; treating
        // "unknown" as "reap" would mass-stop every warm container on
        // every composition restart. Mirrors the old reaper's "never
        // reap on uncertainty" rule.
        let now = Utc::now();
        let action = decide_reap_action(now, now, true, None, None, false, &config());
        assert_eq!(action, ReapAction::None);
    }

    #[test]
    fn stopped_container_under_retention_is_left_alone() {
        let now = Utc::now();
        let finished_at = now - ChronoDuration::hours(1);
        let action = decide_reap_action(
            now,
            now - ChronoDuration::days(1),
            false,
            Some(finished_at),
            None,
            false,
            &config(),
        );
        assert_eq!(action, ReapAction::None);
    }

    #[test]
    fn stopped_container_past_retention_is_removed() {
        let now = Utc::now();
        let finished_at = now - ChronoDuration::days(8);
        let action = decide_reap_action(
            now,
            now - ChronoDuration::days(8),
            false,
            Some(finished_at),
            None,
            false,
            &config(),
        );
        assert_eq!(action, ReapAction::Remove);
    }

    #[test]
    fn stopped_container_with_unknown_finished_at_is_left_alone() {
        let now = Utc::now();
        let action = decide_reap_action(
            now,
            now - ChronoDuration::days(8),
            false,
            None,
            None,
            false,
            &config(),
        );
        assert_eq!(action, ReapAction::None);
    }

    #[test]
    fn running_container_past_forced_recycle_age_is_stopped_first() {
        let now = Utc::now();
        let action = decide_reap_action(
            now,
            now - ChronoDuration::days(8),
            true,
            None,
            Some(Duration::ZERO),
            false,
            &config(),
        );
        assert_eq!(action, ReapAction::Stop);
    }

    #[test]
    fn running_container_past_forced_recycle_age_is_stopped_even_with_veto() {
        // Forced recycle is unconditional: a live background job never
        // outlives the 7-day backstop, unlike an ordinary idle-stop.
        let now = Utc::now();
        let action = decide_reap_action(
            now,
            now - ChronoDuration::days(8),
            true,
            None,
            Some(Duration::ZERO),
            true,
            &config(),
        );
        assert_eq!(action, ReapAction::Stop);
    }

    #[test]
    fn already_stopped_container_past_forced_recycle_age_is_removed_even_within_retention() {
        let now = Utc::now();
        let finished_at = now - ChronoDuration::minutes(5);
        let action = decide_reap_action(
            now,
            now - ChronoDuration::days(8),
            false,
            Some(finished_at),
            None,
            false,
            &config(),
        );
        assert_eq!(action, ReapAction::Remove);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_plan_thresholds() {
        let config = SandboxReaperConfig::default();

        assert_eq!(config.scan_interval, Duration::from_secs(300));
        assert_eq!(config.idle_stop_after, Duration::from_secs(900));
        assert_eq!(
            config.remove_stopped_after,
            Duration::from_secs(7 * 24 * 3600)
        );
        assert_eq!(
            config.forced_recycle_after,
            Duration::from_secs(7 * 24 * 3600)
        );
        assert_eq!(config.label_prefix, "ironclaw");
    }

    #[test]
    fn env_override_resolution_falls_back_on_absent_empty_or_invalid_values() {
        for raw in [
            None,
            Some(String::new()),
            Some("not-a-number".to_string()),
            Some("0".to_string()),
        ] {
            assert_eq!(
                resolve_duration_secs_from_raw(raw, 42),
                Duration::from_secs(42)
            );
        }
    }

    #[test]
    fn env_override_resolution_uses_a_valid_positive_value() {
        assert_eq!(
            resolve_duration_secs_from_raw(Some("120".to_string()), 42),
            Duration::from_secs(120)
        );
    }
}

#[cfg(test)]
mod remove_debounce_tests {
    use super::*;

    #[test]
    fn first_remove_verdict_does_not_act_and_marks_pending() {
        let debounce = RemoveDebounce::default();
        assert!(!debounce.observe("container-a", true));
    }

    #[test]
    fn second_consecutive_remove_verdict_acts() {
        let debounce = RemoveDebounce::default();
        assert!(!debounce.observe("container-a", true));
        assert!(debounce.observe("container-a", true));
    }

    #[test]
    fn a_third_consecutive_remove_verdict_debounces_again() {
        // After acting, the mark is cleared — the *next* Remove verdict is
        // treated as a fresh first sighting, not an already-armed one.
        let debounce = RemoveDebounce::default();
        assert!(!debounce.observe("container-a", true));
        assert!(debounce.observe("container-a", true));
        assert!(!debounce.observe("container-a", true));
        assert!(debounce.observe("container-a", true));
    }

    #[test]
    fn an_interleaved_non_remove_verdict_clears_the_pending_mark() {
        let debounce = RemoveDebounce::default();
        assert!(!debounce.observe("container-a", true));
        assert!(!debounce.observe("container-a", false));
        // The pending mark was cleared by the None/Stop sweep in between, so
        // this Remove verdict is a fresh first sighting, not a second one.
        assert!(!debounce.observe("container-a", true));
    }

    #[test]
    fn a_non_remove_verdict_with_no_prior_pending_mark_is_a_no_op() {
        let debounce = RemoveDebounce::default();
        assert!(!debounce.observe("container-a", false));
    }

    #[test]
    fn distinct_container_ids_are_debounced_independently() {
        let debounce = RemoveDebounce::default();
        assert!(!debounce.observe("container-a", true));
        // container-b's first sighting must not be treated as container-a's
        // second.
        assert!(!debounce.observe("container-b", true));
        assert!(debounce.observe("container-a", true));
    }
}
