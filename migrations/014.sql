ALTER TABLE run ADD COLUMN queue_held INTEGER NOT NULL DEFAULT 0
    CHECK(queue_held=0 OR (queue_held=1 AND status='queued'));
PRAGMA user_version = 14;
