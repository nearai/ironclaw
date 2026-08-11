//! Product-side helpers for descriptor-declared views.
//!
//! The typed [`ProductView`] wrapper itself moved to
//! `ironclaw_product_contracts::descriptors` with the WS5 port inversion
//! (PROPOSAL §6.1.3); what stays here is the parameter/page glue product's own
//! view implementations use plus the fail-closed default provider.
//!
//! [`ProductView`]: ironclaw_product_contracts::descriptors::ProductView

use async_trait::async_trait;
use ironclaw_product_contracts::views::{RebornViewDescriptor, RebornViewPage, RebornViewProvider};
use serde::Deserialize;
use serde::Serialize;

use super::{ProductSurfaceCaller, ProductSurfaceError};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyViewParams {}

pub(super) fn parse_empty_view_params(
    params: serde_json::Value,
) -> Result<(), ProductSurfaceError> {
    serde_json::from_value::<EmptyViewParams>(params)
        .map(|_| ())
        .map_err(ProductSurfaceError::internal_from)
}

pub(super) fn required_string_view_param(
    params: serde_json::Value,
    field: &str,
) -> Result<String, ProductSurfaceError> {
    let object = params
        .as_object()
        .ok_or_else(|| ProductSurfaceError::internal_from("view params must be a JSON object"))?;
    if object.len() != 1 {
        return Err(ProductSurfaceError::internal_from(
            "view params contain unexpected fields",
        ));
    }
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| ProductSurfaceError::internal_from("view params missing string field"))
}

pub(super) fn view_page<T: Serialize>(payload: T) -> Result<RebornViewPage, ProductSurfaceError> {
    view_page_with_cursor(payload, None)
}

pub(super) fn view_page_with_cursor<T: Serialize>(
    payload: T,
    next_cursor: Option<String>,
) -> Result<RebornViewPage, ProductSurfaceError> {
    Ok(RebornViewPage {
        payload: serde_json::to_value(payload).map_err(ProductSurfaceError::internal_from)?,
        next_cursor,
    })
}

/// Fail-closed static default for compositions without an additional view.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableRebornViewProvider;

#[async_trait]
impl RebornViewProvider for UnavailableRebornViewProvider {
    fn descriptor(&self) -> RebornViewDescriptor {
        RebornViewDescriptor {
            id: "__unavailable_product_view",
            paginated: false,
        }
    }

    async fn query(
        &self,
        _caller: ProductSurfaceCaller,
        _params: serde_json::Value,
        _cursor: Option<String>,
    ) -> Result<RebornViewPage, ProductSurfaceError> {
        Err(ProductSurfaceError::service_unavailable(false))
    }
}
