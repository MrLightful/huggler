-- Reclaim pending context if its provider failed before prompt submission.
ALTER TABLE app_context ADD COLUMN claimed_run TEXT;
PRAGMA user_version=35;
