# Generic Extension Correctness Merge-Readiness Checklist

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` to execute independent slices, `superpowers:test-driven-development` for every bug fix, and `superpowers:verification-before-completion` before claiming any section complete.

**Goal:** Ship one manifest-driven, multi-tenant extension lifecycle and channel-delivery system that fixes every QA issue reported on 2026-07-21 and 2026-07-22, is proven by caller-level user journeys, and is safe to merge, deploy, and roll back.

**Architecture:** In this PR, the public lifecycle is derived from personal membership plus manifest-declared setup readiness. It exposes only `uninstalled`, `setup_needed`, and `active`; installation, setup completion, reconciliation, and removal perform all internal work automatically. Tenant admin configuration and personal membership are independent authority axes. A narrow typed membership-authority boundary is preserved for a fast-follow explicit tenant-wide deployment policy, but that policy's durable store, API, and admin UI are intentionally out of scope here. Auth, MCP discovery, channel ingress/egress, automation delivery, and lifecycle events use generic typed contracts; provider-specific code is limited to protocol and vendor mechanics. The composition root wires owner-crate services and does not own product policy.

**Tech Stack:** Rust workspace, Reborn product workflow and composition, Axum WebChat v2 API, React/TypeScript/Vite WebUI, libSQL/PostgreSQL persistence, OAuth, MCP, Slack and Telegram adapters, pytest/Playwright, GitHub Actions live canaries.

---

## 0. How to use this document

This document is the release gate for the corrective PR. The PR is merge-ready only when every applicable checkbox is checked and the evidence is recorded in the PR's `Test Strategy` or linked from it.

- [ ] Every checked implementation item names the production file or symbol that owns the behavior.
- [ ] Every checked regression item names the test that failed before the fix and passed after it.
- [ ] Every checked command item records the exact command, exit code, commit SHA, and run date.
- [ ] Every checked live item links the GitHub Actions run or sanitized QA evidence for the exact PR head.
- [ ] A box is not checked based only on code review, a helper unit test, mocked internal state, or a green unrelated CI job.
- [ ] Named P0 user journeys in this document cannot be waived as “not applicable.”
- [ ] Any genuinely inapplicable test tier is recorded in the PR as `Not applicable:` followed by the specific reason.
- [ ] Failures exposed by deleting legacy or redundant code are investigated as potentially load-bearing behavior; assertions are not weakened merely to make the suite green.
- [ ] All evidence is secret-safe and contains no OAuth codes, bearer tokens, bot tokens, client secrets, PII, or unsanitized provider payloads.
- [ ] The final reviewer verifies this checklist against the exact remote PR head, not a stale local branch or earlier green SHA.

## 1. Desired product contract

### 1.1 Public extension lifecycle

- [ ] The only public lifecycle states are `uninstalled`, `setup_needed`, and `active`.
- [ ] `uninstalled` means the user has no personal membership.
- [ ] `setup_needed` means personal membership exists but manifest-declared user setup is incomplete or live readiness has not been proven.
- [ ] `active` means personal membership exists, every required personal setup gate is satisfied, runtime discovery succeeded where required, and the extension's usable surface is published.
- [ ] A no-setup extension moves directly from `uninstalled` to `active` after install.
- [ ] An OAuth, pairing, or credential extension moves from `uninstalled` to `setup_needed`, completes setup, reconciles internally, and becomes `active` without a second user action.
- [ ] Removal is the sole user-visible disable action.
- [ ] There is no separate public `installed`, `configured`, `activating`, `inactive`, `disabled`, or `activation_failed` lifecycle state masquerading as one of the three states.
- [ ] Internal operational diagnostics can report discovery or provider failures without adding another public lifecycle state.
- [ ] A runtime failure never produces a false `active` state.

### 1.2 Current authority separation and fast-follow foundation

- [ ] Personal extension membership is keyed by `(tenant, user, extension)`.
- [ ] Tenant admin configuration is keyed by `(tenant, extension)` and contains app credentials or shared setup declared by the manifest.
- [ ] Configuring an extension as an admin does not install it for the admin or any other user.
- [ ] An admin's personal install/remove behavior is identical to an ordinary user's personal install/remove behavior.
- [ ] Admin/operator role is never inferred to be extension membership.
- [ ] An admin's personal installation is never persisted as tenant ownership or implicit tenant-wide deployment.
- [ ] Tenant admin configuration is never inferred to grant membership, tool access, channel pairing, OAuth readiness, or delivery authority.
- [ ] Tenant admin configuration is exposed only through the extra-privileged Admin Configuration API/page; ordinary-user lifecycle, catalog, setup, model-tool, prompt, blocker, diagnostic, and error projections never contain its fields, handles, labels, values, or remediation instructions.
- [ ] A single typed membership-authority boundary is consumed by lifecycle, tool resolution, ingress, and egress; its production behavior in this PR is strictly caller-scoped personal membership.
- [ ] The future tenant-wide policy can extend that authority boundary without changing public lifecycle states or reintroducing operator-owned installations.
- [ ] No unused speculative store/API/UI or fixture-only branch claims that tenant-wide deployment already exists.
- [ ] A fast-follow issue/design records the future durable `(tenant, extension)` policy store, admin require/unrequire API and UI, removal denial, migration, and A/B setup-isolation journeys.
- [ ] The current PR documentation clearly labels tenant-wide required extensions as unavailable rather than silently approximating them.

### 1.3 Manifest ownership

- [ ] The manifest declares whether tenant admin configuration is required and describes its fields.
- [ ] The manifest declares user setup requirements, including OAuth, proof-code pairing, credentials, and no-setup cases.
- [ ] The manifest declares OAuth requirement keys, provider/account recipe, requested scopes, callback recipe, and tool-discovery limits.
- [ ] The manifest declares channel pairing presentation such as QR, deep link, countdown, and allowed command semantics.
- [ ] The manifest declares runtime and capability metadata used by the generic lifecycle and UI.
- [ ] Backend, frontend, CLI, canaries, and tests consume the same manifest-derived contract rather than maintaining divergent provider tables.
- [ ] Provider-specific code exists only for genuine vendor protocol mechanics, not lifecycle policy or presentation branching that the manifest can express.

## 2. Reported bug acceptance ledger

Every item in this section corresponds to a reported failure. Checking a box means the exact user-visible failure is reproduced by a regression and is fixed through the production caller path.

### 2.1 Admin configuration, installation, and tenant deployment

- [ ] Operator navigation exposes `/v2/admin/configuration` and it loads for an authenticated operator after a clean build and hard refresh.
- [ ] Admin Configuration shows every manifest-declared extension configuration group, including Slack, Telegram, Google, and non-channel extensions that require tenant setup.
- [ ] Admin Configuration never offers personal installation, activation, pairing, or removal actions.
- [ ] `/v2/extensions/channels` contains install/remove/setup flows for channel extensions.
- [ ] `/v2/extensions/registry` contains the complete extension catalog.
- [ ] Personal installation occurs only from user extension surfaces or model-visible lifecycle capabilities.
- [ ] Admin Configuration clearly distinguishes tenant setup from the admin's own personal membership and does not imply that tenant-wide deployment is already available.
- [ ] The missing tenant-wide deployment control is linked as a fast follow rather than implemented through the old tenant-ownership shortcut.
- [ ] Admin configuration is possible before any user installs the extension.
- [ ] A personal setup response/modal contains only manifest-declared caller setup requirements (OAuth, pairing, or personal credentials); it never contains tenant-admin configuration fields.
- [ ] Provider-issued/runtime-derived metadata such as workspace, app, installation, or bot-user identifiers is not rendered as editable personal setup input.
- [ ] Tenant routing and admission values such as allowed channels, shared subjects, and subject routes appear only on the authorized admin configuration surface, never in a user's install/setup modal.
- [ ] Slack's personal setup modal offers only the caller's OAuth connect/reconnect action after install; it never exposes bot tokens, signing secrets, OAuth client credentials, workspace/app/installation IDs, or routing JSON.
- [ ] The admin-versus-personal setup separation is derived from manifest ownership in the backend projection and has no Slack/Telegram/Notion-specific frontend allowlist or field filter.
- [ ] Ordinary-user wire-contract tests assert that known admin handles and labels are absent, rather than relying only on the frontend to hide them.
- [ ] Repeated secret paste, including three or more consecutive `Ctrl+V` operations, does not blank the page.
- [ ] Secret inputs never hydrate a sentinel, masked secret, or previously stored secret back into the browser.
- [ ] Background refetch preserves unsaved dirty admin fields and does not overwrite a newly pasted secret.
- [ ] Saving one configuration field does not silently clear another secret field.
- [ ] Admin configuration persistence is isolated by tenant.
- [ ] The admin page, user install modal, lifecycle API, and ingress preflight read the same authoritative tenant-configuration projection; a completed Telegram/Slack admin setup cannot appear as `not_configured` elsewhere.

### 2.2 Lifecycle and activation failures

- [ ] No WebUI card, modal, CLI command, model capability, or API route asks the user to `Activate` an extension.
- [ ] The model-visible `extension_activate` capability is absent from the product command registry and model tool surface.
- [ ] The old `POST /api/webchat/v2/extensions/{id}/activate` route returns `404`, not a hidden compatibility success or `503`.
- [ ] Stale onboarding, package, catalog, prompt, and help copy no longer says “then activate.”
- [ ] Installing a no-setup skill or tool returns `active` and exposes its capability in the same completed lifecycle operation.
- [ ] Installing an OAuth extension returns `setup_needed` and starts or offers the correct OAuth flow.
- [ ] Successful OAuth callback performs internal readiness reconciliation and returns the extension as `active` with every manifest-declared usable surface published; tool-bearing manifests additionally expose their discovered or declared tools.
- [ ] Every frontend OAuth status/reconcile request targets a backend route that is actually registered and covered through the real router; the current mocked-test-hidden `POST .../flow/{id}/reconcile` 404 cannot recur.
- [ ] Installing a pairing extension returns `setup_needed` until pairing is complete, then automatically becomes `active`.
- [ ] Removing and reinstalling an extension starts from a clean personal lifecycle without reusing revoked setup state unless the provider credential is intentionally reusable under the manifest contract.
- [ ] A malformed lifecycle tool call returns a stable, model-correctable validation result; it never fails with “the tool input could not be encoded.”
- [ ] Model-visible extension search finds every eligible manifest-backed catalog entry, including Slack and Telegram before personal installation.
- [ ] Model-requested install uses the same canonical lifecycle as WebUI install and returns a real setup gate when personal OAuth/pairing is required.
- [ ] An active extension's discovered tools are present in the model's next tool surface, so the agent does not claim an installed extension has no usable capabilities when tools exist.

### 2.3 Notion and generic hosted MCP failures

- [ ] Notion follows the same generic OAuth lifecycle as every other OAuth MCP extension: install -> `setup_needed` -> OAuth -> `active`.
- [ ] Returning from Notion OAuth never leaves the card at a misleading “installed” state with an Activate button.
- [ ] No post-OAuth Notion action returns `503 Service Unavailable` merely because the UI attempted a redundant activation.
- [ ] The official Notion MCP `tools/list` shape with 24 tools and bounded deep schemas is accepted by the generic MCP client.
- [ ] The regression proves schemas of at least the formerly failing depth 10 are accepted; it is not a Notion-specific special case.
- [ ] Manifest `max_tools` is honored and capped by the host maximum of 1024 tools.
- [ ] MCP discovery remains bounded by the 2 MiB response-body cap and documented schema depth, node, and string limits.
- [ ] MCP parser failures retain a stable diagnostic subcause instead of collapsing every failure into `mcp_invalid_tool_list`.
- [ ] A single invalid tool does not silently invalidate unrelated valid tools unless the contract intentionally fails the whole catalog and explains why.
- [ ] Zero live-discovered tools, failed discovery, or only hidden template tools cannot publish `active`.
- [ ] A transient MCP discovery failure preserves valid user OAuth credentials, leaves the extension `setup_needed`, and permits bounded readiness retry without forcing the user through OAuth again.
- [ ] A permanent or policy-invalid MCP discovery failure records a stable safe cause and follows explicit credential-retention/revocation policy rather than compensating every discovery error identically.
- [ ] An active hosted MCP exposes its model-visible tools in the next caller-visible tool surface without restart or reinstall.
- [ ] The model never asks the user to paste a Notion internal integration token when the manifest specifies OAuth.
- [ ] The generic hosted MCP regression also covers at least one non-Notion extension to prove the fix is not provider-specific.

### 2.4 Slack setup, OAuth, install, and removal failures

- [ ] Tenant admin configures only Slack app-level values such as client ID, client secret, signing secret, and manifest-declared app metadata.
- [ ] Each user independently installs Slack and completes personal OAuth.
- [ ] Slack user OAuth is never described as a tenant-admin OAuth action.
- [ ] OAuth start accepts only the manifest requirement key from the browser; provider, account label, scopes, and callback data are server-derived.
- [ ] Cross-extension OAuth requirement substitution and browser-forged provider/scopes are rejected.
- [ ] Slack install/connection never fails with “the tool input could not be encoded.”
- [ ] A tool or transport error cannot cause the model to invent tenant-admin, shared-tool, bot-token, or user-token setup instructions.
- [ ] A regular user can remove their personal Slack installation through WebUI.
- [ ] `POST /api/webchat/v2/extensions/slack/remove` returns success for a removable personal installation rather than `400 invalid_request`.
- [ ] Removing Slack for user A does not remove Slack for user B or alter tenant admin configuration.
- [ ] An operator can personally install and remove Slack without the installation being persisted as tenant-owned.
- [ ] Slack personal removal through Slack chat, Telegram chat, WebUI, or another model surface invokes the same canonical lifecycle operation.
- [ ] Slack OAuth callback, event subscription, and request URLs match the current canonical routes.
- [ ] Legacy Slack callback/webhook routes, obsolete provider IDs such as retired `slack_personal`, and obsolete environment variables are removed or explicitly migrated.
- [ ] QA, canary, and local development use distinct Slack apps/bots and credentials; no live bot is shared across these environments.
- [ ] Slack setup APIs used by canaries do not return 404 because the harness calls a deleted or legacy route.
- [ ] Slack personal OAuth preflight uses the current manifest provider ID and reports ready when correctly configured.

### 2.5 Telegram setup, pairing, and channel UX failures

- [ ] Telegram tenant configuration makes the bot able to receive inbound messages before a user installs or pairs the extension.
- [ ] An uninstalled or unpaired user messaging the configured bot receives the correct install/connect/pair guidance.
- [ ] Telegram supports `/start` as the current pairing entry command.
- [ ] No UI, model prompt, bot response, manifest, canary, or help text instructs the user to send `/pair`.
- [ ] `/start` triggers or resumes the actual pairing journey rather than entering the text into the agent loop as ordinary user content.
- [ ] The pairing modal preserves the existing countdown, QR code, Telegram deep link, and “open in Telegram” interaction.
- [ ] The QR code and deep link represent the current fresh pairing claim and cannot bind a stale or different user's claim.
- [ ] Pairing code consumption is scoped to tenant and user, is single-winner under concurrency, and cannot be replayed.
- [ ] Failed or expired pairing offers a fresh code through the supported `/start` flow and does not trap the user in a loop.
- [ ] A paired Telegram inbound turn emits one “Ironclaw is thinking...” indication from the Submitted lifecycle event.
- [ ] The thinking indication is retracted or finalized when the run completes, blocks, fails, or is canceled.
- [ ] Telegram receives the final answer exactly once.
- [ ] Unpair/re-pair works in a fresh thread through real HTTP/webhook-shaped ingress.
- [ ] User A and user B can pair independently to distinct Telegram identities.
- [ ] Logging out, switching WebUI accounts, or cycling test accounts never transfers a prior user's channel identity binding or lifecycle projection to the new session.
- [ ] WebUI installation state and channel ingress authorization are derived from the same effective-membership authority; the bot cannot continue normal replies while that user is displayed as uninstalled.
- [ ] Removing Telegram revokes the removed user's active pairing and delayed delivery rights.
- [ ] After removal, the bot does not continue normal agent conversation for that user; if tenant configuration keeps ingress available, it returns only the safe reconnect/install guidance.
- [ ] Telegram removal from Slack, WebUI, Telegram, or another model surface has identical canonical effects.

### 2.6 Slack channel UX parity

- [ ] A configured Slack app can receive inbound messages before the sender installs/connects the personal extension.
- [ ] An uninstalled or unconnected Slack user receives a safe personal connect message rather than silence.
- [ ] The connect response exposes the real user OAuth path and does not instruct the user to ask an admin for personal OAuth.
- [ ] A connected Slack inbound turn emits one thinking indication from the same generic Submitted lifecycle event used by Telegram.
- [ ] Slack receives the final answer exactly once through the same generic delivery coordinator.
- [ ] Shared Slack targets never receive a private OAuth URL; they receive safe no-link guidance.
- [ ] Direct Slack targets receive an OAuth URL only when the current user and sealed route are authorized.

### 2.7 Delayed OAuth final-answer delivery

- [ ] A channel-originated run can remain blocked on OAuth longer than the former observer timeout and still deliver the final answer to the exact source channel after OAuth completes.
- [ ] The final answer shown in WebUI is also delivered to the originating Telegram chat.
- [ ] The same delayed-auth behavior is provider-neutral and works for Slack.
- [ ] Delivery is driven by durable lifecycle events rather than a per-run polling watcher, timeout, or held concurrency permit.
- [ ] `BlockedAuth` emits exactly one prompt per gate reference.
- [ ] `BlockedApproval` emits exactly one prompt per gate reference.
- [ ] Duplicate lifecycle events do not duplicate auth prompts, approval prompts, thinking messages, or final answers.
- [ ] OAuth callback reconciles extension readiness before continuation fan-out resumes the blocked turn.
- [ ] The sealed source route survives process/service reopen and is not reconstructed from display strings.
- [ ] Immediately before egress, the system revalidates tenant, user, thread, adapter, installation, current personal membership, and current pairing/OAuth readiness.
- [ ] A removed or unpaired extension cannot receive a delayed final that was queued before removal.
- [ ] Reinstalling an extension cannot cause a delayed final to leak to a stale installation or adapter binding.

### 2.8 Automation and Delivery Defaults failures

- [ ] A routine created from Telegram inherits the exact sealed source reply target when no explicit delivery target is provided.
- [ ] A routine created from Slack inherits the exact sealed source reply target when no explicit delivery target is provided.
- [ ] An explicit delivery target takes precedence over source inheritance.
- [ ] If no target can be resolved safely, routine creation or delivery fails closed with correct guidance rather than claiming success.
- [ ] Triggered results use the same generic delivery coordinator as ordinary run completion.
- [ ] Triggered delivery is creator-scoped and revalidates membership, pairing, OAuth, and binding immediately before send.
- [ ] Removing or unpairing a channel prevents later automation delivery to that channel.
- [ ] Duplicate trigger execution or delivery retry does not duplicate the external message.
- [ ] Permanent provider send failures do not cause an infinite retry storm.
- [ ] Transient send failures retry according to bounded policy and recover without duplicate delivery.
- [ ] Delivery Defaults lists currently valid external targets for active Slack and Telegram installations.
- [ ] Delivery Defaults does not show stale, removed, unpaired, cross-user, or cross-tenant targets.
- [ ] “Web app only” remains available but is not the only option when valid external targets exist.
- [ ] The model does not invent a pairing code workflow when no corresponding target can be created in the UI or product API.
- [ ] Trigger execution contains no polling watcher with an arbitrary completion timeout; if any remains, it is a merge blocker.

### 2.9 Routine self-mutation safety

- [ ] A scheduled routine cannot create another routine while executing.
- [ ] A scheduled routine cannot update, pause, resume, remove, clone, or duplicate any routine while executing.
- [ ] The denial is enforced at the trigger-management capability boundary using typed invocation origin, not prompt wording.
- [ ] The guard applies to all model, retry, resume, and nested execution paths.
- [ ] A normal interactive user turn retains the authorized routine-management capabilities.
- [ ] The regression proves no new durable routine record or mutation occurs after the denied call.

### 2.10 Authentication UI guidance

- [ ] The WebUI auth card does not tell users to “Open settings” when the action is available directly.
- [ ] The card presents the actual OAuth action/link or manifest-declared setup action.
- [ ] Cancel denies the exact gate and produces one clear terminal response.
- [ ] Auth URLs, codes, and private remediation data are redacted from shared targets, logs, traces, and model-visible failure text.
- [ ] Product errors distinguish transient provider failure, misconfiguration, policy denial, and permanent failure with stable semantics.

## 3. Generic lifecycle implementation gates

### 3.1 Removal of public activation

- [ ] `rg -n --glob '!**/*.md' --glob '!tests/fixtures/**' 'extension_activate|activate_extension|ExtensionActivate|/activate|Activate extension|activateExtension|(^|[^[:alnum:]_])activate([[:space:]]*:|[[:space:]]*=|[[:space:]]*\()' crates tests scripts Cargo.toml` scans executable, API, frontend, and configuration sources without treating design history as live surface; every remaining result is individually classified as an internal runtime checkpoint, unrelated terminology, or a public lifecycle hit to remove.
- [ ] Public Rust enums and DTOs do not contain an Activate operation or state.
- [ ] WebChat v2 route descriptors and router do not register an activate route.
- [ ] Frontend schemas, hooks, mutations, actions, cards, and modals do not contain an activate action.
- [ ] CLI help and command registration do not expose activate.
- [ ] Model-visible capability catalogs and prompt instructions do not expose activate.
- [ ] Recorded fixtures and canary cases do not expect activate.
- [ ] Old activate clients fail visibly with 404 and documented migration guidance; the server does not silently reinterpret the request.

### 3.2 Canonical reconciliation

- [ ] Install creates caller-scoped personal membership and invokes one internal readiness reconciliation path.
- [ ] OAuth completion invokes the same internal readiness reconciliation path.
- [ ] The internal reconcile API/command is genuinely wired end-to-end through the production router or durable worker; frontend mocks cannot invent a route the backend does not serve.
- [ ] Pairing completion invokes the same internal readiness reconciliation path.
- [ ] Credential save invokes the same internal readiness reconciliation path.
- [ ] Reopen/startup recovery can reconcile incomplete records without inventing membership.
- [ ] Reconciliation is idempotent under retries and concurrent callbacks.
- [ ] Reconciliation uses bounded CAS or a backend transaction for read-modify-write; it does not hold a process-local mutex across backend I/O.
- [ ] Reconciliation publishes usable capabilities only after readiness evidence is durable.
- [ ] Partial failure does not leave membership, credentials, published tools, and UI projection in contradictory states.
- [ ] Errors preserve their cause and safe remediation while redacting secrets.

### 3.3 Legacy state handling

- [ ] Existing personal installations have explicit forward migration or blank-slate reset semantics.
- [ ] Existing tenant-owned installation rows are not silently reinterpreted as explicit tenant-required policy.
- [ ] Legacy activation/configuration fields are removed, ignored with a tested compatibility window, or migrated exactly once.
- [ ] PostgreSQL and libSQL receive equivalent schema/data handling.
- [ ] Migration/reset is restart-safe and idempotent.
- [ ] Rollback behavior is documented, including what new state an old binary would not understand.
- [ ] QA and production reset requirements name the exact data classes affected without erasing unrelated tenant/user data by accident.

## 4. Multi-tenant and security gates

- [ ] User A install does not affect user B lifecycle projection.
- [ ] User A OAuth credentials cannot be resolved for user B.
- [ ] User A pairing cannot be claimed, listed, or used by user B.
- [ ] User A removal does not affect user B membership, tools, pairings, targets, or messages.
- [ ] Operator personal state is isolated from users A and B.
- [ ] Tenant A configuration, memberships, policies, and bindings are inaccessible to tenant B.
- [ ] Every lifecycle mutation authorizes the authenticated actor at the product boundary.
- [ ] Every admin configuration and tenant-policy mutation requires the extra admin privilege.
- [ ] Public clients cannot mint trusted actor, tenant, thread, installation, adapter, or reply-target identities.
- [ ] OAuth `state` binds tenant, user, extension requirement, flow, invocation, and callback nonce.
- [ ] Pairing claims bind tenant, user, extension, installation, and expiry.
- [ ] External ingress validates original payload size, signature, replay identity, and actor binding before persistence or prompt construction.
- [ ] External egress crosses `ironclaw_network`/outbound mediation and never injects credentials client-side.
- [ ] OAuth tokens, bot tokens, app secrets, signing secrets, and bearer tokens remain encrypted/host-side and are redacted everywhere else.
- [ ] Shared channel targets never receive private authentication material.
- [ ] Removal revokes future tool resolution and external delivery before reporting durable success.
- [ ] Side-effecting success requires durable/provider evidence plus read-back verification, or is labeled explicitly unverified.

## 5. Dedicated architecture-conformance audit

This is an independent merge gate against `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`, not a style review and not satisfied solely by `cargo test -p ironclaw_architecture`. The document describes a multi-slice end state: untouched pre-existing debt may be recorded as baseline, but this PR must not grow any frozen debt, and every new or substantially rewritten path must follow the target architecture now.

### 5.1 Audit setup and output

- [ ] An auditor who did not author the relevant implementation slice performs the audit after the implementation diff stabilizes.
- [ ] The audit records the exact PR head SHA and merge base from `git merge-base origin/main HEAD`.
- [ ] The audit covers committed changes plus every staged/untracked production file that will enter the PR.
- [ ] A companion report is materialized at `docs/superpowers/plans/2026-07-22-generic-extension-correctness-architecture-audit.md`.
- [ ] Every changed production symbol is classified against the document's five routing surfaces: `ProductSurface`, `AgentLoopHost`, `authorize + dispatch`, `RuntimeLane`, or a substrate port; pure deployment selection is classified as data and composition as assembly.
- [ ] Each finding is labeled `PASS`, `PRE-EXISTING UNCHANGED DEBT`, or `PR BLOCKER`, with file/symbol evidence and the exact search or trace used.
- [ ] Pre-existing debt is only non-blocking when the auditor proves the PR neither adds a new instance nor expands the existing footprint.
- [ ] Every `PR BLOCKER` is fixed and the affected audit row is rerun before merge; no architecture blocker is deferred merely because tests are green.

### 5.2 DTO and type-flow audit: fewer mirrors, named real transitions

- [ ] Every added or materially changed request, response, state, record, projection, wire type, and `From` conversion is inventoried.
- [ ] Each type owns a genuinely distinct state, trust transition, external wire contract, durable record, or runtime-lane boundary; types that only rewrap another type field-for-field are removed.
- [ ] Loop-expressed, authorized, and lane-resolved states remain visibly distinct where the architecture requires them; no extra mid-flight mirror is introduced between those real transitions.
- [ ] Additional context is threaded through the canonical owner type or by reference rather than by cloning an almost-identical DTO in product workflow or composition.
- [ ] Neutral actor, scope, origin, activity, authorization, and resolution vocabulary uses the existing canonical `ironclaw_host_api` types rather than local mirrors.
- [ ] Product/UI wire types deserialize into strong owner-domain types at the boundary and do not leak raw JSON/string lifecycle states into internal flow.
- [ ] `python3 scripts/check-type-duplicates.py` is run and every new candidate pair involving a changed type is classified in the audit report.
- [ ] The changed diff is searched for identity conversions, same-field structs, `*Request`/`*Result`/`*Dto` families, and adjacent types differing only by one optional field.
- [ ] No new type is added to `ironclaw_host_api` unless it is neutral authority vocabulary whose addition is justified as a security-model change rather than a product feature.
- [ ] Existing capability-DTO-collapse and facade-method ratchets do not grow; any shrink is reflected in their checked-in allowlists.

### 5.3 `dyn`, traits, decorators, and optional-service audit

- [ ] Every added or materially changed trait and every new `Arc<dyn ...>`, `Box<dyn ...>`, `&dyn ...`, generic mediator, and decorator is inventoried.
- [ ] A production trait object is retained only for genuine runtime polymorphism, a real trust boundary, or a lower-owner dependency-inversion port implemented above it.
- [ ] A test seam with one production implementation uses a generic, boundary fake, or existing owner port where appropriate rather than imposing a new hot-path production vtable.
- [ ] Every retained trait-object row records the number of production implementations, number of decorators, and exact reason static dispatch or a closed enum is wrong.
- [ ] Closed security-sensitive sets such as runtime lanes remain exhaustive enums; adding tools/extensions as data does not create a new open runtime-lane trait.
- [ ] No `Option<Arc<dyn ...>>`, optional builder field, or `with_*` setter represents a dependency that every production composition always supplies.
- [ ] No trait mirrors another trait method-for-method solely to cross a trusted internal crate seam.
- [ ] Trait changes enumerate and update every production implementation, decorator, adapter, and test double, and validation exercises the complete production wrapper chain.
- [ ] The audit explicitly checks that the new membership-authority foundation is a live, correctly owned dependency-inversion boundary rather than speculative ceremony.

### 5.4 Store/backend and local-specific-struct audit

- [ ] The diff adds no `InMemory*Store` or parallel test-only implementation of domain semantics; tests use the production store over `InMemoryBackend` where the shared filesystem seam supports it.
- [ ] The diff adds no `LocalDev*`, `Hosted*`, `Enterprise*`, or mode-specific public type encoding deployment policy as kernel vocabulary.
- [ ] Backend selection and environment differences are values/configuration at the composition edge, not branches replicated through lifecycle, auth, MCP, channel, or delivery types.
- [ ] Any new local/test constructor delegates to the same production implementation and differs only by injected backend or inert boundary double.
- [ ] libSQL, PostgreSQL, disk, and in-memory behavior do not acquire divergent lifecycle, membership, retry, or delivery rules.
- [ ] Read-modify-write behavior uses the shared bounded CAS/transaction mechanisms rather than a process-local mutex held across backend I/O.
- [ ] No synchronous lock is held across remote `.await`; relevant changed paths are reviewed for `await_holding_lock`, nested store locks, and heartbeat/lease contention.
- [ ] Existing in-memory-store, LocalDev-type, deployment-mode-type, and deployment-mode-branching ratchets do not grow.

### 5.5 Composition-root audit: assembly only

- [ ] Every changed file under `crates/ironclaw_reborn_composition` is reviewed symbol-by-symbol and classified as assembly, adapter registration, configuration selection, or misplaced domain/product behavior.
- [ ] Composition does not own extension lifecycle state transitions, readiness policy, OAuth continuation policy, MCP discovery classification, channel ingress/egress policy, delivery retries/idempotency, automation policy, or membership authorization.
- [ ] Composition wires owner-crate factories/builders/ports; conditionals that decide domain outcomes live behind a factory or service in the owning crate.
- [ ] New host-generic auth behavior lives in one auth engine/service with manifest recipe data; composition does not add vendor-specific OAuth flow logic.
- [ ] Product and channel identifiers, protocols, routes, rendering, and provider mechanics remain in adapters/registries rather than new composition-owned feature branches.
- [ ] Automation uses the same canonical invocation and lifecycle-event delivery machinery; composition does not create a parallel scheduler, dispatcher, watcher, or retry loop.
- [ ] Product mutations use existing generic capability/invoke conduits and do not add a new feature-shaped method to the frozen product facade.
- [ ] Composition receives descriptor sets and product-neutral registries as inputs and fails closed on unresolved configured IDs instead of silently skipping them.
- [ ] The audit reports composition line/symbol additions versus deletions by category and proves the PR does not grow the product-policy footprint.
- [ ] If a feature requires editing composition beyond declarative wiring, the audit identifies the owner crate and the behavior is moved before merge.

### 5.6 Extension, channel, and trust-boundary audit

- [ ] Declarative extension metadata lives in `ironclaw_extensions` or first-party manifest assets, not composition/frontend switch statements.
- [ ] MCP execution remains in the MCP runtime lane, receives only mediated ports, and crosses host network/secret boundaries without exposing ambient authority.
- [ ] Channel protocol mechanics remain in adapters; generic ingress, lifecycle, identity, membership, and delivery policy are provider-neutral.
- [ ] Runtime-lane output, MCP schemas, provider responses, channel payloads, and worker metadata remain untrusted, bounded, validated, invocation-bound, and redacted before reaching trusted flow or the model.
- [ ] Durable events, projections, transport streams, and provider sends remain distinct contracts.
- [ ] The trusted conversation/source route is stored by the conversation owner and resolved through typed ports rather than reconstructed from display strings or transport metadata.
- [ ] Product, loop, and automation origins are sealed by their ingress membranes and feed the same authorization/dispatch path; none can self-assert another origin.
- [ ] No new lower-level crate depends upward on product workflow, WebUI, or composition.
- [ ] No auth, lifecycle, channel, automation, or extension path bypasses authorization, approvals, obligations, mediated egress, or durable idempotency.
- [ ] Recoverable extension/provider failures remain model-visible outcomes with remediation; genuine host failure remains narrow and does not become a dumping ground for user-correctable errors.

### 5.7 State-machine and performance conformance

- [ ] Lifecycle, OAuth continuation, pairing, event delivery, automation delivery, and removal transitions use closed strong types with explicit rejection; new wildcard/catch-all arms cannot silently swallow a state.
- [ ] Replay/idempotency identities are stable and side effects are at-most-once across callback retry, duplicate events, reconnect, process restart, and automation retry.
- [ ] A blocked/gated operation holds no unnecessary concurrency permit or arbitrary per-run watcher budget while waiting for external resolution.
- [ ] Durable state transition/event append ordering cannot commit authoritative state while losing the lifecycle event required for delivery, or the exception is explicitly proven recoverable.
- [ ] Event fan-out is bounded and slow channel subscribers cannot block canonical turn state transitions.
- [ ] Remote-store-sensitive operations avoid sequential round trips and locks where the changed path can batch or reuse authoritative reads safely.
- [ ] Caller-level tests exercise the production composition edge for every new transition that controls auth, membership, tool publication, removal, or external delivery.

### 5.8 Mechanical evidence and sign-off

- [ ] `cargo test -p ironclaw_architecture` passes on the exact PR head.
- [ ] The specific ratchets for capability DTO collapse, facade methods, origin/gate matrices, in-memory stores, LocalDev/deployment-mode types, and deployment-mode branching pass.
- [ ] `scripts/pre-commit-safety.sh` passes after the architecture audit fixes.
- [ ] The audit report includes a before/after table for changed mirror DTOs, changed production `dyn` seams, changed local/mode-specific types, and changed composition-owned policy symbols.
- [ ] The audit report explicitly confirms whether this PR moved the implementation closer to, unchanged from, or farther from each relevant design axis.
- [ ] A reviewer signs off: `ARCHITECTURE PASS: no new mirror DTO, unjustified dyn, local-specific store/type, or composition-owned core policy at the verified full SHA`.

## 6. Clean-surface and bad-code-smell gates

### 6.1 Genericity and legacy removal

- [ ] No generic lifecycle or frontend code branches on `notion`, `slack`, `telegram`, `gmail`, or another extension ID when manifest data can express the behavior.
- [ ] Any remaining provider-name branch is documented as unavoidable vendor protocol mechanics and tested through its adapter.
- [ ] Legacy bespoke Slack/Telegram host paths, callback routes, webhook routes, provider IDs, feature flags, frontend panels, env vars, and setup APIs superseded by the generic architecture are deleted.
- [ ] Deleted legacy symbols have no stale references in code, tests, docs, scripts, workflows, fixtures, or deployment config.
- [ ] No compatibility shim silently preserves the removed Activate lifecycle.
- [ ] No hidden static tool template makes an extension look active after live MCP discovery failed.
- [ ] No admin configuration record is used as a substitute for personal membership.
- [ ] No operator role or admin configuration is used as a substitute for personal membership or a future tenant-wide policy.
- [ ] No per-run watcher, sleep loop, timeout observer, or held concurrency permit owns eventual channel delivery.

### 6.2 Dead code and over-engineering

- [ ] Every added/retained public type, trait, function, constant, field, and variant has a verified production caller or a legitimate used test/support role.
- [ ] There is no added `#[allow(dead_code)]` masking unused implementation.
- [ ] No `Noop*`, `Unsupported*`, `Fake*`, `Static*`, builder method, enum variant, or struct field exists without a real wiring or test consumer.
- [ ] No test is the only caller of dead production code.
- [ ] No test pins a deliberately deleted lifecycle surface.
- [ ] No duplicate test repeats an existing scenario without adding a distinct risk or seam.
- [ ] No identity `From` conversion or mirror DTO adds ceremony without a trust or state transition.
- [ ] No generic parameter exists that production never varies.
- [ ] Negative dead-code claims record the exact symbol search used to prove zero callers.

