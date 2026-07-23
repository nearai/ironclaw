//! Generic read conduit for descriptor-declared product views.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

use super::{
    ChannelInboundSurfaceOutcome, ChannelInboundSurfaceRequest, ProductSurface, ProjectFsFile,
    RebornAttachmentBytes, RebornServicesError, WebUiAuthenticatedCaller,
};

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

/// Stable identifier for one result-bearing product operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProductOperationId {
    ChannelInboundAdmit,
    CreateThread,
    SubmitTurn,
    CancelRun,
    ResolveGate,
    RetryRun,
    ProjectCreate,
    ProjectFsRead,
    FsRead,
    AttachmentRead,
    TraceAccountLoginLink,
    TraceHoldAuthorize,
    OperatorConfigSetKey,
    OperatorServiceLifecycle,
    LlmTestConnection,
    LlmListModels,
    LlmNearAiLogin,
    LlmNearAiWalletLogin,
    LlmCodexLogin,
    AdminUserCreate,
    AdminUserDeleteSecret,
    AutomationPause,
    AutomationResume,
    AutomationRename,
    AutomationDelete,
}

impl ProductOperationId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChannelInboundAdmit => "channel.admit_inbound",
            Self::CreateThread => "webui.create_thread",
            Self::SubmitTurn => "webui.submit_turn",
            Self::CancelRun => "webui.cancel_run",
            Self::ResolveGate => "webui.resolve_gate",
            Self::RetryRun => "webui.retry_run",
            Self::ProjectCreate => "webui.project_create",
            Self::ProjectFsRead => "webui.project_fs_read",
            Self::FsRead => "webui.fs_read",
            Self::AttachmentRead => "webui.attachment_read",
            Self::TraceAccountLoginLink => "webui.trace_account_login_link",
            Self::TraceHoldAuthorize => "webui.trace_hold_authorize",
            Self::OperatorConfigSetKey => "webui.operator_config_set_key",
            Self::OperatorServiceLifecycle => "webui.operator_service_lifecycle",
            Self::LlmTestConnection => "webui.llm_test_connection",
            Self::LlmListModels => "webui.llm_list_models",
            Self::LlmNearAiLogin => "webui.llm_nearai_login",
            Self::LlmNearAiWalletLogin => "webui.llm_nearai_wallet_login",
            Self::LlmCodexLogin => "webui.llm_codex_login",
            Self::AdminUserCreate => "webui.admin_user_create",
            Self::AdminUserDeleteSecret => "webui.admin_user_delete_secret",
            Self::AutomationPause => "webui.automation_pause",
            Self::AutomationResume => "webui.automation_resume",
            Self::AutomationRename => "webui.automation_rename",
            Self::AutomationDelete => "webui.automation_delete",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "channel.admit_inbound" => Some(Self::ChannelInboundAdmit),
            "webui.create_thread" => Some(Self::CreateThread),
            "webui.submit_turn" => Some(Self::SubmitTurn),
            "webui.cancel_run" => Some(Self::CancelRun),
            "webui.resolve_gate" => Some(Self::ResolveGate),
            "webui.retry_run" => Some(Self::RetryRun),
            "webui.project_create" => Some(Self::ProjectCreate),
            "webui.project_fs_read" => Some(Self::ProjectFsRead),
            "webui.fs_read" => Some(Self::FsRead),
            "webui.attachment_read" => Some(Self::AttachmentRead),
            "webui.trace_account_login_link" => Some(Self::TraceAccountLoginLink),
            "webui.trace_hold_authorize" => Some(Self::TraceHoldAuthorize),
            "webui.operator_config_set_key" => Some(Self::OperatorConfigSetKey),
            "webui.operator_service_lifecycle" => Some(Self::OperatorServiceLifecycle),
            "webui.llm_test_connection" => Some(Self::LlmTestConnection),
            "webui.llm_list_models" => Some(Self::LlmListModels),
            "webui.llm_nearai_login" => Some(Self::LlmNearAiLogin),
            "webui.llm_nearai_wallet_login" => Some(Self::LlmNearAiWalletLogin),
            "webui.llm_codex_login" => Some(Self::LlmCodexLogin),
            "webui.admin_user_create" => Some(Self::AdminUserCreate),
            "webui.admin_user_delete_secret" => Some(Self::AdminUserDeleteSecret),
            "webui.automation_pause" => Some(Self::AutomationPause),
            "webui.automation_resume" => Some(Self::AutomationResume),
            "webui.automation_rename" => Some(Self::AutomationRename),
            "webui.automation_delete" => Some(Self::AutomationDelete),
            _ => None,
        }
    }
}

