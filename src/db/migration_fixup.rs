//! PostgreSQL migration checksum fix-up.
//!
//! This module exists because of a single historical accident: PR #1151
//! ("Refactor owner scope across channels and fix default routing fallback")
//! modified `migrations/V6__routines.sql` *in place* after that migration
//! had already shipped in v0.18.0 and been applied to production databases.
//! Refinery records a SipHasher13 checksum of every applied migration in
//! `refinery_schema_history`, and on every startup it re-validates each
//! filesystem migration against the stored checksum. The in-place edit
//! caused refinery to abort startup with:
//!
//!   Error: Migration failed: applied migration V6__routines is different
//!   than filesystem one V6__routines
//!
//! See [issue #1328](https://github.com/nearai/ironclaw/issues/1328).
//!
//! ## Why a runtime fix-up is required
//!
//! Two populations of databases exist in the wild:
//!
//! 1. **Pre-#1151 installs** (v0.18.0 and earlier) — `refinery_schema_history`
//!    holds the checksum of the *original* V6 (`notify_user TEXT NOT NULL
//!    DEFAULT 'default'`).
//! 2. **Post-#1151 installs** (fresh installs of v0.19.0 or any
//!    staging build after the merge) — `refinery_schema_history` holds the
//!    checksum of the *modified* V6 (`notify_user TEXT,`).
//!
//! Reverting V6 on its own (which we have also done) only fixes population
//! #1; population #2 would then break in the opposite direction. To handle
//! both, we recompute the canonical checksum from the embedded V6 SQL on
//! startup and rewrite any divergent row in `refinery_schema_history`
//! before refinery validates it.
//!
//! V13 (`V13__owner_scope_notify_targets.sql`) handles the schema change
//! incrementally for population #1 and is a no-op for population #2
//! (`ALTER COLUMN ... DROP NOT NULL` is idempotent), so both populations
//! converge to the same final schema.
//!
//! ## Why this is safe and narrowly scoped
//!
//! - We only touch one row: `version = 6 AND name = 'routines'`.
//! - We only update when the stored checksum disagrees with the embedded
//!   one — so on a clean install or already-realigned database the call
//!   is a no-op.
//! - We never disable refinery's checksum validation
//!   (`set_abort_divergent(false)`) — that would mask future genuine drift.
//! - The set of known divergences is hard-coded as a list, so adding a
//!   future fix-up is an explicit code change visible in review.
//!
//! See also `migrations/checksums.lock` and the
//! `released_migrations_are_immutable` test, which together prevent any
//! future PR from modifying an already-released migration.

use deadpool_postgres::Object as PgClient;
use refinery::Migration;

use crate::error::DatabaseError;

/// One known historical migration whose on-disk content was modified after
/// release. Add a new entry here only if the same accident ever happens
/// again — the immutability test in `migrations/checksums.lock` is the
/// preferred guard.
/// One known historical migration whose on-disk content was modified after
/// release.
///
/// Lifetime-generic so integration tests can construct stack-allocated
/// instances with non-`'static` borrowed slices (avoiding `Box::leak` to
/// satisfy `'static` bounds). Production `KNOWN_DIVERGENCES` is
/// `&[KnownDivergence<'static>]` and is unaffected.
pub(crate) struct KnownDivergence<'a> {
    pub(crate) version: i32,
    pub(crate) name: &'a str,
    /// The current (canonical) SQL content, embedded at compile time.
    pub(crate) sql: &'a str,
    /// The exact set of historical bad checksums we are willing to rewrite
    /// for this migration. **The fix-up only fires when the stored checksum
    /// matches one of these literals** — any other divergence (manual
    /// tampering, hardware corruption, an unknown future regression) is
    /// left alone so refinery can still abort startup loudly.
    pub(crate) known_bad_checksums: &'a [u64],
    /// Human-readable explanation of why this divergence exists, surfaced
    /// in the realignment warning log so future entries are not coupled to
    /// the V6/#1328 wording.
    pub(crate) explanation: &'a str,
}

