# PR-2: WebUI Command Palette Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Slash commands work in the WebUI: a role-filtered inventory endpoint, an execute endpoint sharing the channel path's audience policy, and a Slack-quality composer menu — plus the implicit-owner fix the PR-1 audit mandated.

**Architecture:** Registry descriptors gain presentation metadata (`title`/`description`/`usage`). Two new product-surface operations (`product.commands.list`, `product.commands.execute`) back `GET /api/webchat/v2/commands` and `POST /api/webchat/v2/threads/{thread_id}/commands`; both resolve the caller's role by direct `AdminUserService::get_user` (env-bearer operator = implicit admin, matching `authorize_admin`), and execute applies the same `required_audience` function channel admission uses. The frontend salvages #6678's server-driven matcher/hook/renderer (aliases deleted), fixes its two latent bugs, and upgrades the composer menu to the design's keyboard-driven bar. Branch: `pr2-webui-command-palette` off `origin/main` (`c5f6c4319`); PR targets `main`. Spec: `docs/superpowers/specs/2026-07-29-product-command-train-design.md` (PR-2 section). Salvage sources verified against `origin/alpine-fight` (`b131a2565`).

**Tech Stack:** Rust 2024 workspace (`ironclaw_assistant`, `ironclaw_extension_host`, `ironclaw_webui`), Axum, React frontend under `crates/ironclaw_webui/frontend` (vitest, no TS in these files — plain JS/JSX conventions), root integration harness.

## Global Constraints

- No `.unwrap()` / `.expect()` in production Rust (tests fine). No new dependencies.
- Aliases stay dead (Decision 8): no `aliases` field anywhere — not in descriptors, DTOs, frontend matchers, or tests.
- Role policy parity: execute uses `ironclaw_assistant::commands::required_audience`; listing filters by descriptor `audience`; env-bearer operator (`caller.operator_config` set) is an implicit admin exactly like `RebornServices::authorize_admin`; directory records (any status) govern over the implicit rule when present for channel actors.
- A Member's `POST /model set …` must produce an `AccessDenied`-kind rejection and never reach `execute_product_model_command` / `invoke_llm_active_set`.
- Rejection help shown to a caller lists only commands their audience may see (no admin names/usage leak to Members).
- WebUI results are ephemeral system notices (no durable timeline events).
- Red-green per behavior change; run suites with `--no-fail-fast`; never pipe test output through `head`/`tail`.
- Frontend: all 11 locale files stay key-parity; `npx vitest run` green; conventions lint green.
- Commit messages end with: `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`

## File Structure

- `crates/ironclaw_assistant/src/commands.rs` — metadata fields + lifecycle metadata table.
- `crates/ironclaw_extension_host/src/channel_command_roles.rs` — implicit-owner rule (audit fix).
- `crates/ironclaw_assistant/src/reborn_services/types.rs` — new DTOs; `reborn_services.rs` — operation consts + facade module decl; NEW `crates/ironclaw_assistant/src/reborn_services/product_commands.rs` — list/execute facade; `reborn_services/product_capability_handlers.rs` — handler variants.
- `crates/ironclaw_webui/src/webui_v2/{descriptors,router,mod,handlers}.rs` — routes.
- Frontend NEW: `pages/chat/lib/chat-commands.ts` + test, `pages/chat/hooks/useChatCommands.ts`. Frontend MODIFY: `lib/api.ts`, `pages/chat/hooks/useChat.ts`, `pages/chat/chat.tsx`, `pages/chat/components/chat-input.tsx`, `pages/chat/components/empty-state.tsx`, `i18n/*.ts` (11 files), `pages/chat/lib/{chat-input,chat,useChat-send}.test.ts`.
- Tests: `crates/ironclaw_assistant/tests/product_commands_contract.rs`, `crates/ironclaw_webui/tests/webui_v2_descriptors_contract.rs`, `tests/integration/webui_v2_product_api.rs`, extension_host resolver unit tests.

---

### Task 1: Registry presentation metadata

**Files:**
- Modify: `crates/ironclaw_assistant/src/commands.rs`
- Test: `crates/ironclaw_assistant/tests/product_commands_contract.rs`

**Interfaces:**
- Produces: `ProductCommandDescriptor { pub name: &'static str, pub audience: CommandAudience, pub title: &'static str, pub description: &'static str, pub usage: &'static str }`. Later tasks read these fields verbatim.

