# PR-1: Role-Gated Command Admission Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Admin-scoped product-command actions (`/model set`, `/model set-provider`, the lifecycle family) execute only for `Owner`/`Admin` accounts; the alias mechanism is deleted; bundled Slack/Telegram manifests declare `["model", "status"]`.

**Architecture:** `ironclaw_product::commands` gains a `CommandAudience` vocabulary (listing audience on descriptors + action-aware `required_audience`). `DirectConversationCommandAdmission` gains an audience step backed by a new `CommandActorRoleResolver` port; the production resolver (`ironclaw_extension_host`) maps channel actor → bound user (`RebornUserIdentityLookup`) → active-account role (`AdminUserService`). Denials reuse the existing wire-stable `ProductRejectionKind::AccessDenied`; the run-delivery observer maps it to fixed admin-denial copy and filters its static help to user-audience commands. Spec: `docs/superpowers/specs/2026-07-29-product-command-train-design.md`.

**Tech Stack:** Rust 2024 workspace, async-trait, serde, tokio tests. Crates touched: `ironclaw_product`, `ironclaw_extension_host`, `ironclaw_host_api` (read-only), `ironclaw_reborn_composition`, `ironclaw_first_party_extensions` (manifests).

## Global Constraints

- No `.unwrap()` / `.expect()` in production code (tests are fine).
- No new dependencies, no persistence migrations, no new `ProductRejectionKind` variants — reuse `AccessDenied` (already wire-stable and serialized).
- Fail closed everywhere: unknown/unbound/suspended actor ⇒ not admin (permanent `AccessDenied`); resolver transport error ⇒ retryable `ProductSurfaceError`, never silent admin or silent member treatment.
- Manifest command declarations stay exact tokens; a declaration failing `validate_declared_product_command` must fail extension activation (existing behavior — do not weaken).
- Admin-scoped commands must never reach their handlers when denied (assert zero command-surface invokes in every denial test).
- Rejection feedback must reveal only user-audience commands; never echo internal reasons — the observer delivers fixed host copy keyed by rejection kind.
- Red-green per behavior change: write the caller-level test, watch it fail for the right reason, then implement.
- Every task ends with the touched crates compiling and their full suites green (`--no-fail-fast`, never pipe through `head`/`tail`).
- Commit messages: end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

## File Structure

- `crates/ironclaw_product/src/commands.rs` — registry: alias deletion; `CommandAudience`; `required_audience`.
- `crates/ironclaw_product/src/command_admission.rs` — `CommandActorRoleResolver` port + audience step in `DirectConversationCommandAdmission`.
- `crates/ironclaw_product/src/run_delivery/observer.rs` — `AccessDenied` feedback copy; user-audience help filter.
- `crates/ironclaw_extension_host/src/channel_command_roles.rs` — NEW: production `ChannelActorRoleResolver`.
- `crates/ironclaw_extension_host/src/channel_host.rs` — `GenericChannelHostDeps.admin_users` field; per-extension resolver construction (~line 761).
- `crates/ironclaw_reborn_composition/src/factory.rs` (~1260) + `src/runtime.rs` (~1664) — thread `AdminUserService` into deps.
- `crates/ironclaw_first_party_extensions/assets/{slack,telegram}/manifest.toml` — declare `["model", "status"]`.
- Tests: `crates/ironclaw_product/tests/product_commands_contract.rs`, `.../product_command_surface_contract.rs`, `.../run_delivery_contract.rs`, `crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs`.

---

### Task 1: Delete the alias mechanism

**Files:**
- Modify: `crates/ironclaw_product/src/commands.rs`
- Test: `crates/ironclaw_product/tests/product_commands_contract.rs`, `crates/ironclaw_product/tests/product_command_surface_contract.rs`

**Interfaces:**
- Produces: `ProductCommandDescriptor { pub name: &'static str }` (no `aliases`); `validate_declared_product_command("progress")` now errs; `command_spec_for_name` / `ProductCommand::descriptor` match by `name` only.
- Downstream tasks rely on: descriptor struct having exactly `name` after this task (Task 2 adds `audience`).

- [ ] **Step 1: Pin the new contract in tests (red).** In `product_commands_contract.rs`: `grep -n "progress\|aliases" crates/ironclaw_product/tests/product_commands_contract.rs`. At each hit:
  - The descriptor-shape pin asserting `model.aliases.is_empty()` (~line 399): delete the assertion.
  - The `validate_declared_product_command("progress").is_ok()` pin (~line 426): flip it:

```rust
// Aliases are retired: `progress` is no longer a declarable token.
assert!(validate_declared_product_command("progress").is_err());
```

  - Any inventory list containing `"progress"` (~line 128): remove the entry so the expected inventory is names only.

- [ ] **Step 2: Run to verify red.** Run: `cargo test -p ironclaw_product --test product_commands_contract --no-fail-fast`
Expected: FAIL — the flipped `is_err()` assertion fails (alias still validates); compile errors are acceptable red only for the deleted-field assertions.

- [ ] **Step 3: Delete the mechanism in `commands.rs`.**
  - `ProductCommandDescriptor`: remove the `aliases: &'static [&'static str]` field.
  - `COMMAND_SPECS`: remove `aliases: &[]` / `aliases: &["progress"]` from both entries.
  - `product_command_descriptors()`: the lifecycle map no longer sets `aliases`.
  - `validate_declared_product_command`: condition becomes `descriptor.name == name` only.
  - `command_spec_for_name`: `spec.descriptor.name == name` only.
  - `ProductCommand::descriptor()`: `descriptor.name == self.name()` only.

- [ ] **Step 4: Run product suite; fix remaining alias references.** Run: `cargo test -p ironclaw_product --no-fail-fast`
Expected: `product_commands_contract` green. In `product_command_surface_contract.rs` two spots reference `progress`:
  - The context test (~396–414) sending `"progress"` and asserting `context.requested_command == "progress"`: keep it — `progress` is now an unknown token; extend the final assertion to also require the rejection: ack is `Rejected` with `ProductRejectionKind::InvalidRequest` and zero `command_surface.invokes()`.
  - The sensitive-blocks matrix row `("alias", "progress", "")` (~432): keep the row; update its label/comment to `("retired-alias", ...)` — semantics are now "unknown token is rejected".
Re-run until green.

- [ ] **Step 5: Sweep the rest of the workspace for alias references.** Run: `grep -rn "aliases" crates/ironclaw_product/ crates/ironclaw_extension_host/ --include="*.rs" | grep -v test` and `grep -rn "\"progress\"" crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs`
Expected: no production hits remain (fix any). E2E uses `/notacommand`, not `progress` — leave it.

- [ ] **Step 6: Commit.**

```bash
git add crates/ironclaw_product
git commit -m "refactor(commands): retire the alias mechanism

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: Audience vocabulary in the registry

**Files:**
- Modify: `crates/ironclaw_product/src/commands.rs`, `crates/ironclaw_product/src/lib.rs` (export)
- Test: `crates/ironclaw_product/tests/product_commands_contract.rs`

**Interfaces:**
- Consumes: Task 1's descriptor shape (`{ name }`).
- Produces (later tasks depend on these exact names):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandAudience { User, Admin }

pub struct ProductCommandDescriptor {
    pub name: &'static str,
    pub audience: CommandAudience,
}

pub fn required_audience(command: &ProductCommand) -> CommandAudience
```

- [ ] **Step 1: Write the failing contract tests.** Append to `product_commands_contract.rs`:

```rust
#[test]
fn listing_audience_is_user_for_model_and_status_and_admin_for_lifecycle() {
    for descriptor in product_command_descriptors() {
        let expected = match descriptor.name {
            "model" | "status" => CommandAudience::User,
            _ => CommandAudience::Admin, // the lifecycle family
        };
        assert_eq!(descriptor.audience, expected, "descriptor {}", descriptor.name);
    }
}

#[test]
fn execution_audience_is_per_action() {
    let user_cases = [
        ProductCommand::Status,
        ProductCommand::Model { action: ProductModelCommand::Status },
        ProductCommand::Unknown { name: "nope".into(), arguments: String::new() },
    ];
    for command in user_cases {
        assert_eq!(required_audience(&command), CommandAudience::User, "{command:?}");
    }
    let admin_cases = [
        ProductCommand::Model { action: ProductModelCommand::Set { model: "m".into() } },
        ProductCommand::Model {
            action: ProductModelCommand::SetProvider { provider: "p".into(), model: None },
        },
        ProductCommand::Lifecycle { action: LifecycleProductAction::ExtensionList },
    ];
    for command in admin_cases {
        assert_eq!(required_audience(&command), CommandAudience::Admin, "{command:?}");
    }
}
```

