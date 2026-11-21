CREATE TABLE automation (
    id TEXT PRIMARY KEY,
    owner TEXT NOT NULL REFERENCES thread(id) ON DELETE CASCADE,
    spec TEXT NOT NULL, next_at INTEGER, signing_key_digest BLOB
) STRICT;
CREATE TABLE automation_run (
    automation TEXT NOT NULL REFERENCES automation(id) ON DELETE CASCADE,
    occurrence TEXT NOT NULL,
    thread TEXT NOT NULL REFERENCES thread(id), run TEXT NOT NULL REFERENCES run(id),
    PRIMARY KEY (automation, occurrence)
) STRICT;
ALTER TABLE turn_effect ADD COLUMN fresh_session INTEGER NOT NULL DEFAULT 0;
ALTER TABLE thread ADD COLUMN session_reset INTEGER NOT NULL DEFAULT 0;
ALTER TABLE turn_effect ADD COLUMN durable_launch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE run ADD COLUMN session_generation INTEGER NOT NULL DEFAULT 0;
-- Retain positions after deletion so a waiting list subscriber cannot miss the next insert.
CREATE TABLE thread_order (position INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE) STRICT;
INSERT INTO thread_order SELECT rowid,id FROM thread;
CREATE TRIGGER remember_thread AFTER INSERT ON thread BEGIN
    INSERT INTO thread_order(id) VALUES(NEW.id);
END;
PRAGMA user_version=27;
