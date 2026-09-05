# Pi loop worker behind the #7908 membrane

Status: implementation plan for a PR stacked on #7908.
Branch: `feat/pi-loop-worker`. Base: `feat/7903-native-loop-sandbox-spike`.

## Goal

Run Pi (`@earendil-works/pi-agent-core`) as an alternative loop worker inside the
persistent per-user sandbox, behind the same host membrane #7908 built for the
canonical Rust worker. The host keeps tenancy, secrets, authorization, model
gateway, transcript, checkpoints, and `LoopExit` validation. The worker holds
no key, no store, and no raw kernel handle.

Two things #7908 does not have and this PR adds:

1. **Worker selection by kind.** The sandboxed driver launches
   `ironclaw-loop-worker` (Rust) or `ironclaw-pi-worker` (Pi) per deployment
   setting. This PR also flips the defaults: the boot profile defaults to
   `hosted-single-tenant-volume-sandboxed` (Docker sandbox), the sandbox loop
   worker is enabled by default under it, and the worker kind defaults to
   `Pi`. Explicit overrides keep working: `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER=false`
   runs the loop in-process, `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER_KIND=rust`
   selects the #7908 content-blind Rust worker, and an explicit `local-dev`
   profile keeps the in-process loop.
2. **A profile-gated content-visible wire mode.** #7908's worker is
   content-blind: `LoopModelRequest.messages` are `LoopMessageRef`s, context
   bundles carry `safe_summary` only, and capability results return by ref. Pi
   cannot own context or compaction on placeholders. This PR adds
   `WorkerContentVisibility::{Blind, Resolved}` to the bootstrap and one new
   host call, `ResolveMessages`, that returns the host-resolved role and
   content for message refs the worker already holds. The Rust worker stays
   `Blind`. The Pi worker is launched `Resolved`. Secrets, authorization,
   other tenants, and stores remain host-side; the resolved transcript is the
   user's own thread inside the user's own container.

## Trust boundary delta (record for reviewers)

| Invariant                                        | #7908                                       | This PR                                                                                                                |
| ------------------------------------------------ | ------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| Worker holds credentials, DB, authorizer, Docker | No                                          | No                                                                                                                     |
| Model call crosses the host (`StreamModel`)      | Yes                                         | Yes, unchanged                                                                                                         |
| Every capability call crosses host authorization | Yes                                         | Yes, unchanged                                                                                                         |
| Worker sees transcript text                      | No                                          | Only when bootstrapped `Resolved`; only refs the host already issued to this run; denied otherwise with `PolicyDenied` |
| Worker can author prompt text                    | Via `BuildPrompt.inline_messages` (already) | Same mechanism; no new write path                                                                                      |
| Host validates `LoopExit`                        | Yes                                         | Yes                                                                                                                    |
| Wire is private, same-build                      | Yes                                         | Frame types are `pub`, documented, `wire_version` 2; still not a public API                                            |

## Wire changes (v2)

- `LoopWorkerBootstrap.content_visibility: WorkerContentVisibility` defaults
  to `Blind` when absent. This field default does not provide wire-version compatibility.
- `LoopWorkerBootstrap.wire_version = 2`. Workers reject a bootstrap whose
  version they do not know.
- `HostCall::ResolveMessages(ResolveMessagesRequest { messages: Vec<LoopModelMessage> })`
  → `Vec<WireResolvedModelMessage { role: String, content_ref: LoopMessageRef, content: String, tool_result: Option<WireResolvedToolResult> }>`.
  Host dispatch: denied unless the bootstrap was `Resolved`; refs must be ones
  the run may see (the same resolver the model gateway uses); budgeted with
  the other RPCs (`HostRpcState`).
- Frame framing unchanged: `u32` big-endian length + JSON, 1 MiB ceiling.
- Host-side port: `LoopMessageContentPort` in `ironclaw_loop_contracts`,
  implemented by `ThreadBackedLoopModelPort` (it already owns
  `resolve_model_messages`). `serve_loop_worker` takes it as an
  `Option<&dyn LoopMessageContentPort>`; `None` means blind. The
  `AgentLoopDriverHost` blanket trait is not widened.

## How the Pi worker uses the wire

