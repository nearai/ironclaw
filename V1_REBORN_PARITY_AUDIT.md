# V1 IronClaw vs Reborn Feature Parity Audit

Date: 2026-06-07

## Scope

This audit compares the current root `ironclaw` binary against the standalone
`ironclaw-reborn` binary and Reborn runtime/composition crates.

This version intentionally excludes external reference implementation parity.
The question here is narrower: what must Reborn gain before it can replace V1
IronClaw for real users and operators?

## Evidence Inspected

- V1 entry and composition:
  - `src/main.rs`
  - `src/app.rs`
  - `src/cli/mod.rs`
  - `src/config/channels.rs`
  - `src/channels/web/`
- V1 command implementations:
  - `src/cli/config.rs`
  - `src/cli/routines.rs`
  - `src/cli/memory.rs`
  - `src/cli/mcp.rs`
  - `src/cli/pairing.rs`
  - `src/cli/service.rs`
  - `src/cli/doctor.rs`
  - `src/cli/logs.rs`
  - `src/cli/models.rs`
  - `src/cli/acp.rs`
- Reborn entry and CLI:
  - `crates/ironclaw_reborn_cli/AGENTS.md`
  - `crates/ironclaw_reborn_cli/Cargo.toml`
  - `crates/ironclaw_reborn_cli/src/cli.rs`
  - `crates/ironclaw_reborn_cli/src/commands/*`
  - `crates/ironclaw_reborn_cli/src/runtime/mod.rs`
- Reborn runtime and composition:
  - `crates/ironclaw_reborn/CLAUDE.md`
  - `crates/ironclaw_reborn/src/runtime.rs`
  - `crates/ironclaw_reborn_composition/CLAUDE.md`
  - `crates/ironclaw_reborn_composition/src/runtime.rs`
  - `crates/ironclaw_reborn_composition/src/factory.rs`
  - `crates/ironclaw_reborn_config/src/config_file.rs`
- Test evidence:
  - `tests/reborn_*`
  - `crates/ironclaw_reborn_cli/tests/smoke.rs`
  - `crates/ironclaw_reborn_composition/tests/*`

## Executive Summary

V1 is still the broad operational product. It owns first-run setup, full CLI
administration, the mature web gateway, channel activation, routines, memory
commands, pairing, MCP/ACP management, service management, import, and most
operator diagnostics.

Reborn is not just a stub. It has a real composed runtime, planned-driver
integration, a narrow `RebornRuntime` facade, Product Workflow contracts,
HostRuntime capabilities, scoped identity/tenant/project architecture,
ProductAuth, WebUI v2 beta, extension lifecycle facades, trace contribution
tools, provider-admin model commands, local-dev persistence, trigger-poller
work, Slack host-beta, hosted MCP/Notion, web access, and GSuite packages.

The main parity problem is not the loop core. The main gap is product control
surface: setup, status, diagnostics, logs, channel management, memory,
routines, service lifecycle, and migration. Reborn can execute a turn, but V1
can operate an assistant installation.

The intended Reborn direction should be API-first: WebUI v2 APIs, backed by
Reborn/Product Workflow facades, should be the canonical command interfaces.
The standalone CLI should not grow into a second primary command plane. It
should stay thin: bootstrap, smoke tests, local debugging, service management,
and wrappers over the same typed APIs when a terminal workflow is genuinely
useful.

Recommended strategy:

1. Keep Reborn's architecture boundaries intact.
2. Build Reborn-native WebUI v2/API facades for the V1 operator workflows.
3. Add migration/import explicitly rather than letting Reborn read V1 state.
4. Retire V1 surfaces only after caller-level parity tests exist.

## Replacement Readiness Snapshot

| Area | V1 status | Reborn status | Replacement readiness |
|---|---|---|---|
| Basic chat/run loop | Mature agent stack via `ironclaw run` and default no-command path | Real `ironclaw-reborn run` and `repl` over composed runtime | Partial. Reborn can run, but production profile and operational wrapping are incomplete. |
| First-run setup | `onboard`, quick mode, step-scoped setup | No equivalent; only `config init` stub generation | Not ready. |
| Config/control plane | CLI config plus gateway-backed settings paths | `config path/init`; model commands patch LLM slot; WebUI v2 API is the intended canonical command surface | Partial. Safe, but missing the API breadth to replace V1 administration. |
| Diagnostics | Broad `doctor`, `status`, gateway logs | `doctor` is shallow; `logs` is explicitly `not-wired`; no full API/CLI status | Not ready. |
| Web gateway/control API | Mature web gateway/control UI in `src/channels/web` | WebUI v2 behind `webui-v2-beta`; off by default | Partial. Strong middleware and Product Workflow design, incomplete API surface. |
| Channels | CLI/TUI, HTTP webhook, Signal, Gateway/WebChat, WASM channels and channel sources | Channel listing, WebUI v2, Slack host-beta, product adapters in progress | Not ready for broad replacement. |
| Routines/cron | CLI CRUD/history plus runtime scheduling and heartbeat | Trigger persistence/poller work in progress; no WebUI v2/API administration parity | Not ready. |
| Memory | CLI search/read/write/tree/status over workspace | Local-dev roots/mounts and scoped file capabilities; no memory control API parity | Not ready. |
| Pairing | `pairing list/approve` plus host APIs | Product actor binding architecture exists; no generic API/admin surface | Partial substrate, missing operator surface. |
| MCP/ACP | Generic MCP management and ACP agent management | Hosted MCP packages exist; no generic API/admin surface | Not ready. |
| Tools/extensions | WASM `tool`, registry, extension manager | Reborn extension lifecycle command, first-party packages, ProductAuth | Partial. Cleaner architecture, narrower install/update/admin surface. |
| Service lifecycle | launchd/systemd management via `service` | No Reborn service command | Not ready. |
| Import/migration | Feature-gated import path and tests | No Reborn import/migration command | Not ready. |
| Security architecture | Mature V1 auth/approval/secrets/sandbox pieces | Stronger scoped identity, WebUI v2 middleware, ProductAuth fail-closed patterns | Reborn is ahead architecturally, but missing some operator flows. |

## V1 Surface Deep Dive

### Entry Point and Runtime Shape

V1 starts in `src/main.rs`.

The main function:

- loads `.env`;
- loads IronClaw env through `bootstrap::load_ironclaw_env`;
- parses `Cli`;
- dispatches many non-agent commands before full startup;
- otherwise builds config, database, secrets, LLM, tools, hooks, workspace,
  channels, gateway, sandbox, and `Agent` dependencies.

