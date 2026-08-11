# Telegram Linked Device — Definition of Done

Revision 6. Every box names something you run or read. Where a property cannot
be tested (a leak's absence, a code shape), the box says **assert in review** and
names the file — that is honest, not a loophole.

> **Status, 2026-08-10 (implementation pass): 51 of 135 ticked.** A user can
> link an account and the agent can act as it — the handshake runs through
> production wiring end to end, proven by
> `tests/integration/group_device_link/scenario_handshake_mints_and_serves.rs`.
>
> **What the unticked boxes mostly are, so the ratio is not read as "half
> built":** the great majority are PR 6 (real MTProto transport), PR 7 (the
> fifteen op implementations and their conformance sweep), and PR 8
> (hardening/load/live-smoke) — the vendor-facing work. **Nothing in this
> feature has ever spoken MTProto**; every test drives a scripted adapter, so
> QR acceptance, DC migration, 2FA, and flood-wait remain unexercised.
> A box is ticked here only where code AND a test exist; where a box says
> "assert in review", it stays unticked until a reviewer has actually
> asserted it.
>
> What the implementation pass changed about the design itself is
> [PROPOSAL §14.5](PROPOSAL.md#145-implementation-pass--what-the-build-changed-about-the-design-2026-08-10) —
> including a **fourth** device-link route (`poll`), which several boxes below
> still describe as three.

---

## PR 1 — Contracts, schema amendment, auth custody

- [x] `DeviceLinkAdapter` carries `mode` and typed `DeviceLinkInput` (phone path
      is expressible through the contract, not only in the UI).
- [x] `VendorAuthRecipe::DeviceLink` — all arms updated; a test proves two
      device-link recipes for one vendor are **compatible** (the
      `_ => false` arm fails at activation, not compile).
- [x] A test proves a device-link recipe is never selected by the keepalive
      sweep.
- [x] `RuntimeCredentialAccountSetup::DeviceLink` + wire round-trip + a test
      that an unknown future kind folds to `Retired`.
- [ ] Nothing emits the new setup variant yet.
- [x] `LifecycleExtensionCredentialSetup` variant added; the
      `ironclaw_extension_manager` exhaustive match compiles.
- [x] `ToolPorts` is **unchanged** — the custody handle arrives via the bind-time
      factory, so none of its six construction sites break.
- [x] Version-plural `StandardOpContract`, `.v2` arms in
      `resolve_standard_schema_ref` and `CapabilityProfileSchemaRef`, and the
      validator decision all land.
- [x] **No new crate** (assert in review: the diff adds no directory under
      `crates/`; custody lands in `ironclaw_auth`).
- [x] Tests prove `sent_unverified` validates against `.v2`, and that neither
      branch present is still a violation (fail-closed preserved).
- [x] `LinkedSessionPort` declared in `ironclaw_extension_contracts`; dated
      clarification added to `crates/contracts/AGENTS.md`.
- [x] Auth owns conflict **detection**: a concurrent write is rejected with the
      current version, never last-writer-wins. (The semantic merge and its tests
      belong to the package — auth cannot parse the blob.)
- [x] The size ceiling rejects an oversized blob; `link_revision` gates
      reconnect **and evicts any live pooled client**.
- [x] `link_revision` carries `#[serde(default)]`; a `CredentialAccount`
      persisted before the change rehydrates.
- [x] `ironclaw_assistant`'s credential-setup and auth-challenge matches compile,
      and the in-channel device-link prompt copy is written.
- [x] Contracts size ceiling raised; contract location scan green.
- [x] `LinkedAccountRef` and `link_revision` are reachable from
      `ironclaw_extension_contracts` — the package can construct its own pool key
      without naming `ironclaw_auth` (its BoundaryRule forbids it).
- [x] The factory takes a **host-issued grant**, not a bare `UserId` — or the
      containment claims in §3.3/§4.4/ADR/PR-3 are downgraded to
      adapter-discipline in the same PR.


## PR 2 — Auth + ADR

- [x] `ADR-device-link-auth-hook.md` merged; the test-retirement rationale in
      PR 3 cites it.
- [x] Every new `src/**/*.rs` has one sub-owner row;
      `cargo test -p ironclaw_auth --test module_charter` green.
- [x] `AuthFlowRecord.step` is `Option` + `#[serde(default)]`; a record persisted
      before the change rehydrates.
- [x] Concurrent polls at one revision advance exactly once; the loser gets `Ok`
      with the advanced record.
- [x] A stale **step** re-mints; an expired **flow** terminalizes; TTL extension
      is capped.
- [x] `AwaitingVendor` projects to `Authenticating` (explicit arm).
- [x] A driver that loses the revision CAS **does not re-invoke the adapter**
      (test with a counting fake).
- [x] Cross-user flow access denied, and not an existence oracle.

## PR 3 — Extension host

- [x] Declared device-link binds; undeclared is refused (`check_binding`).
- [x] `auth_never_binds_is_not_a_binding_field` retired **with a written
      rationale in the same commit**, referencing the PR 2 ADR.
- [x] `DeviceLinkDriver` implemented; the adapter receives a session port it cannot
      re-address to another user or extension (test the scoping).
- [x] Poll rate limiting enforced host-side, not trusted to the adapter.
- [x] `begin` and identifier submission are rate-limited per user and per
      deployment, with a distinct-number cap and a circuit breaker; `Code` and
      `Password` attempts are bounded per flow.

## PR 4 — Frontend

- [x] One QR/countdown implementation; the pairing panel is recomposed over it.
- [x] Every step renders: QR, awaiting, phone number, code, password, success,
      failure.
- [x] The QR ⇄ phone switch works and restarts the flow in the other mode.
- [x] `Failed { restartable: true }` offers "start again" rather than dead-ending.
- [x] Stale-revision responses ignored; polling stops on terminal states.
- [x] Frontend vitest green; descriptors contract green if routes changed.

## PR 5 — Package scaffold (fake vendor)

- [x] `bind` performs no I/O (assert in review: `src/linked/mod.rs`).
- [ ] Pool lock is never held across an await (assert in review: `pool.rs`).
- [ ] Pool eviction → next call reconnects from the blob.
- [ ] Admin-config edit → reactivation → next call still works.
- [x] Both adapters share one `SessionPool` instance (test: revoke evicts what
      the tool adapter would have used).
- [ ] `IronclawSession` round-trips; write-through is debounced, not
      per-mutation.
- [ ] The **semantic merge** on CAS conflict is tested here (package-side):
      peer-cache union bounded, max update cursor, no DC auth key ever removed
      or replaced.
- [x] Every bound in PROPOSAL §7.2 exists as a named constant with a test.
- [ ] Two users' sessions and blobs are isolated — driven **through
      `CapabilityHost` → `ToolAdapter::invoke`**, not at the auth selector seam
      (no dispatch-tier cross-user test exists in the repo today).
- [ ] A second actor's call resolves the **second actor's** credential account
      (owner == actor upstream; this is the credential dimension, not a
      thread-owner check).
- [x] `ownership = ExtensionOwned`, `granted_extensions = []`, and the account is
      ineligible for the host-managed credential fallback — each tested.
- [ ] Integration test links, calls one fake-backed tool, and unlinks through the
      real harness, asserting at seams.
- [ ] Manifest binds exactly the one implemented op.
- [x] `api_id` is an `[admin_configuration]` field and **`api_hash` is
      `secret = true`**.
- [ ] A poll for an absent `flow_id` returns `Failed { restartable: true }`
      host-side (process restart or TTL reap).
- [x] `PENDING_LINK_TTL ≥ flow TTL ≥ step TTL` asserted in one test.
- [x] The package's `Cargo.toml` does not depend on `ironclaw_extension_host`.
- [ ] `telegram_factory_binds_a_channel_and_no_tools` rewritten.
- [x] No `struct InMemory*Store` in package `src/`
      (`telegram_tests_use_the_real_filesystem_state`).
- [x] 999-line budget green **including inline test modules**.
- [x] Composition change is wiring only; `check-composition-budget.sh` green.

## PR 6 — Transport and real login

- [ ] All socket construction confined to `src/linked/transport.rs` (assert in
      review).
- [ ] DC validation in `IronclawSession` rejects private, loopback, and
      link-local addresses (unit test with a poisoned `set_dc_option`).
- [ ] `tokio::spawn(runner.run())` present for every pool.
- [ ] **All** clients drop `updates` (assert in review — a leak's absence is a
      code shape, not a test).
- [ ] Client retry policy is `NoRetries`; write retries are decided in our
      wrapper, never by the client.
- [ ] QR completes by polling re-export: same session, `min(3s, expires-serverNow)`,
      repaint only on byte change, server-time corrected, 1→60s backoff.
- [ ] `MigrateTo` handled via `invoke_in_dc`; new home DC persisted.
- [ ] Self-peer cached after raw-TL success.
- [ ] 2FA: `SESSION_PASSWORD_NEEDED` → fresh `GetPassword` → `check_password`.
- [ ] Per-link mutex serializes vendor ops; a poll during password submission
      cannot interleave (test with an instrumented fake clock).
- [ ] **Post-acceptance abort logs out**: kill the flow after acceptance and
      confirm no device remains on the test account.
- [ ] Completion order is store → mint → report (test by failing the store).
- [ ] `PendingLinks` bounded and TTL-reaped; a reaped **accepted** link logs out
      via `cancel()` and is marked `logout_unverified`.
- [ ] The durable `accepted_at` marker is written before acceptance is acted on;
      startup surfaces every non-terminal device-link flow to its owner.
- [ ] The generation fence is checked before **each** vendor RPC; unlink returns
      immediately and never blocks on in-flight calls.
- [ ] The per-account lease carries a TTL and crash-release.
- [ ] `Dropped` evicts and rehydrates; **no write is retried**.
- [ ] `PooledClient::Drop` aborts its runner.
- [ ] Session survives a process restart without re-linking.
- [ ] Relinking over an orphan blob succeeds (load-then-CAS, never
      `Absent`-only).
- [ ] Reactivation quiesces the old pool and flushes before the new one
      connects (no two live pools writing one session).
- [ ] Keepalive is on: 60 s ping, 75 s disconnect grace.
- [ ] Session encoding is raw bytes, not grammers' serde hex (§7.2).
- [ ] Raw TL in one module; `cargo deny check` green.
- [ ] Carve-out documented; the **three** amended charter sentences are named by
      file and line in the PR body.

## PR 7 — Tools

- [x] Tool ids exactly `telegram.<op>`; no author-declared schema refs; every
      write declares `external_write`; bespoke tools wear no `standard:` ref.
- [ ] The effects-honesty decision (does a raw-socket tool declare `network`?)
      is recorded in the manifest with its reasoning.
- [ ] `[[tools.credentials]]` shape for a device-link session is designed, and
      an un-linked user demonstrably sees no linked-account tools.
- [ ] `messaging_conformance` passes for every bound op, including the evidence
      loop.
- [x] `id == 0` returns **`Completed`** carrying `sent_unverified: true` — never
      `Failed`, never a fabricated `message_ref`.
- [x] `Dropped`/`Io` on a write returns **`messaging.vendor_error`** (outcome
      genuinely unknown) — *not* `sent_unverified`, which asserts delivery.
- [ ] No write op is auto-replayed by any path (assert in review: retry wrapper,
      `Dropped` handling, driver).
- [ ] A media-only message round-trips with `text: ""` plus a vendor content
      marker; pure service messages are filtered from history.
- [ ] `is_self` from `outgoing()`; never fabricated `true`.
- [ ] Conversation refs rehydrate without a cache hit; a migrated basic group
      maps to `messaging.unknown_conversation`.
- [ ] `search_messages` binds **global search only**; per-chat and date-bounded
      search ship as bespoke tools carrying their own schema refs.
- [ ] `MAX_PAGES_PER_CALL`, `MAX_RESULT_BYTES`, the per-message text cap and the
      per-page item cap are enforced (§6.4 bounds, folded into §7.2).
- [ ] Read ops declare `automation = "forbidden"` in the origin gate matrix.
- [ ] Every row of the §6.6 error table has a test (acme-style sweep).
- [ ] `conversation_info.kind` matches §6.3; `counterpart` present whenever
      `kind == "dm"`, on both ops.
- [ ] A broadcast-channel post and an anonymous-admin post round-trip through
      history (the `author.user_ref` fallback works).
- [ ] `delete_message` with count 0 returns `messaging.unknown_message`.
- [ ] `remove_reaction` does read-modify-write, or omits `emoji` after clearing.
- [ ] An undecodable cursor returns `messaging.unsupported_content`, never a
      silent restart at page one.
- [ ] `messaging.rate_limited` carries retry-after; our retry wrapper's sleep
      budget is below `TOOL_CALL_TIMEOUT` (assert both constants in one test).
- [ ] Credential failures surface as `AuthRequired`, not a messaging code.
- [ ] `resolve_user` resolves a display name by dialog-title match, not only
      @usernames.
- [ ] `connection_success_message` no longer claims the extension cannot read or
      send; every other `[channel*]` field is byte-identical.
- [ ] Writes are `default_permission = "ask"`; `product`/`automation` forbidden.
- [ ] Descriptions state the tool acts *as the user* and that final answers are
      host-delivered.
- [ ] Group reads and writes covered by tests, not just DMs (decision 5: both).

## PR 8 — Hardening

- [ ] Load test at **200 concurrent sessions**; task and socket counts recorded
      in the PR body.
- [ ] A failed session does not reconnect until `link_revision` changes.
- [ ] Telegram-side revocation parks the run; re-linking resumes it; the dead
      blob is deleted, not retained.
- [ ] A banned/deactivated account reaches a terminal `Unavailable`, not an
      endless re-link prompt.
- [ ] Unlink with a failed vendor logout reports **explicitly unverified**.
- [ ] Verified (not assumed): an un-linked user sees no linked-account tools.
- [ ] Live smoke script exists and has been run against a real account.

---

## Cross-cutting: security

- [x] **Supply-chain controls shipped with the dependency** (PROPOSAL §11.1,
      ADR "The larger trade this sits inside"), not deferred to hardening:
      every `grammers-*` edge is `=0.10.0` with `default-features = false` and
      an explicit allowlist; versions, `.crate` checksums, registry source, and
      **resolved** feature sets (all eight members, including the five
      transitive ones no manifest can pin) are frozen by
      `crates/app/ironclaw_architecture_tests/tests/reborn_linked_device_supply_chain_pin.rs`
      under both the default and `--all-features` resolution; the socks5
      `proxy` feature is off and asserted off three independent ways; and
      `.github/dependabot.yml` ignores `grammers-*` so a bump can only arrive as
      a deliberate human edit. Every gate was sabotage-tested (proxy on, caret
      range, tampered checksum, dropped pin row, removed ignore) and observed to
      fail. **Residual, deliberately not claimed as closed:** the repository has
      no `CODEOWNERS`, so §11.1's *named* human reviewer cannot be routed — the
      pin is a forcing function proving a human touched the bump, not evidence
      anyone diffed upstream. Stated in the gate's module docs and asserted, so
      the day a `CODEOWNERS` appears the gate demands the residual be rewritten.
- [ ] Dependency review **deliberately deferred** per PROPOSAL §11.1; the
      revisit trigger — **N = 2 linked accounts**, not GA — is tracked somewhere
      that will actually be seen: an issue, not just this document.
- [ ] Session bytes, phone numbers, login codes, **QR login payloads**, and 2FA
      passwords never appear in logs, `Debug`, errors, or the **durable flow
      record** (which stores only `DeviceLinkPayloadHash`; the QR payload is
      projected from memory — decision 1) (assert in review: `linked/login.rs`, `linked/pool.rs`,
      `linked/session_store.rs`, `ironclaw_auth/src/product_prompt.rs`, and every
      `Debug` impl on a type carrying them).
- [ ] Secrets zeroize on drop (assert in review: the `SessionBytes` and
      `DeviceLinkInput` wrappers in `device_link.rs`).
- [ ] No `info!`/`warn!` from background paths:
      `rg 'info!|warn!' crates/extensions/packages/telegram/src/linked/` is empty.
- [ ] Blob encrypted at rest via the existing secrets path; unlink purges the
      secret through the existing credential-cleanup path (which already handles
      quarantine on a failed vendor logout).
- [ ] Decided and recorded: whether `link_revision` is added to the encryption
      AAD (rejects a replayed stale ciphertext — cheap if the AAD builder takes
      it, not worth new machinery otherwise).
- [x] No `.unwrap()` / `.expect()` in new production code.
- [ ] Changed files scanned for byte slicing, hardcoded temp paths, lost error
      causes.

## Cross-cutting: release

- [ ] §11 re-read at release; "still holds" (or the re-review) recorded in the
      release PR body.
- [ ] Product copy states: reads are live and not stored; what the agent reads
      enters the conversation transcript; **the session holds a peer cache**
      (a partial contact/chat graph), which is not message content but is not
      nothing.
- [ ] Unlink UI tells users they can also revoke the device in Telegram, and to
      revoke any unrecognised IronClaw device (the residual crash window).
- [ ] `recover@telegram.org` pre-notified before the first production login.
- [ ] Connect screen states IronClaw connects as a third-party client and
      Telegram may restrict accounts.
- [ ] `cargo fmt`; `cargo clippy --all --benches --tests --examples --all-features -- -D warnings`.
- [ ] `cargo test -p ironclaw_architecture_tests` green before **and** after.
- [ ] `docs/internal/superpowers/specs/2026-07-16-telegram-extension-design.md`
      non-goals updated (they still say "no MTProto/link-device").
- [ ] The linked-accounts design doc points here for Telegram.
- [ ] User documentation covers linking, unlinking, and the limits, and matches
      §3.1's product-copy caveats verbatim on what is and is not stored.
