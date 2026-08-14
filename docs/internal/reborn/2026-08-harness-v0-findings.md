# ACP harness v0 findings

Issue #7624 adds a deliberately narrow evaluation lane for running Claude Code
through the official ACP client protocol. The lane is default-off and selected
by explicit `[harness]` configuration plus a list of run-profile ids. Only
standalone development profiles may route turns; hosted profiles reject
routing and also fail closed if configured with host placement.

Host and Docker placement use the same pins from
`Dockerfile.claude-code-acp`; `scripts/install-claude-code-acp.sh` installs
those pins on a developer machine. The container uses the maintained
`@agentclientprotocol/claude-agent-acp`
adapter. The deprecated `@zed-industries/claude-code-acp` 0.16.2 advertised
session loading but failed to resume a session after its adapter process exited;
the maintained package replaces that implementation.

## Implementation observations

- Run-profile selection is owned by a neutral executor router whose default is
  the canonical Rust executor. The ACP executor implements `TurnRunExecutor`
  directly and has no fallback or knowledge of other loop implementations, so
  another executor can be registered without changing the harness.
- ACP gives Ironclaw a small, typed integration surface: initialize, session
  new/load, prompt, updates, and permission requests. Keeping the protocol at
  this boundary avoided translating Claude Code internals into Ironclaw tools.
- ACP agent-message chunks are accumulated, bounded, and sanitized before they
  enter the existing cumulative `ModelTextDelta` milestone path. The WebUI can
  therefore show the reply while it is generated without a harness-specific
  transport or one durable transcript write per chunk; the completed reply is
  still finalized once through the canonical transcript boundary.
- A stable workspace derived from the typed thread id is enough to retain both
  repository changes and the ACP session id across turns without putting ACP
  state into tenant storage.
- The v0 permission policy intentionally chooses the adapter's first offered
  allow option. This is suitable only for explicitly enabled developer harness
  runs.
- Container termination, bounded update collection, and terminal scheduler
  failures are mandatory. Without all three, a hung adapter can consume a
  worker indefinitely or cause a failed turn to be re-driven.
- The ACP executor receives only an opaque process transport and kill handle.
  Host placement starts a process with an empty ambient environment and an
  explicit developer credential list; Docker placement delegates lifecycle and
  mounts to the existing sandbox lane. Customer secret stores are not reachable
  from either configuration path.

## Evaluation status

The deterministic fake-adapter host tests cover protocol compatibility,
cumulative live text updates, multi-turn session reuse, permission handling,
process death, timeout cleanup, lease release, and reply persistence without
paid API use. The same fake runs through Docker in the repository's gated
Docker lane to pin placement parity.
A live Claude Code smoke run still requires an operator-provided developer key
through the explicitly named host environment variable. No live latency or cost
numbers are recorded here because no credential was supplied during
implementation.

Recommendation: keep the harness experimental and default-off. Consider a
broader product integration only after repeated live trials show stable ACP
behavior and a clear quality or engineering-throughput advantage over the
canonical executor.
