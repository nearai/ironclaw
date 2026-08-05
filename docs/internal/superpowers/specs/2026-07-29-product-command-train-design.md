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
- Admission: `ironclaw_assistant::DirectConversationCommandAdmission` — direct
  conversations only + the channel manifest's declared set, fail-closed;
  rejection help lists only declared commands.
- Dispatch: typed `ProductCommand` → `ProductSurface::invoke` operations
  (`product.model.command`, `product.status.command`,
  `product.lifecycle.command`); binding resolution supplies
  `binding.actor_user_id` (`workflow.rs::dispatch_product_command`).
- Results: `CommandResultView` (title/fields/lines) delivered by
  `RunDeliveryObserver` as a channel message; help text scoped via
  `with_enabled_commands`.
- Registry: `ironclaw_assistant::commands` — descriptors carry name + aliases
  only (no title/description/usage); inventory = `model`, `status`
  (alias `progress`), + ten lifecycle commands.

PR **#6678** (branch `alpine-fight`, OPEN, CONFLICTING) predates the landed
backbone and still contains the unlanded slices this train needs: descriptor
metadata, `GET/POST` WebUI command routes, and the composer slash menu.

## Decisions

| # | Decision | Choice |
|---|----------|--------|
| 1 | Fate of #6678 | **Rebase it** onto main as the PR-2 vehicle; main's landed shapes win every conflict. |
| 2 | Slack-native naming | **Single `/ironclaw` dispatcher** (`/ironclaw status`, `/ironclaw model set …`). Slack's command namespace is workspace-global, built-ins (`/status`) cannot be overridden, and the ecosystem convention is an app-named dispatcher (`/github …`, `/jira …`). No aliases registered; future commands need zero Slack app changes. Direct `/model` registration stays available as a purely additive later option. **Slack-only:** Telegram's command namespace is per-bot (`/status` in the bot DM is unambiguous; groups disambiguate natively with `/status@BotName`), so Telegram keeps direct commands — the dispatcher is Slack presentation, not pipeline vocabulary. |
| 3 | Admin commands | **Role-gate now** (product decision remove-vs-gate is pending with Sergey; gating was chosen so either outcome is reachable — see Contingency). Per-**action** granularity: `/model` bare read is User; `set`/`set-provider` and the lifecycle family are Admin. |
| 4 | Visibility | **Role-filtered everywhere**: WebUI inventory filtered by caller role; Slack natively registers only user-facing commands; channel help text is filtered to user-audience commands (uniform for all actors; per-actor filtering is the WebUI palette's job, admission is the backstop). Admission denial is the backstop — hiding is never the control. |
| 5 | Train order | **Gate → Palette → Slack** (PR-1 → PR-2 → PR-3, stacked). No window where the browser or a manifest exposes ungated tenant controls. |
| 6 | WebUI results | **Ephemeral** system-notice bubble (Slack-like, #6678's shape). Explicitly ephemeral per the gateway-events rule; not a durable timeline event. Durable rendering is a future decision. |
| 7 | Bundled manifests | Declare `["model", "status"]` (in PR-1, same PR as gating). Lifecycle commands stay undeclared on bundled channels; admins manage extensions from the WebUI. Third-party manifests may declare them; admission gates. |
| 8 | Aliases | **Deleted in PR-1.** `progress` is the registry's only alias and is inert everywhere (declarations are exact tokens; nothing declares it). The whole mechanism goes with it: `aliases` descriptor field, alias matching branches, the aliases-never-implicitly-enabled validation rule and its pins (removed deliberately with the rule they pin), and #6678's frontend alias matching. Resurrect from git if a real synonym need appears. |

## Shared pipeline: one center, thin edges

Channels never parse or police commands; they only unwrap transport. All
parsing, privilege policy, execution, and result shaping is defined once.

```
 Slack slash POST         Telegram DM           WebUI composer
 /ironclaw status …       /status …             /status …
       │                       │                     │
 [slack adapter]          [telegram adapter]         │
 dispatcher unwrap →      envelope only              │
 text "/status …"         (text unchanged)           │
       └───────────┬───────────┘                     │
                   ▼                                 ▼
   generic ingress sink (extension_host)   POST /threads/{id}/commands
   → shared slash parser (host_api)        → same shared parser
                   │                                 │
                   ▼                                 ▼
  ┌────────── shared command center (ironclaw_assistant) ──────────┐
  │ registry: names + metadata + audience (commands.rs)          │
  │ typed parse: ProductCommand::from_payload                    │
  │ policy: direct-conv + declared set + required_audience ×     │
  │   actor role (channel door: admission service + role port;   │
  │   webui door: same audience table, authenticated caller)     │
  │ execute: ProductSurface::invoke → per-command handlers       │
  │ result: CommandResultView (title/fields/lines)               │
  └──────────────────────────────────────────────────────────────┘
                   │                                 │
                   ▼                                 ▼
   observer renders + delivers bot msg      ephemeral system-notice
   (per-channel display prefix on help)     (generic renderer)
```

Two doors, zero drift: both consult the same registry, audience table, and
operations — policy is defined once and consulted from both entries.

## PR-1 — Role-gated command admission

### Registry (`crates/ironclaw_assistant/src/commands.rs`)

- `CommandAudience { User, Admin }`.
- `ProductCommandDescriptor` gains `audience: CommandAudience` — the
  **listing** audience. `model`, `status` → `User`; all
  `LifecycleCommandKind` descriptors → `Admin`. Drives help text and (PR-2)
  the palette inventory.
- `required_audience(&ProductCommand) -> CommandAudience` — the
  **execution** audience, action-aware: `Model{Status}` → User,
  `Model{Set|SetProvider}` → Admin, `Status` → User, `Lifecycle{..}` → Admin.
  Lives beside the parse specs so one file owns both tables.
- Alias mechanism deleted (Decision 8): the `progress` alias, the `aliases`
  descriptor field, and every alias branch in name matching
  (`command_spec_for_name`, `validate_declared_product_command`,
  `ProductCommand::descriptor`) are removed, along with the
  alias-non-enablement contract pins.

### Role resolution port

New trait in `ironclaw_assistant` beside `ProductCommandAdmissionService`:
resolve the **bound** user's `AdminUserRole`
(`reborn_services/admin_users.rs`: `Owner | Admin | Member`, `is_admin()`)
from the admission context's `installation_id` + `external_actor_ref`. The
channel-host assembly (`crates/ironclaw_extension_host/src/channel_host.rs`)
supplies the implementation: channel identity binding → bound user id →
admin-users role lookup; composition wires it.

Fail-closed semantics:

- Positive resolution of a non-admin → permanent `AccessDenied`.
- Resolver **error** → retryable failure notice (never silent admin
  treatment, never silent member treatment of an admin).
- Unpaired/unknown actors never reach admission (pairing interceptor
  upstream rejects first).

### Admission (`crates/ironclaw_assistant/src/command_admission.rs`)

`DirectConversationCommandAdmission` order: direct-conversation check →
declared-set check → audience check (only when `required_audience` is Admin:
resolve role, reject non-admins). New distinct rejection notice for the admin
denial ("this command needs an admin account" copy family), separate from the
direct-conversation notice. Sensitive commands never reach their handlers.

Help is role-safe by filtering: the observer's static help includes only
user-audience declared commands, for every actor. Admission rejections carry
internal reasons only; the observer never echoes them. The admin denial is
keyed by the reused wire-stable `ProductRejectionKind::AccessDenied`.

### Manifests

`crates/ironclaw_first_party_extensions/assets/slack/manifest.toml` and
`.../telegram/manifest.toml`: `commands = ["model", "status"]`.

### Tests

- Contract: registry audience tables pinned (listing + execution, 1:1 with
  descriptors).
- Workflow-caller admission matrix: member `/model` read allowed; member
  `/model set` denied with the admin notice; admin `/model set` allowed;
  resolver error → retryable failure; direct-only preserved.
- Observer/run-delivery contract: static command help excludes admin-audience
  commands (uniform for every actor, not per-role — see Decision 4); the
  `AccessDenied` rejection delivers the fixed admin-account notice without
  leaking the internal reason.
- Channel-host e2e at the seam: member `/model set` → denial notice delivered
  and zero command-surface invokes recorded; admin path executes exactly one
  `product.model.command` invoke as the bound user (both through a recording
  `ProductSurface` double; zero turns submitted either way).

## PR-2 — WebUI command palette (the #6678 rebase)

### Rebase posture

Main wins: registry stays in `ironclaw_assistant::commands` (drop the PR's
`ironclaw_host_api::product_commands` move — `ironclaw_extension_host`
already depends on `ironclaw_assistant` for validation);
`DirectConversationCommandAdmission` stays (drop `PairedDmCommandAdmission`);
already-landed slices (classification, manifest opt-in, observer scoping)
drop out. Surviving slices: descriptor metadata
(`title`/`description`/`usage` added to main's descriptor struct beside
PR-1's `audience`), WebUI backend, frontend palette.

### Implicit-owner rule extended to both doors (PR-1 audit fix)

PR-1's final review found the channel role resolver
(`ChannelActorRoleResolver::actor_role` in
`crates/ironclaw_extension_host/src/channel_command_roles.rs`) had no
env-bearer-operator bypass: an operator with no admin-directory record was
permanently denied channel admin commands, while the WebUI door
(`RebornServices::authorize_admin`, and now `caller_is_command_admin` below)
already treated `caller.operator_config` as an implicit admin. PR-2 closes
that gap: when the resolved bound user equals the resolver's
`operator_user_id` and the admin directory has no record at all (`Ok(None)`),
the resolver now also returns `Ok(Some(AdminUserRole::Owner))`. A persisted
directory record of any status — including Suspended — still governs whenever
one exists; the new arm only fires on "no record."

The two doors remain asymmetric in one respect: the WebUI's
`caller.operator_config` bypass short-circuits before any directory lookup
and has no record-governs behavior at all, so a Suspended operator record
denies through the channel resolver but still admits through the WebUI door.
That asymmetry is tracked, not fixed, in issue #6877.

### Backend (`reborn_services/product_commands.rs` facade + webui_v2 routes)

- `GET /api/webchat/v2/commands` — inventory with metadata, filtered by the
  authenticated caller's `AdminUserRole` (direct lookup; no channel port)
  against the listing audience. Members: `model` + `status`. Admins: + the
  lifecycle family. An env-bearer operator caller (`caller.operator_config`)
  is an implicit admin here too, without a directory record.
- `POST /api/webchat/v2/threads/{thread_id}/commands` — shared parser →
  the same `required_audience` policy function the channel admission uses
  (surfaces cannot drift) → the same typed operations. `/status` keeps the
  thread-ownership probe, but a foreign thread and a never-created thread
  both resolve to the identical constant idle `CommandResultView` — never a
  404. (Design review ruled this indistinguishable-idle response
  equivalent-or-better than the originally planned foreign-thread 404: a
  404-vs-200 split would let a caller probe for other users' thread ids one
  guess at a time.) Member `/model set` gets the same permanent
  `AccessDenied` shape; handlers are never reached. Lifecycle commands stay
  listing-only through this route — `execute` rejects every
  `ProductCommand::Lifecycle` as `InvalidRequest` (the same role-filtered help
  text `/status`/`/model`'s help paths use) even for an admin caller who has
  already cleared the audience gate; admins manage extensions from the
  WebUI's Extensions page, not the composer.

### Frontend (`crates/ironclaw_webui/frontend/src/pages/chat/`)

Server-inventory-driven throughout (no hardcoded names): keep the PR's
`chat-commands.ts` matcher/menu/renderer and `useChatCommands`, simplified
by Decision 8 (no alias matching). Composer menu
upgraded to Slack quality: anchored popover above the input; rows render
`/name` — title — description with the matched prefix highlighted; usage hint
for the selected row; ↑/↓ + Enter/Tab complete + Esc dismiss; hover/click;
re-filtering as you type. Results render as the generic system-notice bubble,
ephemeral (Decision 6). Unknown `/text` submits as an ordinary message,
matching channels.

Two fixes landed on top of the rebased `alpine-fight` slice: (1) `EmptyState`
(the landing view's composer, mounted before any thread exists) did not
forward its `commands` prop to the nested `ChatInput`, so the palette was
unreachable from a brand-new thread's first composer — `chat.tsx` now passes
`commands={activeThreadId ? chatCommands : []}` to both `EmptyState` and
`ChatInput`, and `EmptyState` forwards it through, pinned by a dedicated
`EmptyState` prop-forwarding test. (2) The `chat.commandFailed` locale key
(`"Couldn't run that command."` in `en`) was added across all 11 locale files
for `useChat.ts`'s `runCommand` client-side execute-failure path, but a
review-caught defect initially left it dead-wired: the catch block called the
generic `failureMessageForRequestError` helper instead of this key (and the
test mocked that helper, so the disconnect stayed green). Fixed by rewiring
the catch to call `t("chat.commandFailed")` directly and de-mocking the test
to bind the real translator, so the key is now actually reachable.

### Tests

- WebUI caller tier: member vs admin inventory filtering (including the
  operator-implicit-admin case); `/status` on an owned thread returns the
  rendered view, a foreign thread is indistinguishable from a never-created
  one (both settle to the constant idle view, never a 404); member
  `/model set` gets an `AccessDenied` rejection; an admin's lifecycle-command
  execute attempt still gets `InvalidRequest` (listing-only holds even for
  admins).
- Frontend vitest: existing chat-commands suites plus keyboard navigation and
  metadata rendering; the landing-composer forwarding fix pinned via an
  `EmptyState` test; locale key parity (incl. `chat.commandFailed`); `tsc` +
  conventions lint.
- Descriptor→DTO projection contract pinned.

## PR-3 — Native Slack slash commands

### Transport

Reuse the single signed `[channel.ingress]` `events` route — Slack lets every
slash command point at any Request URL, and the HMAC recipe signs the raw
body regardless of content type; verification happens before content-type
branching, so a forged signature on a form body is rejected at the same
ingress layer as a forged JSON body. No manifest-schema or host changes.
`crates/ironclaw_slack_extension/src/payload.rs` gains `normalize_slack_inbound`,
a sibling entry point that branches on the (host-forwarded) Content-Type
header: `application/x-www-form-urlencoded` → the new slash-command form
parser; anything else, including an absent header, → delegates verbatim to
the existing `normalize_slack_event`, so the two entry points share exactly
one JSON parsing implementation. Inside the form branch, a minimal
all-`Option` probe for `ssl_check` is parsed BEFORE the full slash-command
form: Slack's `ssl_check` endpoint-verification POST carries only
`ssl_check` + `token`, never the mandatory `channel_id`/`user_id`/`command`/
`trigger_id` fields a real invocation requires, so the probe must run first
or the handshake would always fail mandatory-field validation.

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
- A registered command **other than** `/ironclaw` pointed at this same
  Request URL (an app-config mistake — a second Slack slash command
  aimed at the identical signed endpoint) is passed through raw as
  `"{command} {text}"` rather than mapped — the adapter does not guess
  intent; the generic classifier/admission layer rejects the unrecognized
  text as an undeclared command, with role-filtered help.
- Actor from `user_id`, conversation from `channel_id`; event id
  `slack-{installation}-slash-{trigger_id}` (namespaced beside the
  event_callback id space; unique per invocation — Slack does not redeliver
  slash commands, unlike the Events API; cite Slack's docs in the PR).
- **Trigger is derived, never hardcoded**: `DirectChat` only when the slash
  form indicates a genuine DM (`channel_name == "directmessage"` /
  `D`-prefixed `channel_id` — the adapter's existing `is_dm_channel`
  semantics), else `BotCommand` (maps to the Shared route, which the
  direct-conversation admission rejects). Hardcoding `DirectChat` would
  silently defeat both the non-DM rejection edge and the connect-nudge
  gate, which key off the same trigger classification.

Downstream — classification, pairing, PR-1 admission, dispatch, observer
bot-DM delivery — is identical to the space-prefixed path. The ingress 200
is ack-after-durable-admission (command execution itself is synchronous
within the ingress request; only the reply posting is async), normally well
inside Slack's ≤3s rule — the router's 20s deadline ceiling is a
pre-existing degraded-backend exposure worth one line in the PR, not new
risk. The visible result is the bot's DM message.

### Help rendering: per-channel invocation prefix

Neutral help renders `/model`, but typing `/model` bare in Slack fails
(client-intercepted). `ChannelPresentation`
(`crates/ironclaw_host_api/src/channel.rs`) gains an optional
`command_prefix: Option<String>` field, declared under the manifest's
`[channel.presentation]` section beside `supports_markdown` /
`max_message_chars`; Slack's manifest sets `command_prefix = "/ironclaw "` so
help and rejection notices render `/ironclaw model` there. Other channels
leave it `None` and keep the plain `/name` rendering.
`ChannelDescriptor::validate` rejects a declared prefix that is empty, does
not start with `/`, contains a control character, or exceeds 32 bytes
(`ChannelDescriptorError::InvalidCommandPrefix`). The space-typed ` /model`
path keeps working as an undocumented fallback.

### Behavioral edges

- Slash invoked outside the bot DM: flows through the same pipeline; the
  direct-conversation admission rejects, and the observer's command-feedback
  path posts the denial notice straight to the invoking channel
  (`envelope.external_conversation_ref()`) — independent of any
  shared-conversation binding or `slack_allowed_channels` allowlist
  resolution, so it is delivered even to a channel never configured there.
  The user sees nothing only if Slack itself refuses the post (the bot is
  not a member of that specific channel) — accepted MVP limitation, noted in
  the PR. `response_url` delivery would remove even that dependency and is
  the future fix.
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
  permanent `AccessDenied` with the admin-account copy; no backend strings,
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
