use axum::Json;
use axum::extract::{Extension, Path, State};
use ironclaw_product::{
    ProductSurfaceCaller, ProductSurfaceError, RUN_ARTIFACT_VIEW, RebornRunArtifact,
    RebornRunArtifactRequest,
};
use serde::Deserialize;

use crate::webui_v2::error::WebUiV2HttpError;
use crate::webui_v2::router::WebUiV2State;

#[derive(Debug, Deserialize)]
pub struct RunArtifactPath {
    pub thread_id: String,
    pub run_id: String,
}

/// `GET /api/webchat/v2/threads/{thread_id}/runs/{run_id}/artifact`
pub async fn get_run_artifact(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Path(path): Path<RunArtifactPath>,
) -> Result<Json<RebornRunArtifact>, WebUiV2HttpError> {
    let params = serde_json::to_value(RebornRunArtifactRequest {
        thread_id: path.thread_id,
        run_id: path.run_id,
    })
    .map_err(ProductSurfaceError::internal_from)?;
    let surface = state.bind_services(caller);
    let page = surface
        .query(ironclaw_host_api::ProductSurfaceQueryRequest {
            view_id: RUN_ARTIFACT_VIEW.id.to_string(),
            input: params,
            cursor: None,
            limit: None,
        })
        .await?;
    let payload = page
        .items
        .into_iter()
        .next()
        .ok_or_else(ProductSurfaceError::internal)?;
    let artifact = serde_json::from_value(payload).map_err(ProductSurfaceError::internal_from)?;
    Ok(Json(artifact))
}
