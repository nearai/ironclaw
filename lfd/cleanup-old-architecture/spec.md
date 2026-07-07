# Spec: cleanup-old-architecture — migration completeness + CLI wiring

Sources: `crates/ironclaw_reborn_migration/` (+ its `CLAUDE.md`),
`crates/ironclaw_reborn_migration/tests/migration_roundtrip.rs`,
`docs/reborn/contracts/migration-compatibility.md`,
`docs/plans/2026-06-25-cas-migration.md`, `src/db/CLAUDE.md`,
`src/history/`. Non-goals and rollback concerns at the end.

## 1. `Command::Migrate` wiring

Add a `Migrate` variant to `crates/ironclaw_reborn_cli/src/commands/mod.rs`
(`Command` enum) with a `commands/migrate.rs` module, mirroring the
existing subcommand shape (`execute(RebornCliContext)`). It is a thin
wrapper over `ironclaw_reborn_migration::run_migration` — the engine stays
in the library (module-owned initialization; no conversion logic in the
CLI crate). Flags mirror the standalone binary
(`crates/ironclaw_reborn_migration/src/main.rs`): source
(libsql path | postgres url, mutually exclusive), target defaulting to the
resolved Reborn storage root, `--tenant-id`/`--agent-id` (default
`default`), `--secret-master-key` (env preferred), `--dry-run`,
`--report <path>`. No `Debug` derive on the args struct (secret-bearing
fields — same rationale as the standalone binary). The JSON
`MigrationReport` goes to stdout or `--report`.

Test-first: a CLI test proving the subcommand parses, dispatches with the
mapped options, honors `--dry-run`, and emits the report (red before the
wiring lands).

## 2. Six gap categories to close (the scored core)

Each category: red→green in `migration_roundtrip.rs` first — the test's
expected gap set is spec-governed and may only SHRINK. Closing a category
means the domain's rows land in a real Reborn store AND the roundtrip
test reads them back through the store's public API. Where Reborn has no
receiving store, building the minimal Reborn-side seam is in scope (in
the owning crate, not in the migration tool).

1. **Non-cron triggers** — routine `Trigger::{Event,SystemEvent,Webhook,
   Manual}` and mission `Cadence::{OnEvent,OnSystemEvent,Webhook,Manual}`
   currently `NoTargetConcept`. Extend the Reborn trigger model
   (`ironclaw_triggers`) so these sources are representable (a
   non-schedule `TriggerSourceKind` or equivalent), then convert them.
2. **routine_runs history** — currently no public run-history insert on
   `TriggerRepository`. Add the seam; migrate run history rows. Run
   history is execution provenance — never dropped.
3. **Settings KV** — v1 `settings` per-user KV → the typed-repository
   path per `docs/reborn/contracts/migration-compatibility.md` §6
   (validate → import into typed repositories; file projections
   allowed as compatibility artifacts, not source of truth). Unknown
   keys must still carry (typed unknown-key bucket), not drop.
4. **agent_jobs chain** — `agent_jobs` + `job_actions` + `job_events` →
   a Reborn job/run history representation. Job events/actions are LLM
   output (retention invariant): every row carries or the row is a typed,
   enumerated skip. Preserve job→conversation links (cross-references
   asserted by mixed-seed eval cases).
5. **memory_document_versions** — per-doc version history migrates
   (compatibility doc §3 maps `versions -> memory_document_versions`;
   `sha256:{hex}` hash format stays valid).
6. **Extension manifest fidelity + WASM binary** — carry the v1 WASM
   binary bytes and a faithful manifest (capabilities from
   `tool_capabilities`, not the `ask`-permission placeholder). The
   placeholder-manifest `manifest_fidelity` loss disappears when closed.

Documented DEGRADATIONS that remain acceptable (enumerated, never
silent): routine/mission `Failed` → `Paused`; non-user/assistant
transcript messages retained in thread `metadata_json.legacy_v1`.

## 3. Cross-cutting invariants (every category, every run)

- **v1-DB-read-only**: the migration NEVER mutates the source. No
  `DELETE FROM`/`DROP TABLE`/`TRUNCATE`/`UPDATE` against v1 stores
  (caps-enforced at 0 in diff; eval-enforced via before/after digest).
- **No silent drops**: every source row is migrated, degraded-and-
  enumerated, or skipped-and-enumerated (typed per-row `LossyItem` with
  domain/source id/field/reason). Reconciliation must hold per domain:
  `migrated_count + skipped_count == source_count`.
