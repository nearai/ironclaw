# Coding Artifact Read Cutover Design

**Date:** 2026-08-11  
**Status:** Approved design; implementation not started  
**Issue:** #7392  
**Target:** Follow-up cutover after PR #7491 benchmark evidence

## Summary

Remove the model-visible `builtin.result_read` capability and recover large tool
outputs through the pinned coding `read` surface instead.

New tool results have one durable representation: a run-tree-scoped artifact.
Small results are delivered inline. When an inline response is truncated, it
includes `Full output: artifact://<numeric-id>`. The model retrieves only the
needed portion with an ordinary pinned coding read selector, for example:

```text
read artifact://7:3001-6000
```

Artifact IDs are shared by a root run and every subagent descended from that
run, matching oh-my-pi's shared parent/subagent artifact manager. Stored bytes are
immutable and retained. Access is authorized by the caller's resource scope and
the durable spawn-tree root, not by possession of the numeric ID.

The implementation removes the current 24 KiB paging loop and the fixed 1 MiB
first-party / 4 MiB durable-result ceilings. Artifact writes are incremental and
stop at the existing run and tenant resource budgets instead of a new global
per-artifact limit.

## Approved decisions

1. **The pinned coding `read` tool is the only model-visible result recovery tool.**
   `builtin.result_read` and provider name `builtin__result_read` are removed in
   the production cutover.
2. **New large outputs are durable artifacts, not virtual wrappers over the
   existing result-record limit.** This is the full pinned upstream behavior, not a
   surface-only rename.
3. **A root run and its subagent tree share one artifact namespace.** The
   existing `spawn_tree_root_run_id` identifies that namespace; a root run uses
   its own run ID.
4. **Artifact size is quota-based.** No replacement 4 MiB or 64 MiB hard cap is
   introduced. Accepted bytes count against existing resource ceilings before
   they are committed.
5. **LLM data is never deleted.** Finalized and incomplete artifact data is
   retained and filterable. Incomplete artifacts are not model-readable.
6. **Historical events are not rewritten.** Existing tool-result records remain
   readable through a private legacy artifact adapter, but no legacy paging tool
   remains in the model surface.
7. **The production switch is atomic.** Artifact creation, `artifact://` reads,
   migration behavior, policy, prompts, and removal of `result_read` ship in one
   cutover. Test-only factories may exercise the new arm before that switch.

## Current state

### IronClaw result hydration

`crates/app/ironclaw_composition/src/runtime/capability_host.rs` currently:

- serializes every capability result into one `Vec<u8>`;
- stores the full payload through
  `SessionThreadService::put_tool_result_record`;
- embeds the first 24 KiB as a model-visible preview;
- exposes `result_ref`, `total_bytes`, and `next_offset` for continuation;
- rejects durable payloads above 4 MiB.

`crates/loop/ironclaw_loop_host/src/result_read.rs` registers the synthetic
`builtin.result_read` capability. It validates that the requested `result_ref`
appears in the current finalized thread history, reads a byte slice through
`SessionThreadService::read_tool_result_record`, and returns another continuation
observation.

`crates/domains/ironclaw_threads/src/contract.rs` fixes the normal chunk at
24 KiB, with an environment override capped at 64 KiB. The filesystem-backed
thread service already stores result payloads as opaque files, but that storage
is private to the result-record API and is subject to the 4 MiB validation cap.

First-party handlers add an earlier limit:
`FIRST_PARTY_MAX_OUTPUT_BYTES` is 1 MiB. The current pinned coding handler therefore
cannot produce the larger artifact before the generic result writer sees it.

### Pinned upstream behavior

At pinned commit `08819b279cf02ae2545e69dad7111ab48d91d35e`, oh-my-pi has no
`result_read` tool.

- `session/artifacts.ts` stores truncated outputs in session-scoped files named
  `<numeric-id>.<tool-type>.log`.
- Parent runs and subagents share one artifact manager and ID space.
- `internal-urls/artifact-protocol.ts` resolves `artifact://<numeric-id>` to the
  current session's backing file.
- Pagination belongs to `read`; large artifacts must use selectors such as
  `artifact://3:1-3000`.
