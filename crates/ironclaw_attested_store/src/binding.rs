//! Durable [`AttestedGateBindingStore`] backends with a write-through cache.
//!
//! The reborn resume port reads the authoritative binding **synchronously**
//! (inside the turn store's sync critical section, see
//! [`ironclaw_attested_runtime::SyncBindingRead`]). A durable store therefore
//! cannot block on DB I/O for that read. Each durable backend keeps an
//! in-memory cache that is:
//!
//! * hydrated from the table on construction (`load`), so bindings survive a
//!   restart, and
//! * write-through on every [`AttestedGateBindingStore::put`].
//!
//! The DB row is the source of truth; the cache is the sync read path. Bindings
//! are stored as a single JSON column and rows are never deleted. A binding is
//! **immutable once written**: the upsert is insert-only per `gate_ref`
//! (`ON CONFLICT DO NOTHING`), so a re-`put` after approval is rejected at the
//! DB level rather than silently changing the binding the resume path verifies
//! against. (If versioned overwrite is ever needed it must carry explicit
//! version/audit metadata; the default is reject.)

#[cfg(any(feature = "postgres", feature = "libsql"))]
use std::collections::HashMap;
#[cfg(any(feature = "postgres", feature = "libsql"))]
use std::sync::Mutex;

#[cfg(any(feature = "postgres", feature = "libsql"))]
use async_trait::async_trait;
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw_attested_runtime::{
    AttestedGateBinding, AttestedGateBindingStore, BindingError, BindingKey, SyncBindingRead,
    validate_binding_key,
};
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw_signing_provider::{GateRef, TenantId};

#[cfg(any(feature = "postgres", feature = "libsql"))]
use crate::error::StoreError;

// The primary key is the tenant-qualified `(tenant, gate_ref)` pair, mirroring
// the grant + ledger tenant-first keying. A binding can never collide across
// tenants, and the insert-only CAS is per `(tenant, gate_ref)`.
#[cfg(any(feature = "postgres", feature = "libsql"))]
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS attested_gate_bindings (
    tenant       TEXT NOT NULL,
    gate_ref     TEXT NOT NULL,
    binding_json TEXT NOT NULL,
    PRIMARY KEY (tenant, gate_ref)
);";

/// The write-through cache shared by both backends.
///
/// The primary map is keyed by the tenant-qualified [`BindingKey`] for the async
/// `get`/`put`; a secondary `gate_ref -> BindingKey` index serves the sync resume
/// read, which carries no tenant.
#[cfg(any(feature = "postgres", feature = "libsql"))]
#[derive(Default)]
struct BindingCache {
    inner: Mutex<HashMap<BindingKey, AttestedGateBinding>>,
    by_gate_ref: Mutex<HashMap<GateRef, BindingKey>>,
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
impl BindingCache {
    fn insert(&self, key: BindingKey, binding: AttestedGateBinding) {
        if let Ok(mut index) = self.by_gate_ref.lock() {
            index.insert(key.gate_ref.clone(), key.clone());
        }
        if let Ok(mut map) = self.inner.lock() {
            map.insert(key, binding);
        }
    }

    /// Tenant-qualified read for the async path.
    fn get(&self, key: &BindingKey) -> Option<AttestedGateBinding> {
        match self.inner.lock() {
            Ok(map) => map.get(key).cloned(),
            Err(_) => {
                tracing::error!("binding cache lock poisoned");
                None
            }
        }
    }

    /// gate_ref-only read for the sync resume path: resolve the gate_ref to its
    /// owning [`BindingKey`] via the index, then read.
    fn get_by_gate_ref(&self, gate_ref: &GateRef) -> Option<AttestedGateBinding> {
        let key = match self.by_gate_ref.lock() {
            Ok(index) => index.get(gate_ref).cloned()?,
            Err(_) => {
                tracing::error!("binding cache gate_ref index lock poisoned");
                return None;
            }
        };
        self.get(&key)
    }
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    use deadpool_postgres::Pool;

    /// Durable PostgreSQL [`AttestedGateBindingStore`] with a write-through cache.
    pub struct PostgresAttestedGateBindingStore {
        pool: Pool,
        cache: BindingCache,
    }

