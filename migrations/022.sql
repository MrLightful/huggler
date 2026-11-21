ALTER TABLE run ADD COLUMN usage_limit_reset_at INTEGER;
CREATE TABLE usage_recovery (
    thread_id TEXT PRIMARY KEY REFERENCES thread(id),
    run_id TEXT NOT NULL UNIQUE REFERENCES run(id),
    reset_at INTEGER NOT NULL,
    plan_mode INTEGER NOT NULL
) STRICT;
CREATE INDEX usage_recovery_due ON usage_recovery(reset_at);
PRAGMA user_version = 22;
