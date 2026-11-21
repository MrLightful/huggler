ALTER TABLE thread ADD COLUMN provider TEXT NOT NULL DEFAULT 'codex';
CREATE TABLE permission (
    id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES run(id),
    request TEXT NOT NULL, decision TEXT, expired INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX permission_run ON permission(run_id);
PRAGMA user_version = 2;
