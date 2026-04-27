# Engine v2 Per-Project Sandbox

## Context

IronClaw engine v2 (Phase 8 of `docs/plans/2026-03-20-engine-v2-architecture.md`) needs sandboxed execution for tools that touch the filesystem or run shell commands. Today the engine bridge (`src/bridge/effect_adapter.rs:1007`) executes every tool directly on the host with no isolation, even though the underlying engine has a `CapabilityLease` + `EffectType` model that was designed to enable this.

The dominant risk is **accidental filesystem damage from LLM mistakes** — `rm -rf`, overwriting files, traversing into `~/.ssh` or `.env`, runaway shell processes. The engine's existing `base_dir` path validation in `src/tools/builtin/path_utils.rs:78` is the only line of defense and it's optional.

The intended outcome is a **persistent per-project workspace container** that:
- Bind-mounts the project's user-facing files at `/project/` so they remain on the host (visible, backupable)
- Owns its own writable filesystem layer where `cargo`, `pip`, `npm`, `apt`, downloaded artifacts, build caches and toolchains accumulate over time — turning the container into the project's "workspace computer"
- Hosts a thin tool-execution daemon that runs `file_*`, `shell`, `list_dir`, `apply_patch` against the mounted project, while everything else (memory, network, LLM, secrets, orchestrator, Monty) stays on the host
- Survives across IronClaw restarts (started/stopped, never `--rm` except on explicit user action) so the accumulated environment persists

## Cross-reference: Issue nearai/ironclaw#1894 (Unified Workspace VFS)

Issue #1894 proposes a unified mount-table abstraction where the agent sees one filesystem and paths route through `MountBackend` variants (`Filesystem`, `Database`, future `S3`, `GitRemote`, etc.). The sandbox in this plan is **the storage backend for the `/project/` mount when isolation is enabled**, not a separate feature toggle. Concretely:

- `/project/` (the agent-facing path prefix) is owned by a `MountBackend`
- When sandbox is off: `MountBackend::Filesystem { root: ~/.ironclaw/projects/<id>/ }` — passthrough to host fs
- When sandbox is on: `MountBackend::ContainerizedFilesystem { project_id }` — JSON-RPC to per-project daemon
- The agent calls `file_read("/project/foo.txt")` either way — the backend swap is invisible to the agent and to the orchestrator

This plan introduces a **minimal subset of #1894's Phase 1** — just enough mount-backend machinery to make the sandbox a backend rather than a special case. The full mount table, `/memory/` routing, search unification, and tool renames from #1894 are independent and can land separately. Both efforts converge cleanly because the sandbox backend is designed to drop into #1894's full mount table without changes.

