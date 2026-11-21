use super::*;

pub(super) fn reserve(
    tx: &Transaction,
    thread: &ThreadId,
    path: &Path,
    common_dir: &str,
    commit: &str,
) -> Result<()> {
    let path = path
        .to_str()
        .ok_or_else(|| Error::Invalid("worktree path requires UTF-8".into()))?;
    tx.execute(
        "INSERT INTO worktree(path,common_dir,base_commit) VALUES(?,?,?)",
        params![path, common_dir, commit],
    )?;
    tx.execute(
        "UPDATE thread SET worktree_path=? WHERE id=?",
        params![path, thread.0],
    )?;
    Ok(())
}

pub(super) fn capture(tx: &Transaction, thread: &ThreadId, through: &RunId) -> Result<()> {
    let cwd: String = tx
        .query_row(
            "SELECT coalesce(t.worktree_path,p.root) FROM thread t JOIN project p ON p.id=t.project
         JOIN run r ON r.thread_id=t.id WHERE t.id=? AND r.id=? AND r.status='completed'
         AND NOT EXISTS(SELECT 1 FROM run later WHERE later.thread_id=t.id
             AND (later.status IN ('running','starting','waiting') OR
             coalesce(later.delivery_slot,later.ordinal)>coalesce(r.delivery_slot,r.ordinal)))",
            params![thread.0, through.0],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            Error::Invalid("capture requires the latest completed turn on an idle thread".into())
        })?;
    let (repository, commit) = crate::worktree::clean_base(Path::new(&cwd))?;
    if tx.execute(
        "INSERT INTO checkpoint VALUES(?,?,?) ON CONFLICT DO NOTHING",
        params![through.0, repository, commit],
    )? == 0
        && revision(tx, thread, through)? != (repository, commit)
    {
        return Err(Error::Invalid(
            "turn already has a different checkpoint".into(),
        ));
    }
    Ok(())
}

pub(super) fn revision(
    db: &Connection,
    thread: &ThreadId,
    run: &RunId,
) -> Result<(String, String)> {
    db.query_row(
        "SELECT c.common_dir,c.git_commit FROM checkpoint c JOIN run r ON r.id=c.run_id
        WHERE r.thread_id=? AND r.id=?",
        params![thread.0, run.0],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("checkpoint for run {run}")))
}

impl Database {
    pub fn checkpoint_pair(
        &self,
        thread: &ThreadId,
        from: &RunId,
        to: &RunId,
    ) -> Result<(String, String, String)> {
        let (repository, base) = revision(&self.db, thread, from)?;
        let (other, head) = revision(&self.db, thread, to)?;
        if repository != other {
            return Err(Error::Invalid(
                "checkpoints belong to different repositories".into(),
            ));
        }
        Ok((repository, base, head))
    }
}
