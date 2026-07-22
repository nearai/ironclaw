# ADR 0001 — Multiple accounts (profiles) per vendor

**Status:** Accepted 2026-07-13. Implementation is a dedicated PR **after P7**
(the first post-train feature PR); the P0–P7 train ships only the wire shape
(see "What the train does now").
**Trigger citation (required by `implementation.md` §14):** `overview.md` §7
excluded "Multiple accounts per vendor per user" with revisit trigger *"a real
work + personal account use case."* The trigger fired 2026-07-13: two Notion
accounts connected; gmail on a personal Google account while docs uses a work
Google account.

## Decision

One user may hold **multiple connected accounts per vendor**. No new entity is
introduced: the existing credential account (`credential_account_id`, per
user × vendor) *is* the profile. Three additions:

1. Account records gain a user-editable `label` (defaulted from the recipe's
   identity claims — email, workspace name) and `is_default`. Invariant:
   exactly one default per (user, vendor) whenever any account exists.
2. One new relation, the **per-extension account binding**:
   `(user, extension_id, vendor) → credential_account_id`, optional. This is
   what the motivating case needs — gmail bound to the personal Google
   account while docs binds to the work one: same vendor, different bindings
   per extension.
3. The wire models a vendor's auth as an **accounts list** (`account_id`,
   `label`, the §6.3 state, `is_default`), plus each extension surface's
   `resolved_account_id` and binding source (`default` | `explicit`).

## Resolution rule (how the system reasons)

```text
resolve_account(user, extension, vendor):
    explicit binding (user, extension, vendor)
    ‖ else the (user, vendor) default account
    ‖ else → the generic auth gate, account-scoped
```

Implemented at the existing requester-authorization stage in the credential
selection path (today's single-account behavior is the named fallback
`select_latest_duplicate_user_reusable_account`,
`crates/ironclaw_reborn_composition/src/product_auth/credentials/runtime_credentials.rs`);
the selection request already carries the requester
(`CredentialAccountSelectionRequest.requester_extension`,
`crates/ironclaw_auth/src/credential.rs`).

## Invariants

- Every credentialed operation executes under exactly one resolved account;
  the audit record carries the resolved account id.
- Exactly one default per (user, vendor) when at least one account exists.
- **Adapters and extensions never observe multiplicity.** Credential
  injection stays host-side; the extension ABI is unchanged by this feature.
- Zero bindings + a single account resolves identically to today's behavior,
  byte for byte (the hiding default).
- The auth gate and its resume carry account context: an expired
  work-account grant reconnects the work account, not the default.

## Semantics against the existing design

- §6.3 account state machine: unchanged — it is already per account (auth
  flows complete to a specific `credential_account_id`).
- §6.4 derived connection status: evaluated against the extension's
  *resolved* account.
- §3.2 shared vendors: scope union and incremental re-consent become
  per-account (a work account may hold narrower consent than a personal one).
- §6.2 removal order step 3: the rule is unchanged, applied per account;
  per-extension bindings are deleted with their extension.
- §4.3 keepalive sweep: already account-enumerating and vendor-blind;
  correct as-is.
- **The manifest is untouched.** A manifest declares *authority* (vendor +
  scopes), never *identity* (which account) — the same line the unified
  taxonomy draws everywhere. A tool declaring per-call account selection is
  an explicit v1 non-goal (see open questions).

## Feature flag and migration

- A deployment flag gates the **UI affordances only** ("connect another
  account", the per-extension binding picker). The resolution rule is total
  and flag-independent. With the flag off, a second connect keeps today's
  replace semantics.
- Migration: backfill `is_default = true` and labels on existing accounts.
  Idempotent, dry-run supported, both database backends.

## Scope fence (v1)

Tool credentials only. Multi-account **channel** surfaces (two Slack
accounts, each an inbound message stream) are excluded; revisit trigger: a
real dual-account channel use case together with a conversation-attribution
design.

## What the train (P0–P7) does now — and nothing else

P6 ships the wire **shape** only: the per-vendor accounts list and each
surface's `resolved_account_id`, with list length ≤ 1 and no selection or
binding behavior (`implementation.md` §10; checklist UI-1 / AUTH-9 evidence
must name the list shape). Everything else in this ADR lands in the dedicated
follow-up PR after P7.

## Verified starting points (2026-07-13)

- The credential store already handles multiplicity:
  `select_unique_configured_account(_for_owner)` and the
  `filesystem_select_unique_configured_account_single_and_multi` test.
- Single-account behavior today is a named selection fallback, not a schema
  constraint: `select_latest_duplicate_user_reusable_account`.
- `CredentialAccountSelectionRequest.requester_extension: Option<ExtensionId>`
  already exists — the per-extension indirection needs no new plumbing
  through dispatch.
- Auth flows complete to a specific `credential_account_id`
  (`crates/ironclaw_auth/src/domain.rs`).

## Open questions for the implementing PR

1. May an extension set a sticky per-extension default at connect time, or
   is the default strictly per (user, vendor)?
2. Label semantics across shared-vendor extensions: one Google account
   serving six extensions must read as one profile everywhere.
3. Per-call account selection ("send this from my work account"):
   permanently out, or deferred behind its own trigger?
