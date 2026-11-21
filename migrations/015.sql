-- Reordering swaps delivery slots while preserving run identity and creation ordinal.
ALTER TABLE run ADD COLUMN delivery_slot INTEGER;
CREATE INDEX run_delivery ON run(thread_id,coalesce(delivery_slot,ordinal));
ALTER TABLE peer_receipt ADD COLUMN original_text TEXT;
UPDATE peer_receipt SET original_text=(SELECT text FROM message WHERE id=message_id);
PRAGMA user_version = 15;