- **Idempotency**: re-running against the same target adds nothing —
  extend the current identity/extension idempotency to triggers, threads,
  memory, settings, jobs, runs (deterministic ids / upsert-by-source-id;
  the "fresh trigger ids per run" behavior in the current tool must be
  fixed as part of gap 1).
- **Dry-run honesty**: `--dry-run` produces the same report a wet run
  would (same stats + losses) and writes NOTHING to the target.
- **Secrets**: re-encrypt via the Reborn store; preserve expiry and usage
  metadata; a secret that fails to decrypt is a per-secret enumerated
  skip, not an abort; plaintext never appears in reports, projections,
  logs, or errors.
- **LLM-data retention**: conversation messages, job events/actions,
  thread transcripts are never dropped or truncated.

## 4. Eval runner profile (`migration`) — implementer-owned file

`tests/integration/lfd/profiles/cleanup-old-architecture.rs`, profile
name `"migration"`. It must:

1. Interpret `setup.profile_extra.seed` — the declarative v1 dataset spec
   (schema below) — by seeding a real v1 libsql DB on a tempdir via v1
   APIs (`ironclaw::db::connect_with_handles` etc., as
   `migration_roundtrip.rs` does).
2. Interpret `setup.profile_extra.run.mode`:
   `"wet"` (one real run) · `"dry"` (dry-run only) · `"dry_then_wet"` ·
   `"double"` (two wet runs) · `"dry_then_double"`.
   Execute through the REAL CLI path (`Command::Migrate` execution seam),
   not by calling converters directly.
3. Answer the three profile-specific state-query kinds (contract in §5).
   Digest the v1 DB before the first run for `v1_db_digest`.

Seed spec shape (`profile_extra.seed`) — all arrays optional, `ref`
values are stable case-local handles:

```jsonc
{
  "backend": "libsql",
  "users":        [{"ref","user_id","email","display_name",
                    "identities":[{"provider","provider_user_id","email_verified"}],
                    "channels":[{"channel","external_id"}]}],
  "conversations":[{"ref","channel","user",              // user = users[].ref
                    "user_valid": false,                  // optional: seed dangling owner
                    "messages":[{"role","content"}]}],   // role: user|assistant|system|tool
  "routines":     [{"ref","name","user","trigger","action","enabled",
                    "status",                             // ok|failed|legacy_enum (legacy_enum = raw
                                                          //   status string invalid in v1's enum)
                    "runs"}],                             // routine_runs history rows to seed
  "missions":     [{"ref","name","user","cadence","status",
                    "thread_messages":[{"role","content"}]}],  // engine-v2 blobs in memory_documents
  "memory_docs":  [{"ref","path","content","versions"}],  // versions = prior version rows to seed
  "settings":     [{"key","value"}],
  "secrets":      [{"ref","name","provider","value","expires_at",
                    "decryptable": false}],               // false = seed ciphertext under a wrong key
  "jobs":         [{"ref","title","conversation","actions","events"}],
  "extensions":   [{"ref","name","version","status","wasm_len",  // deterministic pseudo-random
                    "allowed_secrets":[]}]                        //   wasm bytes of this length
}
```

Trigger/cadence objects: `{"kind":"cron","expr","tz"}` ·
`{"kind":"event","pattern"}` · `{"kind":"system_event","source","event_type"}`
· `{"kind":"webhook","path"}` · `{"kind":"manual"}`.

## 5. Profile state-query contract (the sealed expectations diff these)

Projections are read from the ACTUAL Reborn target stores (public store
APIs / fresh connection), normalized: counts, lexicographically sorted
ref lists, booleans. Never derived from the MigrationReport (report
honesty is itself scored via `migration_report`).

### kind `"v1_db_digest"` — params `{}`

Result: `{"pre": "<sha256>", "post": "<sha256>", "unchanged": bool,
"row_counts": {<table>: n, ...}}`. `pre` is digested before the first
migration run, `post` after the last; `unchanged = (pre == post)`. The
digest covers all v1 table contents in deterministic order. `row_counts`
is informational.

### kind `"migration_projection"` — params `{"domain": "<d>", "after_run": 1|2?}`

`after_run` (default 1; only present when `run.mode` executes two wet
runs) selects which run's post-state snapshot to project. Domains and
result shapes:

- `"threads"` → `{"count": n, "refs": [<conversation/mission ref>...]}` —
  sorted source refs of threads present in the Reborn thread store.
- `"messages"` → `{"count": n,               // first-class migrated msg rows (user+assistant)
   "llm_count": n, "degraded_to_metadata": n, "dropped": 0}` —
  `dropped` counts source LLM rows with no Reborn representation and no
  enumerated skip; MUST be 0.
