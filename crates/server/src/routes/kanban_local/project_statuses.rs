use deployment::Deployment;
use api_types::{
    CreateProjectStatusRequest, ListProjectStatusesQuery, ListProjectStatusesResponse,
    MutationResponse, ProjectStatus, UpdateProjectStatusRequest,
};
use axum::{
    Router,
    extract::{Json, Path, Query, State},
    response::Json as ResponseJson,
    routing::{get, post},
};
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

use super::fallback::db_to_api_status;

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/project_statuses", get(list_project_statuses).post(create_project_status))
        .route("/project_statuses/{status_id}", post(update_project_status))
        .route("/project_statuses/bulk", post(bulk_update_project_statuses))
}

async fn list_project_statuses(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ListProjectStatusesQuery>,
) -> Result<ResponseJson<ApiResponse<ListProjectStatusesResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanProjectStatus::find_by_project(pool, query.project_id).await?;
    let project_statuses = rows.into_iter().map(db_to_api_status).collect();
    Ok(ResponseJson(ApiResponse::success(ListProjectStatusesResponse { project_statuses })))
}

async fn create_project_status(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateProjectStatusRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<ProjectStatus>>>, ApiError> {
    let pool = &deployment.db().pool;
    let data = db::models::kanban::CreateKanbanProjectStatus {
        project_id: request.project_id,
        name: request.name,
        color: Some(request.color),
        sort_order: request.sort_order as i64,
        hidden: Some(request.hidden),
    };
    let row = db::models::kanban::KanbanProjectStatus::create(pool, &data).await?;
    let status = db_to_api_status(row);
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: status.clone(),
        txid: 1,
    })))
}

async fn update_project_status(
    State(deployment): State<DeploymentImpl>,
    Path(status_id): Path<Uuid>,
    Json(request): Json<UpdateProjectStatusRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<ProjectStatus>>>, ApiError> {
    let pool = &deployment.db().pool;
    let existing = sqlx::query_as!(
        db::models::kanban::KanbanProjectStatus,
        r#"SELECT id as "id!: Uuid",
                  project_id as "project_id!: Uuid",
                  name, color, sort_order,
                  hidden as "hidden!: bool",
                  created_at as "created_at!: chrono::DateTime<chrono::Utc>"
           FROM kanban_project_statuses
           WHERE id = $1"#,
        status_id
    )
    .fetch_one(pool)
    .await?;

    let name = request.name.as_ref().unwrap_or(&existing.name);
    let color = request.color.as_ref().unwrap_or(&existing.color);
    let sort_order = request.sort_order.unwrap_or(existing.sort_order as i32) as i64;
    let hidden = request.hidden.unwrap_or(existing.hidden);

    let row = sqlx::query_as!(
        db::models::kanban::KanbanProjectStatus,
        r#"UPDATE kanban_project_statuses
           SET name = $2, color = $3, sort_order = $4, hidden = $5
           WHERE id = $1
           RETURNING id as "id!: Uuid",
                     project_id as "project_id!: Uuid",
                     name, color, sort_order,
                     hidden as "hidden!: bool",
                     created_at as "created_at!: chrono::DateTime<chrono::Utc>""#,
        status_id,
        name,
        color,
        sort_order,
        hidden
    )
    .fetch_one(pool)
    .await?;

    let status = db_to_api_status(row);
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: status.clone(),
        txid: 1,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct BulkUpdateProjectStatusesRequest {
    pub updates: Vec<BulkUpdateProjectStatusItem>,
}

#[derive(Debug, serde::Deserialize)]
pub struct BulkUpdateProjectStatusItem {
    pub id: Uuid,
    #[serde(flatten)]
    pub changes: UpdateProjectStatusRequest,
}

async fn bulk_update_project_statuses(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<BulkUpdateProjectStatusesRequest>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    let pool = &deployment.db().pool;
    for item in request.updates {
        let existing = sqlx::query_as!(
            db::models::kanban::KanbanProjectStatus,
            r#"SELECT id as "id!: Uuid",
                      project_id as "project_id!: Uuid",
                      name, color, sort_order,
                      hidden as "hidden!: bool",
                      created_at as "created_at!: chrono::DateTime<chrono::Utc>"
               FROM kanban_project_statuses
               WHERE id = $1"#,
            item.id
        )
        .fetch_one(pool)
        .await?;

        let name = item.changes.name.as_ref().unwrap_or(&existing.name);
        let color = item.changes.color.as_ref().unwrap_or(&existing.color);
        let sort_order = item.changes.sort_order.unwrap_or(existing.sort_order as i32) as i64;
        let hidden = item.changes.hidden.unwrap_or(existing.hidden);

        sqlx::query!(
            "UPDATE kanban_project_statuses SET name = $2, color = $3, sort_order = $4, hidden = $5 WHERE id = $1",
            item.id,
            name,
            color,
            sort_order,
            hidden
        )
        .execute(pool)
        .await?;
    }
    Ok(ResponseJson(ApiResponse::success(serde_json::json!({ "txid": 1 }))))
}