    impl PostgresAttestedGateBindingStore {
        /// Wrap a pool, run migrations, and hydrate the cache from the table.
        pub async fn connect(pool: Pool) -> Result<Self, StoreError> {
            let store = Self {
                pool,
                cache: BindingCache::default(),
            };
            store.run_migrations().await?;
            store.load().await?;
            Ok(store)
        }

        async fn run_migrations(&self) -> Result<(), StoreError> {
            let client = self.client().await?;
            client
                .batch_execute(SCHEMA)
                .await
                .map_err(StoreError::backend)
        }

        async fn load(&self) -> Result<(), StoreError> {
            let client = self.client().await?;
            let rows = client
                .query(
                    "SELECT tenant, gate_ref, binding_json FROM attested_gate_bindings",
                    &[],
                )
                .await
                .map_err(StoreError::backend)?;
            for row in rows {
                let tenant: String = row.get(0);
                let gate_ref: String = row.get(1);
                let json: String = row.get(2);
                let binding: AttestedGateBinding =
                    serde_json::from_str(&json).map_err(StoreError::backend)?;
                self.cache.insert(
                    BindingKey::new(TenantId::new(tenant), GateRef::new(gate_ref)),
                    binding,
                );
            }
            Ok(())
        }

        async fn client(&self) -> Result<deadpool_postgres::Object, StoreError> {
            self.pool.get().await.map_err(StoreError::backend)
        }
    }

    impl SyncBindingRead for PostgresAttestedGateBindingStore {
        fn get_sync(&self, gate_ref: &GateRef) -> Option<AttestedGateBinding> {
            self.cache.get_by_gate_ref(gate_ref)
        }
    }

    #[async_trait]
    impl AttestedGateBindingStore for PostgresAttestedGateBindingStore {
        async fn put(
            &self,
            key: BindingKey,
            binding: AttestedGateBinding,
        ) -> Result<(), BindingError> {
            // Validate (gate_ref + tenant + self-consistency) before any I/O so a
            // malformed/mis-tenanted binding never reaches the table or cache.
            validate_binding_key(&key, &binding)?;
            let json = serde_json::to_string(&binding).map_err(|error| {
                tracing::error!(%error, "failed to serialize attested gate binding");
                BindingError::Poisoned
            })?;
            let client = self.client().await.map_err(|error| {
                tracing::error!(%error, "failed to acquire connection for binding put");
                BindingError::Poisoned
            })?;
            // Insert-only per (tenant, gate_ref): a binding is immutable once
            // written. A conflicting write (a re-put after approval) is REJECTED,
            // not silently applied — the approved binding must never change under
            // the resume path. `DO NOTHING` + the affected-row count is the
            // DB-level guard (no SELECT-then-INSERT race).
            let inserted = client
                .execute(
                    "INSERT INTO attested_gate_bindings (tenant, gate_ref, binding_json) \
                     VALUES ($1, $2, $3) \
                     ON CONFLICT (tenant, gate_ref) DO NOTHING",
                    &[&key.tenant.as_str(), &key.gate_ref.as_str(), &json],
                )
                .await
                .map_err(|error| {
                    tracing::error!(%error, "failed to persist attested gate binding");
                    BindingError::Poisoned
                })?;
            if inserted == 0 {
                tracing::error!(
                    tenant = %key.tenant.as_str(),
                    gate_ref = %key.gate_ref.as_str(),
                    "rejected attempt to overwrite an existing immutable gate binding"
                );
                return Err(BindingError::AlreadyExists);
            }
            // Write-through only after the durable insert succeeds.
            self.cache.insert(key, binding);
            Ok(())
        }

        async fn get(&self, key: &BindingKey) -> Option<AttestedGateBinding> {
            self.cache.get(key)
        }
    }
}

#[cfg(feature = "postgres")]
pub use postgres::PostgresAttestedGateBindingStore;

// ---------------------------------------------------------------------------
// libSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
mod libsql_backend {
    use super::*;
    use std::sync::Arc;

    /// Durable libSQL [`AttestedGateBindingStore`] with a write-through cache.
    pub struct LibSqlAttestedGateBindingStore {
        db: Arc<libsql::Database>,
        cache: BindingCache,
    }

