PRAGMA foreign_keys = ON;

-- Local Kanban schema for offline mode
-- Mirrors remote Electric SQL shapes with local-first IDs

CREATE TABLE kanban_organizations (
    id         BLOB PRIMARY KEY,
    name       TEXT NOT NULL,
    color      TEXT NOT NULL DEFAULT '#6366f1',
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE TABLE kanban_projects (
    id              BLOB PRIMARY KEY,
    organization_id BLOB NOT NULL,
    name            TEXT NOT NULL,
    color           TEXT NOT NULL DEFAULT '#6366f1',
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (organization_id) REFERENCES kanban_organizations(id) ON DELETE CASCADE
);

CREATE INDEX idx_kanban_projects_organization_id ON kanban_projects(organization_id);

CREATE TABLE kanban_project_statuses (
    id         BLOB PRIMARY KEY,
    project_id BLOB NOT NULL,
    name       TEXT NOT NULL,
    color      TEXT NOT NULL DEFAULT '#6366f1',
    sort_order INTEGER NOT NULL DEFAULT 0,
    hidden     INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (project_id) REFERENCES kanban_projects(id) ON DELETE CASCADE
);

CREATE INDEX idx_kanban_project_statuses_project_id ON kanban_project_statuses(project_id);

CREATE TABLE kanban_tags (
    id         BLOB PRIMARY KEY,
    project_id BLOB NOT NULL,
    name       TEXT NOT NULL,
    color      TEXT NOT NULL DEFAULT '#6366f1',
    FOREIGN KEY (project_id) REFERENCES kanban_projects(id) ON DELETE CASCADE
);

CREATE INDEX idx_kanban_tags_project_id ON kanban_tags(project_id);

CREATE TABLE kanban_issues (
    id                      BLOB PRIMARY KEY,
    project_id              BLOB NOT NULL,
    issue_number            INTEGER,
    simple_id               TEXT,
    status_id               BLOB NOT NULL,
    title                   TEXT NOT NULL,
    description             TEXT,
    priority                TEXT CHECK (priority IN ('urgent','high','medium','low')),
    start_date              TEXT,
    target_date             TEXT,
    completed_at            TEXT,
    sort_order              INTEGER NOT NULL DEFAULT 0,
    parent_issue_id         BLOB,
    parent_issue_sort_order INTEGER,
    extension_metadata      TEXT NOT NULL DEFAULT '{}',
    creator_user_id         BLOB,
    created_at              TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at              TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (project_id) REFERENCES kanban_projects(id) ON DELETE CASCADE,
    FOREIGN KEY (status_id) REFERENCES kanban_project_statuses(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_issue_id) REFERENCES kanban_issues(id) ON DELETE SET NULL
);

CREATE INDEX idx_kanban_issues_project_id    ON kanban_issues(project_id);
CREATE INDEX idx_kanban_issues_status_id     ON kanban_issues(status_id);
CREATE INDEX idx_kanban_issues_parent_issue_id ON kanban_issues(parent_issue_id);
CREATE INDEX idx_kanban_issues_simple_id     ON kanban_issues(simple_id);

CREATE TABLE kanban_issue_assignees (
    id          BLOB PRIMARY KEY,
    issue_id    BLOB NOT NULL,
    user_id     BLOB NOT NULL,
    assigned_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (issue_id) REFERENCES kanban_issues(id) ON DELETE CASCADE
);

CREATE INDEX idx_kanban_issue_assignees_issue_id ON kanban_issue_assignees(issue_id);
CREATE INDEX idx_kanban_issue_assignees_user_id  ON kanban_issue_assignees(user_id);

CREATE TABLE kanban_issue_tags (
    id       BLOB PRIMARY KEY,
    issue_id BLOB NOT NULL,
    tag_id   BLOB NOT NULL,
    FOREIGN KEY (issue_id) REFERENCES kanban_issues(id) ON DELETE CASCADE,
    FOREIGN KEY (tag_id)   REFERENCES kanban_tags(id)   ON DELETE CASCADE,
    UNIQUE (issue_id, tag_id)
);

CREATE INDEX idx_kanban_issue_tags_issue_id ON kanban_issue_tags(issue_id);
CREATE INDEX idx_kanban_issue_tags_tag_id   ON kanban_issue_tags(tag_id);

CREATE TABLE kanban_issue_relationships (
    id               BLOB PRIMARY KEY,
    issue_id         BLOB NOT NULL,
    related_issue_id BLOB NOT NULL,
    relationship_type TEXT NOT NULL CHECK (relationship_type IN ('blocking','related','has_duplicate')),
    created_at       TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (issue_id) REFERENCES kanban_issues(id) ON DELETE CASCADE,
    FOREIGN KEY (related_issue_id) REFERENCES kanban_issues(id) ON DELETE CASCADE,
    UNIQUE (issue_id, related_issue_id, relationship_type)
);

CREATE INDEX idx_kanban_issue_relationships_issue_id ON kanban_issue_relationships(issue_id);

CREATE TABLE kanban_issue_comments (
    id         BLOB PRIMARY KEY,
    issue_id   BLOB NOT NULL,
    author_id  BLOB,
    parent_id  BLOB,
    message    TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (issue_id) REFERENCES kanban_issues(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_id) REFERENCES kanban_issue_comments(id) ON DELETE CASCADE
);

CREATE INDEX idx_kanban_issue_comments_issue_id ON kanban_issue_comments(issue_id);

CREATE TABLE kanban_workspaces (
    id                 BLOB PRIMARY KEY,
    project_id         BLOB NOT NULL,
    owner_user_id      BLOB NOT NULL,
    issue_id           BLOB,
    local_workspace_id BLOB UNIQUE,
    name               TEXT,
    archived           INTEGER NOT NULL DEFAULT 0,
    files_changed      INTEGER,
    lines_added        INTEGER,
    lines_removed      INTEGER,
    created_at         TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at         TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (project_id) REFERENCES kanban_projects(id) ON DELETE CASCADE,
    FOREIGN KEY (issue_id)   REFERENCES kanban_issues(id)   ON DELETE SET NULL,
    FOREIGN KEY (local_workspace_id) REFERENCES workspaces(id) ON DELETE SET NULL
);

CREATE INDEX idx_kanban_workspaces_project_id ON kanban_workspaces(project_id);
CREATE INDEX idx_kanban_workspaces_issue_id   ON kanban_workspaces(issue_id);
CREATE INDEX idx_kanban_workspaces_local_workspace_id ON kanban_workspaces(local_workspace_id);
