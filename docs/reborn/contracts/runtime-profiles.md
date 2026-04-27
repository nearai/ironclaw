# IronClaw Reborn runtime profiles contract

**Date:** 2026-04-26
**Status:** Decision guide / profile boundary
**Depends on:** `docs/reborn/contracts/runtime-selection.md`, `docs/reborn/contracts/capabilities.md`, `docs/reborn/contracts/dispatcher.md`, `docs/reborn/contracts/filesystem.md`, `docs/reborn/contracts/processes.md`, `docs/reborn/contracts/resources.md`, `docs/reborn/contracts/approvals.md`, `docs/reborn/contracts/host-runtime.md`

---

## 1. Purpose

IronClaw should support secure default assistant runtimes, fast local coding-agent sessions, hosted multi-tenant deployments, and enterprise-dedicated deployments without forking the architecture.

The mechanism is a host-owned `RuntimeProfile` constrained by `DeploymentMode`:

```text
same agent loop
same CapabilityHost
same RuntimeDispatcher
same events/audit/resource model
different filesystem/process/network/approval backends
```

A local coding agent can therefore use direct host filesystem and shell backends, while hosted multi-tenant deployments can force tenant-scoped sandboxes, through the same capability and runtime contracts.

The guiding invariant is:

```text
DeploymentMode constrains the maximum authority available.
Profile changes backend permissiveness within that deployment.
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

`DeploymentMode` answers: where is IronClaw running and who owns the machine boundary?

```text
LocalSingleUser, HostedMultiTenant, EnterpriseDedicated
```

`RuntimeProfile` answers: what trust/policy preset should the host apply for this session?

```text
SecureDefault,
LocalSafe, LocalDev, LocalYolo,
HostedSafe, HostedDev, HostedYoloTenantScoped,
EnterpriseSafe, EnterpriseDev, EnterpriseYoloDedicated,
Sandboxed, Experiment
```

The four are separate. `DeploymentMode + RuntimeProfile + tenant/org policy` resolves to an effective runtime policy. For example:

```text
RuntimeKind::Script + SandboxBackend::None + RuntimeProfile::LocalDev
  -> direct local shell capability inside a local coding session

RuntimeKind::Script + SandboxBackend::Srt + RuntimeProfile::SecureDefault
  -> sandboxed script capability for safer/default assistant use

RuntimeKind::Experiment + SandboxBackend::SmolVm + RuntimeProfile::Experiment
  -> disposable Linux coding workspace

DeploymentMode::HostedMultiTenant + RuntimeProfile::HostedDev
  -> tenant workspace plus tenant-scoped sandbox process, never provider host shell

DeploymentMode::EnterpriseDedicated + RuntimeProfile::EnterpriseDev
  -> org-dedicated workspace and org-dedicated runner selected by org policy
```

---

## 3. Profile API sketch

```rust
pub enum DeploymentMode {
    LocalSingleUser,
    HostedMultiTenant,
    EnterpriseDedicated,
}

pub enum RuntimeProfile {
    SecureDefault,
    LocalSafe,
    LocalDev,
    LocalYolo,
    HostedSafe,
    HostedDev,
    HostedYoloTenantScoped,
    EnterpriseSafe,
    EnterpriseDev,
    EnterpriseYoloDedicated,
    Sandboxed,
    Experiment,
}

