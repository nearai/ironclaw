# Coding Artifact Read Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the model-visible `builtin.result_read` tool and recover all truncated capability output through pinned coding `read artifact://<numeric-id>:<selector>`.

**Architecture:** Store new capability output as immutable, chunked artifacts under a namespace shared by one root run and its descendants. Keep `result_ref` as internal completion evidence, but expose only bounded previews and `artifact://` references to the model. Resolve artifacts through a scoped host port, enforce resource budgets while writing, and retain a private adapter for historical result records.

**Tech Stack:** Rust, Tokio, serde/serde_json, `ironclaw_host_api`, `ironclaw_loop_contracts`, `ironclaw_threads`, `ironclaw_filesystem`, `ironclaw_resources`, first-party host runtime, composition, root integration harness.

**Approved design:** `docs/internal/superpowers/specs/2026-08-11-coding-artifact-read-cutover-design.md`

---

## File map and ownership

| Responsibility | Files |
|---|---|
| Pinned artifact URI contract | `tests/fixtures/pinned_coding_contract/**`, `tests/reborn_coding_contract_snapshot.rs` |
| Neutral artifact identities and host port | Create `crates/contracts/ironclaw_host_api/src/artifact.rs`; modify `crates/contracts/ironclaw_host_api/src/lib.rs` |
| Model observation and continuation metadata | `crates/contracts/ironclaw_loop_contracts/src/model_observation.rs`, `crates/contracts/ironclaw_host_api/src/resolution.rs`, `crates/contracts/ironclaw_loop_contracts/src/resolution.rs`, `crates/domains/ironclaw_threads/src/tool_result_reference.rs`, `crates/loop/ironclaw_agent_loop/src/executor/capabilities.rs` |
| Run-tree artifact namespace | `crates/contracts/ironclaw_loop_contracts/src/host/run_context.rs`, `crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs`, subagent/recovery tests |
| Durable artifact grammar and storage | Create `crates/domains/ironclaw_threads/src/tool_artifacts.rs`; modify `crates/domains/ironclaw_threads/src/lib.rs` and crate tests |
| Invocation-scoped artifact services | `crates/kernel/ironclaw_host_runtime/src/invocation_services.rs`, `crates/kernel/ironclaw_host_runtime/src/first_party.rs`, composition factory/runtime files |
| `artifact://` reading | `crates/extensions/ironclaw_extension_support/src/coding/pinned/read.rs`, `mod.rs`, engine tests |
| Artifact-producing result writer | `crates/app/ironclaw_composition/src/runtime/capability_host.rs` and tests |
| Historical projection | `crates/loop/ironclaw_loop_host/src/model_gateway.rs`, `lib.rs`, thread result-record compatibility methods |
| Streaming high-output producers | `crates/kernel/ironclaw_host_runtime/src/first_party.rs`, `first_party_tools/shell*.rs`, `first_party_tools/coding.rs` |
| Atomic removal of result reader | `crates/loop/ironclaw_loop_host/src/result_read.rs`, composition refreshing port/test support, tool disclosure, integration harness |
| Production-shaped coverage | Create `tests/integration/reborn_coding_artifact_read_cutover.rs`; update integration support only where required |

Production must keep the existing result-reader arm until Task 11. Tasks 1–10 use unit tests and test-only construction; no production provider surface may disclose both paging mechanisms.

---

### Task 1: Pin the upstream artifact URI contract

**Files:**
- Create: `tests/fixtures/pinned_coding_contract/golden/artifact_urls.json`
- Create: `tests/fixtures/pinned_coding_contract/output/artifact_protocol.json`
- Modify: `tests/fixtures/pinned_coding_contract/manifest.json`
- Modify: `tests/fixtures/pinned_coding_contract/provenance.json`
- Modify: `tests/reborn_coding_contract_snapshot.rs`
- Modify: `tests/support/pinned_coding_contract/mod.rs`

- [ ] **Step 1: Add failing inventory and provenance assertions**

Add a test that requires the new artifact corpus and checks its pinned source files:

```rust
#[test]
fn artifact_protocol_cases_are_pinned() {
    let snapshot = pinned_contract();
    let cases = snapshot.json_fixture("golden/artifact_urls.json");
    assert_eq!(cases["pinned_commit"], PINNED_COMMIT);
    assert_eq!(cases["cases"].as_array().map(Vec::len), Some(9));
    snapshot.assert_source_sha("packages/coding-agent/src/internal-urls/artifact-protocol.ts");
    snapshot.assert_source_sha("packages/coding-agent/src/session/artifacts.ts");
}
```

- [ ] **Step 2: Run the snapshot test and confirm red**

Run:

```bash
cargo test --test reborn_coding_contract_snapshot artifact_protocol_cases_are_pinned -- --exact
```

Expected: FAIL because `golden/artifact_urls.json` is absent from the manifest.

- [ ] **Step 3: Capture the exact pinned cases**

The fixture must contain these case IDs and expected strings from the pinned upstream commit `08819b279cf02ae2545e69dad7111ab48d91d35e`:

```json
{
  "pinned_commit": "08819b279cf02ae2545e69dad7111ab48d91d35e",
  "cases": [
    {"id":"missing_id","path":"artifact://","error":"artifact:// URL requires a numeric ID: artifact://0"},
    {"id":"nonnumeric_id","path":"artifact://abc","error":"artifact:// ID must be numeric, got: abc"},
    {"id":"missing_artifact","path":"artifact://9","error":"Artifact 9 not found. Available: none"},
    {"id":"full_text","path":"artifact://0","selector":null},
    {"id":"single_range","path":"artifact://0:2-4","selector":"2-4"},
    {"id":"multi_range","path":"artifact://0:1-2,5-6","selector":"1-2,5-6"},
    {"id":"raw_range","path":"artifact://0:raw:2-4","selector":"raw:2-4"},
    {"id":"binary_without_raw","path":"artifact://1","selector":null},
    {"id":"oversized_unsliced","path":"artifact://2","selector":null,"security_deviation":"omit_backing_host_path"}
  ]
}
```

Copy the two pinned TypeScript sources into `tests/fixtures/pinned_coding_contract/sources/` and record their SHA-256 values in provenance. Preserve the upstream MIT license reference already present in the fixture set.

- [ ] **Step 4: Pin the hosted-security deviation separately**

For `oversized_unsliced`, preserve upstream's selector guidance but replace its host-path clause with:

```text
Artifact 2 is <bytes> bytes; full internal resolution is blocked. Use read selectors such as artifact://2:1-3000 or artifact://2:raw:1-3000.
```

Record `deviation_reason = "host paths are not model-visible authority"` so this one difference cannot spread silently.

- [ ] **Step 5: Run the complete fixture suite**

Run:

```bash
cargo test --test reborn_coding_contract_snapshot
```

Expected: all snapshot, provenance, checksum, and inventory tests PASS.

- [ ] **Step 6: Commit the contract fixture**

```bash
git add tests/fixtures/pinned_coding_contract tests/reborn_coding_contract_snapshot.rs tests/support/pinned_coding_contract/mod.rs
git commit -m "test(coding): pin coding artifact URI contract"
```

