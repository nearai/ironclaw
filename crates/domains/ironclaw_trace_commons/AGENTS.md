# ironclaw_trace_commons

The Trace Commons / TraceDAO **client**. It turns a finished turn into a
consent-bearing, redacted envelope, queues it on disk, and submits it to the
Trace Commons service. It is not the tracing/observability subsystem — that is
`ironclaw_observability`.

**Security posture:** this crate is the last thing that runs before user data
leaves the machine. Redaction is a correctness obligation here, not a
nice-to-have. A change that widens what an envelope carries needs a test that
pins the *absence* of the raw value, not just the presence of the redacted one.

## Module map

| Module | Owns |
|---|---|
| `contribution` | The whole contribution pipeline. Its own module charter is in `src/contribution/mod.rs` — **read that before adding code there**; it says which of the 13 submodules a new item belongs to and why. |
| `capture` | The autonomous turn-end capture pipeline: standing-policy gate, envelope build, queue, immediate flush, and the periodic queue-flush worker. Keyed on a scope string + `ConversationMessage`, so it names no turn, thread, or runtime type — `ironclaw_turn_runner` hands it those two inputs. |
| `client` | The host-facing trace client. |
| `onboarding` | Device-key enrollment, invites, and the onboarding protocol. |
| `redaction` | `redact_sensitive_json`, the shared JSON scrubber `contribution` builds on. |
| `conversation_message` | The shared `ConversationMessage` type. |

`contribution` is a directory module with private submodules and a glob
re-export wall in `mod.rs`. That is deliberate: **`contribution::X` is the only
public path for every item**, so the internal layout can change without
touching the four consumer crates (`ironclaw_assistant`,
`ironclaw_composition`, `ironclaw_host_runtime`, `ironclaw_cli`).
Do not make a submodule `pub`, and do not import
`contribution::<submodule>::X` from outside the crate.

Items that only cross module lines inside `contribution` are `pub(crate)`, not
`pub`. Keep it that way — `pub` here is a promise to four other crates.

## Tests

`src/contribution/tests/` mirrors the production charter one module per owner,
with shared fakes in `tests/support.rs`. Put a new test in the module whose
production owner it pins — a test for `policy.rs` goes in `tests/policy.rs`,
not wherever it was convenient to write. The mapping is one-to-one with **two**
stated exceptions — a rule a contributor cannot apply mechanically is the drift
this file exists to prevent, so both are named rather than left to be inferred:

1. `remote`'s four production modules are covered by `tests/claims.rs`,
   `tests/profile.rs`, `tests/account.rs` and `tests/credentials.rs`.
2. **`tests/credentials.rs` is organised by subject, not by owner.** It covers
   the credential-resolution path end to end, which spans two owners:
   `resolve_trace_credentials_at` and `resolve_effective_flush_target_at` live
   in `queue.rs`, while `trace_upload_claim_cache_key` and
   `build_trace_upload_claim_issuer_request` live in `remote/claim.rs` (which
   `tests/claims.rs` also covers). Splitting it by owner would separate the
   auth-mode resolution from the request it produces, which is the thing the
   tests are actually pinning. A new *credential-resolution* test belongs here;
   any other `queue.rs` test belongs in `tests/queue.rs` (✎ prescriptive —
   no such file exists yet; create it with the first non-credential
   `queue.rs` test).

The architecture specificity gate treats any path component
named `tests` as test code, so vendor names are allowed there and are *not*
allowed in the production submodules — except the two carve-outs named in
`crates/app/ironclaw_architecture_tests/tests/reborn_extension_specificity.rs`
(`tool_payloads.rs` and `classification.rs` hold a deliberate vendor
**denylist** for payload redaction and external-write detection; it is a
superset of the bundled package inventory and must stay that way).

## Known gaps (tracked, not accidental)

These are recorded so they are not rediscovered as bugs. See PROPOSAL §6.4.14.

- **Raw `dirs`/`std::env`/`std::fs` access.** The crate resolves its own paths
  instead of taking a `ScopedFilesystem` like every other domain crate. It has
  no `ironclaw_filesystem` dependency and carries a direct `dirs` dependency.
  Re-measured 2026-08-04 (WS6): 39 production call sites across 5 files; the
  8 in `onboarding/device_key.rs` carry 0700-permission logic the mount plane
  cannot express yet, so adoption is two decisions, not one conversion. Do
  not add new raw-filesystem call sites.

Two former entries here are **discharged** (verified 2026-08-05, WS12 F3a) and
struck rather than left to be re-attempted:

- ~~The `recording`/`paths` boundary-laundering re-export modules in
  `lib.rs`.~~ Both are gone; the CLI migrated
  (`commands/traces/mod.rs::trace_contribution_dir` delegates to
  `contribution::trace_contribution_dir_for_scope(None)`). Do not
  re-introduce a re-export module whose only purpose is sparing a consumer a
  dependency declaration.
- ~~The rename to `ironclaw_trace_commons`.~~ Done — this crate *is*
  `ironclaw_trace_commons`, at `crates/domains/ironclaw_trace_commons/`.
