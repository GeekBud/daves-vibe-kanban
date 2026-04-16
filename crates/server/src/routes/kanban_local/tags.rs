use deployment::Deployment;
use api_types::{
    CreateTagRequest, ListTagsQuery, ListTagsResponse, MutationResponse, Tag, UpdateTagRequest,
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

use super::fallback::db_to_api_tag;

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/tags", get(list_tags).post(create_tag))
        .route("/tags/{tag_id}", get(get_tag))
        .route("/tags/{tag_id}", post(update_tag).delete(delete_tag))
}

async fn list_tags(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ListTagsQuery>,
) -> Result<ResponseJson<ApiResponse<ListTagsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanTag::find_by_project(pool, query.project_id).await?;
    let tags = rows.into_iter().map(db_to_api_tag).collect();
    Ok(ResponseJson(ApiResponse::success(ListTagsResponse { tags })))
}

async fn get_tag(
    State(deployment): State<DeploymentImpl>,
    Path(tag_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<Tag>>, ApiError> {
    // Tags don't have find_by_id in our model yet; use raw query or list + filter
    let pool = &deployment.db().pool;
    let rows = sqlx::query_as!(
        db::models::kanban::KanbanTag,
        r#"SELECT id as "id!: Uuid",
                  project_id as "project_id!: Uuid",
                  name, color
           FROM kanban_tags
           WHERE id = $1"#,
        tag_id
    )
    .fetch_optional(pool)
    .await?;
    match rows {
        Some(t) => Ok(ResponseJson(ApiResponse::success(db_to_api_tag(t)))),
        None => Err(ApiError::BadRequest(format!("{}: {}", "Tag", "Tag not found"))),
    }
}

async fn create_tag(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateTagRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<Tag>>>, ApiError> {
    let pool = &deployment.db().pool;
    let data = db::models::kanban::CreateKanbanTag {
        project_id: request.project_id,
        name: request.name,
        color: Some(request.color),
    };
    let row = db::models::kanban::KanbanTag::create(pool, &data).await?;
    let tag = db_to_api_tag(row);
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: tag.clone(),
        txid: 1,
    })))
}

async fn update_tag(
    State(deployment): State<DeploymentImpl>,
    Path(tag_id): Path<Uuid>,
    Json(request): Json<UpdateTagRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<Tag>>>, ApiError> {
    let pool = &deployment.db().pool;
    let existing = sqlx::query_as!(
        db::models::kanban::KanbanTag,
        r#"SELECT id as "id!: Uuid",
                  project_id as "project_id!: Uuid",
                  name, color
           FROM kanban_tags
           WHERE id = $1"#,
        tag_id
    )
    .fetch_one(pool)
    .await?;

    let name = request.name.as_ref().unwrap_or(&existing.name);
    let color = request.color.as_ref().unwrap_or(&existing.color);

    let row = sqlx::query_as!(
        db::models::kanban::KanbanTag,
        r#"UPDATE kanban_tags
           SET name = $2, color = $3
           WHERE id = $1
           RETURNING id as "id!: Uuid",
                     project_id as "project_id!: Uuid",
                     name, color"#,
        tag_id,
        name,
        color
    )
    .fetch_one(pool)
    .await?;

    let tag = db_to_api_tag(row);
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: tag.clone(),
        txid: 1,
    })))
}

async fn delete_tag(
    State(deployment): State<DeploymentImpl>,
    Path(tag_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let pool = &deployment.db().pool;
    db::models::kanban::KanbanTag::delete(pool, tag_id).await?;
    Ok(ResponseJson(ApiResponse::success(())))
}
