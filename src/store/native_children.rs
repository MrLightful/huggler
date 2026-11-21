use super::*;

pub(super) fn record(tx: &Transaction, thread: &ThreadId, child: NativeChild) -> Result<()> {
    tx.execute(
        "INSERT INTO native_child(run_id,native_thread,projection) VALUES(?,?,?)
         ON CONFLICT(run_id,native_thread) DO UPDATE SET projection=excluded.projection",
        params![
            child.run_id.0,
            child.thread_id.0,
            serde_json::to_string(&child)?
        ],
    )?;
    // ACP permissions belong to the session and survive updates while it works.
    // Codex also expires permissions from an earlier turn on the same child.
    let working = child.status.working();
    if !working || child.turn_id.is_some() {
        Database::expire_permissions(
            tx,
            thread,
            &child.run_id,
            Some(&child.thread_id),
            child.turn_id.as_ref().filter(|_| working),
        )?;
    }
    event(tx, thread, Change::NativeChild { child })?;
    Ok(())
}

pub(super) fn for_thread(db: &Connection, thread: &ThreadId) -> Result<Vec<NativeChild>> {
    db.prepare(
        "SELECT c.projection FROM native_child c JOIN run r ON r.id=c.run_id
                WHERE r.thread_id=? ORDER BY r.ordinal,c.rowid",
    )?
    .query_map([&thread.0], |r| r.get::<_, String>(0))?
    .map(|row| Ok(serde_json::from_str(&row?)?))
    .collect()
}

/// On startup every previous provider session is gone. During normal operation
/// only the worker exiting this particular run loses its observation channel.
pub(super) fn disconnect(tx: &Transaction, run: Option<&RunId>) -> Result<()> {
    let open = tx
        .prepare(
            "SELECT r.thread_id,c.projection FROM native_child c JOIN run r ON r.id=c.run_id
                           WHERE c.active AND (?1 IS NULL OR c.run_id=?1) ORDER BY c.rowid",
        )?
        .query_map([run.map(|r| &r.0)], |r| {
            Ok((ThreadId(r.get(0)?), r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (thread, projection) in open {
        let mut child: NativeChild = serde_json::from_str(&projection)?;
        child.status = NativeChildStatus::Disconnected;
        child.can_interrupt = false;
        record(tx, &thread, child)?;
    }
    Ok(())
}