- Bash, eval, and asynchronous follow-up paths return an `artifact://` footer
  when full output is saved.

Our pinned `read` prompt already advertises this contract, but
`crates/extensions/ironclaw_extension_support/src/coding/pinned/read.rs` currently
implements files and directories only. Shipping that prompt without the URI
resolver is an incomplete contract.

## Goals

- Remove `builtin.result_read` from model disclosure and runtime registration.
- Make `artifact://<numeric-id>` a complete pinned-coding read target, including
  selectors, errors, and output formatting.
- Store each new tool result once and retain it durably.
- Let parent and child runs exchange large outputs without copying them.
- Replace fixed result-size ceilings with existing resource accounting.
- Avoid loading an entire artifact when the model requests a bounded selector.
- Preserve result evidence, transcript replay, output digests, and UI activity
  summaries.
- Preserve historical result bytes without rewriting historical messages.
- Keep authorization, approvals, filesystem mediation, and redaction at their
  existing boundaries.

## Non-goals

- Exposing raw filesystem paths for artifact storage.
- Letting artifact IDs act as bearer credentials.
- Deleting or compacting old LLM output.
- Changing user workspace files into artifacts.
- Replacing internal `result_ref` completion evidence. The model paging tool is
  removed; the internal identity remains.
- Adding an artifact browser or download endpoint to WebUI.
- Adding a second database driver or storage backend.
- Allowing unbounded output. Run and tenant resource budgets remain mandatory.
- Shipping other internal URI schemes, URLs, SSH, archives, or documents as
  part of this cutover unless required by `artifact://` parsing reuse.

## Options considered

### 1. Canonical durable artifacts

Write new tool output into a durable artifact namespace and disclose a numeric
`artifact://` reference when the inline preview is incomplete. The pinned coding `read`
tool is the only model-visible reader.

This is selected. It matches upstream behavior, removes the extra tool call
shape, supports parent/subagent handoffs, and permits quota-sized outputs.

### 2. Dual-write result records and artifacts

Keep writing the current result record and also create a new artifact copy.
Remove `result_read` after both are available.

This is rejected as the steady state because it doubles large-output storage,
creates two authorities for the same bytes, and complicates repair. The
implementation may use test-only dual wiring to compare behavior, but
production new writes have one canonical artifact representation at cutover.

### 3. Virtual `artifact://` over existing result records

Keep the current result-record store and map `artifact://<id>` reads onto
`read_tool_result_record`.

This is rejected for new data because it preserves the 4 MiB durable cap and
whole-payload materialization. A private form of this adapter remains only for
historical records created before cutover.

## Architecture

### Keep artifact vocabulary neutral and execution mediated

Neutral identifiers and host-service contracts belong in
`ironclaw_host_api`:

```text
ArtifactId                 numeric ID within one artifact namespace
ArtifactNamespaceId        durable root-run-tree identity
ArtifactRef                artifact://<ArtifactId>
ArtifactReadRequest        namespace + selector/range + caller scope
ArtifactReadChunk          bounded bytes + line/range metadata
ArtifactWriteMetadata      producer, content type, expected budget class
ArtifactWriteHandle        opaque in-progress artifact identity
```

The model-facing read engine must not import thread or turn persistence. A
narrow artifact host port provides allocation, incremental writes, finalization,
and bounded reads. This trait is justified dependency inversion: its concrete
implementation depends on run/thread persistence and `RootFilesystem`, while
the capability engine is lower-level and must not.

`ironclaw_composition` only wires the concrete implementation into invocation
services and the capability-result writer. It owns no artifact policy.

### Reuse the existing spawn-tree root

The durable artifact namespace is:

```text
run.spawn_tree_root_run_id.unwrap_or(run.run_id)
```

The canonical run context carries the resolved namespace so every tool call can
use it without re-querying the turn store. Child-run submission copies the
parent's namespace. Recovery reconstructs it from the existing durable
`spawn_tree_root_run_id` field.

The namespace is always combined with the resource scope used by the run:

```text
tenant + owner user + agent + project + artifact namespace
```

A caller may read an artifact only when its current run resolves to the same
namespace and resource scope. Numeric IDs are deliberately guessable and grant
no authority.

### Allocate numeric IDs without process-local state

