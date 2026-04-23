# Harness Testing Architecture

> Status: Phase 1 of [#2828](https://github.com/nearai/ironclaw/issues/2828).
> Companion to `tests/e2e/CLAUDE.md`, `tests/support/LIVE_TESTING.md`,
> and `docs/internal/live-canary.md`.

This document is the **canonical map** of how IronClaw tests itself. It
exists so contributors can answer one question without re-deriving repo
history each time:

> *"My change touches X. Which testing layer should cover it, and where does
> the test go?"*

If you change harness infrastructure, update this document in the same PR.

---

## TL;DR

We test in **six layers**, ordered from cheapest/fastest to most realistic/expensive:

| # | Layer | Speed | Network? | Owner harness | When to use |
|---|---|---|---|---|---|
| 1 | **Deterministic replay** | < 1s | No | `tests/support/test_rig.rs` + `trace_llm.rs` | Default for any agent-loop, tool, or LLM-driven regression |
| 2 | **Rust integration** | seconds | In-process mocks | `tests/support/gateway_workflow_harness.rs`, ad-hoc `tests/*_integration.rs` | Multi-component wiring, gateway/API contracts, multi-tenant invariants |
| 3 | **Browser/API E2E** | tens of seconds | Local stack | `tests/e2e/scenarios/` (Python/Playwright) | User-visible workflows, transport behavior, OAuth UI |
| 4 | **Live canary** | minutes | Real providers | `tests/support/live_harness.rs` + `.github/workflows/live-canary.yml` | Drift detection against real LLM/MCP/OAuth providers |
| 5 | **Eval / benchmark** | minutes–hours | Mixed | (planned — see #2722) | Baseline-vs-candidate quality measurement over time |
| 6 | **Chaos / load / soak** | hours | Mixed | (planned — see #352, #1777, #1778) | Resilience and scale validation |

**Rule of thumb:** push every regression as far left (cheap, deterministic) as
possible. A live canary that catches a stable bug should usually graduate
into a replay fixture.

---

## Layer 1 — Deterministic replay

The cheapest and most-used layer. Real `Agent`, real tools, real DB
(in-memory libSQL), but the LLM is a **trace replay** that yields
pre-recorded responses.

### What it covers

- Agent loop behavior (turn handling, tool selection, finalization)
- Built-in and WASM tool execution
- Workspace/memory effects
- Engine v2 step/capability/lease invariants
- Hook ordering and skill selection
- Anything that can be expressed as "given this LLM script, the agent
  should do X"

### Key files

| File | Purpose |
|---|---|
| `tests/support/test_rig.rs` | `TestRigBuilder` — wires `Agent` + `TraceLlm` + `TestChannel` |
| `tests/support/trace_llm.rs` | `TraceLlm` provider that replays JSON traces |
| `tests/support/test_channel.rs` | In-process `Channel` impl that captures responses |
| `tests/support/instrumented_llm.rs` | LLM wrapper that records prompts/usage for assertions |
| `tests/support/replay_outcome.rs` | Declarative outcome assertions for trace turns |
| `src/llm/recording.rs` | Shared `HttpInterceptor`, `RecordingHttpInterceptor`, `ReplayingHttpInterceptor` |
| `tests/fixtures/llm_traces/` | Committed trace fixtures (organized by family) |

### When to use

- **Always first.** If a bug can be reproduced with a scripted LLM, it
  belongs here.
- New tool? Add a trace covering at least one happy-path call.
- New finalization rule? Add a trace covering a turn that should
  finalize and one that should not.
- A live canary caught a regression? Capture the trace and land it as a
  permanent fixture (see Promotion Rule below).

### When **not** to use

- Behavior that depends on the actual LLM picking the right tool — that
  belongs in a live canary.
- User-visible UI behavior — that belongs in E2E.

### Adding a fixture

1. Run the scenario live (often via a `tests/e2e_live_*.rs` test with
   `IRONCLAW_LIVE_TEST=1`) to record a trace into
   `tests/fixtures/llm_traces/live/`.
2. Move/rename it into the appropriate family directory and check it in.
3. Add a Rust test that loads the fixture via `TestRigBuilder` and
   asserts the relevant outcome.

---

## Layer 2 — Rust integration harnesses

Real app components wired together with mock external services. Lives
behind `--features integration` when it needs a real Postgres.

### What it covers

- Gateway routes and SSE/WS contracts
- Cross-module wiring (auth flow → DB → channel → tool dispatch)
- Multi-tenant isolation invariants
- MCP and extension lifecycle
- Anything that needs more than the agent loop but less than a browser

### Key files

| File | Purpose |
|---|---|
| `tests/support/gateway_workflow_harness.rs` | Boots gateway in-process, drives via HTTP/SSE |
| `tests/support/mock_openai_server.rs` | Rule-driven OpenAI-compatible HTTP mock |
| `tests/support/mock_mcp_server.rs` | MCP server with OAuth 2.1 + JSON-RPC |
| `tests/mcp_multi_tenant_integration.rs` | Multi-tenant MCP isolation contract |
| `tests/gateway_workflow_integration.rs` | Gateway happy-path workflows |
| `tests/multi_tenant_integration.rs` | Multi-tenant Rust-level isolation |
| `tests/heartbeat_integration.rs` | Heartbeat scheduler integration |
| Many more `tests/*_integration.rs` | Module-specific integration tests |

### When to use

- A bug spans multiple subsystems but does not need a browser.
- A gateway endpoint contract needs to stay stable across refactors.
- An invariant must hold across users/projects/threads.

### When **not** to use

- The behavior is purely agent-loop logic → use Layer 1 (replay).
- The behavior is user-visible → also add a Layer 3 E2E scenario.

---

## Layer 3 — Browser/API E2E

Python + Playwright + a real backend binary. Owned by `tests/e2e/`.
See `tests/e2e/CLAUDE.md`.

### What it covers

- The actual user journey through the web UI
- SSE reconnect, approval modals, OAuth pop-ups, skill prompts
- Cross-channel behavior (web + Telegram + Slack + webhook)
- Anything where "what the user sees" is the contract

### Key files

| File | Purpose |
|---|---|
| `tests/e2e/CLAUDE.md` | Setup, conventions, conftest |
| `tests/e2e/scenarios/test_chat.py` | Core chat flow |
| `tests/e2e/scenarios/test_sse_reconnect.py` | Streaming/SSE behavior |
| `tests/e2e/scenarios/test_v2_auth_oauth_matrix.py` | Auth + OAuth UI matrix |
| `tests/e2e/scenarios/` | All other scenarios |

### When to use

- The bug only reproduces with a real browser.
- A user-visible workflow gained or changed steps.
- Authentication/approval UI needs verification across providers.

### When **not** to use

- Behavior is fully testable at Layer 1 or 2 — keep E2E focused on the
  visible contract, not internal wiring.

---

## Layer 4 — Live canary

Real LLMs, real OAuth, real MCP servers. Runs on a schedule, not on every PR.

See `docs/internal/live-canary.md` and `tests/support/LIVE_TESTING.md`.

### What it covers

- Drift in real LLM provider responses (schema, finish reasons, tool-arg shapes)
- OAuth token refresh paths and provider quirks
- Real-network behavior the mocks cannot simulate
- Recording new fixtures for Layer 1 replay

### Key files

| File | Purpose |
|---|---|
| `tests/support/live_harness.rs` | `LiveTestHarnessBuilder` — switches live vs replay via `IRONCLAW_LIVE_TEST` |
| `tests/support/LIVE_TESTING.md` | How to record/replay live tests locally |
| `docs/internal/live-canary.md` | Lane policy, ownership, scheduling |
| `.github/workflows/live-canary.yml` | Scheduled lane runner |
| `tests/e2e_live*.rs` | Live test scenarios |
| `tests/fixtures/llm_traces/live/` | Recorded fixtures (committed, used as Layer 1 inputs) |

### When to use

- Detecting drift in a real provider that no mock can credibly simulate.
- Validating a new OAuth or extension end-to-end the first time.
- Re-recording a trace fixture after an upstream provider change.

### When **not** to use

- "Just to be sure" — every live lane has cost and flake budget.
  If the assertion can be made deterministic, it should be.

### Lane discipline

Per Phase 4 of #2828, every live lane should be classified as one of:

- **Permanent canary** — drift detection that fundamentally cannot be replayed
- **Temporary migration guard** — exists during a migration, scheduled to retire
- **Replay candidate** — should be downgraded once stabilized

Lanes without a documented owner and a documented reason "this cannot be
replay-only" are candidates for removal.

---

## Layer 5 — Eval / benchmark *(planned)*

Tracked by [#2722](https://github.com/nearai/ironclaw/issues/2722). Not
yet built. Goal: measure agent/harness quality over time with
baseline-vs-candidate runs across a representative scenario pack.

Reported metrics (from #2828):
- completion success rate
- explicit failure modes
- repeated-error loops
- tool-use validity
- wall-clock time
- finalization success

This layer answers "did this change make the agent better or worse?",
which the other five layers cannot.

---

## Layer 6 — Chaos / load / soak *(planned)*

Tracked by [#352](https://github.com/nearai/ironclaw/issues/352),
[#1777](https://github.com/nearai/ironclaw/issues/1777),
[#1778](https://github.com/nearai/ironclaw/issues/1778). Not yet built.
Covers provider chaos, multi-tenant load, and long-running soak.

---

## Coverage matrix

Rows: behavior families. Columns: layers. **`✓`** = covered;
**`~`** = partial; **`·`** = gap (with linked issue where one exists).

| Behavior family | L1 Replay | L2 Integration | L3 E2E | L4 Live |
|---|---|---|---|---|
| Agent loop / tool selection | ✓ | ✓ | ✓ | ✓ |
| Built-in tools | ✓ | ✓ | ~ | ✓ |
| WASM tools | ✓ | ✓ | ~ | ✓ |
| Approvals & gates | ~ | ✓ | ✓ | ~ |
| Auth / OAuth | ~ | ✓ | ✓ | ✓ |
| MCP happy path | ✓ | ✓ | ~ | ✓ |
| MCP failure / re-auth / recovery | · ([#1787](https://github.com/nearai/ironclaw/issues/1787)) | · ([#1787](https://github.com/nearai/ironclaw/issues/1787)) | · ([#1787](https://github.com/nearai/ironclaw/issues/1787)) | · |
| Multi-tenant isolation | ~ | ✓ | · ([#1788](https://github.com/nearai/ironclaw/issues/1788)) | — |
| Routines / heartbeat | ~ | ✓ | · | ~ |
| Gateway-initiated mutations | · ([#643](https://github.com/nearai/ironclaw/issues/643)) | ✓ | ✓ | — |
| SSE / WS reconnect | — | ✓ | ✓ | — |
| Sandbox / Docker | — | ✓ | — | ✓ |
| Eval / quality regression | — | — | — | — ([#2722](https://github.com/nearai/ironclaw/issues/2722)) |
| Chaos / load / soak | — | — | — | — ([#352](https://github.com/nearai/ironclaw/issues/352), [#1777](https://github.com/nearai/ironclaw/issues/1777), [#1778](https://github.com/nearai/ironclaw/issues/1778)) |

`—` means the layer is not the right home for that family. Updates to
this matrix should ship with the PR that closes a gap.

---

## Promotion rule

When a regression is caught in a more expensive layer, it should
graduate to the cheapest layer that can still reproduce it.

```
Production bug
   │
   ▼
Live canary catches it
   │
   ▼
Record trace via IRONCLAW_LIVE_TEST=1
   │
   ├─► Land Layer 1 replay fixture       (always, if loop-reproducible)
   │
   └─► If user-visible:
       Land Layer 3 E2E scenario
```

A bug that lives only in a live canary indefinitely is a smell — it
means we lack a hermetic reproduction. The default outcome of "live
canary went red" is a follow-up PR that lands a replay fixture.

---

## Decision tree: where does this test go?

```
Is the behavior agent-loop / tool / LLM-driven?
├─ Yes ─► Can it be expressed as a scripted LLM trace?
│         ├─ Yes ─► Layer 1 (replay).            [default]
│         └─ No  ─► Layer 4 (live canary).       [needs justification]
│
└─ No  ─► Is it user-visible (browser UI / OAuth pop-up)?
          ├─ Yes ─► Layer 3 (E2E)
          │        + Layer 2 if there is also a backend contract to lock down
          │
          └─ No  ─► Does it span multiple subsystems or assert a multi-tenant
                   /multi-user invariant?
                   ├─ Yes ─► Layer 2 (Rust integration)
                   └─ No  ─► Unit test (in the owning module)
```

If a regression touches more than one box, write more than one test.
Layers complement, they do not substitute.

---

## Naming and tagging

So a Layer 1 replay test and a Layer 3 E2E scenario covering the same
behavior are discoverable as a pair:

- **Trace fixture path** mirrors the behavior family:
  `tests/fixtures/llm_traces/<family>/<scenario>.json`.
- **Rust replay test name** follows `<family>_<scenario>` (e.g.
  `tools::file_write_read`).
- **E2E scenario name** follows `test_<family>_<scenario>.py`.
- **Live canary lane** documents the exact replay fixture it would
  produce on success in `docs/internal/live-canary.md`.

When in doubt, grep for an existing scenario in the same family and
match its convention.

---

## Hermetic policy

Layers 1 and 2 are **hermetic by contract**:

- No outbound network. The mock servers (`mock_openai_server.rs`,
  `mock_mcp_server.rs`) are the only HTTP endpoints involved.
- No reliance on `~/.ironclaw/`. Tests use a tempdir-rooted home where
  filesystem state is needed.
- DB is per-test or per-module: in-memory libSQL by default, Postgres
  via `testcontainers` behind `--features integration`.
- Time may be paused (`#[tokio::test(start_paused = true)]`) where
  scheduler/heartbeat behavior is exercised.

If a Layer 1 or 2 test silently calls out to a real provider, that is a
bug — fix the test, do not normalize it.

Layers 3 and 4 are **not** hermetic by design:

- Layer 3 starts a real backend binary against a local stack.
- Layer 4 calls real providers and is gated to scheduled lanes.

Both should still avoid global side effects (no shared accounts, no
production tenants, no unbounded costs).

---

## Pointers

- Phase 1 tracker: [#2828](https://github.com/nearai/ironclaw/issues/2828)
- Live canary policy: `docs/internal/live-canary.md`
- E2E setup: `tests/e2e/CLAUDE.md`
- Live recording flow: `tests/support/LIVE_TESTING.md`
- Engine v2 eval (planned): [#2722](https://github.com/nearai/ironclaw/issues/2722)
