//! Immutable context captured in the same transaction as the destination prompt.
use super::*;

const BUDGET: usize = 16 * 1024;

pub(super) fn capture(
    tx: &Transaction,
    source: &ThreadId,
    through: &RunId,
    mode: ForkMode,
    inherit: bool,
) -> Result<ForkHistory> {
    let slot: i64 = tx
        .query_row(
            "SELECT coalesce(delivery_slot,ordinal) FROM run
        WHERE thread_id=? AND id=? AND status='completed'",
            params![source.0, through.0],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::Invalid("select a completed source turn".into()))?;
    let mut history = ForkHistory {
        source: source.clone(),
        through: through.clone(),
        mode,
        messages: vec![],
        truncated: false,
    };
    let mut bytes = serde_json::to_vec(&history)?.len();
    if bytes > BUDGET {
        return Err(Error::Invalid(
            "source identity exceeds context budget".into(),
        ));
    }
    let mut stmt=tx.prepare("SELECT m.id,m.run_id,m.role,substr(CAST(m.text AS BLOB),1,2048),m.origin,length(CAST(m.text AS BLOB))>2048
        FROM run r JOIN message m ON r.id=m.run_id WHERE r.thread_id=? AND r.status='completed'
        AND coalesce(r.delivery_slot,r.ordinal)<=? ORDER BY coalesce(r.delivery_slot,r.ordinal) DESC,m.rowid DESC")?;
    let mut rows = stmt.query(params![source.0, slot])?;
    while let Some(row) = rows.next()? {
        let chunk: Vec<u8> = row.get(3)?;
        let text = std::str::from_utf8(&chunk)
            .unwrap_or_else(|e| std::str::from_utf8(&chunk[..e.valid_up_to()]).unwrap());
        let message = super::history::decode_message(row, text.into())?;
        bytes += serde_json::to_vec(&message)?.len() + 1;
        if bytes > BUDGET {
            history.truncated = true;
            break;
        }
        history.truncated |= row.get::<_, bool>(5)?;
        history.messages.push(message);
    }
    if inherit {
        let ancestor: Option<String> = tx
            .query_row(
                "SELECT context FROM context_fork WHERE thread_id=?",
                [&source.0],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(ancestor) = ancestor {
            let ancestor: ForkHistory = serde_json::from_str(&ancestor)?;
            history.truncated |= ancestor.truncated;
            for message in ancestor.messages.into_iter().rev() {
                bytes += serde_json::to_vec(&message)?.len() + 1;
                if bytes > BUDGET {
                    history.truncated = true;
                    break;
                }
                history.messages.push(message);
            }
        }
    }
    history.messages.reverse();
    Ok(history)
}

pub(super) fn save_fork(
    tx: &Transaction,
    thread: &ThreadId,
    history: &ForkHistory,
    instance: &ProviderInstanceId,
) -> Result<()> {
    let native = if history.mode == ForkMode::Native {
        Some(tx.query_row("SELECT t.native_thread,r.native_turn,t.continuation_key FROM thread t
            JOIN run r ON r.thread_id=t.id JOIN configured_provider c ON c.id=t.provider_instance AND c.scope=t.continuation_key
            WHERE t.id=? AND r.id=? AND c.id=? AND t.provider=c.driver AND c.driver='codex'
            AND t.native_thread IS NOT NULL AND r.native_turn IS NOT NULL",params![history.source.0,history.through.0,instance.as_str()],
            |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?
            .ok_or_else(|| Error::Unsupported("native fork requires the original Codex instance and a bound turn".into()))?)
    } else {
        None
    };
    tx.execute("INSERT INTO context_fork(thread_id,source,through_run,context,native_thread,native_turn,native_scope) VALUES(?,?,?,?,?,?,?)",
        params![thread.0,history.source.0,history.through.0,serde_json::to_string(history)?,native.as_ref().map(|n|&n.0),native.as_ref().map(|n|&n.1),native.as_ref().map(|n|&n.2)])?;
    Ok(())
}

pub(super) fn prompt(context: &str, text: &str) -> String {
    format!("Conversation history from Huggler (JSON)\n{context}\n\n{text}")
}

pub(super) fn prepare(tx: &Transaction, id: &RunId, scope: &str, turn: &mut Turn) -> Result<()> {
    if turn.resume.is_some() {
        return Ok(());
    }
    let mut stmt=tx.prepare("SELECT f.context,f.native_thread,f.native_turn,f.native_scope FROM context_fork f JOIN run r ON r.thread_id=f.thread_id WHERE r.id=?")?;
    let mut rows = stmt.query([&id.0])?;
    if let Some(row) = rows.next()? {
        if let Some(parent) = row.get::<_, Option<String>>(1)? {
            if row.get::<_, String>(3)? != scope {
                return Err(Error::Invalid(
                    "restore the source instance configuration to fork".into(),
                ));
            }
            turn.fork = Some((NativeThreadId(parent), NativeTurnId(row.get(2)?)));
        } else {
            turn.text = prompt(&row.get::<_, String>(0)?, &turn.text);
        }
    }
    Ok(())
}

impl Database {
    pub fn fork_history(&self, thread: &ThreadId) -> Result<Option<ForkHistory>> {
        self.thread_position(thread)?;
        self.db
            .query_row(
                "SELECT context FROM context_fork WHERE thread_id=?",
                [&thread.0],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(Into::into)
    }
}