const KNOWN_DIVERGENCES: &[KnownDivergence<'static>] = &[KnownDivergence {
    version: 6,
    name: "routines",
    sql: include_str!("../../migrations/V6__routines.sql"),
    // The single historical bad checksum: V6 with `notify_user TEXT,`
    // (the post-#1151 / v0.19.0 fresh-install variant). Computed from
    // `git show 878a67cd:migrations/V6__routines.sql`. Pinned by the
    // `v6_known_bad_checksum_matches_post_1151_content` test below.
    known_bad_checksums: &[11230857244097235596],
    explanation: "Migration content matches the v0.18.0 release; the schema \
                  change introduced in PR #1151 is applied incrementally by \
                  V13__owner_scope_notify_targets.",
}];

/// Session-level PostgreSQL advisory lock key used to serialize concurrent
/// migration runs across replicas. Set to issue number 1328 for grep-ability
/// (`SELECT * FROM pg_locks WHERE locktype = 'advisory' AND objid = 1328`).
const MIGRATION_LOCK_KEY: i64 = 1328;

/// Run the full PostgreSQL migration sequence: acquire an advisory lock,
/// realign any historically diverged checksums, then run refinery's embedded
/// migrations. Releases the lock on every exit path including errors.
///
/// **This is the single entry point for running PostgreSQL migrations.** Both
/// `Store::run_migrations` and `SetupWizard::run_migrations_postgres`
/// delegate here. Adding a new migration entry point? Call this function;
/// do not re-implement the fix-up + refinery sequence inline.
///
/// ## Why an advisory lock
///
/// Two replicas starting simultaneously against the same database can race:
/// one finishes `realign_diverged_checksums` and commits, then the other's
/// `refinery::Runner::run_async` reads its own SELECT-then-validate pair and
/// the timing between them is unprotected, potentially causing spurious
/// startup failures. The session-level advisory lock serializes the entire
/// fix-up + refinery sequence per database. It also hardens the pre-existing
/// refinery race that has always existed for concurrent multi-replica starts.
///
/// We use a *session-level* lock (not `pg_advisory_xact_lock`) because
/// refinery's `run_async` opens its own internal transactions and an outer
/// transaction-scoped lock would conflict with refinery's transaction
/// boundaries.
pub async fn run_postgres_migrations_with_fixup(
    client: &mut PgClient,
) -> Result<(), DatabaseError> {
    use refinery::embed_migrations;
    // The path is relative to `CARGO_MANIFEST_DIR`, not this file.
    embed_migrations!("migrations");

    // Acquire the lock. Blocks until released by any other holder.
    client
        .execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK_KEY])
        .await
        .map_err(|e| DatabaseError::Migration(format!("acquire migration lock: {e}")))?;

    // Run the realignment + refinery sequence, holding the lock for the
    // duration. We capture the result and *always* release before
    // returning, even on error.
    //
    // `client` is `&mut Object` (from `deadpool_postgres`); the triple
    // deref reaches `tokio_postgres::Client` via
    // `Object → ClientWrapper → Client`, which is what refinery's
    // `AsyncMigrate` impl is bound to.
    let result: Result<(), DatabaseError> = async {
        realign_diverged_checksums_with(client, KNOWN_DIVERGENCES).await?;
        migrations::runner()
            .run_async(&mut ***client)
            .await
            .map_err(|e| DatabaseError::Migration(e.to_string()))?;
        Ok(())
    }
    .await;

    // Always release the lock. If the unlock itself fails, log it and
    // surface the original migration error if there was one — losing the
    // lock release is less important than reporting the underlying cause.
    if let Err(e) = client
        .execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY])
        .await
    {
        tracing::error!(
            error = %e,
            "failed to release migration advisory lock — connection drop will \
             release it eventually, but other replicas may block until then"
        );
    }

    result
}

