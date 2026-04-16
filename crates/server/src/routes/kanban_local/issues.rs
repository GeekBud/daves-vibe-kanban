use deployment::Deployment;
use api_types::{
    CreateIssueRequest, Issue, ListIssuesQuery, ListIssuesResponse, MutationResponse,
    SearchIssuesRequest, UpdateIssueRequest,
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

use super::fallback::db_to_api_issue;

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/issues", get(list_issues).post(create_issue))
        .route("/issues/search", post(search_issues))
        .route("/issues/{issue_id}", get(get_issue).post(update_issue))
        .route("/issues/bulk", post(bulk_update_issues))
}

async fn list_issues(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ListIssuesQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssuesResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanIssue::find_by_project(pool, query.project_id).await?;
    let issues: Vec<Issue> = rows.into_iter().map(db_to_api_issue).collect();
    let total_count = issues.len();
    Ok(ResponseJson(ApiResponse::success(ListIssuesResponse {
        issues,
        total_count,
        limit: total_count,
        offset: 0,
    })))
}

async fn search_issues(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<SearchIssuesRequest>,
) -> Result<ResponseJson<ApiResponse<ListIssuesResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let mut rows = db::models::kanban::KanbanIssue::find_by_project(pool, request.project_id).await?;

    // Simple in-memory filtering (SQLite is fast enough for local usage)
    if let Some(status_id) = request.status_id {
        rows.retain(|r| r.status_id == status_id);
    }
    if let Some(ref status_ids) = request.status_ids {
        rows.retain(|r| status_ids.contains(&r.status_id));
    }
    if let Some(ref priority) = request.priority {
        let p = format!("{:?}", priority).to_lowercase();
        rows.retain(|r| r.priority.as_ref() == Some(&p));
    }
    if let Some(ref search) = request.search {
        let s = search.to_lowercase();
        rows.retain(|r| r.title.to_lowercase().contains(&s));
    }
    if let Some(ref simple_id) = request.simple_id {
        rows.retain(|r| r.simple_id.as_ref() == Some(simple_id));
    }
    if let Some(ref parent_issue_id) = request.parent_issue_id {
        rows.retain(|r| r.parent_issue_id == Some(*parent_issue_id));
    }

    let total_count = rows.len();
    let issues: Vec<Issue> = rows.into_iter().map(db_to_api_issue).collect();
    Ok(ResponseJson(ApiResponse::success(ListIssuesResponse {
        issues,
        total_count,
        limit: total_count,
        offset: 0,
    })))
}

async fn get_issue(
    State(deployment): State<DeploymentImpl>,
    Path(issue_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<Issue>>, ApiError> {
    let pool = &deployment.db().pool;
    let row = db::models::kanban::KanbanIssue::find_by_id(pool, issue_id).await?;
    match row {
        Some(i) => Ok(ResponseJson(ApiResponse::success(db_to_api_issue(i)))),
        None => Err(ApiError::BadRequest(format!("{}: {}", "Issue", "Issue not found"))),
    }
}

fn api_priority_to_string(p: api_types::IssuePriority) -> String {
    use api_types::IssuePriority;
    match p {
        IssuePriority::Urgent => "urgent",
        IssuePriority::High => "high",
        IssuePriority::Medium => "medium",
        IssuePriority::Low => "low",
    }
    .to_string()
}

fn dt_to_string(dt: Option<chrono::DateTime<chrono::Utc>>) -> Option<String> {
    dt.map(|d| d.to_rfc3339())
}

async fn create_issue(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateIssueRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<Issue>>>, ApiError> {
    let pool = &deployment.db().pool;
    let data = db::models::kanban::CreateKanbanIssue {
        project_id: request.project_id,
        status_id: request.status_id,
        title: request.title,
        description: request.description,
        priority: request.priority.map(api_priority_to_string),
        start_date: dt_to_string(request.start_date),
        target_date: dt_to_string(request.target_date),
        completed_at: dt_to_string(request.completed_at),
        sort_order: Some(request.sort_order as i64),
        parent_issue_id: request.parent_issue_id,
        parent_issue_sort_order: request.parent_issue_sort_order.map(|v| v as i64),
        extension_metadata: Some(request.extension_metadata),
        creator_user_id: None,
    };
    let row = db::models::kanban::KanbanIssue::create(pool, &data).await?;
    let issue = db_to_api_issue(row);
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: issue.clone(),
        txid: 1,
    })))
}

async fn update_issue(
    State(deployment): State<DeploymentImpl>,
    Path(issue_id): Path<Uuid>,
    Json(request): Json<UpdateIssueRequest>,
) -> Result<ResponseJson<ApiResponse<MutationResponse<Issue>>>, ApiError> {
    let pool = &deployment.db().pool;
    let data = db::models::kanban::UpdateKanbanIssue {
        status_id: request.status_id,
        title: request.title,
        description: request.description,
        priority: request.priority.map(|p| p.map(api_priority_to_string)),
        start_date: request.start_date.map(dt_to_string),
        target_date: request.target_date.map(dt_to_string),
        completed_at: request.completed_at.map(dt_to_string),
        sort_order: request.sort_order.map(|v| v as i64),
        parent_issue_id: request.parent_issue_id,
        parent_issue_sort_order: request.parent_issue_sort_order.map(|v| v.map(|x| x as i64)),
        extension_metadata: request.extension_metadata,
    };
    let row = db::models::kanban::KanbanIssue::update(pool, issue_id, &data).await?;
    let issue = db_to_api_issue(row);
    Ok(ResponseJson(ApiResponse::success(MutationResponse {
        data: issue.clone(),
        txid: 1,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct BulkUpdateIssuesRequest {
    pub updates: Vec<BulkUpdateIssueItem>,
}

#[derive(Debug, serde::Deserialize)]
pub struct BulkUpdateIssueItem {
    pub id: Uuid,
    #[serde(flatten)]
    pub changes: UpdateIssueRequest,
}

async fn bulk_update_issues(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<BulkUpdateIssuesRequest>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    let pool = &deployment.db().pool;
    for item in request.updates {
        let data = db::models::kanban::UpdateKanbanIssue {
            status_id: item.changes.status_id,
            title: item.changes.title,
            description: item.changes.description,
            priority: item.changes.priority.map(|p| p.map(api_priority_to_string)),
            start_date: item.changes.start_date.map(dt_to_string),
            target_date: item.changes.target_date.map(dt_to_string),
            completed_at: item.changes.completed_at.map(dt_to_string),
            sort_order: item.changes.sort_order.map(|v| v as i64),
            parent_issue_id: item.changes.parent_issue_id,
            parent_issue_sort_order: item.changes.parent_issue_sort_order.map(|v| v.map(|x| x as i64)),
            extension_metadata: item.changes.extension_metadata,
        };
        db::models::kanban::KanbanIssue::update(pool, item.id, &data).await?;
    }
    Ok(ResponseJson(ApiResponse::success(serde_json::json!({ "txid": 1 }))))
}
