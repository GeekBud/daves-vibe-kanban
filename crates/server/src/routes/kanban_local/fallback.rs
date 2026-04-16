use api_types::{
    Issue, IssueAssignee, IssueComment, IssueCommentReaction, IssueFollower, IssueRelationship,
    IssueTag, ListIssueAssigneesResponse, ListIssueCommentReactionsResponse,
    ListIssueCommentsResponse, ListIssueFollowersResponse, ListIssueRelationshipsResponse,
    ListIssueTagsResponse, ListIssuesResponse, ListOrganizationsResponse,
    ListProjectStatusesResponse, ListProjectsResponse, ListTagsResponse, MemberRole,
    OrganizationWithRole, Project, ProjectStatus, Tag, Workspace,
};
use axum::{
    Router,
    extract::{Query, State},
    response::Json as ResponseJson,
    routing::get,
};
use chrono::{DateTime, Utc};
use deployment::Deployment;
use serde::Deserialize;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

#[derive(Debug, Deserialize)]
pub struct ProjectFallbackQuery {
    pub organization_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct ProjectScopedFallbackQuery {
    pub project_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct IssueScopedFallbackQuery {
    pub issue_id: Uuid,
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/fallback/organizations", get(fallback_organizations))
        .route("/fallback/projects", get(fallback_projects))
        .route("/fallback/issues", get(fallback_issues))
        .route("/fallback/tags", get(fallback_tags))
        .route("/fallback/project_statuses", get(fallback_project_statuses))
        .route("/fallback/issue_assignees", get(fallback_issue_assignees))
        .route("/fallback/issue_followers", get(fallback_issue_followers))
        .route("/fallback/issue_tags", get(fallback_issue_tags))
        .route(
            "/fallback/issue_relationships",
            get(fallback_issue_relationships),
        )
        .route("/fallback/issue_comments", get(fallback_issue_comments))
        .route(
            "/fallback/issue_comment_reactions",
            get(fallback_issue_comment_reactions),
        )
        .route("/fallback/workspaces", get(fallback_workspaces))
        .route(
            "/fallback/project_workspaces",
            get(fallback_project_workspaces),
        )
        .route("/fallback/user_workspaces", get(fallback_user_workspaces))
        .route("/fallback/notifications", get(fallback_notifications))
        .route(
            "/fallback/organization_members",
            get(fallback_organization_members),
        )
        .route("/fallback/users", get(fallback_users))
        .route("/fallback/pull_requests", get(fallback_pull_requests))
        .route(
            "/fallback/pull_request_issues",
            get(fallback_pull_request_issues),
        )
}

pub(super) fn db_to_api_project(p: db::models::kanban::KanbanProject) -> Project {
    Project {
        id: p.id,
        organization_id: p.organization_id,
        name: p.name,
        color: p.color,
        sort_order: p.sort_order as i32,
        created_at: p.created_at,
        updated_at: p.updated_at,
    }
}

pub(super) fn db_to_api_issue(i: db::models::kanban::KanbanIssue) -> Issue {
    use api_types::IssuePriority;
    use chrono::{DateTime, Utc};
    use serde_json::Value;

    let priority = i.priority.and_then(|p| match p.as_str() {
        "urgent" => Some(IssuePriority::Urgent),
        "high" => Some(IssuePriority::High),
        "medium" => Some(IssuePriority::Medium),
        "low" => Some(IssuePriority::Low),
        _ => None,
    });

    let parse_dt = |s: &Option<String>| -> Option<DateTime<Utc>> {
        s.as_ref().and_then(|d| {
            DateTime::parse_from_rfc3339(d)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
    };

    let extension_metadata: Value =
        serde_json::from_str(&i.extension_metadata).unwrap_or(Value::Object(Default::default()));

    Issue {
        id: i.id,
        project_id: i.project_id,
        issue_number: i.issue_number.unwrap_or(0) as i32,
        simple_id: i.simple_id.unwrap_or_default(),
        status_id: i.status_id,
        title: i.title,
        description: i.description,
        priority,
        start_date: parse_dt(&i.start_date),
        target_date: parse_dt(&i.target_date),
        completed_at: parse_dt(&i.completed_at),
        sort_order: i.sort_order as f64,
        parent_issue_id: i.parent_issue_id,
        parent_issue_sort_order: i.parent_issue_sort_order.map(|v| v as f64),
        extension_metadata,
        creator_user_id: i.creator_user_id,
        created_at: i.created_at,
        updated_at: i.updated_at,
    }
}

pub(super) fn db_to_api_status(s: db::models::kanban::KanbanProjectStatus) -> ProjectStatus {
    ProjectStatus {
        id: s.id,
        project_id: s.project_id,
        name: s.name,
        color: s.color,
        sort_order: s.sort_order as i32,
        hidden: s.hidden,
        created_at: s.created_at,
    }
}

pub(super) fn db_to_api_tag(t: db::models::kanban::KanbanTag) -> Tag {
    Tag {
        id: t.id,
        project_id: t.project_id,
        name: t.name,
        color: t.color,
    }
}

pub(super) fn db_to_api_assignee(a: db::models::kanban::KanbanIssueAssignee) -> IssueAssignee {
    IssueAssignee {
        id: a.id,
        issue_id: a.issue_id,
        user_id: a.user_id,
        assigned_at: a.assigned_at,
    }
}

pub(super) fn db_to_api_issue_tag(t: db::models::kanban::KanbanIssueTag) -> IssueTag {
    IssueTag {
        id: t.id,
        issue_id: t.issue_id,
        tag_id: t.tag_id,
    }
}

pub(super) fn db_to_api_relationship(
    r: db::models::kanban::KanbanIssueRelationship,
) -> IssueRelationship {
    use api_types::IssueRelationshipType;
    IssueRelationship {
        id: r.id,
        issue_id: r.issue_id,
        related_issue_id: r.related_issue_id,
        relationship_type: match r.relationship_type.as_str() {
            "blocking" => IssueRelationshipType::Blocking,
            "has_duplicate" => IssueRelationshipType::HasDuplicate,
            _ => IssueRelationshipType::Related,
        },
        created_at: r.created_at,
    }
}

pub(super) fn db_to_api_workspace(w: db::models::kanban::KanbanWorkspace) -> Workspace {
    Workspace {
        id: w.id,
        project_id: w.project_id,
        owner_user_id: w.owner_user_id,
        issue_id: w.issue_id,
        local_workspace_id: w.local_workspace_id,
        name: w.name,
        archived: w.archived,
        files_changed: w.files_changed.map(|v| v as i32),
        lines_added: w.lines_added.map(|v| v as i32),
        lines_removed: w.lines_removed.map(|v| v as i32),
        created_at: w.created_at,
        updated_at: w.updated_at,
    }
}

pub(super) fn db_to_api_comment(c: db::models::kanban::KanbanIssueComment) -> IssueComment {
    IssueComment {
        id: c.id,
        issue_id: c.issue_id,
        author_id: c.author_id,
        parent_id: c.parent_id,
        message: c.message,
        created_at: c.created_at,
        updated_at: c.updated_at,
    }
}

pub(super) fn db_to_api_reaction(
    r: db::models::kanban::KanbanIssueCommentReaction,
) -> IssueCommentReaction {
    IssueCommentReaction {
        id: r.id,
        comment_id: r.comment_id,
        user_id: r.user_id,
        emoji: r.emoji,
        created_at: r.created_at,
    }
}

pub(super) fn db_to_api_follower(f: db::models::kanban::KanbanIssueFollower) -> IssueFollower {
    IssueFollower {
        id: f.id,
        issue_id: f.issue_id,
        user_id: f.user_id,
    }
}

async fn fallback_organizations(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<ListOrganizationsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanOrganization::find_all(pool).await?;
    let organizations = rows
        .into_iter()
        .map(|o| OrganizationWithRole {
            id: o.id,
            name: o.name,
            slug: o.slug,
            is_personal: o.is_personal,
            issue_prefix: o.issue_prefix,
            created_at: o.created_at,
            updated_at: o.updated_at,
            user_role: MemberRole::Admin,
        })
        .collect();
    Ok(ResponseJson(ApiResponse::success(
        ListOrganizationsResponse { organizations },
    )))
}

async fn fallback_projects(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<ListProjectsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanProject::find_by_organization(pool, query.organization_id)
        .await?;
    let projects = rows.into_iter().map(db_to_api_project).collect();
    Ok(ResponseJson(ApiResponse::success(ListProjectsResponse {
        projects,
    })))
}

async fn fallback_issues(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
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

async fn fallback_tags(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<ListTagsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanTag::find_by_project(pool, query.project_id).await?;
    let tags = rows.into_iter().map(db_to_api_tag).collect();
    Ok(ResponseJson(ApiResponse::success(ListTagsResponse {
        tags,
    })))
}

async fn fallback_project_statuses(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<ListProjectStatusesResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows =
        db::models::kanban::KanbanProjectStatus::find_by_project(pool, query.project_id).await?;
    let project_statuses = rows.into_iter().map(db_to_api_status).collect();
    Ok(ResponseJson(ApiResponse::success(
        ListProjectStatusesResponse { project_statuses },
    )))
}

async fn fallback_issue_assignees(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueAssigneesResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    // The fallback endpoint expects project-scoped data, but our model is issue-scoped.
    // We need to join through issues to get project-scoped assignees.
    let issues = db::models::kanban::KanbanIssue::find_by_project(pool, query.project_id).await?;
    let mut assignees = Vec::new();
    for issue in issues {
        let rows = db::models::kanban::KanbanIssueAssignee::find_by_issue(pool, issue.id).await?;
        assignees.extend(rows.into_iter().map(db_to_api_assignee));
    }
    Ok(ResponseJson(ApiResponse::success(
        ListIssueAssigneesResponse {
            issue_assignees: assignees,
        },
    )))
}

async fn fallback_issue_followers(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueFollowersResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let issues = db::models::kanban::KanbanIssue::find_by_project(pool, query.project_id).await?;
    let mut followers = Vec::new();
    for issue in issues {
        let rows = db::models::kanban::KanbanIssueFollower::find_by_issue(pool, issue.id).await?;
        followers.extend(rows.into_iter().map(db_to_api_follower));
    }
    Ok(ResponseJson(ApiResponse::success(
        ListIssueFollowersResponse {
            issue_followers: followers,
        },
    )))
}

async fn fallback_issue_tags(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueTagsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let issues = db::models::kanban::KanbanIssue::find_by_project(pool, query.project_id).await?;
    let mut issue_tags = Vec::new();
    for issue in issues {
        let rows = db::models::kanban::KanbanIssueTag::find_by_issue(pool, issue.id).await?;
        issue_tags.extend(rows.into_iter().map(db_to_api_issue_tag));
    }
    Ok(ResponseJson(ApiResponse::success(ListIssueTagsResponse {
        issue_tags,
    })))
}

async fn fallback_issue_relationships(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueRelationshipsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let issues = db::models::kanban::KanbanIssue::find_by_project(pool, query.project_id).await?;
    let mut relationships = Vec::new();
    for issue in issues {
        let rows =
            db::models::kanban::KanbanIssueRelationship::find_by_issue(pool, issue.id).await?;
        relationships.extend(rows.into_iter().map(db_to_api_relationship));
    }
    Ok(ResponseJson(ApiResponse::success(
        ListIssueRelationshipsResponse {
            issue_relationships: relationships,
        },
    )))
}

async fn fallback_issue_comments(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<IssueScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueCommentsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanIssueComment::find_by_issue(pool, query.issue_id).await?;
    let comments = rows.into_iter().map(db_to_api_comment).collect();
    Ok(ResponseJson(ApiResponse::success(
        ListIssueCommentsResponse {
            issue_comments: comments,
        },
    )))
}

async fn fallback_issue_comment_reactions(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<IssueScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<ListIssueCommentReactionsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let comments =
        db::models::kanban::KanbanIssueComment::find_by_issue(pool, query.issue_id).await?;
    let mut reactions = Vec::new();
    for comment in comments {
        let rows =
            db::models::kanban::KanbanIssueCommentReaction::find_by_comment(pool, comment.id)
                .await?;
        reactions.extend(rows.into_iter().map(db_to_api_reaction));
    }
    Ok(ResponseJson(ApiResponse::success(
        ListIssueCommentReactionsResponse {
            issue_comment_reactions: reactions,
        },
    )))
}

async fn fallback_workspaces(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanWorkspace::find_by_project(pool, query.project_id).await?;
    let workspaces: Vec<Workspace> = rows.into_iter().map(db_to_api_workspace).collect();
    Ok(ResponseJson(ApiResponse::success(
        serde_json::json!({ "workspaces": workspaces }),
    )))
}

async fn fallback_pull_requests(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(
        serde_json::json!({ "pull_requests": [] }),
    )))
}

async fn fallback_pull_request_issues(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(
        serde_json::json!({ "pull_request_issues": [] }),
    )))
}

async fn fallback_project_workspaces(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ProjectScopedFallbackQuery>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanWorkspace::find_by_project(pool, query.project_id).await?;
    let workspaces: Vec<Workspace> = rows.into_iter().map(db_to_api_workspace).collect();
    Ok(ResponseJson(ApiResponse::success(
        serde_json::json!({ "workspaces": workspaces }),
    )))
}

async fn fallback_user_workspaces(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = sqlx::query_as!(
        db::models::kanban::KanbanWorkspace,
        r#"SELECT id as "id!: Uuid",
                  project_id as "project_id!: Uuid",
                  owner_user_id as "owner_user_id!: Uuid",
                  issue_id as "issue_id: Uuid",
                  local_workspace_id as "local_workspace_id: Uuid",
                  name,
                  archived as "archived!: bool",
                  files_changed, lines_added, lines_removed,
                  created_at as "created_at!: DateTime<Utc>",
                  updated_at as "updated_at!: DateTime<Utc>"
           FROM kanban_workspaces
           ORDER BY created_at DESC"#
    )
    .fetch_all(pool)
    .await?;
    let workspaces: Vec<Workspace> = rows.into_iter().map(db_to_api_workspace).collect();
    Ok(ResponseJson(ApiResponse::success(
        serde_json::json!({ "workspaces": workspaces }),
    )))
}

async fn fallback_notifications(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(
        serde_json::json!({ "notifications": [] }),
    )))
}

async fn fallback_organization_members(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(
        serde_json::json!({ "organization_members": [] }),
    )))
}

async fn fallback_users(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(
        serde_json::json!({ "users": [] }),
    )))
}