Artifact IDs must remain unique across concurrent tools, processes, restarts,
and resumed runs. Allocation uses a bounded CAS update on one namespace counter
stored through `RootFilesystem`, following the repository's shared CAS helper.
A failed write may leave an unused number. IDs are monotonic, not contiguous.

No process-local mutex is held across backend I/O. No backend-specific branch or
new database driver is introduced.

### Store artifacts as immutable chunk sets

One artifact has an immutable metadata record and fixed-size byte chunks under
the artifact mount. A representative virtual layout is:

```text
/artifacts/<scope>/<namespace>/<artifact-id>/metadata
/artifacts/<scope>/<namespace>/<artifact-id>/chunks/<chunk-index>
```

The exact encoded scope segments use existing strong IDs and path constructors;
raw display strings never form paths.

Metadata records:

- numeric artifact ID and namespace;
- producer capability and internal `result_ref`;
- content type and text encoding;
- total bytes and total lines when textual;
- content digest;
- ordered chunk descriptors, including byte and line boundaries;
- creation and finalization timestamps;
- finalized or incomplete state;
- optional legacy backing reference for pre-cutover records.

The writer creates incomplete metadata first, appends bounded chunks, and
finalizes metadata with CAS only after all chunks and the digest are durable.
Readers ignore incomplete artifacts. A crash may leave incomplete metadata and
chunks; they remain retained and filterable rather than being deleted.

Chunk descriptors let line selectors fetch only intersecting chunks. Raw byte
recovery remains bounded. A read never concatenates the whole artifact merely
to return one selector.

### Account output while it is produced

A quota-aware artifact sink is available to high-output capability adapters.
For each chunk it:

1. reserves or charges output bytes through the existing resource governor;
2. rejects the chunk before persistence if the budget would be exceeded;
3. writes the accepted chunk;
4. updates byte count, line index, and digest state;
5. finalizes only after the producer completes successfully.

Adapters that already return a bounded `serde_json::Value` remain valid. The
capability-result writer serializes those results into the same artifact sink
when they exceed the inline-preview threshold. High-volume first-party tools,
process output, eval, and later sidecars use the streaming sink directly and
return only their bounded preview plus artifact metadata.

The old 1 MiB first-party and 4 MiB durable-result limits are removed from the
artifact path. They are not replaced with a larger magic number. Artifact
persistence adds no full-output copy: streaming-capable producers must write
incrementally, while protocol adapters that inherently materialize one bounded
message retain their transport-specific receive limit.

### Keep one durable result and one internal evidence identity

Every completed capability still produces a `result_ref`. It remains the
internal identity used by completion evidence, transcript rows, replay,
digests, dependent-run resolution, and activity cards.

The artifact is the canonical durable content for new results. The transcript
envelope contains bounded model-visible content plus, when truncated:

```json
{
  "artifact_ref": "artifact://7",
  "total_bytes": 184320,
  "preview": "...bounded first portion..."
}
```

`next_offset` and model-facing `result_ref` continuation metadata are no longer
emitted. A complete inline result need not advertise its artifact URI, although
its full bytes remain durably retained.

### Route `artifact://` through pinned coding read

The pinned coding read flow becomes:

```text
parse path and selector
  -> internal URI detection
  -> artifact URI validation
  -> pre-scoped artifact host read
  -> pinned internal-resource formatting
  -> ordinary model-visible result boundary
```

`artifact://` parsing matches the pinned behavior:

- the host must be a numeric ID;
- missing or nonnumeric IDs return the pinned model-correctable error;
- selectors use the same parser as local reads;
- reads are immutable and safe for parallel execution;
- oversized unsliced reads instruct the model to use selectors.

One hosted-security deviation is explicit and fixture-pinned: upstream's
oversized-artifact error prints the backing host file path for search/copy
workflows. IronClaw must not expose a host path, so it keeps the same selector
guidance but names only the `artifact://` URI. This is the sole intentional
artifact-protocol output difference in this design.

Local files continue through `RootFilesystem`, mount authorization, Hashline
headers, and snapshot registration. Artifact reads do not create Hashline edit
snapshots because artifacts are immutable and not editable workspace files.

