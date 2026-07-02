# Per-user (private) tool installs — #5459 P1

**Branch:** `zetyquickly/issue-5459-private-installs` (based on
`zetyquickly/issue-5459` — needs the credential resolver + the three
test-tool fixtures to exercise a privately installed "network + key"
tool end-to-end).

**Goal:** a regular user can install a WASM tool that only they see and
can dispatch; an admin install stays tenant-wide (today's behavior,
now explicit). Sibling PRs: #5499 (env-seeded tenant-shared
credentials), #5513 (admin UI for the same credential rows).

## Verified mechanism (recon receipts)

- **Identity**: `ExtensionId` = manifest.toml `id`
  (`available_extensions.rs:1579`); `ExtensionInstallationId::new(extension_id)`
  — the installation id IS the extension id (`extension_lifecycle.rs:1133`),
  so there is exactly one install slot per id per tenant.
  `ensure_not_installed` (`extension_lifecycle.rs:~882`) already rejects
  duplicates on both axes.
- **Registry**: one global process-wide snapshot; `publish` upserts by
  `ExtensionId`; duplicate `CapabilityId` insert fails closed
  (`ironclaw_extensions/src/registry.rs:73-79`). Capability ids are
  validated `<extension_id>.<name>`-prefixed (`v2.rs:884-896`), so
  renaming an id at install time would mean rewriting the bundle.
  ⇒ same-id coexistence (shared + private) is NOT cheap; do slots, not
  shadowing.
- **Visibility/dispatch today = blanket per-request grant minting**, not
  authorizer policy: `local_dev_visible_capability_request`
  (`runtime/local_dev.rs:878-942`) mints a grant for EVERY active
  capability to `Principal::Extension(loop_driver)` via
  `extension_surface.grants(&extension_id)`
  (`runtime/local_dev/extension_surface.rs:82-93`). The caller's
  `user_id` is computed in the same function (`:894-899`) and unused for
  filtering. ⇒ one choke point: filter grant minting by installation
  owner. Grant absence = invisible in the surface AND denied at
  dispatch — fail closed for free.
- **Persistence**: installations live in ONE snapshot file
  `/tenants/<t>/system/extensions/.installations/state.json`
  (`extension_installation_store.rs:12`). Owner becomes a field on the
  record, not a new store.
- **Reserved first-party ids** (`github`, `notion`, `web-access`,
  `slack`, nearai, gsuite) are the SYSTEM tier of the id namespace;
  import-path enforcement landed with the #5499 hardening commit.

## Locked decisions

1. **Typed owner, no name prefixes.**
   `InstallationOwner { Tenant, User(UserId) }` on `ExtensionInstallation`,
   `#[serde(default)]` = `Tenant` so existing `state.json` rows
   deserialize unchanged (no migration). The skills-style `shared-`
   prefix does NOT transfer to tools: capability-id prefix validation
   forces bundle rewrites, tool names are model-facing API (prefix churn
   breaks routines on scope change), and a prefix wouldn't solve
   user↔user collisions anyway (registry is global; skills never had
   that problem because skill trees are per-user).