### 6.3 Rust and TypeScript hygiene

- [ ] Changed production Rust contains no `.unwrap()` or `.expect()`.
- [ ] Changed Rust contains no suspicious unchecked byte slicing of external/user UTF-8.
- [ ] External case-insensitive identifiers are normalized at the boundary without corrupting opaque case-sensitive values.
- [ ] Known domain states use enums/strong types rather than raw strings.
- [ ] Cross-module Rust imports prefer `crate::` where appropriate.
- [ ] Errors retain source causes and map to stable user/model-visible categories.
- [ ] New collections, queues, schemas, response bodies, retries, and counters are bounded and use overflow-safe arithmetic.
- [ ] Frontend code adds no `@ts-nocheck`, unjustified `@ts-ignore`, broad `any`, non-null assertion hiding invalid state, or swallowed promise rejection.
- [ ] React event handlers capture event values synchronously before deferred state updates.
- [ ] Secret values are never stored in long-lived client state beyond the active edit and submit lifecycle.
- [ ] There are no hardcoded temp paths, user paths, QA URLs, tokens, app IDs, provider IDs, or environment-specific secrets in production or fixtures.
- [ ] Comments promise only guarantees enforced by code and tests.

### 6.4 Scope and maintainability

- [ ] The diff contains no unrelated formatting, generated files, dependency upgrades, or drive-by refactors.
- [ ] Final diff statistics separate production code, tests/fixtures, documentation, generated assets, and deleted legacy code; aggregate line count is never used to hide production growth.
- [ ] Every production file with a material net addition has a symbol-level justification tied to a distinct state, trust boundary, durable record, protocol requirement, or regression fix.
- [ ] The three-state lifecycle cutover is net subtractive in its public/API/model/CLI/frontend surface; any total production growth comes from separately justified missing behavior rather than preserving old and new lifecycle machinery together.
- [ ] New replacement subsystems include the deletion of their predecessor implementation, settings, tasks, tests, and compatibility shims in the same final diff.
- [ ] The final architecture report includes the top production additions and deletions and identifies any opportunity to collapse duplicate live/automation/channel paths before merge.
- [ ] Files and functions remain focused; large orchestration logic is moved behind the correct owner without creating speculative abstractions.
- [ ] Repeated logic is consolidated only after tests prove the preserved behavior of every entry path.
- [ ] Removed backstops are checked across install, callback, pairing, resume, retry, reopen, and removal paths.
- [ ] New behavior is the smallest implementation that satisfies the tests; speculative future lifecycle states and extension-specific flags are absent.
- [ ] Public API naming describes current semantics rather than historical activation terminology.