Import `CommandAudience`, `required_audience`, `ProductModelCommand`, `LifecycleProductAction` from `ironclaw_product` (add to the existing `use`; check `lib.rs` re-exports and extend them if a name is missing).

- [ ] **Step 2: Run to verify red.** Run: `cargo test -p ironclaw_product --test product_commands_contract --no-fail-fast`
Expected: FAIL to compile — `CommandAudience` / `required_audience` / `audience` field do not exist.

- [ ] **Step 3: Implement in `commands.rs`.** Add the enum (above). Add `audience` to `ProductCommandDescriptor`. Set it at every construction: `model` → `CommandAudience::User`, `status` → `CommandAudience::User` in `COMMAND_SPECS`; the lifecycle map in `product_command_descriptors()` → `CommandAudience::Admin`. Add:

```rust
/// Execution audience, action-aware: `/model` bare is a user-safe read while
/// its `set`/`set-provider` actions mutate operator-wide LLM configuration.
/// `Unknown` is `User` — it never executes (admission rejects undeclared
/// tokens before the audience step) and must not hide behind the admin gate.
pub fn required_audience(command: &ProductCommand) -> CommandAudience {
    match command {
        ProductCommand::Model {
            action: ProductModelCommand::Status,
        } => CommandAudience::User,
        ProductCommand::Model { .. } => CommandAudience::Admin,
        ProductCommand::Status => CommandAudience::User,
        ProductCommand::Lifecycle { .. } => CommandAudience::Admin,
        ProductCommand::Unknown { .. } => CommandAudience::User,
    }
}
```

Export `CommandAudience` and `required_audience` from `lib.rs` beside the existing `commands` re-exports.

- [ ] **Step 4: Run to verify green.** Run: `cargo test -p ironclaw_product --no-fail-fast`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add crates/ironclaw_product
git commit -m "feat(commands): add user/admin audience vocabulary to the registry

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: Observer — admin-denial copy and user-audience help

**Files:**
- Modify: `crates/ironclaw_product/src/run_delivery/observer.rs`, `docs/superpowers/specs/2026-07-29-product-command-train-design.md`
- Test: `crates/ironclaw_product/tests/run_delivery_contract.rs`

**Interfaces:**
- Consumes: Task 2's `CommandAudience` + `product_command_descriptors()`.
- Produces: `post_command_feedback` maps `ProductRejectionKind::AccessDenied` → fixed copy `"This command requires an admin account."`; `with_enabled_commands` includes only user-audience names in the static help.

- [ ] **Step 1: Write the failing tests.** In `run_delivery_contract.rs`, locate the existing command-feedback tests (`grep -n "post_command_feedback\|command_help\|PolicyDenied" crates/ironclaw_product/tests/run_delivery_contract.rs`) and mirror their harness shape for two new cases:

```rust
#[tokio::test]
async fn access_denied_command_rejection_delivers_admin_notice() {
    // Build the observer + envelope exactly as the existing PolicyDenied
    // feedback test does, but with:
    //   ProductInboundAck::Rejected(ProductRejection::permanent(
    //       ProductRejectionKind::AccessDenied,
    //       "admin-audience command from a non-admin actor",
    //   ))
    // Assert the delivered text is exactly:
    //   "This command requires an admin account."
    // and that the internal reason string never appears in the delivery.
}

#[tokio::test]
async fn static_command_help_excludes_admin_audience_commands() {
    // Build the observer with
    //   .with_enabled_commands(["model", "status", "extension_configure"])
    // and drive the same InvalidRequest feedback path the existing help test
    // uses. Assert the delivered help text is exactly
    //   "Available commands:\n/model\n/status"
    // (no /extension_configure).
}
```

Write them as real tests by copying the existing feedback-test harness (observer construction, envelope, ack) — the comments above specify only the deltas.

- [ ] **Step 2: Run to verify red.** Run: `cargo test -p ironclaw_product --test run_delivery_contract --no-fail-fast`
Expected: FAIL — AccessDenied currently falls into the silent-settle `_` arm (no delivery), and help includes `/extension_configure`.

- [ ] **Step 3: Implement.** In `observer.rs`:
  - In `post_command_feedback` (~line 898), add an arm above `PolicyDenied`:

```rust
ProductRejectionKind::AccessDenied => {
    "This command requires an admin account.".to_string()
}
```

  - In `with_enabled_commands` (~line 195), filter to user-audience names before building the help:

```rust
pub fn with_enabled_commands<I, S>(mut self, commands: I) -> Self
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let user_visible: Vec<String> = commands
        .into_iter()
        .filter(|name| {
            product_command_descriptors().any(|descriptor| {
                descriptor.name == name.as_ref()
                    && descriptor.audience == CommandAudience::User
            })
        })
        .map(|name| name.as_ref().to_string())
        .collect();
    self.command_help_text = declared_command_help_text(user_visible);
    self
}
```

  - Check `command_help_text()` (the full-inventory helper in `commands.rs`): `grep -rn "command_help_text()" crates/ --include="*.rs"`. If its only callers are its own tests, delete the function and its test (delete-culture); otherwise leave it.

- [ ] **Step 4: Run to verify green.** Run: `cargo test -p ironclaw_product --no-fail-fast`
Expected: PASS, including all pre-existing feedback pins.

- [ ] **Step 5: Sync the spec.** In `docs/superpowers/specs/2026-07-29-product-command-train-design.md`, PR-1 Admission section, replace the sentence claiming admission builds role-aware help at rejection time with: "Help is role-safe by filtering: the observer's static help includes only user-audience declared commands, for every actor. Admission rejections carry internal reasons only; the observer never echoes them. The admin denial is keyed by the reused wire-stable `ProductRejectionKind::AccessDenied`."

- [ ] **Step 6: Commit.**

```bash
git add crates/ironclaw_product docs/superpowers/specs/2026-07-29-product-command-train-design.md
git commit -m "feat(commands): deliver admin-denial notices and user-audience help

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: Role-resolver port, admission audience step, production wiring

**Files:**
- Modify: `crates/ironclaw_product/src/command_admission.rs`, `crates/ironclaw_product/src/lib.rs`
- Create: `crates/ironclaw_extension_host/src/channel_command_roles.rs` (+ `lib.rs` module line)
- Modify: `crates/ironclaw_extension_host/src/channel_host.rs` (deps field ~331, admission construction ~761), `crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs` (deps ~426), `crates/ironclaw_reborn_composition/src/factory.rs` (~1260), `crates/ironclaw_reborn_composition/src/runtime.rs` (~1664)
- Test: `crates/ironclaw_product/tests/product_command_surface_contract.rs`

**Interfaces:**
- Consumes: Task 2's `required_audience` / `CommandAudience`; existing `AdminUserService::get_user(&TenantId, &UserId) -> Result<Option<AdminUserRecord>, AdminUserError>`; `AdminUserRole::is_admin()`; `AdminUserStatus`; `RebornUserIdentityLookup::resolve_user_identity(provider, provider_user_id) -> Result<Option<UserId>, _>`.
- Produces:

```rust
// ironclaw_product::command_admission
#[async_trait]
pub trait CommandActorRoleResolver: Send + Sync {
    async fn actor_role(
        &self,
        context: &ProductCommandContext,
    ) -> Result<Option<AdminUserRole>, ProductSurfaceError>;
}
pub struct DirectConversationCommandAdmission { /* private */ }
impl DirectConversationCommandAdmission {
    pub fn new<I, S>(
        commands: I,
        roles: Arc<dyn CommandActorRoleResolver>,
    ) -> Result<Self, UnknownProductCommandName>;
}

// ironclaw_extension_host::channel_command_roles
pub struct ChannelActorRoleResolver { /* private */ }
impl ChannelActorRoleResolver {
    pub fn new(
        provider: String,
        identity_lookup: Option<Arc<dyn RebornUserIdentityLookup>>,
        admin_users: Arc<dyn AdminUserService>,
        tenant: TenantId,
        operator_user_id: UserId,
    ) -> Self;
}

// GenericChannelHostDeps gains:
pub admin_users: Arc<dyn ironclaw_product::AdminUserService>,
```

- [ ] **Step 1: Write the failing admission-matrix tests.** In `product_command_surface_contract.rs`, add a fake resolver beside the existing fakes:

```rust
struct FakeRoleResolver {
    role: Option<AdminUserRole>,
    fail: bool,
}