---

### Task 2: Add neutral artifact identities and the host port

**Files:**
- Create: `crates/contracts/ironclaw_host_api/src/artifact.rs`
- Modify: `crates/contracts/ironclaw_host_api/src/lib.rs`
- Modify: `crates/contracts/ironclaw_host_api/tests/host_api_contract.rs`

- [ ] **Step 1: Write failing identity and scope tests**

```rust
#[test]
fn artifact_refs_are_numeric_and_not_authority_bearing() {
    let id = ArtifactId::new(7).expect("positive artifact id");
    assert_eq!(ArtifactRef::new(id).to_string(), "artifact://7");
    assert!(ArtifactRef::parse("artifact://abc").is_err());
    assert!(ArtifactRef::parse("artifact://7/path").is_err());
}

#[test]
fn artifact_read_requests_require_namespace_and_owner_scope() {
    let request = artifact_read_request(7, ArtifactSelector::Lines { start: 3, end: 9 });
    assert_eq!(request.namespace.as_run_id(), host_run_id(root_run_id()));
    assert_eq!(request.owner_scope.tenant_id, tenant_id());
    assert_eq!(request.owner_scope.agent_id, agent_id());
}
```

- [ ] **Step 2: Run the contract tests and confirm red**

```bash
cargo test -p ironclaw_host_api artifact_ -- --nocapture
```

Expected: FAIL with unresolved artifact types.

- [ ] **Step 3: Implement the vocabulary and port**

Create the module with these public shapes:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactId(u64);

