use std::collections::HashMap;

use chrono::Utc;
use deadpool_postgres::Pool;
use ironclaw_host_api::{ReservationStatus, ResourceReservationId, ResourceScope};
use serde_json::Value;

use crate::cas_snapshot::{AsyncStorageWorkerPoolCell, new_worker_pool_cell, run_on_worker_pool};
use crate::{
    AccountSnapshot, BudgetEvent, BudgetPeriod, Clock, NoOpBudgetEventSink, ReservationOutcome,
    ReservationRecord, ResourceAccount, ResourceError, ResourceGovernor, ResourceLimits,
    ResourceReceipt, ResourceState, ResourceTally, SystemClock, account_snapshot_in_state,
    emit_reserve_events, most_specific_account, reconcile_in_state, release_in_state,
    reserve_with_outcome_in_state, set_limit_in_state,
};
use crate::{BudgetEventSink, ResourceEstimate, ResourceUsage};
use std::sync::Arc;

const ACCOUNT_TABLE: &str = "ironclaw_resource_accounts";
const RESERVATION_TABLE: &str = "ironclaw_resource_reservations";

#[derive(Clone)]
pub struct PostgresResourceGovernor {
    pool: Pool,
    clock: Arc<dyn Clock>,
    event_sink: Arc<dyn BudgetEventSink>,
    workers: AsyncStorageWorkerPoolCell,
    worker_count: usize,
}

#[derive(Debug)]
struct AccountRow {
    account: ResourceAccount,
    limits: Option<ResourceLimits>,
    reserved: ResourceTally,
    spent: ResourceTally,
    period_end: Option<chrono::DateTime<Utc>>,
}

