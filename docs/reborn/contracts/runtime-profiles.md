# IronClaw Reborn runtime profiles contract

**Date:** 2026-04-26
**Status:** Decision guide / profile boundary
**Depends on:** `docs/reborn/contracts/runtime-selection.md`, `docs/reborn/contracts/capabilities.md`, `docs/reborn/contracts/dispatcher.md`, `docs/reborn/contracts/filesystem.md`, `docs/reborn/contracts/processes.md`, `docs/reborn/contracts/resources.md`, `docs/reborn/contracts/approvals.md`, `docs/reborn/contracts/host-runtime.md`

---

## 1. Purpose

IronClaw should support both a secure default assistant runtime and a fast local coding-agent experience without forking the architecture.

The mechanism is a host-owned `RuntimeProfile`:

```text
same agent loop
same CapabilityHost
same RuntimeDispatcher
same events/audit/resource model
different filesystem/process/network/approval backends
```

A local coding agent can therefore use direct host filesystem and shell backends, but only through the same capability and runtime contracts used by the secure profile.

The guiding invariant is:

```text
Profile changes backend permissiveness.
Profile does not bypass CapabilityHost.
```

---

## 2. Terminology

`RuntimeKind` answers: what kind of work is this?

```text
ActionScript, Wasm, DeclarativeHttp, Script, Mcp, LocalProcess, AgentLoopProcess, Experiment
```

`SandboxBackend` answers: how is a process-backed runtime contained?

```text
None, Srt, SmolVm, Docker
```

`RuntimeProfile` answers: what trust/policy preset should the host apply for this session?

```text
SecureDefault, LocalSafe, LocalDev, LocalYolo, Sandboxed, Experiment
```

The three are separate. For example:

```text
RuntimeKind::Script + SandboxBackend::None + RuntimeProfile::LocalDev
  -> direct local shell capability inside a local coding session

RuntimeKind::Script + SandboxBackend::Srt + RuntimeProfile::SecureDefault
  -> sandboxed script capability for safer/default assistant use

RuntimeKind::Experiment + SandboxBackend::SmolVm + RuntimeProfile::Experiment
  -> disposable Linux coding workspace
```

---

## 3. Profile API sketch

```rust
pub enum RuntimeProfile {
    SecureDefault,
    LocalSafe,
    LocalDev,
    LocalYolo,
    Sandboxed,
    Experiment,
}

pub struct RuntimeProfileConfig {
    pub filesystem_backend: FilesystemBackendKind,
    pub process_backend: ProcessBackendKind,
    pub network_mode: NetworkMode,
    pub secret_mode: SecretMode,
    pub approval_policy: ApprovalPolicy,
    pub audit_mode: AuditMode,
}

pub enum FilesystemBackendKind {
    ScopedVirtual,
    HostWorkspace {
        root: VirtualOrHostRoot,
        allow_outside_root: bool,
        symlink_policy: SymlinkPolicy,
    },
}

pub enum ProcessBackendKind {
    Srt,
    SmolVm,
    Docker,
    LocalHost {
        cwd: VirtualOrHostRoot,
        shell: ShellProfile,
        inherit_env: EnvPolicy,
    },
}

pub enum NetworkMode {
    Deny,
    Brokered,
    DirectLogged,
    Direct,
}
```

Concrete enum and type names can change. The important boundary is that profile selection resolves to host-owned backend and policy configuration before any agent-originated side effect runs.

---

## 4. Standard profiles

