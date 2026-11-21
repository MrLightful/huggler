CREATE TABLE secret_request (
    id TEXT PRIMARY KEY, run TEXT NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    request_key TEXT NOT NULL, label TEXT NOT NULL, state TEXT NOT NULL, expires INTEGER NOT NULL,
    UNIQUE(run,request_key)
) STRICT;
CREATE TABLE agent_notice (
    run TEXT NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    request_key TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(run,request_key)
) STRICT;
ALTER TABLE environment ADD COLUMN preferences TEXT NOT NULL DEFAULT '{}';
PRAGMA user_version=29;
