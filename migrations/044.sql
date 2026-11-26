-- Track title updates independently of activity so a rename refreshes lists
-- without moving the conversation, and stale title snapshots survive pruning.
ALTER TABLE thread_order ADD COLUMN title_revision INTEGER NOT NULL DEFAULT 0;
CREATE INDEX thread_title_revision ON thread_order(title_revision);
PRAGMA user_version=44;
