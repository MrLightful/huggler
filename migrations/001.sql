CREATE TABLE project (id TEXT PRIMARY KEY, root TEXT NOT NULL UNIQUE) STRICT;
CREATE TABLE thread (
    id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES project(id),
    name TEXT NOT NULL, model TEXT, native_thread TEXT UNIQUE
) STRICT;
CREATE TABLE run (
    id TEXT PRIMARY KEY, thread_id TEXT NOT NULL REFERENCES thread(id),
    ordinal INTEGER NOT NULL, status TEXT NOT NULL, native_turn TEXT, error TEXT,
    UNIQUE(thread_id, ordinal)
) STRICT;
CREATE UNIQUE INDEX active_run ON run(thread_id) WHERE status IN ('starting','running','waiting');
CREATE TABLE message (
    id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES run(id),
    role TEXT NOT NULL CHECK(role IN ('user','assistant')), text TEXT NOT NULL
) STRICT;
CREATE INDEX message_run ON message(run_id);
CREATE TABLE event (
    sequence INTEGER PRIMARY KEY, thread_id TEXT NOT NULL REFERENCES thread(id), change TEXT NOT NULL
) STRICT;
CREATE INDEX thread_event ON event(thread_id, sequence);
CREATE TABLE receipt (
    command_id TEXT PRIMARY KEY, action TEXT NOT NULL, result TEXT NOT NULL
) STRICT;
CREATE TABLE turn_effect (
    run_id TEXT PRIMARY KEY REFERENCES run(id),
    model TEXT,
    state TEXT NOT NULL CHECK(state IN ('pending','claimed'))
) STRICT;
PRAGMA user_version = 1;