impl PostgresResourceGovernor {
    pub fn new(pool: Pool) -> Self {
        let worker_count = pool.status().max_size.max(1);
        Self {
            pool,
            clock: Arc::new(SystemClock),
            event_sink: Arc::new(NoOpBudgetEventSink),
            workers: new_worker_pool_cell(),
            worker_count,
        }
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_event_sink(mut self, sink: Arc<dyn BudgetEventSink>) -> Self {
        self.event_sink = sink;
        self
    }

    pub fn run_migrations(&self) -> Result<(), ResourceError> {
        let pool = self.pool.clone();
        run_on_worker_pool(
            &self.workers,
            "resource-governor-postgres",
            self.worker_count,
            move || async move {
                let client = connect(&pool).await?;
                client
                    .batch_execute(
                        r#"
                    CREATE TABLE IF NOT EXISTS ironclaw_resource_accounts (
                        account_key TEXT PRIMARY KEY,
                        account JSONB NOT NULL,
                        limits JSONB,
                        reserved JSONB NOT NULL,
                        spent JSONB NOT NULL,
                        period_end TEXT,
                        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                    );

                    CREATE TABLE IF NOT EXISTS ironclaw_resource_reservations (
                        reservation_id TEXT PRIMARY KEY,
                        record JSONB NOT NULL,
                        status TEXT NOT NULL,
                        account_keys TEXT[] NOT NULL DEFAULT '{}',
                        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                    );

                    ALTER TABLE ironclaw_resource_reservations
                        ADD COLUMN IF NOT EXISTS account_keys TEXT[] NOT NULL DEFAULT '{}';

                    CREATE INDEX IF NOT EXISTS ironclaw_resource_reservations_status_idx
                        ON ironclaw_resource_reservations (status);
                    CREATE INDEX IF NOT EXISTS ironclaw_resource_reservations_account_keys_idx
                        ON ironclaw_resource_reservations USING GIN (account_keys);
                    "#,
                    )
                    .await
                    .map_err(|error| {
                        storage_error(format!("migrate postgres resource governor: {error}"))
                    })?;
                Ok(())
            },
        )
    }

    fn run<T, Fut, F>(&self, build: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        Fut: std::future::Future<Output = Result<T, ResourceError>> + Send + 'static,
        F: FnOnce(Pool) -> Fut + Send + 'static,
    {
        let pool = self.pool.clone();
        run_on_worker_pool(
            &self.workers,
            "resource-governor-postgres",
            self.worker_count,
            move || build(pool),
        )
    }
}

impl ResourceGovernor for PostgresResourceGovernor {
    fn set_limit(
        &self,
        account: ResourceAccount,
        limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        let now = self.clock.now();
        let account_for_event = account.clone();
        let result = self.run(move |pool| async move {
            let mut client = connect(&pool).await?;
            let tx = client
                .transaction()
                .await
                .map_err(|error| storage_error(format!("begin set limit: {error}")))?;
            lock_account_key_exclusive(&tx, &account).await?;
            let existing_row = read_account_row_tx(&tx, &account).await?;
            let rebuild_from_reservations = existing_row
                .as_ref()
                .is_none_or(|row| !account_row_has_finite_limits(row));
            ensure_account_rows(&tx, std::slice::from_ref(&account)).await?;
            let rows = lock_account_rows(&tx, std::slice::from_ref(&account)).await?;
            let mut state = state_from_rows(rows, HashMap::new());
            set_limit_in_state(&mut state, account.clone(), limits, now);
            if rebuild_from_reservations {
                rebuild_account_tallies_from_reservations(&tx, &account, &mut state).await?;
            }
            write_accounts_for_state(&tx, &[account], &state).await?;
            tx.commit()
                .await
                .map_err(|error| storage_error(format!("commit set limit: {error}")))?;
            Ok(())
        });
        if result.is_ok() {
            self.event_sink.emit(BudgetEvent::LimitChanged {
                account: account_for_event,
                at: now,
            });
        }
        result
    }

    fn reserve_with_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ReservationOutcome, ResourceError> {
        self.reserve_with_id_and_outcome(scope, estimate, ResourceReservationId::new())
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ReservationOutcome, ResourceError> {
        let now = self.clock.now();
        let result = self.run(move |pool| async move {
            let accounts = ResourceAccount::cascade(&scope);
            let mut client = connect(&pool).await?;
            let tx = client
                .transaction()
                .await
                .map_err(|error| storage_error(format!("begin reserve: {error}")))?;
            lock_account_keys_shared(&tx, &accounts).await?;
            let existing_rows = read_account_rows_tx(&tx, &accounts).await?;
            if !account_rows_have_finite_limits(&existing_rows) {
                if reservation_exists(&tx, reservation_id).await? {
                    return Err(ResourceError::ReservationAlreadyExists { id: reservation_id });
                }
                let mut state = state_from_rows(existing_rows, HashMap::new());
                let outcome = reserve_with_outcome_in_state(
                    &mut state,
                    scope,
                    estimate,
                    reservation_id,
                    now,
                )?;
                let record = state
                    .reservations
                    .get(&reservation_id)
                    .cloned()
                    .ok_or_else(|| storage_error("reserve did not produce reservation record"))?;
                insert_reservation(&tx, reservation_id, &record).await?;
                tx.commit()
                    .await
                    .map_err(|error| storage_error(format!("commit reserve: {error}")))?;
                return Ok(outcome);
            }
            ensure_account_rows(&tx, &accounts).await?;
            let rows = lock_account_rows(&tx, &accounts).await?;
            if reservation_exists(&tx, reservation_id).await? {
                return Err(ResourceError::ReservationAlreadyExists { id: reservation_id });
            }
            let mut state = state_from_rows(rows, HashMap::new());
            let outcome =
                reserve_with_outcome_in_state(&mut state, scope, estimate, reservation_id, now)?;
            write_accounts_for_state(&tx, &accounts, &state).await?;
            let record = state
                .reservations
                .get(&reservation_id)
                .cloned()
                .ok_or_else(|| storage_error("reserve did not produce reservation record"))?;
            insert_reservation(&tx, reservation_id, &record).await?;
            tx.commit()
                .await
                .map_err(|error| storage_error(format!("commit reserve: {error}")))?;
            Ok(outcome)
        });
        emit_reserve_events(self.event_sink.as_ref(), &result, now);
        result
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        let now = self.clock.now();
        let result = self.run(move |pool| async move {
            let mut client = connect(&pool).await?;
            let tx = client
                .transaction()
                .await
                .map_err(|error| storage_error(format!("begin reconcile: {error}")))?;
            let record = lock_reservation(&tx, reservation_id).await?;
            let accounts = record.accounts.clone();
            lock_account_keys_shared(&tx, &accounts).await?;
            let existing_rows = read_account_rows_tx(&tx, &accounts).await?;
            if !account_rows_have_finite_limits(&existing_rows) {
                let mut reservations = HashMap::new();
                reservations.insert(reservation_id, record);
                let mut state = state_from_rows(existing_rows, reservations);
                let receipt = reconcile_in_state(&mut state, reservation_id, actual, now)?;
                let record = state
                    .reservations
                    .get(&reservation_id)
                    .cloned()
                    .ok_or_else(|| storage_error("reconcile removed reservation record"))?;
                write_reservation(&tx, reservation_id, &record).await?;
                tx.commit()
                    .await
                    .map_err(|error| storage_error(format!("commit reconcile: {error}")))?;
                return Ok(receipt);
            }
            ensure_account_rows(&tx, &accounts).await?;
            let rows = lock_account_rows(&tx, &accounts).await?;
            let mut reservations = HashMap::new();
            reservations.insert(reservation_id, record);
            let mut state = state_from_rows(rows, reservations);
            let receipt = reconcile_in_state(&mut state, reservation_id, actual, now)?;
            write_accounts_for_state(&tx, &accounts, &state).await?;
            let record = state
                .reservations
                .get(&reservation_id)
                .cloned()
                .ok_or_else(|| storage_error("reconcile removed reservation record"))?;
            write_reservation(&tx, reservation_id, &record).await?;
            tx.commit()
                .await
                .map_err(|error| storage_error(format!("commit reconcile: {error}")))?;
            Ok(receipt)
        });
        if let Ok(receipt) = &result {
            self.event_sink.emit(BudgetEvent::Reconciled {
                account: most_specific_account(&receipt.scope),
                receipt: receipt.clone(),
                at: now,
            });
        }
        result
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        let now = self.clock.now();
        let result = self.run(move |pool| async move {
            let mut client = connect(&pool).await?;
            let tx = client
                .transaction()
                .await
                .map_err(|error| storage_error(format!("begin release: {error}")))?;
            let record = lock_reservation(&tx, reservation_id).await?;
            let accounts = record.accounts.clone();
            lock_account_keys_shared(&tx, &accounts).await?;
            let existing_rows = read_account_rows_tx(&tx, &accounts).await?;
            if !account_rows_have_finite_limits(&existing_rows) {
                let mut reservations = HashMap::new();
                reservations.insert(reservation_id, record);
                let mut state = state_from_rows(existing_rows, reservations);
                let receipt = release_in_state(&mut state, reservation_id, now)?;
                let record = state
                    .reservations
                    .get(&reservation_id)
                    .cloned()
                    .ok_or_else(|| storage_error("release removed reservation record"))?;
                write_reservation(&tx, reservation_id, &record).await?;
                tx.commit()
                    .await
                    .map_err(|error| storage_error(format!("commit release: {error}")))?;
                return Ok(receipt);
            }
            ensure_account_rows(&tx, &accounts).await?;
            let rows = lock_account_rows(&tx, &accounts).await?;
            let mut reservations = HashMap::new();
            reservations.insert(reservation_id, record);
            let mut state = state_from_rows(rows, reservations);
            let receipt = release_in_state(&mut state, reservation_id, now)?;
            write_accounts_for_state(&tx, &accounts, &state).await?;
            let record = state
                .reservations
                .get(&reservation_id)
                .cloned()
                .ok_or_else(|| storage_error("release removed reservation record"))?;
            write_reservation(&tx, reservation_id, &record).await?;
            tx.commit()
                .await
                .map_err(|error| storage_error(format!("commit release: {error}")))?;
            Ok(receipt)
        });
        if let Ok(receipt) = &result {
            self.event_sink.emit(BudgetEvent::Released {
                account: most_specific_account(&receipt.scope),
                receipt: receipt.clone(),
                at: now,
            });
        }
        result
    }

    fn account_snapshot(
        &self,
        account: &ResourceAccount,
    ) -> Result<Option<AccountSnapshot>, ResourceError> {
        let account = account.clone();
        let now = self.clock.now();
        self.run(move |pool| async move {
            let client = connect(&pool).await?;
            let row = read_account_row(&client, &account).await?;
            let reservation_tallies = if row
                .as_ref()
                .is_none_or(|row| !account_row_has_finite_limits(row))
            {
                Some(account_tallies_from_reservations_client(&client, &account).await?)
            } else {
                None
            };
            let mut rows = HashMap::new();
            match (row, reservation_tallies) {
                (Some(mut row), Some(tallies)) => {
                    row.reserved = tallies.reserved;
                    row.spent = tallies.spent;
                    rows.insert(account_key(&account), row);
                }
                (None, Some(tallies))
                    if tallies.reserved != ResourceTally::default()
                        || tallies.spent != ResourceTally::default() =>
                {
                    rows.insert(
                        account_key(&account),
                        AccountRow {
                            account: account.clone(),
                            limits: None,
                            reserved: tallies.reserved,
                            spent: tallies.spent,
                            period_end: None,
                        },
                    );
                }
                (Some(row), None) => {
                    rows.insert(account_key(&account), row);
                }
                (None, _) => {}
            }
            let mut state = state_from_rows(rows, HashMap::new());
            Ok(account_snapshot_in_state(&mut state, &account, now))
        })
    }
}

async fn lock_account_keys_shared(
    tx: &tokio_postgres::Transaction<'_>,
    accounts: &[ResourceAccount],
) -> Result<(), ResourceError> {
    let mut keys = accounts.iter().map(account_key).collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    for key in keys {
        tx.query_one(
            "SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))",
            &[&key],
        )
        .await
        .map_err(|error| storage_error(format!("lock shared account key: {error}")))?;
    }
    Ok(())
}

async fn lock_account_key_exclusive(
    tx: &tokio_postgres::Transaction<'_>,
    account: &ResourceAccount,
) -> Result<(), ResourceError> {
    let key = account_key(account);
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
        &[&key],
    )
    .await
    .map_err(|error| storage_error(format!("lock exclusive account key: {error}")))?;
    Ok(())
}

async fn connect(pool: &Pool) -> Result<deadpool_postgres::Object, ResourceError> {
    pool.get().await.map_err(|error| {
        storage_error(format!("postgres resource governor pool checkout: {error}"))
    })
}

async fn ensure_account_rows(
    tx: &tokio_postgres::Transaction<'_>,
    accounts: &[ResourceAccount],
) -> Result<(), ResourceError> {
    for account in accounts {
        let key = account_key(account);
        let account_json = serde_json::to_value(account).map_err(storage_error)?;
        let reserved = serde_json::to_value(ResourceTally::default()).map_err(storage_error)?;
        let spent = serde_json::to_value(ResourceTally::default()).map_err(storage_error)?;
        tx.execute(
            &format!(
                "INSERT INTO {ACCOUNT_TABLE}
                    (account_key, account, reserved, spent)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (account_key) DO NOTHING"
            ),
            &[&key, &account_json, &reserved, &spent],
        )
        .await
        .map_err(|error| storage_error(format!("ensure account row: {error}")))?;
    }
    Ok(())
}

async fn read_account_rows_tx(
    tx: &tokio_postgres::Transaction<'_>,
    accounts: &[ResourceAccount],
) -> Result<HashMap<String, AccountRow>, ResourceError> {
    let mut rows = HashMap::new();
    for account in accounts {
        if let Some(row) = read_account_row_tx(tx, account).await? {
            rows.insert(account_key(account), row);
        }
    }
    Ok(rows)
}

async fn read_account_row_tx(
    tx: &tokio_postgres::Transaction<'_>,
    account: &ResourceAccount,
) -> Result<Option<AccountRow>, ResourceError> {
    let key = account_key(account);
    let row = tx
        .query_opt(
            &format!(
                "SELECT account, limits, reserved, spent, period_end
                 FROM {ACCOUNT_TABLE}
                 WHERE account_key = $1"
            ),
            &[&key],
        )
        .await
        .map_err(|error| storage_error(format!("read account row: {error}")))?;
    row.map(decode_account_row).transpose()
}

async fn lock_account_rows(
    tx: &tokio_postgres::Transaction<'_>,
    accounts: &[ResourceAccount],
) -> Result<HashMap<String, AccountRow>, ResourceError> {
    let mut rows = HashMap::new();
    for account in accounts {
        let key = account_key(account);
        let row = tx
            .query_one(
                &format!(
                    "SELECT account, limits, reserved, spent, period_end
                     FROM {ACCOUNT_TABLE}
                     WHERE account_key = $1
                     FOR UPDATE"
                ),
                &[&key],
            )
            .await
            .map_err(|error| storage_error(format!("lock account row: {error}")))?;
        rows.insert(key, decode_account_row(row)?);
    }
    Ok(rows)
}

async fn read_account_row(
    client: &deadpool_postgres::Object,
    account: &ResourceAccount,
) -> Result<Option<AccountRow>, ResourceError> {
    let key = account_key(account);
    let row = client
        .query_opt(
            &format!(
                "SELECT account, limits, reserved, spent, period_end
                 FROM {ACCOUNT_TABLE}
                 WHERE account_key = $1"
            ),
            &[&key],
        )
        .await
        .map_err(|error| storage_error(format!("read account row: {error}")))?;
    row.map(decode_account_row).transpose()
}

fn account_rows_have_finite_limits(rows: &HashMap<String, AccountRow>) -> bool {
    rows.values().any(account_row_has_finite_limits)
}

fn account_row_has_finite_limits(row: &AccountRow) -> bool {
    row.limits
        .as_ref()
        .is_some_and(|limits| !limits.is_unlimited())
}

fn decode_account_row(row: tokio_postgres::Row) -> Result<AccountRow, ResourceError> {
    let account: Value = row.get("account");
    let limits: Option<Value> = row.get("limits");
    let reserved: Value = row.get("reserved");
    let spent: Value = row.get("spent");
    let period_end: Option<String> = row.get("period_end");
    let limits: Option<ResourceLimits> = limits
        .map(serde_json::from_value)
        .transpose()
        .map_err(storage_error)?;
    Ok(AccountRow {
        account: serde_json::from_value(account).map_err(storage_error)?,
        limits: limits.clone(),
        reserved: serde_json::from_value(reserved).map_err(storage_error)?,
        spent: serde_json::from_value(spent).map_err(storage_error)?,
        period_end: if matches!(
            limits.as_ref().map(|limits| &limits.period),
            Some(BudgetPeriod::PerInvocation)
        ) {
            None
        } else {
            period_end
                .map(|value| {
                    chrono::DateTime::parse_from_rfc3339(&value)
                        .map(|value| value.with_timezone(&Utc))
                })
                .transpose()
                .map_err(storage_error)?
        },
    })
}

fn state_from_rows(
    rows: HashMap<String, AccountRow>,
    reservations: HashMap<ResourceReservationId, ReservationRecord>,
) -> ResourceState {
    let mut state = ResourceState {
        reservations,
        ..ResourceState::default()
    };
    for row in rows.into_values() {
        if let Some(limits) = row.limits {
            state.limits.insert(row.account.clone(), limits);
        }
        if row.reserved != ResourceTally::default() {
            state
                .reserved_by_account
                .insert(row.account.clone(), row.reserved);
        }
        if row.spent != ResourceTally::default() {
            state
                .usage_by_account
                .insert(row.account.clone(), row.spent);
        }
        if let Some(period_end) = row.period_end {
            state.period_anchors.insert(row.account, period_end);
        }
    }
    state
}

#[derive(Default)]
struct AccountTallies {
    reserved: ResourceTally,
    spent: ResourceTally,
}

async fn rebuild_account_tallies_from_reservations(
    tx: &tokio_postgres::Transaction<'_>,
    account: &ResourceAccount,
    state: &mut ResourceState,
) -> Result<(), ResourceError> {
    let tallies = account_tallies_from_reservations_tx(tx, account).await?;
    if tallies.reserved == ResourceTally::default() {
        state.reserved_by_account.remove(account);
    } else {
        state
            .reserved_by_account
            .insert(account.clone(), tallies.reserved);
    }
    if tallies.spent == ResourceTally::default() {
        state.usage_by_account.remove(account);
    } else {
        state
            .usage_by_account
            .insert(account.clone(), tallies.spent);
    }
    Ok(())
}

async fn account_tallies_from_reservations_tx(
    tx: &tokio_postgres::Transaction<'_>,
    account: &ResourceAccount,
) -> Result<AccountTallies, ResourceError> {
    let key = account_key(account);
    let rows = tx
        .query(
            &format!("SELECT record FROM {RESERVATION_TABLE} WHERE account_keys @> ARRAY[$1]"),
            &[&key],
        )
        .await
        .map_err(|error| storage_error(format!("read reservation rows: {error}")))?;
    account_tallies_from_reservation_rows(rows, account)
}

async fn account_tallies_from_reservations_client(
    client: &deadpool_postgres::Object,
    account: &ResourceAccount,
) -> Result<AccountTallies, ResourceError> {
    let key = account_key(account);
    let rows = client
        .query(
            &format!("SELECT record FROM {RESERVATION_TABLE} WHERE account_keys @> ARRAY[$1]"),
            &[&key],
        )
        .await
        .map_err(|error| storage_error(format!("read reservation rows: {error}")))?;
    account_tallies_from_reservation_rows(rows, account)
}

fn account_tallies_from_reservation_rows(
    rows: Vec<tokio_postgres::Row>,
    account: &ResourceAccount,
) -> Result<AccountTallies, ResourceError> {
    let mut tallies = AccountTallies::default();
    for row in rows {
        let record: Value = row.get("record");
        let record: ReservationRecord = serde_json::from_value(record).map_err(storage_error)?;
        if !record.accounts.iter().any(|candidate| candidate == account) {
            continue;
        }
        match record.status {
            ReservationStatus::Active => tallies.reserved.add_assign(&record.tally),
            ReservationStatus::Reconciled => {
                if let Some(actual) = &record.actual {
                    tallies.spent.add_assign(&ResourceTally::from_usage(actual));
                }
            }
            ReservationStatus::Released => {}
        }
    }
    Ok(tallies)
}

async fn write_accounts_for_state(
    tx: &tokio_postgres::Transaction<'_>,
    accounts: &[ResourceAccount],
    state: &ResourceState,
) -> Result<(), ResourceError> {
    for account in accounts {
        let key = account_key(account);
        let account_json = serde_json::to_value(account).map_err(storage_error)?;
        let limits = state
            .limits
            .get(account)
            .map(serde_json::to_value)
            .transpose()
            .map_err(storage_error)?;
        let reserved = serde_json::to_value(
            state
                .reserved_by_account
                .get(account)
                .cloned()
                .unwrap_or_default(),
        )
        .map_err(storage_error)?;
        let spent = serde_json::to_value(
            state
                .usage_by_account
                .get(account)
                .cloned()
                .unwrap_or_default(),
        )
        .map_err(storage_error)?;
        let period_end = match state.limits.get(account).map(|limits| &limits.period) {
            Some(BudgetPeriod::PerInvocation) => None,
            _ => state
                .period_anchors
                .get(account)
                .map(|value| value.to_rfc3339()),
        };
        tx.execute(
            &format!(
                "INSERT INTO {ACCOUNT_TABLE}
                    (account_key, account, limits, reserved, spent, period_end)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (account_key) DO UPDATE SET
                    account = EXCLUDED.account,
                    limits = EXCLUDED.limits,
                    reserved = EXCLUDED.reserved,
                    spent = EXCLUDED.spent,
                    period_end = EXCLUDED.period_end,
                    updated_at = NOW()"
            ),
            &[&key, &account_json, &limits, &reserved, &spent, &period_end],
        )
        .await
        .map_err(|error| storage_error(format!("write account row: {error}")))?;
    }
    Ok(())
}

async fn reservation_exists(
    tx: &tokio_postgres::Transaction<'_>,
    reservation_id: ResourceReservationId,
) -> Result<bool, ResourceError> {
    let row = tx
        .query_opt(
            &format!("SELECT 1 FROM {RESERVATION_TABLE} WHERE reservation_id = $1"),
            &[&reservation_id.to_string()],
        )
        .await
        .map_err(|error| storage_error(format!("check reservation row: {error}")))?;
    Ok(row.is_some())
}

async fn lock_reservation(
    tx: &tokio_postgres::Transaction<'_>,
    reservation_id: ResourceReservationId,
) -> Result<ReservationRecord, ResourceError> {
    let row = tx
        .query_opt(
            &format!(
                "SELECT record FROM {RESERVATION_TABLE}
                 WHERE reservation_id = $1
                 FOR UPDATE"
            ),
            &[&reservation_id.to_string()],
        )
        .await
        .map_err(|error| storage_error(format!("lock reservation row: {error}")))?;
    let Some(row) = row else {
        return Err(ResourceError::UnknownReservation { id: reservation_id });
    };
    let record: Value = row.get("record");
    serde_json::from_value(record).map_err(storage_error)
}

async fn write_reservation(
    tx: &tokio_postgres::Transaction<'_>,
    reservation_id: ResourceReservationId,
    record: &ReservationRecord,
) -> Result<(), ResourceError> {
    let record_json = serde_json::to_value(record).map_err(storage_error)?;
    let account_keys = record.accounts.iter().map(account_key).collect::<Vec<_>>();
    tx.execute(
        &format!(
            "INSERT INTO {RESERVATION_TABLE}
                (reservation_id, record, status, account_keys)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (reservation_id) DO UPDATE SET
                record = EXCLUDED.record,
                status = EXCLUDED.status,
                account_keys = EXCLUDED.account_keys,
                updated_at = NOW()"
        ),
        &[
            &reservation_id.to_string(),
            &record_json,
            &reservation_status_text(record.status),
            &account_keys,
        ],
    )
    .await
    .map_err(|error| storage_error(format!("write reservation row: {error}")))?;
    Ok(())
}

async fn insert_reservation(
    tx: &tokio_postgres::Transaction<'_>,
    reservation_id: ResourceReservationId,
    record: &ReservationRecord,
) -> Result<(), ResourceError> {
    let record_json = serde_json::to_value(record).map_err(storage_error)?;
    let account_keys = record.accounts.iter().map(account_key).collect::<Vec<_>>();
    let inserted = tx
        .execute(
            &format!(
                "INSERT INTO {RESERVATION_TABLE}
                    (reservation_id, record, status, account_keys)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (reservation_id) DO NOTHING"
            ),
            &[
                &reservation_id.to_string(),
                &record_json,
                &reservation_status_text(record.status),
                &account_keys,
            ],
        )
        .await
        .map_err(|error| storage_error(format!("insert reservation row: {error}")))?;
    if inserted == 0 {
        return Err(ResourceError::ReservationAlreadyExists { id: reservation_id });
    }
    Ok(())
}

fn account_key(account: &ResourceAccount) -> String {
    account.to_string()
}

fn reservation_status_text(status: ReservationStatus) -> &'static str {
    match status {
        ReservationStatus::Active => "active",
        ReservationStatus::Reconciled => "reconciled",
        ReservationStatus::Released => "released",
    }
}

fn storage_error(error: impl std::fmt::Display) -> ResourceError {
    ResourceError::Storage {
        reason: error.to_string(),
    }
}
