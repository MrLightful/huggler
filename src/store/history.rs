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

const HISTORY_PREVIEW_BYTES: usize = 4096;

fn history_preview(bytes: &[u8], column: usize) -> rusqlite::Result<(&str, bool)> {
    let truncated = bytes.len() > HISTORY_PREVIEW_BYTES;
    let bytes = &bytes[..bytes.len().min(HISTORY_PREVIEW_BYTES)];
    let preview = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) if truncated && e.error_len().is_none() => {
            std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap()
        }
        Err(e) => return Err(rusqlite::Error::Utf8Error(column, e)),
    };
    Ok((preview, truncated))
}

impl Database {
    #[cfg(feature = "application")]
    pub fn delete_thread(&mut self, thread: &ThreadId) -> Result<()> {
        self.thread_position(thread)?;
        let tx = self.db.transaction()?;
        if tx.prepare("SELECT 1 FROM run WHERE thread_id=? AND status IN ('queued','starting','waiting','running')")?.exists([&thread.0])?
            || tx.prepare("SELECT 1 FROM native_child JOIN run ON run.id=run_id WHERE thread_id=? AND active")?.exists([&thread.0])? {
            return Err(Error::Busy);
        }
        tx.execute("DELETE FROM automation WHERE owner=?", [&thread.0])?;
        tx.execute("DELETE FROM guest_share WHERE thread=?", [&thread.0])?;
        tx.execute(
            "DELETE FROM remote_task WHERE parent_run IN (SELECT id FROM run WHERE thread_id=?)",
            [&thread.0],
        )?;
        tx.execute("DELETE FROM remote_grant WHERE thread=?", [&thread.0])?;
        for table in [
            "pending_upload",
            "context_fork",
            "imported_history",
            "usage_recovery",
            "event",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE thread_id=?"),
                [&thread.0],
            )?;
        }
        for table in [
            "message",
            "provider_usage",
            "permission",
            "user_question",
            "checkpoint",
            "native_child",
            "steer",
            "command_output",
        ] {
            tx.execute(
                &format!(
                    "DELETE FROM {table} WHERE run_id IN (SELECT id FROM run WHERE thread_id=?)"
                ),
                [&thread.0],
            )?;
        }
        tx.execute("DELETE FROM run WHERE thread_id=?", [&thread.0])?;
        tx.execute("DELETE FROM remote_delivery WHERE run_id IS NULL AND grant_id NOT IN (SELECT id FROM remote_grant)", [])?;
        tx.execute("DELETE FROM thread WHERE id=?", [&thread.0])?;
        mark_conversation_changed(&tx, thread)?;
        tx.commit()?;
        Ok(())
    }

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
        self.db
            .query_row(
                "SELECT coalesce((SELECT max(sequence) FROM event WHERE thread_id=t.id),0)
             FROM thread t WHERE t.id=?",
                [&thread.0],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(thread.to_string()))
    }

    pub fn check_replay(&self, thread: &ThreadId, after: i64) -> Result<()> {
        let floor: i64 = self
            .db
            .query_row(
                "SELECT replay_floor FROM thread WHERE id=?",
                [&thread.0],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(thread.to_string()))?;
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

    pub fn read_message_body(&self, thread: &ThreadId, id: &MessageId) -> Result<String> {
        self.db.query_row("SELECT coalesce(m.attachment_caption,m.text) FROM message m JOIN run r ON r.id=m.run_id
            WHERE r.thread_id=? AND m.id=?", params![thread.0,id.0], |row| row.get(0))
            .optional()?.ok_or_else(|| Error::NotFound(id.to_string()))
    }

    pub fn read_attachment(
        &self,
        thread: &ThreadId,
        message: &MessageId,
        position: usize,
    ) -> Result<String> {
        self.db
            .query_row(
                "SELECT a.content FROM message_attachment a JOIN message m ON m.id=a.message_id
             JOIN run r ON r.id=m.run_id WHERE r.thread_id=? AND m.id=? AND a.position=?",
                params![thread.0, message.0, position as i64],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(message.to_string()))
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
        let mut stmt = self.db.prepare(
            "SELECT m.id,m.run_id,m.role,coalesce(substr(CAST(m.text AS BLOB),1,?6),x''),m.origin,
             CASE WHEN m.attachment_caption IS NOT NULL THEN
                coalesce(substr(CAST(m.attachment_caption AS BLOB),1,?6),x'')
             END,
             CASE WHEN m.attachment_caption IS NOT NULL THEN
                (SELECT json_group_array(name ORDER BY position) FROM message_attachment WHERE message_id=m.id)
             END
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
                    HISTORY_PREVIEW_BYTES as i64 + 1
                ],
                |r| {
                    let (preview, truncated) = history_preview(r.get_ref(3)?.as_blob()?, 3)?;
                    Ok(HistoryItem {
                        message: decode_message(r, preview.into())?,
                        files: match r.get_ref(5)?.as_blob_or_null()? {
                            Some(bytes) => {
                                let (caption, truncated) = history_preview(bytes, 5)?;
                                Some(MessageFiles {
                                    caption: caption.to_owned(),
                                    truncated,
                                    names: serde_json::from_str(r.get_ref(6)?.as_str()?).map_err(
                                        |e| {
                                            rusqlite::Error::FromSqlConversionFailure(
                                                6,
                                                rusqlite::types::Type::Text,
                                                Box::new(e),
                                            )
                                        },
                                    )?,
                                })
                            }
                            None => None,
                        },
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

    fn streaming_text(&self, thread: &ThreadId, run: &RunId) -> Result<String> {
        // Walk the run_output_delta index backwards until the preview is full.
        const LIMIT: usize = 4096;
        let mut statement = self.db.prepare(
            "SELECT substr(json_extract(change,'$.text'), -4096)
             FROM event WHERE thread_id=?
               AND json_extract(change,'$.type')='output_delta'
               AND json_extract(change,'$.text')!=''
               AND json_extract(change,'$.run_id')=?
             ORDER BY sequence DESC",
        )?;
        let chunks =
            statement.query_map(params![thread.0, run.0], |row| row.get::<_, String>(0))?;
        let mut parts = Vec::new();
        let mut remaining = LIMIT;
        for chunk in chunks {
            let mut chunk = chunk?;
            let start = chunk.ceil_char_boundary(chunk.len().saturating_sub(remaining));
            chunk.drain(..start);
            remaining -= chunk.len();
            parts.push(chunk);
            if start != 0 || remaining == 0 {
                break;
            }
        }
        parts.reverse();
        Ok(parts.concat())
    }

    pub fn detail(
        &self,
        thread: &ThreadId,
        before: Option<&HistoryCursor>,
        query: &str,
        limit: usize,
    ) -> Result<ConversationDetail> {
        let history = self.history(thread, before, query, limit)?;
        let (name, instance, model, settled) = self.db.query_row(
            "SELECT name,provider_instance,model,settled FROM thread WHERE id=?",
            [&thread.0],
            |r| Ok((r.get(0)?, r.get::<_, String>(1)?, r.get(2)?, r.get(3)?)),
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
        let foreground_run = self
            .db
            .query_row(
                &format!(
                    "SELECT {RUN_FIELDS} FROM run WHERE thread_id=?
                      AND status IN ('starting','running','waiting')"
                ),
                [&thread.0],
                run_row,
            )
            .optional()?;
        // Outstanding effects are removed on completion. Scan those before
        // looking up their runs, so painting never scans settled history.
        let (queued_count, queue_held) = self.db.query_row(
            "SELECT count(*), coalesce(max(r.queue_held),0)
             FROM turn_effect e CROSS JOIN run r ON r.id=e.run_id
             WHERE r.thread_id=? AND r.status='queued'",
            [&thread.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut streaming_text = String::new();
        let mut command_items = Vec::new();
        if let Some(run) = foreground_run.as_ref().or(latest_run.as_ref()) {
            if run.status == RunStatus::Running {
                streaming_text = self.streaming_text(thread, &run.id)?;
            }
            command_items = self.command_items(thread, &run.id)?;
        }
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
            settled,
            id: thread.clone(),
            name,
            provider_instance: instance.into(),
            model,
            latest_run,
            foreground_run,
            queued_count,
            queue_held,
            streaming_text,
            command_items,
            questions: self.open_questions(thread)?,
            history,
            native_children,
            permissions: self.open_permissions(thread)?,
        })
    }
}
