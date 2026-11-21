use super::*;
use crate::imports::Snapshot;

impl Database {
    pub fn project_root(&self, project: &ProjectId) -> Result<PathBuf> {
        self.db
            .query_row("SELECT root FROM project WHERE id=?", [&project.0], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(PathBuf::from)
            .ok_or_else(|| Error::NotFound(project.to_string()))
    }

    pub fn import_source(&self, thread: &ThreadId) -> Result<Option<ImportProvenance>> {
        self.thread_position(thread)?;
        self.db
            .query_row(
                "SELECT provenance FROM imported_history WHERE thread_id=?",
                [&thread.0],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(Into::into)
    }

    pub fn import(
        &mut self,
        project: &ProjectId,
        thread: &ThreadId,
        name: &str,
        instance: &ProviderInstanceId,
        snapshot: Snapshot,
    ) -> Result<ImportProvenance> {
        if thread.0.trim().is_empty() || name.trim().is_empty() || snapshot.messages.is_empty() {
            return Err(Error::Invalid(
                "import requires a thread ID, name and visible messages".into(),
            ));
        }
        let tx = self.db.transaction()?;
        let provenance = serde_json::to_string(&snapshot.provenance)?;
        if let Some(matches) = tx.query_row("SELECT coalesce(i.provenance=? AND t.project=? AND t.provider_instance=? AND t.name=?,0)
            FROM thread t LEFT JOIN imported_history i ON i.thread_id=t.id WHERE t.id=?",
            params![provenance,project.0,instance.0,name,thread.0], |r|r.get::<_,bool>(0)).optional()? {
            if matches {
                return Ok(snapshot.provenance);
            }
            return Err(Error::Invalid("import destination already exists with different history or settings".into()));
        }
        let driver = creation::instance_driver(&tx, instance.as_str())?;
        let native = match &snapshot.provenance.source {
            ImportSource::AcpRegistry(id) => Some(&id.0),
            _ => None,
        };
        if let Some(native) = native
            && tx.prepare("SELECT 1 FROM thread WHERE native_thread=? AND continuation_key=(SELECT scope FROM configured_provider WHERE id=?)")?
                .exists(params![native, instance.as_str()])?
        {
            return Err(Error::Invalid("native session already belongs to a thread".into()));
        }
        tx.execute("INSERT INTO thread(id,project,name,provider,provider_instance,native_thread,continuation_key)
            SELECT ?1,?2,?3,?4,id,?5,CASE WHEN ?5 IS NOT NULL THEN scope END FROM configured_provider WHERE id=?6",
            params![thread.0,project.0,name,driver,native,instance.0])?;
        event(&tx, thread, Change::Launched)?;
        if let Some(id) = native {
            event(
                &tx,
                thread,
                Change::Bound {
                    native_thread: NativeThreadId(id.clone()),
                },
            )?;
        }
        let mut run = None;
        let mut ordinal = 0;
        for (role, text) in snapshot.messages {
            if run.is_none() || role == Role::User {
                ordinal += 1;
                let id = RunId(new_id());
                tx.execute(
                    "INSERT INTO run(id,thread_id,ordinal,status) VALUES(?,?,?,'completed')",
                    params![id.0, thread.0, ordinal],
                )?;
                run_event(&tx, thread, &id)?;
                run = Some(id);
            }
            message(&tx, thread, run.as_ref().unwrap(), role, None, text)?;
        }
        tx.execute(
            "INSERT INTO imported_history VALUES(?,?,?)",
            params![thread.0, provenance, run.unwrap().0],
        )?;
        tx.commit()?;
        Ok(snapshot.provenance)
    }
}

/// A file import copies the most recent visible context on the first send.
pub(super) fn prepare(tx: &Transaction, run: &RunId, turn: &mut Turn) -> Result<()> {
    if turn.resume.is_some() {
        return Ok(());
    }
    if let Some((thread, through)) = tx
        .query_row(
            "SELECT i.thread_id,i.through_run FROM imported_history i
        JOIN run r ON r.thread_id=i.thread_id WHERE r.id=?",
            [&run.0],
            |r| Ok((ThreadId(r.get(0)?), RunId(r.get(1)?))),
        )
        .optional()?
    {
        let history = context::capture(tx, &thread, &through, ForkMode::Portable, false)?;
        turn.text = turn.output.text.history(
            &turn.output.title,
            &serde_json::to_string(&history)?,
            &turn.text,
        );
    }
    Ok(())
}
