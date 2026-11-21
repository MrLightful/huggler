ALTER TABLE run ADD COLUMN stop_reason TEXT;
CREATE TABLE run_interrupt_receipt (
    sender_thread TEXT NOT NULL REFERENCES thread(id),
    request_key TEXT NOT NULL,
    run_id TEXT NOT NULL REFERENCES run(id),
    reason TEXT,
    PRIMARY KEY (sender_thread, request_key)
) STRICT;
PRAGMA user_version = 9;
