ALTER TABLE thread ADD COLUMN model_effort TEXT;
ALTER TABLE thread ADD COLUMN model_service_tier TEXT;
ALTER TABLE turn_effect ADD COLUMN model_effort TEXT;
ALTER TABLE turn_effect ADD COLUMN model_service_tier TEXT;
PRAGMA user_version=43;
