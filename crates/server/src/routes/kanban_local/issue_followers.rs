use api_types::{
    CreateIssueFollowerRequest, IssueFollower, ListIssueFollowersResponse, MutationResponse,
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
        .route("/issue_followers", get(list_issue_followers).post(create_issue_follower))
        .route("/issue_followers/{follower_id}", delete(delete_issue_follower))
}

#[derive(Debug, serde::Deserialize)]
pub struct ListIssueFollowersQuery {
    pub issue_id: Uuid,
}

async fn list_issue_followers(
    State(deployment): State<DeploymentImpl>,
    axum::extract::Query(query): axum::extract::Query<ListIssueFollowersQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueFollowersResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanIssueFollower::find_by_issue(pool, query.issue_id).await?;
    let issue_followers: Vec<IssueFollower> = rows.into_iter().map(|f| IssueFollower {
        id: f.id,
        issue_id: f.issue_id,
        user_id: f.user_id,
    }).collect();
    Ok(ResponseJson(ApiResponse::success(ListIssueFollowersResponse { issue_followers })))
}

async fn create_issue_follower(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateIssueFollowerRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<IssueFollower>>>, ApiError> {
    let pool = &deployment.db().pool;
    let id = request.id.unwrap_or_else(Uuid::new_v4);
    sqlx::query!(
        "INSERT INTO kanban_issue_followers (id, issue_id, user_id) VALUES ($1, $2, $3)",
        id,
        request.issue_id,
        request.user_id
    )
    .execute(pool)
    .await?;
    let follower = IssueFollower {
        id,
        issue_id: request.issue_id,
        user_id: request.user_id,
    };
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: follower,
        txid: 1,
    })))
}

async fn delete_issue_follower(
    State(deployment): State<DeploymentImpl>,
    Path(follower_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let pool = &deployment.db().pool;
    sqlx::query!("DELETE FROM kanban_issue_followers WHERE id = $1", follower_id)
        .execute(pool)
        .await?;
    Ok(ResponseJson(ApiResponse::success(())))
}
