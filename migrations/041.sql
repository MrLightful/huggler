CREATE TABLE run_activity (
    run_id TEXT NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    item TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    PRIMARY KEY (run_id, item_id)
) STRICT;
PRAGMA user_version=41;
