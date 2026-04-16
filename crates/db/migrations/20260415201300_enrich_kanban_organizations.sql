-- Add missing organization fields needed for local-mode API compatibility
ALTER TABLE kanban_organizations ADD COLUMN slug TEXT NOT NULL DEFAULT 'local';
ALTER TABLE kanban_organizations ADD COLUMN is_personal INTEGER NOT NULL DEFAULT 1;
ALTER TABLE kanban_organizations ADD COLUMN issue_prefix TEXT NOT NULL DEFAULT 'LOC';