pub struct EffectiveRuntimePolicy {
    pub deployment: DeploymentMode,
    pub requested_profile: RuntimeProfile,
    pub resolved_profile: RuntimeProfile,
    pub resource_scope: ResourceScope,
    pub filesystem_backend: FilesystemBackendKind,
    pub process_backend: ProcessBackendKind,
    pub network_mode: NetworkMode,
    pub secret_mode: SecretMode,
    pub approval_policy: ApprovalPolicy,
    pub audit_mode: AuditMode,
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

The resolver must be monotonic with respect to safety:

```text
DeploymentMode + tenant/org policy may reduce requested profile authority.
They may not increase it.
```

Examples:

```text
LocalSingleUser + LocalDev
  -> HostWorkspace + LocalHost

HostedMultiTenant + LocalDev
  -> hard deny; local profiles are not valid in hosted multi-tenant

HostedMultiTenant + HostedDev
  -> TenantWorkspace + tenant-scoped sandbox + brokered network

EnterpriseDedicated + EnterpriseDev
  -> org-dedicated workspace and runner, subject to org admin policy
```

---

## 4. Standard profiles

| Profile | Filesystem | Process | Network | Secrets | Approval posture | Use case |
|---|---|---|---|---|---|---|
| `SecureDefault` | scoped virtual filesystem / declared mounts | SRT, SmolVM, Docker, or no process | brokered through network policy | brokered handles only | policy-driven approvals | default assistant and generated-extension baseline |
| `LocalSafe` | host workspace read, ask on writes | local host shell ask-by-default | brokered or direct-logged | no inherited env by default | ask for writes/shell/destructive actions | cautious local coding agent |
| `LocalDev` | host workspace read/write under selected root | local host shell with dangerous-command gates | direct-logged or brokered | limited inherited env by profile | allow common dev work, ask for dangerous actions | default local coding agent |
| `LocalYolo` | host workspace direct | local host shell direct | direct or direct-logged | inherited env if user opts in | minimal per-call approval | explicit trusted laptop mode |
| `HostedSafe` | tenant workspace read, ask on writes | tenant-scoped sandbox ask-by-default | brokered only | tenant-scoped broker only | ask for writes/process/external effects | hosted multi-tenant safe default |
| `HostedDev` | tenant workspace read/write | tenant-scoped sandbox | brokered/allowlisted only | tenant-scoped broker only | allow tenant dev work, ask for destructive/external effects | hosted multi-tenant developer mode |
| `HostedYoloTenantScoped` | tenant workspace direct | tenant-scoped sandbox direct | brokered/allowlisted only | tenant-scoped broker only | fewer approvals inside tenant boundary | explicit tenant-scoped yolo, never provider-host yolo |
| `EnterpriseSafe` | org-dedicated workspace read, ask on writes | org-dedicated sandbox/runner ask-by-default | brokered/org policy | org KMS/broker | org policy approvals | enterprise cautious mode |
| `EnterpriseDev` | org-dedicated workspace read/write | org-dedicated sandbox/runner | brokered/org policy | org KMS/broker | allow org dev work, ask for destructive actions | enterprise default developer mode |
| `EnterpriseYoloDedicated` | org-dedicated workspace direct if admin-enabled | org-dedicated runner direct if admin-enabled | org policy | org KMS/broker or admin-enabled env | org-admin-defined minimal approvals | dedicated enterprise infra only |
| `Sandboxed` | scoped or read-only mount plus scratch | SRT/Docker/SmolVM | brokered/allowlisted | brokered handles only | policy-driven approvals | safer execution of helper processes |
| `Experiment` | copy-in or read-only repo plus sandbox overlay | SmolVM or Docker | allowlisted/brokered | brokered handles only | ask before host patch apply | package installs, tests, benchmarks, generated code |

`LocalYolo` must be explicit and local-only. `HostedYoloTenantScoped` means fewer approvals inside a tenant sandbox, not direct provider-host authority.

---

## 5. Deployment mode constraints

Deployment mode is the outer authority envelope. Profiles are resolved inside it.

| Deployment mode | Direct host filesystem? | Direct host shell? | Default process backend | Network | Secrets | Yolo meaning |
|---|---:|---:|---|---|---|---|
| `LocalSingleUser` | yes, selected workspace | yes, selected cwd | `LocalHost` or SRT | direct-logged or brokered | local/brokered by profile | direct local machine authority after explicit selection |
| `HostedMultiTenant` | no provider-host access | no provider-host shell | per-tenant SRT/container/microVM | brokered/allowlisted only | tenant-scoped broker only | fewer approvals inside tenant sandbox only |
| `EnterpriseDedicated` | org-dedicated workspace only if admin-enabled | org-dedicated runner only if admin-enabled | org-dedicated runner/container/VM | org policy / brokered | org KMS/broker | org-admin-defined within dedicated infrastructure |

Hosted multi-tenant invariants:

- no profile may expose provider host filesystem or provider host shell
- all storage paths are tenant/user/project/agent scoped where the domain is agent-owned
- all process records carry tenant/user/project/agent resource scope where available
- all secrets are tenant scoped and never inherited from provider host environment
- all network egress is brokered or compiled from tenant policy
- quotas are mandatory for tenant, user, project, run, process, and capability scopes
- hosted yolo means sandbox-yolo, never host-yolo

Enterprise dedicated deployments are allowed to be more permissive only because the infrastructure is dedicated to one organization. That permissiveness is controlled by org admin policy, not by the model or generated extensions. Enterprise direct-runner modes are still not equivalent to a hosted service editing a provider-owned shared host.

This contract intentionally does not include a hosted-control-plane-to-local-runner design. If that becomes necessary later, it should be added as a separate remote-runner contract rather than folded into these profiles.

---

## 6. Local coding agent mode

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

## 7. Direct host access semantics

Direct host access is two independent backend decisions.

### 7.1 Host workspace filesystem

`HostWorkspace` allows capabilities to operate on a selected local project root.

Rules:

- relative paths resolve under the selected workspace root
- writes outside the workspace root are denied unless `allow_outside_root` is explicitly true
- absolute paths are normalized through the profile's path policy
- symlink traversal policy must be explicit
- file operations still emit capability events and audit records
- git dirty-state awareness should run before writes when a git repository is detected

### 7.2 Local host process

`LocalHost` runs commands on the host rather than in SRT/SmolVM/Docker.

Rules:

- process start still goes through `CapabilityHost` and process lifecycle stores
- cwd is the selected workspace root unless a profile permits otherwise
- timeout, cancellation, stdout/stderr limits, and output artifact recording remain mandatory
- environment inheritance is a profile setting, not ambient default
- dangerous-command classifiers and approval gates can still apply
- direct network from local shell may not be fully observable unless proxying is enabled, so profile UI must disclose this

---

## 8. Approval presets

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

## 9. QuickJS / ActionScript interaction

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

## 10. Relationship to a lightweight coding-agent loop

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

## 11. Profile selection rules

Default selection should be conservative:

| Entrypoint | Default profile |
|---|---|
| local CLI coding agent | `LocalDev` or `LocalSafe` |
| explicit `--yolo` local CLI | `LocalYolo` |
| hosted multi-tenant assistant | `HostedSafe` or `HostedDev` |
| hosted multi-tenant explicit yolo | `HostedYoloTenantScoped`, if tenant policy permits |
| enterprise dedicated assistant | `EnterpriseSafe` or `EnterpriseDev` |
| enterprise dedicated explicit yolo | `EnterpriseYoloDedicated`, if org admin policy permits |
| generated/untrusted code experiment | `Experiment` |
| third-party extension helper process | `Sandboxed` |
| stdio MCP server | `Sandboxed`, `LocalDev`, or `EnterpriseDev` depending on deployment and trust source |

Escalation rules:

- hosted multi-tenant deployments cannot select `Local*` profiles
- hosted multi-tenant deployments cannot expose provider host filesystem or shell, even for `HostedYoloTenantScoped`
- enterprise dedicated deployments may select `Enterprise*` profiles only inside org-dedicated infrastructure
- local profiles require a local operator/session and explicit selected workspace root
- `LocalYolo`, `HostedYoloTenantScoped`, and `EnterpriseYoloDedicated` require explicit selection and visible disclosure
- profile changes during a turn require a new turn or explicit approval boundary
- generated extensions cannot declare their own profile; they declare runtime needs, and the host chooses the backend/profile

Visible-surface rules:

- profile-impossible capabilities should be hidden before the model call, not exposed and denied repeatedly
- hosted multi-tenant visible surfaces must omit `LocalHost` and provider-host filesystem affordances entirely
- enterprise visible surfaces may include org-dedicated runner affordances only when org policy enables them
- local visible surfaces may include `HostWorkspace`/`LocalHost` affordances according to `LocalSafe`/`LocalDev`/`LocalYolo`
- capabilities that are possible but approval/auth/resource dependent may remain visible and block structurally at action time
- action-time authorization is still mandatory for visible capabilities because arguments, leases, quotas, auth, and concurrent state can change

---

## 12. Non-goals

This contract does not require:

- making direct host access safe for untrusted remote operation
- designing a hosted-control-plane-to-local-runner or BYO local runner path
- exposing raw host APIs to agent-loop extensions
- allowing arbitrary generated extensions to choose `LocalHost`
- allowing hosted multi-tenant profiles to touch provider host filesystem/shell
- replacing SmolVM/Docker/SRT with local mode
- making QuickJS a raw Node.js runtime
- making local shell/network fully observable without explicit broker/proxy support

Local profiles are for explicit local operator workflows. They are not a shortcut around Reborn authority boundaries.

---

## 13. Concrete recommendation

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

This gives IronClaw a fast local coding-agent mode while preserving the ability to switch the same loop and tools to hosted multi-tenant or enterprise-dedicated profiles later.
