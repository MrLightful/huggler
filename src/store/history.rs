use super::*;

pub(super) fn message_row(r: &Row<'_>) -> rusqlite::Result<Message> {
    decode_message(r, r.get(3)?)
}

pub(super) fn decode_message(r: &Row<'_>, text: String) -> rusqlite::Result<Message> {
    let origin = r
        .get::<_, Option<String>>(4)?
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?;
    Ok(Message {
        id: MessageId(r.get(0)?),
        run_id: RunId(r.get(1)?),
        role: if r.get::<_, String>(2)? == "user" {
            Role::User
        } else {
            Role::Assistant
        },
        text,
        origin,
    })
}

impl Database {
    pub(super) fn open_permissions(&self, id: &ThreadId) -> Result<Vec<PermissionRequest>> {
        self.db
            .prepare(
                "SELECT request FROM permission p JOIN run r ON p.run_id=r.id
            WHERE r.thread_id=? AND p.expired=0 AND p.decision IS NULL ORDER BY p.rowid",
            )?
            .query_map([&id.0], |r| r.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }

    pub fn sequence(&self, thread: &ThreadId) -> Result<i64> {
        Ok(self.db.query_row(
            "SELECT coalesce(max(sequence),0) FROM event WHERE thread_id=?",
            [&thread.0],
            |r| r.get(0),
        )?)
    }

    pub fn check_replay(&self, thread: &ThreadId, after: i64) -> Result<()> {
        let floor: i64 = self.db.query_row(
            "SELECT replay_floor FROM thread WHERE id=?",
            [&thread.0],
            |r| r.get(0),
        )?;
        if after < floor {
            return Err(Error::CursorExpired);
        }
        Ok(())
    }

    pub fn retain_events(&mut self, thread: &ThreadId, count: u32) -> Result<()> {
        if count == 0 {
            return Err(Error::Invalid("retain at least one event".into()));
        }
        self.thread_position(thread)?;
        let tx = self.db.transaction()?;
        let floor: Option<i64> = tx
            .query_row(
                "SELECT sequence FROM event WHERE thread_id=?
            ORDER BY sequence DESC LIMIT 1 OFFSET ?",
                params![thread.0, count],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(floor) = floor {
            tx.execute(
                "UPDATE thread SET replay_floor=? WHERE id=?",
                params![floor, thread.0],
            )?;
            tx.execute(
                "DELETE FROM event WHERE thread_id=? AND sequence<=?",
                params![thread.0, floor],
            )?;
        }
        Ok(tx.commit()?)
    }

    pub fn read_message(&self, thread: &ThreadId, id: &MessageId) -> Result<Message> {
        self.db
            .query_row(
                "SELECT m.id,m.run_id,m.role,m.text,m.origin
                 FROM message m JOIN run r ON r.id=m.run_id
                 WHERE r.thread_id=? AND m.id=?",
                params![thread.0, id.0],
                message_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(id.to_string()))
    }

    pub fn history(
        &self,
        thread: &ThreadId,
        before: Option<&HistoryCursor>,
        query: &str,
        limit: usize,
    ) -> Result<HistoryPage> {
        if !(1..=100).contains(&limit) {
            return Err(Error::Invalid("history page size must be 1 to 100".into()));
        }
        self.thread_position(thread)?;
        let sequence = self.sequence(thread)?;
        let (slot, row) = if let Some(before) = before {
            if before.query != query || before.thread != *thread {
                return Err(Error::Invalid(
                    "cursor belongs to a different history query".into(),
                ));
            }
            if before.sequence != sequence {
                return Err(Error::HistoryChanged);
            }
            self.db
                .query_row(
                    "SELECT coalesce(r.delivery_slot,r.ordinal),m.rowid
                 FROM message m JOIN run r ON r.id=m.run_id
                 WHERE m.id=? AND r.thread_id=?",
                    params![before.before.0, thread.0],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .ok_or_else(|| Error::Invalid("unknown history cursor".into()))?
        } else {
            (i64::MAX, i64::MAX)
        };
        const PREVIEW_BYTES: usize = 4096;
        let mut stmt = self.db.prepare(
            "SELECT m.id,m.run_id,m.role,substr(CAST(m.text AS BLOB),1,?6),m.origin
             FROM run r CROSS JOIN message m ON m.run_id=r.id WHERE r.thread_id=?1
             AND coalesce(r.delivery_slot,r.ordinal)<=?2
             AND (coalesce(r.delivery_slot,r.ordinal),m.rowid)<(?2,?3)
             AND (?4='' OR instr(CAST(lower(m.text) AS BLOB),CAST(lower(?4) AS BLOB))>0)
             ORDER BY coalesce(r.delivery_slot,r.ordinal) DESC,m.rowid DESC LIMIT ?5",
        )?;
        let mut messages = stmt
            .query_map(
                params![
                    thread.0,
                    slot,
                    row,
                    query,
                    limit as i64 + 1,
                    PREVIEW_BYTES as i64 + 1
                ],
                |r| {
                    let bytes = r.get_ref(3)?.as_blob()?;
                    let truncated = bytes.len() > PREVIEW_BYTES;
                    let bytes = &bytes[..bytes.len().min(PREVIEW_BYTES)];
                    let preview = match std::str::from_utf8(bytes) {
                        Ok(text) => text,
                        Err(e) if truncated && e.error_len().is_none() => {
                            std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap()
                        }
                        Err(e) => return Err(rusqlite::Error::Utf8Error(3, e)),
                    };
                    Ok(HistoryItem {
                        message: decode_message(r, preview.into())?,
                        truncated,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let older = if messages.len() > limit {
            messages.pop();
            Some(HistoryCursor {
                query: query.into(),
                thread: thread.clone(),
                sequence,
                before: messages[limit - 1].message.id.clone(),
            })
        } else {
            None
        };
        messages.reverse();
        Ok(HistoryPage {
            messages,
            older,
            sequence,
        })
    }

    pub fn detail(
        &self,
        thread: &ThreadId,
        before: Option<&HistoryCursor>,
        query: &str,
        limit: usize,
    ) -> Result<ConversationDetail> {
        let history = self.history(thread, before, query, limit)?;
        let (name, instance) = self.db.query_row(
            "SELECT name,provider_instance FROM thread WHERE id=?",
            [&thread.0],
            |r| Ok((r.get(0)?, r.get::<_, String>(1)?)),
        )?;
        let latest_run = self
            .db
            .query_row(
                &format!(
                    "SELECT {RUN_FIELDS} FROM run WHERE thread_id=?
                      ORDER BY coalesce(delivery_slot,ordinal) DESC LIMIT 1"
                ),
                [&thread.0],
                run_row,
            )
            .optional()?;
        let native_children = self
            .db
            .prepare(
                "SELECT c.projection FROM native_child c JOIN run r ON r.id=c.run_id
             WHERE r.thread_id=? AND c.active ORDER BY r.ordinal,c.rowid",
            )?
            .query_map([&thread.0], |r| r.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect::<Result<_>>()?;
        Ok(ConversationDetail {
            id: thread.clone(),
            name,
            provider_instance: instance.into(),
            latest_run,
            history,
            native_children,
            permissions: self.open_permissions(thread)?,
        })
    }
}
