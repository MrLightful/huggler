-- Read a running turn's newest text without scanning unrelated thread activity.
CREATE INDEX run_output_delta ON event(thread_id, json_extract(change, '$.run_id'), sequence)
WHERE json_extract(change, '$.type') = 'output_delta' AND json_extract(change, '$.text') != '';
PRAGMA user_version=37;
