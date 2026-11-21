CREATE TABLE app_context (
    thread TEXT PRIMARY KEY REFERENCES thread(id) ON DELETE CASCADE,
    content TEXT NOT NULL
) STRICT;
PRAGMA user_version=31;