| Profile | Filesystem | Process | Network | Secrets | Approval posture | Use case |
|---|---|---|---|---|---|---|
| `SecureDefault` | scoped virtual filesystem / declared mounts | SRT, SmolVM, Docker, or no process | brokered through network policy | brokered handles only | policy-driven approvals | default assistant, remote channels, generated extensions |
| `LocalSafe` | host workspace read, ask on writes | local host shell ask-by-default | brokered or direct-logged | no inherited env by default | ask for writes/shell/destructive actions | cautious local coding agent |
| `LocalDev` | host workspace read/write under selected root | local host shell with dangerous-command gates | direct-logged or brokered | limited inherited env by profile | allow common dev work, ask for dangerous actions | default local coding agent |
| `LocalYolo` | host workspace direct | local host shell direct | direct or direct-logged | inherited env if user opts in | minimal per-call approval | explicit trusted laptop mode |
| `Sandboxed` | scoped or read-only mount plus scratch | SRT/Docker/SmolVM | brokered/allowlisted | brokered handles only | policy-driven approvals | safer execution of helper processes |
| `Experiment` | copy-in or read-only repo plus sandbox overlay | SmolVM or Docker | allowlisted/brokered | brokered handles only | ask before host patch apply | package installs, tests, benchmarks, generated code |

`LocalYolo` must be explicit. It should never be the remote/web/mobile default.

---

## 5. Local coding agent mode

A local coding-agent command should be a profile over the normal host runtime, not a separate product architecture:

```bash
ironclaw code . --profile local-dev
ironclaw code . --profile local-safe
ironclaw code . --profile local-yolo
ironclaw code . --profile sandboxed
```

Startup should print the active trust boundary:

```text
IronClaw Local Coding Agent
Profile: local-dev
Filesystem: direct workspace writes under /repo
Shell: local host shell; approval for dangerous commands
Network: direct/logged
Secrets: inherited env limited by profile
Audit: enabled
```

The user-facing tool surface can remain coding-agent friendly:

| User-facing tool | Capability | LocalDev backend |
|---|---|---|
| `read` | `filesystem.read` | `HostWorkspace` |
| `write` | `filesystem.write` | `HostWorkspace` |
| `edit` | `filesystem.apply_patch` / exact edit | `HostWorkspace` |
| `grep` | `filesystem.grep` / `workspace.search` | `HostWorkspace` |
| `find` | `filesystem.find` | `HostWorkspace` |
| `ls` | `filesystem.list` | `HostWorkspace` |
| `bash` | `process.run` / `shell.run` | `LocalHost` |
| `action_script.run` | `ActionScript` | QuickJS bridge, still host-mediated |
| `experiment.*` | `Experiment` | SmolVM/Docker as selected |

The implementation remains:

```text
AgentTool.execute(...)
  -> CapabilityHost.invoke_json(...)
  -> RuntimeDispatcher.dispatch_json(...)
  -> LocalHost / HostWorkspace backend when the profile allows it
```

Never:

```text
AgentTool.execute(...)
  -> fs/promises or child_process directly
```

---

## 6. Direct host access semantics

Direct host access is two independent backend decisions.

### 6.1 Host workspace filesystem

`HostWorkspace` allows capabilities to operate on a selected local project root.

Rules:

- relative paths resolve under the selected workspace root
- writes outside the workspace root are denied unless `allow_outside_root` is explicitly true
- absolute paths are normalized through the profile's path policy
- symlink traversal policy must be explicit
- file operations still emit capability events and audit records
- git dirty-state awareness should run before writes when a git repository is detected

### 6.2 Local host process

`LocalHost` runs commands on the host rather than in SRT/SmolVM/Docker.

Rules:

- process start still goes through `CapabilityHost` and process lifecycle stores
- cwd is the selected workspace root unless a profile permits otherwise
- timeout, cancellation, stdout/stderr limits, and output artifact recording remain mandatory
- environment inheritance is a profile setting, not ambient default
- dangerous-command classifiers and approval gates can still apply
- direct network from local shell may not be fully observable unless proxying is enabled, so profile UI must disclose this

---

## 7. Approval presets

Profiles should compile into explicit approval policy, not scattered conditionals.

Suggested presets:

### `local-safe`

```text
allow: read/list/search under workspace
ask: writes, shell, network, secret access, outside-workspace paths
block by default: sudo, destructive outside-workspace operations, credential scraping
```

### `local-dev`

```text
allow: reads, writes under workspace, common non-destructive dev commands
ask: rm -rf, chmod/chown, sudo, curl|sh, package publish, git push, secret/env inspection, outside-workspace writes, destructive database commands
block by default: attempts to escape workspace or exfiltrate known secrets unless explicitly approved
```

