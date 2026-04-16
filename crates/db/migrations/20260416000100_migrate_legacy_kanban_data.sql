-- Migrate legacy projects/tasks/workspaces into local kanban schema
-- This is a one-time migration for users switching from remote-sync to pure local mode.

PRAGMA foreign_keys = OFF;

-- ---------------------------------------------------------------------------
-- 1. Migrate projects -> kanban_projects
-- ---------------------------------------------------------------------------
INSERT INTO kanban_projects (id, organization_id, name, color, sort_order, created_at, updated_at)
SELECT
    p.id,
    (SELECT id FROM kanban_organizations WHERE slug = 'local' LIMIT 1),
    p.name,
    '#6366f1',
    0,
    p.created_at,
    p.updated_at
FROM projects p
WHERE NOT EXISTS (SELECT 1 FROM kanban_projects kp WHERE kp.id = p.id);

-- ---------------------------------------------------------------------------
-- 2. Create default statuses for each migrated project
-- ---------------------------------------------------------------------------
INSERT INTO kanban_project_statuses (id, project_id, name, color, sort_order, hidden, created_at)
SELECT lower(hex(randomblob(16))), kp.id, 'To Do', '#6366f1', 0, 0, datetime('now', 'subsec')
FROM kanban_projects kp
WHERE NOT EXISTS (
    SELECT 1 FROM kanban_project_statuses kps
    WHERE kps.project_id = kp.id AND kps.name = 'To Do'
);

INSERT INTO kanban_project_statuses (id, project_id, name, color, sort_order, hidden, created_at)
SELECT lower(hex(randomblob(16))), kp.id, 'In Progress', '#f59e0b', 1, 0, datetime('now', 'subsec')
FROM kanban_projects kp
WHERE NOT EXISTS (
    SELECT 1 FROM kanban_project_statuses kps
    WHERE kps.project_id = kp.id AND kps.name = 'In Progress'
);

INSERT INTO kanban_project_statuses (id, project_id, name, color, sort_order, hidden, created_at)
SELECT lower(hex(randomblob(16))), kp.id, 'Done', '#22c55e', 2, 0, datetime('now', 'subsec')
FROM kanban_projects kp
WHERE NOT EXISTS (
    SELECT 1 FROM kanban_project_statuses kps
    WHERE kps.project_id = kp.id AND kps.name = 'Done'
);

-- ---------------------------------------------------------------------------
-- 3. Migrate tasks -> kanban_issues
-- ---------------------------------------------------------------------------
INSERT INTO kanban_issues (
    id, project_id, issue_number, simple_id, status_id, title, description,
    priority, start_date, target_date, completed_at, sort_order,
    parent_issue_id, parent_issue_sort_order, extension_metadata,
    creator_user_id, created_at, updated_at
)
SELECT
    t.id,
    t.project_id,
    NULL,
    NULL,
    COALESCE(
        (SELECT kps.id FROM kanban_project_statuses kps
         WHERE kps.project_id = t.project_id
           AND kps.name = CASE t.status
               WHEN 'in_progress' THEN 'In Progress'
               WHEN 'done' THEN 'Done'
               ELSE 'To Do'
           END
         LIMIT 1),
        (SELECT kps.id FROM kanban_project_statuses kps
         WHERE kps.project_id = t.project_id LIMIT 1)
    ),
    t.title,
    t.description,
    'medium',
    NULL,
    NULL,
    CASE WHEN t.status = 'done' THEN t.updated_at ELSE NULL END,
    0,
    NULL,
    NULL,
    '{}',
    NULL,
    t.created_at,
    t.updated_at
FROM tasks t
WHERE NOT EXISTS (SELECT 1 FROM kanban_issues ki WHERE ki.id = t.id);

-- ---------------------------------------------------------------------------
-- 4. Migrate workspaces with task_id -> kanban_workspaces
-- ---------------------------------------------------------------------------
INSERT INTO kanban_workspaces (
    id, project_id, owner_user_id, issue_id, local_workspace_id, name,
    archived, files_changed, lines_added, lines_removed, created_at, updated_at
)
SELECT
    lower(hex(randomblob(16))),
    t.project_id,
    X'00000000000000000000000000000000',
    t.id,
    w.id,
    COALESCE(w.name, w.branch),
    w.archived,
    NULL,
    NULL,
    NULL,
    w.created_at,
    w.updated_at
FROM workspaces w
JOIN tasks t ON t.id = w.task_id
WHERE w.task_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM kanban_workspaces kw
      WHERE kw.local_workspace_id = w.id AND kw.issue_id = t.id
  );

-- ---------------------------------------------------------------------------
-- 5. Migrate workspaces WITHOUT task_id into a single catch-all issue
-- ---------------------------------------------------------------------------
-- Create one placeholder issue in the first migrated project.
INSERT INTO kanban_issues (
    id, project_id, issue_number, simple_id, status_id, title, description,
    priority, start_date, target_date, completed_at, sort_order,
    parent_issue_id, parent_issue_sort_order, extension_metadata,
    creator_user_id, created_at, updated_at
)
SELECT
    lower(hex(randomblob(16))),
    (SELECT id FROM kanban_projects ORDER BY created_at LIMIT 1),
    NULL,
    'MIGRATED',
    (SELECT kps.id FROM kanban_project_statuses kps
     ORDER BY kps.sort_order LIMIT 1),
    'Migrated workspaces',
    'Legacy workspaces imported from local database',
    'low',
    NULL,
    NULL,
    NULL,
    9999,
    NULL,
    NULL,
    '{}',
    NULL,
    datetime('now', 'subsec'),
    datetime('now', 'subsec')
WHERE EXISTS (SELECT 1 FROM workspaces w WHERE w.task_id IS NULL)
  AND NOT EXISTS (
      SELECT 1 FROM kanban_issues ki WHERE ki.simple_id = 'MIGRATED'
  );

-- Link unlinked workspaces to the placeholder issue.
INSERT INTO kanban_workspaces (
    id, project_id, owner_user_id, issue_id, local_workspace_id, name,
    archived, files_changed, lines_added, lines_removed, created_at, updated_at
)
SELECT
    lower(hex(randomblob(16))),
    ki.project_id,
    X'00000000000000000000000000000000',
    ki.id,
    w.id,
    COALESCE(w.name, w.branch),
    w.archived,
    NULL,
    NULL,
    NULL,
    w.created_at,
    w.updated_at
FROM workspaces w
CROSS JOIN (SELECT id, project_id FROM kanban_issues WHERE simple_id = 'MIGRATED' LIMIT 1) ki
WHERE w.task_id IS NULL
  AND NOT EXISTS (
      SELECT 1 FROM kanban_workspaces kw
      WHERE kw.local_workspace_id = w.id
  );

PRAGMA foreign_keys = ON;
