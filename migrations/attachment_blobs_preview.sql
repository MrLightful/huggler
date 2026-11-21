ALTER TABLE pending_upload RENAME TO preview_pending_upload;
CREATE TABLE pending_upload (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES thread(id),
    name TEXT NOT NULL,
    content TEXT NOT NULL,
    mime TEXT NOT NULL DEFAULT 'text/plain',
    data BLOB
) STRICT;
INSERT INTO pending_upload
    SELECT id, thread_id, name,
           CASE WHEN mime='text/plain' AND length(content)<=65536 THEN CAST(content AS TEXT) ELSE '' END,
           mime,
           CASE WHEN mime='text/plain' AND length(content)<=65536 THEN NULL ELSE content END
    FROM preview_pending_upload;
DROP TABLE preview_pending_upload;
CREATE INDEX pending_upload_thread ON pending_upload(thread_id);

ALTER TABLE message_attachment RENAME TO preview_message_attachment;
CREATE TABLE message_attachment (
    message_id TEXT NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    content TEXT NOT NULL,
    mime TEXT NOT NULL DEFAULT 'text/plain',
    data BLOB,
    inline_text INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (message_id, position)
) STRICT;
INSERT INTO message_attachment
    SELECT message_id, position, name,
           CASE WHEN inline_text THEN CAST(content AS TEXT) ELSE '' END,
           mime,
           CASE WHEN inline_text THEN NULL ELSE content END,
           inline_text
    FROM preview_message_attachment;
DROP TABLE preview_message_attachment;