/// Realign `refinery_schema_history` rows whose stored checksum disagrees
/// with the canonical checksum of the embedded migration. Must be called
/// before `refinery::Runner::run_async`.
///
/// **Most callers should use [`run_postgres_migrations_with_fixup`]**, which
/// bundles this with refinery and the advisory lock. This function is
/// retained as a public entry point only for callers that already manage
/// their own refinery invocation (none today).
pub async fn realign_diverged_checksums(client: &mut PgClient) -> Result<(), DatabaseError> {
    realign_diverged_checksums_with(client, KNOWN_DIVERGENCES).await
}

/// Inner implementation that takes the divergence list as a parameter, so
/// integration tests can drive it against synthetic rows without colliding
/// with real V6 rows in a shared test database.
pub(crate) async fn realign_diverged_checksums_with(
    client: &mut PgClient,
    divergences: &[KnownDivergence<'_>],
) -> Result<(), DatabaseError> {
    // On a fresh install the history table does not yet exist. Refinery
    // will create it during the first `run_async()` call. There is nothing
    // to realign in that case.
    // Use an unqualified identifier so PostgreSQL resolves the table via
    // the active `search_path` — matching how refinery itself locates the
    // history table. Hard-coding `public.` would silently skip the fix-up
    // on deployments using a non-default schema.
    let history_exists: bool = client
        .query_one(
            "SELECT to_regclass('refinery_schema_history') IS NOT NULL",
            &[],
        )
        .await
        .map_err(|e| DatabaseError::Migration(format!("probe refinery_schema_history: {e}")))?
        .get(0);

    if !history_exists {
        return Ok(());
    }

    for divergence in divergences {
        // Compute the canonical checksum the same way refinery does
        // (SipHasher13 over name, version, sql in that order). Refinery
        // stores the resulting u64 as a decimal string in the `checksum`
        // column.
        let migration_label = format!("V{}__{}", divergence.version, divergence.name);
        let migration = Migration::unapplied(&migration_label, divergence.sql).map_err(|e| {
            DatabaseError::Migration(format!(
                "compute canonical checksum for {migration_label}: {e}"
            ))
        })?;
        let canonical_checksum = migration.checksum().to_string();

        // Defensive: the canonical checksum must never appear in the bad
        // list, otherwise we'd be rewriting already-correct rows. This is
        // a programming error in `KNOWN_DIVERGENCES`, not a runtime
        // condition, but it must still be detected in release builds (a
        // `debug_assert!` would be stripped). Return a hard error so the
        // process refuses to start with a misconfigured fix-up table —
        // see PR #2101 review by @serrrfirat. Cost is one constant-time
        // slice lookup per startup; the `KNOWN_DIVERGENCES` list has at
        // most a handful of entries.
        if divergence
            .known_bad_checksums
            .contains(&migration.checksum())
        {
            return Err(DatabaseError::Migration(format!(
                "{migration_label}: canonical checksum is listed in \
                 known_bad_checksums — this is a programming error in \
                 KNOWN_DIVERGENCES that would silently rewrite \
                 already-correct rows",
            )));
        }

        // Only rewrite rows whose stored checksum is one of the known
        // historical bad values for this migration. Any other divergence
        // (manual tampering, hardware corruption, an unrelated future
        // regression) is intentionally left alone so refinery still aborts
        // startup loudly. See PR #2101 review by @serrrfirat.
        let known_bad: Vec<String> = divergence
            .known_bad_checksums
            .iter()
            .map(|c| c.to_string())
            .collect();

        let updated = client
            .execute(
                "UPDATE refinery_schema_history \
                 SET checksum = $1 \
                 WHERE version = $2 AND name = $3 AND checksum = ANY($4)",
                &[
                    &canonical_checksum,
                    &divergence.version,
                    &divergence.name,
                    &known_bad,
                ],
            )
            .await
            .map_err(|e| {
                DatabaseError::Migration(format!("realign checksum for {migration_label}: {e}"))
            })?;

        if updated > 0 {
            // `warn!` is intentional here even though CLAUDE.md warns
            // against `info!`/`warn!` in background tasks (they corrupt
            // the REPL/TUI). This fix-up runs during database migration
            // at startup, *before* any channel/REPL/TUI is initialized,
            // so terminal-rendering interference is impossible. If this
            // call is ever moved later in startup, downgrade to `debug!`
            // or pre-buffer the message.
            tracing::warn!(
                migration = %migration_label,
                rows = updated,
                "Realigned refinery_schema_history checksum: {}",
                divergence.explanation
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[cfg(feature = "integration")]
    static POSTGRES_MIGRATION_FIXUP_TEST_LOCK: tokio::sync::Mutex<()> =
        tokio::sync::Mutex::const_new(());

    fn migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
    }

    fn parse_lockfile(contents: &str) -> HashMap<String, u64> {
        let mut map = HashMap::new();
        for (lineno, raw) in contents.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=').unwrap_or_else(|| {
                panic!(
                    "checksums.lock line {} is not `name = checksum`: {raw}",
                    lineno + 1
                )
            });
            let parsed: u64 = value.trim().parse().unwrap_or_else(|e| {
                panic!(
                    "checksums.lock line {} has invalid u64 checksum {value}: {e}",
                    lineno + 1
                )
            });
            // Reject duplicate keys: a stray duplicate would silently
            // overwrite an earlier pinned checksum and weaken the
            // immutability guard. Detect it loudly during the test.
            let key = key.trim().to_string();
            if map.insert(key.clone(), parsed).is_some() {
                panic!(
                    "checksums.lock line {} contains duplicate migration key: {key}",
                    lineno + 1
                );
            }
        }
        map
    }

    /// Immutability guard for released migrations.
    ///
    /// Modifying an already-released migration is silently catastrophic:
    /// production databases store a checksum of the original content and
    /// refinery aborts on startup if the file changes (see issue #1328).
    /// This test pins every migration's checksum to a value in
    /// `migrations/checksums.lock`. Modifying any released migration —
    /// even by a single character — fails this test. Adding a new
    /// migration also fails this test until you add a matching lockfile
    /// entry in the same commit.
    ///
    /// **If this test fails, do not "fix" it by editing the lockfile to
    /// match.** The correct response is almost always:
    ///
    /// 1. Revert your edit to the released migration.
    /// 2. Put the schema change in a *new* `V<next>__*.sql` migration.
    /// 3. Add the new migration's checksum to `checksums.lock`.
    ///
    /// The only legitimate reason to overwrite an existing lockfile entry
    /// is if the migration has *never* shipped on `staging` or `main`
    /// (still in your local feature branch). When in doubt, ask.
    #[test]
    fn released_migrations_are_immutable() {
        let dir = migrations_dir();
        let lockfile_path = dir.join("checksums.lock");
        let lockfile_contents = std::fs::read_to_string(&lockfile_path).unwrap_or_else(|e| {
            panic!(
                "missing {}: {e}\nRun `cargo test -p ironclaw -- --ignored \
                 regenerate_migration_checksums_lockfile` to bootstrap it.",
                lockfile_path.display()
            )
        });
        let expected = parse_lockfile(&lockfile_contents);

        let mut sql_files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();
        sql_files.sort();

        let mut errors = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for path in &sql_files {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap();
            let sql = std::fs::read_to_string(path).unwrap();
            let migration = match Migration::unapplied(stem, &sql) {
                Ok(m) => m,
                Err(e) => {
                    errors.push(format!("{stem}: invalid migration name or SQL: {e}"));
                    continue;
                }
            };
            let actual = migration.checksum();
            seen.insert(stem.to_string());

            match expected.get(stem) {
                Some(&pinned) if pinned == actual => {}
                Some(&pinned) => errors.push(format!(
                    "{stem}: checksum mismatch — file produces {actual}, \
                     lockfile pins {pinned}. \
                     If you intentionally modified this migration AND it has \
                     never shipped on staging/main, update checksums.lock. \
                     Otherwise REVERT your edit and put the change in a new \
                     migration."
                )),
                None => errors.push(format!(
                    "{stem}: missing from migrations/checksums.lock. \
                     Add `{stem} = {actual}` to checksums.lock in this commit."
                )),
            }
        }

        for pinned in expected.keys() {
            if !seen.contains(pinned) {
                errors.push(format!(
                    "{pinned}: present in checksums.lock but no matching \
                     migrations/{pinned}.sql file exists. Did you delete a \
                     released migration?"
                ));
            }
        }

        if !errors.is_empty() {
            panic!(
                "released migrations are immutable — {} problem(s):\n  - {}",
                errors.len(),
                errors.join("\n  - ")
            );
        }
    }

    /// Bootstrap helper. Run with:
    ///
    /// ```text
    /// cargo test -p ironclaw -- --ignored regenerate_migration_checksums_lockfile
    /// ```
    ///
    /// Writes a fresh `migrations/checksums.lock` from the current
    /// filesystem state. Only use this when intentionally adding a new
    /// migration or bootstrapping the lockfile for the first time.
    #[test]
    #[ignore]
    fn regenerate_migration_checksums_lockfile() {
        let dir = migrations_dir();
        let mut sql_files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();
        // Natural-sort by parsed migration version (so V2 comes before
        // V10), not lex-sort (which would order V10 before V2). The
        // resulting lockfile reads in numeric order which makes review
        // diffs easier to scan.
        sql_files.sort_by_key(|path| {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            // V<n>__<name> → parse the digits between `V` and `__`.
            stem.strip_prefix('V')
                .and_then(|s| s.split_once("__"))
                .and_then(|(v, _)| v.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });

        let mut output = String::new();
        output.push_str(
            "# Released migration checksums (refinery SipHasher13 over name+version+sql).\n\
             #\n\
             # This file is the immutability guard for released migrations. The\n\
             # `released_migrations_are_immutable` test in src/db/migration_fixup.rs\n\
             # asserts every migration listed below still hashes to the pinned value\n\
             # and that every migration on disk has a pinned value here.\n\
             #\n\
             # Modifying a released migration is forbidden — it desyncs every\n\
             # production database from refinery's checksum validation. See issue\n\
             # #1328 for the historical accident this guard prevents.\n\
             #\n\
             # When adding a new migration, append a new line in the same commit.\n\
             # Regenerate locally with:\n\
             #   cargo test -p ironclaw -- --ignored regenerate_migration_checksums_lockfile\n\n",
        );
        for path in &sql_files {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap();
            let sql = std::fs::read_to_string(path).unwrap();
            let migration = Migration::unapplied(stem, &sql).unwrap();
            output.push_str(&format!("{stem} = {}\n", migration.checksum()));
        }

        let lockfile_path = dir.join("checksums.lock");
        std::fs::write(&lockfile_path, output).unwrap();
        eprintln!("wrote {}", lockfile_path.display());
    }

    /// One-shot helper: print the canonical checksum for an arbitrary
    /// SQL file path supplied via the `MIGRATION_CHECKSUM_PATH` env var.
    /// Used to compute the historical bad V6 checksum for the
    /// `KNOWN_DIVERGENCES` whitelist:
    ///
    /// ```text
    /// MIGRATION_CHECKSUM_PATH=/tmp/v6_modified.sql \
    ///   cargo test -p ironclaw -- --ignored \
    ///   compute_checksum_for_external_file --nocapture
    /// ```
    #[test]
    #[ignore]
    fn compute_checksum_for_external_file() {
        let path = std::env::var("MIGRATION_CHECKSUM_PATH").expect("MIGRATION_CHECKSUM_PATH");
        let stem = std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap()
            .to_string();
        let sql = std::fs::read_to_string(&path).unwrap();
        let migration = Migration::unapplied(&stem, &sql).unwrap();
        eprintln!("{stem} = {}", migration.checksum());
    }

    /// Sanity check that the embedded V6 SQL still hashes to the v0.18.0
    /// checksum. If this fails, V6 has been re-modified and issue #1328
    /// will recur on every existing PostgreSQL deployment.
    ///
    /// This test pins the literal checksum value as a second line of
    /// defence: a malicious or careless edit that updates *both* V6 and
    /// `checksums.lock` would still defeat `released_migrations_are_immutable`,
    /// but it cannot defeat this hard-coded sentinel.
    #[test]
    fn v6_routines_matches_v018_checksum() {
        // The v0.18.0 V6__routines.sql checksum (refinery's SipHasher13
        // over name "routines" + version 6 + the original SQL content).
        // This is the value stored in `refinery_schema_history` on every
        // pre-#1151 PostgreSQL deployment. Do not change.
        const V018_V6_CHECKSUM: u64 = 18049045188188232070;

        let on_disk = std::fs::read_to_string(migrations_dir().join("V6__routines.sql")).unwrap();
        let embedded = KNOWN_DIVERGENCES[0].sql;
        assert_eq!(
            embedded, on_disk,
            "embedded V6 SQL has drifted from migrations/V6__routines.sql"
        );

        let migration = Migration::unapplied("V6__routines", &on_disk).unwrap();
        assert_eq!(
            migration.checksum(),
            V018_V6_CHECKSUM,
            "V6__routines.sql has been modified — it no longer matches the \
             v0.18.0 checksum and issue #1328 will recur on every existing \
             PostgreSQL deployment. Revert your edit and put the schema \
             change in a new migration."
        );
    }

    /// Pin the historical bad V6 checksum (the post-#1151 modified
    /// content) so the realignment whitelist cannot drift. The fix-up
    /// function rewrites *only* rows whose stored checksum matches a
    /// value in `KNOWN_DIVERGENCES[..].known_bad_checksums`. If this
    /// list is corrupted or accidentally widened, refinery's checksum
    /// validation degrades from "narrowly exempt one historical row" to
    /// "silently mask any V6 corruption". This sentinel ensures the V6
    /// entry contains exactly one expected literal value.
    ///
    /// Source for the literal: `git show 878a67cd:migrations/V6__routines.sql`
    /// (the commit from PR #1151 that introduced the divergence).
    #[test]
    fn v6_known_bad_checksum_matches_post_1151_content() {
        const POST_1151_BAD_V6_CHECKSUM: u64 = 11230857244097235596;

        let v6 = &KNOWN_DIVERGENCES[0];
        assert_eq!(v6.version, 6);
        assert_eq!(v6.name, "routines");
        assert_eq!(
            v6.known_bad_checksums,
            &[POST_1151_BAD_V6_CHECKSUM],
            "the V6 known-bad checksum list has been altered. The only \
             value that should appear here is the SipHasher13 of \
             `git show 878a67cd:migrations/V6__routines.sql`. Widening \
             the list silently masks production database corruption — \
             do not change this without a very good reason."
        );

        // Also assert canonical and bad are distinct, otherwise the
        // fix-up would no-op.
        let canonical = Migration::unapplied("V6__routines", v6.sql)
            .unwrap()
            .checksum();
        assert_ne!(
            canonical, POST_1151_BAD_V6_CHECKSUM,
            "canonical V6 checksum collides with known-bad — fix-up would no-op"
        );
    }

    /// Integration test that drives `realign_diverged_checksums_with`
    /// against a real PostgreSQL instance, codifying the manual smoke
    /// test from PR #2101 and closing the last untested seam noted by
    /// @serrrfirat. Skips gracefully if no database is reachable.
    ///
    /// To avoid racing real V6 rows in shared CI databases, the test
    /// uses a synthetic version `99999` row with name `test_routines`
    /// and a custom `KnownDivergence` slice — the production
    /// `KNOWN_DIVERGENCES` constant is left untouched.
    ///
    /// Run with:
    ///
    /// ```text
    /// DATABASE_URL=postgres://localhost/ironclaw_test \
    ///     cargo test --features integration --lib \
    ///     db::migration_fixup::tests::realign_repairs_known_bad_checksum_against_postgres
    /// ```
    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn realign_repairs_known_bad_checksum_against_postgres() {
        use deadpool_postgres::{Manager, Pool};
        use tokio_postgres::{Config, NoTls};

        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://localhost/ironclaw_test".to_string());
        let config: Config = match database_url.parse() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skipping: invalid DATABASE_URL ({e})");
                return;
            }
        };
        let mgr = Manager::new(config, NoTls);
        let pool = match Pool::builder(mgr).max_size(2).build() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("skipping: failed to build pool ({e})");
                return;
            }
        };
        let mut client = match pool.get().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skipping: database unavailable ({e})");
                return;
            }
        };
        let _guard = POSTGRES_MIGRATION_FIXUP_TEST_LOCK.lock().await;

        // Make sure refinery_schema_history exists. We can't rely on
        // the test DB having had migrations run, so create it on
        // demand using refinery's own DDL shape (4 columns matching
        // the tokio_postgres driver in refinery 0.8.16).
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS refinery_schema_history ( \
                    version INT4 PRIMARY KEY, \
                    name VARCHAR(255), \
                    applied_on VARCHAR(255), \
                    checksum VARCHAR(255))",
            )
            .await
            .expect("create refinery_schema_history");

        // Use a synthetic divergence so we don't touch any real V*
        // row that another test or migration might depend on.
        const TEST_VERSION: i32 = 99999;
        const TEST_NAME: &str = "test_routines";
        const TEST_SQL: &str = "-- synthetic test migration\nSELECT 1;\n";
        // Compute the canonical checksum the same way the production
        // code does, plus a deliberately-wrong "bad" value to seed.
        let canonical = Migration::unapplied(&format!("V{TEST_VERSION}__{TEST_NAME}"), TEST_SQL)
            .unwrap()
            .checksum();
        let bad: u64 = canonical.wrapping_add(1);
        // `KnownDivergence` is lifetime-generic, so we can borrow a
        // stack-allocated slice here — no `Box::leak` needed (would
        // otherwise flag under leak sanitizers).
        let bad_checksums = [bad];
        let test_divergences = [KnownDivergence {
            version: TEST_VERSION,
            name: TEST_NAME,
            sql: TEST_SQL,
            known_bad_checksums: &bad_checksums,
            explanation: "test fixture for PR #2101 integration test",
        }];

        // Clean any leftover row from a previous run, then seed the
        // bad checksum.
        client
            .execute(
                "DELETE FROM refinery_schema_history WHERE version = $1",
                &[&TEST_VERSION],
            )
            .await
            .expect("cleanup pre-run");
        client
            .execute(
                "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) \
                 VALUES ($1, $2, $3, $4)",
                &[
                    &TEST_VERSION,
                    &TEST_NAME,
                    &"2026-01-01T00:00:00Z",
                    &bad.to_string(),
                ],
            )
            .await
            .expect("seed bad checksum");

        // Run the realignment with our synthetic divergence list.
        super::realign_diverged_checksums_with(&mut client, &test_divergences)
            .await
            .expect("realign");

        // Assert the row now holds the canonical checksum.
        let row = client
            .query_one(
                "SELECT checksum FROM refinery_schema_history WHERE version = $1",
                &[&TEST_VERSION],
            )
            .await
            .expect("read back row");
        let stored: String = row.get(0);
        assert_eq!(
            stored,
            canonical.to_string(),
            "realign should have rewritten the bad checksum to canonical",
        );

        // Run the realignment a second time — it should be a no-op
        // because the row no longer matches any known-bad value.
        super::realign_diverged_checksums_with(&mut client, &test_divergences)
            .await
            .expect("realign idempotent");
        let row = client
            .query_one(
                "SELECT checksum FROM refinery_schema_history WHERE version = $1",
                &[&TEST_VERSION],
            )
            .await
            .expect("read back row 2");
        let stored: String = row.get(0);
        assert_eq!(stored, canonical.to_string(), "second run should no-op");

        // Cleanup.
        client
            .execute(
                "DELETE FROM refinery_schema_history WHERE version = $1",
                &[&TEST_VERSION],
            )
            .await
            .expect("cleanup post-run");
    }

    /// Regression test for the defensive check that rejects a
    /// `KnownDivergence` whose canonical checksum is also listed in its
    /// own `known_bad_checksums`. Such a misconfiguration would silently
    /// rewrite already-correct rows to themselves; the production code
    /// returns `Err(DatabaseError::Migration(...))` to refuse startup.
    ///
    /// Like the realignment integration test above, this requires a
    /// `PgClient` and so is gated on `feature = "integration"`. The
    /// error fires *before* any UPDATE query, so the test only needs a
    /// reachable database — `refinery_schema_history` does not even
    /// need to exist.
    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn rejects_canonical_in_known_bad_checksums() {
        use deadpool_postgres::{Manager, Pool};
        use tokio_postgres::{Config, NoTls};

        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://localhost/ironclaw_test".to_string());
        let config: Config = match database_url.parse() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skipping: invalid DATABASE_URL ({e})");
                return;
            }
        };
        let mgr = Manager::new(config, NoTls);
        let pool = match Pool::builder(mgr).max_size(2).build() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("skipping: failed to build pool ({e})");
                return;
            }
        };
        let mut client = match pool.get().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skipping: database unavailable ({e})");
                return;
            }
        };
        let _guard = POSTGRES_MIGRATION_FIXUP_TEST_LOCK.lock().await;

        // Make sure refinery_schema_history exists so we exercise the
        // post-history-check branch (the early-return on missing table
        // would otherwise mask the validation we want to test).
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS refinery_schema_history ( \
                    version INT4 PRIMARY KEY, \
                    name VARCHAR(255), \
                    applied_on VARCHAR(255), \
                    checksum VARCHAR(255))",
            )
            .await
            .expect("create refinery_schema_history");

        // Construct a deliberately-misconfigured divergence where the
        // canonical checksum appears in its own known-bad list.
        const TEST_SQL: &str = "-- bad-config test fixture\nSELECT 2;\n";
        let canonical = Migration::unapplied("V99998__bad_config", TEST_SQL)
            .unwrap()
            .checksum();
        // Stack-allocated, no Box::leak — `KnownDivergence` is
        // lifetime-generic.
        let bad_checksums = [canonical];
        let bad_divergences = [KnownDivergence {
            version: 99998,
            name: "bad_config",
            sql: TEST_SQL,
            known_bad_checksums: &bad_checksums,
            explanation: "intentional misconfig for PR #2101 regression test",
        }];

        let result = super::realign_diverged_checksums_with(&mut client, &bad_divergences).await;
        match result {
            Err(crate::error::DatabaseError::Migration(msg)) => {
                assert!(
                    msg.contains("known_bad_checksums"),
                    "expected error to mention known_bad_checksums, got: {msg}",
                );
                assert!(
                    msg.contains("V99998__bad_config"),
                    "expected error to identify the offending migration label, got: {msg}",
                );
            }
            Err(other) => panic!("expected Migration error, got: {other:?}"),
            Ok(()) => {
                panic!("expected Err — canonical checksum in known_bad list should refuse startup")
            }
        }
    }
}
