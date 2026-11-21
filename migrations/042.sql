ALTER TABLE message ADD COLUMN timestamp_ms INTEGER;
ALTER TABLE run ADD COLUMN started_at_ms INTEGER;
ALTER TABLE run ADD COLUMN finished_at_ms INTEGER;
PRAGMA user_version=42;