#[async_trait::async_trait]
impl CommandActorRoleResolver for FakeRoleResolver {
    async fn actor_role(
        &self,
        _context: &ProductCommandContext,
    ) -> Result<Option<AdminUserRole>, ProductSurfaceError> {
        if self.fail {
            return Err(ProductSurfaceError::from_status(
                ProductSurfaceErrorCode::Unavailable,
                503,
                true,
            ));
        }
        Ok(self.role)
    }
}
```

(Confirm the exact `ProductSurfaceErrorCode` variant with `grep -n "Unavailable" crates/ironclaw_host_api/src/*.rs` — use the taxonomy's service-unavailable code.) Then four tests, each mirroring the existing `manifest_command_admission_is_exact_and_blocks_sensitive_handlers` workflow harness (`DefaultProductSurface::new(...)` + `with_product_command_admission_service` + `with_product_command_surface` + `sample_command_envelope_with_trigger` + `submit_inbound`), with admission built as `DirectConversationCommandAdmission::new(["model", "status"], Arc::new(FakeRoleResolver { .. }))`:

```rust
// 1. member_admin_action_is_access_denied_without_execution:
//    role: Some(AdminUserRole::Member), command "model", args "set gpt-x"
//    → ack Rejected with kind == ProductRejectionKind::AccessDenied,
//      command_surface.invokes().is_empty()
// 2. member_user_action_executes: role Some(Member), command "model", args ""
//    → command_surface invoke recorded for "product.model.command"
// 3. admin_admin_action_executes: role Some(AdminUserRole::Owner),
//    command "model", args "set gpt-x" → invoke recorded
// 4. resolver_failure_is_a_retryable_error_not_silent_membership:
//    fail: true, command "model", args "set gpt-x"
//    → workflow.submit_inbound(...) returns Err (the product-surface
//      failure path), and command_surface.invokes().is_empty()
```

Write them fully (copy the existing harness lines; the four comments define the deltas). Also update every existing `DirectConversationCommandAdmission::new([...])` call in this file to pass `Arc::new(FakeRoleResolver { role: Some(AdminUserRole::Member), fail: false })` — existing tests exercise user-audience paths and must stay green with a member.

- [ ] **Step 2: Run to verify red.** Run: `cargo test -p ironclaw_product --test product_command_surface_contract --no-fail-fast`
Expected: FAIL to compile — `CommandActorRoleResolver` and the two-arg `new` do not exist.

- [ ] **Step 3: Implement the port + audience step in `command_admission.rs`.**

```rust
use crate::commands::{CommandAudience, required_audience};
use crate::reborn_services::{AdminUserRole};
use std::sync::Arc;

/// Resolves the admin-boundary role of the ACTIVE bound user behind an
/// inbound channel actor. `Ok(None)` means unbound actor, missing record, or
/// suspended account — all treated as not-admin (fail closed). `Err` means
/// transient resolution failure; the command fails retryable rather than
/// silently degrading to member or admin treatment.
#[async_trait]
pub trait CommandActorRoleResolver: Send + Sync {
    async fn actor_role(
        &self,
        context: &ProductCommandContext,
    ) -> Result<Option<AdminUserRole>, ProductSurfaceError>;
}

pub struct DirectConversationCommandAdmission {
    allowed_commands: BTreeSet<String>,
    roles: Arc<dyn CommandActorRoleResolver>,
}
```

`new` gains the `roles: Arc<dyn CommandActorRoleResolver>` parameter and stores it. In `admit`, after the declared-set check:

```rust
if required_audience(command) == CommandAudience::Admin {
    let role = self.roles.actor_role(context).await?;
    if !role.is_some_and(AdminUserRole::is_admin) {
        return Ok(ProductCommandAdmission::Rejected(
            ProductRejection::permanent(
                ProductRejectionKind::AccessDenied,
                "admin-audience command from a non-admin actor",
            ),
        ));
    }
}
```

(If `AdminUserRole` is not already re-exported at the crate root for the `use` above, import it as `crate::reborn_services::admin_users::AdminUserRole` and add a root re-export in `lib.rs` beside the other admin-user types. Export `CommandActorRoleResolver` from `lib.rs`.)

- [ ] **Step 4: Run the product tier.** Run: `cargo test -p ironclaw_product --no-fail-fast`
Expected: the four new tests PASS; `ironclaw_extension_host` does not compile yet — that is the next step, not a stopping point.

- [ ] **Step 5: Production resolver in `ironclaw_extension_host`.** Create `crates/ironclaw_extension_host/src/channel_command_roles.rs` (register `pub mod channel_command_roles;` in `lib.rs`):

```rust
//! Production role resolution for channel-command admission: verified inbound
//! actor → bound IronClaw user (channel identity binding) → active-account
//! admin-boundary role (admin-users directory).

use async_trait::async_trait;
use ironclaw_host_api::{ProductSurfaceError, RebornUserIdentityLookup, TenantId, UserId};
use ironclaw_product::{
    AdminUserError, AdminUserRole, AdminUserService, AdminUserStatus,
    CommandActorRoleResolver, ProductCommandContext,
};
use std::sync::Arc;

pub struct ChannelActorRoleResolver {
    provider: String,
    identity_lookup: Option<Arc<dyn RebornUserIdentityLookup>>,
    admin_users: Arc<dyn AdminUserService>,
    tenant: TenantId,
    operator_user_id: UserId,
}

impl ChannelActorRoleResolver {
    pub fn new(
        provider: String,
        identity_lookup: Option<Arc<dyn RebornUserIdentityLookup>>,
        admin_users: Arc<dyn AdminUserService>,
        tenant: TenantId,
        operator_user_id: UserId,
    ) -> Self {
        Self { provider, identity_lookup, admin_users, tenant, operator_user_id }
    }

    fn unavailable() -> ProductSurfaceError {
        // Use the same service-unavailable constructor grep'd in Task 4 Step 1.
        ProductSurfaceError::from_status(
            ironclaw_host_api::ProductSurfaceErrorCode::Unavailable,
            503,
            true,
        )
    }
}

#[async_trait]
impl CommandActorRoleResolver for ChannelActorRoleResolver {
    async fn actor_role(
        &self,
        context: &ProductCommandContext,
    ) -> Result<Option<AdminUserRole>, ProductSurfaceError> {
        let user_id = match &self.identity_lookup {
            Some(lookup) => match lookup
                .resolve_user_identity(&self.provider, context.external_actor_ref.id())
                .await
            {
                Ok(Some(user_id)) => user_id,
                Ok(None) => return Ok(None),
                Err(_) => return Err(Self::unavailable()),
            },
            // Composition paths without the durable identity store run under
            // the operator-actor policy: the operator IS the actor.
            None => self.operator_user_id.clone(),
        };
        match self.admin_users.get_user(&self.tenant, &user_id).await {
            Ok(Some(record)) if record.status == AdminUserStatus::Active => Ok(Some(record.role)),
            Ok(_) => Ok(None),
            Err(AdminUserError::Unavailable) => Err(Self::unavailable()),
            Err(_) => Err(ProductSurfaceError::from_status(
                ironclaw_host_api::ProductSurfaceErrorCode::Internal,
                500,
                false,
            )),
        }
    }
}
```

Adjust the two `ProductSurfaceErrorCode` variants and the `ExternalActorRef` accessor (`.id()`) to the actual names found by: `grep -n "pub enum ProductSurfaceErrorCode" -A 20 crates/ironclaw_host_api/src/*.rs` and `grep -n "impl ExternalActorRef" -A 15 crates/ironclaw_product/src/*.rs crates/ironclaw_host_api/src/**/*.rs`. Confirm the provider string convention against the existing consumer `crates/ironclaw_extension_host/src/provider_identity.rs:144` (`.resolve_user_identity(&self.provider, provider_user_id)`) and source `provider` from the same per-extension value the assembly gives that policy.

- [ ] **Step 6: Thread deps + per-extension construction.** In `channel_host.rs`:
  - Add to `GenericChannelHostDeps` (~331): `pub admin_users: Arc<dyn ironclaw_product::AdminUserService>,` with a doc comment: `/// Admin-users directory backing channel-command role gating.`
  - At the admission construction (~761), build the resolver with the same provider value the provider-identity policy for this extension uses (grep `provider` within this assembly function), then:

```rust
let roles = Arc::new(crate::channel_command_roles::ChannelActorRoleResolver::new(
    provider.clone(),
    self.deps.identity_lookup.clone(),
    Arc::clone(&self.deps.admin_users),
    self.deps.identity.tenant_id.clone(),
    self.deps.identity.operator_user_id.clone(),
));
workflow = workflow.with_product_command_admission_service(Arc::new(
    ironclaw_product::DirectConversationCommandAdmission::new(
        channel.commands.iter().map(String::as_str),
        roles,
    )
    .map_err(|error| { /* keep the existing error-mapping closure */ })?,
));
```

- [ ] **Step 7: Update the three deps construction sites.**
  - `channel_host/e2e_tests.rs` (~426): add a fake — define beside the harness fakes:

```rust
#[derive(Default)]
struct FakeAdminUsers {
    roles: std::sync::Mutex<std::collections::HashMap<String, AdminUserRole>>,
}
// impl AdminUserService: get_user returns an Active AdminUserRecord with the
// mapped role (default AdminUserRole::Member for unknown ids); list/create/
// update/delete return Err(AdminUserError::Internal) — the harness never
// calls them. Copy field shapes from
// crates/ironclaw_product/src/reborn_services/admin_users.rs.
```

    Wire `admin_users: Arc::new(FakeAdminUsers::default())` into the deps literal for now (Task 5 makes it configurable).
  - `factory.rs` (~1260) and `runtime.rs` (~1664): thread the `AdminUserService` the composition already builds for the WebUI admin routes (`RebornAdminUserDirectory` in `crates/ironclaw_reborn_composition/src/admin_user_directory.rs`, constructed in `product_surface.rs`). Locate that construction (`grep -n "RebornAdminUserDirectory" crates/ironclaw_reborn_composition/src/`), share the `Arc` (store it on the same struct that carries `channel_identity_store`, or construct a second adapter from the same `RebornUserDirectory` handle), and set `admin_users:` in both deps literals.

