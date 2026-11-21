use super::*;
use crate::providers::CommandFrame;

const MAX_BYTES: usize = 32 * 1024;

fn bounded(text: &str) -> (&str, bool) {
    let start = text.ceil_char_boundary(text.len().saturating_sub(MAX_BYTES));
    (&text[start..], start > 0)
}

pub(super) fn record(
    tx: &Transaction,
    thread: &ThreadId,
    run: &RunId,
    item: &str,
    frame: CommandFrame,
) -> Result<()> {
    if item.is_empty() || item.len() > 512 {
        return Ok(());
    }
    let previous = tx
        .query_row(
            "SELECT text,truncated,finished FROM command_output WHERE run_id=? AND item=?",
            params![run.0, item],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()?;
    let discovered = previous.is_none();
    let (mut text, mut truncated, finished) = previous.unwrap_or_default();
    if let CommandFrame::Snapshot {
        activity: Some(item),
        ..
    } = &frame
    {
        crate::store::activity::record(tx, thread, run, item.clone())?;
    }
    if finished {
        return Ok(());
    }
    let finished = match frame {
        CommandFrame::Append(delta) => {
            let (tail, cut) = bounded(&delta);
            if cut {
                text.clear();
            }
            text.push_str(tail);
            truncated |= cut;
            false
        }
        CommandFrame::Snapshot {
            output, finished, ..
        } => {
            if let Some(output) = output {
                text = output;
                truncated = false;
            }
            finished
        }
    };
    let (tail, cut) = bounded(&text);
    tx.execute(
        "INSERT INTO command_output(run_id,item,revision,text,truncated,finished) VALUES(?,?,1,?,?,?)
         ON CONFLICT(run_id,item) DO UPDATE SET revision=revision+1,text=excluded.text,
         truncated=excluded.truncated,finished=excluded.finished",
        params![run.0,item,tail,truncated || cut,finished],
    )?;
    if discovered {
        event(
            tx,
            thread,
            Change::CommandOutputAvailable {
                run_id: run.clone(),
                item: item.into(),
            },
        )?;
    }
    Ok(())
}

impl Database {
    pub fn command_items(&self, thread: &ThreadId, run: &RunId) -> Result<Vec<String>> {
        self.find_run(thread, run)?;
        Ok(self
            .db
            .prepare("SELECT item FROM command_output WHERE run_id=? ORDER BY rowid")?
            .query_map([&run.0], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn command_output(
        &self,
        thread: &ThreadId,
        run: &RunId,
        item: &str,
    ) -> Result<CommandOutput> {
        self.db.query_row(
            "SELECT c.revision,c.text,c.truncated,c.finished OR r.status IN ('completed','interrupted','failed')
             FROM command_output c JOIN run r ON r.id=c.run_id WHERE r.thread_id=? AND r.id=? AND c.item=?",
            params![thread.0,run.0,item],
            |r| Ok(CommandOutput { revision:r.get(0)?,text:r.get(1)?,truncated:r.get(2)?, closed:r.get(3)? }),
        ).optional()?.ok_or_else(|| Error::NotFound("command output".into()))
    }
}
