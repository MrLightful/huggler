.bail on
.headers on
.mode list
.separator " | "

-- Follow each accepted dispatch to the run it created on the addressed thread.
SELECT r.command_id, r.status AS receipt, r.result_sequence, u.ordinal AS run
FROM orchestration_command_receipts AS r
JOIN orchestration_events AS e ON e.command_id = r.command_id AND e.stream_id = r.aggregate_id
JOIN orchestration_v2_projection_runs AS u
  ON u.run_id = json_extract(e.payload_json, '$.id') AND u.thread_id = e.stream_id
WHERE r.command_type = 'message.dispatch' AND e.event_type = 'run.created'
ORDER BY u.ordinal;

.print
SELECT sequence, event_type
FROM orchestration_events
WHERE command_id = 'command:fixture:provider_thread_resume:message-2:8478dfa5-4dc9-4a11-8643-f5bef8200b7d'
ORDER BY sequence;

.print
SELECT e.sequence, u.ordinal AS run, json_extract(e.payload_json, '$.status') AS run_status
FROM orchestration_events AS e
JOIN orchestration_v2_projection_runs AS u ON u.run_id = json_extract(e.payload_json, '$.id')
WHERE e.event_type IN ('run.created', 'run.updated')
ORDER BY e.sequence;

.print
SELECT u.ordinal AS run, u.thread_id AS app_thread,
       json_extract(p.payload_json, '$.nativeThreadRef.nativeId') AS native_thread,
       json_extract(t.payload_json, '$.nativeTurnRef.nativeId') AS native_turn
FROM orchestration_v2_projection_runs AS u
JOIN orchestration_v2_projection_run_attempts AS a ON a.run_id = u.run_id
JOIN orchestration_v2_projection_provider_turns AS t ON t.run_attempt_id = a.attempt_id
JOIN orchestration_v2_projection_provider_threads AS p
  ON p.provider_thread_id = t.provider_thread_id AND p.provider_thread_id = a.provider_thread_id
  AND p.thread_id = u.thread_id
ORDER BY u.ordinal, a.attempt_ordinal, t.ordinal;

.print
SELECT i.ordinal, m.role, json_quote(json_extract(m.payload_json, '$.text')) AS message
FROM orchestration_v2_projection_turn_items AS i
JOIN orchestration_v2_projection_messages AS m ON m.message_id = json_extract(i.payload_json, '$.messageId')
ORDER BY i.ordinal;

.print
SELECT ordinal_within_scope AS checkpoint, status, json_extract(payload_json, '$.files') AS files
FROM orchestration_v2_projection_checkpoints
ORDER BY ordinal_within_scope;

.print
SELECT p.status AS provider_thread, s.status AS provider_session,
       json_extract(s.payload_json, '$.lastError') AS last_error
FROM orchestration_v2_projection_provider_threads AS p
JOIN orchestration_v2_projection_provider_sessions AS s ON s.provider_session_id = p.provider_session_id;
