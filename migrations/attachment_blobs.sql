ALTER TABLE pending_upload ADD COLUMN mime TEXT NOT NULL DEFAULT 'text/plain';
ALTER TABLE pending_upload ADD COLUMN data BLOB;
ALTER TABLE message_attachment ADD COLUMN mime TEXT NOT NULL DEFAULT 'text/plain';
ALTER TABLE message_attachment ADD COLUMN data BLOB;
ALTER TABLE message_attachment ADD COLUMN inline_text INTEGER NOT NULL DEFAULT 1;