The read capability remains subject to normal capability authorization and
resource accounting. The artifact host port additionally enforces run-tree and
resource-scope membership; workspace mount possession alone is insufficient.

### Preserve redaction and untrusted-output handling

Artifact storage retains the exact tool output bytes. Redaction is a delivery
concern and never mutates stored LLM data.

Every preview and every `read artifact://` response crosses the existing
model-visible scrub and secret/control-marker validation before entering model
context. Artifact output remains `UntrustedToolOutput`. A tool cannot bypass
that boundary by referring to its artifact through `grep`, bash expansion, or a
future sidecar; every model-visible derived result crosses the same scrub.

Credentials stay host-side. No secret mount or raw host path is exposed through
artifact metadata.

## End-to-end flows

### Small output

```text
tool completes
  -> artifact sink finalizes canonical bytes
  -> complete bounded preview enters transcript
  -> result_ref remains internal evidence
  -> model receives no continuation instruction
```

### Large output

```text
tool streams chunks
  -> resource governor accepts and accounts each chunk
  -> artifact finalizes
  -> transcript receives preview + artifact://N + total bytes
  -> model calls read artifact://N:<selector>
  -> artifact port authorizes current run-tree scope
  -> only matching chunks are read and formatted
```

### Parent/subagent handoff

```text
parent creates artifact://N under tree root R
  -> child run inherits artifact namespace R
  -> child read resolves N under R and the shared resource scope
  -> unrelated run tree guesses N
  -> authorization fails without revealing whether N exists
```

### Historical result

```text
replay encounters old result_ref without artifact metadata
  -> private legacy adapter validates the historical finalized thread reference
  -> adapter allocates durable numeric artifact metadata in the current tree
  -> backing source points to the retained old tool-result record
  -> model projection exposes artifact://N
  -> pinned coding read uses the artifact port
```

This does not rewrite the historical message or copy its bytes. The adapter is
not a capability and is never disclosed to the model.

### Quota exhaustion

```text
producer offers next chunk
  -> resource reservation fails
  -> chunk is not committed
  -> artifact remains incomplete and undisclosed
  -> capability returns model-visible budget exhaustion
  -> retained incomplete bytes remain available to operators, not the model
```

## Atomic cutover

### Build and prove the new arm without production mixing

Before production registration changes, test-only wiring provides:

- the artifact store over in-memory, local, libSQL, and PostgreSQL filesystem
  backends;
- a run-tree-scoped artifact port;
- artifact-producing capability result writes;
- pinned coding `read artifact://`;
- historical-result projection;
- no synthetic `result_read` capability.

Production continues to expose the old arm until all tests and benchmark gates
pass. The model never sees both continuation tools in one production surface.

### Flip all production contracts together

The cutover changes these surfaces in one release:

1. capability result writes finalize artifacts;
2. truncated observations emit `artifact://<id>`;
3. pinned coding `read` resolves artifact URIs;
4. parent and child run contexts carry the shared artifact namespace;
5. provider disclosure removes `builtin__result_read`;
6. refreshing capability-port construction stops registering the synthetic
   result reader;
7. result-read schema, parser, environment knob, prompts, exports, test support,
   and production tests are removed;
8. legacy tool-result storage remains private and read-only for historical
   compatibility.

The migration does not rename historical `result_ref` values or delete result
records.

### Roll back safely

Rollback deploys the previous binary. During the compatibility window, new
artifact metadata retains the producing `result_ref`, and the artifact store can
project finalized artifact bytes through the old private result-record contract
for rollback reads. This rollback projection is deployment compatibility, not a
model-visible alias.

After the rollback window closes, remove only the projection writer. Retain all
artifact and historical result bytes permanently.

## Error behavior

- Invalid artifact URI or selector: pinned coding model-correctable input error.
- Artifact absent, incomplete, or outside the run tree: one indistinguishable
  unavailable error; do not reveal existence across scopes.
- Resource budget exhausted: model-visible budget failure with accepted byte
  evidence; no success claim.
- Storage unavailable during write: host error that ends or retries the run
  under existing disposition policy; never disclose an unfinalized ID.
- Storage unavailable during read: retryable host error, distinct from an
  unavailable artifact.
