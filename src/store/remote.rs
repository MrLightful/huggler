use super::*;
use crate::remote::{Access, Address, Grant, Reply, Request, Write};
use sha2::{Digest, Sha256};

pub(super) fn digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

struct Authority {
    id: String,
    sender: Address,
    operate: bool,
}
impl Database {
    fn environment(&self) -> Result<EnvironmentId> {
        Ok(EnvironmentId(self.db.query_row(
            "SELECT id FROM environment",
            [],
            |r| r.get(0),
        )?))
    }
    pub fn address(&self, thread: &ThreadId) -> Result<Address> {
        self.thread_position(thread)?;
        Ok(Address {
            environment: self.environment()?,
            thread: thread.clone(),
        })
    }
    pub fn approve_link(
        &self,
        thread: &ThreadId,
        sender: &Address,
        access: Access,
    ) -> Result<Grant> {
        let address = self.address(thread)?;
        if sender.thread.0.trim().is_empty()
            || sender.environment.0.is_empty()
            || sender.environment == address.environment
        {
            return Err(Error::Invalid(
                "expected a sender in another environment".into(),
            ));
        }
        let id = new_id();
        let bearer = new_id();
        self.db.execute(
            "INSERT INTO remote_grant VALUES(?,?,?,?,?,?)",
            params![
                id,
                digest(&bearer),
                thread.0,
                sender.environment.0,
                sender.thread.0,
                access == Access::Operate
            ],
        )?;
        Ok(Grant {
            id,
            address,
            bearer,
        })
    }
    pub fn revoke_link(&self, id: &str) -> Result<bool> {
        Ok(self
            .db
            .execute("DELETE FROM remote_grant WHERE id=?", [id])?
            > 0)
    }
    fn authority(&self, token: &str, target: &Address, child_read: bool) -> Result<Authority> {
        if target.environment != self.environment()? {
            return Err(Error::Forbidden);
        }
        self.db.query_row("SELECT id,sender_env,sender_thread,operate FROM remote_grant g WHERE digest=?1 AND
            (thread=?2 OR (?3 AND EXISTS (SELECT 1 FROM remote_delivery d JOIN run r ON r.id=d.run_id WHERE d.grant_id=g.id AND r.thread_id=?2)))",
            params![digest(token),target.thread.0,child_read],|r| Ok(Authority{id:r.get(0)?,sender:Address{environment:EnvironmentId(r.get(1)?),thread:ThreadId(r.get(2)?)},operate:r.get(3)?}))
            .optional()?.ok_or(Error::Forbidden)
    }
    pub fn permits_agent_tools(&self, run: &RunId) -> Result<bool> {
        Ok(!self
            .db
            .prepare("SELECT 1 FROM remote_delivery WHERE run_id=?")?
            .exists([&run.0])?)
    }
    pub fn remote_read(&self, token: &str, request: Request) -> Result<Reply> {
        match request {
            Request::Read { target, before } => {
                self.authority(token, &target, true)?;
                Ok(Reply::History {
                    history: self.history(&target.thread, before.as_ref(), "", 50)?,
                    address: target,
                })
            }
            Request::Result { target, run } => {
                self.authority(token, &target, true)?;
                let result = self.find_run(&target.thread, &run)?;
                let text = self.db.query_row("SELECT text FROM message WHERE run_id=? AND role='assistant' ORDER BY rowid DESC LIMIT 1",[&run.0],|r| r.get(0)).optional()?;
                Ok(Reply::Result {
                    status: result.status,
                    text,
                })
            }
            Request::Write { .. } => Err(Error::Forbidden),
        }
    }
    pub fn remote_write(
        &mut self,
        token: &str,
        target: &Address,
        sender_run: &RunId,
        key: &str,
        action: &Write,
    ) -> Result<(Reply, Option<RunJob>)> {
        let auth = self.authority(token, target, false)?;
        if !auth.operate {
            return Err(Error::Forbidden);
        }
        if key.trim().is_empty() || sender_run.0.is_empty() {
            return Err(Error::Invalid("missing sender run or request key".into()));
        }
        let intent = serde_json::to_string(&(sender_run, action))?;
        let tx = self.db.transaction()?;
        if let Some((prior, receipt)) = tx
            .query_row(
                "SELECT intent,receipt FROM remote_delivery WHERE grant_id=? AND sender_run=? AND request_key=?",
                params![auth.id, sender_run.0, key],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if prior != intent {
                return Err(Error::Conflict {
                    command: CommandId(key.into()),
                    original_thread: target.thread.clone(),
                    requested_thread: target.thread.clone(),
                });
            }
            return Ok((serde_json::from_str(&receipt)?, None));
        }
        let (thread, text) = match action {
            Write::Send { text } => (target.thread.clone(), text),
            Write::Delegate { worktree: true, .. } => {
                return Err(Error::Unsupported("remote worktree isolation".into()));
            }
            Write::Delegate { task, .. } => (
                creation::create_agent_thread(&tx, &target.thread, "Remote task", None)?,
                task,
            ),
        };
        let origin = MessageOrigin {
            environment_id: Some(auth.sender.environment),
            thread_id: auth.sender.thread,
            run_id: sender_run.clone(),
            request_key: key.into(),
        };
        let model = tx.query_row("SELECT model FROM thread WHERE id=?", [&thread.0], |r| {
            r.get(0)
        })?;
        let (run_id, message_id, _, delivery) =
            start_message(&tx, &thread, text, model, Some(origin), true)?;
        let job = (delivery == PeerDelivery::Started).then(|| (thread.clone(), run_id.clone()));
        let reply = Reply::Accepted {
            address: Address {
                environment: target.environment.clone(),
                thread: thread.clone(),
            },
            receipt: PeerReceipt {
                thread_id: thread,
                run_id: run_id.clone(),
                message_id,
                delivery,
                task_id: None,
            },
        };
        tx.execute(
            "INSERT INTO remote_delivery VALUES(?,?,?,?,?,?)",
            params![
                auth.id,
                sender_run.0,
                key,
                intent,
                serde_json::to_string(&reply)?,
                run_id.0
            ],
        )?;
        tx.commit()?;
        Ok((reply, job))
    }
    pub fn reserve_remote_task(
        &self,
        parent: &ThreadId,
        run: &RunId,
        key: &str,
        target: &Address,
        task: &str,
    ) -> Result<()> {
        self.find_run(parent, run)?;
        if key.trim().is_empty() || task.trim().is_empty() {
            return Err(Error::Invalid("empty delegation key or task".into()));
        }
        let destination = serde_json::to_string(target)?;
        let inserted=self.db.execute("INSERT INTO remote_task(parent_run,request_key,destination,task) VALUES(?,?,?,?) ON CONFLICT DO NOTHING",params![run.0,key,destination,task])?;
        if inserted==0 && !self.db.prepare("SELECT 1 FROM remote_task WHERE parent_run=? AND request_key=? AND destination=? AND task=?")?.exists(params![run.0,key,destination,task])? {
            return Err(Error::Conflict{command:CommandId(key.into()),original_thread:parent.clone(),requested_thread:parent.clone()})
        }
        Ok(())
    }
    pub fn record_remote_child(
        &self,
        run: &RunId,
        key: &str,
        child: &Address,
        child_run: &RunId,
    ) -> Result<()> {
        let count=self.db.execute("UPDATE remote_task SET child=?1,child_run=?2 WHERE parent_run=?3 AND request_key=?4 AND (child IS NULL OR (child=?1 AND child_run=?2))",params![serde_json::to_string(child)?,child_run.0,run.0,key])?;
        if count == 0 {
            return Err(Error::Invalid("remote delegation identity changed".into()));
        }
        Ok(())
    }
    pub fn remote_child(
        &self,
        parent: &ThreadId,
        run: &RunId,
        key: &str,
    ) -> Result<(Address, RunId, Option<RunId>)> {
        self.find_run(parent, run)?;
        let (child, child_run, wake)=self.db.query_row("SELECT child,child_run,wake FROM remote_task WHERE parent_run=? AND request_key=? AND child IS NOT NULL",params![run.0,key],|r|Ok((r.get::<_,String>(0)?,RunId(r.get(1)?),r.get::<_,Option<String>>(2)?.map(RunId)))).optional()?.ok_or_else(||Error::NotFound("remote child receipt".into()))?;
        Ok((serde_json::from_str(&child)?, child_run, wake))
    }
    pub fn finish_remote_task(
        &mut self,
        parent: &ThreadId,
        run: &RunId,
        key: &str,
        child: &Address,
        child_run: &RunId,
        status: RunStatus,
        text: Option<String>,
    ) -> Result<(Option<RunId>, Option<RunJob>)> {
        let (_, _, wake) = self.remote_child(parent, run, key)?;
        if wake.is_some() {
            return Ok((wake, None));
        }
        let tx = self.db.transaction()?;
        let origin = MessageOrigin {
            environment_id: Some(child.environment.clone()),
            thread_id: child.thread.clone(),
            run_id: child_run.clone(),
            request_key: key.into(),
        };
        let model = tx.query_row("SELECT model FROM thread WHERE id=?", [&parent.0], |r| {
            r.get(0)
        })?;
        let result = format!("Remote task {key} {status:?}\n{}", text.unwrap_or_default());
        let (wake, _, _, delivery) =
            start_message(&tx, parent, &result, model, Some(origin), true)?;
        tx.execute(
            "UPDATE remote_task SET wake=? WHERE parent_run=? AND request_key=?",
            params![wake.0, run.0, key],
        )?;
        tx.commit()?;
        Ok((
            Some(wake.clone()),
            (delivery == PeerDelivery::Started).then(|| (parent.clone(), wake)),
        ))
    }
}
