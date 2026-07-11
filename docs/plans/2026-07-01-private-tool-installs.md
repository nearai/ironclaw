# Per-user (private) tool installs — #5459 P1

**Branch:** `zetyquickly/issue-5459-private-installs` (based on
`zetyquickly/issue-5459` — needs the credential resolver + the three
test-tool fixtures to exercise a privately installed "network + key"
tool end-to-end).

**Goal:** the tenant (operator) install makes a tool available to
everyone; anyone else who installs a catalog tool gets it for
themselves — and **any number of users can independently install the
same tool**. A member's install is invisible and non-dispatchable to
everyone who hasn't installed it. Sibling PRs: #5499 (env-seeded
tenant-shared credentials), #5513 (admin UI for the same credential
rows).

## 2026-07-08 pivot: membership, not slots

The first iteration of this plan (locked 2026-07-01, reviewed in
#5525) modeled ownership as **one install slot per extension id per
tenant** with admin-wins eviction. Manual testing killed it: with a
tool privately installed by one member, every other user's install
attempt failed with a masked "extension id unavailable" — surfaced in
the WebUI as a bare "Validation" banner on a tool that *looked*
installable (foreign private installs are deliberately invisible).
Decision (Emil, 2026-07-08): that complexity buys nothing the product
wants. The model is now **membership**:

- One installation **row** per id (unchanged — the store keys by
  `installation_id == extension_id`), but the owner is either `Tenant`
  or a **set of member user ids**.
- A member installing a tool that others already hold **joins the
  member set**; the bundle is registered/materialized only once, by
  the first installer.
- **Admin-wins eviction survives as a semantic, not as machinery**
  (Emil, 2026-07-08): an operator install still evicts every member's
  private installation — but with one row per id, that eviction IS the
  single write that replaces the member set with `Tenant`. The
  snapshot/restore/compensation machinery of the slot iteration is
  deleted, and with it the "unavailable" denial: a member's install of
  a catalog tool now only fails for real reasons (already theirs, or
  already tenant-shared).

serrrfirat's #5525 review findings were fixes *within* the slot
iteration, not a request for slots; the ones about masking, caller
derivation, credential-preflight ordering, fail-closed owner
visibility, and policy extraction carry over to the membership model
unchanged. The eviction-compensation finding is moot (no eviction).

## Verified mechanism (recon receipts)

- **Identity**: `ExtensionId` = manifest.toml `id`
  (`available_extensions.rs`); `ExtensionInstallationId::new(extension_id)`
  — the installation id IS the extension id, so there is exactly one
  installation **row** per id per tenant. That row now carries the
  member set; it does not limit how many users hold the tool.
- **Registry**: one global process-wide snapshot; `publish` upserts by
  `ExtensionId`; duplicate `CapabilityId` insert fails closed
  (`ironclaw_extensions/src/registry.rs`). Capability ids are
  validated `<extension_id>.<name>`-prefixed. ⇒ the bundle is
  published **once**; per-user visibility is enforced at grant
  minting, not by duplicate registration. This is exactly why
  membership is cheap where same-id *separate installs* were not.
- **Visibility/dispatch = per-caller grant minting**:
  `local_dev_visible_capability_request` mints grants via
  `extension_surface.grants(&extension_id)` filtered by
  `owner.visible_to(caller)`
  (`runtime/local_dev/extension_surface.rs`). Grant absence =
  invisible in the surface AND denied at dispatch — fail closed for
  free. Membership only changes what `visible_to` checks (set
  membership instead of single-owner equality).
- **Persistence**: installations live in ONE snapshot file
  (`.installations/state.json` under the tenant's extension root,
  `extension_installation_store.rs`). Owner stays a field on the
  record, not a new store.
- **Reserved first-party ids** (`github`, `notion`, `web-access`,
  `slack`, nearai, gsuite) are the SYSTEM tier of the id namespace;
  import-path enforcement landed with the #5499 hardening commit.

## Locked decisions

1. **Typed owner, no name prefixes.**
   `InstallationOwner { Tenant, Users { user_ids: BTreeSet<UserId> } }`
   on `ExtensionInstallation`, `#[serde(default)]` = `Tenant` so
   pre-#5459 `state.json` rows deserialize unchanged (no migration).
   Rows written by the slot iteration (`{kind: "user", user_id}`)
   deserialize as a singleton member set — dev homes created on this
   branch keep loading. New writes serialize the set shape.
2. **Install rules — join, don't gate** (Emil, 2026-07-08):

   | Row state | Member installs same id | Operator installs same id |
   |---|---|---|
   | no row | ✓ row created, `Users{them}` | ✓ `Tenant` |
   | `Tenant` | ✗ "already installed" (it is already available to them) | ✗ "already installed" |
   | `Users(S)`, caller ∈ S | ✗ "already installed" | — |
   | `Users(S)`, caller ∉ S | ✓ **joins**: row becomes `Users(S ∪ {caller})` | ✓ **evicts every member's private installation**; the tenant row takes the id (members regain the tool through the shared install) |

   Join and evict-to-tenant are row-only updates: the package is
   already registered, materialized, and (if enabled) published, so
   there is nothing to compensate — eviction is one write replacing
   the member set with `Tenant`, not a disable/deregister/republish
   dance. Anti-squatting still holds: the operator can always take any
   id tenant-wide, and nobody loses access when they do.
3. **Remove = leave the set.** A member removing a tool they hold is
   removed from the member set (row-only update; others keep the
   tool). The **last** member leaving triggers the full teardown
   (disable, deregister, unpublish, delete rows/files) — same
   compensated path as today. `Tenant` rows stay operator-only to
   remove. Non-members get the masked "is not installed" denial.
4. **Masking stays, the install-path denial goes.** Non-members still
   cannot see, activate, remove, or probe a tool they don't hold
   (`ensure_caller_may_operate`, list filtering, credential-preflight
   ordering — all unchanged from the #5525 review fixes). But install
   by a non-member now *succeeds*, so the anti-enumeration property is
   strictly stronger: the outcome of "install X" no longer depends on
   whether someone else holds X.
5. **Credential continuity, including across eviction.** Secrets are
   keyed `(owner scope, handle)`, decoupled from installations. Each
   member's dispatch resolves caller-first → tenant-shared →
   AuthRequired (`secret_owner_scope`), so two members of the same
   tool use their own keys (or the tenant-shared one) — and after an
   operator install evicts their private installations, the resolver
   still finds each user's personal key under the same handle, so
   their key keeps winning for their calls. Extension-id-keyed
   credential grants (OAuth `granted_extensions`) keep matching
   because eviction preserves the id string. Unchanged from the slot
   iteration.
6. **UI: one list, badges.** Entries badged `shared` / `mine`
   (`install_scope`: `Tenant` → shared, member set containing the
   caller → mine). A tool held by others but not the caller appears
   simply as installable — indistinguishable from a fresh tool, by
   design.
7. **Admin signal** = `operator_webui_config` capability (env-owner
   today; role-derived admin when P0 role wiring lands — resolve in
   one place, don't re-derive per layer).

## Implementation steps (test-first per step)

1. `crates/ironclaw_extensions/src/installations.rs` —
   `InstallationOwner::Users { user_ids }`; `user(uid)` constructor
   builds a singleton set (keeps migration + fixture call sites);
   `visible_to` = tenant or membership; join/leave helpers. Tests:
   legacy no-field row → Tenant; slot-iteration `{kind:"user"}` row →
   singleton set; set round-trip; tenant rows still serialize without
   the field (rollback shape).
2. `extension_host/extension_lifecycle/install_policy.rs` — replace
   `decide_occupied_slot`/eviction-snapshot types with a pure
   `decide_install(existing_owner, claimant)` →
   `Fresh | Join | EvictToTenant | AlreadyInstalled`;
   `ensure_caller_may_operate` checks membership. Delete
   `EvictedPrivateInstall`.
3. `extension_host/extension_lifecycle.rs` — install branches: fresh
   (register + materialize + persist, existing compensation), join /
   evict-to-tenant (row-only upsert). Remove branches: leave-set
   (row-only) vs last-member full teardown. Delete
   `ensure_slot_available` / `evict_private_installation` /
   `restore_evicted_private_install` / `fail_install_restoring_evicted`.
   `import_bundle`'s `ensure_not_installed` stays (a bundle cannot be
   swapped under live installs).
4. Projections — `installation_owners()` map, settings-tools catalog
   filter, `install_scope_for_owner`, grants/provider-trust filtering:
   all reduce to `visible_to`/membership; signatures unchanged.
5. WebUI — badges unchanged; install-button shows for any catalog
   tool the caller doesn't hold.
6. Facade-level tests (`tests/private_install_tests.rs`): two members
   independently install the same tool and both dispatch it;
   non-member masked on activate/remove and list; member remove
   leaves the other member intact; last-member remove tears down;
   operator install evicts both members' private installs and both
   keep access through the shared tool (with their own credentials
   still resolving); command-path caller derivation (kept from #5525).

## Adversarial review (2026-07-01 slot iteration) — what carries over

- **[blocker] settings/tools catalog leaked private tools** — FIXED
  and carried over: `RebornOperatorToolCatalog::list_operator_tools`
  is caller-aware; the composition catalog joins
  `installation_owners()` and drops foreign-private tools, failing
  closed on unreadable owner data. Pinned by
  `operator_tool_catalog_hides_foreign_private_tools` (webui.rs).
- **[minor] activate TOCTOU across hosted-MCP discovery** — FIXED and
  carried over: `activate_inner` re-runs `ensure_caller_may_operate`
  after re-acquiring the lock.
- **[should-fix] eviction bricked the id on partial failure** —
  obsolete: eviction no longer exists; join/convert are single row
  writes.

## Non-goals / phase 2

- **Per-user bundle versions** (alice and bob holding *different*
  builds of the same id): the catalog holds one bundle per id;
  membership shares it. Owner-scoped catalog/registry keys stay out
  of scope.
- **Per-user suppression** of a tenant-wide tool ("hide this shared
  tool for me").
- **User-uploaded private tools ("bring your own").** `import` (zip →
  catalog) stays admin-only, and the `AvailableExtensionCatalog` is a
  single tenant-global catalog, so a member can only *install* a tool
  an admin already imported (or a bundled/first-party one) — they
  cannot introduce a brand-new WASM tool that only they see. True BYO
  private tools would require owner-scoping the import + catalog-browse
  layer, plus keeping arbitrary-WASM upload behind a deliberate
  capability gate. Deferred.
- **Skills** — same shared-vs-private axis, different mechanics; owned
  by the P4 plan (`shared-` prefix IS right there, per-user trees).

## Accepted tradeoffs (operator awareness / release notes)

- **Rollback is a full outage once any member install exists.** Tenant
  rows serialize byte-identical to pre-#5459 (owner field skipped when
  `Tenant`), so a rollback loads a state.json holding only shared
  tools. But a member row carries `owner: {kind: users, …}`, which an
  older binary's `deny_unknown_fields` wire struct rejects — and the
  store fails the WHOLE state.json load, so `serve` refuses to start
  until the file is hand-edited. One member clicking "install" turns a
  rollback into an outage. Flag in release notes; the real fix is a
  forward-compatible older binary (out of scope here).
- **SSO users (incl. real admins) always install for themselves on
  this branch.** The tenant-operator identity is the env-bearer
  (`IRONCLAW_REBORN_WEBUI_USER_ID`); SSO logins mint UUID user ids
  that never equal it, so every SSO user installs as a member —
  cannot install shared tools tenant-wide and cannot remove a
  tenant-shared tool (operator-only check). Under membership this is
  no longer a functional wall (they can always get the tool for
  themselves); the remaining gap is administrative and closes when P0
  role wiring lands (decision 7).

## Known upstream issue (not this plan's problem)

`projection::tests::live_progress_stream::skill_learned_bubble_delivers_when_sse_resumes_from_advanced_durable_cursor`
fails deterministically at the branch's merge-base (inherited from
main). Do not chase it here.
