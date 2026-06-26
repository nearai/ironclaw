use ironclaw_filesystem::{Entry, IndexKey, IndexValue, RecordKind, VersionedEntry};
use ironclaw_product_workflow::{
    ProductWorkflowError, ScopedLifecycleInstallation, lifecycle_package_kind_label,
};

use super::{
    SCOPED_LIFECYCLE_ID_RESERVATION_RECORD_KIND, SCOPED_LIFECYCLE_ID_TOMBSTONE_RECORD_KIND,
    SCOPED_LIFECYCLE_RECORD_KIND, SCOPED_LIFECYCLE_TOMBSTONE_RECORD_KIND,
    ScopedLifecycleInstallationIdReservation, VersionedScopedLifecycleInstallation,
    scoped_lifecycle_durable_error, scoped_lifecycle_transient,
};

pub(super) fn entry_for_scoped_lifecycle_installation(
    installation: &ScopedLifecycleInstallation,
) -> Result<Entry, ProductWorkflowError> {
    entry_for_scoped_lifecycle_record(installation, SCOPED_LIFECYCLE_RECORD_KIND)
}

pub(super) fn tombstone_entry_for_scoped_lifecycle_installation(
    installation: &ScopedLifecycleInstallation,
) -> Result<Entry, ProductWorkflowError> {
    entry_for_scoped_lifecycle_record(installation, SCOPED_LIFECYCLE_TOMBSTONE_RECORD_KIND)
}

pub(super) fn entry_for_installation_id_reservation(
    reservation: &ScopedLifecycleInstallationIdReservation,
) -> Result<Entry, ProductWorkflowError> {
    entry_for_installation_id_reservation_record(
        reservation,
        SCOPED_LIFECYCLE_ID_RESERVATION_RECORD_KIND,
    )
}

pub(super) fn tombstone_entry_for_installation_id_reservation(
    reservation: &ScopedLifecycleInstallationIdReservation,
) -> Result<Entry, ProductWorkflowError> {
    entry_for_installation_id_reservation_record(
        reservation,
        SCOPED_LIFECYCLE_ID_TOMBSTONE_RECORD_KIND,
    )
}

fn entry_for_installation_id_reservation_record(
    reservation: &ScopedLifecycleInstallationIdReservation,
    record_kind: &'static str,
) -> Result<Entry, ProductWorkflowError> {
    let payload = serde_json::json!({
        "installation_id": reservation.installation_id,
        "package_ref": reservation.package_ref,
        "ownership": reservation.ownership,
    });
    let kind = RecordKind::new(record_kind).map_err(|error| {
        scoped_lifecycle_durable_error("construct scoped lifecycle id record kind", error)
    })?;
    let entry = Entry::record(kind, &payload)
        .map_err(|error| scoped_lifecycle_durable_error("serialize installation id entry", error))?
        .with_indexed(
            index_key("tenant_id")?,
            text(reservation.ownership.tenant_id().as_str()),
        )
        .with_indexed(
            index_key("installation_id")?,
            text(reservation.installation_id.as_str()),
        )
        .with_indexed(
            index_key("package_kind")?,
            text(lifecycle_package_kind_label(reservation.package_ref.kind)),
        )
        .with_indexed(
            index_key("package_id")?,
            text(reservation.package_ref.id.as_str()),
        )
        .with_indexed(index_key("ownership")?, text(reservation.ownership.label()));
    Ok(entry)
}

