CREATE TABLE checkpoint (
    run_id TEXT PRIMARY KEY REFERENCES run(id),
    common_dir TEXT NOT NULL,
    git_commit TEXT NOT NULL
) STRICT;
PRAGMA user_version = 20;