V1's composition root is `AppBuilder` in `src/app.rs`. It wires:

- database backends and migrations;
- DB-backed config reload;
- secrets store;
- LLM and reload handles;
- safety layer;
- tool registry and WASM runtime;
- embeddings and workspace;
- extension manager;
- MCP session/process managers;
- hooks;
- cost guard;
- session manager;
- ownership cache;
- gateway log broadcaster.

Replacement implication: Reborn does not need to copy this shape, but every
user-facing capability that depends on these components needs a Reborn-owned
equivalent facade or an explicit migration boundary.

### V1 CLI Breadth

The root `ironclaw` command has a broad operator surface:

- `run`
- `onboard`
- `config`
- `tool`
- `registry`
- `channels`
- `routines` / `cron`
- `mcp`
- `memory`
- `pairing`
- `profile`
- `service`
- `skills`
- `hooks`
- `models`
- `doctor`
- `logs`
- `status`
- `completion`
- feature-gated `import`
- `login`
- hidden `worker`, `claude-bridge`, `acp-bridge`
- `acp`

Several of these commands intentionally avoid starting the full agent and run
as direct administrative commands. That matters for Reborn: replacement parity
is not just "can the model answer a message." Operators need out-of-band
inspection and repair commands.

### V1 Setup and Config

V1 setup is centered on `onboard`, with:

- full interactive wizard;
- quick provider/model setup;
- step-scoped reconfiguration;
- setup hints from top-level error formatting;
- DB and TOML/env layering;
- DB-backed settings migration and reload in `AppBuilder`.

V1 config commands are broad enough for day-to-day support:

- list settings;
- filter settings;
- get one key;
- set one key;
- generate config;
- show config path.

Replacement implication: Reborn `config init` is useful but not enough. Reborn
needs first-run and repair UX before it can become the operator binary.

### V1 Routines and Background Work

V1 `src/cli/routines.rs` exposes:

- `list`;
- `create`;
- `edit`;
- `enable`;
- `disable`;
- `delete`;
- `history`.

It uses the shared DB trait and V1 routine domain types. The runtime also owns
heartbeat and event-driven routines.

Replacement implication: Reborn's trigger substrate must grow operator-facing
CRUD/history and result delivery before V1 routines can be retired.

### V1 Memory

V1 memory CLI exposes:

- `search`;
- `read`;
- `write`;
- `tree`;
- `status`.

It is backed by the workspace abstraction, supports DB-backed memory, and can
use embeddings/cache config. This is a high-trust user data surface.

Replacement implication: Reborn needs a memory control API and migration path
early. A CLI wrapper is useful, but should not be the canonical interface.
Users will treat hidden or inaccessible memory as data loss even if the runtime
can technically access a local-dev filesystem.

### V1 Channels and Gateway

V1 channels and gateway are broad:

- CLI/TUI;
- HTTP webhook;
- Signal;
- web gateway/WebChat;
- WASM channel router/runtime;
- extension manager activation;
- startup active channel handling;
- v1/v2 Telegram exclusivity guard in config.

The web gateway in `src/channels/web` owns chat, auth callbacks, SSE/WS,
history, logs, onboarding state, extension setup, and other product UI paths.

Replacement implication: Reborn WebUI v2 and product adapters must become the
primary route for channel setup, inbound turns, outbound delivery, pairing, and
status. Bridging by importing V1 web code into Reborn would violate Reborn
composition guidance.

### V1 MCP, ACP, Tools, and Extensions

V1 includes generic MCP server management:

- add;
- remove;
- list;
- auth;
- test;
- toggle.

It also includes ACP agent management and WASM tool/registry commands. These
are operationally important because users can add capabilities without a code
change.

Replacement implication: Reborn has first-party packages and hosted MCP, but
it is not yet a generic user-managed capability host through its canonical
control plane.

## API-First Reborn Command Interface

The stronger Reborn direction is to treat WebUI v2 APIs as the canonical
command interface for product administration and user workflows.

That means Reborn parity should not be measured as "add every V1 CLI
subcommand to `ironclaw-reborn`." The V1 CLI is evidence of required
capabilities, not necessarily the interface shape Reborn should copy.

Recommended model:

1. Product Workflow and Reborn services own the domain behavior.
2. WebUI v2 descriptors expose stable HTTP/SSE/WS control-plane APIs over
   those services.
3. Browser UI, local CLI, service wrappers, and tests call the same APIs or
   the same typed facades behind those APIs.
4. CLI commands remain thin conveniences for bootstrapping, smoke testing,
   automation, and local operator workflows.

This is a good idea if the WebUI v2 APIs are designed as product/control-plane
contracts, not as browser implementation details. The API layer should be
stable, typed, descriptor-backed, auth/rate/body-limit aware, and backed by
Product Workflow facades. In that form, it prevents Reborn from growing two
parallel command planes.

This is a bad idea if "WebUI v2 API" comes to mean "put command behavior in
route handlers." Route handlers should parse/authorize/limit/adapt requests,
then call canonical services. They should not own business logic, state
machines, migration rules, setup flows, or channel-specific policy.

Thermo-nuclear maintainability verdict:

- **Approve the API-first direction.** It is the cleaner architecture and it
  aligns with Reborn's facade/product-boundary model.
- **Do not approve a browser-route-first implementation.** That would recreate
  V1 drift in a different layer and make the codebase more tangled.
- **Require shared typed contracts.** If CLI and WebUI need the same operation,
  the operation belongs in a Reborn service/Product Workflow facade with the
  WebUI descriptor and CLI wrapper both calling it.
- **Keep the CLI small by policy.** A growing `ironclaw-reborn` CLI should be
  treated as a smell unless the command is bootstrap-only, service lifecycle,
  script automation, or a thin client over the canonical control-plane API.

## Reborn Surface Deep Dive

### Reborn CLI Shape

`ironclaw-reborn` is intentionally narrow:

- `channels`
- `completion`
- `config`
- `doctor`
- `extension`
- `hooks`
- `logs`
- `models`
- `profile`
- `repl`
- `run`
- feature-gated `serve`
- `skills`
- `traces`

This command surface follows `crates/ironclaw_reborn_cli/AGENTS.md`:

- commands live one file per module;
- commands use `RebornCliContext` for boot config;
- state must live under `IRONCLAW_REBORN_HOME` / `~/.ironclaw/reborn`;
- the CLI must not depend on root `ironclaw` or `src/*` runtime modules;
- provider/model UX must enter through the Reborn composition provider-admin
  facade.

This boundary is correct and should stay. The work is to add missing surfaces
through Reborn facades, not through V1 imports.

### Reborn Run and REPL

`ironclaw-reborn run` is real.

The command:

- resolves `RebornCliContext`;
- builds `RebornRuntimeInput`;
- builds `RebornRuntime` through `build_reborn_runtime`;
- creates a conversation;
- sends one message or runs a stdin REPL;
- prints assistant reply text;
- shuts down runtime.

The REPL currently supports only:

- normal message lines;
- `/help`;
- `/exit`;
- `/quit`.

Replacement implication: the basic turn path exists, but it should not become
the main product command surface. Reborn run is a runtime entrance and smoke
path. The durable administration surface should be WebUI v2 APIs over Reborn
services, with CLI wrappers only where a terminal path is useful.

### Reborn Runtime Composition

Reborn composition is cleaner than V1:

- `ironclaw_reborn` owns driver-side loop integration.
- `ironclaw_reborn_composition` owns top-level Reborn startup composition.
- Downstream callers should use facade-shaped handles only.
- Product traffic should enter through explicit Reborn adapters.
- Product auth should use `ironclaw_auth` ports, not V1 OAuth/pending maps.
- WebUI v2 is a native Reborn surface entering Product Workflow directly.

The main user-facing Reborn runtime handle is `RebornRuntime`, with task-level
methods like:

- `new_conversation`;
- `send_user_message`;
- `send_user_message_with_cancellation`;
- `shutdown`.

Replacement implication: this facade is the right target for new product
control APIs and WebUI/channel adapters. Missing operator surfaces should call
this or adjacent Reborn facades, not lower substrate handles.

### Reborn Config and State

Reborn config is in `crates/ironclaw_reborn_config`.

The operator file is `$IRONCLAW_REBORN_HOME/config.toml`. The model is:

- provider catalog in `providers.json`;
- selection in `config.toml`;
- runtime config resolved by composition;
- precedence: compiled defaults < file < env vars < CLI flags;
- secrets are env-only and inline secret-shaped values are rejected at parse
  time.

Parsed sections include:

- `[boot]`;
- `[identity]`;
- `[policy]`;
- `[drivers]`;
- `[harness]`;
- `[runner]`;
- `[skills]`;
- `[llm.<slot>]`;
- `[webui]`;
- `[slack]`;
- `[budget]`;
- `[trigger_poller]`.

But `ironclaw-reborn run` currently rejects some parsed sections as not wired:

- `[policy]`;
- `[drivers]`;
- `[harness]`;
- `[identity].default_project` for `run`/`repl`.

The runtime builder currently supports `local-dev` and `local-dev-yolo` through
`local_runtime_build_input_with_options`; production wiring is explicitly a
follow-up.

Replacement implication: Reborn config is safer and more explicit than V1 in
some ways, but replacement requires production profile wiring and full
selection of policy/driver/harness.

### Reborn WebUI v2

Reborn WebUI v2 is behind the `webui-v2-beta` Cargo feature. By default,
`serve` is not compiled into the binary.

When compiled, WebUI v2 is strong architecturally:

- descriptor-driven body limits;
- descriptor-driven rate limits;
- bearer auth;
- restricted `?token=` shim only for EventSource;
- same-origin WebSocket enforcement;
- host-supplied public route mount for SSO;
- Reborn ProductAuth routes;
- extension pairing route support for Slack host-beta;
- Slack route management when configured.

Replacement implication: WebUI v2 APIs should become the replacement command
plane for V1 gateway and V1 administrative commands. The UI needs views, but
the more important parity layer is the underlying API breadth: routines,
memory, channel status, logs, setup, pairing, extension lifecycle, model
settings, blocked turn/gate UX, and durable event fanout.

### Reborn ProductAuth and Extensions

Reborn ProductAuth is a major improvement over V1 route-local patterns.

Current guardrails require:

- product auth through `ironclaw_auth` ports;
- no V1 OAuth routes;
- no V1 pending maps;
- no V1 `ExtensionManager`;
- no route-local raw HTTP clients;
- raw token values must not go through chat commands, model-visible messages,
  serializable DTOs, projections, or route-local pending maps.

Reborn extension lifecycle exists through an `extension` CLI command with:

- search;
- install;
- activate;
- remove.

Replacement implication: ProductAuth and extension lifecycle should be the
center of Reborn channel/provider setup. The missing part is API/operator
breadth, not the core architecture.

### Reborn Diagnostics

Reborn diagnostics are currently shallow:

- `doctor` prints home, source, profile, v1_state, and driver registry.
- `logs` prints `entries: 0`, `status: not-wired`, and `v1_state: not-used`.
- no Reborn `status` command exists.

Replacement implication: this is the clearest P0 gap. Operators need
subsystem status before trusting Reborn. The canonical surface should be a
status/diagnostics API, with `ironclaw-reborn status/doctor/logs` as thin
clients if kept.

### Reborn Traces

Reborn has a substantial trace contribution CLI:

- opt in;
- opt out;
- status;
- preview;
- enqueue;
- flush queue;
- queue status;
- credit;
- submit;
- list submissions;
- revoke;
- ingest health.

Replacement implication: this is a Reborn-native capability with more surface
area than many replacement basics. It should remain, but parity sequencing
should not let trace tooling outrun core operator workflows.

## Detailed Gap Inventory

### P0 Gaps: Must Close Before Reborn Can Replace V1

#### Production Runtime Profile

Observed:

- Reborn config parses production-oriented fields.
- `run` builds local runtime input.
- unsupported `[policy]`, `[drivers]`, `[harness]`, and some identity fields
  fail loud.

Missing:

- production `RebornCompositionProfile` path for `run`;
- durable storage requirements for production;
- production readiness validation exposed to CLI;
- driver selection from config;
- approval policy selection from config;
- explicit migration-dry-run behavior.

Why P0:

Reborn cannot be the main binary while the only fully wired run modes are
local-dev oriented.

#### Diagnostics, Logs, and Status APIs

Observed:

- V1 has `doctor`, `logs`, and `status`.
- Reborn `doctor` is a boot-config check.
- Reborn `logs` is hardcoded `not-wired`.
- Reborn has no canonical status API or `status` wrapper.