```text
Bootstrap(Resolved, tool_definitions, run_context)
  -> BuildPrompt { mode: TextOnly, max_messages: default }   host prompt bundle (identity, skills, context refs)
  -> ResolveMessages(bundle.messages)                         Pi's initial AgentMessage[] with text
  -> Pi Agent { streamFn, tools, transformContext }
       transformContext:
         -> BuildPrompt -> ResolveMessages
         -> Compact on host-reported window eviction, then rebuild context
       streamFn:
         -> StreamModel with the exact host-built prompt grant
         -> map the response into Pi assistant/tool-call events
       tool.execute:
         -> InvokeCapability with the original host-issued candidate
         -> AppendCapabilityResultRef
         -> ResolveMessages with the returned transcript message reference
       checkpoints:
         -> StageCheckpointPayload -> Checkpoint before model, side effect,
            blocking, and terminal boundaries
       Resume:
         -> LoadCheckpointPayload -> replay pending calls with gate identity
       final reply:
         -> FinalizeAssistantMessage
  -> Outcome(LoopExit) ; wait OutcomeAck ; exit 0
```

Pi is configured with no built-in tools, no extensions, no project trust, no
package install, no provider client. Its only provider is the `streamFn` that
calls the host.

The host returns model chunks after the model RPC finishes. This worker does
not provide token-by-token wire streaming. Pi core supplies the agent loop;
host-managed compaction supplies durable summaries. No comparative loop
quality result is claimed without running the three-lane experiment.

## Work packages (one PR, parallel agents)

| Package                     | Owner                       | Files                                                                                                                                                                                                                                                                                            |
| --------------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| A. Wire v2 + content port   | Rust contracts/loop_host    | `crates/contracts/ironclaw_loop_contracts/src/host/model.rs` (port), `crates/loop/ironclaw_loop_host/src/remote_host/{protocol,server,client,tests}.rs`, `crates/loop/ironclaw_loop_host/src/lib.rs` (impl on `ThreadBackedLoopModelPort`), `docs/internal/reborn/contracts/loop-worker-wire.md` |
| B. Worker kind selection    | turn_runner/composition/cli | `crates/loop/ironclaw_turn_runner/src/sandboxed_planned_driver.rs`, `runtime.rs`, `crates/app/ironclaw_composition/src/{input,sandbox,runtime}.rs`, `crates/app/ironclaw_cli/src/runtime/mod.rs`, `.env.example`, READMEs                                                                        |
| C. Pi worker                | TypeScript, Bun             | `docker/sandbox/pi-worker/**`, `Dockerfile.sandbox-worker`                                                                                                                                                                                                                                       |
| D. Conformance + experiment | tests/docs                  | `crates/loop/ironclaw_turn_runner/tests/loop_worker_conformance.rs`, `tests/integration/reborn_sandbox_shell_turn.rs` (Pi lane), `docs/internal/reborn/2026-09-loop-worker-three-lane-experiment.md`                                                                                             |

## Acceptance

1. `cargo test -p ironclaw_loop_host remote_host` passes, including: blind
   bootstrap denies `ResolveMessages`; resolved bootstrap returns content for
   issued refs and rejects foreign refs; workers reject unsupported wire versions.
2. `cargo test -p ironclaw_turn_runner` passes; `LoopWorkerKind::Rust` launches
   `/usr/local/bin/ironclaw-loop-worker` with `Blind`; `LoopWorkerKind::Pi`
   launches `/usr/local/bin/ironclaw-pi-worker` with `Resolved`.
3. `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER_KIND` accepts `rust|pi`, defaults
   `pi`, invalid fails startup (same shape as the existing switch). Unset
   `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER` defaults the sandbox loop worker on
   under the sandboxed default profile.
4. Conformance harness drives both worker binaries through a scripted host over
   local stdio (no Docker): bootstrap, one model call, one capability call,
   one checkpoint, cancel, outcome ack. Pi lane skips when `bun` is absent.
5. `bun test` in `docker/sandbox/pi-worker` passes against a fake host.
6. `docker build -f Dockerfile.sandbox-worker` produces both
   `/usr/local/bin/ironclaw-loop-worker` and `/usr/local/bin/ironclaw-pi-worker`.
7. `cargo test -p ironclaw_architecture_tests` passes after ratchet updates.
8. Real-Docker integration test (`IRONCLAW_REQUIRE_DOCKER_TESTS=1`) has a Pi
   lane behind `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER_KIND=pi`.

## Rollback

Set `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER_KIND=rust` to return to the #7908
worker behavior; set `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER=false` (or boot the
explicit `local-dev` profile) to return to the in-process driver, where
`builtin.shell` still runs in the sandbox with sandboxed tools. An existing
`config.toml` that names its profile explicitly (for example the previous
default `local-dev`) is never silently rewritten. Pi runs use the
`pi_worker_session` checkpoint schema.
Do not switch worker kind while runs are paused: Rust and Pi checkpoint
payloads are not interchangeable. There is no new database migration.
The workspace path migration and pinned iron-proxy HTTP/HTTPS limitation
from #7908 remain unchanged.
