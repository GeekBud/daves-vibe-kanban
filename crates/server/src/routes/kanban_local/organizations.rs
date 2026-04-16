use api_types::{
    CreateOrganizationRequest, CreateOrganizationResponse, GetOrganizationResponse,
    ListOrganizationsResponse, MemberRole, Organization, OrganizationWithRole,
};
use axum::{
    Router,
    extract::{Json, Path, State},
    response::Json as ResponseJson,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use deployment::Deployment;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/organizations", get(list_organizations).post(create_organization))
        .route("/organizations/{org_id}", get(get_organization))
        .route("/organizations/{org_id}/members", get(list_members))
}

fn db_to_api_organization(o: db::models::kanban::KanbanOrganization) -> Organization {
    Organization {
        id: o.id,
        name: o.name,
        slug: o.slug,
        is_personal: o.is_personal,
        issue_prefix: o.issue_prefix,
        created_at: o.created_at,
        updated_at: o.updated_at,
    }
}

fn db_to_api_organization_with_role(o: db::models::kanban::KanbanOrganization) -> OrganizationWithRole {
    OrganizationWithRole {
        id: o.id,
        name: o.name,
        slug: o.slug,
        is_personal: o.is_personal,
        issue_prefix: o.issue_prefix,
        created_at: o.created_at,
        updated_at: o.updated_at,
        user_role: MemberRole::Admin,
    }
}

async fn list_organizations(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<ListOrganizationsResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let rows = db::models::kanban::KanbanOrganization::find_all(pool).await?;
    let organizations = rows.into_iter().map(db_to_api_organization_with_role).collect();
    Ok(ResponseJson(ApiResponse::success(ListOrganizationsResponse { organizations })))
}

async fn get_organization(
    State(deployment): State<DeploymentImpl>,
    Path(org_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<GetOrganizationResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let row = sqlx::query_as!(
        db::models::kanban::KanbanOrganization,
        r#"SELECT id as "id!: Uuid",
                  name,
                  slug,
                  is_personal as "is_personal!: bool",
                  issue_prefix,
                  color,
                  sort_order,
                  created_at as "created_at!: DateTime<Utc>",
                  updated_at as "updated_at!: DateTime<Utc>"
           FROM kanban_organizations
           WHERE id = $1"#,
        org_id
    )
    .fetch_optional(pool)
    .await?;
    match row {
        Some(o) => Ok(ResponseJson(ApiResponse::success(GetOrganizationResponse {
            organization: db_to_api_organization(o),
            user_role: "ADMIN".to_string(),
        }))),
        None => Err(ApiError::BadRequest(format!("{}: {}", "Organization", "Organization not found"))),
    }
}

async fn create_organization(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateOrganizationRequest>,
) -> Result<ResponseJson<ApiResponse<CreateOrganizationResponse>>, ApiError> {
    let pool = &deployment.db().pool;
    let id = Uuid::new_v4();
    let now = chrono::Utc::now();
    let slug = request.slug;
    let row = sqlx::query_as!(
        db::models::kanban::KanbanOrganization,
        r#"INSERT INTO kanban_organizations (id, name, slug, is_personal, issue_prefix, color, sort_order, created_at, updated_at)
           VALUES ($1, $2, $3, 0, '', '#6366f1', 0, $4, $4)
           RETURNING id as "id!: Uuid", name, slug, is_personal as "is_personal!: bool",
                     issue_prefix, color, sort_order,
                     created_at as "created_at!: DateTime<Utc>",
                     updated_at as "updated_at!: DateTime<Utc>""#,
        id,
        request.name,
        slug,
        now
    )
    .fetch_one(pool)
    .await?;

    let org = db_to_api_organization_with_role(row);
    Ok(ResponseJson(ApiResponse::success(CreateOrganizationResponse { organization: org })))
}

async fn list_members(
    State(deployment): State<DeploymentImpl>,
    Path(_org_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<serde_json::Value>>, ApiError> {
    // Local mode has no real member system; return empty list
    Ok(ResponseJson(ApiResponse::success(serde_json::json!({ "members": [] }))))
}
