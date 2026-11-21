CREATE TABLE workspace_setup (
    project TEXT PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
    state TEXT NOT NULL, detail TEXT
) STRICT;
CREATE TABLE allocated_workspace (
    project TEXT PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
    common_dir TEXT NOT NULL, base_commit TEXT NOT NULL
) STRICT;
PRAGMA user_version=28;