- [ ] **Step 1: Failing tests.** In `product_commands_contract.rs`:
  - New test `every_descriptor_has_presentation_metadata`: iterate `product_command_descriptors()`; assert `title`, `description`, `usage` are non-empty; assert `usage.starts_with(&format!("/{}", descriptor.name))`.
  - New test `command_audience_serializes_snake_case`: `assert_eq!(serde_json::to_string(&CommandAudience::User).unwrap(), "\"user\"");` and `"\"admin\""` for Admin (kills a PR-1 deferral).
  - Extend the model-descriptor test with `assert_eq!(model.title, "Model");` and rename it `command_registry_declares_model_with_user_audience_and_metadata` (kills the stale-name deferral).
- [ ] **Step 2: Run to verify red.** Run: `cargo test -p ironclaw_assistant --test product_commands_contract --no-fail-fast`
Expected: compile FAIL (`title` unknown field).
- [ ] **Step 3: Implement.** Add the three `&'static str` fields. Populate `COMMAND_SPECS`:
  - `model`: title `Model`, description `Show or switch the active LLM provider and model`, usage `/model [<model> | set-provider <provider> [--model <model>]]`
  - `status`: title `Status`, description `Show what the assistant is doing in this conversation`, usage `/status`
  Add `fn lifecycle_command_metadata(kind: LifecycleCommandKind) -> (&'static str, &'static str, &'static str)` returning (title, description, usage) per row, exact strings:
  - ExtensionSearch: `Search extensions` / `Search the extension registry` / `/extension_search <query>`
  - ExtensionList: `List extensions` / `List installed extensions` / `/extension_list`
  - ExtensionInstall: `Install extension` / `Install an extension by id` / `/extension_install <id>`
  - ExtensionAuth: `Connect extension account` / `Start authentication for an installed extension` / `/extension_auth <id>`
  - ExtensionActivate: `Activate extension` / `Activate an installed extension` / `/extension_activate <id>`
  - ExtensionConfigure: `Configure extension` / `Update an installed extension's configuration values` / `/extension_configure <id> <json>`
  - ExtensionRemove: `Remove extension` / `Remove an installed extension` / `/extension_remove <id>`
  - SkillSearch: `Search skills` / `Search the skill registry` / `/skill_search <query>`
  - SkillInstall: `Install skill` / `Install a skill from JSON content` / `/skill_install <json>`
  - SkillRemove: `Remove skill` / `Remove an installed skill` / `/skill_remove <id or name>`
  Wire into the lifecycle `.map(...)` closure in `product_command_descriptors()`.
- [ ] **Step 4: Green.** Run: `cargo test -p ironclaw_assistant --no-fail-fast`
- [ ] **Step 5: Commit** `feat(commands): add presentation metadata to command descriptors`.

---

### Task 2: Implicit-owner rule in the channel role resolver (PR-1 audit fix)

**Files:**
- Modify: `crates/ironclaw_extension_host/src/channel_command_roles.rs`

**Interfaces:**
- Consumes: existing `ChannelActorRoleResolver { operator_user_id, admin_users, ... }`.
- Produces: behavior only — when the resolved bound user equals `operator_user_id` and `get_user` returns `Ok(None)`, `actor_role` returns `Ok(Some(AdminUserRole::Owner))`. A persisted record (any status, including Suspended) still governs.

- [ ] **Step 1: Failing tests.** In the file's `#[cfg(test)]` module (mirror existing fakes):
  - `operator_bound_actor_without_directory_record_is_implicit_owner`: seed the identity fake so the actor resolves to the SAME user id as the resolver's `operator_user_id`; `FakeAdminUsers` has no record; assert `Ok(Some(AdminUserRole::Owner))`.
  - `operator_bound_actor_with_suspended_record_is_not_admin`: same identity, but a Suspended Owner record exists; assert `Ok(None)` (record governs).
  - `non_operator_bound_actor_without_record_stays_none`: distinct user id, no record; assert `Ok(None)`.
  - Also cover the two PR-1-deferred branches: `admin_users` fake returning `AdminUserError::Internal` → error with `is_retryable() == false`; a bound user whose record exists with `Member` role → `Ok(Some(Member))`.
- [ ] **Step 2: Red.** Run: `cargo test -p ironclaw_extension_host channel_command_roles --no-fail-fast`
Expected: the implicit-owner test FAILS (`Ok(None)`).
- [ ] **Step 3: Implement.** In `actor_role`'s `get_user` match, change the no-record arm:

```rust
// The env-bearer operator has no directory record but is the deployment's
// implicit owner — mirror `RebornServices::authorize_admin`'s contract. A
// persisted record (any status) still governs when present.
Ok(None) if user_id == self.operator_user_id => Ok(Some(AdminUserRole::Owner)),
Ok(_) => Ok(None),
```

(Keep the Active-record arm above it unchanged; the Suspended-record case flows into `Ok(_) => Ok(None)` because the record is `Some`.) Note: the match must distinguish `Ok(Some(record))` non-Active (→ `Ok(None)`) from `Ok(None)` (→ implicit-owner check) — restructure arms accordingly.
- [ ] **Step 4: Green + suites.** Run: `cargo test -p ironclaw_extension_host --no-fail-fast`
- [ ] **Step 5: Commit** `fix(commands): treat the operator's bound actor as implicit owner

The env-bearer operator has no admin-directory record, so channel admin
commands permanently denied the deployment owner while the WebUI door
admitted them (authorize_admin operator_config bypass) — the two-door
drift flagged by the PR-1 final review. Directory records still govern
when present.`

---

### Task 3: WebUI facade — list/execute operations (audience-aware, written fresh)

**Files:**
- Modify: `crates/ironclaw_assistant/src/reborn_services/types.rs`, `crates/ironclaw_assistant/src/reborn_services.rs`, `crates/ironclaw_assistant/src/reborn_services/product_capability_handlers.rs`, `crates/ironclaw_assistant/src/lib.rs`
- Create: `crates/ironclaw_assistant/src/reborn_services/product_commands.rs`
- Test: `crates/ironclaw_assistant/tests/product_command_surface_contract.rs` (or the reborn_services contract file if the facade tests fit better there — extend, don't duplicate)

**Interfaces:**
- Consumes: Task 1 metadata fields; `required_audience`; `AdminUserService::get_user`; `AdminUserStatus`; `caller.operator_config`; existing `execute_product_model_command` / `execute_product_status_command`; `parse_product_slash_command`; `ProductCommand::from_payload`; `EmptyProductCommandInput`; `ProductSurfaceCommandDescriptor`.
- Produces (exact, later tasks import these):

```rust
// types.rs (all Serialize+Deserialize, PartialEq, Eq, Debug, Clone):
pub struct RebornProductCommandInfo { pub name: String, pub title: String, pub description: String, pub usage: String }
pub struct RebornProductCommandListResponse { pub commands: Vec<RebornProductCommandInfo> }
pub struct RebornExecuteProductCommandRequest { pub thread_id: String, pub text: String }
pub struct RebornCommandRejection { pub kind: crate::ProductRejectionKind, pub message: String }
pub struct RebornExecuteProductCommandResponse {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub result: Option<crate::commands::CommandResultView>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub rejection: Option<RebornCommandRejection>,
}
// reborn_services.rs consts:
pub const PRODUCT_COMMAND_LIST_COMMAND_ID: &str = "product.commands.list";
pub const PRODUCT_COMMAND_LIST_COMMAND: ProductSurfaceCommandDescriptor<EmptyProductCommandInput, RebornProductCommandListResponse> = ProductSurfaceCommandDescriptor::new(PRODUCT_COMMAND_LIST_COMMAND_ID);
pub const PRODUCT_COMMAND_EXECUTE_COMMAND_ID: &str = "product.commands.execute";
pub const PRODUCT_COMMAND_EXECUTE_COMMAND: ProductSurfaceCommandDescriptor<RebornExecuteProductCommandRequest, RebornExecuteProductCommandResponse> = ProductSurfaceCommandDescriptor::new(PRODUCT_COMMAND_EXECUTE_COMMAND_ID);
// facade methods on RebornServices (in reborn_services/product_commands.rs):
pub(super) async fn list_product_commands(&self, caller: ProductSurfaceCaller) -> Result<RebornProductCommandListResponse, ProductSurfaceError>
pub(super) async fn execute_product_command(&self, caller: ProductSurfaceCaller, request: RebornExecuteProductCommandRequest) -> Result<RebornExecuteProductCommandResponse, ProductSurfaceError>
```

- [ ] **Step 1: Failing facade tests (through the workflow caller tier the existing contract files use).** Cases:
  - `member_command_list_excludes_admin_audience`: fake admin-users returns an Active Member record → list contains exactly `model`, `status` with Task 1's metadata strings; no lifecycle names.
  - `admin_command_list_includes_lifecycle_family`: Active Admin record → 12 entries.
  - `operator_config_caller_lists_admin_commands`: caller with `operator_config` set, no directory record → 12 entries (implicit admin).
  - `member_execute_model_set_is_access_denied_without_llm_invoke`: execute `text = "/model set fake"` → `rejection.kind == ProductRejectionKind::AccessDenied`, `result.is_none()`, and the LLM-config seam records zero calls.
  - `member_execute_model_read_returns_view`: `text = "/model"` → `result` is the model view.
  - `member_execute_unknown_command_help_excludes_admin_names`: `text = "/nope"` → `rejection.kind == InvalidRequest`, message contains `/model` and `/status`, does NOT contain `extension_configure` or `skill_remove`.
  - `execute_status_on_foreign_thread_is_not_found`: mirrors the existing foreign-thread 404 pin shape for `/status` through this new entry.
  Follow the harness idiom already in the chosen contract file for constructing `RebornServices` with fakes (grep for the existing `execute_product_model_command`/admin-users test setups and extend that harness; do not stand up a new one).
- [ ] **Step 2: Red.** Run the chosen contract test file. Expected: compile FAIL (missing types/methods).
- [ ] **Step 3: Implement.**
  - DTOs in `types.rs`; export through `reborn_services.rs`'s `pub use types::{...}` and `lib.rs`'s re-export list (derive current alphabetical slots — do not trust old line numbers).
  - Consts beside the other `*_COMMAND` descriptors.
  - New submodule `reborn_services/product_commands.rs` (declare `mod product_commands;` beside the existing submodule decls):

```rust
impl RebornServices {
    async fn caller_is_command_admin(&self, caller: &ProductSurfaceCaller) -> Result<bool, ProductSurfaceError> {
        if caller.operator_config { return Ok(true); } // env-bearer operator: implicit admin (authorize_admin parity)
        match self.admin_users.get_user(&caller.tenant_id, &caller.user_id).await {
            Ok(Some(record)) => Ok(record.status == AdminUserStatus::Active && record.role.is_admin()),
            Ok(None) => Ok(false),
            Err(AdminUserError::Unavailable) => Err(/* retryable 503 per existing taxonomy */),
            Err(_) => Err(/* internal 500, non-retryable */),
        }
    }

    fn visible_descriptors(is_admin: bool) -> impl Iterator<Item = ProductCommandDescriptor> {
        product_command_descriptors().filter(move |d| is_admin || d.audience == CommandAudience::User)
    }

    fn caller_command_help_text(is_admin: bool) -> String {
        // Audience-filtered full-inventory help; same "Available commands:" shape
        // as declared_command_help_text but over the registry, not a declared set.
    }
}
```

  - `list_product_commands`: `caller_is_command_admin` → map `visible_descriptors` into `RebornProductCommandInfo` (presentation order: registry order).
  - `execute_product_command`: parse via `parse_product_slash_command(&request.text, ProductTriggerReason::DirectChat)`. Every parse-stage failure — `Ok(None)` (not slash text), payload validation error, or a `ProductCommand::from_payload` rejection (bad arguments) — becomes `{ kind: InvalidRequest, message: caller_command_help_text(...) }`; internal rejection reasons are never surfaced (leak rule, matching the channel observer's InvalidRequest→help behavior). Then:
    - `required_audience(&command) == Admin && !caller_is_command_admin(...)` → rejection `{ kind: AccessDenied, message: "This command requires an admin account." }`.
    - `ProductCommand::Model { action }` → `self.execute_product_model_command(caller, action).await?` → `result`.
    - `ProductCommand::Status` → `self.execute_product_status_command(caller, ProductStatusCommandInput { thread_id: request.thread_id }).await?`.
    - `ProductCommand::Lifecycle { .. } | ProductCommand::Unknown { .. }` → rejection `{ kind: InvalidRequest, message: caller_command_help_text(...) }` (lifecycle stays non-executable from the composer in PR-2 even for admins — listing-only, per the spec).
  - Handler wiring in `product_capability_handlers.rs`: `ProductCommandList` / `ProductCommandExecute` variants, `parse` arms on the two const IDs, `invoke` arms calling the facade methods (`command_output(...)` idiom). Add a pinning test for `ProductCommandHandler::parse` covering ALL its ids (none exists today).
- [ ] **Step 4: Green + crate suite.** Run: `cargo test -p ironclaw_assistant --no-fail-fast`
- [ ] **Step 5: Commit** `feat(webui): add role-filtered command list and execute operations`.

---

### Task 4: WebUI routes

**Files:**
- Modify: `crates/ironclaw_webui/src/webui_v2/descriptors.rs`, `router.rs`, `mod.rs`, `handlers.rs`
- Test: `crates/ironclaw_webui/tests/webui_v2_descriptors_contract.rs`, `tests/integration/webui_v2_product_api.rs`

**Interfaces:**
- Consumes: Task 3 consts/DTOs.
- Produces: `GET /api/webchat/v2/commands` → `RebornProductCommandListResponse`; `POST /api/webchat/v2/threads/{thread_id}/commands` body `{ "text": string }` → `RebornExecuteProductCommandResponse`. Route ids `webui.v2.list_commands` / `webui.v2.execute_command`.

- [ ] **Step 1: Failing tests.** Extend `webui_v2_descriptors_contract.rs` with the two new route descriptors (id, method, pattern, effect path `ProductSurface`, audit class `UserAction`; execute has a 4 KiB body limit + mutation rate limit; list uses the read policy). Extend `tests/integration/webui_v2_product_api.rs` (grep first for its existing auth/actor helpers and admin-user seeding; extend the existing file's harness): member list excludes lifecycle; admin list includes them; member `POST /model set x` → HTTP 200 with body `rejection.kind == "access_denied"`; `/status` on an owned thread returns a result view; foreign thread → 404.
- [ ] **Step 2: Red.** Run: `cargo test -p ironclaw_webui --no-fail-fast` (descriptor pin fails). Integration file red run: `cargo test --test reborn_integration_webui_v2_product_api --no-fail-fast` — derive the exact test target name from `tests/integration/CLAUDE.md` before running.
- [ ] **Step 3: Implement** exactly per the verified insertion points:
  - `descriptors.rs`: constants `WEBUI_V2_ROUTE_LIST_COMMANDS = "webui.v2.list_commands"`, `WEBUI_V2_ROUTE_EXECUTE_COMMAND = "webui.v2.execute_command"`, patterns `/api/webchat/v2/commands` and `/api/webchat/v2/threads/{thread_id}/commands`; `list_commands_descriptor()` with `read_policy(read_rate_limit(), AuditTraceClass::UserAction, AllowedEffectPath::ProductSurface, StreamingMode::None)`; `execute_command_descriptor()` with `mutation_policy(body_limit_kib(4), mutation_rate_limit(), AuditTraceClass::UserAction, AllowedEffectPath::ProductSurface)`; insert both calls directly above `list_automations_descriptor()` in `webui_v2_routes()`. NOT operator-only.
  - `router.rs`: `.route(WEBUI_V2_PATTERN_LIST_COMMANDS, get(handlers::list_commands))` and `.route(WEBUI_V2_PATTERN_EXECUTE_COMMAND, post(handlers::execute_command))` directly above the `WEBUI_V2_PATTERN_LIST_AUTOMATIONS` route; add both patterns to the descriptors import block.
  - `mod.rs`: re-export the two route ids and two handlers alphabetically.
  - `handlers.rs`: import `PRODUCT_COMMAND_EXECUTE_COMMAND, PRODUCT_COMMAND_LIST_COMMAND` (current alphabetical slot); add:

```rust
/// `GET /api/webchat/v2/commands`
pub async fn list_commands(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
) -> Result<Json<ironclaw_assistant::RebornProductCommandListResponse>, WebUiV2HttpError> {
    let response = invoke_product_command(state.services(), caller, PRODUCT_COMMAND_LIST_COMMAND, EmptyProductCommandInput {}).await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct ExecuteCommandBody { text: String }

/// `POST /api/webchat/v2/threads/:thread_id/commands`
pub async fn execute_command(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Path(thread_id): Path<String>,
    Json(body): Json<ExecuteCommandBody>,
) -> Result<Json<ironclaw_assistant::RebornExecuteProductCommandResponse>, WebUiV2HttpError> {
    let response = invoke_product_command(state.services(), caller, PRODUCT_COMMAND_EXECUTE_COMMAND,
        ironclaw_assistant::RebornExecuteProductCommandRequest { thread_id, text: body.text }).await?;
    Ok(Json(response))
}
```

  (Match the surrounding handlers' exact extractor ordering/idioms if they differ from the above.)
- [ ] **Step 4: Green.** Run webui crate + the integration target; then `cargo test -p ironclaw_architecture_tests --no-fail-fast`.
- [ ] **Step 5: Commit** `feat(webui): mount the command list and execute routes`.

---

### Task 5: Frontend salvage + the two #6678 bug fixes

**Files:**
- Create: `crates/ironclaw_webui/frontend/src/pages/chat/lib/chat-commands.ts` + `chat-commands.test.ts`, `crates/ironclaw_webui/frontend/src/pages/chat/hooks/useChatCommands.ts`
- Modify: `frontend/src/lib/api.ts`, `pages/chat/hooks/useChat.ts`, `pages/chat/chat.tsx`, `pages/chat/components/chat-input.tsx`, `pages/chat/components/empty-state.tsx`, all 11 `i18n/*.ts`, `pages/chat/lib/chat-input.test.ts`, `pages/chat/lib/chat.test.ts`, `pages/chat/lib/useChat-send.test.ts`

Salvage from `origin/alpine-fight` (`git show origin/alpine-fight:<path>`), adapted:

- [ ] **Step 1: `chat-commands.ts`** — port alpine's three exports, **deleting the alias branches** (`matchCommand` matches `command.name === first` only; `commandMenuMatches` filters `command.name.startsWith(prefix)` only; `renderCommandResultMarkdown` unchanged). Port the test file dropping the alias fixture and the `/progress` + alias-case assertions; keep canonical-match, unknown, prefix-filter, whitespace-stops-suggesting, and render suites.
- [ ] **Step 2: `api.ts`** — append after `sendMessage` (before the `// --- Timeline ---` comment):

```js
// --- Product commands ---
export function listChatCommands() {
  return apiFetch(`${V2_BASE}/commands`);
}
export function executeChatCommand({ threadId, text }) {
  return apiFetch(`${V2_BASE}/threads/${encodeURIComponent(threadId ?? "")}/commands`, {
    method: "POST",
    body: JSON.stringify({ text }),
  });
}
```

- [ ] **Step 3: `useChatCommands.ts`** — port as-is (module-level cache, `[]` on failure). **`useChat.ts`** — port alpine's `runCommand` callback verbatim (uses `executeChatCommand`, `renderCommandResultMarkdown`, `CHAT_MESSAGE_ROLES.SYSTEM`, `pendingSeqRef`, and `t("chat.commandFailed")` on client error), add `runCommand` to the returned object. **`chat.tsx`** — port the interception (`useChatCommands()`, destructure `runCommand`, in `handleSend` before `send(...)`: `if (activeThreadId && images.length === 0 && attachments.length === 0 && matchCommand(content, chatCommands)) { return await runCommand(content); }` with deps updated), pass `commands={activeThreadId ? chatCommands : []}` to both `<EmptyState>` and `<ChatInput>`.
- [ ] **Step 4: `chat-input.tsx`** — port alpine's menu block as the baseline (listbox before the `attachmentError` block, rows from `commandMenuMatches`, click completes to `/name `). **Bug fix 1:** `empty-state.tsx` — add `commands = []` to its props and forward `commands={commands}` to its internal `<ChatInput>`. **Bug fix 2 (i18n):** add `"chat.commandMenu": "Commands",` AND `"chat.commandFailed": "Couldn't run that command.",` to `en.ts`, plus translated entries in the other 10 locales (`ar, de, es, fr, hi, ja, ko, pt-BR, uk, zh-CN` — translate both strings; follow each file's neighboring `chat.*` entries for placement).
- [ ] **Step 5: Tests.** Port alpine's test deltas: `chat-input.test.ts` mock line (`commandMenuMatches: () => [],`), `chat.test.ts` (`contextOverrides` param + `useChatCommands`/`matchCommand` mocks + the two interception tests), `useChat-send.test.ts` appended `runCommand` failure-localizer test. ADD a new test pinning bug fix 1: rendering `EmptyState` with a non-empty `commands` list forwards it to `ChatInput` (menu reachable from the landing composer). Verify locale-parity coverage picks up both new keys (run whatever key-parity suite exists; if none covers these files, extend the existing i18n parity test).
- [ ] **Step 6: Run.** From `crates/ironclaw_webui/frontend`: `npx vitest run` (expect all green incl. new suites) and the repo's frontend conventions lint (`npm run lint` or the script `package.json` defines — derive, don't guess).
- [ ] **Step 7: Commit** `feat(webui): wire the composer command palette end to end`.

---

### Task 6: Composer menu to Slack quality

**Files:**
- Modify: `crates/ironclaw_webui/frontend/src/pages/chat/components/chat-input.tsx`, `pages/chat/lib/chat-commands.ts` (+ tests for both)

Design bar (spec PR-2): anchored popover above the input; rows render `/name` — **title** — description with the matched prefix highlighted; usage hint for the selected row; ↑/↓ move selection, Enter/Tab complete to `/name `, Esc dismisses (until the draft's command prefix changes); hover/click select; re-filter as you type; menu suppressed once whitespace follows the command word (existing matcher rule).

- [ ] **Step 1: Failing tests.** Extend `chat-commands.test.ts` with a pure selection-state helper contract (next/prev wraparound, reset on filter change). Extend `chat-input.test.ts`: ArrowDown moves the active row; Enter with menu open completes text to `/model ` and does NOT send; Tab completes; Esc closes and a subsequent Enter sends normally; rows render title and description; the active row shows the usage hint; typed prefix is highlighted (assert on the split-text markup).
- [ ] **Step 2: Red.** `npx vitest run pages/chat/lib/chat-input.test.ts pages/chat/lib/chat-commands.test.ts`
- [ ] **Step 3: Implement.** Keep logic in `chat-commands.ts` (add a small pure `commandMenuState` reducer/helpers for selection so the component stays thin). In `chat-input.tsx`: anchored popover container above the textarea (`role="listbox"`, active row `aria-selected`), keydown handling layered BEFORE the existing Enter-to-send handler only while the menu is open, highlight by splitting `command.name` on the typed prefix, selected-row usage hint line, Esc sets a dismissed flag cleared when the command token changes. Follow the file's existing styling system (reuse its class/token conventions — no new CSS framework).
- [ ] **Step 4: Green.** `npx vitest run` (full) + conventions lint + `npx tsc --noEmit` if the frontend build runs it (derive from package.json).
- [ ] **Step 5: Commit** `feat(webui): upgrade the command menu to keyboard-driven palette`.

---

### Task 7: Gauntlet + spec sync + PR

- [ ] **Step 1:** `cargo fmt`; both clippy lanes (`cargo clippy --all --tests --examples -- -D warnings`, same `--all-features`) + `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- [ ] **Step 2:** `cargo test -p ironclaw_assistant -p ironclaw_extension_host -p ironclaw_webui -p ironclaw_architecture_tests --no-fail-fast`; `scripts/pre-commit-safety.sh`; integration target(s) from Task 4; `RUST_MIN_STACK=67108864 bash scripts/reborn-e2e-rust.sh`.
- [ ] **Step 3:** Frontend: `npx vitest run` + lint from `crates/ironclaw_webui/frontend`.
- [ ] **Step 4: Spec sync.** Update the spec's PR-2 section for: the implicit-owner rule (now implemented, both doors); lifecycle commands are listing-only in the WebUI (execute rejects them, admins manage via Extensions page); the landing-composer fix; `chat.commandFailed` addition. Fix any other drift found re-reading the section against the diff.
- [ ] **Step 5: Commit** `chore(webui): gauntlet fixes and spec sync for the command palette`, then push `pr2-webui-command-palette` and open the PR against `main` titled `feat(webui): role-filtered command palette (PR-2 of command train)` — body summarizes: endpoints + policy parity, implicit-owner audit fix, #6678 salvage with its two bug fixes, palette UX, ephemeral results; follow-ups: PR-3 Slack-native, durable results, #6875. End the body with the standard generation footer.

## Self-review checklist

- Spec PR-2 coverage: metadata ✔ (T1), role-filtered GET ✔ (T3/T4), shared `required_audience` on POST ✔ (T3), thread-ownership probe reuse ✔ (T3 tests), composer palette bar ✔ (T5/T6), ephemeral results ✔ (T5), unknown-text-submits-as-message ✔ (T5 chat.tsx logic + tests).
- Audit carry-overs: implicit-owner ✔ (T2 + T3's `operator_config` parity), serde pin ✔ (T1), stale test name ✔ (T1), untested resolver branches ✔ (T2).
- Leak rules: member help excludes admin names at every rejection site (T3 tests).