    impl LibSqlAttestedGateBindingStore {
        /// Wrap a db handle, run migrations, and hydrate the cache.
        pub async fn connect(db: Arc<libsql::Database>) -> Result<Self, StoreError> {
            let store = Self {
                db,
                cache: BindingCache::default(),
            };
            store.run_migrations().await?;
            store.load().await?;
            Ok(store)
        }

        async fn run_migrations(&self) -> Result<(), StoreError> {
            let conn = self.connect_db().await?;
            conn.execute_batch(SCHEMA)
                .await
                .map_err(StoreError::backend)?;
            Ok(())
        }

        async fn load(&self) -> Result<(), StoreError> {
            let conn = self.connect_db().await?;
            let mut rows = conn
                .query(
                    "SELECT tenant, gate_ref, binding_json FROM attested_gate_bindings",
                    (),
                )
                .await
                .map_err(StoreError::backend)?;
            while let Some(row) = rows.next().await.map_err(StoreError::backend)? {
                let tenant: String = row.get(0).map_err(StoreError::backend)?;
                let gate_ref: String = row.get(1).map_err(StoreError::backend)?;
                let json: String = row.get(2).map_err(StoreError::backend)?;
                let binding: AttestedGateBinding =
                    serde_json::from_str(&json).map_err(StoreError::backend)?;
                self.cache.insert(
                    BindingKey::new(TenantId::new(tenant), GateRef::new(gate_ref)),
                    binding,
                );
            }
            Ok(())
        }

        async fn connect_db(&self) -> Result<libsql::Connection, StoreError> {
            let conn = self.db.connect().map_err(StoreError::backend)?;
            conn.query("PRAGMA busy_timeout = 5000", ())
                .await
                .map_err(StoreError::backend)?;
            Ok(conn)
        }
    }

    impl SyncBindingRead for LibSqlAttestedGateBindingStore {
        fn get_sync(&self, gate_ref: &GateRef) -> Option<AttestedGateBinding> {
            self.cache.get_by_gate_ref(gate_ref)
        }
    }

    #[async_trait]
    impl AttestedGateBindingStore for LibSqlAttestedGateBindingStore {
        async fn put(
            &self,
            key: BindingKey,
            binding: AttestedGateBinding,
        ) -> Result<(), BindingError> {
            // Validate (gate_ref + tenant + self-consistency) before any I/O so a
            // malformed/mis-tenanted binding never reaches the table or cache.
            validate_binding_key(&key, &binding)?;
            let json = serde_json::to_string(&binding).map_err(|error| {
                tracing::error!(%error, "failed to serialize attested gate binding");
                BindingError::Poisoned
            })?;
            let conn = self.connect_db().await.map_err(|error| {
                tracing::error!(%error, "failed to open libsql connection for binding put");
                BindingError::Poisoned
            })?;
            // Insert-only per (tenant, gate_ref): a binding is immutable once
            // written. A conflicting re-put is REJECTED at the DB level
            // (`DO NOTHING` + affected-row count), never silently overwriting the
            // approved binding under the resume path.
            let inserted = conn
                .execute(
                    "INSERT INTO attested_gate_bindings (tenant, gate_ref, binding_json) \
                     VALUES (?1, ?2, ?3) \
                     ON CONFLICT (tenant, gate_ref) DO NOTHING",
                    libsql::params![key.tenant.as_str(), key.gate_ref.as_str(), json],
                )
                .await
                .map_err(|error| {
                    tracing::error!(%error, "failed to persist attested gate binding");
                    BindingError::Poisoned
                })?;
            if inserted == 0 {
                tracing::error!(
                    tenant = %key.tenant.as_str(),
                    gate_ref = %key.gate_ref.as_str(),
                    "rejected attempt to overwrite an existing immutable gate binding"
                );
                return Err(BindingError::AlreadyExists);
            }
            self.cache.insert(key, binding);
            Ok(())
        }

        async fn get(&self, key: &BindingKey) -> Option<AttestedGateBinding> {
            self.cache.get(key)
        }
    }
}

#[cfg(feature = "libsql")]
pub use libsql_backend::LibSqlAttestedGateBindingStore;
