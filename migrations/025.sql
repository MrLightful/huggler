CREATE TABLE environment (id TEXT PRIMARY KEY) STRICT;
INSERT INTO environment VALUES(lower(hex(randomblob(16))));

-- Only the peer keeps the bearer. The host stores its digest and the approved origin.
CREATE TABLE remote_grant (
    id TEXT PRIMARY KEY, digest BLOB NOT NULL UNIQUE, thread TEXT NOT NULL REFERENCES thread(id),
    sender_env TEXT NOT NULL, sender_thread TEXT NOT NULL, operate INTEGER NOT NULL
) STRICT;
CREATE TABLE remote_delivery (
    grant_id TEXT NOT NULL, sender_run TEXT NOT NULL, request_key TEXT NOT NULL, intent TEXT NOT NULL,
    receipt TEXT NOT NULL, run_id TEXT NOT NULL UNIQUE REFERENCES run(id),
    PRIMARY KEY(grant_id,sender_run,request_key)
) STRICT;
CREATE TABLE remote_task (
    parent_run TEXT NOT NULL REFERENCES run(id), request_key TEXT NOT NULL, destination TEXT NOT NULL,
    task TEXT NOT NULL, child TEXT, child_run TEXT, wake TEXT REFERENCES run(id),
    PRIMARY KEY(parent_run,request_key)
) STRICT;
PRAGMA user_version = 25;