fn entry_for_scoped_lifecycle_record(
    installation: &ScopedLifecycleInstallation,
    record_kind: &'static str,
) -> Result<Entry, ProductWorkflowError> {
    let payload = serde_json::to_value(installation)
        .map_err(|error| scoped_lifecycle_durable_error("serialize installation", error))?;
    let kind = RecordKind::new(record_kind).map_err(|error| {
        scoped_lifecycle_durable_error("construct scoped lifecycle record kind", error)
    })?;
    let entry = Entry::record(kind, &payload)
        .map_err(|error| scoped_lifecycle_durable_error("serialize installation entry", error))?
        .with_indexed(
            index_key("tenant_id")?,
            text(installation.tenant_id().as_str()),
        )
        .with_indexed(
            index_key("installation_id")?,
            text(installation.installation_id.as_str()),
        )
        .with_indexed(
            index_key("package_kind")?,
            text(lifecycle_package_kind_label(installation.package_ref.kind)),
        )
        .with_indexed(
            index_key("package_id")?,
            text(installation.package_ref.id.as_str()),
        )
        .with_indexed(
            index_key("ownership")?,
            text(installation.ownership.label()),
        )
        .with_indexed(
            index_key("enabled")?,
            IndexValue::Bool(installation.enabled),
        )
        .with_indexed(
            index_key("updated_at_ms")?,
            IndexValue::I64(installation.updated_at.timestamp_millis()),
        );
    Ok(entry)
}

pub(super) fn is_scoped_lifecycle_tombstone(entry: &Entry) -> bool {
    entry
        .kind
        .as_ref()
        .is_some_and(|kind| kind.as_str() == SCOPED_LIFECYCLE_TOMBSTONE_RECORD_KIND)
}

pub(super) fn is_installation_id_tombstone(entry: &Entry) -> bool {
    entry
        .kind
        .as_ref()
        .is_some_and(|kind| kind.as_str() == SCOPED_LIFECYCLE_ID_TOMBSTONE_RECORD_KIND)
}

pub(super) fn parse_installation_id_reservation(
    entry: Entry,
) -> Result<ScopedLifecycleInstallationIdReservation, ProductWorkflowError> {
    let payload = entry
        .parse_json::<serde_json::Value>()
        .map_err(|error| scoped_lifecycle_durable_error("deserialize installation id", error))?;
    let installation_id = serde_json::from_value(reservation_field(&payload, "installation_id")?)
        .map_err(|error| {
        scoped_lifecycle_durable_error("deserialize installation id", error)
    })?;
    let package_ref =
        serde_json::from_value(reservation_field(&payload, "package_ref")?).map_err(|error| {
            scoped_lifecycle_durable_error("deserialize installation id package", error)
        })?;
    let ownership =
        serde_json::from_value(reservation_field(&payload, "ownership")?).map_err(|error| {
            scoped_lifecycle_durable_error("deserialize installation id ownership", error)
        })?;
    Ok(ScopedLifecycleInstallationIdReservation {
        installation_id,
        package_ref,
        ownership,
    })
}

fn reservation_field(
    payload: &serde_json::Value,
    field: &'static str,
) -> Result<serde_json::Value, ProductWorkflowError> {
    payload
        .get(field)
        .cloned()
        .ok_or_else(|| scoped_lifecycle_transient(format!("scoped lifecycle missing {field}")))
}

fn parse_scoped_lifecycle_installation(
    entry: VersionedEntry,
) -> Result<ScopedLifecycleInstallation, ProductWorkflowError> {
    let installation = entry
        .entry
        .parse_json::<ScopedLifecycleInstallation>()
        .map_err(|error| scoped_lifecycle_durable_error("deserialize installation", error))?;
    installation.validate()?;
    Ok(installation)
}

pub(super) fn parse_versioned_scoped_lifecycle_installation(
    entry: VersionedEntry,
) -> Result<VersionedScopedLifecycleInstallation, ProductWorkflowError> {
    let path = entry.path.clone();
    let version = entry.version;
    let installation = parse_scoped_lifecycle_installation(entry)?;
    Ok(VersionedScopedLifecycleInstallation {
        path,
        installation,
        version,
    })
}

fn index_key(value: &'static str) -> Result<IndexKey, ProductWorkflowError> {
    IndexKey::new(value)
        .map_err(|error| scoped_lifecycle_durable_error("construct lifecycle index key", error))
}

fn text(value: &str) -> IndexValue {
    IndexValue::Text(value.to_string())
}
