//! Durable product workflow [`IdempotencyLedger`] storage adapters.

#![cfg_attr(
    not(any(feature = "libsql", feature = "postgres")),
    allow(dead_code, unused_imports)
)]

use std::sync::Arc;

#[cfg(any(feature = "libsql", feature = "postgres"))]
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, IndexKey, IndexValue, RecordKind, RecordVersion,
    RootFilesystem,
};
use ironclaw_host_api::VirtualPath;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_product_workflow::IdempotencyLedger;
use ironclaw_product_workflow::{
    ActionFingerprintKey, ActionPhase, IdempotencyDecision, ProductInboundAction,
    ProductWorkflowError,
};

const DEFAULT_IN_FLIGHT_LEASE: Duration = Duration::seconds(60);
const DEFAULT_LEDGER_ROOT: &str = "/engine/product_workflow/idempotency/actions";
const ACTION_RECORD_KIND: &str = "product_workflow_action";

struct FilesystemIdempotencyLedger {
    filesystem: Arc<dyn RootFilesystem>,
    root: VirtualPath,
    in_flight_lease: Duration,
}

impl FilesystemIdempotencyLedger {
    fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self::with_in_flight_lease(filesystem, DEFAULT_IN_FLIGHT_LEASE)
    }

    fn with_in_flight_lease(
        filesystem: Arc<dyn RootFilesystem>,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            filesystem,
            root: default_ledger_root(),
            in_flight_lease,
        }
    }

    fn with_root(
        filesystem: Arc<dyn RootFilesystem>,
        root: VirtualPath,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            filesystem,
            root,
            in_flight_lease,
        }
    }

    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        let path = action_path(&self.root, &fingerprint)?;
        let action = ProductInboundAction::begin(fingerprint, received_at);
        match self
            .filesystem
            .put(&path, entry_for_action(&action)?, CasExpectation::Absent)
            .await
        {
            Ok(_) => return Ok(IdempotencyDecision::New(action)),
            Err(FilesystemError::VersionMismatch { .. }) => {}
            Err(error) => return Err(filesystem_error("reserve action", error)),
        }

        loop {
            let Some((prior, version)) = load_action(self.filesystem.as_ref(), &path).await? else {
                return Err(transient("idempotency ledger conflict row disappeared"));
            };
            if prior.is_terminal() {
                return Ok(IdempotencyDecision::Replay(prior));
            }
            if fresh_in_flight(&prior, received_at, self.in_flight_lease) {
                return Err(in_flight_error());
            }

            let replacement = ProductInboundAction::begin(prior.fingerprint.clone(), received_at);
            match self
                .filesystem
                .put(
                    &path,
                    entry_for_action(&replacement)?,
                    CasExpectation::Version(version),
                )
                .await
            {
                Ok(_) => return Ok(IdempotencyDecision::New(replacement)),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("reclaim action", error)),
            }
        }
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let path = action_path(&self.root, &action.fingerprint)?;
        loop {
            let Some((current, version)) = load_action(self.filesystem.as_ref(), &path).await?
            else {
                return Err(transient(
                    "idempotency reservation missing before terminal settle",
                ));
            };
            if current.is_terminal() {
                if current.action_id == action.action_id {
                    return Ok(());
                }
                return Err(transient(
                    "idempotency reservation was superseded before terminal settle",
                ));
            }
            if current.action_id != action.action_id {
                return Err(transient(
                    "idempotency reservation was superseded before terminal settle",
                ));
            }

            match self
                .filesystem
                .put(
                    &path,
                    entry_for_action(&action)?,
                    CasExpectation::Version(version),
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("settle action", error)),
            }
        }
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let path = action_path(&self.root, &action.fingerprint)?;
        loop {
            let Some((current, version)) = load_action(self.filesystem.as_ref(), &path).await?
            else {
                return Ok(());
            };
            if current.is_terminal() || current.action_id != action.action_id {
                return Ok(());
            }

            let mut released = current;
            released.received_at = expired_received_at(released.received_at, self.in_flight_lease);
            match self
                .filesystem
                .put(
                    &path,
                    entry_for_action(&released)?,
                    CasExpectation::Version(version),
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("release action", error)),
            }
        }
    }
}

/// libSQL-backed product workflow idempotency ledger using the shared
/// SQL filesystem backend for persistence.
#[cfg(feature = "libsql")]
pub struct RebornLibSqlIdempotencyLedger {
    inner: FilesystemIdempotencyLedger,
}

#[cfg(feature = "libsql")]
impl RebornLibSqlIdempotencyLedger {
    pub fn new(filesystem: Arc<LibSqlRootFilesystem>) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::new(filesystem),
        }
    }

    pub fn with_in_flight_lease(
        filesystem: Arc<LibSqlRootFilesystem>,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::with_in_flight_lease(filesystem, in_flight_lease),
        }
    }

    pub fn with_root(
        filesystem: Arc<LibSqlRootFilesystem>,
        root: VirtualPath,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::with_root(filesystem, root, in_flight_lease),
        }
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl IdempotencyLedger for RebornLibSqlIdempotencyLedger {
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        self.inner.begin_or_replay(fingerprint, received_at).await
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        self.inner.settle(action).await
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        self.inner.release(action).await
    }
}

