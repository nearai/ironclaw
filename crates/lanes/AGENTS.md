# `crates/lanes/` — execution mechanisms: the kernel decides, the lane executes

**Layer(s):** `runtimes` · **Crates:** 4 · **May depend on:** contracts
(`ironclaw_host_api`, `ironclaw_extension_contracts`) plus the sibling
`ironclaw_wasm_limiter` · **Depended on by:** the kernel's
`ironclaw_host_runtime` (the closed lane executor; the relationship inverts —
the kernel depends on the lanes, never the reverse), plus `ironclaw_hooks`
(loops) for the shared limiter and `composition`/`extension_host` for the MCP
lane's construction.

## What this family is

How an *already-authorized* invocation actually runs. A lane receives work only
after it has crossed the kernel membrane carrying a sealed `Authorized` witness
bound to one `RuntimeLane` variant; its job is to load or connect the selected
mechanism, run the request under mediated services, and hand back a normalized
outcome or a bounded, host-visible failure class. A lane never decides whether
work is allowed, never re-derives the lane choice, and can only refuse to run —
an unconfigured lane fails closed, never open.

## The crates

| Crate | Charter (one line) | Go here when |
| --- | --- | --- |
| [`ironclaw_mcp`](./ironclaw_mcp) | The MCP lane: JSON-RPC over host-mediated HTTP only — no HTTP client dependency in the crate at all | An MCP server's tools must execute behind the egress port |
| [`ironclaw_sandbox`](./ironclaw_sandbox) | The sandboxed-process lane: typed `SandboxProcessPlan` contract + Docker/broker/credential-firewall/CA machinery + the script backend; sole owner of the `bollard`/`rcgen`/`libc` cone | An OS process must run inside a validated, containerized plan |
| [`ironclaw_wasm`](./ironclaw_wasm) | The WASM component lane: deny-by-default host imports, fresh store per call, fuel/epoch/memory limits; owns the tool/channel ABI in its crate-local [`wit/`](./ironclaw_wasm/wit) | A WASM component must be loaded, metered, and executed |
| [`ironclaw_wasm_limiter`](./ironclaw_wasm_limiter) | The shared wasmtime `ResourceLimiter`, so the tool lane and the hook engine cannot silently diverge on limits | Any wasmtime host in the workspace needs resource ceilings |

**The lane contract** (the governing law of the family, from
`families/lanes.md`): accept only a canonical, already-authorized invocation;
use mediated services exclusively — scoped-down mounts, one-shot staged
secrets, policy-scoped egress arrive **by injection** from the kernel's
host-runtime layer, never as a lane-owned client or store; return a normalized
outcome or a bounded failure class; never run a parallel lifecycle. The lane
set is closed: `RuntimeLane` is an exhaustively-matched enum, and adding a lane
is a reviewed contract change, never a registry entry a crate adds on its own.

## What never belongs here

- **Authorization, approval, or trust-policy logic of any kind** →
  `crates/kernel/`; a lane executes what was decided, and can only narrow, not
  widen, what it was authorized for.
- **Ambient network or secret access** → every credential and egress path
  arrives kernel-injected; a lane-owned HTTP client is the defect
  `reborn_runtime_http_egress_has_single_network_boundary` exists to catch.