## 7. Test-first and coverage-quality gates

### 7.1 TDD discipline

- [ ] Every reported bug has a regression that was observed failing for the expected reason before its fix.
- [ ] Red-phase evidence is recorded as a test name plus failure assertion, not merely “test failed.”
- [ ] Tests exercise the highest deterministic production seam that can observe the user behavior.
- [ ] Helper tests supplement but never replace caller-level coverage for side-effect gates, membership, egress, auth, persistence, or dispatch.
- [ ] External providers are represented by hermetic protocol doubles in deterministic suites; live canaries are supplemental.
- [ ] Tests use real in-memory/durable implementations where practical rather than mocks that duplicate the implementation.
- [ ] Timing tests use deterministic event control, bounded virtual time, or explicit synchronization rather than flaky sleeps.
- [ ] Tests assert durable state or captured mediated side effects, not only `Completed` status or mock call counts.
- [ ] Denial, cancellation, duplicate, restart, conflict, partial failure, stale identity, wrong user, wrong tenant, and redaction edges are covered where applicable.
- [ ] No new test is ignored, TODO-pinned, dependent on local keychain, or silently skipped in CI.

### 7.2 Required P0 user journeys

- [ ] `no_setup_install_becomes_active_without_activate`: install a no-setup extension and prove capability availability in the same lifecycle.
- [ ] `oauth_install_callback_becomes_active_without_activate`: install OAuth extension, observe setup gate, complete callback, prove active/tools with no second action.
- [ ] `hosted_mcp_deep_schema_tools_are_published`: discover the real Notion-shaped 24-tool/deep-schema response and prove model-visible tools.
- [ ] `slack_personal_oauth_is_not_admin_oauth`: admin config exists, user A installs/connects, user B remains separate.
- [ ] `personal_setup_excludes_admin_configuration`: the real setup handler projects only caller OAuth/pairing/credentials and excludes admin secrets, provider-derived metadata, and routing fields for a manifest-backed channel extension.
- [ ] `extension_user_lifecycle_isolation`: Alice install/remove does not affect Bob; operator personal install/remove is ordinary; admin config persists.
- [ ] `admin_config_does_not_install_or_grant_membership`: admin config persists while users A, B, and operator remain independently uninstalled until their own install.
- [ ] `operator_personal_install_remove_is_user_scoped`: operator install/remove changes only the operator's membership and never creates tenant ownership.
- [ ] `telegram_unpaired_inbound_prompts_start`: configured bot receives unpaired inbound and sends correct `/start` guidance.
- [ ] `telegram_start_pairing_round_trip`: `/start` creates/consumes fresh single-use claim and enables the exact user.
- [ ] `slack_unconnected_inbound_prompts_personal_oauth`: configured app responds to an unconnected user with safe personal OAuth guidance.
- [ ] `external_channel_delivers_final_after_oauth_outlives_delivery_poll_window`: delayed OAuth completion delivers the final once to the sealed source route.
- [ ] `duplicate_lifecycle_events_deliver_once`: duplicate Submitted/Blocked/Completed events produce one thinking/prompt/final sequence.
- [ ] `channel_removal_revokes_delayed_delivery`: removal/unpair before completion blocks the queued final.
- [ ] `cross_surface_remove_is_canonical`: removal requested through Slack/WebUI/model surface produces identical membership, pairing, tool, and target effects.
- [ ] `external_source_trigger_captures_delivery`: routine created from Telegram/Slack inherits source target and external result is observed.
- [ ] `trigger_delivery_revalidates_removed_target`: remove/unpair after schedule creation and prove no external send.
- [ ] `permanent_channel_send_failure_does_not_retry_forever`: bounded retry and later recovery without duplicates.
- [ ] `trigger_self_create_denied`: scheduled routine cannot create another routine and durable routine count is unchanged.
- [ ] `trigger_self_mutation_denied`: scheduled routine cannot update/pause/resume/remove any routine.
- [ ] `admin_secret_repeated_paste_survives_refetch`: repeated paste and background refetch retain correct dirty values without secret hydration.
- [ ] `auth_card_offers_direct_action`: browser journey proves no incorrect Settings instruction.
- [ ] `delivery_defaults_lists_active_external_targets`: WebUI lists only the current user's valid Slack/Telegram targets.
- [ ] `pairing_claim_isolation_and_single_winner`: wrong user loses and concurrent consume has exactly one winner.
- [ ] `unpair_repair_fresh_thread_http_journey`: real route/webhook-shaped journey succeeds after fresh pairing.

