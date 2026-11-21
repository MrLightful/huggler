CREATE TABLE native_child (
    run_id TEXT NOT NULL REFERENCES run(id),
    native_thread TEXT NOT NULL,
    projection TEXT NOT NULL,
    active INTEGER GENERATED ALWAYS AS
        (json_extract(projection,'$.status') IN ('starting','running')) VIRTUAL,
    PRIMARY KEY (run_id, native_thread)
) STRICT;
CREATE INDEX active_native_children ON native_child(run_id) WHERE active;
PRAGMA user_version = 6;
