use deployment::Deployment;
use api_types::{
    CreateIssueAssigneeRequest, IssueAssignee, ListIssueAssigneesResponse, MutationResponse,
};
use axum::{
    Router,
    extract::{Json, Path, State},
    response::Json as ResponseJson,
    routing::{get, post},
};
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

use super::fallback::db_to_api_assignee;

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/issue_assignees", get(list_issue_assignees).post(create_issue_assignee))
        .route("/issue_assignees/{issue_assignee_id}", get(get_issue_assignee).delete(delete_issue_assignee))
}

#[derive(Debug, serde::Deserialize)]
pub struct ListIssueAssigneesQuery {
    pub issue_id: Uuid,
}

async fn list_issue_assignees(
    State(deployment): State<DeploymentImpl>,
    axum::extract::Query(query): axum::extract::Query<ListIssueAssigneesQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueAssigneesResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanIssueAssignee::find_by_issue(pool, query.issue_id).await?;
    let issue_assignees = rows.into_iter().map(db_to_api_assignee).collect();
    Ok(ResponseJson(ApiResponse::success(ListIssueAssigneesResponse { issue_assignees })))
}

async fn get_issue_assignee(
    State(deployment): State<DeploymentImpl>,
    Path(issue_assignee_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<IssueAssignee>>, ApiError> {
    let pool = &deployment.db().pool;
    let row = sqlx::query_as!(
        db::models::kanban::KanbanIssueAssignee,
        r#"SELECT id as "id!: Uuid",
                  issue_id as "issue_id!: Uuid",
                  user_id as "user_id!: Uuid",
                  assigned_at as "assigned_at!: chrono::DateTime<chrono::Utc>"
           FROM kanban_issue_assignees
           WHERE id = $1"#,
        issue_assignee_id
    )
    .fetch_optional(pool)
    .await?;
    match row {
        Some(a) => Ok(ResponseJson(ApiResponse::success(db_to_api_assignee(a)))),
        None => Err(ApiError::BadRequest(format!("{}: {}", "IssueAssignee", "Not found"))),
    }
}

async fn create_issue_assignee(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateIssueAssigneeRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<IssueAssignee>>>, ApiError> {
    let pool = &deployment.db().pool;
    let id = request.id.unwrap_or_else(Uuid::new_v4);
    let assigned_at = chrono::Utc::now();
    sqlx::query!(
        "INSERT INTO kanban_issue_assignees (id, issue_id, user_id, assigned_at) VALUES ($1, $2, $3, $4)",
        id,
        request.issue_id,
        request.user_id,
        assigned_at
    )
    .execute(pool)
    .await?;
    let assignee = IssueAssignee {
        id,
        issue_id: request.issue_id,
        user_id: request.user_id,
        assigned_at,
    };
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: assignee,
        txid: 1,
    })))
}

async fn delete_issue_assignee(
    State(deployment): State<DeploymentImpl>,
    Path(issue_assignee_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let pool = &deployment.db().pool;
    sqlx::query!("DELETE FROM kanban_issue_assignees WHERE id = $1", issue_assignee_id)
        .execute(pool)
        .await?;
    Ok(ResponseJson(ApiResponse::success(())))
}
