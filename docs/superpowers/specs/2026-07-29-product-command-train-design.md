# Product Command Train: Role Gating, WebUI Palette, Native Slack Slash

**Date:** 2026-07-29
**Status:** Approved design (see Decisions). Implementation plan to follow.

## Problem

Three gaps in the product command surface, plus one security hole found while
scoping them:

1. **Slack UX:** commands only work with a leading space (` /status`) because
   Slack's client intercepts bare `/text` for its own slash-command system —
   unregistered commands error client-side and never reach the app.
2. **WebUI UX:** the web composer has no command support at all — `/status`
   typed in web chat submits an ordinary LLM turn.
3. **Admin backdoor (security):** `/model set` and `/model set-provider`
   execute `LlmConfigService::set_active` — an **operator-wide** LLM
   provider/model hot-swap — with no role check on the command path
   (`reborn_services.rs::execute_product_model_command` →
   `invoke_llm_active_set`). The ten lifecycle commands
   (`extension_configure`, `skill_install`, …) are likewise tenant-wide and
   are legal in any channel manifest's `commands = [...]` declaration
   (`validate_declared_product_command` accepts the full registry). Bundled
   Slack/Telegram declare only `["status"]` today, so the hole is latent on
   first-party channels — but any third-party extension manifest could expose
   tenant controls to its paired DM users.

## Current state (main, 2026-07-29)

The generic backbone landed via #6816 and neighbors:

- Classification: `ironclaw_extension_host::extension_ingress` runs
  `classify_channel_inbound_text` (in
  `ironclaw_host_api::product_adapter::inbound`) on every normalized channel
  message; `/cmd args` becomes `ChannelInboundClassification::Command` with a
  normalized `InboundCommandPayload`.
- Admission: `ironclaw_product::DirectConversationCommandAdmission` — direct
  conversations only + the channel manifest's declared set, fail-closed;
  rejection help lists only declared commands.
- Dispatch: typed `ProductCommand` → `ProductSurface::invoke` operations
  (`product.model.command`, `product.status.command`,
  `product.lifecycle.command`); binding resolution supplies
  `binding.actor_user_id` (`workflow.rs::dispatch_product_command`).
- Results: `CommandResultView` (title/fields/lines) delivered by
  `RunDeliveryObserver` as a channel message; help text scoped via
  `with_enabled_commands`.
- Registry: `ironclaw_product::commands` — descriptors carry name + aliases
  only (no title/description/usage); inventory = `model`, `status`
  (alias `progress`), + ten lifecycle commands.

PR **#6678** (branch `alpine-fight`, OPEN, CONFLICTING) predates the landed
backbone and still contains the unlanded slices this train needs: descriptor
metadata, `GET/POST` WebUI command routes, and the composer slash menu.

## Decisions

| # | Decision | Choice |
|---|----------|--------|
| 1 | Fate of #6678 | **Rebase it** onto main as the PR-2 vehicle; main's landed shapes win every conflict. |
| 2 | Slack-native naming | **Single `/ironclaw` dispatcher** (`/ironclaw status`, `/ironclaw model set …`). Slack's command namespace is workspace-global, built-ins (`/status`) cannot be overridden, and the ecosystem convention is an app-named dispatcher (`/github …`, `/jira …`). No aliases registered; future commands need zero Slack app changes. Direct `/model` registration stays available as a purely additive later option. |
| 3 | Admin commands | **Role-gate now** (product decision remove-vs-gate is pending with Sergey; gating was chosen so either outcome is reachable — see Contingency). Per-**action** granularity: `/model` bare read is User; `set`/`set-provider` and the lifecycle family are Admin. |
| 4 | Visibility | **Role-filtered everywhere**: WebUI inventory filtered by caller role; Slack natively registers only user-facing commands; channel help text lists only what the actor may run. Admission denial is the backstop — hiding is never the control. |
| 5 | Train order | **Gate → Palette → Slack** (PR-1 → PR-2 → PR-3, stacked). No window where the browser or a manifest exposes ungated tenant controls. |
| 6 | WebUI results | **Ephemeral** system-notice bubble (Slack-like, #6678's shape). Explicitly ephemeral per the gateway-events rule; not a durable timeline event. Durable rendering is a future decision. |
| 7 | Bundled manifests | Declare `["model", "status"]` (in PR-1, same PR as gating). Lifecycle commands stay undeclared on bundled channels; admins manage extensions from the WebUI. Third-party manifests may declare them; admission gates. The `progress` registry alias stays (typed path, Telegram) but is not Slack-registered and not declared. |

## PR-1 — Role-gated command admission

### Registry (`crates/ironclaw_product/src/commands.rs`)

- `CommandAudience { User, Admin }`.
- `ProductCommandDescriptor` gains `audience: CommandAudience` — the
  **listing** audience. `model`, `status` → `User`; all
  `LifecycleCommandKind` descriptors → `Admin`. Drives help text and (PR-2)
  the palette inventory.
- `required_audience(&ProductCommand) -> CommandAudience` — the
  **execution** audience, action-aware: `Model{Status}` → User,
  `Model{Set|SetProvider}` → Admin, `Status` → User, `Lifecycle{..}` → Admin.
  Lives beside the parse specs so one file owns both tables.

### Role resolution port

New trait in `ironclaw_product` beside `ProductCommandAdmissionService`:
resolve the **bound** user's `AdminUserRole`
(`reborn_services/admin_users.rs`: `Owner | Admin | Member`, `is_admin()`)
from the admission context's `installation_id` + `external_actor_ref`. The
channel-host assembly (`crates/ironclaw_extension_host/src/channel_host.rs`)
supplies the implementation: channel identity binding → bound user id →
admin-users role lookup; composition wires it.

Fail-closed semantics:

- Positive resolution of a non-admin → permanent `PolicyDenied`.
- Resolver **error** → retryable failure notice (never silent admin
  treatment, never silent member treatment of an admin).
- Unpaired/unknown actors never reach admission (pairing interceptor
  upstream rejects first).

### Admission (`crates/ironclaw_product/src/command_admission.rs`)

`DirectConversationCommandAdmission` order: direct-conversation check →
declared-set check → audience check (only when `required_audience` is Admin:
resolve role, reject non-admins). New distinct rejection notice for the admin
denial ("this command needs an admin account" copy family), separate from the
direct-conversation notice. Sensitive commands never reach their handlers.

Help text becomes role-aware: built from the declared set filtered to the
actor's listing audience at rejection time (admission has both the set and
the resolved role). Empty filtered set keeps the existing "commands are not
available in this channel" line. The observer's static help field remains as
generic fallback only.

