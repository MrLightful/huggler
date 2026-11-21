-- Keep the approved origin and target of past writes after disabling the bearer.
ALTER TABLE remote_grant ADD COLUMN revoked INTEGER NOT NULL DEFAULT 0;
CREATE INDEX remote_grant_origin ON remote_grant(thread, sender_env, sender_thread);
PRAGMA user_version=32;
