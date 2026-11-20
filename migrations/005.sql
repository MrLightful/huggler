CREATE TABLE task_cancel_receipt (
    task_id TEXT NOT NULL REFERENCES delegated_task(id),
    request_key TEXT NOT NULL,
    PRIMARY KEY (task_id, request_key)
) STRICT;
PRAGMA user_version = 5;
