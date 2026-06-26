use ironclaw_host_api::{TenantId, VirtualPath};
use ironclaw_product_workflow::{
    LifecyclePackageRef, ProductWorkflowError, ScopedLifecycleInstallation,
    ScopedLifecycleInstallationId, ScopedLifecycleOwnership, lifecycle_package_kind_label,
};

use super::{
    DEFAULT_SCOPED_LIFECYCLE_ROOT, ScopedLifecycleInstallationIdReservation,
    scoped_lifecycle_durable_error,
};

pub(super) fn default_scoped_lifecycle_root() -> VirtualPath {
    VirtualPath::new(DEFAULT_SCOPED_LIFECYCLE_ROOT).expect("DEFAULT_SCOPED_LIFECYCLE_ROOT is valid") // safety: hard-coded /engine virtual path literal.
}

pub(super) fn scoped_lifecycle_tenant_installations_path(
    root: &VirtualPath,
    tenant_id: &TenantId,
) -> Result<VirtualPath, ProductWorkflowError> {
    let path = format!(
        "{}/tenants/{}/installations",
        root.as_str().trim_end_matches('/'),
        hex_component(tenant_id.as_str())
    );
    VirtualPath::new(path)
        .map_err(|error| scoped_lifecycle_durable_error("construct tenant lifecycle path", error))
}

pub(super) fn scoped_lifecycle_installation_path(
    root: &VirtualPath,
    installation: &ScopedLifecycleInstallation,
) -> Result<VirtualPath, ProductWorkflowError> {
    scoped_lifecycle_installation_path_for_parts(
        root,
        installation.tenant_id(),
        &installation.ownership,
        &installation.package_ref,
    )
}

pub(super) fn scoped_lifecycle_installation_path_for_reservation(
    root: &VirtualPath,
    reservation: &ScopedLifecycleInstallationIdReservation,
) -> Result<VirtualPath, ProductWorkflowError> {
    scoped_lifecycle_installation_path_for_parts(
        root,
        reservation.ownership.tenant_id(),
        &reservation.ownership,
        &reservation.package_ref,
    )
}

fn scoped_lifecycle_installation_path_for_parts(
    root: &VirtualPath,
    tenant_id: &TenantId,
    ownership: &ScopedLifecycleOwnership,
    package_ref: &LifecyclePackageRef,
) -> Result<VirtualPath, ProductWorkflowError> {
    let path = format!(
        "{}/{}/{}/{}.json",
        scoped_lifecycle_tenant_installations_path(root, tenant_id)?.as_str(),
        ownership_path_component(ownership),
        lifecycle_package_kind_label(package_ref.kind),
        hex_component(package_ref.id.as_str())
    );
    VirtualPath::new(path).map_err(|error| {
        scoped_lifecycle_durable_error("construct scoped lifecycle installation path", error)
    })
}

pub(super) fn scoped_lifecycle_installation_id_path(
    root: &VirtualPath,
    tenant_id: &TenantId,
    installation_id: &ScopedLifecycleInstallationId,
) -> Result<VirtualPath, ProductWorkflowError> {
    let path = format!(
        "{}/tenants/{}/installation_ids/{}.json",
        root.as_str().trim_end_matches('/'),
        hex_component(tenant_id.as_str()),
        hex_component(installation_id.as_str())
    );
    VirtualPath::new(path)
        .map_err(|error| scoped_lifecycle_durable_error("construct installation id path", error))
}

fn ownership_path_component(ownership: &ScopedLifecycleOwnership) -> String {
    match ownership {
        ScopedLifecycleOwnership::AdminShared { .. } => "admin_shared".to_string(),
        ScopedLifecycleOwnership::UserPrivate { user_id, .. } => {
            format!("user_private/{}", hex_component(user_id.as_str()))
        }
    }
}

fn hex_component(value: &str) -> String {
    hex::encode(value)
}