/// Typed declaration for one ProductSurface operation.
///
/// Operations are the result-bearing sibling of API-only capability invocation:
/// the transport still sends an opaque command id plus JSON input, but handlers
/// keep request/response DTOs tied to the declaration instead of calling a
/// concrete facade method directly.
#[derive(Debug, PartialEq, Eq)]
pub struct ProductOperation<Params, Output> {
    pub id: ProductOperationId,
    _types: PhantomData<fn(Params) -> Output>,
}

impl<Params, Output> Clone for ProductOperation<Params, Output> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Params, Output> Copy for ProductOperation<Params, Output> {}

impl<Params, Output> ProductOperation<Params, Output> {
    pub const fn new(id: ProductOperationId) -> Self {
        Self {
            id,
            _types: PhantomData,
        }
    }

    pub fn request(&self, input: Params) -> Result<ProductOperationRequest, RebornServicesError>
    where
        Params: Serialize,
    {
        Ok(ProductOperationRequest {
            operation_id: self.id.as_str().to_string(),
            input: serde_json::to_value(input).map_err(RebornServicesError::internal_from)?,
            typed_input: None,
        })
    }
}

/// One registered, result-bearing product operation invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductOperationRequest {
    pub operation_id: String,
    pub input: serde_json::Value,
    #[serde(skip)]
    pub typed_input: Option<ProductOperationTypedInput>,
}

impl ProductOperationRequest {
    pub fn channel_inbound(request: ChannelInboundSurfaceRequest) -> Self {
        Self {
            operation_id: ProductOperationId::ChannelInboundAdmit.as_str().to_string(),
            input: serde_json::Value::Null,
            typed_input: Some(ProductOperationTypedInput::ChannelInbound(Box::new(
                request,
            ))),
        }
    }
}

/// Host-only typed operation input carried over the same ProductSurface command
/// conduit as JSON operations. This field is skipped for browser/API JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductOperationTypedInput {
    ChannelInbound(Box<ChannelInboundSurfaceRequest>),
}

/// One result-bearing product operation response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductOperationResponse {
    Json(serde_json::Value),
    ProjectFile(ProjectFsFile),
    Attachment(RebornAttachmentBytes),
    ChannelInbound(Box<ChannelInboundSurfaceOutcome>),
}

impl ProductOperationResponse {
    pub fn json<T: Serialize>(value: T) -> Result<Self, RebornServicesError> {
        Ok(Self::Json(
            serde_json::to_value(value).map_err(RebornServicesError::internal_from)?,
        ))
    }

    pub fn project_file(file: ProjectFsFile) -> Self {
        Self::ProjectFile(file)
    }

    pub fn attachment(bytes: RebornAttachmentBytes) -> Self {
        Self::Attachment(bytes)
    }

    pub fn channel_inbound(outcome: ChannelInboundSurfaceOutcome) -> Self {
        Self::ChannelInbound(Box::new(outcome))
    }

    pub fn into_json<T: DeserializeOwned>(self) -> Result<T, RebornServicesError> {
        match self {
            Self::Json(value) => {
                serde_json::from_value(value).map_err(RebornServicesError::internal_from)
            }
            Self::ProjectFile(_) | Self::Attachment(_) | Self::ChannelInbound(_) => Err(
                RebornServicesError::internal_from("operation returned non-JSON"),
            ),
        }
    }

    pub fn into_project_file(self) -> Result<ProjectFsFile, RebornServicesError> {
        match self {
            Self::ProjectFile(file) => Ok(file),
            Self::Json(_) | Self::Attachment(_) | Self::ChannelInbound(_) => Err(
                RebornServicesError::internal_from("operation returned non-file result"),
            ),
        }
    }

    pub fn into_attachment(self) -> Result<RebornAttachmentBytes, RebornServicesError> {
        match self {
            Self::Attachment(bytes) => Ok(bytes),
            Self::Json(_) | Self::ProjectFile(_) | Self::ChannelInbound(_) => Err(
                RebornServicesError::internal_from("operation returned non-attachment bytes"),
            ),
        }
    }

    pub fn into_channel_inbound(self) -> Result<ChannelInboundSurfaceOutcome, RebornServicesError> {
        match self {
            Self::ChannelInbound(outcome) => Ok(*outcome),
            Self::Json(_) | Self::ProjectFile(_) | Self::Attachment(_) => Err(
                RebornServicesError::internal_from("operation returned non-channel result"),
            ),
        }
    }
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
/// `ProductSurface` with feature-specific read methods.
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
