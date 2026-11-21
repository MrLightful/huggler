ALTER TABLE message ADD COLUMN attachment_caption TEXT;
CREATE TABLE message_attachment (
    message_id TEXT NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    content TEXT NOT NULL,
    PRIMARY KEY (message_id, position)
) STRICT;
PRAGMA user_version=40;
