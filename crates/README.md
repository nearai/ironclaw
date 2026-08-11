# IronClaw crates

`crates/` is the whole IronClaw Reborn production stack: one Cargo workspace
arranged as **ten family directories**, each owning one kind of
responsibility. There is no legacy tier — the v1 monolith and its crates are
gone.

This page is the human map. For agent routing and the enforced dependency
matrix use [`AGENTS.md`](./AGENTS.md); for the runtime narrative (how a turn
actually executes) read [`Architecture.md`](./Architecture.md); for the
design record — per-family specs, the frozen proposal, the live checklist —
read
[`docs/reborn/target-architecture/`](../docs/reborn/target-architecture/README.md).

## Mental model

Authority is narrow and explicit, and the tree mirrors it:

1. **`contracts/` describe** — shared vocabulary and ports every tier may
   see. Nothing here executes, persists, or names a vendor.
2. **`kernel/` decides** — the authority perimeter: trust, authorization,
   approvals, resources, runtime policy, the capability membrane, process
   lifecycle, turn admission, and the mediated host services.
3. **`substrates/`, `events/`, `domains/` hold mechanism and state** —
   privileged mechanisms (files, secrets, network), append-only evidence and
   read models, and typed record owners. All of it is mediated by the kernel.
4. **`lanes/` execute** — already-authorized work runs in isolation (WASM,
   MCP, the container sandbox) and returns normalized outcomes.
5. **`loop/` decides agent behavior** — deliberately untrusted; it can only
   *request* effects through typed ports.
6. **`extensions/` package integrations** — manifests, the generic host, and
   one self-contained directory per installable package.
7. **`product/` owns the experience** — the assistant, the operator control
   plane, the WebUI, the OpenAI-compatible API.
8. **`app/` assembles and enforces** — composition wires it, the `ironclaw`
   binary boots it, the architecture tests keep the rules mechanical.

One request stitches them together: the **loop** chooses a tool, the
**kernel** permits it, a **lane** runs it, and **domains** and **events**
remember it. If a change adds authority or persistence, put it in the family
that owns that boundary — never thread it through a UI or runtime crate.

## The ten families

Counts measured 2026-08-05 (`cargo metadata --no-deps`; the enforcing gate is
`python3 scripts/ci/check-target-tree.py`). Every family has an `AGENTS.md`
(its boundary and crate table) and an in-depth spec in
[`docs/reborn/target-architecture/families/`](../docs/reborn/target-architecture/families/);
every crate has a `README.md`.

| Directory | Crates | What lives there |
| --- | --- | --- |
| [`contracts/`](./contracts/AGENTS.md) | 6 | `host_api`, `common`, `prompt_envelope`, `loop_contracts`, `extension_contracts`, `product_contracts` |
| [`substrates/`](./substrates/AGENTS.md) | 6 | `filesystem`, `libsql_runtime`, `secrets`, `network`, `safety`, `observability` |
| [`events/`](./events/AGENTS.md) | 4 | `event_log`, `event_store`, `event_projections`, `event_streams` |
| [`domains/`](./domains/AGENTS.md) | 12 | `threads`, `conversations`, `triggers`, `memory`, `skills`, `auth`, `attachments`, `extractors`, `identity`, `llm`, `trace_commons`, `outbound` |
| [`kernel/`](./kernel/AGENTS.md) | 9 | `trust`, `authorization`, `approvals`, `resources`, `runtime_policy`, `capabilities`, `processes`, `turns`, `host_runtime` |
| [`lanes/`](./lanes/AGENTS.md) | 4 | `wasm`, `wasm_limiter`, `mcp`, `sandbox` |
| [`loop/`](./loop/AGENTS.md) | 4 | `agent_loop`, `loop_host`, `turn_runner`, `hooks` |
| [`extensions/`](./extensions/AGENTS.md) | 8 | `extension_registry`, `extension_host`, `extension_manager`, `extension_support`, plus the 4 crates under `packages/` (below) |
| [`product/`](./product/AGENTS.md) | 5 | `assistant`, `operator`, `openai_compat`, `webui`, `host_ingress` |
| [`app/`](./app/AGENTS.md) | 4 | `composition`, `cli` (package `ironclaw` — the binary), `config`, `architecture_tests` |

Crate directories carry the full package name
(`crates/kernel/ironclaw_turns` → package `ironclaw_turns`), so moving a
crate between families is never a rename. The exceptions are
`app/ironclaw_cli` (package `ironclaw`) and the extension packages below.

### Extension packages

`crates/extensions/packages/` holds 14 self-contained package directories —
the only place in the workspace where vendor names may appear. Four are
workspace crates: `slack` and `telegram` (channel adapters),
`memory-native` and `mem0` (memory providers). Ten are data-only packages
(manifest + prompts/schemas, some with prebuilt WASM): `github`, `gmail`,
`google-calendar`, `google-docs`, `google-drive`, `google-sheets`,
`google-slides`, `nearai-mcp`, `notion-mcp`, `web-access`. Each package
directory has its own `README.md`, crate or not.

### The workspace beyond `crates/`

62 of the workspace's **64 packages** live under `crates/`. The other two are
the root package `ironclaw_integration_tests` (the in-process integration
suite driving `tests/integration/`) and `tools/ironclaw_stress`. One package
is deliberately excluded from the workspace: `tools/ironclaw_silk_decoder`,
a standalone helper with its own toolchain requirements.

Family directories are ownership, not compile-time trust units — the
enforced dependency ladder is the seven-layer matrix in
[`AGENTS.md`](./AGENTS.md).

## Where to make a change

Pick the family with the table above, then follow the read order in
[`AGENTS.md`](./AGENTS.md): family `AGENTS.md` → crate `README.md` → crate
working rules / module spec → `docs/reborn/contracts/`. Two program-wide
rules to know before starting:

- Product handlers, channels, scheduled triggers, and agent callers go
  through `ProductSurface` and the capability contracts — never around
  authorization, approvals, or the owning domain operation (root `AGENTS.md`,
  "Security and runtime invariants").
- LLM data is never deleted. Durable state goes through the `RootFilesystem`
  mount catalog; in-memory maps are caches, never the source of truth (root
  `AGENTS.md`).

## Quick commands

From the repository root:

```bash
cargo test -p <crate_name>                  # narrowest first
cargo test -p ironclaw_architecture_tests   # dependency/layer gates
python3 scripts/ci/check-target-tree.py     # tree vs documented package set
```

The full gate (`cargo fmt`, workspace clippy, workspace tests) is documented
in the root `AGENTS.md`. Some crates test backends conditionally (PostgreSQL,
libSQL, WASM) — read the crate's `README.md` and module spec before assuming
a command covers them.