### 7.3 Deleted/refactored test parity

- [ ] The authoritative deleted-test audit is attached or linked from the PR:
      [deleted-test parity audit](2026-07-22-generic-extension-correctness-deleted-test-parity-audit.md).
      It disproves the former 274 claim and records 1,024 exact path/name
      removals, 953 names absent globally, and 685 tests in deleted files.
- [ ] Every deleted user behavior is mapped to `preserved`, `re-expressed`, `ratified removed`, or `missing regression`.
- [ ] The eight previously identified missing P0 journeys are all covered: delayed channel OAuth, A/B lifecycle isolation, fresh-thread re-pair, permanent-send recovery, repeated admin paste, hosted-MCP callback/tools, pairing isolation/concurrency, and source-target automation delivery.
- [ ] Tests that previously asserted the obsolete Activate state are rewritten around the new user contract rather than deleted without replacement.
- [ ] Legacy/bespoke channel tests are re-expressed through generic seams where their behavior still applies.
- [ ] No assertion is weakened solely to accept the refactor's current behavior.
- [ ] `tests/integration/coverage-floor.toml` is updated only through its documented same-PR recapture process when intentional coverage is added.

## 8. Deterministic verification matrix

Run fast-to-slow. Record exact outputs against the final PR SHA.

### 8.1 Formatting and frontend

