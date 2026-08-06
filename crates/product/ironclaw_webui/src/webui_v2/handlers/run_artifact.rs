use axum::Json;
use axum::extract::{Extension, Path, State};
use ironclaw_assistant::{
    RUN_ARTIFACT_VIEW, RebornRunArtifact, RebornThreadArtifact, THREAD_ARTIFACT_VIEW,
};
use ironclaw_product_contracts::product_wire::{
    RebornRunArtifactRequest, RebornThreadArtifactRequest,
};
use ironclaw_product_contracts::surface::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceQueryRequest,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::webui_v2::error::WebUiV2HttpError;
use crate::webui_v2::router::WebUiV2State;

#[derive(Debug, Deserialize)]
pub struct RunArtifactPath {
    pub thread_id: String,
    pub run_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ThreadArtifactPath {
    pub thread_id: String,
}

async fn query_single<P, T>(
    state: &WebUiV2State,
    caller: ProductSurfaceCaller,
    view_id: &str,
    request: P,
) -> Result<T, WebUiV2HttpError>
where
    P: Serialize,
    T: DeserializeOwned,
{
    let input = serde_json::to_value(request).map_err(ProductSurfaceError::internal_from)?;
    let page = state
        .bind_services(caller)
        .query(ProductSurfaceQueryRequest {
            view_id: view_id.to_string(),
            input,
            cursor: None,
            limit: None,
        })
        .await?;
    let payload = page
        .items
        .into_iter()
        .next()
        .ok_or_else(ProductSurfaceError::internal)?;
    Ok(serde_json::from_value(payload).map_err(ProductSurfaceError::internal_from)?)
}

/// `GET /api/webchat/v2/threads/{thread_id}/runs/{run_id}/artifact`
pub async fn get_run_artifact(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Path(path): Path<RunArtifactPath>,
) -> Result<Json<RebornRunArtifact>, WebUiV2HttpError> {
    Ok(Json(
        query_single(
            &state,
            caller,
            RUN_ARTIFACT_VIEW.id,
            RebornRunArtifactRequest {
                thread_id: path.thread_id,
                run_id: path.run_id,
            },
        )
        .await?,
    ))
}

/// `GET /api/webchat/v2/threads/{thread_id}/artifact`
pub async fn get_thread_artifact(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Path(path): Path<ThreadArtifactPath>,
) -> Result<Json<RebornThreadArtifact>, WebUiV2HttpError> {
    Ok(Json(
        query_single(
            &state,
            caller,
            THREAD_ARTIFACT_VIEW.id,
            RebornThreadArtifactRequest {
                thread_id: path.thread_id,
            },
        )
        .await?,
    ))
}