impl ArtifactId {
    pub fn new(value: u64) -> Result<Self, HostApiError>;
    pub const fn get(self) -> u64;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactNamespaceId(RunId);

impl ArtifactNamespaceId {
    pub fn from_root_run(run_id: RunId) -> Self;
    pub fn as_run_id(&self) -> RunId;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactOwnerScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactRef {
    id: ArtifactId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactSelector {
    Full,
    Lines { start: u64, end: u64 },
    MultiLines(Vec<ArtifactLineRange>),
    RawLines { start: u64, end: u64 },
}

#[derive(Debug, Clone)]
pub struct ArtifactReadRequest {
    pub owner_scope: ArtifactOwnerScope,
    pub namespace: ArtifactNamespaceId,
    pub artifact_id: ArtifactId,
    pub selector: ArtifactSelector,
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReadChunk {
    pub content: Vec<u8>,
    pub content_type: String,
    pub total_bytes: u64,
    pub total_lines: Option<u64>,
    pub complete: bool,
}

#[async_trait]
pub trait ArtifactAccessPort: Send + Sync {
    async fn read(&self, request: ArtifactReadRequest)
        -> Result<Option<ArtifactReadChunk>, ArtifactAccessError>;
}
```

Define the host-owned `ArtifactPersistencePort` in the same module. Its `allocate`, `append`, and `finalize` request types carry `ArtifactOwnerScope` and `ArtifactNamespaceId`; `append` also carries a validated opaque `ArtifactWriteHandle`. The port returns `CompletedArtifact { artifact_ref, byte_len, total_lines, content_type, digest }`. `ArtifactOwnerScope::from_resource_scope` copies only tenant, owner user, agent, and project; mission, thread, and invocation IDs are deliberately excluded so descendants in one spawn tree share artifacts. This is the only unscoped write contract. Task 6 adds wrappers that capture authority before a tool runs.

- [ ] **Step 4: Validate parsing without paths or query strings**

`ArtifactRef::parse` must accept only `artifact://<decimal u64>`. Match upstream allocation exactly: zero is valid and a fresh namespace allocates `artifact://0`. Reject signs, whitespace, path suffixes, query strings, fragments, and overflow consistently in the parser and fixture tests.

- [ ] **Step 5: Run host API tests**

```bash
cargo test -p ironclaw_host_api artifact_
```

Expected: PASS.

- [ ] **Step 6: Commit the neutral contracts**

```bash
git add crates/contracts/ironclaw_host_api/src/artifact.rs crates/contracts/ironclaw_host_api/src/lib.rs crates/contracts/ironclaw_host_api/tests/host_api_contract.rs
git commit -m "feat(host-api): define scoped artifact references"
```

---

### Task 3: Add artifact-backed model observations

**Files:**
- Modify: `crates/contracts/ironclaw_loop_contracts/src/model_observation.rs`
- Modify: `crates/contracts/ironclaw_host_api/src/resolution.rs`
- Modify: `crates/contracts/ironclaw_loop_contracts/src/resolution.rs`
- Modify: `crates/domains/ironclaw_threads/src/tool_result_reference.rs`
- Modify: `crates/loop/ironclaw_agent_loop/src/executor/capabilities.rs`

- [ ] **Step 1: Write failing round-trip tests**

Add one contract test for a truncated artifact observation and one for a complete inline result:

```rust
let observation = ModelVisibleToolObservation {
    schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
    status: ToolObservationStatus::Success,
    summary: "Tool completed; full output: artifact://7".to_string(),
    detail: ToolObservationDetail::ArtifactReference {
        artifact_ref: "artifact://7".to_string(),
        byte_len: 100_000,
        preview: Some("first bounded lines".to_string()),
        total_bytes: 100_000,
        item_count: None,
    },
    artifacts: vec![ModelVisibleArtifact {
        artifact_ref: "artifact://7".to_string(),
        summary: "Full tool output".to_string(),
    }],
    recovery: None,
    trust: ObservationTrust::UntrustedToolOutput,
};
observation.validate().expect("artifact observation validates");
```

Assert serialized content contains `artifact://7` and contains neither `next_offset` nor a model-visible `result_ref`.

- [ ] **Step 2: Confirm the new variant is red**

```bash
cargo test -p ironclaw_loop_contracts artifact_reference
```

Expected: FAIL because `ArtifactReference` does not exist.

- [ ] **Step 3: Add `ArtifactReference` and preview metadata**

Add this non-exhaustive detail variant:

```rust
ArtifactReference {
    artifact_ref: String,
    byte_len: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
    total_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    item_count: Option<u64>,
},
```

Add `artifact_ref: Option<String>` to `ResultPreviewMeta`. Keep the old fields deserializable for historical records, but new constructors must not populate `referenced_result_ref` or `next_offset`.

- [ ] **Step 4: Update validation and replay rendering**

Update both typed validation in `model_observation.rs` and JSON-shape validation in `tool_result_reference.rs`. Artifact refs must parse through `ArtifactRef`, previews remain untrusted output, and artifact metadata survives safe-summary fallback.

Update `result_preview_parts` to accept both variants:

```rust
match detail {
    ToolObservationDetail::ArtifactReference {
        artifact_ref,
        preview,
        total_bytes,
        item_count,
        ..
    } => artifact_preview_parts(artifact_ref, preview, total_bytes, item_count, summary),
    ToolObservationDetail::ResultReference { .. } => legacy_result_preview_parts(...),
    _ => empty,
}
```

Update `result_reference_observation_from_outcome` to emit `ArtifactReference` whenever `preview_meta.artifact_ref` is present. Retain `ResultReference` reconstruction only for historical outcomes.

- [ ] **Step 5: Run the affected contract suites**

```bash
cargo test -p ironclaw_loop_contracts artifact_reference
cargo test -p ironclaw_threads tool_result_reference
cargo test -p ironclaw_agent_loop artifact_reference
```

Expected: PASS, including old result-reference round trips.

- [ ] **Step 6: Commit observation support**

```bash
git add crates/contracts/ironclaw_loop_contracts/src/model_observation.rs crates/contracts/ironclaw_host_api/src/resolution.rs crates/contracts/ironclaw_loop_contracts/src/resolution.rs crates/domains/ironclaw_threads/src/tool_result_reference.rs crates/loop/ironclaw_agent_loop/src/executor/capabilities.rs
git commit -m "feat(loop): carry artifact-backed result previews"
```

---

### Task 4: Propagate one artifact namespace through a spawn tree and capability dispatch

**Files:**
- Modify: `crates/contracts/ironclaw_loop_contracts/src/host/run_context.rs`
- Modify: `crates/contracts/ironclaw_host_api/src/scope.rs`
- Modify: `crates/contracts/ironclaw_host_api/src/dispatch.rs`
- Modify: `crates/kernel/ironclaw_capabilities/src/dispatch.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/services/tool_resolver.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/services/runtime_adapters.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/invocation_services.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party.rs`
- Modify: `crates/loop/ironclaw_loop_host/src/capability_port.rs`
- Modify: `crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs`
- Modify: `crates/loop/ironclaw_loop_host/src/subagent_spawn_port.rs`
- Modify: `crates/loop/ironclaw_turn_runner/src/subagent/await_edge/resolver.rs`
- Test: existing run-context, dispatch, invocation-service, and subagent tests in those crates

- [ ] **Step 1: Write failing root/child/recovery and dispatch tests**

```rust
#[tokio::test]
async fn claimed_root_and_descendant_resolve_one_artifact_namespace() {
    let root = claimed_run(None);
    let child = claimed_run(Some(root.state.run_id));
    assert_eq!(host_context(&root).await.artifact_namespace.as_run_id(), host_run_id(root.state.run_id));
    assert_eq!(host_context(&child).await.artifact_namespace.as_run_id(), host_run_id(root.state.run_id));
}
```

Add a recovery assertion that reconstructed parent context preserves the namespace stored at spawn time. Add a dispatch test that records the value at `FirstPartyCapabilityRequest` and proves it equals the loop context's effective namespace; a non-loop request must carry `None`.

- [ ] **Step 2: Confirm the namespace fields are missing**

```bash
cargo test -p ironclaw_turn_runner artifact_namespace
cargo test -p ironclaw_host_runtime artifact_namespace
```

Expected: FAIL with no artifact namespace field.

- [ ] **Step 3: Extend `LoopRunContext` compatibly**

Add:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub artifact_namespace: Option<ArtifactNamespaceId>,
```

Add a total accessor:

```rust
pub fn effective_artifact_namespace(&self) -> ArtifactNamespaceId {
    self.artifact_namespace.clone().unwrap_or_else(|| {
        ArtifactNamespaceId::from_root_run(
            ironclaw_host_api::ids::RunId::from_uuid(self.run_id.as_uuid()),
        )
    })
}
```

Old serialized contexts therefore resolve to their own run without migration.

- [ ] **Step 4: Resolve production context from durable run state**

In `RebornLoopDriverHostFactory::create_host`, set:

```rust
let artifact_root = claimed
    .state
    .spawn_tree_root_run_id
    .unwrap_or(claimed.state.run_id);
loop_run_context.artifact_namespace = Some(ArtifactNamespaceId::from_root_run(
    ironclaw_host_api::ids::RunId::from_uuid(artifact_root.as_uuid()),
));
```

At child spawn, persist the parent's effective namespace in cached parent context. Recovery must cross-check the durable child's `spawn_tree_root_run_id` before trusting cached context.

- [ ] **Step 5: Seal the namespace into execution and dispatch**

Add `artifact_namespace: Option<ArtifactNamespaceId>` to `ExecutionContext`, `CapabilityDispatchRequest`, `RuntimeLaneRequest`, `InvocationServicesResolutionRequest`, and `FirstPartyCapabilityRequest`.

`invocation_context_from_visible` copies `run_context.effective_artifact_namespace()` into `ExecutionContext`. Authorization must seal the value into the authorized invocation instead of accepting it from capability input. `RuntimeDispatcher`, `RegistryLaneToolResolver`, and both runtime-adapter service-resolution calls forward it unchanged. Non-loop callers retain `None`. Do not derive the namespace from `run_id` after the loop boundary because descendants deliberately use the root run's ID.

- [ ] **Step 6: Run runner, capability, runtime, and subagent tests**

```bash
cargo test -p ironclaw_host_api artifact_namespace
cargo test -p ironclaw_capabilities artifact_namespace
cargo test -p ironclaw_turn_runner artifact_namespace
cargo test -p ironclaw_loop_host subagent --lib
cargo test -p ironclaw_host_runtime artifact_namespace
```

Expected: PASS.

- [ ] **Step 7: Commit namespace propagation**

```bash
git add crates/contracts/ironclaw_loop_contracts/src/host/run_context.rs crates/contracts/ironclaw_host_api/src crates/kernel/ironclaw_capabilities/src/dispatch.rs crates/kernel/ironclaw_host_runtime/src crates/loop/ironclaw_loop_host/src crates/loop/ironclaw_turn_runner/src
git commit -m "feat(turns): propagate artifact run-tree namespace"
```

---

### Task 5: Implement immutable chunked artifact storage

**Files:**
- Create: `crates/domains/ironclaw_threads/src/tool_artifacts.rs`
- Modify: `crates/domains/ironclaw_threads/src/lib.rs`
- Modify: `crates/kernel/ironclaw_resources/src/lib.rs`
- Modify: `crates/kernel/ironclaw_resources/src/filesystem_governor.rs`
- Test: `crates/domains/ironclaw_threads/tests/session_thread_contract.rs`
- Test: add backend conformance coverage beside existing filesystem service suites
- Test: existing in-memory, persistent-store, and filesystem resource-governor conformance suites

- [ ] **Step 1: Write failing store conformance cases**

Define one shared async suite and run it against in-memory, local, libSQL, and PostgreSQL roots. It must prove:

```rust
pub async fn tool_artifact_store_conformance(
    store: &DurableToolArtifactStore,
    persistence: &dyn ArtifactPersistencePort,
    access: &dyn ArtifactAccessPort,
) {
    let first = persistence.allocate(context()).await.expect("allocate first");
    let second = persistence.allocate(context()).await.expect("allocate second");
    assert_eq!(first.artifact_id.get(), 0);
    assert_eq!(second.artifact_id.get(), 1);

    persistence.append(append(&first, b"one\ntwo\n")).await.expect("append");
    assert!(access.read(read(first.artifact_id)).await.expect("read").is_none());

    persistence.append(append(&first, b"three\nfour\n")).await.expect("append");
    let completed = persistence.finalize(finalize(first, producer_meta())).await
        .expect("finalize");
    let lines = access.read(lines_request(completed.artifact_id, 2, 3)).await
        .expect("read").expect("finalized artifact");
    assert_eq!(lines.content, b"two\nthree\n");
}
```

Run the same suite against in-memory, local, libSQL, and PostgreSQL `RootFilesystem` implementations. Add concurrent allocation, digest mismatch, incomplete visibility, quota rejection before append, restart, and cross-scope cases. `DurableToolArtifactStore` is the concrete threads-owned adapter implementing the two neutral host ports; do not introduce a second store trait.

- [ ] **Step 2: Confirm the store and reservation-growth operation are red**

```bash
cargo test -p ironclaw_threads tool_artifact --features test-support
cargo test -p ironclaw_resources grow_reservation
```

Expected: FAIL because the concrete store and `ResourceGovernor::grow_reservation` do not exist.

- [ ] **Step 3: Define artifact metadata and write handles**

Use strong types and a fixed chunk size:

```rust
pub const TOOL_ARTIFACT_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolArtifactBacking {
    ChunkSet { chunks: Vec<ToolArtifactChunkMeta> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolArtifactMetadata {
    pub artifact_id: ArtifactId,
    pub namespace: ArtifactNamespaceId,
    pub owner_scope: ArtifactOwnerScope,
    pub producer_capability_id: CapabilityId,
    pub content_type: String,
    pub total_bytes: u64,
    pub total_lines: Option<u64>,
    pub digest: ContentDigest,
    pub backing: ToolArtifactBacking,
    pub created_at: DateTime<Utc>,
    pub finalized_at: Option<DateTime<Utc>>,
}
```

The metadata state is incomplete when `finalized_at` is absent. Do not add a delete method.

- [ ] **Step 4: Add fail-closed reservation growth**

Add `ResourceGovernor::grow_reservation(reservation_id, additional: ResourceEstimate) -> Result<ResourceReservation, ResourceError>` and implement it for `PersistentResourceGovernor`, `FilesystemResourceGovernor`, and `InMemoryResourceGovernor`. The operation uses the existing bounded CAS transaction, verifies the reservation remains active, checks the full account cascade, and increases only reserved capacity. It does not charge usage; final `reconcile` charges the actual output once.

Cover concurrent growth, limit rejection, closed reservations, storage failure, and reconcile-after-growth. This is the pre-persistence quota primitive used by the artifact writer: grow first, then persist the accepted chunk.

- [ ] **Step 5: Implement backend-neutral storage through `RootFilesystem`**

Use the repository's bounded CAS helper for the namespace counter. Store metadata and chunks as opaque entries. Create incomplete metadata before the first chunk, write each chunk with `CasExpectation::Absent`, and finalize metadata with `CasExpectation::Version`.

Build paths only from validated tenant, owner-user, agent, project, namespace, numeric artifact ID, and chunk-index segments. Never include mission, thread, or invocation IDs, and never concatenate provider tool names into paths.

- [ ] **Step 6: Add indexed line-range reads**

Track each chunk's `start_byte`, `end_byte`, `start_line`, and `end_line` while writing. A line selector loads only descriptors that intersect the requested range, reads only those chunk entries, and trims boundary lines. Full unsliced reads above the pinned inline threshold return an `OversizedUnsliced` domain error without loading chunks.

- [ ] **Step 7: Preserve incomplete bytes**

On write failure, leave incomplete metadata and committed chunks. They remain queryable through operator/storage diagnostics but `read` returns `None` to model callers. No cleanup routine may delete them.

- [ ] **Step 8: Run all storage backends and governor implementations**

```bash
cargo test -p ironclaw_threads tool_artifact --features test-support
cargo test -p ironclaw_resources grow_reservation
```

Expected: in-memory and local PASS without external services; libSQL PASS; PostgreSQL PASS when Docker/testcontainers are available and report the repository-standard skip otherwise. All three governor implementations pass the same reservation-growth contract.

- [ ] **Step 9: Commit artifact storage**

```bash
git add crates/domains/ironclaw_threads/src/tool_artifacts.rs crates/domains/ironclaw_threads/src/lib.rs crates/domains/ironclaw_threads/tests crates/kernel/ironclaw_resources/src
git commit -m "feat(threads): store quota-bound chunked tool artifacts"
```

---

### Task 6: Wire scoped artifact access into production invocation services

**Files:**
- Modify: `crates/contracts/ironclaw_host_api/src/artifact.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/invocation_services.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/services.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/services/runtime_adapters.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party.rs`
- Modify: `crates/app/ironclaw_composition/src/factory/production_backend_assembly.rs`
- Test: `crates/kernel/ironclaw_host_runtime/src/invocation_services/tests.rs`
- Test: `crates/kernel/ironclaw_host_runtime/src/services/tests.rs`
- Test: `crates/app/ironclaw_composition/src/factory/tests.rs`

- [ ] **Step 1: Write failing scope-binding and quota-order tests**

```rust
#[tokio::test]
async fn invocation_artifact_reader_is_bound_to_run_tree_scope() {
    let services = resolver.resolve(request_for(run_a())).expect("services");
    let chunk = services.artifact_reader.expect("reader")
        .read(ArtifactReadTarget::new(id(0), lines(1, 2)))
        .await.expect("read");
    assert_eq!(recorded_request().namespace, run_a().effective_artifact_namespace());
    assert_eq!(recorded_request().scope, run_a().scope.to_resource_scope());
}
```

Assert a caller cannot supply a different namespace in tool input. Add a first-party adapter test proving `grow_reservation` occurs before the corresponding artifact append, and that a growth rejection writes no chunk.

- [ ] **Step 2: Confirm the service fields are red**

```bash
cargo test -p ironclaw_host_runtime invocation_artifact
```

Expected: FAIL because `InvocationServices` has no artifact binding.

- [ ] **Step 3: Add pre-scoped wrappers**

Add these interfaces to `artifact.rs`:

```rust
#[async_trait]
pub trait ScopedArtifactReader: Send + Sync {
    async fn read(&self, target: ArtifactReadTarget)
        -> Result<Option<ArtifactReadChunk>, ArtifactAccessError>;
}

#[async_trait]
pub trait ArtifactWriter: Send + Sync {
    async fn allocate(&self, metadata: ArtifactWriteMetadata)
        -> Result<ArtifactWriteHandle, ArtifactWriteError>;
    async fn append(&self, handle: &ArtifactWriteHandle, chunk: &[u8])
        -> Result<(), ArtifactWriteError>;
    async fn finalize(&self, handle: ArtifactWriteHandle)
        -> Result<CompletedArtifact, ArtifactWriteError>;
}

#[async_trait]
pub trait AccountedArtifactPersister: Send + Sync {
    async fn persist(
        &self,
        metadata: ArtifactWriteMetadata,
        bytes: &[u8],
        receipt: &ResourceReceipt,
    ) -> Result<CompletedArtifact, ArtifactWriteError>;
}
```

All wrappers capture `ArtifactOwnerScope::from_resource_scope(&scope)` and `ArtifactNamespaceId`; model input carries only `ArtifactId` and selector. `ArtifactWriter` additionally captures an active reservation ID and grows it before each append. `AccountedArtifactPersister` accepts only a reconciled receipt whose `ResourceScope` derives to the same owner scope and rejects a byte count above `receipt.actual.output_bytes`; it then persists bounded chunks without charging them again.

- [ ] **Step 4: Add store ports to host-runtime construction**

`HostRuntimeServices` accepts `Arc<dyn ArtifactAccessPort>` and `Arc<dyn ArtifactPersistencePort>` through one builder method. `production_backend_assembly.rs` creates one `DurableToolArtifactStore` over `stores.filesystem` and injects clones of its two ports. Host runtime owns only neutral traits; it must not depend on `ironclaw_threads`.

Add to `InvocationServices`:

```rust
pub artifact_reader: Option<Arc<dyn ScopedArtifactReader>>,
pub artifact_writer: Option<Arc<dyn ArtifactWriter>>,
pub accounted_artifact_persister: Option<Arc<dyn AccountedArtifactPersister>>,
```

`ConfiguredInvocationServicesResolver::resolve` binds the reader and accounted persister whenever `artifact_namespace` is present. Bind the streaming writer only when the request also carries an active reservation ID: reorder the first-party adapter from plan → resolve → reserve to plan → reserve → resolve, pass the reservation ID in `InvocationServicesResolutionRequest`, and leave the existing RAII guard armed across resolution and handler execution. `ServiceResolvedRuntimeAdapter` retains the resolved accounted persister until its inner materialized lane returns a reconciled receipt.

- [ ] **Step 5: Enforce production availability**

The pinned coding `read` capability fails closed with an unavailable host error when an `artifact://` input is used without `artifact_reader`; local paths continue to work. Agent-scoped production host construction fails if either unscoped artifact port is absent. An agent-scoped streaming handler fails if `artifact_writer` is absent, and a materialized adapter fails if `accounted_artifact_persister` is absent. Non-loop/admin dispatches may resolve all scoped fields as `None`.

- [ ] **Step 6: Run host-runtime and composition tests**

```bash
cargo test -p ironclaw_host_runtime invocation_artifact
SKIP_FRONTEND_BUILD=1 cargo test -p ironclaw_composition invocation_artifact
```

Expected: PASS, including reserve-before-resolve ordering, accounted-persistence validation, and fail-closed production wiring.

- [ ] **Step 7: Commit the binding**

```bash
git add crates/contracts/ironclaw_host_api/src/artifact.rs crates/kernel/ironclaw_host_runtime/src crates/app/ironclaw_composition/src/factory/production_backend_assembly.rs
git commit -m "feat(host-runtime): bind scoped artifact services"
```

---

### Task 7: Teach the pinned coding `read` to resolve `artifact://`

**Files:**
- Modify: `crates/extensions/ironclaw_extension_support/src/coding/pinned/mod.rs`
- Modify: `crates/extensions/ironclaw_extension_support/src/coding/pinned/read.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party_tools/coding.rs`
- Test: `tests/reborn_coding_engines.rs`

- [ ] **Step 1: Add failing differential engine cases**

Extend the engine harness with a recording `ScopedArtifactReader` and drive every Task 1 fixture:

```rust
#[tokio::test]
async fn read_artifact_selector_matches_pinned_output() {
    let context = pinned_context_with_artifact(7, "alpha\nbeta\ngamma\ndelta\n");
    let output = pinned::read(&context, json!({"path":"artifact://7:2-3"}))
        .await.expect("artifact read");
    assert_eq!(output["output"], "2:beta\n3:gamma");
    assert_eq!(context.artifact_reads(), vec![lines_target(7, 2, 3)]);
}
```

- [ ] **Step 2: Confirm `artifact://` currently falls into filesystem resolution**

```bash
cargo test --test reborn_coding_engines read_artifact_selector_matches_pinned_output -- --exact
```

Expected: FAIL with path-resolution or not-found output.

- [ ] **Step 3: Add the pre-scoped reader to `CodingEngineContext`**

```rust
pub struct CodingEngineContext {
    pub filesystem: Arc<dyn RootFilesystem>,
    pub mounts: MountView,
    pub scope: ResourceScope,
    pub run_id: Option<RunId>,
    pub snapshots: Arc<CodingSnapshotRegistry>,
    pub artifact_reader: Option<Arc<dyn ScopedArtifactReader>>,
}
```

Update every test constructor explicitly. Do not create an ambient global resolver.

- [ ] **Step 4: Split URI parsing before local path probing**

In `read`, detect `artifact://` before `literal_path_exists` and `resolve_input_path`. Parse the selector once with the existing selector parser, convert it to `ArtifactReadTarget`, invoke the scoped port, then render the returned text with pinned internal-resource formatting.

Do not register Hashline snapshots for artifacts. Do not return a backing virtual or host path.

- [ ] **Step 5: Implement exact errors and binary behavior**

Use the Task 1 fixtures for missing/non-numeric/unavailable IDs, binary content, selectors, and oversized unsliced reads. Keep the one documented hosted deviation pathless.

- [ ] **Step 6: Run engine and registration tests**

```bash
cargo test --test reborn_coding_engines artifact
cargo test --test reborn_coding_contract_snapshot artifact
cargo test -p ironclaw_host_runtime coding --lib
```

Expected: PASS.

- [ ] **Step 7: Commit artifact reads**

```bash
git add crates/extensions/ironclaw_extension_support/src/coding/pinned crates/kernel/ironclaw_host_runtime/src/first_party_tools/coding.rs tests/reborn_coding_engines.rs
git commit -m "feat(coding): read scoped artifact URIs"
```

---

### Task 8: Carry one durable artifact through dispatch and loop observations

**Files:**
- Modify: `crates/contracts/ironclaw_host_api/src/dispatch.rs`
- Modify: `crates/kernel/ironclaw_capabilities/src/dispatch.rs`
- Modify: `crates/kernel/ironclaw_capabilities/src/registry.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/services/runtime_adapters.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/services/wasm_execution.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party.rs`
- Modify: `crates/loop/ironclaw_loop_host/src/capability_port.rs`
- Modify: `crates/app/ironclaw_composition/src/runtime/capability_host.rs`
- Modify: `crates/app/ironclaw_composition/src/runtime/capability_host/outbound_delivery.rs`
- Modify: `crates/app/ironclaw_composition/src/root/product_live_adapters.rs`
- Modify: direct synthetic `CapabilityResultWrite` callers in `crates/loop/ironclaw_loop_host/src/{external_tool_capability.rs,subagent_spawn_port.rs,tool_disclosure_port.rs,skill_activation/skill_activation_capability.rs}` and `crates/product/ironclaw_assistant/src/project_create_capability.rs`
- Test: affected capability-dispatch, runtime-adapter, and composition result-writer suites

- [ ] **Step 1: Write failing dispatch-to-observation tests**

Add a runtime-adapter test that returns a payload above `RESULT_PREVIEW_MAX_BYTES` and asserts the adapter result contains one finalized `CompletedArtifact`, bounded JSON output, and a receipt whose `actual.output_bytes` equals the full serialized bytes.

Add a caller-level test around `StagedCapabilityIo::write_capability_result`:

```rust
#[tokio::test]
async fn truncated_result_uses_completed_artifact_without_continuation_offset() {
    let completed = completed_artifact("artifact://0", RESULT_PREVIEW_MAX_BYTES as u64 + 1);
    let write = io().write_capability_result(result_write(preview(), Some(completed))).await
        .expect("write result");
    let observation = write.model_observation.expect("observation");
    assert_matches!(observation.detail,
        ToolObservationDetail::ArtifactReference { ref artifact_ref, .. }
            if artifact_ref == "artifact://0");
    assert!(!serde_json::to_string(&observation).unwrap().contains("next_offset"));
}
```

Add complete-inline, binary, artifact-persistence failure, quota-growth failure, and a synthetic capability fallback test.

- [ ] **Step 2: Confirm dispatch cannot carry completed artifacts**

```bash
cargo test -p ironclaw_host_runtime runtime_result_artifact
SKIP_FRONTEND_BUILD=1 cargo test -p ironclaw_composition truncated_result_uses_completed_artifact_without_continuation_offset -- --exact
```

Expected: FAIL because runtime and loop result contracts have no completed artifact.

- [ ] **Step 3: Artifactize runtime output at the accounting seam**

Add `artifact: Option<CompletedArtifact>` to `RuntimeAdapterResult` and `CapabilityDispatchResult`.

For materialized Script, MCP, WASM, and first-party results, preserve the existing execution and reconciliation order. After reconciliation succeeds, `ServiceResolvedRuntimeAdapter` or `FirstPartyRuntimeAdapter` serializes the canonical result once and calls `AccountedArtifactPersister` with that exact byte slice and receipt. The persister verifies the receipt covers the bytes, writes bounded chunks, and returns `CompletedArtifact`. The adapter then replaces `output` with the bounded inline preview and forwards the descriptor through `RuntimeDispatcher`. Quota denial therefore occurs before artifact persistence, and persistence does not charge the bytes again.

Streaming first-party producers use the active-reservation writer in Task 10: grow → append for each chunk, then reconcile once after finalization. Add `artifact: None` to test-only/legacy adapter results and make agent-scoped production dispatch reject `None`.

- [ ] **Step 4: Carry the descriptor into `CapabilityResultWrite`**

Add `completed_artifact: Option<CompletedArtifact>` to `CapabilityResultWrite`. The normal loop capability path copies it from `CapabilityDispatchResult`; it never recreates an artifact from preview JSON. Update every direct constructor listed above.

For host-internal synthetic capabilities that do not pass through runtime dispatch, `StagedCapabilityIo` uses an exact-size scoped fallback writer: serialize first, reserve exactly `output_bytes`, persist, then reconcile. This path is not available to external/runtime adapters and cannot accept caller-supplied scope or namespace.

- [ ] **Step 5: Replace new durable result-record writes**

For a normal dispatched result, `StagedCapabilityIo`:

1. validates the completed artifact's byte length and digest against dispatch evidence;
2. stages only the bounded preview best-effort for immediate in-process consumers;
3. creates a complete inline observation or an `ArtifactReference` observation;
4. appends transcript evidence with the original internal `result_ref`.

Remove `DURABLE_TOOL_RESULT_MAX_BYTES` and stop calling `put_tool_result_record` for new writes. Do not store a second copy of output bytes.

- [ ] **Step 6: Render the model hint with the artifact URI**

Replace `truncated_preview_summary` with:

```rust
fn truncated_artifact_summary(artifact_ref: &ArtifactRef, item_count: Option<u64>) -> String {
    let base = format!("Tool completed; full output: {artifact_ref}");
    item_count
        .map(|count| format!("{base}. Full result is a JSON array of {count} items."))
        .unwrap_or(base)
}
```

Populate `ToolObservationDetail::ArtifactReference` and `ModelVisibleArtifact` with the same URI. Keep `CapabilityWriteResult.result_ref`, byte length, output digest, display preview, trajectory observer, diagnostics, and timeline behavior unchanged. Artifact identity never replaces result identity in completion or dependent-run contracts.

- [ ] **Step 7: Run dispatch and result-writer tests**

```bash
cargo test -p ironclaw_capabilities runtime_result_artifact
cargo test -p ironclaw_host_runtime runtime_result_artifact
SKIP_FRONTEND_BUILD=1 cargo test -p ironclaw_composition capability_result
SKIP_FRONTEND_BUILD=1 cargo test -p ironclaw_composition artifact_reference
```

Expected: PASS; each new result has one finalized artifact, one accounting charge, and no new tool-result record.

- [ ] **Step 8: Commit artifact-backed dispatch results**

```bash
git add crates/contracts/ironclaw_host_api/src/dispatch.rs crates/kernel/ironclaw_capabilities/src crates/kernel/ironclaw_host_runtime/src crates/loop/ironclaw_loop_host/src crates/app/ironclaw_composition/src crates/product/ironclaw_assistant/src
git commit -m "feat(loop): carry durable artifacts through tool results"
```

---

### Task 9: Project historical result records into artifacts

**Files:**
- Modify: `crates/domains/ironclaw_threads/src/tool_artifacts.rs`
- Modify: `crates/domains/ironclaw_threads/src/service.rs`
- Modify: `crates/loop/ironclaw_loop_host/src/lib.rs`
- Modify: `crates/loop/ironclaw_loop_host/src/model_gateway.rs`
- Test: `crates/loop/ironclaw_loop_host` replay/model-context tests

- [ ] **Step 1: Write a failing old-thread replay test**

Seed a finalized historical `ToolResultReferenceEnvelope` whose observation contains `ResultReference`, `next_offset`, and a retained old tool-result record. Load model context under the new arm and assert:

```rust
assert!(model_message.content.contains("artifact://0"));
assert!(!model_message.content.contains("result_read"));
assert!(!model_message.content.contains("next_offset"));
assert_eq!(history_message.content, original_historical_json);
```

- [ ] **Step 2: Confirm replay still advertises `result_read`**

```bash
cargo test -p ironclaw_loop_host historical_result_projects_artifact -- --exact
```

Expected: FAIL because replay returns the old observation unchanged.

- [ ] **Step 3: Add an idempotent legacy artifact projection**

Extend `ToolArtifactBacking` with:

```rust
LegacyResultRecord { thread_id: ThreadId, result_ref: String }
```

Before a historical tool-result observation enters model context:

1. detect a truncated `ResultReference` observation;
2. validate that the referenced result is finalized in its original thread;
3. find or create artifact metadata keyed by `(owner_scope, namespace, thread_id, legacy_result_ref)`;
4. use the retained tool-result record as a read-only backing source without copying bytes;
5. return an in-memory `ArtifactReference` observation for model projection;
6. leave the stored envelope unchanged.

Store the allocated numeric mapping durably so replay and concurrent readers return the same artifact ID. Use bounded CAS and make conflicting mappings fail closed. The legacy reader translates artifact line selectors into repeated bounded `read_tool_result_record` calls, retains only intersecting output, and streams digest/line-index construction; it never materializes the full old record.

- [ ] **Step 4: Authorize old child-thread results through run lineage**

The adapter must verify the source thread belongs to the same durable spawn tree before mapping it. Another thread with a guessed `result_ref` must receive the same unavailable result as a missing artifact.

- [ ] **Step 5: Run historical, replay, and compaction tests**

```bash
cargo test -p ironclaw_loop_host historical_result
cargo test -p ironclaw_loop_host model_context
cargo test -p ironclaw_loop_host compaction
```

Expected: PASS; persisted historical bytes/messages remain unchanged.

- [ ] **Step 6: Commit historical compatibility**

```bash
git add crates/domains/ironclaw_threads/src crates/loop/ironclaw_loop_host/src
git commit -m "feat(loop): project historical results as artifacts"
```

---

### Task 10: Stream high-output first-party results under resource quotas

**Files:**
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party_tools/mod.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party_tools/shell.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party_tools/shell_core.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party_tools/coding.rs`
- Test: `crates/kernel/ironclaw_host_runtime/tests/first_party_builtin_tools.rs`

- [ ] **Step 1: Write failing quota and no-copy tests**

Use a recording artifact writer and resource governor. Produce output larger than the old 1 MiB limit and assert:

```rust
assert_eq!(result.output["artifact_ref"], "artifact://0");
assert!(result.output["output"].as_str().unwrap().len() <= RESULT_PREVIEW_MAX_BYTES);
assert_eq!(recorded_artifact_bytes(), produced_bytes());
assert_eq!(resource_usage.output_bytes, produced_bytes() as u64);
```

For budget exhaustion, assert the rejected chunk is absent and no finalized URI is returned.

- [ ] **Step 2: Confirm the old first-party cap fails**

```bash
cargo test -p ironclaw_host_runtime first_party_large_output_artifact -- --exact
```

Expected: FAIL with `FIRST_PARTY_MAX_OUTPUT_BYTES` budget rejection.

- [ ] **Step 3: Add an artifact-backed first-party result shape**

Extend `FirstPartyCapabilityResult` with an optional finalized artifact descriptor while keeping bounded JSON output:

```rust
pub struct FirstPartyCapabilityResult {
    pub output: Value,
    pub usage: ResourceUsage,
    pub artifact: Option<CompletedArtifact>,
}
```

Provide constructors `inline` and `with_artifact`; do not let handlers construct inconsistent byte counts.

- [ ] **Step 4: Stream shell capture directly to the artifact sink**

The shell output collector keeps only the pinned head/tail preview in memory. For every chunk it grows the active reservation, then appends the accepted bytes to `ArtifactWriter`. It finalizes after the process terminates, whether the exit status is zero or nonzero, and appends `[raw output: artifact://N]` exactly as pinned. Only allocation/append/finalize failure leaves an incomplete artifact and suppresses a finalized URI; already accepted bytes remain retained.

- [ ] **Step 5: Make pinned coding adapters artifact-aware without changing small output**

Pinned coding `read`, `glob`, and `grep` keep their pinned bounded outputs. If an engine provides a spill payload, forward it through the same writer and return the URI; otherwise the first-party adapter uses the materialized accounted-persistence path from Task 8. Do not raise local-read output limits merely because artifact storage exists.

- [ ] **Step 6: Account accepted artifact bytes once**

Chunk growth reserves capacity before persistence; it does not charge usage. Final dispatch reconciles the active reservation once with the full artifact byte count and must not add the bounded preview a second time. Persisted byte evidence, reservation growth, and reconciled usage must agree.

- [ ] **Step 7: Run first-party and governor tests**

```bash
cargo test -p ironclaw_host_runtime first_party_large_output
cargo test -p ironclaw_host_runtime shell_artifact
cargo test -p ironclaw_resources artifact
```

Expected: PASS, with outputs above 1 MiB accepted only when the configured run budget allows them.

- [ ] **Step 8: Commit streaming output**

```bash
git add crates/kernel/ironclaw_host_runtime/src/first_party.rs crates/kernel/ironclaw_host_runtime/src/first_party_tools crates/kernel/ironclaw_host_runtime/tests
git commit -m "feat(runtime): stream large tool output to artifacts"
```

---

### Task 11: Atomically remove `builtin.result_read`

**Files:**
- Remove: `crates/loop/ironclaw_loop_host/src/result_read.rs`
- Modify: `crates/loop/ironclaw_loop_host/src/lib.rs`
- Modify: `crates/app/ironclaw_composition/src/runtime/capability_host/refreshing_capability_port.rs`
- Remove/modify: `crates/app/ironclaw_composition/src/test_support/result_read.rs`
- Modify: `crates/app/ironclaw_composition/src/test_support/mod.rs`
- Modify: `crates/loop/ironclaw_loop_host/src/tool_disclosure.rs`
- Modify: `crates/domains/ironclaw_threads/src/contract.rs`
- Modify: `crates/domains/ironclaw_threads/src/tool_result_records.rs`
- Modify: tests and harness profiles that explicitly register or script `result_read`

- [ ] **Step 1: Add a failing provider-surface assertion**

```rust
#[tokio::test]
async fn production_surface_has_read_and_no_result_read() {
    let names = production_provider_tool_names().await;
    assert!(names.contains(&"read".to_string()));
    assert!(!names.contains(&"builtin__result_read".to_string()));
    assert!(!names.contains(&"result_read".to_string()));
}
```

Also assert a truncated production-shaped result advertises `artifact://` and is recoverable with `read`.

- [ ] **Step 2: Confirm the old synthetic tool is still present**

```bash
SKIP_FRONTEND_BUILD=1 cargo test -p ironclaw_composition production_surface_has_read_and_no_result_read -- --exact
```

Expected: FAIL because refreshing capability construction registers it unconditionally.

- [ ] **Step 3: Remove registration, exports, schema, and test wrappers**

Delete `result_read_capability`, its synthetic handler, parser, input schema, test-support wrapper, constants, and composition registration. Remove `RESULT_READ_CAPABILITY_ID` exclusions from tests; provider lists should compare directly without filtering it out.

- [ ] **Step 4: Remove the paging knob and new-write cap**

Delete `IRONCLAW_TOOL_RESULT_READ_MAX_BYTES`, `effective_tool_result_read_max_bytes`, and result-read-specific schema limits. Keep private legacy record read methods and their bounded internal request type only for Task 9 compatibility. Rename comments so they no longer advertise a model tool.

- [ ] **Step 5: Remove obsolete continuation metadata from new code paths**

New constructors must not set `next_offset` or `referenced_result_ref`. Retain serde-compatible fields only where historical wire decoding requires them. Mark them explicitly as historical compatibility and cover with a replay test.

- [ ] **Step 6: Run surface and architecture tests**

```bash
cargo test -p ironclaw_loop_host
SKIP_FRONTEND_BUILD=1 cargo test -p ironclaw_composition refreshing_capability_port
cargo test -p ironclaw_architecture_tests
```

Expected: PASS; no production provider definition names `result_read`.

- [ ] **Step 7: Commit the atomic cutover**

```bash
git add -A crates/loop/ironclaw_loop_host crates/app/ironclaw_composition crates/domains/ironclaw_threads
git commit -m "feat(loop): replace result reader with pinned coding artifact reads"
```

---

### Task 12: Prove the production path end to end

**Files:**
- Create: `tests/integration/reborn_coding_artifact_read_cutover.rs`
- Modify: integration harness support only for recording artifact observations and scripted model calls
- Modify: `tests/CLAUDE.md` only if the root integration binary count changes

- [ ] **Step 1: Write the failing large-result scenario**

Script the model to:

1. call a capability that emits more than the inline preview;
2. observe `artifact://1`;
3. call `read` with `artifact://1:3001-6000`;
4. finish with content present only in that range.

Assert at the seam:

```rust
assert_eq!(provider_tool_names.iter().filter(|name| *name == "read").count(), 1);
assert!(!provider_tool_names.iter().any(|name| name.contains("result_read")));
assert_eq!(recorded_calls[1].name, "read");
assert_eq!(recorded_calls[1].arguments["path"], "artifact://1:3001-6000");
assert!(final_reply.contains("sentinel-line-4500"));
```

Do not assert only run completion.

- [ ] **Step 2: Add parent/child sharing and isolation scenarios**

Drive a real spawned child. The parent creates an artifact, the child reads it, and the parent receives evidence from the child. Add negative cases for a different root run, tenant, owner user, agent/project scope, and incomplete artifact. Each negative case must return the same unavailable shape.

- [ ] **Step 3: Add replay and compaction scenarios**

Restart the harness after artifact finalization, reload context, compact it, and prove the artifact reference remains readable without embedding the full output. Seed one pre-cutover result record and prove the historical projection.

- [ ] **Step 4: Run the test red before the production flip is merged into the branch**

```bash
cargo test --test reborn_coding_artifact_read_cutover -- --nocapture
```

Expected before Tasks 8–11: FAIL on missing artifact continuation. Expected after Tasks 8–11: PASS.

- [ ] **Step 5: Run the complete integration binary**

```bash
cargo test --test reborn_coding_artifact_read_cutover
```

Expected: all large-output, lineage, isolation, restart, compaction, quota, and legacy cases PASS.

- [ ] **Step 6: Commit whole-path coverage**

```bash
git add tests/integration/reborn_coding_artifact_read_cutover.rs tests/integration/support tests/CLAUDE.md
git commit -m "test(loop): cover coding artifact read cutover"
```

---

### Task 13: Run cleanup, quality gates, and paired benchmark

**Files:**
- Modify: owning contracts/comments that still describe model-visible result paging
- Modify: `.env.example` only to remove `IRONCLAW_TOOL_RESULT_READ_MAX_BYTES` if documented there
- Modify: benchmark evidence in the PR body, not a new repository document

- [ ] **Step 1: Search for obsolete production vocabulary**

Use the repository search tool over `crates/`, `tests/`, `.env.example`, and owned contracts for:

```text
RESULT_READ_CAPABILITY_ID
builtin.result_read
builtin__result_read
effective_tool_result_read_max_bytes
IRONCLAW_TOOL_RESULT_READ_MAX_BYTES
use result_read
next_offset
```

Expected: no model-facing registration, prompt, policy, or new-write path remains. `next_offset` and `ResultReference` may remain only in explicitly named historical decoding tests and compatibility code.

- [ ] **Step 2: Run formatting**

```bash
cargo fmt
```

Expected: exit 0.

- [ ] **Step 3: Run narrow crate suites**

```bash
cargo test -p ironclaw_host_api
cargo test -p ironclaw_loop_contracts
cargo test -p ironclaw_threads
cargo test -p ironclaw_extension_support
cargo test -p ironclaw_agent_loop
cargo test -p ironclaw_loop_host
cargo test -p ironclaw_host_runtime
SKIP_FRONTEND_BUILD=1 cargo test -p ironclaw_composition
cargo test --test reborn_coding_contract_snapshot
cargo test --test reborn_coding_engines
cargo test --test reborn_coding_artifact_read_cutover
cargo test -p ironclaw_architecture_tests
```

Expected: all available tests PASS; PostgreSQL legs use the repository-standard Docker skip when unavailable.

- [ ] **Step 4: Run zero-warning lint for affected crates**

```bash
SKIP_FRONTEND_BUILD=1 cargo clippy -p ironclaw_host_api -p ironclaw_loop_contracts -p ironclaw_threads -p ironclaw_extension_support -p ironclaw_agent_loop -p ironclaw_loop_host -p ironclaw_host_runtime -p ironclaw_composition --all-targets --all-features -- -D warnings
```

Expected: exit 0 with no warnings.

- [ ] **Step 5: Check documentation placement**

```bash
python3 scripts/ci/docs_publication_boundary.py
```

Expected: `docs/ publication boundary: every page is published or fenced`.

- [ ] **Step 6: Smoke-test the real serve path**

Run the shipping binary with a test profile, submit a turn that produces a large shell or fixture output, observe `artifact://N`, call `read artifact://N:<range>`, and confirm the requested sentinel appears. Capture the exact command/profile and output in the PR test evidence.

- [ ] **Step 7: Run the paired benchmark**

Run claw-swe-bench-lite with the same DeepSeek V4 Flash model and task set on:

1. the stock/result-read baseline;
2. the artifact-read branch.

Record pass rate, result-recovery calls, tokens, persisted bytes, read failures, quota failures, and time to useful continuation. Do not compare against the checked-in Qwen baseline.

- [ ] **Step 8: Perform final code review**

Review the complete diff for scope authorization, artifact ID authority mistakes, accidental deletion, unbounded allocations, duplicate byte charging, error-cause loss, old provider names, and production old/new mixture. Resolve every Critical/High/Medium finding before shipping.

- [ ] **Step 9: Commit cleanup**

```bash
git add crates tests .env.example docs/internal
git commit -m "chore(loop): finalize artifact read cutover"
```

If the cleanup produced no tracked changes, do not create an empty commit.

---

## Definition of done

- Provider tools contain pinned coding `read` and no `result_read` spelling.
- Every truncated result exposes a numeric, readable `artifact://` URI.
- Parent and descendants share artifacts; unrelated scopes cannot infer existence.
- New output is constrained by resource budgets rather than 1 MiB/4 MiB result caps.
- Artifact persistence and selector reads do not load the complete artifact.
- Result evidence, replay, compaction, child completion, and activity cards retain internal `result_ref` behavior.
- Historical result records remain readable without event mutation.
- Incomplete and finalized artifact bytes are retained; no delete path exists.
- Pinned fixtures pass, including the one documented host-path omission.
- Four-backend conformance, production integration, architecture tests, lint, smoke test, and paired benchmark evidence are complete.

## Not covered

This plan does not implement archives, SQLite, documents, notebooks, images,
URLs, SSH, other internal URI schemes, remaining pinned coding tools, or a WebUI artifact
browser. It does not make PR #7491 merge-ready by itself; the PR remains the
benchmark arm until the full #7392 cutover is implemented and validated.
