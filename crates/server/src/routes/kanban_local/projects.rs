use deployment::Deployment;
use api_types::{
    CreateProjectRequest, ListProjectsQuery, ListProjectsResponse, MutationResponse, Project,
    UpdateProjectRequest,
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

use super::fallback::db_to_api_project;

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{project_id}", get(get_project))
        .route("/projects/{project_id}", post(update_project))
        .route("/projects/bulk", post(bulk_update_projects))
}

async fn list_projects(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ListProjectsQuery>,
) -> Result<ResponseJson<ApiResponse<ListProjectsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanProject::find_by_organization(pool, query.organization_id).await?;
    let projects = rows.into_iter().map(db_to_api_project).collect();
    Ok(ResponseJson(ApiResponse::success(ListProjectsResponse { projects })))
}

async fn get_project(
    State(deployment): State<DeploymentImpl>,
    Path(project_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<Project>>, ApiError> {
    let pool = &deployment.db().pool;
    let row = db::models::kanban::KanbanProject::find_by_id(pool, project_id).await?;
    match row {
        Some(p) => Ok(ResponseJson(ApiResponse::success(db_to_api_project(p)))),
        None => Err(ApiError::BadRequest(format!("{}: {}", "Project", "Project not found"))),
    }
}

async fn create_project(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateProjectRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<Project>>>, ApiError> {
    let pool = &deployment.db().pool;
    let data = db::models::kanban::CreateKanbanProject {
        organization_id: request.organization_id,
        name: request.name,
        color: Some(request.color),
    };
    let row = db::models::kanban::KanbanProject::create(pool, &data).await?;
    let project = db_to_api_project(row);
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: project.clone(),
        txid: 1,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateProjectPath {
    #[serde(flatten)]
    pub request: UpdateProjectRequest,
}

async fn update_project(
    State(deployment): State<DeploymentImpl>,
    Path(project_id): Path<Uuid>,
    Json(request): Json<UpdateProjectRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<Project>>>, ApiError> {
    let pool = &deployment.db().pool;
    let data = db::models::kanban::UpdateKanbanProject {
        name: request.name,
        color: request.color,
        sort_order: request.sort_order.map(|v| v as i64),
    };
    let row = db::models::kanban::KanbanProject::update(pool, project_id, &data).await?;
    let project = db_to_api_project(row);
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: project.clone(),
        txid: 1,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct BulkUpdateProjectsRequest {
    pub updates: Vec<BulkUpdateProjectItem>,
}

#[derive(Debug, serde::Deserialize)]
pub struct BulkUpdateProjectItem {
    pub id: Uuid,
    #[serde(flatten)]
    pub changes: UpdateProjectRequest,
}

async fn bulk_update_projects(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<BulkUpdateProjectsRequest>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    let pool = &deployment.db().pool;
    for item in request.updates {
        let data = db::models::kanban::UpdateKanbanProject {
            name: item.changes.name,
            color: item.changes.color,
            sort_order: item.changes.sort_order.map(|v| v as i64),
        };
        db::models::kanban::KanbanProject::update(pool, item.id, &data).await?;
    }
    Ok(ResponseJson(ApiResponse::success(serde_json::json!({ "txid": 1 }))))
}
