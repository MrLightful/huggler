-- Each outstanding request may have reached the receiver. A definitive denial
-- releases only its own attempt; a lost response keeps the original intent.
ALTER TABLE remote_task ADD COLUMN unresolved_attempts INTEGER NOT NULL DEFAULT 1;
PRAGMA user_version=34;
