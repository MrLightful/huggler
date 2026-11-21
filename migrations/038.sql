-- Retain list revisions on tombstones so deleting a conversation wakes list subscribers.
ALTER TABLE thread_order ADD COLUMN activity INTEGER NOT NULL DEFAULT 0;
UPDATE thread_order SET activity=coalesce((SELECT max(sequence) FROM event WHERE thread_id=thread_order.id),0);
CREATE INDEX thread_activity ON thread_order(activity);

-- Read actionable work across all conversations without scanning completed requests or runs.
CREATE INDEX open_permission_run ON permission(run_id) WHERE expired=0 AND decision IS NULL;
CREATE INDEX open_question_run ON user_question(run_id) WHERE state IN ('pending','answering');
CREATE INDEX queued_thread ON run(thread_id,queue_held) WHERE status='queued';
PRAGMA user_version=38;
