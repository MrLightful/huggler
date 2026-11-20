CREATE TABLE delegated_task (
    id TEXT PRIMARY KEY,
    parent_thread TEXT NOT NULL REFERENCES thread(id),
    parent_run TEXT NOT NULL REFERENCES run(id),
    request_key TEXT NOT NULL,
    intent TEXT NOT NULL,
    child_thread TEXT NOT NULL UNIQUE REFERENCES thread(id),
    child_run TEXT NOT NULL UNIQUE REFERENCES run(id),
    cancelled INTEGER NOT NULL DEFAULT 0,
    result_run TEXT REFERENCES run(id),
    delivery TEXT NOT NULL DEFAULT 'pending'
        CHECK(delivery IN ('pending','acknowledged','delivered','disposed')),
    wake_run TEXT REFERENCES run(id),
    UNIQUE(parent_thread, request_key)
) STRICT;
CREATE INDEX pending_completion ON delegated_task(parent_thread) WHERE delivery='pending';
ALTER TABLE run ADD COLUMN stop_requested INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 4;
