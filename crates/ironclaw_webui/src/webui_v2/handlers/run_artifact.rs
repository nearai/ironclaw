use axum::Json;
use axum::extract::{Extension, Path, State};
use ironclaw_product_workflow::{
    IronClawRunArtifact, IronClawRunArtifactRequest, IronClawServicesError, IronClawViewQuery,
    RUN_ARTIFACT_VIEW, WebUiAuthenticatedCaller,
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
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(path): Path<RunArtifactPath>,
) -> Result<Json<IronClawRunArtifact>, WebUiV2HttpError> {
    let params = serde_json::to_value(IronClawRunArtifactRequest {
        thread_id: path.thread_id,
        run_id: path.run_id,
    })
    .map_err(IronClawServicesError::internal_from)?;
    let page = state
        .services()
        .query(
            caller,
            IronClawViewQuery {
                view_id: RUN_ARTIFACT_VIEW.id.to_string(),
                params,
                cursor: None,
            },
        )
        .await?;
    let artifact =
        serde_json::from_value(page.payload).map_err(IronClawServicesError::internal_from)?;
    Ok(Json(artifact))
}
