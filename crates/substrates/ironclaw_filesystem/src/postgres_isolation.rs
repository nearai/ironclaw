//! Per-test isolated PostgreSQL database provisioning (`test-support`).
//!
//! Contract suites whose assertions draw on database-wide state (absolute
//! cursor values, global prune bookkeeping, schema-level triggers) cannot
//! share one database: sibling tests shift the numbers. Each such test
//! provisions a private database on the configured server instead — the
//! isolation the jsonl/libsql twins get from per-test temp files.
//!
//! This module is the single home of that scaffolding. It was first written
//! inline in `ironclaw_filesystem/tests/db_root_filesystem_contract.rs` (the
//! fabric contract's `IsolatedDatabase`), then copied — with the sweep
//! hardening below — into the event-store and product-workflow-ledger
//! suites, at which point the copies were byte-identical but for the name
//! prefix and env var. The sweep's correctness argument depends on the
//! epoch-in-name convention holding at *every* site, so the copies were
//! hoisted here (parameterised by prefix and env var) rather than left to
//! drift. The fabric suite still carries its older per-call variant beside
//! its container-resolution and skip-flag machinery; see the note at its
//! `IsolatedDatabase`.
//!
//! # Sweep hardening (why this differs from the fabric original)
//!
//! - **One stale-database sweep per test binary, ahead of every creation.**
//!   Sweeping per provisioning call can race a sibling test of the same
//!   binary between its `CREATE DATABASE` and first connection; a single
//!   up-front sweep removes that window while still collecting what failed
//!   runs left behind.
//! - **Only databases at least [`STALE_SWEEP_MIN_AGE_SECS`] old are swept.**
//!   A prefix-only sweep could delete a concurrent *process*'s freshly
//!   created database in the window between its `CREATE DATABASE` and its
//!   first connection (zero backends — nothing for a no-`FORCE` drop to
//!   refuse); the epoch embedded in every name keeps anything younger than
//!   the cutoff out of the sweep's `SELECT` entirely, so that window is
//!   unreachable. Leftovers from crashed runs collect on the first run after
//!   the cutoff.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::OnceCell;

/// Only sweep databases at least this old. See the module docs.
const STALE_SWEEP_MIN_AGE_SECS: u64 = 3600;

/// What a provisioner does when the configured server cannot be reached.
#[derive(Clone, Copy, Debug)]
pub enum PostgresUnreachable {
    /// Past a configured URL, every failure is a broken environment rather
    /// than an unconfigured one, so it panics — for suites whose Postgres
    /// leg has no CI executor, where a silent skip would let the suite pass
    /// while testing nothing.
    Panic,
    /// Parse and connect failures skip with a notice — for suites that keep
    /// the reachability-skip semantics of a sibling shared-database helper.
    /// Past a reachable server, provisioning failures still panic.
    Skip,
}

/// One suite's provisioning identity: which env var names the server, what
/// the private databases are called, and how unreachability is reported.
///
/// Declare one `static` per test binary — the once-per-binary sweep and the
/// per-test name counter live on the instance:
///
/// ```ignore
/// static POSTGRES: IsolatedPostgresProvisioner = IsolatedPostgresProvisioner::new(
///     "postgres my-suite contract",
///     "IRONCLAW_MY_SUITE_POSTGRES_URL",
///     "mysuite_isolated_",
///     PostgresUnreachable::Panic,
/// );
/// ```
pub struct IsolatedPostgresProvisioner {
    /// Names the suite in skip notices.
    suite: &'static str,
    env_var: &'static str,
    prefix: &'static str,
    unreachable: PostgresUnreachable,
    stale_sweep: OnceCell<()>,
    next_database: AtomicU64,
}

/// A provisioned private database. Dropping the handles does not drop the
/// database; call [`IsolatedPostgresDatabase::cleanup`] on the way out of a
/// passing test.
pub struct IsolatedPostgresDatabase {
    url: String,
    config: tokio_postgres::Config,
    admin: tokio_postgres::Client,
    name: String,
}