2. **Slot rules — admin-wins eviction** (Emil, 2026-07-01):

   | Slot state | User installs same id | Admin installs same id |
   |---|---|---|
   | free | ✓ owned by `User(them)` | ✓ owned by `Tenant` |
   | `Tenant`-owned | ✗ "already available as a shared tool" | ✗ already installed |
   | `User(x)`-owned | ✗ generic "extension id unavailable" (don't leak the owner) | ✓ **evicts the private install**; tenant record takes the slot |

   Rationale: anti-squatting — a user can never reserve an id against
   the org (imagine a private `gmail` blocking the admin) — and
   self-healing escalation: two users want the same tool privately →
   admin installs it shared → everyone gets it.
3. **Supersede, don't destroy.** Eviction rewrites installation state
   only (natural: the store keys by installation_id == extension_id, so
   the tenant record overwrites the slot). The evicted user's wasm
   artifacts and private credentials are NOT deleted. Credential
   cleanup is a separate explicit route
   (`product_auth_serve/lifecycle.rs`) — never wired into
   install/remove/evict.
4. **Credential continuity across eviction.** Secrets are keyed
   `(owner scope, handle)`, decoupled from installations. After
   eviction the resolver (`secret_owner_scope`, caller-first →
   tenant-shared → AuthRequired) still finds the user's personal key
   under the same handle — their key keeps winning for their calls.
   Extension-id-keyed credential grants (OAuth `granted_extensions`)
   keep matching because admin-wins preserves the id string.
5. **UI: one list, badges.** Entries badged `shared` / `mine`; the
   badge flip (mine → shared) is the MVP-sufficient eviction notice.
6. **Admin signal** = `operator_webui_config` capability (env-owner
   today; role-derived admin when P0 role wiring lands — resolve in one
   place, don't re-derive per layer).

## Implementation steps (test-first per step)

1. `crates/ironclaw_extensions/src/installations.rs` —
   `InstallationOwner` enum + field on `ExtensionInstallation`
   (`#[serde(default)]` = Tenant). Tests: legacy JSON row (no field)
   deserializes as Tenant; round-trip with `User(uid)`.
2. `crates/ironclaw_reborn_composition/src/extension_lifecycle.rs` —
   install derives owner from the caller (admin → Tenant, else
   User); slot rules incl. admin-wins eviction; `installed_summaries`
   filtered to tenant-owned + caller-owned. Tests through the lifecycle
   facade: each row of the slot table; eviction preserves the evicted
   user's secret rows.
3. `active_model_visible_capabilities()`
   (`RebornLocalExtensionManagementPort`) — carry owner per capability
   (join from the installation record) into `ActiveExtensionCapability`.
4. `runtime/local_dev/extension_surface.rs` — `grants()` /
   `provider_trust()` take the caller's `user_id` (already computed at
   the call site, `local_dev.rs:894-899`) and filter
   `owner == Tenant || owner == User(caller)`.
5. WebUI (`ironclaw_webui_v2` + `_static`) — `shared`/`mine` badge on
   extension cards; install-button behavior per slot rules; thread the
   admin flag one hop down into the lifecycle context (same shape as
   the #5513 operator endpoint).
6. Integration tests (through the caller, per
   `tests/support/reborn/CLAUDE.md`): bob cannot SEE or DISPATCH
   alice's private tool (fail-closed check on both the surface and
   dispatch paths); admin install evicts alice's private install and
   alice's personal credential still resolves; market-data fixture
   installed privately still gates on missing key and runs with the
   tenant-shared key.

## Adversarial review (2026-07-01, run wmg0t4d3w) — confirmed + resolved

- **[blocker] settings/tools catalog leaked private tools** — the
  `GET /api/webchat/v2/settings/tools` catalog
  (`ActiveRegistryOperatorToolCatalog`) read the global registry with no
  owner filter, so any member saw another user's private capability id +
  metadata (dispatch stayed denied — grants are per-caller — but the
  existence/metadata leaked). FIXED: `RebornOperatorToolCatalog::list_operator_tools`
  is now `async` + caller-aware; the composition catalog joins
  `installation_owners()` and drops foreign-private tools; `find_operator_tool`
  masks them on the get/set-key paths too. Pinned by
  `operator_tool_catalog_hides_foreign_private_tools` (webui.rs) +
  `installation_owners` assert in `active_capabilities_carry_installation_owner`.
- **[should-fix] eviction bricked the id on partial failure** — a tenant
  install that failed after eviction deregistered the private package left
  the id un-installable tenant-wide until restart. FIXED: eviction is now
  retry-safe (tolerates an already-absent lifecycle package), so the admin's
  retry heals the slot. Pinned by
  `admin_install_retry_heals_after_eviction_then_persist_failure`.
- **[minor] activate TOCTOU across hosted-MCP discovery** — ownership was
  checked only in phase 1; the slot could change hands during the discovery
  network call. FIXED: `activate_inner` re-runs `ensure_caller_may_operate`
  after re-acquiring the lock, mapping a change to
  `hosted_mcp_changed_during_discovery_error`.

Two accepted tradeoffs (documented, not code-fixed — see below).

## Non-goals / phase 2

- **Multi-user private installs of the same id** (alice AND bob both
  privately holding `market-data`): requires owner-scoped registry keys
  + caller-first-then-tenant dispatch resolution (the
  `secret_owner_scope` precedence, applied to the registry). Only if
  demanded; the admin-escalation path covers the common case.
- **Per-user suppression** of a tenant-wide tool ("hide this shared
  tool for me").
- **User-uploaded private tools ("bring your own").** `import` (zip →
  catalog) stays admin-only, and the `AvailableExtensionCatalog` is a
  single tenant-global catalog, so a member can only privately *install*
  a tool an admin already imported (or a bundled/first-party one) — they
  cannot introduce a brand-new WASM tool that only they see. True BYO
  private tools would require owner-scoping the import + catalog-browse
  layer (one level below P1's install-scope), plus keeping arbitrary-WASM
  upload behind a deliberate capability gate. Deferred.
- **Restore-on-admin-remove** (evicted private install coming back when
  the admin removes the shared tool) — nice-to-have; MVP leaves the
  user to reinstall.
- **Skills** — same shared-vs-private axis, different mechanics; owned
  by the P4 plan (`shared-` prefix IS right there, per-user trees).

## Accepted tradeoffs (operator awareness / release notes)

- **Rollback is a full outage once any private install exists.** Tenant
  rows serialize byte-identical to pre-#5459 (owner field skipped when
  `Tenant`), so a rollback loads a state.json holding only shared tools.
  But a user-private row carries `owner: {kind: user, …}`, which an older
  binary's `deny_unknown_fields` wire struct rejects — and the store fails
  the WHOLE state.json load, so `serve` refuses to start until the file is
  hand-edited. One member clicking "install" turns a rollback into an
  outage. Flag in release notes; the real fix is a forward-compatible
  older binary (out of scope here).
- **SSO users (incl. real admins) always install private on this branch.**
  The tenant-operator identity is the env-bearer `IRONCLAW_REBORN_WEBUI_USER_ID`;
  SSO logins mint UUID user ids that never equal it, so every SSO user
  derives `User(..)`: private installs only, cannot install/administer
  shared tools, and — one behavior regression — cannot remove a
  tenant-shared tool (new "only the tenant admin can remove" check) that
  a browser user could remove pre-#5459. Matches decision 6 (env-owner
  today, role-derived admin at P0); a private-install squat on a
  first-party id is un-evictable from an SSO browser until P0 lands.

## Known upstream issue (not this plan's problem)

`projection::tests::live_progress_stream::skill_learned_bubble_delivers_when_sse_resumes_from_advanced_durable_cursor`
fails deterministically at the branch's merge-base (inherited from
main). Do not chase it here.
