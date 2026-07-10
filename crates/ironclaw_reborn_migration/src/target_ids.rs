//! Stable target identifiers for replay-safe migration writes.
//!
//! The namespace and seed format are versioned. Changing either would mint a
//! second copy of every migrated record, so a future incompatible format must
//! introduce a new manifest schema version.

use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_triggers::TriggerId;
use uuid::Uuid;

use crate::error::MigrationError;
use crate::report::MigrationReport;

const MIGRATION_ID_SCHEMA: &str = "ironclaw-v1-to-reborn/v1";
const MIGRATION_NAMESPACE: Uuid = Uuid::from_u128(0xd735d0a7_891d_4e57_ba2e_368f2a36a82c);

#[derive(Debug, Clone)]
pub(crate) struct MigrationIdentity {
    manifest_schema_version: u32,
    source_fingerprint: String,
}

impl MigrationIdentity {
    pub(crate) fn from_report(report: &MigrationReport) -> Result<Self, MigrationError> {
        let manifest = report.manifest.as_ref().ok_or_else(|| {
            MigrationError::InvalidInput(
                "converter execution requires a sealed migration manifest".to_string(),
            )
        })?;
        Ok(Self {
            manifest_schema_version: manifest.manifest_schema_version,
            source_fingerprint: manifest.source_fingerprint.value.clone(),
        })
    }

    pub(crate) fn trigger_id(
        &self,
        domain: &str,
        source_primary_id: &str,
        tenant_id: &TenantId,
        agent_id: &AgentId,
    ) -> Result<TriggerId, MigrationError> {
        let uuid = self.scoped_uuid(
            domain,
            source_primary_id,
            tenant_id.as_str(),
            agent_id.as_str(),
        );
        let ulid = ulid::Ulid::from(uuid.as_u128());
        TriggerId::parse(&ulid.to_string()).map_err(|error| {
            MigrationError::InvalidInput(format!(
                "could not derive deterministic {domain} target id: {error}"
            ))
        })
    }

    pub(crate) fn message_key(
        &self,
        thread_id: Uuid,
        message_index: usize,
        source_primary_id: Option<&str>,
    ) -> String {
        let source_primary_id = source_primary_id
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{thread_id}\0{message_index}"));
        self.scoped_uuid("message", &source_primary_id, "transcript", "transcript")
            .to_string()
    }

    pub(crate) fn thread_source_binding(&self, thread_id: Uuid) -> String {
        let suffix = self.scoped_uuid("thread-binding", &thread_id.to_string(), "thread", "thread");
        format!("migration:v1:{suffix}")
    }

    fn scoped_uuid(
        &self,
        domain: &str,
        source_primary_id: &str,
        tenant_id: &str,
        agent_id: &str,
    ) -> Uuid {
        let seed = format!(
            "{MIGRATION_ID_SCHEMA}\0{}\0{}\0{domain}\0{source_primary_id}\0{tenant_id}\0{agent_id}",
            self.manifest_schema_version, self.source_fingerprint
        );
        Uuid::new_v5(&MIGRATION_NAMESPACE, seed.as_bytes())
    }

    #[cfg(test)]
    fn for_test(source_fingerprint: &str) -> Self {
        Self {
            manifest_schema_version: 1,
            source_fingerprint: source_fingerprint.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, TenantId};
    use uuid::Uuid;

    use super::MigrationIdentity;

    #[test]
    fn trigger_ids_are_stable_and_scope_sensitive() {
        let tenant_a = TenantId::new("tenant-a").unwrap();
        let tenant_b = TenantId::new("tenant-b").unwrap();
        let agent = AgentId::new("agent-a").unwrap();
        let identity = MigrationIdentity::for_test("source-a");

        let first = identity
            .trigger_id("routine", "routine-1", &tenant_a, &agent)
            .unwrap();
        let replay = identity
            .trigger_id("routine", "routine-1", &tenant_a, &agent)
            .unwrap();
        let other_scope = identity
            .trigger_id("routine", "routine-1", &tenant_b, &agent)
            .unwrap();

        assert_eq!(first, replay);
        assert_ne!(first, other_scope);
    }

    #[test]
    fn source_fingerprint_partitions_target_ids() {
        let tenant = TenantId::new("tenant-a").unwrap();
        let agent = AgentId::new("agent-a").unwrap();
        let source_a = MigrationIdentity::for_test("source-a");
        let source_b = MigrationIdentity::for_test("source-b");

        assert_ne!(
            source_a
                .trigger_id("routine", "routine-1", &tenant, &agent)
                .unwrap(),
            source_b
                .trigger_id("routine", "routine-1", &tenant, &agent)
                .unwrap()
        );
    }

    #[test]
    fn synthesized_message_keys_are_stable_and_order_sensitive() {
        let thread = Uuid::parse_str("1cdfa15a-a8e7-4868-a25d-6fbde771d438").unwrap();
        let identity = MigrationIdentity::for_test("source-a");
        assert_eq!(
            identity.message_key(thread, 2, None),
            identity.message_key(thread, 2, None)
        );
        assert_ne!(
            identity.message_key(thread, 2, None),
            identity.message_key(thread, 3, None)
        );
        assert_eq!(
            identity.message_key(thread, 2, Some("source-id")),
            identity.message_key(thread, 9, Some("source-id"))
        );
        assert_eq!(
            identity.thread_source_binding(thread),
            identity.thread_source_binding(thread)
        );
    }
}
