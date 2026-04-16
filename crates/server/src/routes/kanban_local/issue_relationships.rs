use deployment::Deployment;
use api_types::{
    CreateIssueRelationshipRequest, IssueRelationship, ListIssueRelationshipsResponse,
    MutationResponse,
};
use axum::{
    Router,
    extract::{Json, Path, State},
    response::Json as ResponseJson,
    routing::{delete, get, post},
};
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

use super::fallback::db_to_api_relationship;

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/issue_relationships", get(list_issue_relationships).post(create_issue_relationship))
        .route("/issue_relationships/{relationship_id}", delete(delete_issue_relationship))
}

#[derive(Debug, serde::Deserialize)]
pub struct ListIssueRelationshipsQuery {
    pub issue_id: Uuid,
}

async fn list_issue_relationships(
    State(deployment): State<DeploymentImpl>,
    axum::extract::Query(query): axum::extract::Query<ListIssueRelationshipsQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueRelationshipsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanIssueRelationship::find_by_issue(pool, query.issue_id).await?;
    let issue_relationships = rows.into_iter().map(db_to_api_relationship).collect();
    Ok(ResponseJson(ApiResponse::success(ListIssueRelationshipsResponse { issue_relationships })))
}

async fn create_issue_relationship(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateIssueRelationshipRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<IssueRelationship>>>, ApiError> {
    let pool = &deployment.db().pool;
    let id = request.id.unwrap_or_else(Uuid::new_v4);
    let relationship_type = match request.relationship_type {
        api_types::IssueRelationshipType::Blocking => "blocking",
        api_types::IssueRelationshipType::HasDuplicate => "has_duplicate",
        api_types::IssueRelationshipType::Related => "related",
    };
    let created_at = chrono::Utc::now();
    sqlx::query!(
        "INSERT INTO kanban_issue_relationships (id, issue_id, related_issue_id, relationship_type, created_at) VALUES ($1, $2, $3, $4, $5)",
        id,
        request.issue_id,
        request.related_issue_id,
        relationship_type,
        created_at
    )
    .execute(pool)
    .await?;
    let relationship = IssueRelationship {
        id,
        issue_id: request.issue_id,
        related_issue_id: request.related_issue_id,
        relationship_type: request.relationship_type,
        created_at,
    };
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: relationship,
        txid: 1,
    })))
}

async fn delete_issue_relationship(
    State(deployment): State<DeploymentImpl>,
    Path(relationship_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let pool = &deployment.db().pool;
    sqlx::query!("DELETE FROM kanban_issue_relationships WHERE id = $1", relationship_id)
        .execute(pool)
        .await?;
    Ok(ResponseJson(ApiResponse::success(())))
}
