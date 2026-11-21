CREATE TABLE command_output (
    run_id TEXT NOT NULL REFERENCES run(id),
    item TEXT NOT NULL,
    revision INTEGER NOT NULL,
    text TEXT NOT NULL,
    truncated INTEGER NOT NULL,
    finished INTEGER NOT NULL,
    PRIMARY KEY(run_id, item)
) STRICT;
PRAGMA user_version = 23;
