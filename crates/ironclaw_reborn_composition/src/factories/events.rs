//! Durable event/audit log factory.
//!
//! `ironclaw_events` is merged with in-memory reference backends. Postgres /
//! libSQL durable backends are deliberately deferred to later grouped Reborn
//! PRs that depend on `ironclaw_filesystem` and the database substrates.
//! Until those backends exist, the `Production` profile would fail the
//! "no in-memory fallback" rule. The in-memory backend is wired here for all
//! profiles so the rest of the graph can be validated; the
//! [`super::gate_substrate`] check for `durable_event_backend` triggers when
//! `Production` is selected without an explicit durable backend.

use std::sync::Arc;

use ironclaw_events::{InMemoryDurableAuditLog, InMemoryDurableEventLog};

use crate::{RebornBuildError, RebornBuildInput, RebornProductionServices};

pub(crate) fn build(
    input: &RebornBuildInput,
    services: &mut RebornProductionServices,
) -> Result<(), RebornBuildError> {
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    services.event_log = Some(event_log);
    services.audit_log = Some(audit_log);

    if input.profile == crate::RebornProfile::Production {
        // The in-memory backend is fine for LocalDev / MigrationDryRun, but
        // Production requires a durable Postgres/libSQL-backed log. The
        // factory for that log lives in a follow-up PR — issue #3022
        // (open as of 2026-04-29; no implementing PR in flight)
        // gates the cutover and will replace this gate with a real
        // builder when it lands.
        return Err(RebornBuildError::SubstrateNotImplemented {
            service: "durable_event_backend",
        });
    }

    Ok(())
}
