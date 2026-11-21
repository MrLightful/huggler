CREATE TABLE steer (
    command_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES run(id),
    text TEXT NOT NULL,
    outcome TEXT NOT NULL
) STRICT;
CREATE INDEX steer_run ON steer(run_id);
PRAGMA user_version = 21;
