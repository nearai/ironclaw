# IronClaw Reborn runtime selection contract

**Date:** 2026-04-26
**Status:** Decision guide / contract boundary
**Depends on:** `docs/reborn/contracts/capabilities.md`, `docs/reborn/contracts/dispatcher.md`, `docs/reborn/contracts/processes.md`, `docs/reborn/contracts/scripts.md`, `docs/reborn/contracts/wasm.md`, `docs/reborn/contracts/host-runtime.md`
**Profile presets:** `docs/reborn/contracts/runtime-profiles.md`

---

## 1. Purpose

IronClaw Reborn has several execution needs that look similar but have different security and ergonomics requirements:

- code-as-control-flow for agent reasoning and capability composition
- extension-provided durable capabilities
- simple REST/API integrations
- local process and stdio MCP execution
- full coding experiments with package installs and shell commands
- network egress governance across all of the above

This document fixes the selection rules so we do not collapse these needs into one mega-runtime or one mega-tool.

The guiding rule is:

```text
RuntimeKind describes what kind of work is being executed.
SandboxBackend describes how a process-backed runtime is contained.
CapabilityHost remains the authority boundary.
```

A sandbox can reduce blast radius, but it does not grant authority. Grants, leases, approvals, resource accounting, secret handling, network policy, and audit stay outside the runtime lane.

---

## 2. Runtime taxonomy

```rust
enum RuntimeKind {
    ActionScript,
    Wasm,
    DeclarativeHttp,
    Script,
    Mcp,
    LocalProcess,
    AgentLoopProcess,
    Experiment,
}

enum SandboxBackend {
    None,
    Srt,
    SmolVm,
    Docker,
}
```

`SandboxBackend::Srt`, `SandboxBackend::SmolVm`, and `SandboxBackend::Docker` are not public authority surfaces. They are implementation choices for process-backed lanes.

Examples:

```text
RuntimeKind::ActionScript       -> QuickJS isolate, later Pyodide/WASI CPython
RuntimeKind::Wasm               -> Wasmtime component/module
RuntimeKind::DeclarativeHttp    -> host-owned HTTP dispatcher, no child process
RuntimeKind::Mcp                -> stdio/server process, usually sandboxed by SRT or VM/container
RuntimeKind::Script             -> declared command runner, sandboxed when process-backed
RuntimeKind::Experiment         -> disposable or persistent coding workspace, usually SmolVM/Docker
```

---

## 3. Short decision table

| Need | Preferred lane | Why |
|---|---|---|
| Replace Monty for normal CodeAct/tool composition | `ActionScript` on QuickJS | Real JS syntax, no ambient host authority, cheap embedding, async host bridge |
| Keep Python-shaped CodeAct later | `ActionScript` on Pyodide or WASI CPython | Realer Python than Monty, still capability-shaped; heavier than QuickJS |
| Run local MCP stdio server | `Mcp` with `SandboxBackend::Srt` | Long-running process amortizes SRT startup; host controls filesystem/network policy |
| Run trusted-ish helper script | `Script` with `SandboxBackend::Srt` | Real host interpreter/tooling with process sandbox defense-in-depth |
| Run generated or untrusted coding experiment | `Experiment` with `SandboxBackend::SmolVm` | Full Linux, package installs, persistent overlay, stronger hypervisor boundary |
| Run heavy reproducible build/test workload | `Experiment` with SmolVM or Docker | Real shell and dependencies; choose backend by image/cgroup/host support needs |
| Implement durable extension capability | `Wasm` | Portable no-ambient-authority implementation runtime |
| Wrap simple REST API | `DeclarativeHttp` | No process or WASM needed; host owns network, credential injection, audit |
| Govern outbound HTTP/DNS | Network broker/policy layer | Cross-cutting egress boundary, not an execution runtime |
| Execute raw shell | Explicit capability inside `Script`/`Experiment` | Never ambient CodeAct behavior; always scoped, audited, and sandboxed |

---

## 4. Comparison table

