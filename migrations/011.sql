CREATE TABLE worktree (
    path TEXT PRIMARY KEY,
    common_dir TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    ready INTEGER NOT NULL DEFAULT 0 CHECK (ready IN (0, 1))
) STRICT;
ALTER TABLE thread ADD COLUMN worktree_path TEXT REFERENCES worktree(path);
PRAGMA user_version = 11;
