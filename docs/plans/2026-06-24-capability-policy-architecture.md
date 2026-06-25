# Capability Policy Architecture (v1)

**Status:** settled design — ready to outline implementation
**Date:** 2026-06-24 (settled 2026-06-25)
**Build target:** IronClaw **Reborn** (Rust) — Reborn crates only, **not** engine v2 or legacy `src/`. goclaw is **reference only** (Appendix B).
**Continues:** nearai/ironclaw#4628 — this design picks up from it (see §1.5).

## Goal

Let a company run one hosted IronClaw where an **admin** decides which
tools/skills each **user** can use, with what settings, under whose credentials,
and with what approval behaviour — while users add and operate their own. The
"customer-company with an admin + users" case is one configuration of a general
model, not a special path.

---

## 1. Settled decisions

- **Build target = Reborn.** goclaw is reference only.
- **One account type — `User`.** A user is *secrets + tool access + its own
  memory*. It is a **person** or a **shared SSO account** (e.g.
  `engineering@company.com`).
- **Roles `Owner > Admin > Member`** — a role a User *holds*, not a kind of
  account.
- **Tenant = the company**, and for v1 **tenant = the default project**. Projects
  are the future scoping unit; the default project spans the whole tenant.
- **Capabilities** (tools + skills in v1; MCP/extensions later) are governed on
  **four dimensions**: availability, configuration, identity, approval.
- **Per-capability defaults** in the capability manifest; policy resolves
  `capability default → tenant baseline (admin-set) → user`.
- **Identity is user-keyed or admin-keyed** (who provides the credential).
- **Admin curates; users self-serve where allowed.**
- **Memory is per-user.** No project/tenant shared memory — a shared account is
  one user, so its memory *is* the team's shared memory.
- **Bot / room addressability is a future seam.** The model already allows a
  User to be addressed by others; we build no Slack/channel code in v1.