| Runtime / approach | What it is | Real language? | Full OS / shell? | Isolation model | Best IronClaw role | Recommendation |
|---|---|---:|---:|---|---|---|
| Monty | Embedded pseudo-Python CodeAct VM | Partial Python-like | No | Interpreter-level no-ambient-authority sandbox | Legacy CodeAct/tool-composition runtime | Deprecate as primary path |
| QuickJS ActionScript | Embedded real JavaScript runtime | Yes, JS | No | JS isolate / embedded VM with host-defined globals | Primary Monty replacement for CodeAct | Strong yes |
| Runline-style QuickJS/WASM | Agent-written JS in QuickJS/WASM with host-proxied actions | Yes, JS | No | JS/WASM sandbox; action bridge outside sandbox | Pattern and catalog source for ActionScript | Borrow pattern, not wholesale runtime |
| Deno/V8 isolate | Heavier embedded JS/TS runtime | Yes, JS/TS | No by default | V8 isolate with custom host ops | Future advanced JS/TS backend | Maybe later |
| Pyodide / CPython-WASM | CPython compiled to WebAssembly | Mostly real Python | No | WASM sandbox | Python ActionScript backend | Possible, heavier |
| WASI CPython | CPython on WASI/Wasmtime | Mostly real Python | No | WASI capability sandbox | Long-term Python ActionScript backend | Promising but more work |
| RustPython | Python interpreter implemented in Rust | Python-like, incomplete | No | Embedded interpreter | Possible Monty alternative | Risk repeating Monty compatibility pain |
| Native CPython embedding | Real CPython inside host process | Yes | Potentially, unless stripped | Weak in-process sandbox | Trusted local helpers only | Avoid for untrusted model code |
| SRT | OS-level sandbox for host processes | Host Python/Node/Bash | Host-process shell | macOS Seatbelt / Linux bubblewrap plus proxy filtering | Lightweight process sandbox backend | Use for MCP/local process/script helpers |
| SmolVM (`smol-machines/smolvm`) | Local Linux microVM runtime using libkrun/KVM/Hypervisor.framework | Yes, inside guest | Yes | Per-workload VM/hypervisor | Coding experiment runtime / disposable agent computer | Strong yes for experiments |
| SmolVM (`CelestoAI/SmolVM`) | AI-agent microVM sandbox platform | Yes, inside guest | Yes | MicroVM sandbox infrastructure | Agent computer/browser/session sandbox | Interesting; evaluate separately |
| Docker | Container runtime | Yes, inside container | Yes | Namespaces/cgroups, shared kernel | Heavy/reproducible sandbox backend | Keep for selected workloads |
| Wasmtime/WASM | Portable capability runtime | Depends on source language | No | WASM capability sandbox | Extension capability implementation runtime | Keep |
| Declarative HTTP | Host-executed API call descriptor | No | No | Host network/secret policy | Simple REST integrations | Strong yes |
| MCP stdio process | External MCP server process | Server-defined | Process runtime | Needs SRT/Docker/SmolVM/etc. | Tool server integration lane | Use with sandbox backend |
| Raw shell | Direct Bash/sh command execution | N/A | Yes | Depends on backend | Explicit approved capability only | Never as ambient CodeAct default |
| CrabTrap-like network broker | Egress policy/proxy layer | N/A | N/A | HTTP/DNS/proxy enforcement | Cross-runtime network boundary | Add as policy layer, not runtime |

---

## 5. CodeAct-specific comparison

`CodeAct` here means code as the control plane for branching, variables, loops, async capability calls, parallelism, intermediate state, and final answer construction. It does not mean arbitrary shell access.

| Feature CodeAct needs | Monty | QuickJS ActionScript | Python WASM ActionScript | Real Python/Node via SRT | SmolVM experiment |
|---|---:|---:|---:|---:|---:|
| Real language syntax | No / partial | Yes | Mostly yes | Yes | Yes |
| No ambient filesystem | Yes | Yes if not exposed | Yes if WASM FS restricted | Only via sandbox policy | Via VM mount/copy policy |
| No ambient network | Yes | Yes if no raw `fetch` | Yes if no network exposed | Via SRT/proxy policy | Network off or allowlisted |
| No ambient secrets | Yes | Yes | Yes | Must scrub env/mounts | Must not inject raw secrets |
| Host-mediated capability calls | Yes | Yes via `ic.call` | Yes via `ic.call` bridge | Yes via SDK/stdio bridge | Yes, but less natural |
| Explicit finalization | `FINAL(...)` | `ic.final(...)` | `ic.final(...)` | SDK final frame | Wrapper protocol |
| Async/parallel calls | `await` / `asyncio.gather` | Promises / `ic.parallel` | Possible | Possible | Possible but heavier |
| Context/state injection | `context`, `goal`, `state` | `ic.context`, `ic.state` | `ic.context`, `ic.state` | SDK context frame | Session/filesystem oriented |
| Interpreter/resource limits | Built in | Host/runtime limits needed | WASM/host limits | OS/process limits needed | VM/resource limits |
| Shell/package installs | No | No | No/limited | Possible but risky | Yes, natural |
| Best use | Legacy safe composition | Primary CodeAct replacement | Python CodeAct later | Process-backed helpers/MCP | Coding experiments |

Monty did not provide true shell-like behavior. It provided REPL-like tool composition. Shell behavior only existed when Monty called a host-approved shell capability.