/// PostgreSQL-backed product workflow idempotency ledger using the shared
/// SQL filesystem backend for persistence.
#[cfg(feature = "postgres")]
pub struct RebornPostgresIdempotencyLedger {
    inner: FilesystemIdempotencyLedger,
}

#[cfg(feature = "postgres")]
impl RebornPostgresIdempotencyLedger {
    pub fn new(filesystem: Arc<PostgresRootFilesystem>) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::new(filesystem),
        }
    }

    pub fn with_in_flight_lease(
        filesystem: Arc<PostgresRootFilesystem>,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::with_in_flight_lease(filesystem, in_flight_lease),
        }
    }

    pub fn with_root(
        filesystem: Arc<PostgresRootFilesystem>,
        root: VirtualPath,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::with_root(filesystem, root, in_flight_lease),
        }
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl IdempotencyLedger for RebornPostgresIdempotencyLedger {
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        self.inner.begin_or_replay(fingerprint, received_at).await
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        self.inner.settle(action).await
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        self.inner.release(action).await
    }
}

fn transient(reason: impl Into<String>) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: reason.into(),
    }
}

fn durable_error(operation: &'static str, error: impl std::fmt::Display) -> ProductWorkflowError {
    let error_type = std::any::type_name_of_val(&error);
    tracing::error!(
        operation,
        error_type,
        "product workflow idempotency ledger failed"
    );
    transient(format!("idempotency ledger failed to {operation}"))
}

fn filesystem_error(operation: &'static str, error: FilesystemError) -> ProductWorkflowError {
    durable_error(operation, error)
}

fn fresh_in_flight(
    action: &ProductInboundAction,
    received_at: DateTime<Utc>,
    lease: Duration,
) -> bool {
    !action.is_terminal() && action.received_at + lease > received_at
}

fn in_flight_error() -> ProductWorkflowError {
    transient("idempotency fingerprint already in flight; retry after recovery lease")
}

fn expired_received_at(received_at: DateTime<Utc>, lease: Duration) -> DateTime<Utc> {
    received_at - lease - Duration::seconds(1)
}

async fn load_action(
    filesystem: &dyn RootFilesystem,
    path: &VirtualPath,
) -> Result<Option<(ProductInboundAction, RecordVersion)>, ProductWorkflowError> {
    let Some(entry) = filesystem
        .get(path)
        .await
        .map_err(|error| filesystem_error("load action", error))?
    else {
        return Ok(None);
    };
    let action = entry
        .entry
        .parse_json()
        .map_err(|error| durable_error("deserialize action", error))?;
    Ok(Some((action, entry.version)))
}

fn entry_for_action(action: &ProductInboundAction) -> Result<Entry, ProductWorkflowError> {
    let payload =
        serde_json::to_value(action).map_err(|error| durable_error("serialize action", error))?;
    let kind = RecordKind::new(ACTION_RECORD_KIND)
        .map_err(|error| durable_error("construct action record kind", error))?;
    let entry = Entry::record(kind, &payload)
        .map_err(|error| durable_error("serialize action entry", error))?
        .with_indexed(
            index_key("adapter_id")?,
            text(action.fingerprint.adapter_id.as_str()),
        )
        .with_indexed(
            index_key("installation_id")?,
            text(action.fingerprint.installation_id.as_str()),
        )
        .with_indexed(
            index_key("source_binding_key")?,
            text(action.fingerprint.source_binding_key.as_str()),
        )
        .with_indexed(
            index_key("external_event_id")?,
            text(action.fingerprint.external_event_id.as_str()),
        )
        .with_indexed(index_key("phase")?, text(phase_label(action.phase)))
        .with_indexed(
            index_key("received_at_ms")?,
            IndexValue::I64(action.received_at.timestamp_millis()),
        );
    Ok(entry)
}

fn index_key(value: &'static str) -> Result<IndexKey, ProductWorkflowError> {
    IndexKey::new(value).map_err(|error| durable_error("construct action index key", error))
}

fn text(value: &str) -> IndexValue {
    IndexValue::Text(value.to_string())
}

fn phase_label(phase: ActionPhase) -> &'static str {
    match phase {
        ActionPhase::Received => "received",
        ActionPhase::Dispatched => "dispatched",
        ActionPhase::Settled => "settled",
        ActionPhase::DeduplicatedReplay => "deduplicated_replay",
    }
}

fn action_path(
    root: &VirtualPath,
    fingerprint: &ActionFingerprintKey,
) -> Result<VirtualPath, ProductWorkflowError> {
    let path = format!(
        "{}/{}/{}/{}/{}.json",
        root.as_str().trim_end_matches('/'),
        hex_component(fingerprint.adapter_id.as_str()),
        hex_component(fingerprint.installation_id.as_str()),
        hex_component(fingerprint.source_binding_key.as_str()),
        hex_component(fingerprint.external_event_id.as_str())
    );
    VirtualPath::new(path).map_err(|error| durable_error("construct action path", error))
}

fn default_ledger_root() -> VirtualPath {
    VirtualPath::new(DEFAULT_LEDGER_ROOT).expect("DEFAULT_LEDGER_ROOT is valid") // safety: hard-coded /engine virtual path literal.
}

fn hex_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