Deferred / minor (don't block the build): config-merge semantics, whether
`Deny` is a hard floor, who holds the tenant **Owner** role, bot-naming.

---

## 1.5 Prior art — anchor to #4628, compose with the Reborn track

This design **continues [#4628 "Admin-shared tools and skills with per-user
auth"]**, the canonical issue for this exact problem (same user stories, same
open question, same benchmark link). #4628 is the *initial step*; this is the
continuation.

**Target: the Reborn stack only** — `crates/ironclaw_reborn*`,
`ironclaw_product_workflow`, and the Reborn lifecycle / package / trust /
credential / capability services. **Not** engine v2, **not** the legacy `src/`
engine. Where a primitive already exists in Reborn we **compose with it**; and
per #4628's *Canonical interface*, we do **not** mutate model-visible tools from
route handlers or use process-global registries / shared FS scan roots — tool
visibility flows through `CapabilityCatalog` / `ToolSurfaceService` style
boundaries.

### Already landed — build on top
| What | Issue/PR | How our model uses it |
|---|---|---|
| DB-backed users, **RBAC**, admin secrets provisioning, multi-tenant isolation | **#1626** (merged) | the `User` + role + per-user secrets substrate — don't re-invent roles |
| Product-auth: per-user OAuth, credential setup/refresh, account-scoped staging + most-recently-used runtime selection | **#3289**, **#4354** (landed) | **the Identity dimension (user-keyed)** — this *is* the per-user-auth mechanism |
| User-scoped skills in Settings (system/workspace read-only, isolated storage) | **#4527** (landed) | the user-private overlay for skills |
| Local-dev extension lifecycle registry (catalog + active registry) | **#4066** (landed) | capability-catalog plumbing |
| Approval "always-allow" persisted as tool settings; Slack admin/operator-token visibility | **#5195**, **#5185** (merged) | **the Approval dimension** persistence + the admin/operator surface |

### In-flight — the foundation we build *into* (newer than #4544)
| What | Issue/PR | Relationship |
|---|---|---|
| **Scoped lifecycle ownership + effective package-set resolution** for admin-shared & user-private installs | **#4544** (open PR) | **this is our `CapabilityPolicyStore` + availability resolution** — extend it, don't parallel it |
| Expose **user-scoped tool settings** | **#5256** (open PR) | the Availability dimension, user layer |
| **Slack personal (user-token) tool** | **#5177** (open PR) | the exact "shared app, each user connects own account" flow |
| Model **memory as a userland extension** (#3537) | **#5205** (open PR) | the per-user **memory** in "User = secrets + tools + memory" |
| Hosted single-tenant volume profile | **#5259** (open PR) | the hosting shape for a tenant |

### Direction to lower into (don't reinvent the policy format)
| What | Issue/PR | Relationship |
|---|---|---|
| Declarative `CapabilityPolicySpec` — trust ceilings, effects, mounts, network, **approval kept separate from grants** | **#4120** | our four product dimensions sit **above** this runtime grant model; lower into it rather than invent a parallel format |
| Config-as-Code epic (tenant blueprints, harnesses) | **#3036** | the home for declarative tenant policy |
| Production / multi-tenant extension lifecycle wiring | **#4091** | the scoped-lifecycle backend our admin surface needs |

### Gaps this design must close (the continuation work)
- **Tenant publish surface + precedence** — no canonical product/API to publish a
  tool/skill tenant-wide with deterministic `system → tenant/admin → project →
  user-private` precedence (storage foundation in #4544; surface missing).
- **Admin custom-tool lifecycle** — trust, ownership, update, rollback,
  disable/revoke, visibility (#4628 gap).
- **Propagation without restart** — admin changes must reach already-running user
  agents. **#3490** ("admin tools settings not propagated to users") is the open
  bug; **#5242** ("Tools page shows operator-only tools error for WebUI users")
  shows the surface is live but rough.
- **Bind user-owned credential accounts to admin-owned config** — the open
  question; our admin-keyed/user-keyed split must reconcile with #4354's per-user
  credential-account selection.
- **Benchmark** — non-LLM vault state: **benchmarks#59** (vault service:
  `secret_list`/`secret_delete` + between-turn OAuth events) and **benchmarks#62**.
  Required because these flows mutate state outside the LLM loop.

### One reconciliation to make
Our **"shared account = a User you SSO into"** (`engineering@`) and the issues'
**"admin-managed app config + each member's own credential account selected at
runtime"** (#4354) are *two mechanisms* for the same need. Choose one (or define
when each applies) before implementing — shipping both is the risk #4628 warns of.

---

## 2. Vocabulary

- **Channel** — a way to *talk to* IronClaw (Slack, Telegram, Web). Transport only.
- **Room** — what Slack calls a "channel" (`#eng`): an external space/group. Not
  a channel here. Relevant only to the future bot seam (§13).
- **Capability** — a tool, skill, or extension; the governed thing.
- **User** — the one account type; the principal grants attach to.
- **Project** — a shared scope/substrate; the default project == the tenant.
- **Role** — `Owner | Admin | Member`, held by a User.

---

## 3. Primitives

| Primitive | What it is | Notes |
|---|---|---|
| **Tenant** | the company | hard isolation wall; owns no policy itself; **= default project (v1)** |
| **Project** | a shared scope/substrate | roles + policy baseline; one **default** per tenant, users auto-join; **dormant beyond the default until multi-project (future)** |
| **User** | the one account type | **secrets + tool grants + own memory**; person or shared SSO account; the principal |
| **Capability** | tool / skill / extension | carries a per-capability **default policy** (§6) |

A run is `(tenant, user)` for v1 (`project` is implicit = default). **User**
answers *who acts*; the **persona/runtime** (system prompt, model) is an attribute
of the User — in Reborn the existing `AgentId` sits under the account, not as a
product concept.

---

## 4. Roles

A User holds exactly one role on the tenant (= default project):

- **Owner** — governance: delete/transfer the tenant, billing, **appoint admins**.
  (Holder TBD — company top person or NEAR AI as host; non-blocking.)
- **Admin** — operations: **create users** (incl. shared accounts), set their
  **capabilities / configuration / approval policy**, provide **admin-keyed**
  credentials, manage members. (`director@` is an Admin.)
- **Member** — a regular user: uses what they're granted; sets their **own keys**
  for user-keyed tools; answers approval prompts on accounts they operate.
  Operates only themselves — plus any shared account they can SSO into (§5).

Role ≠ account type. `director@` is a *User* with the *Admin* role;
`engineering@` is a *User* with the *Member* role.

---

## 5. Users, personal and shared

Every actor is a `User` carrying three things — **secrets, tool grants, and its
own memory**. Two flavours, distinguished only by how they authenticate:

**Personal user** (e.g. `director@`)
- authenticates as a person (SSO).
- private — only that person acts as it.
- uses its own credentials and memory.

**Shared account** (e.g. `engineering@company.com`)
- a User whose login is a **shared SSO account**. Anyone the IdP (Google
  Workspace) lets sign in as `engineering@company.com` can log into IronClaw *as*
  `engineering@`. **"Who may operate it" is governed by the IdP — there is no
  IronClaw operator list.** IronClaw sees one User.
- has its **own memory** → this *is* the shared team memory.
- **two config surfaces:**
  - *logged in as `engineering@`* → set keys for its **user-keyed** tools and
    answer its **approval prompts**.
  - *as an Admin* (`director@`, via webui / API, without logging in as it) → set
    which **capabilities** it has, provide **admin-keyed** credentials, set its
    **approval policy**.

A shared account is "a user with limited capabilities" exactly because an Admin
decides what it may do, while whoever operates it fills in the rest.

---

## 6. The four dimensions

Every capability is governed on four orthogonal dimensions:

| Dimension | Answers | Values | Who sets it |
|---|---|---|---|
| **Availability** | Can this user see/invoke it? | `Available \| Hidden` | Admin (per-capability default → tenant → per-user grants) |
| **Configuration** | What shared settings does it run with? | JSON | Admin |
| **Identity** | Whose credential does the call use? | `None \| user-keyed \| admin-keyed` | mode = Admin; **the key itself**: the *user* (user-keyed) or the *Admin* (admin-keyed) |
| **Approval** | Prompt / always-allow / deny? | `Prompt \| AlwaysAllow \| Deny` | Admin (policy); the **operator answers** the prompts |

### Identity, concretely (the "two kinds of tool")

- **user-keyed** — the tool is available and the **user sets their own key**
  ("introduce yourself"). Missing key → an **auth gate** prompts the user to add it.
- **admin-keyed** — the tool is available but **only the Admin sets the key**; the
  user just uses it and cannot set it. Missing key → the tool is simply **not
  usable** until the Admin provides it.

A capability that needs a key but has none **resolves to unavailable** for that
user (with the user-keyed case offering the auth gate).

---

## 7. Per-capability defaults

Each capability declares a default in its **manifest**:

| Capability | availability | approval | identity |
|---|---|---|---|
| `builtin.web_search` | Available | Prompt | None |
| `builtin.time` | Available | AlwaysAllow | None |
| `builtin.shell` | Hidden | Prompt | None |
| `mcp.slack` | Hidden | Prompt | user-keyed |
| `skill.code-review` | Available | AlwaysAllow | None |

Zero admin config ⇒ a user already has the safe built-ins and nothing
credentialed.

---

## 8. Resolution

`resolve(user, capability) -> EffectivePolicy`. For v1 (tenant = default project):

```
1. acc = capability.default_policy            # the manifest
2. acc = apply(acc, tenant_delta[capability])  # admin-set baseline for everyone
3. acc = apply(acc, user_delta[capability])    # per-user: admin grants + user's own key
4. return acc
```

`apply` overrides each present field (deep-merge for `config`). Resolution is
**live** — never cached across a request. The **user layer has two writers** on
different fields: the **Admin** writes per-user availability/config/approval
(e.g. "web_search for Bob only"); the **user** writes their own credential for
user-keyed tools. The **project layer** (between tenant and user) is the
insertion point when multi-project lands.

`EffectivePolicy = { available, config, identity_mode, credential_ref, approval }`.

---

## 9. Enforcement — one gate

All four dimensions are enforced at **tool dispatch** (`ToolDispatcher::dispatch`
— everything already routes through it). The resolver is consulted once per call:

| Dimension | Enforcement |
|---|---|
| Availability | `Hidden` (or needs-key-and-none) ⇒ not in the catalog **and** dispatch rejects |
| Configuration | merged `config` injected into the invocation |
| Identity | credential resolved; user-keyed + missing ⇒ auth gate; admin-keyed + missing ⇒ unavailable |
| Approval | `AlwaysAllow` ⇒ proceed · `Prompt` ⇒ approval gate · `Deny` ⇒ reject |

---

## 10. Data model (logical)

```
Tenant            id
Project           id, tenant_id, is_default          # default == tenant (v1)
User              id, tenant_id, role(Owner|Admin|Member),
                  kind(person|shared), auth(SSO ref),  # "who can be a shared account" = IdP
                  memory_ref                            # per-user memory
Capability        id, kind, default_policy             # from manifest
CapabilityPolicyDelta
                  scope(Project|User), scope_id, capability_id,
                  availability?, config_patch?, identity_mode?, approval?   # sparse
CapabilityCredential
                  capability_id, tenant_id, user_id?,   # user_id null = admin-keyed/shared
                  secret_ref, status(Active|NeedsAuth)
```

Notes: a User's **secrets** = its `CapabilityCredential` rows; its **tools** =
the resolved availability over `CapabilityPolicyDelta`; its **memory** =
`memory_ref`. No separate Agent/subject entity; no IronClaw operator list (IdP
owns that).

---

## 11. Ports (interfaces to build)

```
CapabilityCatalog       list/describe capabilities (+ default_policy)
CapabilityPolicyStore   get/upsert/delete deltas at scope Project|User   # admin-gated writes
PolicyResolver          resolve(user_id, capability_id) -> EffectivePolicy
CredentialBinding       resolve(capability, tenant, user, mode); begin_user_auth(...); set_admin_key(...)
PolicyAdmin             set_availability/config/approval; grant(scope, capability); create_user(kind); manage members
```

---

## 12. Worked example — Acme

Tenant `acme` (= its default project). Admin **`director@`**. Members **Bob**,
**Carol**. Shared account **`engineering@`** (Member; operated by whoever
Google Workspace lets sign in as it).

`director@` (Admin) sets:
1. `web_search` Available **for Bob** (per-user grant) → Bob has it, Carol doesn't.
2. `mcp.slack` config `{workspace: acme}`, identity **admin-keyed**, and provides
   the key → all who have it can use Slack; nobody sets a key themselves.
3. `gmail` identity **user-keyed** → each person connects their own Gmail via the
   auth gate; `engineering@`'s operator connects the team mailbox.
4. `shell` left `Hidden`.

Nothing here is Acme-specific; every line is a delta.

---

## 13. Future seams (designed-for, not built)

- **Multiple projects** — insert the project layer in the cascade; a User acts in
  a project; per-project policy baselines. The model already has `Project`.
- **Bot / room addressability** — a User can be *addressed by others* in a room.
  Whoever integrates a channel (Slack, …) maps "room → who may address this user"
  and routes messages; operational config (keys/approvals) never surfaces to that
  group. No Slack code in v1.
- **MCP / extensions** as capability kinds.
- **Time-boxed grants** (deltas with expiry → Reborn capability leases).

---

## 14. Open / deferred (non-blocking)

- Tenant **Owner** holder (company top person vs NEAR AI host).
- Config-merge semantics (deep-merge vs replace).
- `Deny` as a hard floor vs overridable.

---

## 15. Continuation plan — child issues under #4628

One Reborn-scoped issue per gap (one line each):

- **Tenant publish surface** — Reborn product/API + WebUI for an Admin to publish, disable, or revoke a built-in or custom tool/skill tenant-wide, writing through #4544's scoped-lifecycle store (no FS scan roots, no route-level tool mutation).
- **Precedence resolver on #4544** — a `PolicyResolver` over #4544's effective package-set that applies `capability default → tenant → (project) → user` precedence plus the four dimensions, consulted once at `ToolDispatcher::dispatch`.
- **Propagation fix (#3490)** — push admin/tenant capability changes to already-running user agents without restart, closing #3490 and the operator-only-tools WebUI bug #5242.
- **Credential-account binding** — bind admin-owned tool config to a user-owned credential account (user-keyed) vs an admin-set shared key (admin-keyed), reusing #4354's account staging + MRU selection, after resolving the shared-account-vs-shared-config question (§1.5).
- **Benchmark (benchmarks#59)** — add the vault service to the benchmark interceptor so non-LLM secret state (OAuth/UI/CLI mutations, between-turn events) is testable for the per-user auth completion flows.

---

## 16. Verified implementation seams (live code, checked 2026-06-25)

A workflow mapped + adversarially verified the live tree (9/11 claims confirmed;
the 2 corrections are folded in below).

- **Dispatch seam (the one enforcement point).** Availability is enforced today
  as a coarse allow/deny list via `CapabilitySurfaceProfileResolver::resolve(&LoopRunContext)`
  (`crates/ironclaw_loop_support/src/capability_allow_set.rs:51`), called once per
  turn at `crates/ironclaw_reborn/src/loop_driver_host.rs:1367`, wrapped by
  `CapabilitySurfaceProfileFilter`; returns `CapabilityAllowSet` (`All |
  Allowlist(BTreeSet<CapabilityId>)`). **Four impls exist** — `AllowAll`
  (local-dev), `Empty` (production, fail-closed), `Static`, and
  `SubagentCapabilitySurfaceResolver` (our resolver must coexist with the subagent
  one). None read tenant/user yet. **This is where `PolicyResolver` plugs in.**
- **Run identity is reachable here.** `LoopRunContext` exposes
  `scope.tenant_id: TenantId`, `scope.thread_owner`, and `actor:
  Option<TurnActor>{user_id}` → enough to build a `(tenant, user)` subject.
- **Approval — most mature.** `ironclaw_approvals`: `PersistentApprovalPolicyStore`
  (`policy.rs:180`), `CapabilityPermissionState` (`AlwaysAllow | AskEachTime |
  Disabled`), `AutoApproveSetting`. *Correction:* the WebUI surfaces it **not**
  via a dedicated tools API but through `list_operator_config` /
  `set_operator_config_key` (`reborn_services.rs:2722/2775`) with config keys
  `tool.<capability_id>` and `agent.auto_approve_tools`. The publish surface
  should mirror this operator-config pattern.
- **Identity — wired.** `ironclaw_auth::CredentialOwnership` (`credential.rs:29`):
  `UserReusable | SharedAdminManaged | ExtensionOwned | System`. user-keyed =
  `UserReusable` via `RuntimeCredentialAccountResolver` + MRU (#4354, landed);
  admin-keyed = `SharedAdminManaged` (exists, but no policy chooses the mode yet).
- **Per-capability defaults — absent.** `CapabilityDescriptor`
  (`crates/ironclaw_host_api/src/capability.rs:65`) has
  `default_permission/effects/trust_ceiling/runtime_credentials` but **no
  `default_policy`** — §7 must add the field.
- **Roles — where admin actually lives (corrects Appendix A).** The admin gate is
  **`src/ownership::UserRole` (`Owner/Admin/Regular`, `is_admin()` at `mod.rs:67`)
  + `AdminScope` (`src/tenant.rs`)** — *not* `ProjectRole`. `ProjectRole` is the
  separate `ironclaw_projects` membership model. Admin-gating reuses
  `src/ownership`; reconcile before relying on the "recast ProjectRole" line.
- **#4544 (confirmed open, not in tree).** `ScopedLifecycleInstallationStore`
  (upsert/get/delete/list/`list_effective`), `ScopedLifecycleOwnership`
  (`AdminShared{tenant} | UserPrivate{tenant,user}`), `ScopedLifecycleSubject`,
  `resolve_effective_scoped_lifecycle_installations()` (precedence: user-private >
  admin-shared, tie-break `updated_at` then id). **Installations only — no policy
  deltas, no four-dimension resolver.**

### Build order

1. **(now — #4544-independent) Policy types + manifest default.** Define
   `EffectivePolicy`, the four-dimension types, and the `PolicyResolver` port (new
   `ironclaw_capability_policy` crate or in `ironclaw_loop_support`), and add
   `default_policy` to `CapabilityDescriptor`. Uses only existing types; mergeable
   without #4544.
2. **(on #4544 merge) First slice — `ScopedLifecyclePolicyCapabilitySurfaceResolver`.**
   A `CapabilitySurfaceProfileResolver` that builds `ScopedLifecycleSubject` from
   `LoopRunContext`, calls `list_effective`, maps `LifecyclePackageRef →
   CapabilityId`, returns the allow-set; fail-closed; replaces `Empty` in
   production. Delivers **availability** end-to-end. Tests modeled on
   `tests/runtime_policy_tool_visibility_integration.rs`.
3. **Tenant publish surface** — admin-gated `RebornServicesApi` methods →
   `ScopedLifecycleInstallationStore` (AdminShared), mirroring the operator-config
   WebUI pattern.
4. **Full precedence resolver** — layer config/identity/approval onto the
   availability resolver.
5. **Credential-account binding** — after the §1.5 shared-account-vs-shared-config
   decision.
6. **Benchmark (#59)** — in the benchmarks repo.

---

## Appendix A — Reborn mapping (build target; composes with §1.5)

| Concept | Status — reuse / extend |
|---|---|
| Tenant / User / roles / per-user secrets | **exist (#1626)** — `TenantId`/`UserId`, RBAC, admin secrets, multi-tenant isolation. Add `kind(person\|shared)`. **Admin gate today = `src/ownership::UserRole` (Owner/Admin/Regular) + `AdminScope`** — reuse it; `ProjectRole(Owner/Editor/Viewer)` is the separate `ironclaw_projects` model (see §16) |
| Project + default project | `ProjectRecord` exists; **add** `is_default` + auto-create on tenant init |
| Memory (per-user) | exists; **compose with #5205** (memory as a userland extension) |
| Identity (user-keyed / admin-keyed) | **exists (#3289 / #4354)** — product-auth, per-user OAuth, account-scoped staging + MRU selection. **Wire to it**; don't add new keying |
| Approval | persistent approval-policy port + `AlwaysAllow`, and **#5195** (always-allow persisted as tool settings) — wire the `approval` field |
| Availability + ownership + effective resolution | **in-flight (#4544)** scoped-lifecycle ownership + effective package-set resolution, and **#5256** user-scoped tool settings — **extend these** |
| Enforcement | `ToolDispatcher::dispatch` — consult the resolver once per call |
| Shared-account auth | SSO via IdP (Google Workspace); no IronClaw operator list (reconcile vs #4354 — §1.5) |
| **Extend / add (the genuinely new work)** | the **config / identity / approval** dimensions layered on #4544's package-set; the **tenant publish surface + precedence**; **`PolicyAdmin` + WebUI**; per-capability **`default_policy`** in manifests; lower toward declarative **#4120 / #3036**. **Not** a parallel policy engine. |

## Appendix B — goclaw (reference only — https://github.com/nextlevelbuilder/goclaw)

Nothing built here. Go multi-tenant agent gateway; **PostgreSQL 18 + pgvector**
(server) and **SQLite via `modernc.org/sqlite`** (lite). It proves the shape:

| goclaw | our equivalent |
|---|---|
| `TenantData` + `TenantUserData(role)` | roles on the default project |
| `builtin_tool_tenant_configs(tool, tenant, enabled, settings)` | `CapabilityPolicyDelta` @ tenant |
| `skill_tenant_configs` + `skill_*_grants` | availability deltas |
| `mcp_oauth_tokens(server, tenant, user)` — null user = shared | identity: user-keyed vs admin-keyed |
| `requireTenantAdmin` / `requireMasterScope` | Admin / Owner roles |

goclaw leaves projects reserved-but-unbuilt and has only a partial approval
dimension — the inverse of Reborn's gaps.
