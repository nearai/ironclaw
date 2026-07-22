//! Tenant-level concurrency ceiling for the `hosted-single-tenant-volume-sandboxed`
//! profile (D3-2).
//!
//! `ironclaw_authorization::obligations_for_grant` already emits a
//! `ReserveResources` obligation for every `EffectKind::SpawnProcess`
//! capability grant (D3-1), and `ironclaw_host_runtime::obligations::
//! reserve_resource_obligation` already reserves against whatever
//! `ResourceGovernor` composition wires in. Both are no-ops today because no
//! deployment ever calls `set_limit` for a `SpawnProcess`-relevant account —
//! this module is the one caller that does, for the sandboxed profile only.
//!
//! Kept as its own module (not inlined into `factory.rs`, which is already
//! thousands of lines) so the boot call site stays a single line.

use std::sync::Arc;

use ironclaw_host_api::{TenantId, UserId};
use ironclaw_resources::{ResourceAccount, ResourceError, ResourceGovernor, ResourceLimits};

/// Overrides the sandboxed profile's per-tenant concurrent `SpawnProcess`
/// ceiling. Unset, or set to a non-positive/unparseable value, falls back to
/// [`DEFAULT_SANDBOX_MAX_CONCURRENT`].
pub(crate) const SANDBOX_MAX_CONCURRENT_ENV: &str = "IRONCLAW_SANDBOX_MAX_CONCURRENT";

/// Default per-tenant concurrent sandbox-process ceiling when
/// [`SANDBOX_MAX_CONCURRENT_ENV`] is not set. Deliberately small: the
/// sandboxed profile runs one Docker container per shell invocation, and an
/// unbounded ceiling defeats the point of D3-2 (bounding a single tenant's
/// concurrent container fan-out).
pub(crate) const DEFAULT_SANDBOX_MAX_CONCURRENT: u32 = 4;

/// Resolves the configured ceiling from [`SANDBOX_MAX_CONCURRENT_ENV`],
/// falling back to [`DEFAULT_SANDBOX_MAX_CONCURRENT`] when the env var is
/// absent, empty, non-numeric, or zero (zero would mean "no sandboxed shell
/// calls ever succeed", which is never an intentional deployment choice —
/// operators who want that should not enable the sandboxed profile).
pub(crate) fn sandbox_max_concurrent_from_env() -> u32 {
    resolve_sandbox_max_concurrent_from_raw(std::env::var(SANDBOX_MAX_CONCURRENT_ENV).ok())
}

/// Pure resolution of the sandbox max-concurrency ceiling from an already-read
/// raw env value. Kept separate from [`sandbox_max_concurrent_from_env`] so
/// tests can exercise the parse/validate/default logic directly with an
/// explicit `Some`/`None` input instead of mutating process env — a raw
/// `std::env::var` read does not observe
/// `ironclaw_common::env_helpers::set_runtime_env`'s thread-local override,
/// so round-tripping through real env vars in tests is both unnecessary and
/// unreliable under parallel test execution.
pub(crate) fn resolve_sandbox_max_concurrent_from_raw(raw: Option<String>) -> u32 {
    raw.and_then(|raw| raw.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SANDBOX_MAX_CONCURRENT)
}

/// Sets the per-user `max_concurrency_slots` ceiling on `governor` for
/// `user_id` within `tenant_id`. Composition calls this once at boot, only
/// for the `hosted-single-tenant-volume-sandboxed` profile — every other
/// profile leaves the account unlimited, matching D3-1's "no-op until D3-2"
/// note. Scoped per-user (not per-tenant) so one user cannot starve every
/// other user in the tenant by exhausting the shared ceiling.
///
/// This is what turns the D3-1 `ReserveResources` obligation from a no-op
/// into an actual gate: `FilesystemResourceGovernor`/
/// `InMemoryResourceGovernor::reserve_with_outcome` check `max_concurrency_slots`
/// against the account's current outstanding reservations, so the
/// `N+1`th concurrent `SpawnProcess` reservation for this user is denied as
/// a model-visible outcome (never a host error) once this ceiling is set.
pub(crate) fn apply_sandbox_user_ceiling(
    governor: &Arc<dyn ResourceGovernor>,
    tenant_id: TenantId,
    user_id: UserId,
    max_concurrent: u32,
) -> Result<(), ResourceError> {
    governor.set_limit(
        ResourceAccount::user(tenant_id, user_id),
        ResourceLimits::default().set_max_concurrency_slots(max_concurrent),
    )
}