- [ ] `cd docs && mint dev`
- [ ] `cd docs && mint broken-links`
- [ ] `cargo fmt --all -- --check`
- [ ] `cd crates/ironclaw_webui/frontend && pnpm install --frozen-lockfile`
- [ ] `cd crates/ironclaw_webui/frontend && pnpm lint`
- [ ] `cd crates/ironclaw_webui/frontend && pnpm test`
- [ ] `cd crates/ironclaw_webui/frontend && pnpm build`

### 8.2 Owning crate suites

- [ ] `cargo test -p ironclaw_extensions --no-fail-fast`
- [ ] `cargo test -p ironclaw_extension_host --no-fail-fast`
- [ ] `cargo test -p ironclaw_auth --no-fail-fast`
- [ ] `cargo test -p ironclaw_mcp --no-fail-fast`
- [ ] `cargo test -p ironclaw_conversations --no-fail-fast`
- [ ] `cargo test -p ironclaw_product_workflow --no-fail-fast`
- [ ] `cargo test -p ironclaw_reborn_composition --all-features --no-fail-fast`
- [ ] `cargo test -p ironclaw_webui --all-features --no-fail-fast`
- [ ] `cargo test -p ironclaw_host_runtime --all-features --no-fail-fast`
- [ ] `cargo test -p ironclaw_dispatcher --all-features --no-fail-fast`
- [ ] Every changed owning crate not listed above has its full unfiltered suite recorded.

