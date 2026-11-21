use super::*;
use crate::repository::{Binding, Driver, LinkedItem};

impl Database {
    pub fn bind_repository(
        &self,
        project: &ProjectId,
        driver: Driver,
        origin: String,
        repository: &str,
    ) -> Result<Binding> {
        self.project_root(project)?;
        let id=self.db.query_row(
            "INSERT INTO repository_binding VALUES(?,?,?,?,?)
             ON CONFLICT(project,driver,origin,repository) DO UPDATE SET repository=excluded.repository RETURNING id",
            params![new_id(),project.0,serde_json::to_string(&driver)?,origin,repository],|r|r.get(0))?;
        Ok(Binding {
            id,
            project: project.clone(),
            driver,
            origin,
            repository: repository.into(),
        })
    }
    pub fn repository_binding(&self, project: &ProjectId, id: &str) -> Result<Binding> {
        let (driver, origin, repository) = self
            .db
            .query_row(
                "SELECT driver,origin,repository FROM repository_binding WHERE id=? AND project=?",
                params![id, project.0],
                |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or(Error::Forbidden)?;
        Ok(Binding {
            id: id.into(),
            project: project.clone(),
            driver: serde_json::from_str(&driver)?,
            origin,
            repository,
        })
    }
    pub fn link_repository_item(
        &self,
        thread: &ThreadId,
        binding: &str,
        item: &str,
    ) -> Result<String> {
        let project = self
            .db
            .query_row("SELECT project FROM thread WHERE id=?", [&thread.0], |r| {
                Ok(ProjectId(r.get(0)?))
            })
            .optional()?
            .ok_or(Error::Forbidden)?;
        let repo = self.repository_binding(&project, binding)?;
        let number = match repo.driver {
            Driver::Linear => item
                .strip_prefix(&repo.repository)
                .and_then(|tail| tail.strip_prefix('-')),
            _ => Some(item),
        };
        if !number.is_some_and(|s| s.parse::<u64>().is_ok_and(|n| n > 0 && n.to_string() == s)) {
            return Err(Error::Invalid(
                "expected a repository item identifier".into(),
            ));
        }
        Ok(self.db.query_row(
            "INSERT INTO repository_link VALUES(?,?,?,?) ON CONFLICT(thread,binding,item)
             DO UPDATE SET item=excluded.item RETURNING id",
            params![new_id(), thread.0, binding, item],
            |r| r.get(0),
        )?)
    }
    pub fn repository_links(&self, thread: &ThreadId) -> Result<Vec<LinkedItem>> {
        self.thread_position(thread)?;
        Ok(self
            .db
            .prepare("SELECT id,binding,item FROM repository_link WHERE thread=? ORDER BY rowid")?
            .query_map([&thread.0], |r| {
                Ok(LinkedItem {
                    link: r.get(0)?,
                    binding: r.get(1)?,
                    item: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn unlink_repository_item(&self, thread: &ThreadId, link: &str) -> Result<bool> {
        Ok(self.db.execute(
            "DELETE FROM repository_link WHERE thread=? AND id=?",
            params![thread.0, link],
        )? > 0)
    }
    pub fn watch_repository_item(&self, thread: &ThreadId, link: &str) -> Result<String> {
        self.db.query_row("INSERT INTO repository_watch(id,link) SELECT ?,id FROM repository_link WHERE id=? AND thread=?
          ON CONFLICT(link) DO UPDATE SET link=excluded.link RETURNING id",params![new_id(),link,thread.0],|r|r.get(0))
            .optional()?.ok_or(Error::Forbidden)
    }
    pub fn unwatch_repository_item(&self, thread: &ThreadId, watch: &str) -> Result<bool> {
        Ok(self.db.execute("DELETE FROM repository_watch WHERE id=? AND link IN (SELECT id FROM repository_link WHERE thread=?)",params![watch,thread.0])?>0)
    }
    pub fn sync_repository_watch(
        &mut self,
        watch: &str,
        revision: i64,
        state: &str,
    ) -> Result<(Option<RunId>, Option<RunJob>)> {
        let tx = self.db.transaction()?;
        let (prior,previous,thread,item,repo,model)=tx.query_row(
            "SELECT w.revision,w.state,l.thread,l.item,b.repository,t.model FROM repository_watch w
             JOIN repository_link l ON l.id=w.link JOIN repository_binding b ON b.id=l.binding JOIN thread t ON t.id=l.thread WHERE w.id=?",
            [watch],|r|Ok((r.get::<_,Option<i64>>(0)?,r.get::<_,Option<String>>(1)?,ThreadId(r.get(2)?),r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get(5)?)))
            .optional()?.ok_or(Error::Forbidden)?;
        if prior.is_some_and(|r| r >= revision) {
            return Ok((None, None));
        }
        tx.execute(
            "UPDATE repository_watch SET revision=?, state=? WHERE id=?",
            params![revision, state, watch],
        )?;
        let mut result = (None, None);
        if previous.is_some_and(|s| s != state) {
            let text = format!("Repository update for {repo} item {item}\n{state}");
            let (run, _, _, delivery) = start_message(&tx, &thread, &text, model, None, true)?;
            tx.execute(
                "UPDATE turn_effect SET durable_launch=1 WHERE run_id=?",
                [&run.0],
            )?;
            result = (
                Some(run.clone()),
                (delivery == PeerDelivery::Started).then_some((thread, run)),
            );
        }
        tx.commit()?;
        Ok(result)
    }
}
