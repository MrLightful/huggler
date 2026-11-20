-- Only the original delegation owns the child thread. Later task launches on
-- that thread are retried through peer_receipt and have no delegation key.
ALTER TABLE task_cancel_receipt RENAME TO old_cancel_receipt;
ALTER TABLE delegated_task RENAME TO old_task;
CREATE TABLE delegated_task (
    id TEXT PRIMARY KEY,
    parent_thread TEXT NOT NULL REFERENCES thread(id),
    parent_run TEXT NOT NULL REFERENCES run(id),
    request_key TEXT,
    intent TEXT,
    child_thread TEXT NOT NULL REFERENCES thread(id),
    child_run TEXT NOT NULL UNIQUE REFERENCES run(id),
    cancelled INTEGER NOT NULL DEFAULT 0,
    result_run TEXT REFERENCES run(id),
    delivery TEXT NOT NULL DEFAULT 'pending'
        CHECK(delivery IN ('pending','acknowledged','delivered','disposed')),
    wake_run TEXT REFERENCES run(id),
    UNIQUE(parent_thread, request_key),
    CHECK ((request_key IS NULL) = (intent IS NULL))
) STRICT;
INSERT INTO delegated_task SELECT * FROM old_task ORDER BY rowid;
CREATE TABLE task_cancel_receipt (
    task_id TEXT NOT NULL REFERENCES delegated_task(id),
    request_key TEXT NOT NULL,
    PRIMARY KEY (task_id, request_key)
) STRICT;
INSERT INTO task_cancel_receipt SELECT * FROM old_cancel_receipt;
DROP TABLE old_cancel_receipt;
DROP TABLE old_task;
CREATE UNIQUE INDEX child_owner ON delegated_task(child_thread) WHERE request_key IS NOT NULL;
CREATE INDEX pending_completion ON delegated_task(parent_thread) WHERE delivery='pending';
CREATE INDEX task_parent_run ON delegated_task(parent_run);

ALTER TABLE run ADD COLUMN task_id TEXT REFERENCES delegated_task(id);
-- Old child runs, including queued work and nested wakes, belonged to the
-- original task until its result was fixed. Later runs were independent.
UPDATE run SET task_id=t.id FROM delegated_task t
WHERE run.thread_id=t.child_thread AND
    (t.result_run IS NULL OR run.ordinal <= (SELECT ordinal FROM run WHERE id=t.result_run));
CREATE INDEX task_runs ON run(task_id,ordinal) WHERE task_id IS NOT NULL;
CREATE TRIGGER bind_child_run AFTER INSERT ON delegated_task BEGIN
    UPDATE run SET task_id=NEW.id WHERE id=NEW.child_run;
END;
PRAGMA user_version = 8;
