DROP INDEX active_native_children;
ALTER TABLE native_child DROP COLUMN active;
ALTER TABLE native_child ADD COLUMN active INTEGER GENERATED ALWAYS AS
    (json_extract(projection,'$.status') IN ('starting','running','requires_action')) VIRTUAL;
CREATE INDEX active_native_children ON native_child(run_id) WHERE active;
PRAGMA user_version = 12;
