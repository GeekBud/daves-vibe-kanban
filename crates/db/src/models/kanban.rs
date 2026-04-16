use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::{FromRow, SqlitePool};
use ts_rs::TS;
use uuid::Uuid;

// =============================================================================
// Organizations
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanOrganization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub is_personal: bool,
    pub issue_prefix: String,
    pub color: String,
    pub sort_order: i64,
    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
    #[ts(type = "Date")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateKanbanOrganization {
    pub name: String,
    pub color: Option<String>,
}

impl KanbanOrganization {
    pub async fn find_all(pool: &SqlitePool) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanOrganization,
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
               ORDER BY sort_order ASC, created_at ASC"#
        )
        .fetch_all(pool)
        .await
    }

    pub async fn create(pool: &SqlitePool, data: &CreateKanbanOrganization) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        let color = data.color.as_deref().unwrap_or("#6366f1");
        sqlx::query_as!(
            KanbanOrganization,
            r#"INSERT INTO kanban_organizations (id, name, color)
               VALUES ($1, $2, $3)
               RETURNING id as "id!: Uuid", name, slug, is_personal as "is_personal!: bool",
                         issue_prefix, color, sort_order,
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            data.name,
            color
        )
        .fetch_one(pool)
        .await
    }

    /// Seed a default local organization if the table is empty.
    pub async fn seed_default(pool: &SqlitePool) -> Result<(), sqlx::Error> {
        let count = sqlx::query!("SELECT COUNT(*) as count FROM kanban_organizations")
            .fetch_one(pool)
            .await?;
        if count.count == 0 {
            let id = Uuid::new_v4();
            let now = chrono::Utc::now();
            sqlx::query!(
                "INSERT INTO kanban_organizations (id, name, slug, is_personal, issue_prefix, color, sort_order, created_at, updated_at) VALUES ($1, $2, $3, 1, $4, $5, 0, $6, $6)",
                id,
                "Local",
                "local",
                "LOC",
                "#6366f1",
                now
            )
            .execute(pool)
            .await?;
        }
        Ok(())
    }
}