impl IsolatedPostgresProvisioner {
    pub const fn new(
        suite: &'static str,
        env_var: &'static str,
        prefix: &'static str,
        unreachable: PostgresUnreachable,
    ) -> Self {
        Self {
            suite,
            env_var,
            prefix,
            unreachable,
            stale_sweep: OnceCell::const_new(),
            next_database: AtomicU64::new(0),
        }
    }

    /// Provision a fresh private database, sweeping stale leftovers first.
    ///
    /// Returns `None` when the env var is unset (always a skip, with a
    /// notice) or, under [`PostgresUnreachable::Skip`], when the URL does
    /// not parse or the server does not answer.
    pub async fn provision(&self) -> Option<IsolatedPostgresDatabase> {
        let base_url = match std::env::var(self.env_var) {
            Ok(url) => url,
            Err(_) => {
                eprintln!("skipping {}: {} not set", self.suite, self.env_var);
                return None;
            }
        };
        let admin_config = match base_url.parse::<tokio_postgres::Config>() {
            Ok(config) => config,
            Err(error) => match self.unreachable {
                PostgresUnreachable::Panic => {
                    // This module is gated behind `test-support`; its only entry
                    // point is a contract suite's provisioner and no production path
                    // constructs one. Failing loud is the contract: a suite whose
                    // backend is configured but unusable must red, never silently
                    // skip its Postgres leg (the inert-guard rule).
                    panic!(
                        "{} does not parse as a postgres connection string: {error}",
                        self.env_var
                    ) // safety: test-only provisioning; a misconfigured suite must fail loud
                }
                PostgresUnreachable::Skip => {
                    eprintln!("skipping {}: invalid url ({error})", self.suite);
                    return None;
                }
            },
        };
        let (admin, connection) = match admin_config.connect(tokio_postgres::NoTls).await {
            Ok(connected) => connected,
            Err(error) => match self.unreachable {
                PostgresUnreachable::Panic => {
                    panic!("connect to the configured postgres server: {error}") // safety: test-only provisioning; a configured-but-unreachable server must fail the suite, never degrade it to a skip
                }
                PostgresUnreachable::Skip => {
                    eprintln!("skipping {}: database unavailable ({error})", self.suite);
                    return None;
                }
            },
        };
        tokio::spawn(async move {
            let _ = connection.await;
        });
        self.stale_sweep
            .get_or_init(|| async {
                // Collect databases previously failed runs unwound past. Age-
                // gated by the epoch in each name so a concurrent run's fresh
                // database is never selected — even inside its zero-backend
                // window between `CREATE DATABASE` and first connection,
                // which a backend-count check (or the no-`FORCE` drop below)
                // cannot protect. No `FORCE`: anything old but still held
                // open by a live run additionally refuses to drop.
                if let Ok(stale) = admin
                    .query(
                        &format!(
                            "SELECT datname FROM pg_database WHERE datname LIKE '{}%'",
                            self.prefix
                        ),
                        &[],
                    )
                    .await
                {
                    let now = unix_epoch_secs();
                    for row in stale {
                        let name = row.get::<_, String>(0);
                        let stale_enough =
                            isolated_database_epoch(&name, self.prefix).is_none_or(|epoch| {
                                now.saturating_sub(epoch) > STALE_SWEEP_MIN_AGE_SECS
                            });
                        if !stale_enough {
                            continue;
                        }
                        let _ = admin
                            .execute(&format!("DROP DATABASE IF EXISTS {name}"), &[])
                            .await;
                    }
                }
            })
            .await;
        // Identifiers cannot be bind parameters in DDL. The interpolations
        // are compile-time prefixes and process-generated suffixes (epoch +
        // pid + counter) or come from `pg_database`, never caller input.
        let name = format!(
            "{}{}_{}_{}",
            self.prefix,
            unix_epoch_secs(),
            std::process::id(),
            self.next_database.fetch_add(1, Ordering::Relaxed)
        );
        admin
            .execute(&format!("CREATE DATABASE {name}"), &[])
            .await
            .expect("create the isolated database (the role needs CREATEDB)"); // safety: test-only provisioning; without CREATEDB there is no per-test isolation, which is exactly what the absolute-cursor asserts depend on
        let mut config = admin_config;
        config.dbname(&name);
        Some(IsolatedPostgresDatabase {
            url: connection_string_with_dbname(&base_url, &name),
            config,
            admin,
            name,
        })
    }
}

