use super::*;

pub(super) fn require_queued(tx: &Transaction, thread: &ThreadId, run: &RunId) -> Result<()> {
    let status: Option<String> = tx
        .query_row(
            "SELECT status FROM run WHERE thread_id=? AND id=?",
            params![thread.0, run.0],
            |r| r.get(0),
        )
        .optional()?;
    match status.as_deref() {
        Some("queued") => Ok(()),
        None => Err(Error::NotFound(run.to_string())),
        Some(_) => Err(Error::Invalid("run is no longer queued".into())),
    }
}

pub(super) fn edit(
    tx: &Transaction,
    thread: &ThreadId,
    run: &RunId,
    text: &str,
) -> Result<(MessageId, i64)> {
    require_queued(tx, thread, run)?;
    if text.trim().is_empty() {
        return Err(Error::Invalid("empty message".into()));
    }
    let (id, origin) = tx.query_row(
        "UPDATE message SET attachment_caption=NULL,text=? WHERE run_id=? AND role='user' RETURNING id,origin",
        params![text, run.0],
        |r| Ok((MessageId(r.get(0)?), r.get::<_, Option<String>>(1)?)),
    )?;
    tx.execute("DELETE FROM message_attachment WHERE message_id=?", [&id.0])?;
    let seq = event(
        tx,
        thread,
        Change::Message {
            message: Message {
                id: id.clone(),
                run_id: run.clone(),
                role: Role::User,
                origin: origin
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
                text: text.into(),
            },
        },
    )?;
    Ok((id, seq))
}

pub(super) fn move_before(
    tx: &Transaction,
    thread: &ThreadId,
    run: &RunId,
    before: Option<&RunId>,
) -> Result<Vec<i64>> {
    require_queued(tx, thread, run)?;
    let rows=tx.prepare("SELECT id,coalesce(delivery_slot,ordinal) AS slot FROM run WHERE thread_id=? AND status='queued' ORDER BY slot")?
        .query_map([&thread.0],|r|Ok((RunId(r.get(0)?),r.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut ordered: Vec<_> = rows
        .iter()
        .map(|(id, _)| id)
        .filter(|id| *id != run)
        .collect();
    let index = match before {
        Some(id) => ordered
            .iter()
            .position(|other| *other == id)
            .ok_or_else(|| {
                Error::Invalid("expected a different queued run on this thread".into())
            })?,
        None => ordered.len(),
    };
    ordered.insert(index, run);
    let mut events = Vec::new();
    for (next, (previous, slot)) in ordered.into_iter().zip(&rows) {
        if next != previous {
            tx.execute(
                "UPDATE run SET delivery_slot=? WHERE id=?",
                params![slot, next.0],
            )?;
            events.push(run_event(tx, thread, next)?);
        }
    }
    Ok(events)
}
