use api_types::{
    CreateIssueCommentRequest, IssueComment, ListIssueCommentsResponse, MutationResponse,
    UpdateIssueCommentRequest,
};
use axum::{
    Router,
    extract::{Json, Path, State},
    response::Json as ResponseJson,
    routing::{delete, get, post},
};
use deployment::Deployment;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/issue_comments", get(list_issue_comments).post(create_issue_comment))
        .route("/issue_comments/{comment_id}", post(update_issue_comment).delete(delete_issue_comment))
}

#[derive(Debug, serde::Deserialize)]
pub struct ListIssueCommentsQuery {
    pub issue_id: Uuid,
}

async fn list_issue_comments(
    State(deployment): State<DeploymentImpl>,
    axum::extract::Query(query): axum::extract::Query<ListIssueCommentsQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueCommentsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanIssueComment::find_by_issue(pool, query.issue_id).await?;
    let issue_comments: Vec<IssueComment> = rows.into_iter().map(|c| IssueComment {
        id: c.id,
        issue_id: c.issue_id,
        author_id: c.author_id,
        parent_id: c.parent_id,
        message: c.message,
        created_at: c.created_at,
        updated_at: c.updated_at,
    }).collect();
    Ok(ResponseJson(ApiResponse::success(ListIssueCommentsResponse { issue_comments })))
}

async fn create_issue_comment(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateIssueCommentRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<IssueComment>>>, ApiError> {
    let pool = &deployment.db().pool;
    let id = request.id.unwrap_or_else(Uuid::new_v4);
    let now = chrono::Utc::now();
    sqlx::query!(
        "INSERT INTO kanban_issue_comments (id, issue_id, author_id, parent_id, message, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6)",
        id,
        request.issue_id,
        None::<Uuid>,
        request.parent_id,
        request.message,
        now
    )
    .execute(pool)
    .await?;
    let comment = IssueComment {
        id,
        issue_id: request.issue_id,
        author_id: None,
        parent_id: request.parent_id,
        message: request.message,
        created_at: now,
        updated_at: now,
    };
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: comment,
        txid: 1,
    })))
}

async fn update_issue_comment(
    State(deployment): State<DeploymentImpl>,
    Path(comment_id): Path<Uuid>,
    Json(request): Json<UpdateIssueCommentRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<IssueComment>>>, ApiError> {
    let pool = &deployment.db().pool;
    let existing = sqlx::query_as!(
        db::models::kanban::KanbanIssueComment,
        r#"SELECT id as "id!: Uuid",
                  issue_id as "issue_id!: Uuid",
                  author_id as "author_id: Uuid",
                  parent_id as "parent_id: Uuid",
                  message,
                  created_at as "created_at!: chrono::DateTime<chrono::Utc>",
                  updated_at as "updated_at!: chrono::DateTime<chrono::Utc>"
           FROM kanban_issue_comments
           WHERE id = $1"#,
        comment_id
    )
    .fetch_one(pool)
    .await?;

    let message = request.message.as_ref().unwrap_or(&existing.message);
    let parent_id = request.parent_id.as_ref().map(|v| *v).flatten().or(existing.parent_id);
    let now = chrono::Utc::now();

    let row = sqlx::query_as!(
        db::models::kanban::KanbanIssueComment,
        r#"UPDATE kanban_issue_comments
           SET message = $2, parent_id = $3, updated_at = $4
           WHERE id = $1
           RETURNING id as "id!: Uuid",
                     issue_id as "issue_id!: Uuid",
                     author_id as "author_id: Uuid",
                     parent_id as "parent_id: Uuid",
                     message,
                     created_at as "created_at!: chrono::DateTime<chrono::Utc>",
                     updated_at as "updated_at!: chrono::DateTime<chrono::Utc>""#,
        comment_id,
        message,
        parent_id,
        now
    )
    .fetch_one(pool)
    .await?;

    let comment = IssueComment {
        id: row.id,
        issue_id: row.issue_id,
        author_id: row.author_id,
        parent_id: row.parent_id,
        message: row.message,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: comment,
        txid: 1,
    })))
}

async fn delete_issue_comment(
    State(deployment): State<DeploymentImpl>,
    Path(comment_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let pool = &deployment.db().pool;
    sqlx::query!("DELETE FROM kanban_issue_comments WHERE id = $1", comment_id)
        .execute(pool)
        .await?;
    Ok(ResponseJson(ApiResponse::success(())))
}