/// Resolves the tenant id the sandboxed-profile boot ceiling applies to:
/// the local-runtime identity's tenant when one was supplied, else the same
/// `reborn_cli()` default identity every other local-runtime call site falls
/// back to (mirrors `local_dev_extension_lifecycle_surface_context` in
/// `factory.rs`).
pub(crate) fn resolve_local_runtime_tenant_id(
    local_runtime_identity: Option<&crate::input::RebornLocalRuntimeIdentity>,
) -> Result<TenantId, crate::RebornBuildError> {
    if let Some(identity) = local_runtime_identity {
        return Ok(identity.tenant_id.clone());
    }
    let default_identity = crate::runtime_input::RebornRuntimeIdentity::reborn_cli();
    TenantId::new(default_identity.tenant_id).map_err(|error| {
        crate::RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{InvocationId, ResourceEstimate, ResourceScope, UserId};
    use ironclaw_resources::InMemoryResourceGovernor;

    use super::*;

    fn tenant(id: &str) -> TenantId {
        TenantId::new(id.to_string()).expect("valid tenant id")
    }

    fn scope_for(tenant_id: &TenantId, user_id: &UserId) -> ResourceScope {
        ResourceScope {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    #[test]
    fn env_override_is_read_and_validated() {
        assert_eq!(
            resolve_sandbox_max_concurrent_from_raw(None),
            DEFAULT_SANDBOX_MAX_CONCURRENT,
            "unset falls back to the default"
        );

        assert_eq!(
            resolve_sandbox_max_concurrent_from_raw(Some("7".to_string())),
            7,
            "a valid number is used"
        );

        assert_eq!(
            resolve_sandbox_max_concurrent_from_raw(Some("0".to_string())),
            DEFAULT_SANDBOX_MAX_CONCURRENT,
            "zero must not disable the sandboxed profile entirely"
        );

        assert_eq!(
            resolve_sandbox_max_concurrent_from_raw(Some("not-a-number".to_string())),
            DEFAULT_SANDBOX_MAX_CONCURRENT,
            "unparseable falls back to the default"
        );
    }

    /// D3-2's headline behavior, now scoped per-user (not per-tenant): once
    /// a user's ceiling is applied, the `N+1`th concurrent reservation for
    /// that same user is *denied* — a model-visible outcome from
    /// `ResourceGovernor::reserve`, never a host panic/error — while a
    /// sibling user in the same tenant is unaffected. This strictly
    /// supersedes the old per-tenant test: the old behavior let one user
    /// starve every other user in the tenant, which this change fixes.
    #[test]
    fn ceiling_denies_the_second_concurrent_reservation_for_the_same_user_but_not_a_sibling_user() {
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let tenant_id = tenant("sandboxed-tenant");
        let user_id = UserId::new("user-a").expect("valid user id");

        apply_sandbox_user_ceiling(&governor, tenant_id.clone(), user_id.clone(), 1)
            .expect("setting a finite ceiling on an empty account succeeds");

        let first = governor
            .reserve(
                scope_for(&tenant_id, &user_id),
                ResourceEstimate::default().set_concurrency_slots(1),
            )
            .expect("first reservation is within the ceiling");
        let second = governor.reserve(
            scope_for(&tenant_id, &user_id),
            ResourceEstimate::default().set_concurrency_slots(1),
        );
        assert!(
            second.is_err(),
            "second concurrent reservation for a capped user must be denied"
        );

        // A sibling user in the SAME tenant is unaffected — this is the
        // headline behavior change from the old per-tenant ceiling, where one
        // user could starve every other user in the tenant.
        let sibling_user = UserId::new("user-b").expect("valid user id");
        governor
            .reserve(
                scope_for(&tenant_id, &sibling_user),
                ResourceEstimate::default().set_concurrency_slots(1),
            )
            .expect("a sibling user in the same tenant is unaffected by user-a's ceiling");

        drop(first);
    }
}