### 8.3 Reborn integration suites

- [ ] `cargo test --test reborn_group_extensions --no-fail-fast`
- [ ] `cargo test --test reborn_group_triggers --no-fail-fast`
- [ ] `cargo test --test reborn_group_journeys --no-fail-fast`
- [ ] `cargo test --test reborn_integration_auth_gate --no-fail-fast`
- [ ] `cargo test --test reborn_integration_oauth_connect --no-fail-fast`
- [ ] `cargo test --test reborn_integration_oauth_popup_journeys --no-fail-fast`
- [ ] `cargo test --test reborn_integration_reopen_resume_through_gate --no-fail-fast`
- [ ] `cargo test --test reborn_integration_channel_connection_projection --no-fail-fast`
- [ ] `cargo test --test reborn_integration_extension_ingress --no-fail-fast`
- [ ] `cargo test --test reborn_integration_extension_delivery --no-fail-fast`
- [ ] `cargo test --test reborn_integration_extension_runtime --no-fail-fast`
- [ ] `cargo test --test reborn_integration_extension_user_lifecycle_isolation --no-fail-fast`
- [ ] `cargo test --test reborn_integration_mcp --no-fail-fast`
- [ ] `cargo test --test reborn_integration_triggered_submit --no-fail-fast`
- [ ] `bash scripts/reborn-e2e-rust.sh`

