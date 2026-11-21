use super::*;

pub(super) fn active_codex_turn(tx: &Transaction, thread: &ThreadId) -> Result<Option<RunId>> {
    Ok(tx
        .query_row(
            "SELECT r.id FROM run r JOIN thread t ON t.id=r.thread_id
         WHERE t.id=? AND t.provider='codex' AND r.status='running'
         AND r.stop_requested=0 AND r.native_turn IS NOT NULL",
            [&thread.0],
            |r| Ok(RunId(r.get(0)?)),
        )
        .optional()?)
}

pub(super) fn request(
    tx: &Transaction,
    thread: &ThreadId,
    run: &RunId,
    command: &CommandId,
    text: &str,
) -> Result<i64> {
    let driver: String =
        tx.query_row("SELECT provider FROM thread WHERE id=?", [&thread.0], |r| {
            r.get(0)
        })?;
    if driver != "codex" {
        return Err(Error::Unsupported(
            "active steering requires Codex app-server".into(),
        ));
    }
    if text.trim().is_empty() {
        return Err(Error::Invalid("empty steering message".into()));
    }
    if tx.execute(
        "INSERT INTO steer(command_id,run_id,text,outcome)
        SELECT ?1,id,?2,?3 FROM run WHERE id=?4 AND thread_id=?5 AND status='running'
        AND stop_requested=0 AND native_turn IS NOT NULL",
        params![
            command.0,
            text,
            serde_json::to_string(&SteerOutcome::Pending)?,
            run.0,
            thread.0
        ],
    )? == 0
    {
        return Err(Error::Invalid("expected a running foreground turn".into()));
    }
    event(
        tx,
        thread,
        Change::Steer {
            run_id: run.clone(),
            command_id: command.clone(),
            outcome: SteerOutcome::Pending,
        },
    )
}

pub(super) fn settle(
    tx: &Transaction,
    thread: &ThreadId,
    run: &RunId,
    command: &CommandId,
    outcome: SteerOutcome,
) -> Result<()> {
    let text = tx
        .query_row(
            "UPDATE steer SET outcome=?1 WHERE run_id=?2 AND command_id=?3 AND outcome=?4
         RETURNING text",
            params![
                serde_json::to_string(&outcome)?,
                run.0,
                command.0,
                serde_json::to_string(&SteerOutcome::Pending)?
            ],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    if let Some(text) = text {
        if outcome == SteerOutcome::Accepted {
            message(tx, thread, run, Role::User, None, text)?;
        }
        event(
            tx,
            thread,
            Change::Steer {
                run_id: run.clone(),
                command_id: command.clone(),
                outcome,
            },
        )?;
    }
    Ok(())
}

pub(super) fn disconnect(tx: &Transaction, thread: &ThreadId, run: &RunId) -> Result<()> {
    let pending = tx
        .prepare("SELECT command_id FROM steer WHERE run_id=? AND outcome=? ORDER BY rowid")?
        .query_map(
            params![run.0, serde_json::to_string(&SteerOutcome::Pending)?],
            |r| Ok(CommandId(r.get(0)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in pending {
        settle(tx, thread, run, &id, SteerOutcome::Uncertain)?;
    }
    Ok(())
}

impl Database {
    pub fn steer_outcome(&self, thread: &ThreadId, id: &CommandId) -> Result<SteerOutcome> {
        let json: String = self
            .db
            .query_row(
                "SELECT s.outcome FROM steer s JOIN run r ON r.id=s.run_id
             WHERE r.thread_id=? AND s.command_id=?",
                params![thread.0, id.0],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(id.to_string()))?;
        Ok(serde_json::from_str(&json)?)
    }
}