What this plan does NOT do that #1894 does:
- Does NOT rename `file_read`/`file_write`/`list_dir`/`apply_patch` to `read_file`/`write_file`/`list` (#1894 Phase 3)
- Does NOT route `memory_*` through the workspace mount table (#1894 Phase 2)
- Does NOT add `search`, `expand_mount`, or dynamic mount expansion (#1894 Phase 5)
- Does NOT introduce `materialize()` / `sync_back()` for db-backed `/project/` mounts — the sandbox uses a real filesystem (bind mount) so no copy in/out
- Does NOT remove the orchestrator/container tool registration split — that disappears naturally because the daemon registers the same `Tool` trait impls as the host, not a parallel "container-only" tool surface

What this plan DOES that #1894 will reuse:
- `MountBackend` trait + minimal `Workspace::resolve_mount(path) -> &dyn MountBackend` API
- `FilesystemBackend` (passthrough) and `ContainerizedFilesystemBackend` (JSON-RPC)
- `/project/` as the canonical path prefix for the user's project files
- The principle that backends swap transparently under unchanged tool surfaces

## Locked design decisions

1. **Granularity**: One container per `Project` (`crates/ironclaw_engine/src/types/project.rs`). All threads in the same project — including sub-threads spawned by `rlm_query()` which inherit `project_id` at `crates/ironclaw_engine/src/runtime/manager.rs:505` — share the container.
2. **Persistence**: Named container. `docker create` once, `docker start`/`stop` lifecycle, removed only on project deletion or explicit user reset. Container's own writable layer holds installed dependencies and accumulates over time.
3. **Mount**: `~/.ironclaw/projects/<user_id>/<project_id>/` (host) → `/project/` (container) bind mount, read-write. Project's user files live here.
4. **Daemon model**: Container PID 1 is a minimal init (`tini`). Tool-execution daemon is spawned via `docker exec -i sandbox_daemon` per IronClaw session. Daemon talks JSON-RPC over stdin/stdout to the host.
5. **What's inside**: Only `file_read`, `file_write`, `list_dir`, `apply_patch`, `shell`. Each instantiated with `base_dir=/project/` so existing path validation enforces sandboxing. cwd defaults to `/project/`.
6. **What's on host**: Everything else — `memory_*` (db-backed), `web_fetch`, `http`, `time`, `json`, `message`, `plan`, `system`, `tool_info`, secrets, all WASM tools, all MCP tools, all skill/extension management, the entire engine v2 orchestrator + Monty.
7. **IPC**: NDJSON over stdin/stdout. One JSON request per line, one response per line. Request/response correlated by `id`. Container uses default Docker bridge networking so `git clone`, `cargo build`, `pip install`, etc. work. Outbound network restriction via the existing proxy (`src/sandbox/proxy/`) is a follow-up.
8. **Secret crossing**: Orchestrator may pass secrets as tool parameters (e.g. `shell_exec(env={"FOO": secret})`). The container is per-project and not externally reachable, so this is documented as caller responsibility, not blocked.
9. **Rollout gate**: Opt-in via `SANDBOX_ENABLED=true` env var. When set, the `/project/` mount uses `ContainerizedFilesystemBackend`. When unset, `FilesystemBackend`. Default unset until stable. The env var disappears when #1894's full mount table lands and configuration moves into deployment-mode mount tables.

## Architecture

### Storage layers per project

```
Host filesystem                            Container filesystem
~/.ironclaw/                               (anonymous writable layer, persists across stop/start)
  projects/                                /
    <project_id>/        <─── bind ──>    /project/        (user's files, shared with host)
      README.md                            /root/.cargo/    (cargo install state)
      src/                                 /root/.npm/      (npm cache)
      ...                                  /root/.local/    (pip user packages)
                                           /usr/local/bin/  (system-installed binaries)
                                           /var/cache/apt/  (apt packages)
                                           ...
```

The user sees their files in `~/.ironclaw/projects/<user_id>/<project_id>/`. The container sees the same files at `/project/`. Anything the agent installs via `shell_exec` lives in the container's writable layer and persists for the project's lifetime.

### Component layout

```
Host process (IronClaw)                                Container (per project)
─────────────────────                                  ──────────────────────
Engine v2                                              PID 1: tini (idle)
  └─ Python orchestrator (Monty)
       └─ tool dispatch                                PID N: sandbox_daemon
            └─ EffectBridgeAdapter                       ├─ stdin: NDJSON requests
                 └─ tool.execute(params)                  ├─ stdout: NDJSON responses
                      │                                   └─ ToolRegistry (base_dir=/project/)
                      └─ Workspace::resolve_mount("/project/foo.txt")
                           │                                    ├─ file_read
                           ├─ ContainerizedFilesystemBackend    ├─ file_write
                           │    └─ docker exec -i daemon ──┐    ├─ list_dir
                           │                                │    ├─ apply_patch
                           └─ FilesystemBackend             │    └─ shell (cwd=/project/)
                                └─ direct OS read           │
                                                            ▼
                                                    JSON-RPC dispatch
```

Key point: tool implementations don't change. They read paths from their input, normalize via `base_dir`, and call OS APIs. What changes is **where the OS APIs run** — host or container. The `Workspace` resolves the mount and routes the call.

For v1 of this plan, the routing is narrowly scoped: only the five sandboxed tools (`file_read`, `file_write`, `list_dir`, `apply_patch`, `shell`) consult the workspace mount table when their input contains a `/project/` path. Other tools and other paths bypass the mount table. This keeps the change small while establishing the abstraction that #1894 will fully generalize.

## Mount backend abstraction (minimal subset of #1894)

```rust
// crates/ironclaw_engine/src/workspace/mount.rs (new)

pub trait MountBackend: Send + Sync {
    async fn read(&self, rel_path: &Path) -> Result<Vec<u8>, MountError>;
    async fn write(&self, rel_path: &Path, content: &[u8]) -> Result<(), MountError>;
    async fn list(&self, rel_path: &Path, depth: usize) -> Result<Vec<DirEntry>, MountError>;
    async fn patch(&self, rel_path: &Path, diff: &str) -> Result<(), MountError>;
    async fn shell(&self, command: &str, env: HashMap<String, String>, cwd: Option<&Path>) -> Result<ShellOutput, MountError>;
}

pub struct WorkspaceMounts {
    mounts: Vec<(String, Arc<dyn MountBackend>)>,  // (prefix, backend), longest-prefix-first
}

impl WorkspaceMounts {
    pub fn resolve(&self, path: &str) -> Option<(Arc<dyn MountBackend>, PathBuf)> {
        // longest-prefix match → return (backend, relative path)
    }
}
```

Two backend implementations:

```rust
// FilesystemBackend — passthrough to host filesystem
pub struct FilesystemBackend {
    root: PathBuf,  // e.g. ~/.ironclaw/projects/<id>/
}

// ContainerizedFilesystemBackend — JSON-RPC to per-project daemon
pub struct ContainerizedFilesystemBackend {
    project_id: ProjectId,
    sandbox: Arc<ProjectSandboxManager>,  // shared across all backends in this process
}
```

## JSON-RPC protocol (NDJSON over daemon stdin/stdout)

**Request** (host → daemon, one per line):
```json
{"id":"<uuid>","method":"execute_tool","params":{"name":"shell","input":{"command":"cargo build"},"context":{"thread_id":"<uuid>","step_id":"<uuid>","user_id":"<id>","timeout_ms":300000}}}
```

**Success response** (daemon → host):
```json
{"id":"<uuid>","result":{"output":{"stdout":"...","stderr":"...","exit_code":0},"metadata":{"duration_ms":1234}}}
```

**Error response**:
```json
{"id":"<uuid>","error":{"code":"tool_error|sandbox_error|timeout|unknown_tool","message":"...","details":{}}}
```

**Other methods (v1)**:
- `{"method":"health"}` → `{"result":{"status":"ok","tools":["file_read","file_write","list_dir","apply_patch","shell"]}}`
- `{"method":"shutdown"}` → daemon flushes in-flight calls then exits

**Concurrency (v1)**: Daemon serializes — processes one request at a time. Multiple in-flight requests from different threads queue. Multiplexing via in-flight `id` map is a future optimization. Rationale: fs/shell side-effects are ordered, and serializing avoids races against the same `/project/` files from concurrent threads.

## Lifecycle

### Container lifecycle states

```
(none) ──create──> Stopped ──start──> Running ──stop──> Stopped
                      ▲                  │                 │
                      └──────────────────┘                 │
                                                           ▼
                                                     (project deleted)
                                                          remove
```

- **create**: First sandboxed tool call from any thread in the project, OR explicit `project init`. Builds `~/.ironclaw/projects/<user_id>/<project_id>/` if missing. Runs `docker create --name ironclaw-sandbox-<project_id> --mount type=bind,source=<host>,target=/project ironclaw/sandbox:latest`. Cold path, ~1–2s.
- **start**: Container exists but stopped. `docker start`. Warm path, ~200–500ms. Then `docker exec -i ironclaw-sandbox-<project_id> /usr/local/bin/sandbox_daemon` to attach a daemon process.
- **stop**: Idle timeout (default 30 min after last tool call) or engine shutdown. Daemon receives `shutdown` method, flushes, exits cleanly. `docker stop` brings the container down. State on disk persists.
- **remove**: Only on explicit project deletion or user-invoked "reset environment". `docker rm` deletes the writable layer. Bind-mounted host folder is left untouched (user files are not the sandbox's to delete).

### Per-thread integration

Threads do not own containers. They use the project's container if it exists, lazily create it on first sandboxed tool call. No per-thread cleanup. When a thread completes/fails, the container continues running for other threads. Suspended/Waiting threads do not affect the container — but they also do not extend the idle timer.

### Crash recovery

- **Container crash mid-call**: Detect via daemon stdin/stdout pipe close. Mark in-flight call as failed with `sandbox_error`. On next call, restart the container (it's stopped now) and spawn a fresh daemon.
- **Daemon crash, container alive**: Same — restart daemon via `docker exec -i`. Container's accumulated state is preserved.
- **Host restart**: On engine startup, scan for `ironclaw-sandbox-*` containers in stopped state. Leave them. Re-attach lazily on first tool call. Stale containers from deleted projects can be cleaned via a future `ironclaw doctor` subcommand.

## Files and modules

### New code

- `crates/ironclaw_engine/src/workspace/mount.rs` — `MountBackend` trait, `WorkspaceMounts`, `MountError`, `DirEntry`, `ShellOutput`. Minimal subset of #1894's mount-table types.
- `crates/ironclaw_engine/src/types/project.rs` — add `workspace_path: Option<PathBuf>` field to `Project`. Accessor returns `~/.ironclaw/projects/<user_id>/<project_id>/` if unset.
- `src/bridge/sandbox/mod.rs` — new submodule of bridge. Public types: `ProjectSandboxManager`, `ContainerHandle`, `SandboxConfig`.
- `src/bridge/sandbox/manager.rs` — `ProjectSandboxManager` owns `Arc<RwLock<HashMap<ProjectId, ContainerHandle>>>` plus an idle-timeout background task. Provides `dispatch(project_id, request) -> Response`.
- `src/bridge/sandbox/handle.rs` — `ContainerHandle` wraps the `bollard::Container` + the active `docker exec` stream. Owns the request/response correlation map (`HashMap<RequestId, oneshot::Sender<Response>>`). Background reader task parses NDJSON from daemon stdout, looks up `id`, sends to the waiting oneshot.
- `src/bridge/sandbox/protocol.rs` — `Request`, `Response`, `Error` Serde types. `SANDBOX_TOOL_NAMES: &[&str] = &["file_read","file_write","list_dir","apply_patch","shell"]` constant.
- `src/bridge/sandbox/lifecycle.rs` — Docker-side container create/start/stop/remove. Reuses `connect_docker()` from `src/sandbox/container.rs` (~line 647). Builds project folder if missing. Image-pull check on first use.
- `src/bridge/sandbox/backend.rs` — `ContainerizedFilesystemBackend` implements `MountBackend` by translating each method into a JSON-RPC call via `ProjectSandboxManager`.
- `src/bridge/workspace_filesystem.rs` (new, small) — `FilesystemBackend` implements `MountBackend` as direct OS passthrough scoped to a `root: PathBuf`.
- `src/bin/sandbox_daemon.rs` — new binary. Reads NDJSON from stdin, dispatches to a local `ToolRegistry` populated with only the five sandboxed tools (each constructed with `base_dir=/project/`, shell `working_dir=/project/`). Writes responses to stdout. No network. No state. Exits on `shutdown` or stdin EOF.
- `crates/Dockerfile.sandbox` — minimal image. Base: `debian:stable-slim` or `ubuntu:24.04`. Installs `tini`, `git`, `curl`, common build tools (`build-essential`, `pkg-config`). Copies the `sandbox_daemon` binary to `/usr/local/bin/`. CMD: `["tini","--","tail","-f","/dev/null"]`.

### Modified code

- `src/bridge/effect_adapter.rs:1007` — `execute_action()`/`execute_action_internal()` checks `SANDBOX_TOOL_NAMES.contains(&action_name)` AND the tool's path argument starts with `/project/`. If both, looks up the mount via `WorkspaceMounts::resolve("/project/...")`, calls the resolved backend method directly. Otherwise, falls through to existing direct dispatch.
- `src/bridge/effect_adapter.rs` constructor — accept `Arc<WorkspaceMounts>`. Initialized in `src/bridge/router.rs` based on `SANDBOX_ENABLED`: if set, register `ContainerizedFilesystemBackend(project_id)` for `/project/`; otherwise register `FilesystemBackend(host_path)`.
- `src/bridge/router.rs` — instantiate `WorkspaceMounts` once at engine startup. Wire `ProjectSandboxManager` and the chosen backend into `EffectBridgeAdapter::new`.
- `src/main.rs` / `src/app.rs` — read `SANDBOX_ENABLED` env var early; if set, ensure Docker is reachable on startup and warn if not.
- `Cargo.toml` — confirm `bollard` is available to the main crate (already used by `src/sandbox/`).

### Reused without modification

- `src/sandbox/container.rs` — `connect_docker()`, image pull helpers, `ContainerRunner` building blocks
- `src/tools/builtin/file.rs` — `file_read`, `file_write`, `list_dir`, `apply_patch` work as-is when constructed with `base_dir=/project/`
- `src/tools/builtin/shell.rs` — works as-is when constructed with `working_dir=/project/`
- `src/tools/builtin/path_utils.rs:78` — existing `validate_path` enforces the sandbox at the tool layer
- `src/tools/registry.rs` — used by the daemon to register the five sandbox tools
- `crates/ironclaw_engine/src/runtime/manager.rs` — no changes; the engine doesn't need to know about sandboxes. All routing lives in the bridge.

## Edge cases addressed

- **`shell_exec` cwd**: Daemon constructs the shell tool with `working_dir=/project/` so commands without an explicit `workdir` param run in the project root.
- **Sub-threads via `rlm_query()`**: Inherit `project_id` (`crates/ironclaw_engine/src/runtime/manager.rs:505`), so they automatically use the parent project's container. No special case needed.
- **Project folder doesn't exist on first call**: `lifecycle::ensure_project_dir(project_id)` creates `~/.ironclaw/projects/<user_id>/<project_id>/` with mode 0700 before `docker create`. Idempotent.
- **Cold-start latency**: First sandboxed tool call to a stopped container pays ~500ms (start) + ~50ms (exec daemon). First call to a never-created project pays ~1–2s (create + start + image pull on first ever run). Acceptable; not in the hot path of conversational chat.
- **Container crash distinction**: `ContainerHandle::dispatch` returns `EngineError::ToolError` with `code=sandbox_error` for IPC failures. The orchestrator can distinguish these from `code=tool_error`.
- **Image management**: `ironclaw/sandbox:latest` built from `crates/Dockerfile.sandbox`. For local dev, `docker build -f crates/Dockerfile.sandbox -t ironclaw/sandbox:dev .` and use `IRONCLAW_SANDBOX_IMAGE=ironclaw/sandbox:dev`. v1 documents the manual command; CI publishing is future polish.
- **Stale containers from deleted projects**: Out of scope for v1. Future `ironclaw doctor` subcommand.
- **No project folder configured / sandbox disabled**: When `SANDBOX_ENABLED=false`, the `/project/` mount uses `FilesystemBackend(~/.ironclaw/projects/<id>/)` (or whatever the user configured). Tools still go through the mount table — same code path, different backend.
- **Resource limits**: Inherit defaults from `src/sandbox/config.rs::ResourceLimits` (memory, CPU shares). Configurable via `SandboxConfig::for_project(project_id)`.
- **Concurrent threads dispatching to one container**: Daemon serializes in v1. The host-side `ContainerHandle` queues requests via a tokio `mpsc` channel; the writer task drains the queue and writes one request at a time, the reader task reads responses and resolves oneshots by id. Throughput is bounded but ordering is guaranteed.
- **Two IronClaw processes accessing the same project container**: Single-user single-installation assumption for v1. Both would `docker exec -i` separate daemons against the same container — daemons don't share state, but they share the writable filesystem. Out of scope to fully solve; documented as "do not run two IronClaws against the same project".
- **Daemon binary discovery**: Compiled into the container image at build time. Container image version pinned in `SandboxConfig`.
- **Path semantics across backends**: `FilesystemBackend` and `ContainerizedFilesystemBackend` both treat `/project/foo.txt` as `foo.txt` relative to the mount root. Tool input passes through `Workspace::resolve_mount` which strips the `/project/` prefix before calling the backend. Tools never see absolute host paths or absolute container paths — they see relative paths inside the mount.
- **What if the agent writes via `file_write` and then runs `shell ls`?** Both go through the same mount → same backend → same container (or same host fs). The shell sees the file because the bind mount is live.

## Implementation phases

Each phase is independently shippable and reviewable.

### Phase 1: Mount-backend abstraction (subset of #1894 Phase 1) — DONE (#2211)

- Create `crates/ironclaw_engine/src/workspace/mount.rs` with `MountBackend` trait, `WorkspaceMounts`, `MountError`, `DirEntry`, `ShellOutput`.
- Create `src/bridge/sandbox/intercept.rs` with `maybe_intercept` and `SANDBOX_TOOL_NAMES`. (Originally planned as `src/bridge/workspace_filesystem.rs`; the actual location keeps the bridge-side glue colocated under `src/bridge/sandbox/`.)
- Add `WorkspaceMounts` field to `EffectBridgeAdapter` via `set_workspace_mounts()` setter (consistent with existing optional collaborators like `set_http_interceptor`). Default is `None`; tests inject manually until Phase 6 wires the router.
- Modify `EffectBridgeAdapter::execute_action_internal()` to detect `/project/` paths in input for the five sandbox-eligible tools and route through `WorkspaceMounts::resolve(path)`. Behavior is unchanged because the default backend slot is `None`.
- Tests: round-trip read/write via `FilesystemBackend`; mount resolution longest-prefix-match; `intercept_actually_dispatches_into_backend` (counting backend test); 5 integration tests in `tests/engine_v2_sandbox_integration.rs` driving `EffectBridgeAdapter::execute_action()` end-to-end per the "Test Through the Caller" rule.
- This phase ships with no Docker dependency. It establishes the abstraction layer.

### Phase 2: Project workspace folder concept — DONE

- Add `workspace_path: Option<PathBuf>` to `crates/ironclaw_engine/src/types/project.rs`.
- Add accessor that defaults to `~/.ironclaw/projects/<user_id>/<project_id>/`.
- Add `ensure_project_dir(&self) -> io::Result<PathBuf>` helper that creates the dir with mode 0700 if missing.
- Migrate `HybridStore` to persist the new field.
- Wire the per-project `FilesystemBackend(workspace_path)` into the mount table on thread spawn.
- Tests: round-trip persistence, default path, idempotent dir creation.

### Phase 3: Standalone daemon binary — DONE

- Create `src/bin/sandbox_daemon.rs`.
- Build a `ToolRegistry` containing only the five sandboxed tools, each with `base_dir=/project/` (configurable via env var for local testing).
- NDJSON read loop on stdin, dispatch, write response on stdout.
- `health` and `shutdown` methods.
- Test directly with `cargo run --bin sandbox_daemon` outside Docker — pipe JSON in via shell, verify output.
- No Docker dependency yet.

### Phase 4: Sandbox container image — DONE

- Add `crates/Dockerfile.sandbox`.
- Multi-stage build: stage 1 builds `sandbox_daemon` binary (Rust); stage 2 is `debian:stable-slim` + tini + common build tools + the binary.
- Document the build command in `crates/Dockerfile.sandbox` header.

### Phase 5: ProjectSandboxManager + ContainerizedFilesystemBackend — DONE

- Implement `src/bridge/sandbox/{manager,handle,protocol,lifecycle,backend}.rs`.
- `ContainerHandle::new(project_id, project_path)` does the create-if-missing → start → exec daemon dance and returns a handle with stdin/stdout streams.
- `ContainerizedFilesystemBackend` implements `MountBackend` by serializing each method into a JSON-RPC `execute_tool` call.
- Integration test: programmatically create a real container, call `file_write` then `file_read`, verify roundtrip via the mount-backend → JSON-RPC → daemon path.
- Idle-timeout background task lives in `ProjectSandboxManager::spawn_idle_reaper()`.

### Phase 6: Wire into EffectBridgeAdapter and gate behind env var — DONE

- In `src/bridge/router.rs`, when `SANDBOX_ENABLED=true`, register `ContainerizedFilesystemBackend(project_id)` for `/project/` instead of `FilesystemBackend`.
- Surface IPC errors as `EngineError::ToolError` with `code=sandbox_error`.
- End-to-end test with `ENGINE_V2=true SANDBOX_ENABLED=true`: spawn a thread, agent calls `shell_exec("echo hi > /project/test.txt")`, then `file_read("/project/test.txt")`, verify both go through the container and the host sees the file at `~/.ironclaw/projects/<project_id>/test.txt`.

### Phase 7: Polish

- Container restart on daemon/container crash.
- Cleanup of stopped container handles after extended idle.
- Logs/metrics: tool dispatch latency, container start time, in-flight queue depth.
- Test that `cargo install rg && rg foo` persists across container stop/start.
- Documentation in `crates/ironclaw_engine/CLAUDE.md` and `docs/plans/2026-03-20-engine-v2-architecture.md` (mark Phase 8 as in progress).
- Brief note in nearai/ironclaw#1894 about the partial mount-backend abstraction this lands.

## Verification

```bash
# Build
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo build --bin sandbox_daemon
docker build -f crates/Dockerfile.sandbox -t ironclaw/sandbox:dev .

# Unit tests for the mount backends and daemon (no Docker needed)
cargo test -p ironclaw_engine workspace::
cargo test --lib bridge::sandbox
cargo test --test engine_v2_sandbox_integration

# Integration test with real Docker (gated on Docker availability)
cargo test --features integration sandbox_

# End-to-end manual test
ENGINE_V2=true SANDBOX_ENABLED=true \
  IRONCLAW_SANDBOX_IMAGE=ironclaw/sandbox:dev \
  RUST_LOG=ironclaw::bridge::sandbox=debug \
  cargo run

# In the REPL:
#   "create a file foo.txt in /project with content hello"
#   "what's in /project/foo.txt"
#   "install ripgrep with cargo, then grep for hello in /project/foo.txt"
#   stop ironclaw
#   start ironclaw
#   "is ripgrep still installed? grep for hello again"

# Verify on the host:
ls ~/.ironclaw/projects/<project_id>/
cat ~/.ironclaw/projects/<project_id>/foo.txt

# Verify the container persists:
docker ps -a --filter name=ironclaw-sandbox
docker exec -it ironclaw-sandbox-<project_id> ls /root/.cargo/bin/

# Verify cleanup on idle (after 30 min or with shortened timeout)
docker ps --filter name=ironclaw-sandbox  # should show "Exited"
```

## Out of scope for v1 (future polish)

- Per-tool effect classification on the `Tool` trait (separate Phase 8 prerequisite, addressed independently)
- `PolicyEngine` wiring in the bridge (separate, addressed independently)
- Outbound network sandboxing via the existing proxy
- Two-phase commit for `WriteExternal`/`Financial` effects
- Concurrent multiplexed dispatch to one daemon (in-flight `id` map exists; we just serialize for v1)
- Per-project resource limit configuration UI
- "Reset project environment" CLI command
- "Clone project environment" via `docker commit`
- Custom `Dockerfile.<project_id>` for user-customized base images
- `ironclaw doctor` cleanup of stale containers
- WASM channel sandboxing (Phase 8.3 — likely already transparent through the bridge, needs verification)
- nearai/ironclaw#1894 Phase 2 (route `memory_*` through workspace mounts), Phase 3 (unify tool surface, rename to `read_file`/`write_file`/`list`/`search`), Phase 5 (dynamic mount expansion, future `S3`/`GitRemote` backends)
- Hosted-mode db-backed `/project/` with `materialize()` / `sync_back()` for shell tool — only relevant when #1894's full mount-table model lands
