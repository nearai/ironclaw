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
struct KnownDivergence {
    version: i32,
    name: &'static str,
    /// The current (canonical) SQL content, embedded at compile time.
    sql: &'static str,
    /// The exact set of historical bad checksums we are willing to rewrite
    /// for this migration. **The fix-up only fires when the stored checksum
    /// matches one of these literals** — any other divergence (manual
    /// tampering, hardware corruption, an unknown future regression) is
    /// left alone so refinery can still abort startup loudly.
    known_bad_checksums: &'static [u64],
    /// Human-readable explanation of why this divergence exists, surfaced
    /// in the realignment warning log so future entries are not coupled to
    /// the V6/#1328 wording.
    explanation: &'static str,
}

const KNOWN_DIVERGENCES: &[KnownDivergence] = &[KnownDivergence {
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

/// Realign `refinery_schema_history` rows whose stored checksum disagrees
/// with the canonical checksum of the embedded migration. Must be called
/// before `refinery::Runner::run_async`.
pub async fn realign_diverged_checksums(client: &mut PgClient) -> Result<(), DatabaseError> {
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

    for divergence in KNOWN_DIVERGENCES {
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
        // condition, so failing loudly here is appropriate.
        debug_assert!(
            !divergence
                .known_bad_checksums
                .contains(&migration.checksum()),
            "{migration_label}: canonical checksum is listed as known-bad",
        );

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
            map.insert(key.trim().to_string(), parsed);
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
        sql_files.sort();

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
}
