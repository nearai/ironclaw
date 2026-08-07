use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use ironclaw_assistant::{
    ADMIN_THREAD_SCRAPE_ARTIFACT_VIEW, ADMIN_THREAD_SCRAPE_RUN_ARTIFACT_VIEW,
    ADMIN_THREAD_SCRAPE_THREADS_VIEW, RUN_ARTIFACT_VIEW, RebornListThreadsResponse,
    RebornRunArtifact, RebornThreadArtifact, THREAD_ARTIFACT_VIEW,
};
use ironclaw_product_contracts::product_wire::{
    RebornAdminThreadScrapeArtifactRequest, RebornAdminThreadScrapeListRequest,
    RebornAdminThreadScrapeRunArtifactRequest, RebornRunArtifactRequest,
    RebornThreadArtifactRequest,
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

#[derive(Debug, Deserialize)]
pub struct AdminThreadScrapeListQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdminThreadScrapeThreadPath {
    pub user_id: String,
    pub thread_id: String,
}

#[derive(Debug, Deserialize)]
pub struct AdminThreadScrapeRunPath {
    pub user_id: String,
    pub thread_id: String,
    pub run_id: String,
}

/// `GET /api/webchat/v2/admin/users/{user_id}/thread-scrape/threads`
pub async fn admin_list_thread_scrape_threads(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Path(user_id): Path<String>,
    Query(query): Query<AdminThreadScrapeListQuery>,
) -> Result<Json<RebornListThreadsResponse>, WebUiV2HttpError> {
    let user_id = super::parse_admin_user_id(user_id)?;
    let surface = state.bind_services(caller);
    // Sibling handlers `take()` the cursor so it travels one wire slot: the
    // transport page cursor, which the dispatch arm merges back into the
    // request before the list call.
    let mut query = query;
    let cursor = query.cursor.take();
    let response = ADMIN_THREAD_SCRAPE_THREADS_VIEW
        .query_on(
            &surface,
            RebornAdminThreadScrapeListRequest {
                user_id,
                limit: query.limit,
                cursor: None,
            },
            cursor,
        )
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/admin/users/{user_id}/thread-scrape/threads/{thread_id}/artifact`
pub async fn admin_get_thread_scrape_artifact(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Path(path): Path<AdminThreadScrapeThreadPath>,
) -> Result<Json<RebornThreadArtifact>, WebUiV2HttpError> {
    let user_id = super::parse_admin_user_id(path.user_id)?;
    let thread_id = path.thread_id;
    let artifact = query_single(
        &state,
        caller,
        ADMIN_THREAD_SCRAPE_ARTIFACT_VIEW.id,
        RebornAdminThreadScrapeArtifactRequest { user_id, thread_id },
    )
    .await?;
    Ok(Json(artifact))
}

/// `GET /api/webchat/v2/admin/users/{user_id}/thread-scrape/threads/{thread_id}/runs/{run_id}/artifact`
pub async fn admin_get_thread_scrape_run_artifact(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Path(path): Path<AdminThreadScrapeRunPath>,
) -> Result<Json<RebornRunArtifact>, WebUiV2HttpError> {
    let user_id = super::parse_admin_user_id(path.user_id)?;
    let thread_id = path.thread_id;
    let run_id = path.run_id;
    let artifact = query_single(
        &state,
        caller,
        ADMIN_THREAD_SCRAPE_RUN_ARTIFACT_VIEW.id,
        RebornAdminThreadScrapeRunArtifactRequest {
            user_id,
            thread_id,
            run_id,
        },
    )
    .await?;
    Ok(Json(artifact))
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