### 8.4 Focused load-bearing contracts

- [ ] `cargo test -p ironclaw_product_workflow --test run_delivery_contract`
- [ ] `cargo test -p ironclaw_reborn_composition external_channel_delivers_final_after_oauth_outlives_delivery_poll_window --features test-support`
- [ ] MCP contract includes and passes `concrete_mcp_http_client_discovers_bounded_deep_openapi_schema`.
- [ ] WebUI descriptor contracts prove the activate route is absent and lifecycle/admin routes have correct auth, rate, body, audit, and effect metadata.
- [ ] Product command registry contract proves `extension_activate` is absent.
- [ ] libSQL and PostgreSQL parity tests cover any changed lifecycle, policy, binding, or idempotency persistence.

### 8.5 Clippy, feature matrix, and safety

- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo clippy --all --tests --examples -- -D warnings`
- [ ] `cargo clippy --all --tests --examples --all-features -- -D warnings`
- [ ] `cargo test -p ironclaw_architecture`
- [ ] `scripts/pre-commit-safety.sh`
- [ ] `scripts/ci/check-reborn-qa-fixtures.sh`
- [ ] `cargo build`
- [ ] `cargo test --features integration` for every database/runtime path whose owning guide requires it.

### 8.6 Browser E2E

- [ ] Browser test covers operator Admin Configuration navigation and manifest-driven groups.
- [ ] Browser test proves Admin Configuration is separate from the operator's and ordinary users' personal installation state.
- [ ] Browser test covers user A and B independent install/setup/remove state.
- [ ] Browser test covers no Activate button or second-click state after OAuth.
- [ ] Browser test covers Notion OAuth return -> active -> tools visible.
- [ ] Browser test covers Slack personal OAuth and removal.
- [ ] Browser test covers Slack install -> setup modal and proves the only personal setup affordance is OAuth while every admin/runtime/routing field remains absent.
- [ ] Browser test covers Telegram QR/deep-link/countdown and `/start` pairing.
- [ ] Browser test covers Delivery Defaults external targets and stale-target removal.
- [ ] Browser test covers the direct auth card action and cancellation.
- [ ] Browser test covers repeated admin secret paste with a refetch between edits.
- [ ] Relevant pytest/Playwright files are included in a committed CI matrix; local-only green tests do not count.

## 9. Canary and live-QA gates

- [ ] Deterministic suites are green before dispatching live canaries.
- [ ] Live Canary is dispatched for the exact PR head using `target_ref`, `target_branch`, and `target_pr`.
- [ ] `reborn-webui-v2-live-qa` full relevant case set passes on the exact PR head.
- [ ] `auth-channels` passes with current manifest provider IDs, routes, and scopes.
- [ ] `auth-browser-consent` or the appropriate real-browser OAuth lane passes for OAuth lifecycle changes.
- [ ] `workflow-canary` passes channel/routine delivery scenarios.
- [ ] Relevant Slack, Telegram, Google, and MCP/Notion live cases pass with sanitized artifacts.
- [ ] A failed canary is classified from logs/artifacts as product regression, harness defect, provider/configuration issue, or proven transient; it is never rerun blindly to seek green.
- [ ] Any harness/provider-ID/configuration defect discovered by canary has a deterministic validation or harness test before rerun.
- [ ] Canary assertions use the three-state lifecycle and do not call or expect Activate.
- [ ] Canary setup uses current generic admin/user APIs and no deleted Slack setup endpoint.
- [ ] Canary bots/apps are environment-isolated from QA, production, and local manual testing.
- [ ] Canary artifacts pass secret scrubbing and contain no live credentials or PII.
- [ ] All required PR CI checks are green after the final push; an earlier green SHA does not count.

## 10. Manual QA matrix on the deployed PR head

Use fresh users and fresh provider identities where possible. Record sanitized timestamps, users, expected result, actual result, and linked logs.

- [ ] A clean local full-stack build runs frontend, backend, workers, channel ingress, OAuth callbacks, MCP discovery, and all enabled extensions from the exact PR worktree/SHA.
- [ ] The running backend process working directory and reported build SHA match the frontend/assets being tested; no stale worktree or old launch service is serving the browser.
- [ ] Local test accounts and operator session authenticate independently without printing bearer tokens into the PR or test artifacts.

### 10.1 Operator/admin

- [ ] Operator signs in and sees Admin Configuration and Admin Extensions.
- [ ] Operator configures Telegram, Slack, Google, and every other manifest-required admin group without personally installing them.
- [ ] Operator can personally install/remove an extension as an ordinary user without changing tenant configuration or any other user's state.
- [ ] The UI does not expose or imply tenant-wide required deployment until its fast-follow store/API/policy exists.

### 10.2 User A

- [ ] User A starts with no inherited personal extensions after a clean reset.
- [ ] User A installs no-setup extension and it is immediately active.
- [ ] User A installs Notion, completes OAuth, returns active, and invokes a real discovered Notion tool.
- [ ] User A installs Slack, completes personal OAuth, uses a Slack tool/channel, and removes it successfully.
- [ ] User A installs Telegram, pairs through `/start`/QR/deep link, receives thinking and final replies, then removes it.
- [ ] User A creates a Telegram and Slack routine and observes external delivery.

### 10.3 User B and isolation

- [ ] User B remains uninstalled while user A installs personal extensions.
- [ ] User B cannot see or use user A's Slack/Notion credentials, Telegram pairing, delivery target, or tools.
- [ ] User B independently installs, authenticates/pairs, uses, and removes each relevant extension.
- [ ] User A removal does not change user B.
- [ ] Admin configuration alone does not change user A or B membership/readiness.

### 10.4 Channel edge journeys

- [ ] Unpaired Telegram sender gets one correct `/start` response.
- [ ] Unconnected Slack sender gets one correct personal-connect response.
- [ ] A Telegram-originated Gmail OAuth run completes after several minutes and returns the actual final answer to Telegram and WebUI exactly once.
- [ ] The same delayed OAuth journey works from Slack.
- [ ] Removing a channel during a blocked/long run prevents stale final delivery.
- [ ] Removing Telegram from Slack stops normal Telegram responses for that user.
- [ ] Reinstall/re-pair uses a fresh installation binding and does not revive stale targets.
- [ ] A routine scheduled from Telegram delivers externally, then stops after removal/unpair.

## 11. Configuration, deployment, and data-reset gates

- [ ] All required config keys are declared by the owning Reborn config/manifest contracts.
- [ ] QA and production environment variable inventories are compared with the new manifests and route contracts.
- [ ] Obsolete Slack callback/webhook variables and feature flags are removed from code, Railway, docs, local shared env templates, and canary configuration.
- [ ] Canonical OAuth callback URLs for Slack, Google/Gmail, Notion, and other OAuth extensions are documented and configured in provider consoles.
- [ ] Canonical Telegram/Slack ingress URLs are documented and configured per environment.
- [ ] QA, production, canary, and local bots/apps are distinct and named consistently.
- [ ] No secret value is committed, pasted into PR text, test output, screenshots, or artifacts.
- [ ] A pre-deploy state inventory identifies extension manifests/packages, tenant configuration, personal memberships, legacy tenant-owned rows, OAuth flows/tokens, pairings/bindings, delivery targets, reply contexts, idempotency records, and routine delivery targets.
- [ ] The deploy plan explicitly states which state is migrated, preserved, invalidated, or wiped.
- [ ] Any wipe is narrowly scoped and has a verified backup/restore path before execution.
- [ ] Blank-slate reset, if required by the merged architecture, is rehearsed in QA before production.
- [ ] Post-reset admin configuration is applied before user testing, while personal installs/auth/pairing are recreated by each user.
- [ ] Deployment order prevents old binaries from writing incompatible state after the new schema/policy is live.
- [ ] Rollback steps include binary rollback, config rollback, provider callback rollback, and state compatibility/restore.
- [ ] Health checks prove WebUI, lifecycle APIs, OAuth callbacks, Slack/Telegram ingress, MCP discovery, and external delivery after deployment.

## 12. Documentation and contract gates

- [ ] Relevant `docs/reborn/contracts/` documents describe the three-state lifecycle and the three authority axes.
- [ ] Auth contract documents server-derived manifest OAuth recipes and automatic callback reconciliation.
- [ ] Conversation/delivery contract documents sealed source-route persistence and event-driven delayed delivery.
- [ ] Trigger contract documents source-target inheritance, revalidation, idempotency, and scheduled self-mutation denial.
- [ ] Admin/user UI documentation distinguishes tenant configuration, personal install, and personal setup, and links the explicit tenant-wide deployment fast follow.
- [ ] Telegram documentation uses `/start` only and matches QR/deep-link/countdown behavior.
- [ ] Slack documentation distinguishes app-level admin configuration from personal OAuth.
- [ ] MCP documentation states readiness requires successful bounded live discovery and model-visible tools.
- [ ] API documentation removes Activate and obsolete Slack routes.
- [ ] `CHANGELOG.md` describes user-visible lifecycle, admin, OAuth/MCP, channel, automation, and removal fixes.
- [ ] `FEATURE_PARITY.md` is updated if any channel or extension parity status changed.
- [ ] Local runbooks and shared-env guidance reflect current callbacks, ports, bots/apps, and extension setup without containing secrets.
- [ ] PR title and summary name every major layer in the actual diff.

## 13. Final diff and review gates

- [ ] `git diff --check` passes.
- [ ] The exact final changed-file list is reviewed for accidental edits and untracked required files.
- [ ] Changed production files are searched for `.unwrap()`, `.expect()`, unchecked byte slices, hardcoded temp paths, secrets, provider-specific lifecycle branches, Activate remnants, and stale route names.
- [ ] All sibling instances of each fixed bug pattern are searched across `crates/`, frontend, tests, scripts, docs, and workflows.
- [ ] Every new trait/method change has an implementation/decorator/test-double inventory.
- [ ] Every persistence change has concurrency, interruption, reopen, libSQL, and PostgreSQL consideration.
- [ ] Every ingress/egress change has actor scope, size limits, redaction, replay/idempotency, and authoritative side-effect evidence review.
- [ ] Clean-surface audit reports no confirmed dead code, shim tests, or over-engineering introduced by the diff.
- [ ] Architecture review reports no policy in composition and no improper dependency edge.
- [ ] Security review covers OAuth, secrets, channel identity, tenant isolation, admin authority, and shared-target redaction.
- [ ] At least one reviewer audits the implementation against this checklist rather than only reading the PR summary.
- [ ] All actionable review threads are resolved in code/tests or explicitly dispositioned with maintainer agreement.
- [ ] No unresolved requested-changes review remains.
- [ ] The branch contains the latest target branch and has no merge conflicts.
- [ ] GitHub `mergeStateStatus`, required checks, and review decision are acceptable on the exact final SHA.

## 14. Pull request completion gates

- [ ] PR Summary explains the user-visible failures and the generic architecture that fixes them.
- [ ] Change Type accurately checks bug fix, refactor, security, documentation, and any other applicable categories.
- [ ] Linked issues and bug reports are included.
- [ ] Validation lists every command actually run; no command is claimed from an earlier SHA.
- [ ] `Test Strategy` completes every field and lists all applicable risks: model, browser, side effect, persistence, security/permissions, external provider, and cross-component behavior.
- [ ] `Tests added or updated` names unit/contract, Reborn integration, recorded fixture, browser E2E, backend/runtime, and live canary evidence or a precise N/A reason.
- [ ] `What the tests prove` maps directly to the reported bug ledger.
- [ ] Security Impact documents permissions, network, secrets, tool execution, channel identity, and admin authority changes.
- [ ] Reborn Trust-Boundary Checklist is fully completed with commands/evidence.
- [ ] Database Impact documents schema/data/reset behavior for libSQL and PostgreSQL.
- [ ] Blast Radius names lifecycle, auth, MCP, channels, delivery, automations, WebUI, config, and canaries.
- [ ] Rollback Plan is actionable and includes hidden side effects and provider configuration.
- [ ] Review Follow-Through contains no unowned merge blocker.
- [ ] Review track is C because the diff crosses runtime, security, persistence, external providers, and deployment configuration.
- [ ] PR is marked Ready for Review only after deterministic checks pass and all P0 regressions exist.
- [ ] PR is marked merge-ready only after final-head CI, required live canaries, checklist review, and merge-state verification pass.

## 15. Final merge authorization

All of the following must be true at the same time:

- [ ] Every reported bug in Section 2 is checked with caller-level evidence.
- [ ] Every P0 user journey in Section 7.2 is checked and runs in CI.
- [ ] All deterministic verification in Section 8 is green on the exact PR head.
- [ ] Required browser/live QA and canary gates in Sections 9 and 10 are green on the exact PR head.
- [ ] Multi-tenant, security, architecture, and clean-surface reviews have no unresolved blocker.
- [ ] Config, data-reset/migration, deployment, and rollback plans are reviewed and executable.
- [ ] The pull request template is complete, accurate, and linked to the evidence.
- [ ] GitHub reports the final head mergeable with required approvals and no unresolved review threads.
- [ ] The final reviewer explicitly records `READY TO MERGE: all generic extension correctness gates satisfied` together with the full verified commit SHA.

Until every box above is satisfied or explicitly dispositioned under the rules in Section 0, the PR is **not ready to merge**.
