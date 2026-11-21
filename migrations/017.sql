CREATE TABLE context_fork (
    thread_id TEXT PRIMARY KEY REFERENCES thread(id),
    source TEXT NOT NULL REFERENCES thread(id),
    through_run TEXT NOT NULL REFERENCES run(id),
    context TEXT NOT NULL,
    native_thread TEXT,
    native_turn TEXT,
    native_scope TEXT,
    CHECK ((native_thread IS NULL) = (native_turn IS NULL)),
    CHECK ((native_thread IS NULL) = (native_scope IS NULL))
) STRICT;
PRAGMA user_version = 17;
