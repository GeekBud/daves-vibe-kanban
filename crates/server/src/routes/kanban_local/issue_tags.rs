use deployment::Deployment;
use api_types::{CreateIssueTagRequest, IssueTag, ListIssueTagsResponse, MutationResponse};
use axum::{
    Router,
    extract::{Json, Path, State},
    response::Json as ResponseJson,
    routing::{get, post},
};
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

use super::fallback::db_to_api_issue_tag;

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/issue_tags", get(list_issue_tags).post(create_issue_tag))
        .route("/issue_tags/{issue_tag_id}", get(get_issue_tag).delete(delete_issue_tag))
}

#[derive(Debug, serde::Deserialize)]
pub struct ListIssueTagsQuery {
    pub issue_id: Uuid,
}

async fn list_issue_tags(
    State(deployment): State<DeploymentImpl>,
    axum::extract::Query(query): axum::extract::Query<ListIssueTagsQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueTagsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanIssueTag::find_by_issue(pool, query.issue_id).await?;
    let issue_tags = rows.into_iter().map(db_to_api_issue_tag).collect();
    Ok(ResponseJson(ApiResponse::success(ListIssueTagsResponse { issue_tags })))
}

async fn get_issue_tag(
    State(deployment): State<DeploymentImpl>,
    Path(issue_tag_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<IssueTag>>, ApiError> {
    let pool = &deployment.db().pool;
    let row = sqlx::query_as!(
        db::models::kanban::KanbanIssueTag,
        r#"SELECT id as "id!: Uuid",
                  issue_id as "issue_id!: Uuid",
                  tag_id as "tag_id!: Uuid"
           FROM kanban_issue_tags
           WHERE id = $1"#,
        issue_tag_id
    )
    .fetch_optional(pool)
    .await?;
    match row {
        Some(t) => Ok(ResponseJson(ApiResponse::success(db_to_api_issue_tag(t)))),
        None => Err(ApiError::BadRequest(format!("{}: {}", "IssueTag", "Not found"))),
    }
}

async fn create_issue_tag(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateIssueTagRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<IssueTag>>>, ApiError> {
    let pool = &deployment.db().pool;
    let id = request.id.unwrap_or_else(Uuid::new_v4);
    sqlx::query!(
        "INSERT INTO kanban_issue_tags (id, issue_id, tag_id) VALUES ($1, $2, $3)",
        id,
        request.issue_id,
        request.tag_id
    )
    .execute(pool)
    .await?;
    let issue_tag = IssueTag {
        id,
        issue_id: request.issue_id,
        tag_id: request.tag_id,
    };
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: issue_tag,
        txid: 1,
    })))
}

async fn delete_issue_tag(
    State(deployment): State<DeploymentImpl>,
    Path(issue_tag_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let pool = &deployment.db().pool;
    sqlx::query!("DELETE FROM kanban_issue_tags WHERE id = $1", issue_tag_id)
        .execute(pool)
        .await?;
    Ok(ResponseJson(ApiResponse::success(())))
}
