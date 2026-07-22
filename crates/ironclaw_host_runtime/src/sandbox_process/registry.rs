//! Labels-as-identity container registry for the persistent per-user
//! sandbox model. Two responsibilities:
//!
//! 1. Docker label construction/parsing for `{tenant, user, created_at}`
//!    identity — crash-safe (survives daemon restart), no DB.
//! 2. A push-based in-memory last-activity map. The exec transport calls
//!    [`SandboxActivityRegistry::touch`] after every successful command;
//!    the reaper only ever reads via `idle_for`/`last_activity` — it
//!    never inspects container stats to infer activity. Labels are
//!    immutable post-create and NEVER carry this mutable state.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use bollard::models::ContainerSummary;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{TenantId, UserId};

use super::user_key::RebornSandboxUserKey;

pub(crate) fn label_tenant(prefix: &str) -> String {
    format!("{prefix}.tenant")
}
pub(crate) fn label_user(prefix: &str) -> String {
    format!("{prefix}.user")
}
pub(crate) fn label_created_at(prefix: &str) -> String {
    format!("{prefix}.created_at")
}

pub(crate) fn build_user_container_labels(
    prefix: &str,
    tenant_id: &TenantId,
    user_id: &UserId,
) -> HashMap<String, String> {
    HashMap::from([
        (label_tenant(prefix), tenant_id.as_str().to_string()),
        (label_user(prefix), user_id.as_str().to_string()),
        (label_created_at(prefix), Utc::now().to_rfc3339()),
    ])
}

pub(crate) fn user_container_label_filter(
    prefix: &str,
    tenant_id: &TenantId,
    user_id: &UserId,
) -> HashMap<String, Vec<String>> {
    HashMap::from([(
        "label".to_string(),
        vec![
            format!("{}={}", label_tenant(prefix), tenant_id.as_str()),
            format!("{}={}", label_user(prefix), user_id.as_str()),
        ],
    )])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserContainerCandidate {
    pub(crate) container_id: String,
    pub(crate) created_at: DateTime<Utc>,
}

impl UserContainerCandidate {
    pub(crate) fn from_summary(container: &ContainerSummary, label_prefix: &str) -> Option<Self> {
        let container_id = container.id.clone()?;
        let labels = container.labels.as_ref()?;
        let created_at = labels
            .get(&label_created_at(label_prefix))
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))?;
        Some(Self {
            container_id,
            created_at,
        })
    }

    pub(crate) fn age(&self, now: DateTime<Utc>) -> Duration {
        (now - self.created_at).to_std().unwrap_or(Duration::ZERO)
    }
}

/// Push-based in-memory map of per-user last-activity timestamps, keyed on
/// [`RebornSandboxUserKey`]. Cross-crate consumers (e.g. the reborn runtime
/// composition wiring that owns the reaper loop) need to construct and pass
/// this registry, so it is `pub` and re-exported at the crate root — unlike
/// the label helpers and candidate type above, which stay internal to this
/// crate.
#[derive(Debug, Default)]
pub struct SandboxActivityRegistry {
    last_activity: Mutex<HashMap<RebornSandboxUserKey, Instant>>,
}

impl SandboxActivityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RebornSandboxUserKey, Instant>> {
        // Recover from poisoning rather than panic: a background reaper
        // must never crash the whole process over a prior panic elsewhere
        // that poisoned this unrelated mutex.
        self.last_activity
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(crate) fn touch(&self, key: &RebornSandboxUserKey) {
        self.lock().insert(key.clone(), Instant::now());
    }

    pub(crate) fn last_activity(&self, key: &RebornSandboxUserKey) -> Option<Instant> {
        self.lock().get(key).copied()
    }

    pub(crate) fn forget(&self, key: &RebornSandboxUserKey) {
        self.lock().remove(key);
    }

    pub(crate) fn idle_for(&self, key: &RebornSandboxUserKey, now: Instant) -> Option<Duration> {
        self.last_activity(key)
            .map(|activity| now.saturating_duration_since(activity))
    }
}

