//! Guard that draft memory profile schema refs documented in
//! `docs/reborn/contracts/memory-profiles.md` resolve to real files on disk.
//!
//! This is purely a drift guard; no runtime behavior depends on these files
//! yet, but renames must not silently break the docs/contract layer.

use std::path::PathBuf;

use ironclaw_host_api::CapabilityProfileSchemaRef;

const REFS: &[&str] = &[
    "schemas/memory/context-retrieve.input.v1.json",
    "schemas/memory/context-retrieve.output.v1.json",
    "schemas/memory/interaction-record.input.v1.json",
    "schemas/memory/interaction-record.output.v1.json",
    "schemas/memory/document-read.input.v1.json",
    "schemas/memory/document-read.output.v1.json",
    "schemas/memory/document-write.input.v1.json",
    "schemas/memory/document-write.output.v1.json",
];

#[test]
fn memory_profile_schema_refs_exist_on_disk() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let contracts_dir = manifest_dir
        .ancestors()
        .map(|ancestor| ancestor.join("docs/reborn/contracts"))
        .find(|candidate| candidate.is_dir())
        .expect("could not locate docs/reborn/contracts from crate manifest dir");

    for raw in REFS {
        // Every documented ref must pass the shared schema-ref validator.
        let parsed = CapabilityProfileSchemaRef::new(*raw)
            .unwrap_or_else(|err| panic!("schema ref {raw:?} failed validation: {err}"));
        let path = contracts_dir.join(parsed.as_str());
        assert!(
            path.is_file(),
            "missing memory profile schema file: {}",
            path.display()
        );
    }
}
