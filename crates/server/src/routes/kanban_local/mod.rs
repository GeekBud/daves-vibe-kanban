use axum::Router;

use crate::DeploymentImpl;

mod fallback;
mod issues;
mod issue_assignees;
mod issue_comment_reactions;
mod issue_comments;
mod issue_followers;
mod issue_relationships;
mod issue_tags;
mod organizations;
mod projects;
mod project_statuses;
mod tags;
mod workspaces;

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .merge(fallback::router())
        .merge(projects::router())
        .merge(issues::router())
        .merge(tags::router())
        .merge(project_statuses::router())
        .merge(issue_assignees::router())
        .merge(issue_tags::router())
        .merge(issue_relationships::router())
        .merge(issue_comments::router())
        .merge(issue_comment_reactions::router())
        .merge(issue_followers::router())
        .merge(organizations::router())
        .merge(workspaces::router())
}
