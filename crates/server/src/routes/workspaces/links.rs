use api_types::{CreateWorkspaceRequest, PullRequestStatus, UpsertPullRequestRequest};
use axum::{
    Extension, Json, Router,
    extract::{Path as AxumPath, State},
    middleware::from_fn_with_state,
    response::Json as ResponseJson,
    routing::{delete, post},
};
use db::models::{
    kanban::{CreateKanbanWorkspace, KanbanWorkspace},
    merge::MergeStatus,
    pull_request::PullRequest,
    workspace::Workspace,
};
use deployment::Deployment;
use serde::Deserialize;
use services::services::remote_client::RemoteClientError;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError, middleware::load_workspace_middleware};

#[derive(Debug, Deserialize)]
pub struct LinkWorkspaceRequest {
    pub project_id: Uuid,
    pub issue_id: Uuid,
}

pub async fn link_workspace(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<LinkWorkspaceRequest>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let owner_user_id = Uuid::parse_str(deployment.user_id()).unwrap_or_else(|_| Uuid::nil());
    KanbanWorkspace::create_or_replace(
        &deployment.db().pool,
        &CreateKanbanWorkspace {
            project_id: payload.project_id,
            owner_user_id,
            issue_id: Some(payload.issue_id),
            local_workspace_id: Some(workspace.id),
            name: workspace.name.clone(),
            archived: workspace.archived,
            files_changed: None,
            lines_added: None,
            lines_removed: None,
        },
    )
    .await?;

    if let Ok(client) = deployment.remote_client() {
        if let Err(e) = client
            .create_workspace(CreateWorkspaceRequest {
                project_id: payload.project_id,
                local_workspace_id: workspace.id,
                issue_id: payload.issue_id,
                name: workspace.name.clone(),
                archived: Some(workspace.archived),
                files_changed: None,
                lines_added: None,
                lines_removed: None,
            })
            .await
        {
            tracing::warn!("Failed to sync workspace link to remote: {}", e);
        } else {
            let pool = deployment.db().pool.clone();
            let ws_id = workspace.id;
            let client = client.clone();
            tokio::spawn(async move {
                let pull_requests = match PullRequest::find_by_workspace_id(&pool, ws_id).await {
                    Ok(prs) => prs,
                    Err(e) => {
                        tracing::error!(
                            "Failed to fetch PRs for workspace {} during link: {}",
                            ws_id,
                            e
                        );
                        return;
                    }
                };
                for pr in pull_requests {
                    let pr_status = match pr.pr_status {
                        MergeStatus::Open => PullRequestStatus::Open,
                        MergeStatus::Merged => PullRequestStatus::Merged,
                        MergeStatus::Closed => PullRequestStatus::Closed,
                        MergeStatus::Unknown => continue,
                    };
                    let _ = (pr_status, &client);
                }
            });
        }
    }

    Ok(ResponseJson(ApiResponse::success(())))
}

pub async fn unlink_workspace(
    AxumPath(workspace_id): AxumPath<uuid::Uuid>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    sqlx::query!(
        "DELETE FROM kanban_workspaces WHERE local_workspace_id = $1",
        workspace_id
    )
    .execute(&deployment.db().pool)
    .await
    .ok();

    if let Ok(client) = deployment.remote_client() {
        match client.delete_workspace(workspace_id).await {
            Ok(()) => {}
            Err(RemoteClientError::Http { status: 404, .. }) => {}
            Err(e) => return Err(e.into()),
        }
    }

    Ok(ResponseJson(ApiResponse::success(())))
}

pub fn router(deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    let post_router = Router::new()
        .route("/", post(link_workspace))
        .layer(from_fn_with_state(
            deployment.clone(),
            load_workspace_middleware,
        ));

    let delete_router = Router::new().route("/", delete(unlink_workspace));

    post_router.merge(delete_router)
}
