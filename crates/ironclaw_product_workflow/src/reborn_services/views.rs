//! Generic read conduit for descriptor-declared product views.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

use super::{ProductSurface, RebornServicesError, WebUiAuthenticatedCaller};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyViewParams {}

/// Stable metadata for one read-only product view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebornViewDescriptor {
    pub id: &'static str,
    pub paginated: bool,
}

/// Typed declaration for one ProductSurface read view.
///
/// The wire conduit remains [`RebornViewQuery`] / [`RebornViewPage`]. This
/// wrapper keeps declaration sites tied to the request/response DTOs and gives
/// callers a shared way to encode query params and decode payloads without
/// hand-written `serde_json` glue at every route.
#[derive(Debug, PartialEq, Eq)]
pub struct ProductView<Params, Output> {
    pub id: &'static str,
    pub paginated: bool,
    _types: PhantomData<fn(Params) -> Output>,
}

impl<Params, Output> Clone for ProductView<Params, Output> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Params, Output> Copy for ProductView<Params, Output> {}

impl<Params, Output> ProductView<Params, Output> {
    pub const fn new(id: &'static str, paginated: bool) -> Self {
        Self {
            id,
            paginated,
            _types: PhantomData,
        }
    }

    pub const fn paginated(id: &'static str) -> Self {
        Self::new(id, true)
    }

    pub const fn unpaginated(id: &'static str) -> Self {
        Self::new(id, false)
    }

    pub const fn descriptor(&self) -> RebornViewDescriptor {
        RebornViewDescriptor {
            id: self.id,
            paginated: self.paginated,
        }
    }
}

impl<Params, Output> ProductView<Params, Output>
where
    Params: Serialize,
{
    pub fn query(
        &self,
        params: Params,
        cursor: Option<String>,
    ) -> Result<RebornViewQuery, RebornServicesError> {
        Ok(RebornViewQuery {
            view_id: self.id.to_string(),
            params: serde_json::to_value(params).map_err(RebornServicesError::internal_from)?,
            cursor,
        })
    }
}

impl<Params, Output> ProductView<Params, Output>
where
    Output: DeserializeOwned,
{
    pub fn decode_page(&self, page: RebornViewPage) -> Result<Output, RebornServicesError> {
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
    }
}

impl<Params, Output> ProductView<Params, Output>
where
    Params: Serialize,
    Output: DeserializeOwned,
{
    pub async fn query_on<S>(
        &self,
        surface: &S,
        caller: WebUiAuthenticatedCaller,
        params: Params,
        cursor: Option<String>,
    ) -> Result<Output, RebornServicesError>
    where
        S: ProductSurface + ?Sized,
    {
        let page = surface.query(caller, self.query(params, cursor)?).await?;
        self.decode_page(page)
    }
}

/// One registered, read-only product view invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornViewQuery {
    pub view_id: String,
    pub params: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// One page returned by the generic product view conduit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornViewPage {
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

pub(super) fn parse_empty_view_params(
    params: serde_json::Value,
) -> Result<(), RebornServicesError> {
    serde_json::from_value::<EmptyViewParams>(params)
        .map(|_| ())
        .map_err(RebornServicesError::internal_from)
}

pub(super) fn required_string_view_param(
    params: serde_json::Value,
    field: &str,
) -> Result<String, RebornServicesError> {
    let object = params
        .as_object()
        .ok_or_else(|| RebornServicesError::internal_from("view params must be a JSON object"))?;
    if object.len() != 1 {
        return Err(RebornServicesError::internal_from(
            "view params contain unexpected fields",
        ));
    }
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| RebornServicesError::internal_from("view params missing string field"))
}

pub(super) fn view_page<T: Serialize>(payload: T) -> Result<RebornViewPage, RebornServicesError> {
    view_page_with_cursor(payload, None)
}

pub(super) fn view_page_with_cursor<T: Serialize>(
    payload: T,
    next_cursor: Option<String>,
) -> Result<RebornViewPage, RebornServicesError> {
    Ok(RebornViewPage {
        payload: serde_json::to_value(payload).map_err(RebornServicesError::internal_from)?,
        next_cursor,
    })
}

/// One composition-supplied implementation behind the generic view conduit.
///
/// Product features register descriptors and providers instead of growing
/// `RebornServicesApi` with feature-specific read methods.
#[async_trait]
pub trait RebornViewProvider: Send + Sync {
    fn descriptor(&self) -> RebornViewDescriptor;

    async fn query(
        &self,
        caller: WebUiAuthenticatedCaller,
        params: serde_json::Value,
        cursor: Option<String>,
    ) -> Result<RebornViewPage, RebornServicesError>;
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
        _caller: WebUiAuthenticatedCaller,
        _params: serde_json::Value,
        _cursor: Option<String>,
    ) -> Result<RebornViewPage, RebornServicesError> {
        Err(RebornServicesError::service_unavailable(false))
    }
}
