CREATE TABLE pending_upload (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES thread(id),
    name TEXT NOT NULL,
    content TEXT NOT NULL
) STRICT;
CREATE INDEX pending_upload_thread ON pending_upload(thread_id);
PRAGMA user_version = 18;
