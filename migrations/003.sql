ALTER TABLE message ADD COLUMN origin TEXT;
CREATE TABLE peer_receipt (
    sender_thread TEXT NOT NULL REFERENCES thread(id),
    request_key TEXT NOT NULL,
    message_id TEXT NOT NULL UNIQUE REFERENCES message(id),
    result TEXT NOT NULL,
    PRIMARY KEY (sender_thread, request_key)
) STRICT;
PRAGMA user_version = 3;
