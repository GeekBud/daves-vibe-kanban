PRAGMA foreign_keys = ON;

CREATE TABLE kanban_issue_followers (
    id       BLOB PRIMARY KEY,
    issue_id BLOB NOT NULL,
    user_id  BLOB NOT NULL,
    FOREIGN KEY (issue_id) REFERENCES kanban_issues(id) ON DELETE CASCADE,
    UNIQUE (issue_id, user_id)
);

CREATE INDEX idx_kanban_issue_followers_issue_id ON kanban_issue_followers(issue_id);
CREATE INDEX idx_kanban_issue_followers_user_id  ON kanban_issue_followers(user_id);

CREATE TABLE kanban_issue_comment_reactions (
    id         BLOB PRIMARY KEY,
    comment_id BLOB NOT NULL,
    user_id    BLOB NOT NULL,
    emoji      TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (comment_id) REFERENCES kanban_issue_comments(id) ON DELETE CASCADE,
    UNIQUE (comment_id, user_id, emoji)
);

CREATE INDEX idx_kanban_issue_comment_reactions_comment_id ON kanban_issue_comment_reactions(comment_id);