- `"triggers"` → `{"count": n, "refs": [<routine/mission name>...]}` (all
  trigger sources, cron and non-cron).
- `"routine_runs"` → `{"count": n}` — migrated run-history rows.
- `"settings"` → `{"count": n, "keys": [<key>...]}` (sorted).
- `"jobs"` → `{"count": n, "actions": n, "events": n, "events_dropped": 0}`.
- `"memory"` → `{"count": n, "paths": [<path>...]}` (sorted;
  non-engine memory documents).
- `"memory_versions"` → `{"count": n}` — migrated version rows.
- `"secrets"` → `{"count": n, "secrets": [{"ref": <name>,
   "provider": <p>, "decryptable": true,   // proven by a real decrypt via the Reborn store
   "has_plaintext": false,                  // true iff plaintext leaked into any projection/report surface
   "expires_at": <iso|null>}...]}` (sorted by ref). NEVER include
  plaintext or ciphertext material.
- `"identities"` → `{"count": n, "oauth": n, "channels": n}`.
- `"extensions"` → `{"count": n, "refs": [<name>...],
   "wasm_binary_carried": bool,             // byte-for-byte equal to the seeded binary, for ALL installs
   "manifest_fidelity_carried": bool}`.     // manifest reflects v1 capabilities, not a placeholder

Dry-only runs (`run.mode == "dry"`): projections read the (empty) target;
counts are 0.

### kind `"migration_report"` — params `{"run": "wet"|"dry", "view": "reconciliation"|"lossy"}`

`run` selects which executed run's report (in `dry_then_*` modes the dry
report vs the first wet report).

- `view: "reconciliation"` → `{"dry_run": bool, "all_accounted": bool,
  "domains": {<domain>: {"source_count": n, "migrated_count": n,
  "degraded_count": n, "skipped_count": n, "fully_accounted": bool}}}`.
  Domain keys: `thread`, `message`, `routine`, `routine_runs`, `mission`,
  `setting`, `job`, `memory`, `memory_versions`, `secret`, `identity`,
  `extension`. Semantics: `migrated_count` includes degraded-form rows;
  `degraded_count` ⊆ migrated (each degraded row also enumerated in the
  report); `skipped_count` = rows not carried (each enumerated);
  `fully_accounted = migrated + skipped == source`;
  `all_accounted` = every domain `fully_accounted`. `source_count` comes
  from counting the SOURCE DB, not from the report's own claims.
- `view: "lossy"` → `{"items": [{"domain": <d>, "source_ref": <case seed
  ref of the affected row>, "field": <field|"*">, "reason":
  "unparseable"|"degraded"|"no_target_concept"|"no_target_field"}...]}` —
  the report's enumerated items mapped back to seed refs (the profile
  seeded the rows, so it owns the id→ref map).

Until the profile implements these kinds, cases return
`status: "unsupported"` and score 0 — expected for the skeleton.

## 6. Deletion-ledger process (unscored stretch; from the lane goal)

Only after: holdout bar hit AND the full repo suite green. Maintain the
ledger in this file (append a `## Deletion ledger` section). Every entry
names: the v1 module/route/config/binary path/test being removed; the
Reborn owner or replacement path; the executable tests proving
replacement behavior (the migration-fidelity eval cases count as
replacement evidence per the ADDENDA reconciliation); runtime
compatibility + rollback risk; docs/parity files to change
(`FEATURE_PARITY.md` edits serialize with lanes 01/06 per the ADDENDA
conflict table). No deletion is eligible until its entry is complete.
One coherent surface per cycle; never delete a test because it fails
after deletion — retarget it if the behavior still matters; no new
feature behavior in `src/` ever.

## 7. Non-goals

- Postgres-source parity beyond what `run_migration` already supports —
  libsql-first (Docker-free) per the brief; Postgres variants only where
  the harness supports them.
- Live-LLM behavior of migrated state (no live cases in this lane).
- Rewriting v1 schemas or "fixing" v1 data in place (read-only source).
- Building a generic v2 settings UI, job execution engine, or trigger
  firing for the new source kinds — REPRESENTATION + fidelity is in
  scope; new runtime behavior on top is other lanes' work.

## 8. Rollback / risk notes

- The migration tool is additive on the target; the v1 source is
  untouched — rollback = delete the target store and re-run.
- Idempotency changes (deterministic trigger/thread ids) alter the
  current tool's re-run semantics; the roundtrip test pins the new
  upsert behavior.
- `mounts.rs` alias→path layout must be reconciled with composition when
  `Command::Migrate` lands (documented caveat in the crate) — runtime
  readback of migrated state is the acceptance seam for that.
