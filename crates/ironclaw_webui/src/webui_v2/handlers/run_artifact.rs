use axum::Json;
use axum::extract::{Extension, Path, State};
use ironclaw_product_workflow::{
    RUN_ARTIFACT_VIEW, RebornRunArtifact, RebornRunArtifactRequest, RebornServicesError,
    RebornThreadArtifact, RebornThreadArtifactRequest, RebornViewQuery, THREAD_ARTIFACT_VIEW,
    WebUiAuthenticatedCaller,
};
use serde::Deserialize;

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

/// `GET /api/webchat/v2/threads/{thread_id}/runs/{run_id}/artifact`
pub async fn get_run_artifact(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(path): Path<RunArtifactPath>,
) -> Result<Json<RebornRunArtifact>, WebUiV2HttpError> {
    let params = serde_json::to_value(RebornRunArtifactRequest {
        thread_id: path.thread_id,
        run_id: path.run_id,
    })
    .map_err(RebornServicesError::internal_from)?;
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: RUN_ARTIFACT_VIEW.id.to_string(),
                params,
                cursor: None,
            },
        )
        .await?;
    let artifact =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    Ok(Json(artifact))
}

/// `GET /api/webchat/v2/threads/{thread_id}/artifact`
pub async fn get_thread_artifact(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(path): Path<ThreadArtifactPath>,
) -> Result<Json<RebornThreadArtifact>, WebUiV2HttpError> {
    let params = serde_json::to_value(RebornThreadArtifactRequest {
        thread_id: path.thread_id,
    })
    .map_err(RebornServicesError::internal_from)?;
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: THREAD_ARTIFACT_VIEW.id.to_string(),
                params,
                cursor: None,
            },
        )
        .await?;
    let artifact =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    Ok(Json(artifact))
}
