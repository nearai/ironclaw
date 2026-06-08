use ironclaw_host_api::{ResourceScope, ScopedPath};
use ironclaw_product_workflow::{ActionFingerprintKey, ProductWorkflowError};

use super::durable_error;

const DEFAULT_LEDGER_ROOT: &str = "/engine/product_workflow/idempotency/actions";

pub(super) fn action_path(
    root: &ScopedPath,
    fingerprint: &ActionFingerprintKey,
) -> Result<ScopedPath, ProductWorkflowError> {
    let path = format!(
        "{}/{}/{}/{}/{}/{}/{}.json",
        root.as_str().trim_end_matches('/'),
        hex_component(fingerprint.adapter_id.as_str()),
        hex_component(fingerprint.installation_id.as_str()),
        hex_component(fingerprint.external_actor_ref.kind()),
        hex_component(fingerprint.external_actor_ref.id()),
        hex_component(fingerprint.source_binding_key.as_str()),
        hex_component(fingerprint.external_event_id.as_str())
    );
    ScopedPath::new(path).map_err(|error| durable_error("construct action path", error))
}

pub(super) fn prune_lease_path(root: &ScopedPath) -> Result<ScopedPath, ProductWorkflowError> {
    let path = format!(
        "{}/_control/prune_lease.json",
        root.as_str().trim_end_matches('/')
    );
    ScopedPath::new(path).map_err(|error| durable_error("construct prune lease path", error))
}

pub(super) fn default_scoped_ledger_root() -> ScopedPath {
    ScopedPath::new(DEFAULT_LEDGER_ROOT).expect("default ledger root is a valid scoped path") // safety: DEFAULT_LEDGER_ROOT is also valid in the scoped path grammar.
}

pub(super) fn scoped_ledger_root_for_scope(root: ScopedPath, scope: &ResourceScope) -> ScopedPath {
    let agent_id = scope
        .agent_id
        .as_ref()
        .map(|agent_id| agent_id.as_str())
        .unwrap_or("_");
    let project_id = scope
        .project_id
        .as_ref()
        .map(|project_id| project_id.as_str())
        .unwrap_or("_");
    let mission_id = scope
        .mission_id
        .as_ref()
        .map(|mission_id| mission_id.as_str())
        .unwrap_or("_");
    let thread_id = scope
        .thread_id
        .as_ref()
        .map(|thread_id| thread_id.as_str())
        .unwrap_or("_");
    let path = format!(
        "{}/_scope/{}/{}/{}/{}/{}/{}",
        root.as_str().trim_end_matches('/'),
        hex_component(scope.tenant_id.as_str()),
        hex_component(scope.user_id.as_str()),
        hex_component(agent_id),
        hex_component(project_id),
        hex_component(mission_id),
        hex_component(thread_id)
    );
    ScopedPath::new(path).expect("scope-partitioned ledger root is a valid scoped path") // safety: the input root is valid and every appended component is hex-encoded.
}

fn hex_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