### Manifests

`crates/ironclaw_first_party_extensions/assets/slack/manifest.toml` and
`.../telegram/manifest.toml`: `commands = ["model", "status"]`.

### Tests

- Contract: registry audience tables pinned (listing + execution, 1:1 with
  descriptors).
- Workflow-caller admission matrix: member `/model` read allowed; member
  `/model set` denied with the admin notice; admin `/model set` allowed;
  resolver error → retryable failure; direct-only preserved; help filtered
  per role.
- Channel-host e2e at the seam: member `/model set` → denial notice delivered
  **and** active LLM config unchanged; admin path executes and the snapshot
  reflects the change.

## PR-2 — WebUI command palette (the #6678 rebase)

### Rebase posture

Main wins: registry stays in `ironclaw_product::commands` (drop the PR's
`ironclaw_host_api::product_commands` move — `ironclaw_extension_host`
already depends on `ironclaw_product` for validation);
`DirectConversationCommandAdmission` stays (drop `PairedDmCommandAdmission`);
already-landed slices (classification, manifest opt-in, observer scoping)
drop out. Surviving slices: descriptor metadata
(`title`/`description`/`usage` added to main's descriptor struct beside
PR-1's `audience`), WebUI backend, frontend palette.

### Backend (`reborn_services/product_commands.rs` facade + webui_v2 routes)

- `GET /api/webchat/v2/commands` — inventory with metadata, filtered by the
  authenticated caller's `AdminUserRole` (direct lookup; no channel port)
  against the listing audience. Members: `model` + `status`. Admins: + the
  lifecycle family.
- `POST /api/webchat/v2/threads/{thread_id}/commands` — shared parser →
  the same `required_audience` policy function the channel admission uses
  (surfaces cannot drift) → the same typed operations. `/status` keeps the
  thread-ownership probe (foreign-thread 404 pinned). Member `/model set`
  gets the same permanent `PolicyDenied` shape; handlers are never reached.

### Frontend (`crates/ironclaw_webui/frontend/src/pages/chat/`)

Server-inventory-driven throughout (no hardcoded names): keep the PR's
`chat-commands.ts` matcher/menu/renderer and `useChatCommands`. Composer menu
upgraded to Slack quality: anchored popover above the input; rows render
`/name` — title — description with the matched prefix highlighted; usage hint
for the selected row; ↑/↓ + Enter/Tab complete + Esc dismiss; hover/click;
re-filtering as you type. Results render as the generic system-notice bubble,
ephemeral (Decision 6). Unknown `/text` submits as an ordinary message,
matching channels.

### Tests

- WebUI caller tier: member vs admin inventory filtering; `/status` on an
  owned thread returns the rendered view; foreign thread 404 (kept pin);
  member `/model set` policy rejection.
- Frontend vitest: existing chat-commands suites plus keyboard navigation and
  metadata rendering; locale key parity; `tsc` + conventions lint.
- Descriptor→DTO projection contract pinned.

## PR-3 — Native Slack slash commands

### Transport

Reuse the single signed `[channel.ingress]` `events` route — Slack lets every
slash command point at any Request URL, and the HMAC recipe signs the raw
body regardless of content type. No manifest-schema or host changes.
`crates/ironclaw_slack_extension/src/{payload,channel}.rs` learn the payload
shapes: JSON → existing event path; form-encoded with `command` → slash
invocation; form-encoded `ssl_check=1` → immediate empty 200.

### Normalization (dispatcher mapping)

The single registered command is `/ironclaw`; its `text` is the real
command. The adapter maps the form payload to the same normalized inbound
message events produce:

- `command="/ironclaw"`, `text="status …"` → normalized text `"/status …"`
  (prepend `/` to the trimmed text; defensively strip a leading `/` the user
  may have typed, so `/ironclaw /status` also works).
- Bare `/ironclaw` or `/ironclaw help` → normalized text `"/help"` — not a
  registered command, so it deterministically takes the existing
  unknown-command rejection path, which delivers the role-filtered
  "Available commands" help. (If a real `help` command ever joins the
  registry, this mapping upgrades gracefully into executing it.)
- Actor from `user_id`, conversation from `channel_id`, `DirectChat`
  trigger, event id derived from `trigger_id` (unique per invocation; Slack
  does not redeliver slash commands).

Downstream — classification, pairing, PR-1 admission, dispatch, observer
bot-DM delivery — is identical to the space-prefixed path. Ingress ACK is
the immediate empty 200 (Slack's ≤3s rule); the visible result is the bot's
DM message.

### Help rendering: per-channel invocation prefix

Neutral help renders `/model`, but typing `/model` bare in Slack fails
(client-intercepted). `ChannelDescriptor` presentation gains an optional
command display prefix; Slack's manifest sets `/ironclaw ` so help and
rejection notices render `/ironclaw model` there. Other channels keep the
plain `/name` rendering. The space-typed ` /model` path keeps working as an
undocumented fallback.

### Behavioral edges

- Slash invoked outside the bot DM: flows through the same pipeline; the
  direct-conversation admission rejects; if the bot cannot post into that
  conversation the user sees nothing — accepted MVP limitation, noted in the
  PR. `response_url` support is the future fix.
- Unpaired user: existing connect-nudge path.
- Natively registered but manifest-undeclared command: rejects with
  role-filtered help. Declaration is the single source of truth; Slack
  registration is presentation only.

### Registration (docs, not code)

`docs/reborn/setup-slack-for-reborn-binary.md` + `docs/channels/slack.mdx`
gain one app-manifest entry registering `/ironclaw` (description + usage
hint `status | model set <model> | help` — Slack renders these in
autocomplete) pointing at the events URL, with the rationale stated: the
namespace is workspace-global, built-ins like `/status` cannot be
overridden, and the app-named dispatcher is the ecosystem convention.
Admin/lifecycle commands gain no registration or usage-hint mention
(Decision 4). The Slack manifest's declared set stays `["model", "status"]`
— declarations govern the underlying commands, not the dispatcher spelling.
Telegram is unchanged.

### Tests

- Adapter conformance: dispatcher mapping (`/ironclaw status …` →
  `"/status …"`, leading-slash strip, bare/`help` → `"/help"`, fields,
  trigger, event id); `ssl_check` short-circuit; malformed form rejected;
  JSON events unaffected.
- Channel-host e2e: signed slash-shaped body for a paired DM runs
  `/ironclaw status` end-to-end → rendered status result delivered, zero
  turns; bare `/ironclaw` → help notice rendered with the `/ironclaw `
  display prefix; non-DM slash → direct-only denial; signature failure still
  rejects at ingress (pin extended over form bodies).

## Cross-cutting error handling

- Role-resolver failures are retryable, sanitized notices; member denials are
  permanent `PolicyDenied` with the admin-account copy; no backend strings,
  paths, or provider details in any notice (existing
  `ProductSurfaceError` taxonomy).
- All new rejection copy flows through the existing observer notice paths;
  the WebUI returns the same sanitized `ProductSurfaceErrorKind` families.

## Out of scope / flagged follow-ups

- **WebUI Inference tab role gap:** the llm-config routes
  (`LlmConfigService`) appear to have no role gate either — the same
  tenant-wide hole via browser Settings. Sibling fix, decide with Sergey's
  answer; not in this train.
- **Contingency for Sergey's decision:** if the product call is **remove**
  (not gate), the audience vocabulary makes it a small delta — delete the
  lifecycle descriptors/parse arms (registry validation then fails closed on
  any manifest declaring them) and keep the audience machinery for
  `/model set`. Nothing in PR-2/PR-3 changes.
- **Telegram native commands:** `setMyCommands` can register the command
  menu programmatically — cheap sibling of PR-3 if wanted.
- **Direct `/model` Slack registration:** no built-in collision; purely
  additive beside the dispatcher if the shorter spelling is ever wanted.
- **`response_url` delivery** for out-of-DM slash rejections.
- **Durable command results** in the WebUI timeline (Decision 6 revisit).
- **Future user commands** (`/new`, `/stop`, `/compact`, Telegram `/start`
  deep-link) ride the same registry + audience model.