// =============================================================================
// Projects
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanProject {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    pub color: String,
    pub sort_order: i64,
    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
    #[ts(type = "Date")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateKanbanProject {
    pub organization_id: Uuid,
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
pub struct UpdateKanbanProject {
    pub name: Option<String>,
    pub color: Option<String>,
    pub sort_order: Option<i64>,
}

impl KanbanProject {
    pub async fn find_by_organization(pool: &SqlitePool, organization_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanProject,
            r#"SELECT id as "id!: Uuid",
                      organization_id as "organization_id!: Uuid",
                      name, color, sort_order,
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM kanban_projects
               WHERE organization_id = $1
               ORDER BY sort_order ASC, created_at ASC"#,
            organization_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanProject,
            r#"SELECT id as "id!: Uuid",
                      organization_id as "organization_id!: Uuid",
                      name, color, sort_order,
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM kanban_projects
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn create(pool: &SqlitePool, data: &CreateKanbanProject) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        let color = data.color.as_deref().unwrap_or("#6366f1");
        sqlx::query_as!(
            KanbanProject,
            r#"INSERT INTO kanban_projects (id, organization_id, name, color)
               VALUES ($1, $2, $3, $4)
               RETURNING id as "id!: Uuid",
                         organization_id as "organization_id!: Uuid",
                         name, color, sort_order,
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            data.organization_id,
            data.name,
            color
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update(pool: &SqlitePool, id: Uuid, data: &UpdateKanbanProject) -> Result<Self, sqlx::Error> {
        let existing = Self::find_by_id(pool, id).await?.ok_or(sqlx::Error::RowNotFound)?;
        let name = data.name.as_ref().unwrap_or(&existing.name);
        let color = data.color.as_ref().unwrap_or(&existing.color);
        let sort_order = data.sort_order.unwrap_or(existing.sort_order);
        sqlx::query_as!(
            KanbanProject,
            r#"UPDATE kanban_projects
               SET name = $2, color = $3, sort_order = $4, updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id as "id!: Uuid",
                         organization_id as "organization_id!: Uuid",
                         name, color, sort_order,
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            name,
            color,
            sort_order
        )
        .fetch_one(pool)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result: sqlx::sqlite::SqliteQueryResult = sqlx::query!("DELETE FROM kanban_projects WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

// =============================================================================
// Project Statuses
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanProjectStatus {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub color: String,
    pub sort_order: i64,
    pub hidden: bool,
    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateKanbanProjectStatus {
    pub project_id: Uuid,
    pub name: String,
    pub color: Option<String>,
    pub sort_order: i64,
    pub hidden: Option<bool>,
}

impl KanbanProjectStatus {
    pub async fn find_by_project(pool: &SqlitePool, project_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanProjectStatus,
            r#"SELECT id as "id!: Uuid",
                      project_id as "project_id!: Uuid",
                      name, color, sort_order,
                      hidden as "hidden!: bool",
                      created_at as "created_at!: DateTime<Utc>"
               FROM kanban_project_statuses
               WHERE project_id = $1
               ORDER BY sort_order ASC"#,
            project_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn create(pool: &SqlitePool, data: &CreateKanbanProjectStatus) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        let color = data.color.as_deref().unwrap_or("#6366f1");
        let hidden = data.hidden.unwrap_or(false);
        sqlx::query_as!(
            KanbanProjectStatus,
            r#"INSERT INTO kanban_project_statuses (id, project_id, name, color, sort_order, hidden)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id as "id!: Uuid",
                         project_id as "project_id!: Uuid",
                         name, color, sort_order,
                         hidden as "hidden!: bool",
                         created_at as "created_at!: DateTime<Utc>""#,
            id,
            data.project_id,
            data.name,
            color,
            data.sort_order,
            hidden
        )
        .fetch_one(pool)
        .await
    }
}

// =============================================================================
// Tags
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanTag {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateKanbanTag {
    pub project_id: Uuid,
    pub name: String,
    pub color: Option<String>,
}

impl KanbanTag {
    pub async fn find_by_project(pool: &SqlitePool, project_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanTag,
            r#"SELECT id as "id!: Uuid",
                      project_id as "project_id!: Uuid",
                      name, color
               FROM kanban_tags
               WHERE project_id = $1
               ORDER BY name ASC"#,
            project_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn create(pool: &SqlitePool, data: &CreateKanbanTag) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        let color = data.color.as_deref().unwrap_or("#6366f1");
        sqlx::query_as!(
            KanbanTag,
            r#"INSERT INTO kanban_tags (id, project_id, name, color)
               VALUES ($1, $2, $3, $4)
               RETURNING id as "id!: Uuid",
                         project_id as "project_id!: Uuid",
                         name, color"#,
            id,
            data.project_id,
            data.name,
            color
        )
        .fetch_one(pool)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result: sqlx::sqlite::SqliteQueryResult = sqlx::query!("DELETE FROM kanban_tags WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

// =============================================================================
// Issues
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanIssue {
    pub id: Uuid,
    pub project_id: Uuid,
    pub issue_number: Option<i64>,
    pub simple_id: Option<String>,
    pub status_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub start_date: Option<String>,
    pub target_date: Option<String>,
    pub completed_at: Option<String>,
    pub sort_order: i64,
    pub parent_issue_id: Option<Uuid>,
    pub parent_issue_sort_order: Option<i64>,
    pub extension_metadata: String,
    pub creator_user_id: Option<Uuid>,
    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
    #[ts(type = "Date")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateKanbanIssue {
    pub project_id: Uuid,
    pub status_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub start_date: Option<String>,
    pub target_date: Option<String>,
    pub completed_at: Option<String>,
    pub sort_order: Option<i64>,
    pub parent_issue_id: Option<Uuid>,
    pub parent_issue_sort_order: Option<i64>,
    pub extension_metadata: Option<JsonValue>,
    pub creator_user_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, TS)]
pub struct UpdateKanbanIssue {
    pub status_id: Option<Uuid>,
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub priority: Option<Option<String>>,
    pub start_date: Option<Option<String>>,
    pub target_date: Option<Option<String>>,
    pub completed_at: Option<Option<String>>,
    pub sort_order: Option<i64>,
    pub parent_issue_id: Option<Option<Uuid>>,
    pub parent_issue_sort_order: Option<Option<i64>>,
    pub extension_metadata: Option<JsonValue>,
}

impl KanbanIssue {
    pub async fn find_by_project(pool: &SqlitePool, project_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanIssue,
            r#"SELECT id as "id!: Uuid",
                      project_id as "project_id!: Uuid",
                      issue_number, simple_id,
                      status_id as "status_id!: Uuid",
                      title, description, priority,
                      start_date, target_date, completed_at,
                      sort_order,
                      parent_issue_id as "parent_issue_id: Uuid",
                      parent_issue_sort_order,
                      extension_metadata as "extension_metadata!",
                      creator_user_id as "creator_user_id: Uuid",
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM kanban_issues
               WHERE project_id = $1
               ORDER BY sort_order ASC, created_at ASC"#,
            project_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanIssue,
            r#"SELECT id as "id!: Uuid",
                      project_id as "project_id!: Uuid",
                      issue_number, simple_id,
                      status_id as "status_id!: Uuid",
                      title, description, priority,
                      start_date, target_date, completed_at,
                      sort_order,
                      parent_issue_id as "parent_issue_id: Uuid",
                      parent_issue_sort_order,
                      extension_metadata as "extension_metadata!",
                      creator_user_id as "creator_user_id: Uuid",
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM kanban_issues
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn create(pool: &SqlitePool, data: &CreateKanbanIssue) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();
        let sort_order = data.sort_order.unwrap_or(0);
        let extension_metadata = data.extension_metadata.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "{}".to_string());
        let description = data.description.as_deref();
        let priority = data.priority.as_deref();
        let start_date = data.start_date.as_deref();
        let target_date = data.target_date.as_deref();
        let completed_at = data.completed_at.as_deref();
        // parent_issue_sort_order is Option<i64>, can be passed directly
        sqlx::query_as!(
            KanbanIssue,
            r#"INSERT INTO kanban_issues (
                   id, project_id, status_id, title, description, priority,
                   start_date, target_date, completed_at, sort_order,
                   parent_issue_id, parent_issue_sort_order, extension_metadata, creator_user_id
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
               RETURNING id as "id!: Uuid",
                         project_id as "project_id!: Uuid",
                         issue_number, simple_id,
                         status_id as "status_id!: Uuid",
                         title, description, priority,
                         start_date, target_date, completed_at,
                         sort_order,
                         parent_issue_id as "parent_issue_id: Uuid",
                         parent_issue_sort_order,
                         extension_metadata as "extension_metadata!",
                         creator_user_id as "creator_user_id: Uuid",
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            data.project_id,
            data.status_id,
            data.title,
            description,
            priority,
            start_date,
            target_date,
            completed_at,
            sort_order,
            data.parent_issue_id,
            data.parent_issue_sort_order,
            extension_metadata,
            data.creator_user_id,
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update(pool: &SqlitePool, id: Uuid, data: &UpdateKanbanIssue) -> Result<Self, sqlx::Error> {
        let existing = Self::find_by_id(pool, id).await?.ok_or(sqlx::Error::RowNotFound)?;
        let status_id = data.status_id.unwrap_or(existing.status_id);
        let title = data.title.as_ref().unwrap_or(&existing.title);
        let description = data.description.as_ref().map(|v| v.as_deref()).flatten().or(existing.description.as_deref());
        let priority = data.priority.as_ref().map(|v| v.as_deref()).flatten().or(existing.priority.as_deref());
        let start_date = data.start_date.as_ref().map(|v| v.as_deref()).flatten().or(existing.start_date.as_deref());
        let target_date = data.target_date.as_ref().map(|v| v.as_deref()).flatten().or(existing.target_date.as_deref());
        let completed_at = data.completed_at.as_ref().map(|v| v.as_deref()).flatten().or(existing.completed_at.as_deref());
        let sort_order = data.sort_order.unwrap_or(existing.sort_order);
        let parent_issue_id = data.parent_issue_id.as_ref().map(|v| *v).flatten().or(existing.parent_issue_id);
        let parent_issue_sort_order: Option<i64> = data.parent_issue_sort_order.as_ref().map(|v| *v).flatten().or(existing.parent_issue_sort_order);
        let extension_metadata = data.extension_metadata.as_ref().map(|v| v.to_string()).unwrap_or(existing.extension_metadata);
        let description = description; // already Option<&str>
        let priority = priority;
        let start_date = start_date;
        let target_date = target_date;
        let completed_at = completed_at;
        sqlx::query_as!(
            KanbanIssue,
            r#"UPDATE kanban_issues
               SET status_id = $2, title = $3, description = $4, priority = $5,
                   start_date = $6, target_date = $7, completed_at = $8, sort_order = $9,
                   parent_issue_id = $10, parent_issue_sort_order = $11,
                   extension_metadata = $12, updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id as "id!: Uuid",
                         project_id as "project_id!: Uuid",
                         issue_number, simple_id,
                         status_id as "status_id!: Uuid",
                         title, description, priority,
                         start_date, target_date, completed_at,
                         sort_order,
                         parent_issue_id as "parent_issue_id: Uuid",
                         parent_issue_sort_order,
                         extension_metadata as "extension_metadata!",
                         creator_user_id as "creator_user_id: Uuid",
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            status_id,
            title,
            description,
            priority,
            start_date,
            target_date,
            completed_at,
            sort_order,
            parent_issue_id,
            parent_issue_sort_order,
            extension_metadata,
        )
        .fetch_one(pool)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result: sqlx::sqlite::SqliteQueryResult = sqlx::query!("DELETE FROM kanban_issues WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

// =============================================================================
// Issue Assignees
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanIssueAssignee {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub user_id: Uuid,
    #[ts(type = "Date")]
    pub assigned_at: DateTime<Utc>,
}

impl KanbanIssueAssignee {
    pub async fn find_by_issue(pool: &SqlitePool, issue_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanIssueAssignee,
            r#"SELECT id as "id!: Uuid",
                      issue_id as "issue_id!: Uuid",
                      user_id as "user_id!: Uuid",
                      assigned_at as "assigned_at!: DateTime<Utc>"
               FROM kanban_issue_assignees
               WHERE issue_id = $1"#,
            issue_id
        )
        .fetch_all(pool)
        .await
    }
}

// =============================================================================
// Issue Tags
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanIssueTag {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub tag_id: Uuid,
}

impl KanbanIssueTag {
    pub async fn find_by_issue(pool: &SqlitePool, issue_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanIssueTag,
            r#"SELECT id as "id!: Uuid",
                      issue_id as "issue_id!: Uuid",
                      tag_id as "tag_id!: Uuid"
               FROM kanban_issue_tags
               WHERE issue_id = $1"#,
            issue_id
        )
        .fetch_all(pool)
        .await
    }
}

// =============================================================================
// Issue Relationships
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanIssueRelationship {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub related_issue_id: Uuid,
    pub relationship_type: String,
    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
}

impl KanbanIssueRelationship {
    pub async fn find_by_issue(pool: &SqlitePool, issue_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanIssueRelationship,
            r#"SELECT id as "id!: Uuid",
                      issue_id as "issue_id!: Uuid",
                      related_issue_id as "related_issue_id!: Uuid",
                      relationship_type,
                      created_at as "created_at!: DateTime<Utc>"
               FROM kanban_issue_relationships
               WHERE issue_id = $1"#,
            issue_id
        )
        .fetch_all(pool)
        .await
    }
}

// =============================================================================
// Issue Comments
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanIssueComment {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub author_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub message: String,
    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
    #[ts(type = "Date")]
    pub updated_at: DateTime<Utc>,
}

impl KanbanIssueComment {
    pub async fn find_by_issue(pool: &SqlitePool, issue_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanIssueComment,
            r#"SELECT id as "id!: Uuid",
                      issue_id as "issue_id!: Uuid",
                      author_id as "author_id: Uuid",
                      parent_id as "parent_id: Uuid",
                      message,
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM kanban_issue_comments
               WHERE issue_id = $1
               ORDER BY created_at ASC"#,
            issue_id
        )
        .fetch_all(pool)
        .await
    }
}

// =============================================================================
// Issue Followers
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanIssueFollower {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub user_id: Uuid,
}

impl KanbanIssueFollower {
    pub async fn find_by_issue(pool: &SqlitePool, issue_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanIssueFollower,
            r#"SELECT id as "id!: Uuid",
                      issue_id as "issue_id!: Uuid",
                      user_id as "user_id!: Uuid"
               FROM kanban_issue_followers
               WHERE issue_id = $1"#,
            issue_id
        )
        .fetch_all(pool)
        .await
    }
}

// =============================================================================
// Issue Comment Reactions
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanIssueCommentReaction {
    pub id: Uuid,
    pub comment_id: Uuid,
    pub user_id: Uuid,
    pub emoji: String,
    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
}

impl KanbanIssueCommentReaction {
    pub async fn find_by_comment(pool: &SqlitePool, comment_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanIssueCommentReaction,
            r#"SELECT id as "id!: Uuid",
                      comment_id as "comment_id!: Uuid",
                      user_id as "user_id!: Uuid",
                      emoji,
                      created_at as "created_at!: DateTime<Utc>"
               FROM kanban_issue_comment_reactions
               WHERE comment_id = $1"#,
            comment_id
        )
        .fetch_all(pool)
        .await
    }
}

// =============================================================================
// Workspaces (Kanban-linked)
// =============================================================================
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct KanbanWorkspace {
    pub id: Uuid,
    pub project_id: Uuid,
    pub owner_user_id: Uuid,
    pub issue_id: Option<Uuid>,
    pub local_workspace_id: Option<Uuid>,
    pub name: Option<String>,
    pub archived: bool,
    pub files_changed: Option<i64>,
    pub lines_added: Option<i64>,
    pub lines_removed: Option<i64>,
    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
    #[ts(type = "Date")]
    pub updated_at: DateTime<Utc>,
}

impl KanbanWorkspace {
    pub async fn find_by_project(pool: &SqlitePool, project_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            KanbanWorkspace,
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
               WHERE project_id = $1
               ORDER BY created_at DESC"#,
            project_id
        )
        .fetch_all(pool)
        .await
    }
}