---

## 6. Required ActionScript contract

The Monty replacement must preserve the useful CodeAct semantics without pretending that a partial language is full Python.

Minimum host SDK surface:

```javascript
await ic.call(name, params)
await ic.parallel([{ name, params }, ...])
ic.final(value)
ic.log(value)
ic.artifact(name, value)
await ic.context()
ic.state
```

Generated stubs may sit on top of the SDK:

```javascript
import { web_fetch, github_list_issues } from "ironclaw/tools";

const page = await web_fetch({ url: "https://example.com" });
const issues = await github_list_issues({ owner: "nearai", repo: "ironclaw" });
ic.final({ page: page.text.slice(0, 500), issue_count: issues.items.length });
```

Rules:

- no raw filesystem, network, environment, process, or secret access by default
- all authority-bearing work goes through `CapabilityHost`
- direct network APIs such as raw `fetch` are absent unless bound to the host network broker
- direct shell/subprocess APIs are absent in ActionScript mode
- `ic.call` must preserve leases, approval pause/resume, audit, resource accounting, and redaction
- `ic.parallel` is a convenience over independently authorized capability calls, not a batch authority bypass
- `ic.final` is a structured final-answer signal, not a plain stdout convention

---

## 7. Process-backed runtime rules

For SRT, SmolVM, Docker, MCP, and raw shell-backed work:

- model/user input must not become raw host flags, Docker flags, VM flags, mount specs, or environment variables
- runner profile, command, image, mounts, and network policy come from validated extension/capability descriptors or host policy
- child processes receive scoped scratch filesystems by default
- host project access should prefer copy-in or read-only mount plus patch/artifact export
- writable host mounts require explicit grant and approval
- raw secrets should not enter child environments unless the capability contract explicitly requires it and policy approves it
- network access should route through the host network broker or backend-specific policy compiled by the host
- stdout/stderr/output must be bounded and recorded as scoped artifacts/results
- cancellation, timeout, and resource accounting remain host-owned

SRT is best for lightweight process containment and long-running local servers. SmolVM/Docker are better for package installation, dirty workspaces, and heavier coding experiments.

---

## 8. Experiment runtime posture

Coding experiments need real shell behavior, but that shell should not be the default CodeAct control plane.

Preferred flow:

```text
Host repo
  -> copy into sandbox or mount read-only
  -> run package installs/tests/builds inside SmolVM or Docker
  -> collect stdout/stderr/artifacts
  -> export patch/diff
  -> host applies patch through filesystem capability if approved
```

Avoid:

```text
agent shell has unrestricted writable mount to host repo
agent shell receives raw API tokens and unrestricted internet
agent shell is the audit unit instead of provider/action capability calls
```

The experiment runtime may expose shell commands, but it must remain a capability-backed process with scoped lifecycle, artifacts, cancellation, and audit.

---

## 9. Anti-patterns to avoid

- Replacing Monty with one opaque `execute_runtime` or `execute_runline` mega-tool.
- Making SRT, SmolVM, or Docker a new authority boundary.
- Treating runtime sandboxing as a substitute for grants, approvals, secret brokerage, and audit.
- Giving generated extensions custom host APIs instead of existing role contracts.
- Exposing a huge external action catalog by default.
- Auditing only generic HTTP or generic shell when the semantic operation is provider/action-specific.
- Injecting raw secrets into prompts, snippets, or child environments as the default integration strategy.
- Using pseudo-Python as the primary model-authored language again.
- Letting ActionScript grow ambient Node/Python OS APIs until it becomes raw shell by accident.

---

## 10. Concrete recommendations

1. Keep `CapabilityHost`, `RuntimeDispatcher`, approvals, resources, run-state, and process lifecycle as the stable authority path.
2. Deprecate Monty as the primary CodeAct runtime because Python-like incompatibilities cause avoidable LLM errors.
3. Add `RuntimeKind::ActionScript` with QuickJS first.
4. Add Python WASM only if real Python-shaped CodeAct remains important after QuickJS lands.
5. Use SRT as a `SandboxBackend` for local process, script, MCP stdio, and external agent-loop processes.
6. Use SmolVM as the preferred `Experiment` backend for multi-step coding experiments and untrusted generated code.
7. Keep Docker as an optional experiment/script backend for reproducible images, cgroups, and CI-like workloads.
8. Keep WASM for durable extension capabilities and declarative HTTP for simple REST integrations.
9. Add or preserve a cross-cutting network broker/policy layer for all outbound HTTP/DNS paths.
10. Make raw shell an explicit, scoped, audited capability inside process-backed lanes, never the ambient CodeAct substrate.
