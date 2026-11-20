ALTER TABLE turn_effect ADD COLUMN plan_mode INTEGER NOT NULL DEFAULT 0;
CREATE TABLE user_question (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES run(id),
    questions TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','answering','answered','expired')),
    responder_run TEXT REFERENCES run(id)
) STRICT;
CREATE INDEX user_question_run ON user_question(run_id, state);
PRAGMA user_version = 7;
