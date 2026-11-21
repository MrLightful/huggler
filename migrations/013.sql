-- Rebuild the parent table to scope the original native_thread UNIQUE constraint.
-- Preserve rowid because thread-list subscriptions use insertion order.
CREATE TABLE thread_v13 (
    id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES project(id),
    name TEXT NOT NULL, model TEXT, native_thread TEXT,
    provider TEXT NOT NULL, worktree_path TEXT REFERENCES worktree(path), provider_instance TEXT NOT NULL,
    continuation_key TEXT, UNIQUE(provider_instance,native_thread)
) STRICT;
INSERT INTO thread_v13(rowid,id,project,name,model,native_thread,provider,worktree_path,provider_instance)
SELECT rowid,id,project,name,model,native_thread,provider,worktree_path,provider FROM thread;
DROP TABLE thread;
ALTER TABLE thread_v13 RENAME TO thread;
CREATE TABLE provider_usage (
    run_id TEXT PRIMARY KEY REFERENCES run(id),
    input_tokens INTEGER NOT NULL CHECK(input_tokens>=0),
    output_tokens INTEGER NOT NULL CHECK(output_tokens>=0)
) STRICT;
PRAGMA user_version = 13;
