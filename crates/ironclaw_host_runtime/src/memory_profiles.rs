//! Host-defined memory capability-profile catalog (issue #3537).
//!
//! Capability profiles are host-defined portability contracts: an extension may
//! claim that one of its provider-prefixed capabilities *implements* a profile
//! operation, and the host validates that claim with the conformance harness in
//! [`ironclaw_capabilities`]. This module owns the concrete, host-defined memory
//! profile contracts so the manifest layer (the bundled `ironclaw.memory.native`
//! extension), the binding layer (`profile_id -> extension_id`), and the repo
//! conformance tests all read the *same* contract definitions rather than
//! re-deriving them from loose strings.
//!
//! The profiles mirror `docs/reborn/contracts/memory-profiles.md`. Schema refs
//! resolve under `docs/reborn/contracts/` and are drift-guarded by
//! `ironclaw_capabilities`'s `memory_profile_schema_refs_exist` test.
//!
//! `memory.semantic_search.v1` is intentionally absent: it depends on a
//! host-mediated embedding/vector port that does not exist yet (see the doc's
//! "Deferred" section). It must be added before semantic search ships.
//!
//! This module is contract vocabulary only — building a catalog does not bind a
//! provider, grant trust, or dispatch anything.

use ironclaw_host_api::{
    CapabilityProfileContract, CapabilityProfileId, CapabilityProfileOperationContract,
    CapabilityProfileOperationId, HostApiError,
};

/// `memory.context_retrieval.v1` — host-mediated, provider-neutral context
/// retrieval before a model call. Visibility is `host_internal`.
pub const MEMORY_CONTEXT_RETRIEVAL_PROFILE_ID: &str = "memory.context_retrieval.v1";
/// `memory.interaction_log.v1` — host-recorded sanitized interaction logging.
/// Visibility is `host_internal`.
pub const MEMORY_INTERACTION_LOG_PROFILE_ID: &str = "memory.interaction_log.v1";
/// `memory.document_store.v1` — the read/write document store that backs
/// `/memory` filesystem routing and model-facing memory tools.
pub const MEMORY_DOCUMENT_STORE_PROFILE_ID: &str = "memory.document_store.v1";

/// Every host-defined memory profile id, in catalog order. Kept as a slice so
/// the binding layer can iterate the *required* profile set without rebuilding
/// the (validated) contract objects.
pub const MEMORY_CAPABILITY_PROFILE_IDS: &[&str] = &[
    MEMORY_CONTEXT_RETRIEVAL_PROFILE_ID,
    MEMORY_INTERACTION_LOG_PROFILE_ID,
    MEMORY_DOCUMENT_STORE_PROFILE_ID,
];

/// Required operation of `memory.context_retrieval.v1`.
pub const MEMORY_CONTEXT_RETRIEVE_OPERATION_ID: &str = "memory.context.retrieve.v1";
/// Required operation of `memory.interaction_log.v1`.
pub const MEMORY_INTERACTION_RECORD_OPERATION_ID: &str = "memory.interaction.record.v1";
/// First required operation of `memory.document_store.v1`.
pub const MEMORY_DOCUMENT_READ_OPERATION_ID: &str = "memory.document.read.v1";
/// Second required operation of `memory.document_store.v1`.
pub const MEMORY_DOCUMENT_WRITE_OPERATION_ID: &str = "memory.document.write.v1";

/// Extension-local relative schema refs (resolve under `docs/reborn/contracts/`).
const CONTEXT_RETRIEVE_INPUT_SCHEMA_REF: &str = "schemas/memory/context-retrieve.input.v1.json";
const CONTEXT_RETRIEVE_OUTPUT_SCHEMA_REF: &str = "schemas/memory/context-retrieve.output.v1.json";
const INTERACTION_RECORD_INPUT_SCHEMA_REF: &str = "schemas/memory/interaction-record.input.v1.json";
const INTERACTION_RECORD_OUTPUT_SCHEMA_REF: &str =
    "schemas/memory/interaction-record.output.v1.json";
const DOCUMENT_READ_INPUT_SCHEMA_REF: &str = "schemas/memory/document-read.input.v1.json";
const DOCUMENT_READ_OUTPUT_SCHEMA_REF: &str = "schemas/memory/document-read.output.v1.json";
const DOCUMENT_WRITE_INPUT_SCHEMA_REF: &str = "schemas/memory/document-write.input.v1.json";
const DOCUMENT_WRITE_OUTPUT_SCHEMA_REF: &str = "schemas/memory/document-write.output.v1.json";

fn operation(
    id: &str,
    input_schema_ref: &str,
    output_schema_ref: &str,
) -> Result<CapabilityProfileOperationContract, HostApiError> {
    CapabilityProfileOperationContract::new(
        CapabilityProfileOperationId::new(id)?,
        input_schema_ref,
        output_schema_ref,
    )
}