- Digest or chunk metadata mismatch: fail closed as integrity failure and emit
  operator diagnostics without raw content.
- Non-text artifact without `:raw`: pinned binary-content notice.
- Unsupported legacy backing: model-visible unavailable result plus operator
  diagnostic; historical bytes remain untouched.

## Verification strategy

### Contract fixtures

Extend `tests/fixtures/pinned_coding_contract/` with pinned artifact URI cases:

- missing, nonnumeric, absent, and valid IDs;
- single and multi-range selectors;
- `:raw` selector ordering;
- oversized unsliced artifact notice;
- binary content;
- exact error and output formatting.

Differential tests compare IronClaw output with pinned upstream at commit
`08819b279cf02ae2545e69dad7111ab48d91d35e`. The hosted-security fixture
separately pins the single approved deviation: omit the upstream backing host
path from the oversized-artifact error.

### Storage conformance

One shared suite runs against in-memory, local, libSQL, and PostgreSQL backends
and proves:

- concurrent allocation produces unique monotonic IDs;
- chunk and metadata writes survive restart;
- finalization is atomic from the reader's perspective;
- range reads fetch only intersecting chunks;
- quota rejection commits no over-budget chunk;
- incomplete artifacts are retained but unreadable by the model;
- digest mismatch fails closed;
- no artifact delete path exists.

### Caller-level integration

Production-shaped integration tests drive the canonical agent loop and assert:

- a large tool output returns `artifact://N`, not `next_offset` or a model-facing
  `result_ref`;
- the model continues with `read`, and receives the requested lines;
- `builtin__result_read` is absent from provider tools;
- parent and child runs can read one another's artifacts;
- an unrelated run tree and another tenant/user/project cannot;
- an old result record becomes readable through the legacy projection;
- replay and compaction preserve the artifact reference without embedding full
  output;
- resource accounting includes persisted artifact bytes;
- no successful side effect is reported before artifact finalization.

### Regression and architecture checks

Run the narrow crate suites for host API, filesystem, threads, loop contracts,
loop host, host runtime, composition, and agent loop. Run
`cargo test -p ironclaw_architecture_tests` for the new host-service contract and
dependency wiring. Search the production tree for `RESULT_READ_CAPABILITY_ID`,
`builtin.result_read`, `builtin__result_read`, and the removed environment knob;
only historical fixture data may remain.

### Benchmark gate

Run the same claw-swe-bench-lite model and task set on both arms. Record:

- pass rate;
- calls spent recovering large output;
- input/output tokens;
- artifact read failures;
- total persisted artifact bytes;
- time to first useful continuation;
- run failures from output limits or quota exhaustion.

The cutover must eliminate `result_read` calls, produce successful `read
artifact://` recovery, and show no regression in task success or scope
isolation. Performance evidence is reported; no fixed percentage improvement is
claimed before the paired run exists.

## Acceptance criteria

- The provider tool surface contains pinned coding `read` and no `result_read` tool.
- Every incomplete inline result includes a readable `artifact://<numeric-id>`.
- Pinned coding `read` matches pinned artifact selector, output, and error fixtures except
  for the documented omission of a backing host path.
- New artifact size is constrained by resource budgets, not 1 MiB/4 MiB global
  result caps.
- Artifact persistence and selector reads are bounded-memory operations;
  streaming-capable producers do not buffer full artifacts.
- Root and descendant runs share artifacts; unrelated runs and scopes cannot
  infer or read them.
- Stored bytes are immutable, retained, and digest-verified.
- Historical result records remain readable without event rewrites.
- Transcript replay, compaction, dependent runs, and activity cards retain their
  existing internal `result_ref` evidence.
- Production never exposes old and new paging tools together.
- The four filesystem backends pass one conformance suite.
- Architecture tests and production-shaped integration tests pass.

## What this design does not cover

This design does not implement the other pinned `read` targets (archives,
SQLite, documents, notebooks, images, URLs, SSH, or non-artifact internal URI
schemes), the remaining pinned coding tools, or a WebUI artifact browser. Those remain
separate #7392 slices. It also does not authorize merging PR #7491 as a final
cutover; that PR remains the benchmark arm until this artifact design and the
remaining pinned coding surface are implemented and verified.
