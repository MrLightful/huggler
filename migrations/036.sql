-- Preserve the host's wording in disposed task results and in their event history.
ALTER TABLE delegated_task ADD COLUMN cancel_message TEXT NOT NULL DEFAULT 'task cancelled';
PRAGMA user_version=36;