/// Build the `memory.context_retrieval.v1` profile contract.
pub fn context_retrieval_profile() -> Result<CapabilityProfileContract, HostApiError> {
    CapabilityProfileContract::new(
        CapabilityProfileId::new(MEMORY_CONTEXT_RETRIEVAL_PROFILE_ID)?,
        vec![operation(
            MEMORY_CONTEXT_RETRIEVE_OPERATION_ID,
            CONTEXT_RETRIEVE_INPUT_SCHEMA_REF,
            CONTEXT_RETRIEVE_OUTPUT_SCHEMA_REF,
        )?],
    )
}

/// Build the `memory.interaction_log.v1` profile contract.
pub fn interaction_log_profile() -> Result<CapabilityProfileContract, HostApiError> {
    CapabilityProfileContract::new(
        CapabilityProfileId::new(MEMORY_INTERACTION_LOG_PROFILE_ID)?,
        vec![operation(
            MEMORY_INTERACTION_RECORD_OPERATION_ID,
            INTERACTION_RECORD_INPUT_SCHEMA_REF,
            INTERACTION_RECORD_OUTPUT_SCHEMA_REF,
        )?],
    )
}

/// Build the `memory.document_store.v1` profile contract (two required ops).
pub fn document_store_profile() -> Result<CapabilityProfileContract, HostApiError> {
    CapabilityProfileContract::new(
        CapabilityProfileId::new(MEMORY_DOCUMENT_STORE_PROFILE_ID)?,
        vec![
            operation(
                MEMORY_DOCUMENT_READ_OPERATION_ID,
                DOCUMENT_READ_INPUT_SCHEMA_REF,
                DOCUMENT_READ_OUTPUT_SCHEMA_REF,
            )?,
            operation(
                MEMORY_DOCUMENT_WRITE_OPERATION_ID,
                DOCUMENT_WRITE_INPUT_SCHEMA_REF,
                DOCUMENT_WRITE_OUTPUT_SCHEMA_REF,
            )?,
        ],
    )
}

/// Build the full host-defined memory capability-profile catalog.
///
/// Returned in [`MEMORY_CAPABILITY_PROFILE_IDS`] order. Construction validates
/// every id, operation id, and schema ref, so a malformed catalog fails closed
/// at the host (and in tests) rather than at dispatch.
pub fn memory_capability_profiles() -> Result<Vec<CapabilityProfileContract>, HostApiError> {
    Ok(vec![
        context_retrieval_profile()?,
        interaction_log_profile()?,
        document_store_profile()?,
    ])
}

/// Look up one host-defined memory profile contract by id.
///
/// Returns `Ok(None)` for a well-formed id that is not a host-defined memory
/// profile (e.g. a third-party profile), and `Err` only if the host catalog
/// itself fails to build. Used by the binding layer to fail closed when a
/// configured `profile_id` is not a known memory profile.
pub fn memory_capability_profile(
    profile_id: &CapabilityProfileId,
) -> Result<Option<CapabilityProfileContract>, HostApiError> {
    Ok(memory_capability_profiles()?
        .into_iter()
        .find(|contract| contract.id() == profile_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_builds_three_profiles_in_declared_order() {
        let catalog = memory_capability_profiles().expect("catalog must build");
        let ids: Vec<&str> = catalog.iter().map(|c| c.id().as_str()).collect();
        assert_eq!(ids, MEMORY_CAPABILITY_PROFILE_IDS);
    }

    #[test]
    fn context_retrieval_requires_the_retrieve_operation() {
        let contract = context_retrieval_profile().expect("context retrieval profile");
        let ops: Vec<&str> = contract
            .required_operations()
            .iter()
            .map(|op| op.id().as_str())
            .collect();
        assert_eq!(ops, vec![MEMORY_CONTEXT_RETRIEVE_OPERATION_ID]);
    }

    #[test]
    fn document_store_requires_read_and_write_operations() {
        let contract = document_store_profile().expect("document store profile");
        let mut ops: Vec<&str> = contract
            .required_operations()
            .iter()
            .map(|op| op.id().as_str())
            .collect();
        // `CapabilityProfileContract` sorts required operations by id.
        ops.sort_unstable();
        assert_eq!(
            ops,
            vec![
                MEMORY_DOCUMENT_READ_OPERATION_ID,
                MEMORY_DOCUMENT_WRITE_OPERATION_ID
            ]
        );
    }

    #[test]
    fn lookup_returns_known_profile_and_none_for_unknown() {
        let known = CapabilityProfileId::new(MEMORY_DOCUMENT_STORE_PROFILE_ID).unwrap();
        let found = memory_capability_profile(&known).expect("lookup must not error");
        assert_eq!(found.as_ref().map(|c| c.id()), Some(&known));

        // A well-formed id that is not a host-defined memory profile.
        let other = CapabilityProfileId::new("honcho.representation.v1").unwrap();
        let missing = memory_capability_profile(&other).expect("lookup must not error");
        assert!(missing.is_none());
    }
}