/// A single tracked background (`background: true`) shell launch, kept
/// per-user so the foreground command path can render a "still-live
/// background processes" footer. A named struct rather than a `(u32,
/// String)` tuple — `jobs_for` return values flow into formatting code
/// where a bare tuple's field order is not self-documenting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackgroundJob {
    pub(crate) pid: u32,
    pub(crate) command_preview: String,
}

/// Push-based in-memory map of per-user background job launches, keyed on
/// [`RebornSandboxUserKey`] the same way [`SandboxActivityRegistry`] is.
/// Kept as a sibling registry (single responsibility) rather than folded
/// into the activity map.
#[derive(Debug, Default)]
pub(crate) struct BackgroundJobRegistry {
    jobs: Mutex<HashMap<RebornSandboxUserKey, Vec<BackgroundJob>>>,
}

impl BackgroundJobRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RebornSandboxUserKey, Vec<BackgroundJob>>> {
        self.jobs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(crate) fn record(&self, key: &RebornSandboxUserKey, pid: u32, command_preview: String) {
        self.lock()
            .entry(key.clone())
            .or_default()
            .push(BackgroundJob {
                pid,
                command_preview,
            });
    }

    pub(crate) fn jobs_for(&self, key: &RebornSandboxUserKey) -> Vec<BackgroundJob> {
        self.lock().get(key).cloned().unwrap_or_default()
    }

    pub(crate) fn drop_dead(&self, key: &RebornSandboxUserKey, alive_pids: &[u32]) {
        if let Some(jobs) = self.lock().get_mut(key) {
            jobs.retain(|job| alive_pids.contains(&job.pid));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{TenantId, UserId};

    #[test]
    fn label_filter_targets_tenant_and_user_labels_only() {
        let tenant = TenantId::new("tenant-a").unwrap();
        let user = UserId::new("user-a").unwrap();

        let filter = user_container_label_filter("ironclaw", &tenant, &user);

        assert_eq!(
            filter.get("label").unwrap(),
            &vec![
                "ironclaw.tenant=tenant-a".to_string(),
                "ironclaw.user=user-a".to_string(),
            ]
        );
    }

    #[test]
    fn candidate_parses_created_at_and_ignores_missing_labels() {
        let tenant = TenantId::new("tenant-a").unwrap();
        let user = UserId::new("user-a").unwrap();
        let labels = build_user_container_labels("ironclaw", &tenant, &user);
        let container = ContainerSummary {
            id: Some("abc123".to_string()),
            labels: Some(labels),
            ..Default::default()
        };

        let candidate = UserContainerCandidate::from_summary(&container, "ironclaw")
            .expect("round-tripped labels must parse");

        assert_eq!(candidate.container_id, "abc123");

        let missing = ContainerSummary {
            id: Some("no-labels".to_string()),
            labels: None,
            ..Default::default()
        };
        assert!(UserContainerCandidate::from_summary(&missing, "ironclaw").is_none());
    }

    #[test]
    fn activity_registry_touch_then_idle_for_reports_elapsed_duration() {
        let registry = SandboxActivityRegistry::new();
        let tenant = TenantId::new("t").unwrap();
        let user = UserId::new("u").unwrap();
        let scope = ironclaw_host_api::ResourceScope {
            tenant_id: tenant,
            user_id: user,
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        let key = RebornSandboxUserKey::from_scope(&scope);

        assert!(registry.last_activity(&key).is_none());
        registry.touch(&key);
        let idle = registry.idle_for(&key, Instant::now() + Duration::from_secs(5));
        assert!(idle.unwrap() >= Duration::from_secs(5));
    }

    #[test]
    fn activity_registry_forget_clears_the_entry() {
        let registry = SandboxActivityRegistry::new();
        let scope = ironclaw_host_api::ResourceScope {
            tenant_id: TenantId::new("t").unwrap(),
            user_id: UserId::new("u").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        let key = RebornSandboxUserKey::from_scope(&scope);
        registry.touch(&key);

        registry.forget(&key);

        assert!(registry.last_activity(&key).is_none());
    }
}
