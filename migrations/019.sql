CREATE TABLE imported_history (
    thread_id TEXT PRIMARY KEY REFERENCES thread(id),
    provenance TEXT NOT NULL,
    through_run TEXT NOT NULL REFERENCES run(id)
) STRICT;
PRAGMA user_version = 19;
