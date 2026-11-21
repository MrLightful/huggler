-- Receipt identities outlive the destination's local history. A retry must never
-- create another child when the original was deliberately deleted.
CREATE TABLE remote_delivery_v33 (
    grant_id TEXT NOT NULL, sender_run TEXT NOT NULL, request_key TEXT NOT NULL, intent TEXT NOT NULL,
    receipt TEXT NOT NULL, run_id TEXT UNIQUE REFERENCES run(id) ON DELETE SET NULL,
    PRIMARY KEY(grant_id,sender_run,request_key)
) STRICT;
INSERT INTO remote_delivery_v33 SELECT * FROM remote_delivery ORDER BY rowid;
DROP TABLE remote_delivery;
ALTER TABLE remote_delivery_v33 RENAME TO remote_delivery;

-- A schedule owns its occurrence history even after a destination is removed.
CREATE TABLE automation_run_v33 (
    automation TEXT NOT NULL REFERENCES automation(id) ON DELETE CASCADE,
    occurrence TEXT NOT NULL, thread TEXT NOT NULL, run TEXT NOT NULL,
    PRIMARY KEY(automation, occurrence)
) STRICT;
INSERT INTO automation_run_v33 SELECT * FROM automation_run ORDER BY rowid;
DROP TABLE automation_run;
ALTER TABLE automation_run_v33 RENAME TO automation_run;
PRAGMA user_version=33;