- [ ] **Step 8: Run the crate suites.** Run: `cargo test -p ironclaw_product -p ironclaw_extension_host --no-fail-fast` and `cargo build -p ironclaw_reborn_composition`
Expected: PASS / compile clean. E2E command tests still pass — the bundled manifest still declares only `status` (user-audience) so role gating is not yet observable there.

- [ ] **Step 9: Run architecture tier (dependency edges changed).** Run: `cargo test -p ironclaw_architecture --no-fail-fast`
Expected: PASS — `extension_host → ironclaw_product` already exists; no new edges.

- [ ] **Step 10: Commit.**

```bash
git add crates/ironclaw_product crates/ironclaw_extension_host crates/ironclaw_reborn_composition
git commit -m "feat(commands): role-gate admin-audience command actions at admission

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: Manifests declare model+status; end-to-end role proof

**Files:**
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/manifest.toml` (~183), `crates/ironclaw_first_party_extensions/assets/telegram/manifest.toml` (~35)
- Test: `crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs`

**Interfaces:**
- Consumes: Task 4's `FakeAdminUsers` + deps field; existing harness (`HarnessOptions`, `build_harness_with_options`, `post_event`, `wait_for_post_messages_matching`, `harness.command_executions.invokes()`, `harness.coordinator.submitted_turn_count()`).
- Produces: `HarnessOptions.actor_role: AdminUserRole` (default `Member`).

- [ ] **Step 1: Write the failing e2e tests.** In `e2e_tests.rs`:
  - Add `actor_role: AdminUserRole` to `HarnessOptions` with default `AdminUserRole::Member`; in `build_harness_with_options`, seed `FakeAdminUsers` so the harness's bound user id (the `USER` const the dm-command test asserts as caller) maps to `options.actor_role`.
  - Add a DM slash body const beside the existing `DM_COMMAND` (copy the JSON shape at ~3510, text `"/model set fake-model"`, fresh `ts`/event id): `DM_MODEL_SET`.
  - Two tests mirroring `dm_slash_command_executes_and_delivers_rendered_result` (~4644):

```rust
#[tokio::test]
async fn member_dm_model_set_is_denied_with_admin_notice_and_no_execution() {
    // default options (Member); post DM_MODEL_SET; drain.
    // Assert egress delivered exactly "This command requires an admin account.",
    // harness.command_executions.invokes().is_empty(),
    // submitted_turn_count unchanged (no turn).
}

#[tokio::test]
async fn admin_dm_model_set_executes_via_command_surface() {
    // options.actor_role = AdminUserRole::Owner; post DM_MODEL_SET; drain.
    // Assert exactly one invoke ("product.model.command", USER),
    // and a rendered "Model updated"-titled feedback is NOT required here —
    // the fake surface returns a stub payload; assert the invoke instead.
}
```

  - Update the existing help pin `unknown_dm_slash_command_returns_inventory_help_without_a_turn` (~4680): expected text becomes `"Available commands:\n/model\n/status"`; delete the now-wrong `!text.contains("/model")` assertion; keep the lifecycle exclusions.

