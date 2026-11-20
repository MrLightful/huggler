CREATE TABLE peer_creation (
    caller_thread TEXT NOT NULL REFERENCES thread(id),
    request_key TEXT NOT NULL,
    position INTEGER NOT NULL,
    thread_id TEXT NOT NULL UNIQUE REFERENCES thread(id),
    initial_run TEXT UNIQUE REFERENCES run(id),
    requested_title TEXT,
    requested_provider TEXT,
    PRIMARY KEY (caller_thread, request_key, position)
) STRICT;
PRAGMA user_version = 10;
