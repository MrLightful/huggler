use super::*;
use crate::automation::{Delivery, Destination, Occurrence, Schedule, Trigger};
use sha2::{Digest, Sha256};

fn occurrence(row: &Row<'_>) -> rusqlite::Result<Occurrence> {
    Ok(Occurrence {
        key: row.get(0)?,
        thread: ThreadId(row.get(1)?),
        run: RunId(row.get(2)?),
    })
}

fn release(
    tx: &Transaction,
    id: &ScheduleId,
    owner: ThreadId,
    spec: &Schedule,
    key: String,
) -> Result<(Option<Occurrence>, Option<RunJob>)> {
    let prior = tx
        .query_row(
            "SELECT occurrence,thread,run FROM automation_run WHERE automation=? AND occurrence=?",
            params![id.0, key],
            occurrence,
        )
        .optional()?;
    if prior.is_some() {
        return Ok((prior, None));
    }
    let thread = match spec.destination {
        Destination::NewThread => {
            creation::create_agent_thread(tx, &owner, "Scheduled task", None)?
        }
        _ => owner,
    };
    let model = tx.query_row("SELECT model FROM thread WHERE id=?", [&thread.0], |r| {
        r.get(0)
    })?;
    let (run, job) = start_durable_message(
        tx,
        &thread,
        &spec.prompt,
        model,
        matches!(spec.destination, Destination::FreshSession),
    )?;
    tx.execute(
        "INSERT INTO automation_run VALUES(?,?,?,?)",
        params![id.0, key, thread.0, run.0],
    )?;
    Ok((Some(Occurrence { key, thread, run }), job))
}

impl Database {
    pub fn create_schedule(
        &self,
        owner: &ThreadId,
        spec: &Schedule,
        now: i64,
        signing_key: Option<&[u8]>,
    ) -> Result<ScheduleId> {
        self.thread_position(owner)?;
        let webhook = matches!(spec.trigger, Trigger::Webhook { .. });
        if spec.prompt.trim().is_empty()
            || webhook != signing_key.is_some()
            || signing_key.is_some_and(|k| k.len() < 16)
            || matches!(&spec.trigger, Trigger::Webhook {event} if event.trim().is_empty())
        {
            return Err(Error::Invalid(
                "expected a prompt and a signing key of at least 16 bytes for webhook triggers"
                    .into(),
            ));
        }
        let next = match spec.trigger {
            Trigger::Once { at } => Some(at),
            Trigger::Interval {
                first,
                seconds: 1..,
            } => Some(first),
            _ => spec.trigger.next(now)?,
        };
        let id = ScheduleId(new_id());
        self.db.execute(
            "INSERT INTO automation VALUES(?,?,?,?,?)",
            params![
                id.0,
                owner.0,
                serde_json::to_string(spec)?,
                next,
                signing_key.map(|k| Sha256::digest(k).to_vec())
            ],
        )?;
        Ok(id)
    }
    pub fn schedules(&self, thread: &ThreadId) -> Result<Vec<(ScheduleId, Schedule, Option<i64>)>> {
        let mut stmt = self
            .db
            .prepare("SELECT id,spec,next_at FROM automation WHERE owner=? ORDER BY rowid")?;
        let mut rows = stmt.query([&thread.0])?;
        let mut result = vec![];
        while let Some(r) = rows.next()? {
            result.push((
                ScheduleId(r.get(0)?),
                serde_json::from_str(&r.get::<_, String>(1)?)?,
                r.get(2)?,
            ));
        }
        Ok(result)
    }
    pub fn tick_schedule(
        &mut self,
        id: &ScheduleId,
        now: i64,
    ) -> Result<(Option<Occurrence>, Option<RunJob>)> {
        let tx = self.db.transaction()?;
        let (owner, spec, due) = tx
            .query_row(
                "SELECT owner,spec,next_at FROM automation WHERE id=?",
                [&id.0],
                |r| {
                    Ok((
                        ThreadId(r.get(0)?),
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(id.to_string()))?;
        let Some(due) = due.filter(|t| *t <= now) else {
            return Ok((None, None));
        };
        let spec: Schedule = serde_json::from_str(&spec)?;
        let released = release(&tx, id, owner, &spec, due.to_string())?;
        tx.execute(
            "UPDATE automation SET next_at=? WHERE id=?",
            params![spec.trigger.next(now)?, id.0],
        )?;
        tx.commit()?;
        Ok(released)
    }

    pub fn deliver_webhook(
        &mut self,
        id: &ScheduleId,
        key: &[u8],
        delivery: Delivery,
    ) -> Result<(Option<Occurrence>, Option<RunJob>)> {
        let tx = self.db.transaction()?;
        let (owner, spec) = tx
            .query_row(
                "SELECT owner,spec FROM automation WHERE id=? AND signing_key_digest=?",
                params![id.0, Sha256::digest(key).to_vec()],
                |r| Ok((ThreadId(r.get(0)?), r.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or(Error::Forbidden)?;
        let spec: Schedule = serde_json::from_str(&spec)?;
        if !matches!(&spec.trigger, Trigger::Webhook { event } if event == &delivery.event) {
            return Ok((None, None));
        }
        if delivery.id.trim().is_empty() || delivery.id.len() > 256 {
            return Err(Error::Invalid("invalid webhook delivery ID".into()));
        }
        let released = release(&tx, id, owner, &spec, delivery.id)?;
        tx.commit()?;
        Ok(released)
    }
    pub fn schedule_history(&self, id: &ScheduleId) -> Result<Vec<Occurrence>> {
        Ok(self.db.prepare("SELECT occurrence,thread,run FROM automation_run WHERE automation=? ORDER BY rowid")?
            .query_map([&id.0], occurrence)?.collect::<rusqlite::Result<_>>()?)
    }
    pub fn delete_schedule(&self, id: &ScheduleId) -> Result<bool> {
        Ok(self
            .db
            .execute("DELETE FROM automation WHERE id=?", [&id.0])?
            != 0)
    }
}
