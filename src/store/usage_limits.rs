use super::*;

pub(super) fn record(
    tx: &Transaction,
    thread: &ThreadId,
    run: &RunId,
    reset: Option<i64>,
) -> Result<()> {
    tx.execute(
        "UPDATE run SET usage_limit_reset_at=? WHERE id=?",
        params![reset, run.0],
    )?;
    // A newer prompt, a queued run or an owned task takes precedence. Only a
    // future provider reset can arm a continuation for this failure.
    tx.execute(
        "INSERT INTO usage_recovery(thread_id,run_id,reset_at,plan_mode)
         SELECT r.thread_id,r.id,?1,e.plan_mode FROM run r JOIN turn_effect e ON e.run_id=r.id
         WHERE r.id=?2 AND r.thread_id=?3 AND r.status='running' AND r.stop_requested=0
         AND r.task_id IS NULL AND ?1>unixepoch()
         AND NOT EXISTS(SELECT 1 FROM run other WHERE other.thread_id=r.thread_id
             AND (other.status='queued' OR coalesce(other.delivery_slot,other.ordinal)>coalesce(r.delivery_slot,r.ordinal)))",
        params![reset,run.0,thread.0],
    )?;
    Ok(())
}

impl Database {
    pub fn resume_limited(&mut self) -> Result<(Option<u64>, Vec<RunJob>)> {
        let tx = self.db.transaction()?;
        let now: i64 = tx.query_row("SELECT unixepoch()", [], |r| r.get(0))?;
        let due = tx.prepare(
            "DELETE FROM usage_recovery WHERE reset_at<=? RETURNING thread_id,run_id,reset_at,plan_mode"
        )?.query_map([now], |r| Ok((ThreadId(r.get(0)?),RunId(r.get(1)?),r.get::<_,i64>(2)?,r.get::<_,bool>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut jobs = Vec::new();
        for (thread, failed, reset, plan) in due {
            if now - reset > 86_400 {
                continue;
            }
            let model: Option<String> =
                tx.query_row("SELECT model FROM thread WHERE id=?", [&thread.0], |r| {
                    r.get(0)
                })?;
            let (next, _, _, _) = start_message(
                &tx,
                &thread,
                self.output.text.resume_after_limit(),
                model,
                None,
                false,
            )?;
            tx.execute(
                "UPDATE turn_effect SET plan_mode=? WHERE run_id=?",
                params![plan, next.0],
            )?;
            event(
                &tx,
                &thread,
                Change::UsageLimitContinuation {
                    failed,
                    run_id: next.clone(),
                },
            )?;
            jobs.push((thread, next));
        }
        let delay = tx.query_row("SELECT min(reset_at)-? FROM usage_recovery", [now], |r| {
            r.get::<_, Option<i64>>(0)
        })?;
        tx.commit()?;
        Ok((delay.map(i64::unsigned_abs), jobs))
    }
}