### `local-yolo`

```text
allow: workspace reads/writes, local shell, normal network
ask: optional only for catastrophic/outside-root actions depending on user config
require: explicit startup confirmation and visible warning
```

Even `local-yolo` should keep audit, timeout, cancellation, output caps, path normalization, and redaction of obvious secrets from logs.

---

## 8. QuickJS / ActionScript interaction

Runtime profiles do not make `ActionScript` ambient.

Even in `LocalYolo`, QuickJS should not automatically get:

```text
fs
child_process
process.env
raw fetch
```

QuickJS remains the code-as-control-flow runtime:

```javascript
const file = await filesystem.read({ path: "README.md" });
const result = await shell.run({ command: "cargo test" });
ic.final({ filePreview: file.text.slice(0, 200), tests: result.exitCode });
```

Those calls still resolve through:

```text
QuickJS ic.call(...)
  -> ActionScriptHostBridge
  -> CapabilityHost
  -> RuntimeDispatcher
  -> profile-selected backend
```

A permissive local profile makes capabilities more permissive; it does not turn ActionScript into unstructured Node.js.

---

## 9. Relationship to a lightweight coding-agent loop

A local coding agent can borrow the `pi-mono` `packages/coding-agent` ergonomics:

- cwd-bound sessions
- `read`/`bash`/`edit`/`write`/`grep`/`find`/`ls` tool vocabulary
- print/RPC/TUI modes
- resource/project context loading
- extension hooks
- streaming UI events

But the IronClaw version must replace authority-bearing implementations:

```text
coding-agent style tool surface
  -> AgentTool wrapper
  -> CapabilityHost
  -> RuntimeDispatcher
  -> profile-selected backend
```

Extension hooks can ask, annotate, or block early, but they are not the final security layer. Grants, leases, approvals, secret brokerage, resources, process lifecycle, and audit remain host-owned.

---

## 10. Profile selection rules

Default selection should be conservative:

| Entrypoint | Default profile |
|---|---|
| remote/web/mobile assistant | `SecureDefault` |
| local CLI coding agent | `LocalDev` or `LocalSafe` |
| explicit `--yolo` local CLI | `LocalYolo` |
| generated/untrusted code experiment | `Experiment` |
| third-party extension helper process | `Sandboxed` |
| stdio MCP server | `Sandboxed` or `LocalDev` depending on trust/install source |

Escalation rules:

- remote channels cannot silently switch into direct host access
- local profiles require a local operator/session and explicit selected workspace root
- `LocalYolo` requires explicit user selection and startup disclosure
- profile changes during a turn require a new turn or explicit approval boundary
- generated extensions cannot declare their own profile; they declare runtime needs, and the host chooses the backend/profile

---

## 11. Non-goals

This contract does not require:

- making direct host access safe for untrusted remote operation
- exposing raw host APIs to agent-loop extensions
- allowing arbitrary generated extensions to choose `LocalHost`
- replacing SmolVM/Docker/SRT with local mode
- making QuickJS a raw Node.js runtime
- making local shell/network fully observable without explicit broker/proxy support

Local profiles are for explicit local operator workflows. They are not a shortcut around Reborn authority boundaries.

---

## 12. Concrete recommendation

Implement local coding support as:

```text
RuntimeProfile::LocalDev
  filesystem_backend = HostWorkspace { root = cwd, allow_outside_root = false }
  process_backend = LocalHost { cwd, inherit_env = limited }
  network_mode = DirectLogged or Brokered
  approval_policy = allow common dev work, ask for dangerous actions
  audit_mode = enabled
```

Keep the rest of the architecture identical:

```text
agent_loop
  -> AgentTool wrappers
  -> CapabilityHost
  -> RuntimeDispatcher
  -> HostWorkspace / LocalHost / DeclarativeHttp / QuickJS / Experiment backends
```

This gives IronClaw a fast local coding-agent mode while preserving the ability to switch the same loop and tools back to secure/sandboxed profiles later.
