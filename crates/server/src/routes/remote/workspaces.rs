use api_types::Workspace;
use axum::{
    Router,
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::get,
};
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

pub(super) fn router() -> Router<DeploymentImpl> {
    Router::new().route(
        "/workspaces/by-local-id/{local_workspace_id}",
        get(get_workspace_by_local_id),
    )
}

async fn get_workspace_by_local_id(
    State(deployment): State<DeploymentImpl>,
    Path(local_workspace_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<Option<Workspace>>>, ApiError> {
    let client = match deployment.remote_client() {
        Ok(c) => c,
        Err(_) => return Ok(ResponseJson(ApiResponse::success(None))),
    };
    match client.get_workspace_by_local_id(local_workspace_id).await {
        Ok(workspace) => Ok(ResponseJson(ApiResponse::success(Some(workspace)))),
        Err(_) => Ok(ResponseJson(ApiResponse::success(None))),
    }
}
