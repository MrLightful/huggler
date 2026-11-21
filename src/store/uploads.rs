//! Attachment bytes and claims are committed in the same transaction as the message.
use super::*;
use crate::attachments::{INLINE_TEXT_BYTES, validate_file};

pub(super) fn uses_preview_schema(tx: &Transaction) -> Result<bool> {
    Ok(tx.prepare(
        "SELECT 1 FROM pragma_table_info('message_attachment') WHERE name='content' AND type='BLOB'",
    )?.exists([])?)
}

pub(super) fn migrate(tx: &Transaction) -> Result<bool> {
    if tx
        .prepare("SELECT 1 FROM pragma_table_info('message_attachment') WHERE name='data'")?
        .exists([])?
    {
        return Ok(false);
    }
    // This extension also applies to hosts on a different numbered migration.
    // Leave the old text column writable for builds that still use it.
    let sql = if uses_preview_schema(tx)? {
        include_str!("../../migrations/attachment_blobs_preview.sql")
    } else {
        include_str!("../../migrations/attachment_blobs.sql")
    };
    tx.execute_batch(sql)?;
    Ok(true)
}

impl Database {
    pub fn prepare_upload(
        &self,
        thread: &ThreadId,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<UploadId> {
        self.thread_position(thread)?;
        let (content, data) = if mime == "text/plain" && bytes.len() <= INLINE_TEXT_BYTES {
            validate_text(name, bytes.len())?;
            let text = std::str::from_utf8(bytes)
                .map_err(|_| Error::Invalid("text attachment requires UTF-8".into()))?;
            (text, None)
        } else {
            validate_file(name, mime, bytes)?;
            ("", Some(bytes))
        };
        let id = UploadId(new_id());
        self.db.execute(
            "INSERT INTO pending_upload(id,thread_id,name,mime,content,data) VALUES(?,?,?,?,?,?)",
            params![id.0, thread.0, name, mime, content, data],
        )?;
        Ok(id)
    }

    pub fn discard_upload(&self, thread: &ThreadId, id: &UploadId) -> Result<()> {
        self.db.execute(
            "DELETE FROM pending_upload WHERE thread_id=? AND id=?",
            params![thread.0, id.0],
        )?;
        Ok(())
    }
}

pub(super) fn validate_text(name: &str, size: usize) -> Result<()> {
    if name.trim().is_empty() || name.len() > 255 || size > INLINE_TEXT_BYTES {
        return Err(Error::Invalid(
            "expected a name up to 255 bytes and at most 64 KiB of text".into(),
        ));
    }
    Ok(())
}

pub(super) fn file_row(row: &Row<'_>) -> rusqlite::Result<Attachment> {
    Ok(Attachment {
        name: row.get(0)?,
        mime: row.get(1)?,
        bytes: row.get(2)?,
    })
}

pub(super) fn claim(
    tx: &Transaction,
    thread: &ThreadId,
    ids: &[UploadId],
    text_only: bool,
) -> Result<Vec<Attachment>> {
    let mut sizes = tx.prepare_cached(
        "SELECT mime,coalesce(length(data),length(CAST(content AS BLOB)))
         FROM pending_upload WHERE id=? AND thread_id=?",
    )?;
    let mut total = 0;
    let mut images = 0;
    for id in ids {
        let (mime, size) = sizes
            .query_row(params![id.0, thread.0], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)? as usize))
            })
            .optional()?
            .ok_or_else(|| Error::Invalid("upload is unavailable for this thread".into()))?;
        if text_only {
            if !mime.starts_with("text/") {
                return Err(Error::Unsupported(
                    "question attachments require UTF-8 text".into(),
                ));
            }
            if size > INLINE_TEXT_BYTES {
                return Err(Error::Invalid(
                    "question attachments can be up to 64 KiB each".into(),
                ));
            }
        }
        total += size;
        if crate::attachments::is_native_image(&mime) {
            images += size;
        }
    }
    crate::attachments::validate_total_size(total, images)?;
    let mut attachments = Vec::with_capacity(ids.len());
    let mut claim = tx.prepare_cached(
        "DELETE FROM pending_upload WHERE id=? AND thread_id=?
         RETURNING name,mime,coalesce(data,CAST(content AS BLOB))",
    )?;
    for id in ids {
        let file = claim
            .query_row(params![id.0, thread.0], file_row)
            .optional()?
            .ok_or_else(|| Error::Invalid("upload is unavailable for this thread".into()))?;
        attachments.push(file);
    }
    Ok(attachments)
}

