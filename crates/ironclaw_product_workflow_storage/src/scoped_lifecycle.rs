mod entries;
mod paths;
mod store;

use ironclaw_filesystem::{FilesystemError, RecordVersion};
use ironclaw_host_api::VirtualPath;
use ironclaw_product_workflow::{
    LifecyclePackageRef, ProductWorkflowError, ScopedLifecycleInstallation,
    ScopedLifecycleInstallationId, ScopedLifecycleOwnership,
};

pub use store::FilesystemScopedLifecycleInstallationStore;
#[cfg(feature = "libsql")]
pub use store::RebornLibSqlScopedLifecycleInstallationStore;
#[cfg(feature = "postgres")]
pub use store::RebornPostgresScopedLifecycleInstallationStore;

const DEFAULT_SCOPED_LIFECYCLE_ROOT: &str = "/engine/product_workflow/scoped_lifecycle";
const SCOPED_LIFECYCLE_RECORD_KIND: &str = "scoped_lifecycle_installation";
const SCOPED_LIFECYCLE_TOMBSTONE_RECORD_KIND: &str = "scoped_lifecycle_tombstone";
const SCOPED_LIFECYCLE_ID_RESERVATION_RECORD_KIND: &str = "scoped_lifecycle_installation_id";
const SCOPED_LIFECYCLE_ID_TOMBSTONE_RECORD_KIND: &str =
    "scoped_lifecycle_installation_id_tombstone";

enum PackagePathState {
    Absent,
    Tombstone(RecordVersion),
    Occupied,
}

enum InstallationIdState {
    Absent,
    Tombstone(RecordVersion),
    Reserved(ScopedLifecycleInstallationIdReservation, RecordVersion),
}

struct VersionedScopedLifecycleInstallation {
    path: VirtualPath,
    installation: ScopedLifecycleInstallation,
    version: RecordVersion,
}

#[derive(Debug, Clone, PartialEq)]
struct ScopedLifecycleInstallationIdReservation {
    installation_id: ScopedLifecycleInstallationId,
    package_ref: LifecyclePackageRef,
    ownership: ScopedLifecycleOwnership,
}

impl ScopedLifecycleInstallationIdReservation {
    fn new(installation: &ScopedLifecycleInstallation) -> Self {
        Self {
            installation_id: installation.installation_id.clone(),
            package_ref: installation.package_ref.clone(),
            ownership: installation.ownership.clone(),
        }
    }

    fn matches_installation(&self, installation: &ScopedLifecycleInstallation) -> bool {
        self.installation_id == installation.installation_id
            && self.package_ref == installation.package_ref
            && self.ownership == installation.ownership
    }
}

fn scoped_lifecycle_transient(reason: impl Into<String>) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: reason.into(),
    }
}

fn scoped_lifecycle_invalid_request(reason: &'static str) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: reason.to_string(),
    }
}

fn scoped_lifecycle_durable_error(
    operation: &'static str,
    error: impl std::fmt::Display,
) -> ProductWorkflowError {
    let error_type = std::any::type_name_of_val(&error);
    tracing::error!(
        operation,
        error_type,
        "product workflow scoped lifecycle store failed"
    );
    scoped_lifecycle_transient(format!("scoped lifecycle store failed to {operation}"))
}

fn scoped_lifecycle_filesystem_error(
    operation: &'static str,
    error: FilesystemError,
) -> ProductWorkflowError {
    scoped_lifecycle_durable_error(operation, error)
}
