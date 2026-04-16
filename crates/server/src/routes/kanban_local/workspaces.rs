use deployment::Deployment;
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

use super::fallback::db_to_api_workspace;

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/workspaces/by-local-id/{local_workspace_id}", get(get_workspace_by_local_id))
}

async fn get_workspace_by_local_id(
    State(deployment): State<DeploymentImpl>,
    Path(local_workspace_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<Workspace>>, ApiError> {
    let pool = &deployment.db().pool;
    let row = sqlx::query_as!(
        db::models::kanban::KanbanWorkspace,
        r#"SELECT id as "id!: Uuid",
                  project_id as "project_id!: Uuid",
                  owner_user_id as "owner_user_id!: Uuid",
                  issue_id as "issue_id: Uuid",
                  local_workspace_id as "local_workspace_id: Uuid",
                  name,
                  archived as "archived!: bool",
                  files_changed, lines_added, lines_removed,
                  created_at as "created_at!: chrono::DateTime<chrono::Utc>",
                  updated_at as "updated_at!: chrono::DateTime<chrono::Utc>"
           FROM kanban_workspaces
           WHERE local_workspace_id = $1"#,
        local_workspace_id
    )
    .fetch_optional(pool)
    .await?;
    match row {
        Some(w) => Ok(ResponseJson(ApiResponse::success(db_to_api_workspace(w)))),
        None => Err(ApiError::BadRequest(format!("{}: {}", "Workspace", "Workspace not found"))),
    }
}