Missing:

- Reborn subsystem probes;
- log source and tail/follow API;
- active run/turn worker status;
- provider/model status;
- storage/secrets status;
- WebUI/trigger/channel/extension status;
- readiness classification.

Why P0:

Operators cannot debug or support a Reborn installation without this.

#### First-Run Setup API

Observed:

- V1 `onboard` is the recommended first command.
- Reborn `config init` writes editable stubs.

Missing:

- provider/model setup API and optional interactive client;
- API-key-env setup guidance and validation;
- profile selection;
- local-dev-yolo disclosure flow outside `run`;
- WebUI token/user setup;
- channel setup entry points;
- repair-oriented setup.

Why P0:

Without setup, Reborn is usable mainly by developers who already understand
the config model.

#### Config Administration API

Observed:

- V1 supports list/get/set/init/path.
- Reborn supports path/init, and model commands patch the default LLM slot.

Missing:

- list effective config via API;
- get one key via API;
- set one key via API;
- validate config without starting runtime;
- explain precedence;
- show unsupported parsed fields with remediation;
- safe config mutation for webui/slack/trigger/budget sections.

Why P0:

Operators need to inspect and repair config without manually editing TOML.
CLI support can wrap the API, but the API should be canonical.

### P1 Gaps: Needed for Daily Use

#### WebUI v2 Replacement Surface

Observed:

- V1 gateway is broad.
- Reborn WebUI v2 is secure and descriptor-driven but beta-gated.

Missing:

- default/release decision for `serve`;
- complete chat/history API parity;
- settings/config API;
- memory API;
- routines/jobs API;
- channel status/setup API;
- logs/diagnostics API;
- extension lifecycle API;
- pairing API;
- model provider API;
- durable event replay/fanout beyond local-dev follow-ups.

#### Routines and Triggers

Observed:

- V1 routines CLI has CRUD/history.
- Reborn has trigger repository/poller work and first-party trigger
  capabilities.

Missing:

- Reborn routine/trigger CRUD/history API;
- WebUI routine management as a client of that API;
- external result delivery;
- failure history;
- trigger ownership and retention/tombstone operator visibility;
- migration from V1 routines.

#### Memory and Workspace

Observed:

- V1 memory CLI is broad.
- Reborn local-dev filesystems and host file capabilities exist.

Missing:

- memory search/read/write/tree/status API;
- memory import from V1;
- visible identity/system prompt mapping;
- search/index status;
- caller-level tests through Reborn memory surfaces.

#### Pairing and Actor Binding

Observed:

- V1 has pairing CLI.
- Reborn has Product Workflow binding and Slack personal binding pieces.

Missing:

- generic Reborn pairing list/approve/revoke API;
- channel/product actor binding inspection;
- WebUI pairing flow;
- migration from V1 pairing store where appropriate.

#### Channel Replacement

Observed:

- V1 owns broad channel startup and WASM channel activation.
- Reborn has product adapter direction and Slack host-beta.

Missing:

- HTTP webhook replacement;
- Signal replacement;
- Telegram v2 replacement path;
- Discord replacement path;
- channel enable/disable/status APIs;
- common inbound/outbound delivery status;
- channel health/reconnect model.

### P2 Gaps: Needed for Full Operational Parity

#### Generic MCP and ACP Management

Observed:

- V1 has generic MCP and ACP CLIs.
- Reborn has hosted MCP packages and first-party extension work.

Missing:

- generic MCP add/remove/list/auth/test/toggle APIs;
- ACP agent add/remove/list/toggle/test APIs or an explicit replacement decision;
- migration from V1 MCP/ACP configs.

#### Tool and Registry Administration

Observed:

- V1 has `tool` and `registry`.
- Reborn has `extension search/install/activate/remove`.

Missing:

- equivalent WASM tool management;
- installed package index and update semantics;
- registry repair/status;
- conflict-aware uninstall/cleanup;
- clearer mapping between V1 tools and Reborn extensions/capabilities.

#### Service Lifecycle

Observed:

- V1 has OS service management.
- Reborn has no service command.

Missing:

- launchd/systemd install/start/stop/status for `ironclaw-reborn`;
- service config path and env handling;
- log integration with Reborn logs/status.

#### Import and Migration

Observed:

- V1 has feature-gated import code and tests.
- Reborn has no import command.
- Reborn intentionally does not use V1 state.

Missing:

- V1-to-Reborn migration plan;
- V1 workspace memory migration;
- V1 routines migration;
- V1 channel/pairing migration;
- V1 config/provider migration;
- idempotency and dry-run reports.

### P3 Gaps: Useful but Not Replacement Blockers

- richer REPL commands beyond `/help`, `/exit`, `/quit`;
- advanced trace UX integration with WebUI/status;
- non-core channel breadth after the main deployed channels are ported;
- advanced media/voice/multimodal work;
- deep plugin ecosystem parity;
- extensive Control UI polish.

## Existing GitHub Issue Coverage

Checked open `nearai/ironclaw` issues on 2026-06-08 with `gh issue list`.
Most parity gaps already have at least umbrella coverage. The main risk is not
missing issue creation; it is that several issues are broad enough that the
operator API acceptance criteria from this audit could be lost unless copied
into the issue bodies or split into sub-issues.

