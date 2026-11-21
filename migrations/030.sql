CREATE TABLE repository_binding (
    id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    driver TEXT NOT NULL, origin TEXT NOT NULL, repository TEXT NOT NULL,
    UNIQUE(project, driver, origin, repository)
) STRICT;
CREATE TABLE repository_link (
    id TEXT PRIMARY KEY, thread TEXT NOT NULL REFERENCES thread(id) ON DELETE CASCADE,
    binding TEXT NOT NULL REFERENCES repository_binding(id) ON DELETE CASCADE, item TEXT NOT NULL,
    UNIQUE(thread, binding, item)
) STRICT;
CREATE TABLE repository_watch (
    id TEXT PRIMARY KEY, link TEXT NOT NULL UNIQUE REFERENCES repository_link(id) ON DELETE CASCADE,
    revision INTEGER, state TEXT
) STRICT;
PRAGMA user_version=30;