impl IsolatedPostgresDatabase {
    /// Connection string for the private database, in the same libpq form
    /// the configured URL used — for stores that take the raw string.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Parsed connection config with `dbname` already pointed at the
    /// private database — for callers that build their own pool.
    pub fn config(&self) -> &tokio_postgres::Config {
        &self.config
    }

    /// Drop the database on the way out of a passing test.
    ///
    /// A courtesy, not a guarantee: a failing assertion unwinds straight
    /// past it, so a red run can leave its database behind — that is what
    /// the provisioner's sweep collects on the next run. `FORCE` closes
    /// store-pool connections that have not gone away by the time the
    /// handles drop.
    pub async fn cleanup(self) {
        let Self { admin, name, .. } = self;
        let _ = admin
            .execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"), &[])
            .await;
    }
}

fn unix_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the UNIX epoch") // safety: test-only provisioning; a pre-1970 clock would make the age-gated stale-database sweep meaningless
        .as_secs()
}

/// Parses the creation epoch out of `<prefix><epoch>_<pid>_<counter>`.
/// `None` marks a pre-timestamp legacy name: definitionally past the cutoff,
/// collected immediately.
fn isolated_database_epoch(name: &str, prefix: &str) -> Option<u64> {
    name.strip_prefix(prefix)?
        .split('_')
        .next()?
        .parse::<u64>()
        .ok()
}

/// Rewrites the database name inside a libpq connection string, preserving
/// every other component. `tokio_postgres::Config` parses both libpq forms
/// but cannot serialise back, and raw-string consumers (the event-store
/// builder) take the string, so the rewrite happens at the string level:
/// - URL form (`postgres://…`): replace the path segment, keep any query.
/// - Key-value form: append `dbname=…` — `tokio_postgres` applies keys in
///   order, so the appended one wins.
fn connection_string_with_dbname(base: &str, dbname: &str) -> String {
    match base.find("://") {
        Some(scheme_idx) => {
            let after_scheme = scheme_idx + "://".len();
            let (without_query, query) = match base[after_scheme..].find('?') {
                Some(offset) => base.split_at(after_scheme + offset),
                None => (base, ""),
            };
            let authority_end = without_query[after_scheme..]
                .find('/')
                .map(|offset| after_scheme + offset)
                .unwrap_or(without_query.len());
            format!("{}/{dbname}{query}", &without_query[..authority_end])
        }
        None => format!("{base} dbname={dbname}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{connection_string_with_dbname, isolated_database_epoch};

    #[test]
    fn url_form_replaces_the_path_and_keeps_the_query() {
        assert_eq!(
            connection_string_with_dbname(
                "postgres://user:pw@host:5432/postgres?sslmode=disable",
                "iso_1"
            ),
            "postgres://user:pw@host:5432/iso_1?sslmode=disable"
        );
    }

    #[test]
    fn url_form_without_a_path_gains_one() {
        assert_eq!(
            connection_string_with_dbname("postgres://host:5432", "iso_1"),
            "postgres://host:5432/iso_1"
        );
    }

    #[test]
    fn key_value_form_appends_dbname_so_the_last_key_wins() {
        assert_eq!(
            connection_string_with_dbname("host=localhost user=postgres dbname=postgres", "iso_1"),
            "host=localhost user=postgres dbname=postgres dbname=iso_1"
        );
    }

    #[test]
    fn epoch_parses_out_of_the_canonical_name_shape() {
        assert_eq!(
            isolated_database_epoch("pref_1754400000_42_7", "pref_"),
            Some(1_754_400_000)
        );
    }

    #[test]
    fn legacy_names_without_an_epoch_read_as_immediately_stale() {
        assert_eq!(isolated_database_epoch("pref_abcdef", "pref_"), None);
        assert_eq!(isolated_database_epoch("other_123", "pref_"), None);
    }
}