| Audit gap | Existing issue coverage | Coverage assessment |
|---|---|---|
| Reborn cutover contract and product-surface migration | [#3031](https://github.com/nearai/ironclaw/issues/3031), [#2987](https://github.com/nearai/ironclaw/issues/2987), [#3020](https://github.com/nearai/ironclaw/issues/3020), [#3029](https://github.com/nearai/ironclaw/issues/3029) | Direct umbrella coverage. Use this audit as the replacement checklist input. |
| Production runtime profile and composition | [#3026](https://github.com/nearai/ironclaw/issues/3026), [#3333](https://github.com/nearai/ironclaw/issues/3333), [#3602](https://github.com/nearai/ironclaw/issues/3602), [#3045](https://github.com/nearai/ironclaw/issues/3045), [#3087](https://github.com/nearai/ironclaw/issues/3087) | Direct coverage. This maps to the audit's first P0. |
| API-first Product Workflow / WebUI v2 command plane | [#3280](https://github.com/nearai/ironclaw/issues/3280), [#4488](https://github.com/nearai/ironclaw/issues/4488), [#4483](https://github.com/nearai/ironclaw/issues/4483), [#3953](https://github.com/nearai/ironclaw/issues/3953) | Direct architectural coverage. The audit adds the policy that route handlers must stay adapters over shared facades. |
| WebUI v2 beta readiness, auth, and acceptance tests | [#3607](https://github.com/nearai/ironclaw/issues/3607), [#3613](https://github.com/nearai/ironclaw/issues/3613), [#3615](https://github.com/nearai/ironclaw/issues/3615), [#3608](https://github.com/nearai/ironclaw/issues/3608), [#3609](https://github.com/nearai/ironclaw/issues/3609), [#3809](https://github.com/nearai/ironclaw/issues/3809) | Direct coverage for beta hardening. API breadth for setup/config/logs/status still needs explicit acceptance criteria. |
| CLI/TUI/setup migration onto Reborn services | [#3284](https://github.com/nearai/ironclaw/issues/3284), [#4118](https://github.com/nearai/ironclaw/issues/4118) | Partial direct coverage. Needs an API-first reading: CLI wrappers should call typed services/control-plane APIs. |
| First-run setup and config administration | [#3284](https://github.com/nearai/ironclaw/issues/3284), [#4118](https://github.com/nearai/ironclaw/issues/4118), [#3036](https://github.com/nearai/ironclaw/issues/3036), [#3045](https://github.com/nearai/ironclaw/issues/3045) | Broad coverage, but no narrow issue found for the exact WebUI v2 setup/config list/get/set/validate API parity. Consider a sub-issue. |
| Diagnostics, logs, and status APIs | [#3602](https://github.com/nearai/ironclaw/issues/3602), [#4427](https://github.com/nearai/ironclaw/issues/4427), [#4353](https://github.com/nearai/ironclaw/issues/4353), [#83](https://github.com/nearai/ironclaw/issues/83) | Partial coverage. `doctor`/`logs`/`status` API parity appears under readiness and CLI expansion, but a dedicated Reborn operator diagnostics API issue would reduce ambiguity. |
| Memory and workspace product surface | [#3287](https://github.com/nearai/ironclaw/issues/3287), [#3537](https://github.com/nearai/ironclaw/issues/3537), [#87](https://github.com/nearai/ironclaw/issues/87), [#1782](https://github.com/nearai/ironclaw/issues/1782) | Direct coverage for migration/product surface. Add explicit search/read/write/tree/status and V1 import criteria to avoid under-scoping. |
| Routines, triggers, missions, jobs | [#3290](https://github.com/nearai/ironclaw/issues/3290), [#3873](https://github.com/nearai/ironclaw/issues/3873), [#4475](https://github.com/nearai/ironclaw/issues/4475), [#4432](https://github.com/nearai/ironclaw/issues/4432), [#4439](https://github.com/nearai/ironclaw/issues/4439) | Direct coverage. The missing piece is a crisp CRUD/history/failure-delivery API checklist. |
| Channel replacement and ProductAdapter migration | [#3577](https://github.com/nearai/ironclaw/issues/3577), [#3285](https://github.com/nearai/ironclaw/issues/3285), [#3581](https://github.com/nearai/ironclaw/issues/3581), [#3582](https://github.com/nearai/ironclaw/issues/3582), [#3616](https://github.com/nearai/ironclaw/issues/3616), [#4491](https://github.com/nearai/ironclaw/issues/4491) | Direct coverage. Add channel status/setup/health/reconnect API requirements to the relevant channel issues. |
| ProductAuth, SSO, and credential surfaces | [#4175](https://github.com/nearai/ironclaw/issues/4175), [#4116](https://github.com/nearai/ironclaw/issues/4116), [#4204](https://github.com/nearai/ironclaw/issues/4204), [#4180](https://github.com/nearai/ironclaw/issues/4180), [#4181](https://github.com/nearai/ironclaw/issues/4181), [#4382](https://github.com/nearai/ironclaw/issues/4382), closed [#4201](https://github.com/nearai/ironclaw/issues/4201) | Mostly direct coverage. #4201 is complete for product-facing auth HTTP routes. Remaining ProductAuth work should stay focused on production backend parity, OAuth PKCE HA safety, SSO follow-ups, and default credential account UX. |
| Generic MCP, ACP, WASM tools, skills, extensions | [#3288](https://github.com/nearai/ironclaw/issues/3288), [#4176](https://github.com/nearai/ironclaw/issues/4176), [#2246](https://github.com/nearai/ironclaw/issues/2246), [#3905](https://github.com/nearai/ironclaw/issues/3905), [#3283](https://github.com/nearai/ironclaw/issues/3283) | Direct coverage. #3288 should be read narrowly as production/scoped capability lifecycle admin parity. A scoped lifecycle record/store foundation now exists for admin-shared plus user-private package resolution; WebUI/API wiring, adapter activation, and ACP-specific replacement remain follow-ups. |
| OpenAI-compatible API ingress and streaming | [#3283](https://github.com/nearai/ironclaw/issues/3283), [#4442](https://github.com/nearai/ironclaw/issues/4442), [#4443](https://github.com/nearai/ironclaw/issues/4443), [#4444](https://github.com/nearai/ironclaw/issues/4444), [#4445](https://github.com/nearai/ironclaw/issues/4445), [#4446](https://github.com/nearai/ironclaw/issues/4446), [#4447](https://github.com/nearai/ironclaw/issues/4447) | Direct coverage. This is adjacent to V1 parity but important for Reborn product API consistency. |
| Service lifecycle | [#83](https://github.com/nearai/ironclaw/issues/83), [#89](https://github.com/nearai/ironclaw/issues/89), [#3284](https://github.com/nearai/ironclaw/issues/3284) | Weak coverage. No narrow Reborn launchd/systemd install/start/stop/status parity issue was found. |
| V1-to-Reborn import and migration | [#3029](https://github.com/nearai/ironclaw/issues/3029), [#3290](https://github.com/nearai/ironclaw/issues/3290), [#3287](https://github.com/nearai/ironclaw/issues/3287), [#3288](https://github.com/nearai/ironclaw/issues/3288) | Direct umbrella coverage. Needs a concrete dry-run/idempotency/reporting checklist. |
| Regression and vertical-slice tests | [#3067](https://github.com/nearai/ironclaw/issues/3067), [#3613](https://github.com/nearai/ironclaw/issues/3613), [#4431](https://github.com/nearai/ironclaw/issues/4431), [#4447](https://github.com/nearai/ironclaw/issues/4447) | Direct coverage. Use the validation matrix below to shape test acceptance. |

Recommended issue hygiene:

1. Do not create duplicate umbrella issues for Reborn parity.
2. Add this audit's P0/P1 acceptance criteria to the existing umbrella issues.
3. Create narrow sub-issues only for weak spots: diagnostics/logs/status API,
   setup/config API parity, service lifecycle, ACP-specific replacement, and
   migration dry-run/reporting semantics if not already captured in issue
   bodies.
4. Treat "V1 CLI parity" issues as "typed service/API parity plus optional
   thin CLI wrapper" issues.

### Issue Shape Problems and Fix

The issue set is not mostly stale by date. Many Reborn issues were created or
refreshed recently. The bigger problem is shape:

- Some issues are **too broad but not outcome-owned**. They collect many links
  and constraints, but do not name the end-to-end product capability an owner
  can ship.
- Some issues are **too implementation-defined for an epic**. They prescribe
  service shapes, trait sketches, backend lists, and tests in enough detail
  that ownership becomes "execute this spec" rather than "own this outcome."
- Some issues are **too narrow for planning**. Regression-test issues like
  [#4431](https://github.com/nearai/ironclaw/issues/4431) are useful, but they
  should not be treated as parity ownership units.
- Some older issues are **directionally stale** because they frame parity as
  CLI/TUI migration rather than WebUI v2 API-first control-plane parity.

Recommended structure:

1. Create or refresh a small set of outcome epics.
   Each epic should describe a user/operator capability, not an implementation
   module. Good epic titles:
   - `Epic: Reborn operator setup and config control plane`
   - `Epic: Reborn diagnostics, logs, and readiness control plane`
   - `Epic: Reborn memory parity`
   - `Epic: Reborn routines and triggers parity`
   - `Epic: Reborn channel setup/status parity`
   - `Epic: Reborn V1 migration and rollback`
2. Keep implementation decomposition optional.
   Engineers can create narrower tickets when useful, but the epic should not
   require a prebuilt task tree. The epic owns the outcome, invariants,
   dependencies, and acceptance criteria.
3. Add an owner-facing epic template:

```text
Outcome
- What user/operator can do when this epic is done.

Non-negotiable invariants
- Security, data ownership, compatibility, and Reborn boundary rules.

Canonical interface
- WebUI v2 API / typed service facade first.
- CLI only as optional thin wrapper when justified.

Done means
- 3-7 observable acceptance criteria.
- Caller-level tests named.
- Docs/migration notes named.

Out of scope
- Explicitly named to prevent ownership sprawl.
```

4. Reclassify current issues:
   - [#3031](https://github.com/nearai/ironclaw/issues/3031) should remain the
     parent product-surface tracker, but link to outcome epics rather than
     being the only owner surface.
   - [#3026](https://github.com/nearai/ironclaw/issues/3026) is valuable but
     over-specified for an epic. Keep it as a production-composition spec
     under a shorter production-readiness epic.
   - [#3029](https://github.com/nearai/ironclaw/issues/3029) is also valuable
     but too large for one end-to-end owner. Reframe it by migration domain:
     config/settings, secrets, memory, routines, channels/pairing, extensions,
     events/audit, and rollback/reporting.
   - [#3284](https://github.com/nearai/ironclaw/issues/3284) should be refreshed
     to say CLI/TUI/setup migrates onto API-first Reborn services, not that CLI
     itself is the primary parity target.
   - [#4431](https://github.com/nearai/ironclaw/issues/4431) is the right size
     for a focused regression ticket. It can be linked from a broader
     visible-capability/readiness epic, but should not define the epic.

Net effect: epics become ownable outcomes; engineers retain freedom to break
down implementation as they see fit.

### Proposed Epic Set

Use [#3031](https://github.com/nearai/ironclaw/issues/3031) as the parent
tracker, then create or refresh these outcome epics under it. This should be
the active owner surface for V1-to-Reborn parity.

| Epic | Owns | Existing issues to consolidate |
|---|---|---|
| `Epic: Reborn production readiness and cutover gate` | Reborn can run in production mode with explicit config, durable services, fail-closed readiness, no accidental exposure, and rollback gating. | [#3026](https://github.com/nearai/ironclaw/issues/3026), [#3333](https://github.com/nearai/ironclaw/issues/3333), [#3602](https://github.com/nearai/ironclaw/issues/3602), [#3020](https://github.com/nearai/ironclaw/issues/3020), [#3032](https://github.com/nearai/ironclaw/issues/3032), [#3045](https://github.com/nearai/ironclaw/issues/3045), [#3067](https://github.com/nearai/ironclaw/issues/3067) |
| `Epic: Reborn WebUI v2 command/control API foundation` | WebUI v2 APIs are the canonical command plane: typed Product Workflow doors, stable HTTP/SSE/WS contracts, auth/rate/body limits, OpenAPI/AsyncAPI, and route handlers as adapters. | [#3280](https://github.com/nearai/ironclaw/issues/3280), [#4488](https://github.com/nearai/ironclaw/issues/4488), [#4483](https://github.com/nearai/ironclaw/issues/4483), [#3953](https://github.com/nearai/ironclaw/issues/3953), [#3607](https://github.com/nearai/ironclaw/issues/3607), [#3613](https://github.com/nearai/ironclaw/issues/3613), [#3615](https://github.com/nearai/ironclaw/issues/3615) |
| `Epic: Reborn operator setup, config, diagnostics, and service lifecycle` | An operator can install, configure, inspect, debug, tail logs, check readiness, and manage local service lifecycle through APIs plus thin CLI wrappers where justified. | [#3284](https://github.com/nearai/ironclaw/issues/3284), [#4118](https://github.com/nearai/ironclaw/issues/4118), [#3036](https://github.com/nearai/ironclaw/issues/3036), [#4427](https://github.com/nearai/ironclaw/issues/4427), [#4353](https://github.com/nearai/ironclaw/issues/4353), [#83](https://github.com/nearai/ironclaw/issues/83), [#89](https://github.com/nearai/ironclaw/issues/89) |
| `Epic: Reborn memory parity` | Users and operators can inspect and manage persistent memory through Reborn: search, read, write, tree/status views, indexing status, ownership/privacy behavior, and V1 memory import criteria. `Workspace` should be treated as the backing abstraction here unless a separate user-facing workspace outcome is defined. | [#3287](https://github.com/nearai/ironclaw/issues/3287), [#3537](https://github.com/nearai/ironclaw/issues/3537), [#87](https://github.com/nearai/ironclaw/issues/87), [#1782](https://github.com/nearai/ironclaw/issues/1782) |
| `Epic: Reborn routines and triggers parity` | Users and operators can create, edit, enable, disable, delete, inspect history, and diagnose scheduled/event-triggered background work through Reborn APIs. | [#3290](https://github.com/nearai/ironclaw/issues/3290), [#3873](https://github.com/nearai/ironclaw/issues/3873), [#4475](https://github.com/nearai/ironclaw/issues/4475), [#4432](https://github.com/nearai/ironclaw/issues/4432), [#4439](https://github.com/nearai/ironclaw/issues/4439) |
| `Epic: Reborn channels, ProductAdapters, and actor binding parity` | Main deployed channels can be configured, bound, inspected, receive inbound turns, deliver outbound responses, report health, and reconnect through ProductAdapter surfaces. | [#3577](https://github.com/nearai/ironclaw/issues/3577), [#3285](https://github.com/nearai/ironclaw/issues/3285), [#3581](https://github.com/nearai/ironclaw/issues/3581), [#3582](https://github.com/nearai/ironclaw/issues/3582), [#3616](https://github.com/nearai/ironclaw/issues/3616), [#4491](https://github.com/nearai/ironclaw/issues/4491), [#4203](https://github.com/nearai/ironclaw/issues/4203) |
| No new broad capability/ProductAuth epic | The broad capability/ProductAuth umbrella is already decomposed. Keep ownership in narrowed issues: production/scoped capability lifecycle admin parity, ProductAuth production backend/PKCE HA safety, staged credential consumers, SSO follow-ups, and default credential account UX. | [#3288](https://github.com/nearai/ironclaw/issues/3288), [#4175](https://github.com/nearai/ironclaw/issues/4175), [#4176](https://github.com/nearai/ironclaw/issues/4176), [#4204](https://github.com/nearai/ironclaw/issues/4204), [#4382](https://github.com/nearai/ironclaw/issues/4382), closed [#4201](https://github.com/nearai/ironclaw/issues/4201) |
| `Epic: Reborn V1 migration, dry-run, and rollback` | Explicit, idempotent V1-to-Reborn import with inventory, dry-run, redacted reports, quarantine, source-of-truth rules, rollback notes, and PostgreSQL/libSQL fixtures. | [#3029](https://github.com/nearai/ironclaw/issues/3029), [#3287](https://github.com/nearai/ironclaw/issues/3287), [#3290](https://github.com/nearai/ironclaw/issues/3290), [#3288](https://github.com/nearai/ironclaw/issues/3288) |

Avoid creating more top-level epics until these have owners. New implementation
issues are optional; engineers should create them only when they help execution.

### Self-Contained Epic Format

An epic should be self-contained enough that an owner can drive it without
reading ten sibling issues first. It should not be so detailed that it becomes
a prewritten implementation plan.

Use this shape:

```text
Problem
- What V1 capability or Reborn cutover risk this epic closes.

User/operator outcome
- What a user, operator, or integrator can do when this is complete.

Canonical interface
- WebUI v2 API / typed service facade first.
- CLI only as a thin wrapper when justified.

Scope
- Included workflows.
- Included data/state domains.
- Included compatibility promises.

Acceptance criteria
- Observable, testable end states.
- Written from caller/operator behavior, not internal function names.

Required validation
- Caller-level tests, migration tests, security assertions, and docs updates.

Non-goals
- Explicitly excluded work.

Close criteria
- What must be true before the epic is closed.
```

Acceptance criteria should be clear but not overfit to one implementation.
Prefer:

```text
- Operator can validate effective Reborn config through a WebUI v2 API and
  gets redacted, actionable errors for unsupported fields.
```

Avoid:

```text
- Add `fn validate_reborn_config(input: ConfigInput)` to crate X and call it
  from route Y.
```

Each epic should have 5-9 acceptance criteria. A good set covers:

1. the happy-path workflow;
2. the failure or degraded-state workflow;
3. security/redaction/authorization behavior;
4. persistence or migration behavior if state is involved;
5. API-first interface behavior;
6. optional CLI wrapper behavior if needed;
7. caller-level validation.

Implementation tickets can contain exact traits, files, backend matrices, and
test commands if engineers choose to create them. The epic should say what
must be true, not how every line is built.

Example acceptance criteria for `Epic: Reborn operator setup, config,
diagnostics, and service lifecycle`:

- A fresh Reborn install can be configured through WebUI v2 APIs for provider,
  model, profile, and WebUI access without manually editing TOML for the common
  path.
- Operators can list effective config, get a key, set supported keys, validate
  unsupported fields, and see precedence with secrets redacted.
- Reborn exposes readiness/status for runtime, storage, secrets, provider,
  WebUI, trigger poller, channels, and extension subsystems.
- Reborn logs can be queried and tailed through the canonical API with token,
  secret, host-path, and prompt-content redaction.
- `ironclaw-reborn` CLI commands, where present, are thin clients over the same
  services/APIs and do not become a second command plane.
- Service lifecycle support covers install/start/stop/status for the supported
  local OS targets or explicitly documents unsupported targets.
- Caller-level tests exercise the WebUI v2/router or service facade paths for
  setup, config, diagnostics, logs, and service lifecycle.
- Relevant setup/config/operator docs are updated with migration and rollback
  notes.

### Close or Consolidate Candidates

Do not mass-close blindly. For each issue, leave a closing comment pointing to
the replacement epic and, when useful, quote the moved acceptance criterion.

Likely close after linking to the new epics:

- [#2987](https://github.com/nearai/ironclaw/issues/2987) if [#3031](https://github.com/nearai/ironclaw/issues/3031)
  plus the proposed outcome epics become the active Reborn planning surface.
- [#3484](https://github.com/nearai/ironclaw/issues/3484) if contributor
  runway work is now represented by the channel/capability/product-surface
  epics.
- [#3697](https://github.com/nearai/ironclaw/issues/3697),
  [#3699](https://github.com/nearai/ironclaw/issues/3699), and
  [#3700](https://github.com/nearai/ironclaw/issues/3700) if the product-live
  workflow framing is superseded by Product Workflow/WebUI v2/channel epics.
- [#3576](https://github.com/nearai/ironclaw/issues/3576) if the useful
  harvested runtime/security patterns are already copied into concrete Reborn
  issues.
- [#3537](https://github.com/nearai/ironclaw/issues/3537) if memory-as-extension
  is a stale design direction; otherwise consolidate it into the memory parity
  epic with an explicit decision needed.

Likely refresh rather than close:

- [#3026](https://github.com/nearai/ironclaw/issues/3026): keep as a production
  composition spec under the production-readiness epic.
- [#3029](https://github.com/nearai/ironclaw/issues/3029): keep the migration
  contract, but reframe ownership by migration domain.
- [#3284](https://github.com/nearai/ironclaw/issues/3284): refresh from
  CLI/TUI-first wording to API-first setup/config/operator control plane.
- [#3577](https://github.com/nearai/ironclaw/issues/3577): keep as channel port
  tracker, but attach it to the channel epic and make health/setup/status
  criteria explicit.

Keep narrow tickets open when they are immediately actionable:

- [#4431](https://github.com/nearai/ironclaw/issues/4431) and similar regression
  tests.
- [#3581](https://github.com/nearai/ironclaw/issues/3581) / [#3582](https://github.com/nearai/ironclaw/issues/3582)
  if Telegram/WeChat are still planned deployed channels.
- [#4118](https://github.com/nearai/ironclaw/issues/4118) if provider add/login
  parity is still a needed thin wrapper over ProductAuth/control-plane APIs.

## Reborn Strengths to Preserve

Do not regress these while chasing V1 parity:

- no root `ironclaw` / `src/*` runtime dependencies in Reborn CLI;
- explicit `v1_state: not-used`;
- Reborn home separation;
- fail-closed unsupported config sections;
- inline secret rejection in config;
- ProductAuth through trait-shaped ports;
- no raw token values through chat/model-visible DTOs;
- WebUI v2 descriptor-driven body/rate limits;
- WebSocket same-origin enforcement;
- scoped identity and tenant/user/agent/project isolation;
- facade-shaped composition handles;
- caller-level Product Workflow boundaries;
- local-dev-yolo host access disclosure.

## Priority Roadmap

### Phase 0: Replacement Contract

Goal: define exactly what "Reborn replaces V1" means.

Deliverables:

1. A checked-in replacement checklist with required API surfaces and any
   optional CLI wrappers.
2. A state migration policy: no implicit V1 reads, explicit dry-run imports only.
3. A production profile readiness contract.
4. A decision on which V1 features remain legacy-only during transition.
5. A rule that WebUI v2 route handlers adapt to shared Reborn services rather
   than owning business logic.

### Phase 1: API Operator Minimum

Goal: an operator can install, configure, run, inspect, and debug Reborn
through canonical WebUI v2/control-plane APIs. CLI commands are optional thin
clients over those APIs or shared facades.

Deliverables:

1. setup/onboarding API for provider/model/profile/WebUI token basics.
2. config list/get/set/validate API.
3. diagnostics API equivalent to a real `doctor`.
4. logs API with tail/follow or event-stream access.
5. status/readiness API.
6. production profile wiring for `run`.
7. optional `ironclaw-reborn` wrappers for bootstrap/smoke/local automation.
8. tests for every API above through WebUI v2/router or service facade.

### Phase 2: Daily Product Workflows

Goal: common V1 workflows no longer require the V1 binary.

Deliverables:

1. Reborn memory API, with optional CLI wrapper.
2. Reborn routines/trigger API, with optional CLI wrapper.
3. Reborn pairing API, with optional CLI wrapper.
4. Reborn WebUI v2 daily UI surfaces backed by those APIs.
5. Reborn channel status/setup APIs for the main deployed channels.
6. V1-to-Reborn dry-run import for config, memory, routines, and pairing.

### Phase 3: Capability Administration

Goal: Reborn can administer user-installed capabilities through one canonical
control plane.

Deliverables:

1. Wire the new production/scoped lifecycle foundation into WebUI/API and
   optional thin CLI wrappers.
2. Generic MCP management or a documented hosted-extension replacement.
3. WASM package admin parity where it is not already represented by extension
   lifecycle.
4. ACP-specific replacement decision or explicit non-goal.
5. ProductAuth production backend parity, OAuth PKCE HA safety, staged
   credential consumers, SSO follow-ups, and default credential account UX.

### Phase 4: Migration and Retirement

Goal: V1 can be retired for users who opt into Reborn.

Deliverables:

1. idempotent V1-to-Reborn import.
2. migration report with skipped/unsupported records.
3. backward-compatible rollback notes.
4. caller-level regression tests for migrated memory/routines/pairing/config.
5. documented cutover path from `ironclaw` to `ironclaw-reborn`.

## Suggested Validation Matrix

| Work area | Minimum validation |
|---|---|
| API command parity | WebUI v2/router tests or service-facade tests for every canonical operation |
| CLI wrappers | `cargo test -p ironclaw_reborn_cli` plus binary smoke tests only for thin wrappers that remain |
| Reborn architecture boundary | `cargo test -p ironclaw_architecture reborn` |
| Runtime profile wiring | `cargo test -p ironclaw_reborn` and `cargo test -p ironclaw_reborn_composition` |
| WebUI v2 | `cargo test -p ironclaw_reborn_composition --features webui-v2-beta` and descriptor mount tests |
| ProductAuth | product-auth route/service tests, no raw-token serialization assertions |
| Trigger/routines | repository/poller tests plus API CRUD/history integration tests |
| Memory | caller-level API tests and migration idempotency tests |
| Channels | ingress to Product Workflow to outbound delivery tests |
| Migration | dry-run and idempotency tests with synthetic V1 state |

## Bottom Line

Reborn should be treated as a real replacement architecture with an incomplete
product shell, not as a prototype. The next work should not be broad feature
chasing. It should be a focused replacement sequence:

1. production run profile;
2. WebUI v2/control-plane APIs for setup/config/doctor/logs/status;
3. API-backed memory/routines/pairing;
4. WebUI v2 daily UI surfaces over those APIs;
5. channel replacement for the channels users already depend on;
6. explicit migration;
7. thin CLI wrappers only where they simplify bootstrap, automation, or local
   debugging.

Once those land, Reborn can start replacing V1 in real usage. Until then, V1
remains the operational binary even though Reborn is the better long-term
runtime architecture.