pub(super) fn receipt_action(action: &Action) -> Result<String> {
    use sha2::{Digest, Sha256};
    // Keep the original receipt format for existing actions. File bytes already live in
    // the message table, so only persist a digest of each payload for idempotency.
    if let Action::EnqueueFiles { text, files } = action {
        Attachment::validate_batch(files)?;
        let files: Vec<_> = files
            .iter()
            .map(|file| {
                (
                    &file.name,
                    &file.mime,
                    format!("{:x}", Sha256::digest(&file.bytes)),
                )
            })
            .collect();
        return Ok(serde_json::to_string(&("enqueue_files", text, files))?);
    }
    Ok(serde_json::to_string(action)?)
}

fn inline_text(file: &Attachment) -> Option<&str> {
    if file.bytes.len() > INLINE_TEXT_BYTES {
        return None;
    }
    if file.mime == "text/plain" {
        return std::str::from_utf8(&file.bytes).ok();
    }
    file.text()
}

pub(super) fn prompt(output: &dyn crate::AgentText, caption: &str, files: &[Attachment]) -> String {
    let parts: Vec<_> = files
        .iter()
        .map(|file| match inline_text(file) {
            Some(text) => output.attachment(&file.name, text),
            None => format!(
                "Attached file {} ({})",
                serde_json::json!(file.name),
                file.mime
            ),
        })
        .collect();
    output.with_attachments(caption, &parts)
}

pub(super) fn save(
    tx: &Transaction,
    message: &MessageId,
    caption: &str,
    files: &[Attachment],
) -> Result<()> {
    tx.execute(
        "UPDATE message SET attachment_caption=? WHERE id=?",
        params![caption, message.0],
    )?;
    let mut insert = tx.prepare_cached(
        "INSERT INTO message_attachment(message_id,position,name,mime,content,data,inline_text)
         VALUES(?,?,?,?,?,?,?)",
    )?;
    for (position, file) in files.iter().enumerate() {
        let text = inline_text(file);
        insert.execute(params![
            message.0,
            position as i64,
            file.name,
            file.mime,
            text.unwrap_or_default(),
            text.is_none().then_some(file.bytes.as_slice()),
            text.is_some()
        ])?;
    }
    Ok(())
}

pub(super) fn load_turn_files(
    tx: &Transaction,
    message: &MessageId,
) -> Result<Vec<(i64, Attachment)>> {
    Ok(tx
        .prepare(
            "SELECT name,mime,coalesce(data,CAST(content AS BLOB)),position FROM message_attachment
         WHERE message_id=? AND inline_text=0 ORDER BY position",
        )?
        .query_map([&message.0], |row| Ok((row.get(3)?, file_row(row)?)))?
        .collect::<rusqlite::Result<_>>()?)
}

pub(super) fn clean_cache(db: &Connection, directory: &Path) -> Result<()> {
    // Cache files are derived from SQLite. A filesystem error during cleanup
    // should not prevent opening the database.
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Ok(());
    };
    let mut present =
        db.prepare("SELECT 1 FROM message_attachment WHERE message_id=? AND inline_text=0")?;
    for entry in entries.flatten() {
        if let Some(id) = entry.file_name().to_str()
            && uuid::Uuid::parse_str(id).is_ok()
            && !present.exists([id])?
        {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}