- [ ] **Step 2: Run to verify red.** Run: `cargo test -p ironclaw_extension_host --no-fail-fast -- channel_host`
Expected: the two new tests FAIL (member `/model set` currently rejected as *undeclared* with help text, not the admin notice — manifest still declares only `status`), and the help pin FAILS (no `/model` yet).

- [ ] **Step 3: Declare the commands.** In both manifests change `commands = ["status"]` → `commands = ["model", "status"]`.

- [ ] **Step 4: Run to verify green.** Run: `cargo test -p ironclaw_extension_host --no-fail-fast`
Expected: PASS — denial comes from the audience step now, help lists both commands, admin path invokes.

- [ ] **Step 5: Full-suite sweep of every crate the PR touched.** Run: `cargo test -p ironclaw_product -p ironclaw_extension_host -p ironclaw_first_party_extensions -p ironclaw_reborn_composition --no-fail-fast`
Expected: PASS (fix anything surfaced; do not weaken pre-existing assertions — stop and report if a surfaced failure looks like real behavior).

- [ ] **Step 6: Commit.**

```bash
git add crates/ironclaw_first_party_extensions crates/ironclaw_extension_host
git commit -m "feat(channels): declare model+status on bundled channels behind role gating

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: Gauntlet, spec sync, PR

**Files:**
- Modify (if drift found): touched crates only.

- [ ] **Step 1: Format.** Run: `cargo fmt`
Expected: clean diff or formatting-only changes (amend into the last commit if any).

- [ ] **Step 2: Clippy, both lanes (feature-matrix rule).**
Run: `cargo clippy --all --tests --examples -- -D warnings`
Run: `cargo clippy --all --tests --examples --all-features -- -D warnings`
Expected: zero warnings in both.

- [ ] **Step 3: Architecture + safety gates.**
Run: `cargo test -p ironclaw_architecture --no-fail-fast`
Run: `scripts/pre-commit-safety.sh`
Expected: PASS. (ARCH-SPRAWL flags on uncommitted merges are known false positives; there is no merge here, so investigate any hit.)

- [ ] **Step 4: Reborn integration harness (contract changed).**
Run: `RUST_MIN_STACK=67108864 bash scripts/reborn-e2e-rust.sh`
Expected: PASS (the stack floor is the precedented local requirement for the integration lanes).

- [ ] **Step 5: Re-read the spec against the diff.** Open `docs/superpowers/specs/2026-07-29-product-command-train-design.md` PR-1 section; confirm every bullet is implemented or explicitly amended (Task 3 Step 5 already amended the help design). Fix any drift in the spec, not the code, if the code matches approved decisions.

- [ ] **Step 6: Push and open the PR — CONFIRM WITH BEN FIRST.** Pause and ask before pushing. Then:

```bash
git push -u origin ivy-coreopsis
gh pr create --base main --title "feat(commands): role-gate admin command actions (PR-1 of command train)" --body "$(cat <<'EOF'
Implements PR-1 of docs/superpowers/specs/2026-07-29-product-command-train-design.md:
audience vocabulary (user/admin, per-action), CommandActorRoleResolver port with
the production channel resolver (identity binding -> active-account role),
AccessDenied admin denials with fixed observer copy, user-audience help
filtering, alias mechanism retired, bundled Slack/Telegram declare
["model", "status"].

Closes the operator-wide /model set backdoor for non-admin channel actors.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

## Self-review checklist (run after writing code, before the PR)

- Spec coverage: registry audience ✔ (Task 2), role port + fail-closed ✔ (Task 4), admission order direct→declared→audience ✔ (Task 4), distinct admin notice ✔ (Task 3), role-safe help ✔ (Task 3, amended), manifests ✔ (Task 5), alias deletion ✔ (Task 1), e2e seam asserts ✔ (Task 5).
- No `.unwrap()`/`.expect()`/`unwrap_or_default()` on the new production paths.
- Every new rejection path asserts zero command-surface invokes.