- **Budget authority** → lanes take the narrow
  `ironclaw_host_api::resource::RuntimeResourceBudget` port
  (reserve/reconcile/release), never `ResourceGovernor`; `ironclaw_resources`
  is a dev-dependency only in both lanes that use it (#7067).
- **Product behavior, presentation, or vendor/product names** → a lane executes
  "a WASM component" or "a sandboxed process", never "the GitHub tool" or
  "Slack"; what a package *is* belongs to `crates/extensions/`.
- **The extension registry crate** → lanes consume the neutral
  `ironclaw_extension_contracts` vocabulary instead (the WS3 carve-out).
- **A second, parallel lifecycle or supervisor** → process-lifecycle authority
  is the kernel's (`ironclaw_processes`).
- **Host-specific WASM machinery in the limiter** → store setup, loading, and
  bindings stay with each consumer; the limiter stays a zero-dependency leaf.

**Measured deviations from the family target, stated so nobody cites this file
as proof they are gone** (both measured 2026-08-05 via `cargo metadata` /
source):

- `ironclaw_sandbox` holds **normal deps on three substrate crates** —
  `ironclaw_network`, `ironclaw_safety`, `ironclaw_secrets` — carried in by the
  WS3 merge of the Docker/broker/CA machinery. `families/lanes.md`'s "a lane
  never adds a secrets, network, or filesystem crate as a dependency of its
  own" is the *target*; these edges are live and mechanically legal
  (runtimes > substrates in the layer ladder), and no gate forbids them today.
- `ironclaw_sandbox/src/script.rs` still shells out directly
  (`Command::new("docker")`) instead of routing through
  `SandboxCommandTransport` — carried as **Known debt** in
  [`ironclaw_sandbox/AGENTS.md`](./ironclaw_sandbox/AGENTS.md), open on
  CHECKLIST's WS3 sandbox row.

## The rules, and what enforces them

All in `crates/app/ironclaw_architecture_tests` unless noted; run
`cargo test -p ironclaw_architecture_tests`.

- **Layer matrix.** Every crate declares `layer = "runtimes"`;
  `reborn_workspace_crates_declare_layers_and_follow_layer_matrix`
  (`reborn_dependency_boundaries.rs`) enforces the ladder — no lane may name a
  kernel, loop, product, or app crate.
- **No direct networking in any lane.**
  `reborn_runtime_http_egress_has_single_network_boundary` scans the `src/`
  trees of `ironclaw_wasm`, `ironclaw_sandbox`, `ironclaw_mcp` (and
  `ironclaw_host_runtime`) for `reqwest::Client`, ad-hoc DNS
  (`to_socket_addrs`), and revived v1 SSRF helpers.
- **The limiter is a leaf, checked outbound-only.**
  `wasm_sandbox_core_module_stays_domain_free_v1_parity_kernel` asserts
  `ironclaw_wasm_limiter` has **zero workspace dependencies**, and pins the
  `wasm_sandbox_core` module's domain-free rule (including two literal phrases
  in `ironclaw_wasm/AGENTS.md` — edit that file with the gate in mind). No
  `BoundaryRule` names `ironclaw_wasm_limiter` as its `crate_name`: the
  limiter's inbound edges are governed by the layer ladder and the same-layer
  inventory alone.
- **The one runtimes→runtimes edge is pinned.** `wasm → wasm_limiter` is the
  family's only same-layer edge, inventoried in
  `reborn_same_layer_edge_inventory.rs` (owner `lanes/`, decided WS3); a new
  same-layer edge fails
  `reborn_every_same_layer_edge_is_inventoried_and_no_entry_is_stale`.
- **The MCP lane's failure-token charter is armed.**
  `crates/lanes/ironclaw_mcp/tests/module_charter.rs` (run
  `cargo test -p ironclaw_mcp`) fails any module outside `diagnostics` that
  mints a failure string, and requires the crate's `AGENTS.md` to keep naming
  the rule and the gate.
- **A family directory is never a compilation or trust unit.** The enforced
  truth is each crate's declared layer; family placement is ownership and
  discoverability only (PROPOSAL §5). Moving a crate between families is not a
  rename (§5.1).

**`wit/` lives inside `ironclaw_wasm`, and that is load-bearing.** The tool and
channel ABIs are crate assets (PROPOSAL §6.6.1), so this family directory holds
no non-crate directory of its own — which is what §11.2.1's no-stray-toplevel
check exists to require. The cost is paid by the guests instead: every
`wasm-src/` component reaches the ABI by relative path, so moving this crate
rewrites nine `wit-bindgen` `path:` args and forces a rebuild of the six
committed `.wasm` binaries (`scripts/ci/check-wasm-artifact-freshness.py` keys
each artifact to a digest of its whole `wasm-src/` tree and forbids re-recording
without rebuilding). That is why the move shipped as its own PR (WS7 2/2), and
why the next one should not be undertaken casually.

## Crossing out of this family

- **Down to `crates/contracts/`** — `ironclaw_host_api` for authority/resource
  vocabulary and the ports lanes implement (`SandboxCommandTransport`,
  `RuntimeResourceBudget`); `ironclaw_extension_contracts` for
  manifest-declared surface data.
- **Up to `crates/kernel/`** — never as a dependency; the kernel's host-runtime
  layer selects lanes through its closed lane executor and injects every
  mediated service. If a lane seems to need kernel state, the design answer is
  a narrower port in contracts, not an edge.
- **Sideways to `crates/extensions/`** — only in the sense of vocabulary:
  what a package declares (via `extension_contracts`), never the registry crate
  or vendor adapters.
- **Up to `crates/loop/`** — `ironclaw_hooks` consumes the limiter (legal:
  loops sit above runtimes); lanes never look back up.

## Sources

[`docs/reborn/target-architecture/families/lanes.md`](../../docs/reborn/target-architecture/families/lanes.md)
(full charter, boundaries, security posture) · PROPOSAL §6.6 (per-crate
contracts, incl. the WS3/WS7 amendments), §8 (dependency model), §11.2.6/§12
(driver and decision logs) · the gates named above.
