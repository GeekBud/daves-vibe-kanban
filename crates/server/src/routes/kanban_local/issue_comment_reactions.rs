use api_types::{
    CreateIssueCommentReactionRequest, IssueCommentReaction, ListIssueCommentReactionsResponse,
    MutationResponse, UpdateIssueCommentReactionRequest,
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
        .route("/issue_comment_reactions", get(list_reactions).post(create_reaction))
        .route("/issue_comment_reactions/{reaction_id}", post(update_reaction).delete(delete_reaction))
}

#[derive(Debug, serde::Deserialize)]
pub struct ListReactionsQuery {
    pub comment_id: Uuid,
}

async fn list_reactions(
    State(deployment): State<DeploymentImpl>,
    axum::extract::Query(query): axum::extract::Query<ListReactionsQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueCommentReactionsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanIssueCommentReaction::find_by_comment(pool, query.comment_id).await?;
    let issue_comment_reactions: Vec<IssueCommentReaction> = rows.into_iter().map(|r| IssueCommentReaction {
        id: r.id,
        comment_id: r.comment_id,
        user_id: r.user_id,
        emoji: r.emoji,
        created_at: r.created_at,
    }).collect();
    Ok(ResponseJson(ApiResponse::success(ListIssueCommentReactionsResponse { issue_comment_reactions })))
}

async fn create_reaction(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateIssueCommentReactionRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<IssueCommentReaction>>>, ApiError> {
    let pool = &deployment.db().pool;
    let id = request.id.unwrap_or_else(Uuid::new_v4);
    let now = chrono::Utc::now();
    let user_id = deployment.user_id().parse().ok().unwrap_or_else(Uuid::new_v4);
    sqlx::query!(
        "INSERT INTO kanban_issue_comment_reactions (id, comment_id, user_id, emoji, created_at) VALUES ($1, $2, $3, $4, $5)",
        id,
        request.comment_id,
        user_id,
        request.emoji,
        now
    )
    .execute(pool)
    .await?;
    let reaction = IssueCommentReaction {
        id,
        comment_id: request.comment_id,
        user_id,
        emoji: request.emoji,
        created_at: now,
    };
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: reaction,
        txid: 1,
    })))
}

async fn update_reaction(
    State(deployment): State<DeploymentImpl>,
    Path(reaction_id): Path<Uuid>,
    Json(request): Json<UpdateIssueCommentReactionRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<IssueCommentReaction>>>, ApiError> {
    let pool = &deployment.db().pool;
    let existing = sqlx::query_as!(
        db::models::kanban::KanbanIssueCommentReaction,
        r#"SELECT id as "id!: Uuid",
                  comment_id as "comment_id!: Uuid",
                  user_id as "user_id!: Uuid",
                  emoji,
                  created_at as "created_at!: chrono::DateTime<chrono::Utc>"
           FROM kanban_issue_comment_reactions
           WHERE id = $1"#,
        reaction_id
    )
    .fetch_one(pool)
    .await?;

    let emoji = request.emoji.as_ref().unwrap_or(&existing.emoji);
    let row = sqlx::query_as!(
        db::models::kanban::KanbanIssueCommentReaction,
        r#"UPDATE kanban_issue_comment_reactions
           SET emoji = $2
           WHERE id = $1
           RETURNING id as "id!: Uuid",
                     comment_id as "comment_id!: Uuid",
                     user_id as "user_id!: Uuid",
                     emoji,
                     created_at as "created_at!: chrono::DateTime<chrono::Utc>""#,
        reaction_id,
        emoji
    )
    .fetch_one(pool)
    .await?;

    let reaction = IssueCommentReaction {
        id: row.id,
        comment_id: row.comment_id,
        user_id: row.user_id,
        emoji: row.emoji,
        created_at: row.created_at,
    };
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: reaction,
        txid: 1,
    })))
}

async fn delete_reaction(
    State(deployment): State<DeploymentImpl>,
    Path(reaction_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let pool = &deployment.db().pool;
    sqlx::query!("DELETE FROM kanban_issue_comment_reactions WHERE id = $1", reaction_id)
        .execute(pool)
        .await?;
    Ok(ResponseJson(ApiResponse::success(())))
}
