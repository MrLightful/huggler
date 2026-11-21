CREATE TABLE guest_share (
    id TEXT PRIMARY KEY, thread TEXT NOT NULL REFERENCES thread(id),
    invite BLOB UNIQUE, session BLOB UNIQUE, name TEXT
) STRICT;
CREATE TABLE guest_submission (
    id TEXT PRIMARY KEY, share TEXT NOT NULL REFERENCES guest_share(id) ON DELETE CASCADE,
    request_key TEXT NOT NULL, text TEXT NOT NULL, decision TEXT, run TEXT REFERENCES run(id),
    UNIQUE(share,request_key)
) STRICT;
PRAGMA user_version=26;
