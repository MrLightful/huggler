//! Text attachments are carried inline across drivers. Staging and claim both live in SQLite.
use super::*;

impl Database {
    pub fn prepare_upload(
        &self,
        thread: &ThreadId,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<UploadId> {
        self.thread_position(thread)?;
        if mime != "text/plain" {
            return Err(Error::Unsupported(
                "only text/plain uploads are supported".into(),
            ));
        }
        validate(name, bytes.len())?;
        let content = std::str::from_utf8(bytes)
            .map_err(|_| Error::Invalid("text attachment requires UTF-8".into()))?;
        let id = UploadId(new_id());
        self.db.execute(
            "INSERT INTO pending_upload(id,thread_id,name,content) VALUES(?,?,?,?)",
            params![id.0, thread.0, name, content],
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

pub(super) fn validate(name: &str, size: usize) -> Result<()> {
    if name.trim().is_empty() || name.len() > 255 || size > 64 * 1024 {
        return Err(Error::Invalid(
            "expected a name up to 255 bytes and at most 64 KiB of text".into(),
        ));
    }
    Ok(())
}

pub(super) fn claim(
    tx: &Transaction,
    thread: &ThreadId,
    ids: &[UploadId],
) -> Result<Vec<TextAttachment>> {
    let mut attachments = vec![];
    for id in ids {
        let (name, content): (String, String) = tx
            .query_row(
                "DELETE FROM pending_upload WHERE id=? AND thread_id=? RETURNING name,content",
                params![id.0, thread.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::Invalid("upload is unavailable for this thread".into()))?;
        attachments.push(TextAttachment { name, content });
    }
    Ok(attachments)
}
