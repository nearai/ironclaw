//! The typed declaration wrappers for the three `ProductSurface` operation
//! shapes — command, capability, and view (PROPOSAL §6.1.3, which assigns
//! "command/view/capability **descriptor types** … the *types*, not product's
//! concrete constants, which stay in product as the frozen inventory").
//!
//! A descriptor is a `&'static str` id plus the request/response types that id
//! speaks, and the encode/decode glue that saves every declaration site from
//! hand-written `serde_json`. It carries no id of its own: the inventory of
//! concrete commands, capabilities, and views is product's frozen surface and
//! stays in `ironclaw_product`. What lives here is only the *shape* a
//! declaration takes, so a transport can hold a descriptor, call it on a
//! [`BoundProductSurface`], and decode the answer without compiling product.
//!
//! Never here: a concrete descriptor constant, a handler, or a provider.

use std::marker::PhantomData;

use ironclaw_host_api::ids::{ActivityId, CapabilityId};
use ironclaw_host_api::resolution::Resolution;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::surface::{
    BoundProductSurface, ProductSurfaceError, ProductSurfaceInvokeRequest,
    ProductSurfaceQueryRequest,
};
use crate::views::{RebornViewDescriptor, RebornViewPage, RebornViewQuery};

/// Input shape for a command that takes no arguments.
///
/// `deny_unknown_fields` is the contract: a client that sends a body to a
/// no-argument command gets a rejection rather than silent acceptance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyProductCommandInput {}

/// Typed declaration for one `ProductSurface` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductSurfaceCommandDescriptor<Input, Output> {
    pub id: &'static str,
    _types: PhantomData<fn(Input) -> Output>,
}

impl<Input, Output> ProductSurfaceCommandDescriptor<Input, Output> {
    pub const fn new(id: &'static str) -> Self {
        Self {
            id,
            _types: PhantomData,
        }
    }

    pub fn capability_id(&self) -> Result<CapabilityId, ProductSurfaceError> {
        CapabilityId::new(self.id).map_err(ProductSurfaceError::internal_from)
    }
}

impl<Input, Output> ProductSurfaceCommandDescriptor<Input, Output>
where
    Input: Serialize,
    Output: DeserializeOwned,
{
    pub async fn invoke_on(
        &self,
        surface: &BoundProductSurface,
        input: Input,
        activity_id: ActivityId,
    ) -> Result<Output, ProductSurfaceError> {
        let input = serde_json::to_value(input).map_err(ProductSurfaceError::internal_from)?;
        let response = surface
            .invoke(ProductSurfaceInvokeRequest {
                operation_id: self.capability_id()?,
                input,
                activity_id,
            })
            .await?;
        serde_json::from_value(response.output).map_err(ProductSurfaceError::internal_from)
    }
}

/// ProductSurface operation descriptor.
///
/// Capability declarations stay as one stable id plus origin/policy metadata
/// elsewhere. Product-workflow-owned ids are backed by product's capability
/// handler registry; runtime-backed ids delegate to the wired first-party
/// capability invoker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductCapabilityDescriptor {
    pub id: &'static str,
}

impl ProductCapabilityDescriptor {
    pub const fn product_operation(id: &'static str) -> Self {
        Self { id }
    }

    pub const fn api_only(id: &'static str) -> Self {
        Self::product_operation(id)
    }

    pub fn capability_id(&self) -> Result<CapabilityId, ProductSurfaceError> {
        CapabilityId::new(self.id).map_err(ProductSurfaceError::internal_from)
    }

    pub async fn invoke_on<T>(
        &self,
        surface: &BoundProductSurface,
        input: T,
        activity_id: ActivityId,
    ) -> Result<Resolution, ProductSurfaceError>
    where
        T: Serialize,
    {
        let input = serde_json::to_value(input).map_err(ProductSurfaceError::internal_from)?;
        let response = surface
            .invoke(ProductSurfaceInvokeRequest {
                operation_id: self.capability_id()?,
                input,
                activity_id,
            })
            .await?;
        serde_json::from_value(response.output).map_err(ProductSurfaceError::internal_from)
    }
}

/// Typed declaration for one `ProductSurface` read view.
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
    ) -> Result<RebornViewQuery, ProductSurfaceError> {
        Ok(RebornViewQuery {
            view_id: self.id.to_string(),
            params: serde_json::to_value(params).map_err(ProductSurfaceError::internal_from)?,
            cursor,
        })
    }
}

impl<Params, Output> ProductView<Params, Output>
where
    Output: DeserializeOwned,
{
    pub fn decode_page(&self, page: RebornViewPage) -> Result<Output, ProductSurfaceError> {
        serde_json::from_value(page.payload).map_err(ProductSurfaceError::internal_from)
    }
}

impl<Params, Output> ProductView<Params, Output>
where
    Params: Serialize,
    Output: DeserializeOwned,
{
    pub async fn query_on(
        &self,
        surface: &BoundProductSurface,
        params: Params,
        cursor: Option<String>,
    ) -> Result<Output, ProductSurfaceError> {
        let query = self.query(params, cursor)?;
        let page = surface
            .query(ProductSurfaceQueryRequest {
                view_id: query.view_id,
                input: query.params,
                cursor: query.cursor,
                limit: None,
            })
            .await?;
        let payload = page
            .items
            .into_iter()
            .next()
            .ok_or_else(ProductSurfaceError::internal)?;
        self.decode_page(RebornViewPage {
            payload,
            next_cursor: page.next_cursor,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
    struct Params {
        name: String,
    }

    #[test]
    fn a_view_descriptor_round_trips_its_id_paging_and_query_encoding() {
        const VIEW: ProductView<Params, Params> = ProductView::paginated("product.things.list");
        assert_eq!(
            VIEW.descriptor(),
            RebornViewDescriptor {
                id: "product.things.list",
                paginated: true,
            }
        );
        let query = VIEW
            .query(
                Params {
                    name: "n".to_string(),
                },
                Some("cur".to_string()),
            )
            .expect("encode");
        assert_eq!(query.view_id, "product.things.list");
        assert_eq!(query.cursor.as_deref(), Some("cur"));
        let decoded = VIEW
            .decode_page(RebornViewPage {
                payload: query.params,
                next_cursor: None,
            })
            .expect("decode");
        assert_eq!(decoded.name, "n");
    }

    #[test]
    fn command_and_capability_descriptors_derive_their_capability_id_from_the_declared_id() {
        const COMMAND: ProductSurfaceCommandDescriptor<EmptyProductCommandInput, ()> =
            ProductSurfaceCommandDescriptor::new("product.thing.do");
        const CAPABILITY: ProductCapabilityDescriptor =
            ProductCapabilityDescriptor::product_operation("product.thing.grant");
        assert_eq!(
            COMMAND.capability_id().expect("capability id").as_str(),
            "product.thing.do"
        );
        assert_eq!(
            CAPABILITY.capability_id().expect("capability id").as_str(),
            "product.thing.grant"
        );
        assert_eq!(
            ProductCapabilityDescriptor::api_only("product.thing.grant"),
            CAPABILITY
        );
    }

    #[test]
    fn the_no_argument_command_input_rejects_a_body() {
        serde_json::from_value::<EmptyProductCommandInput>(serde_json::json!({}))
            .expect("empty object is the contract");
        serde_json::from_value::<EmptyProductCommandInput>(serde_json::json!({"a": 1}))
            .expect_err("unknown fields are denied, not ignored");
    }
}
