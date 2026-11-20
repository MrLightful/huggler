mod native_children;
mod tasks;
use crate::{
    contracts::*,
    providers::{Turn, Update},
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use std::{
    fs::{self, File},
    path::Path,
};

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

const RUN_FIELDS: &str = "id, ordinal, status, native_turn, error";

fn run_row(row: &Row<'_>) -> rusqlite::Result<Run> {
    let status: String = row.get(2)?;
    let status = match status.as_str() {
        "queued" => RunStatus::Queued,
        "starting" => RunStatus::Starting,
        "running" => RunStatus::Running,
        "waiting" => RunStatus::Waiting,
        "completed" => RunStatus::Completed,
        "failed" => RunStatus::Failed,
        "interrupted" => RunStatus::Interrupted,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(Run {
        id: RunId(row.get(0)?),
        ordinal: row.get(1)?,
        status,
        native_turn: row.get::<_, Option<String>>(3)?.map(NativeTurnId),
        error: row.get(4)?,
    })
}

fn run(db: &Connection, id: &RunId) -> Result<Run> {
    Ok(db.query_row(
        &format!("SELECT {RUN_FIELDS} FROM run WHERE id=?"),
        [&id.0],
        run_row,
    )?)
}

fn event(tx: &Transaction<'_>, thread: &ThreadId, change: Change) -> Result<i64> {
    tx.execute(
        "INSERT INTO event(thread_id,change) VALUES(?,?)",
        params![thread.0, serde_json::to_string(&change)?],
    )?;
    Ok(tx.last_insert_rowid())
}
fn run_event(tx: &Transaction<'_>, thread: &ThreadId, id: &RunId) -> Result<i64> {
    event(tx, thread, Change::Run { run: run(tx, id)? })
}
fn message(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    run_id: &RunId,
    role: Role,
    origin: Option<MessageOrigin>,
    text: String,
) -> Result<(MessageId, i64)> {
    let id = MessageId(new_id());
    let stored_role = match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    tx.execute(
        "INSERT INTO message(id,run_id,role,text,origin) VALUES(?,?,?,?,?)",
        params![
            id.0,
            run_id.0,
            stored_role,
            text,
            origin.as_ref().map(serde_json::to_string).transpose()?
        ],
    )?;
    let seq = event(
        tx,
        thread,
        Change::Message {
            message: Message {
                id: id.clone(),
                run_id: run_id.clone(),
                role,
                origin,
                text,
            },
        },
    )?;
    Ok((id, seq))
}

fn start_message(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    text: &str,
    model: Option<String>,
    origin: Option<MessageOrigin>,
) -> Result<(RunId, MessageId, Vec<i64>, PeerDelivery)> {
    if text.trim().is_empty() {
        return Err(Error::Invalid("empty message".into()));
    }
    let busy: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM run WHERE thread_id=? AND status IN ('starting','running','waiting','queued'))",
        [&thread.0], |r| r.get(0),
    )?;
    if busy && origin.is_none() {
        return Err(Error::Busy);
    }
    let (status, delivery) = if busy {
        ("queued", PeerDelivery::Queued)
    } else {
        ("starting", PeerDelivery::Started)
    };
    let id = RunId(new_id());
    tx.execute("INSERT INTO run(id,thread_id,status,ordinal) SELECT ?,?,?,COALESCE(MAX(ordinal),0)+1 FROM run WHERE thread_id=?",
        params![id.0,thread.0,status,thread.0])?;
    let started = run_event(tx, thread, &id)?;
    let (msg, seq) = message(tx, thread, &id, Role::User, origin, text.to_owned())?;
    tx.execute(
        "INSERT INTO turn_effect(run_id,model,state) VALUES(?,?,'pending')",
        params![id.0, model],
    )?;
    Ok((id, msg, vec![started, seq], delivery))
}

pub(crate) struct Database {
    db: Connection,
    _lock: File,
}

impl Database {
    pub fn open(dir: &Path) -> Result<Self> {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.recursive(true).create(dir)?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("runtime.lock"))?;
        lock.try_lock().map_err(|err| match err {
            std::fs::TryLockError::WouldBlock => Error::AlreadyOpen,
            std::fs::TryLockError::Error(e) => Error::Io(e),
        })?;
        let mut db = Connection::open(dir.join("huggler.sqlite3"))?;
        db.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA temp_store=MEMORY;",
        )?;
        let tx = db.transaction()?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let migrations = [
            include_str!("../migrations/001.sql"),
            include_str!("../migrations/002.sql"),
            include_str!("../migrations/003.sql"),
            include_str!("../migrations/004.sql"),
            include_str!("../migrations/005.sql"),
            include_str!("../migrations/006.sql"),
        ];
        if !(0..=migrations.len() as i64).contains(&version) {
            return Err(Error::SchemaVersion(version));
        }
        for migration in &migrations[version as usize..] {
            tx.execute_batch(migration)?;
        }
        tx.execute_batch(
            "CREATE TEMP TABLE agent_grant(token TEXT PRIMARY KEY, run_id TEXT UNIQUE NOT NULL)",
        )?;
        // Both pending and claimed starts belong to the old process. A send might
        // have reached Codex before its reply was persisted. Never issue it twice.
        // A task launch or wake still pending at this point never reached a provider.
        // Put it back in order; claimed provider effects keep the recovery policy below.
        tx.execute("UPDATE run SET status='queued' WHERE status='starting' AND stop_requested=0
            AND id IN (SELECT run_id FROM turn_effect WHERE state='pending')
            AND (id IN (SELECT child_run FROM delegated_task) OR id IN (SELECT wake_run FROM delegated_task))", [])?;
        let lost = tx
            .prepare(
                "SELECT r.id, r.thread_id FROM turn_effect e
                      JOIN run r ON r.id=e.run_id WHERE r.status != 'queued' ORDER BY r.rowid",
            )?
            .query_map([], |row| Ok((RunId(row.get(0)?), ThreadId(row.get(1)?))))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, thread) in lost {
            Self::finish_tx(
                &tx,
                &thread,
                &id,
                Err(Error::Provider("runtime exited during turn".into())),
            )?;
        }
        native_children::disconnect(&tx, None)?;
        tx.commit()?;
        Ok(Self { db, _lock: lock })
    }

    pub fn project(&self, root: &Path) -> Result<Project> {
        let path = root
            .to_str()
            .ok_or_else(|| Error::Invalid("workspace path requires UTF-8".into()))?;
        self.db.execute(
            "INSERT INTO project(id,root) VALUES(?,?) ON CONFLICT(root) DO NOTHING",
            params![new_id(), path],
        )?;
        let id = self
            .db
            .query_row("SELECT id FROM project WHERE root=?", [path], |r| r.get(0))?;
        Ok(Project {
            id: ProjectId(id),
            root: root.into(),
        })
    }

    pub fn dispatch(&mut self, cmd: &Command) -> Result<(Receipt, bool)> {
        let tx = self.db.transaction()?;
        let action = serde_json::to_string(&cmd.action)?;
        let prior = tx
            .query_row(
                "SELECT action,result FROM receipt WHERE command_id=?",
                [&cmd.id.0],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((original, result)) = prior {
            let receipt: Receipt = serde_json::from_str(&result)?;
            if receipt.thread_id != cmd.thread_id || action != original {
                return Err(Error::Conflict {
                    command: cmd.id.clone(),
                    original_thread: receipt.thread_id,
                    requested_thread: cmd.thread_id.clone(),
                });
            }
            return Ok((receipt, false));
        }
        let thread = &cmd.thread_id;
        if thread.0.trim().is_empty() || cmd.id.0.trim().is_empty() {
            return Err(Error::Invalid("empty thread or command ID".into()));
        }
        let mut receipt = Receipt {
            command_id: cmd.id.clone(),
            thread_id: thread.clone(),
            message_id: None,
            run_id: None,
            event_sequences: vec![],
        };
        let selection = tx
            .query_row("SELECT model FROM thread WHERE id=?", [&thread.0], |r| {
                r.get::<_, Option<String>>(0)
            })
            .optional()?;
        if selection.is_none()
            && !matches!(
                cmd.action,
                Action::Launch { .. } | Action::LaunchProvider { .. }
            )
        {
            return Err(Error::NotFound(thread.to_string()));
        }
        match &cmd.action {
            Action::Launch {
                project,
                name,
                model,
            }
            | Action::LaunchProvider {
                project,
                name,
                model,
                ..
            } => {
                let provider = match cmd.action {
                    Action::LaunchProvider { provider, .. } => provider,
                    _ => Provider::Codex,
                };
                if name.trim().is_empty() {
                    return Err(Error::Invalid("empty thread name".into()));
                }
                if selection.is_some() {
                    return Err(Error::Invalid("thread already exists".into()));
                }
                let n = tx.execute("INSERT INTO thread(id,name,model,provider,project) SELECT ?,?,?,?,id FROM project WHERE id=?",
                    params![thread.0,name,model,provider.key(),project.0])?;
                if n == 0 {
                    return Err(Error::NotFound(project.to_string()));
                }
                receipt
                    .event_sequences
                    .push(event(&tx, thread, Change::Launched)?);
            }
            Action::SetModel { model } => {
                if selection.as_ref() != Some(model) {
                    tx.execute(
                        "UPDATE thread SET model=? WHERE id=?",
                        params![model, thread.0],
                    )?;
                    receipt.event_sequences.push(event(
                        &tx,
                        thread,
                        Change::ModelSelected {
                            model: model.clone(),
                        },
                    )?);
                }
            }
            Action::Send { text } => {
                let (id, msg, events, _) =
                    start_message(&tx, thread, text, selection.flatten(), None)?;
                receipt.run_id = Some(id);
                receipt.message_id = Some(msg);
                receipt.event_sequences = events;
            }
        }
        tx.execute(
            "INSERT INTO receipt(command_id,action,result) VALUES(?,?,?)",
            params![cmd.id.0, action, serde_json::to_string(&receipt)?],
        )?;
        tx.commit()?;
        Ok((receipt, true))
    }

    pub fn claim(&mut self, id: &RunId) -> Result<Turn> {
        let tx = self.db.transaction()?;
        let turn = tx.query_row(
            "SELECT p.root,t.name,m.text,e.model,t.native_thread,t.provider,m.origin FROM run r
             JOIN turn_effect e ON e.run_id=r.id
             JOIN thread t ON t.id=r.thread_id JOIN project p ON p.id=t.project
             JOIN message m ON m.run_id=r.id WHERE r.id=? AND m.role='user'",
            [&id.0],
            |r| {
                Ok(Turn {
                    provider: Provider::from_key(&r.get::<_, String>(5)?)?,
                    agent_tools: None,
                    cwd: r.get::<_, String>(0)?.into(),
                    name: r.get(1)?,
                    text: match r.get::<_, Option<String>>(6)? {
                        Some(origin) => format!(
                            "Peer message from Huggler {origin}\n\n{}",
                            r.get::<_, String>(2)?
                        ),
                        None => r.get(2)?,
                    },
                    model: r.get(3)?,
                    resume: r.get::<_, Option<String>>(4)?.map(NativeThreadId),
                })
            },
        )?;
        if tx.execute(
            "UPDATE turn_effect SET state='claimed' WHERE run_id=? AND state='pending'",
            [&id.0],
        )? != 1
        {
            return Err(Error::Invalid("turn already claimed".into()));
        }
        tx.commit()?;
        Ok(turn)
    }

    pub fn update(&mut self, thread: &ThreadId, id: &RunId, update: Update) -> Result<()> {
        let tx = self.db.transaction()?;
        match update {
            Update::NativeChild(child) => native_children::record(&tx, thread, child)?,
            Update::ForegroundComplete => {
                Self::finish_tx(&tx, thread, id, Ok(()))?;
                tx.execute("DELETE FROM agent_grant WHERE run_id=?", [&id.0])?;
            }
            Update::Bound(native_thread) => {
                let changed = tx.execute(
                    "UPDATE thread SET native_thread=? WHERE id=? AND native_thread IS NULL",
                    params![native_thread.0, thread.0],
                )?;
                if changed == 1 {
                    event(&tx, thread, Change::Bound { native_thread })?;
                }
            }
            Update::Started(turn) => {
                tx.execute(
                    "UPDATE run SET status='running',native_turn=? WHERE id=?",
                    params![turn.map(|t| t.0), id.0],
                )?;
                run_event(&tx, thread, id)?;
            }
            Update::Finalizing(text) => {
                if tx.execute(
                    "UPDATE run SET status='waiting' WHERE id=? AND stop_requested=0",
                    [&id.0],
                )? == 0
                {
                    return Err(Error::Interrupted);
                }
                message(&tx, thread, id, Role::Assistant, None, text)?;
                run_event(&tx, thread, id)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn request_permission(
        &mut self,
        thread: &ThreadId,
        request: PermissionRequest,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        if tx.execute(
            "INSERT INTO permission(id,run_id,request)
             SELECT ?1,id,?3 FROM run WHERE id=?2 AND stop_requested=0",
            params![
                request.id.0,
                request.run_id.0,
                serde_json::to_string(&request)?
            ],
        )? == 0
        {
            return Err(Error::Interrupted);
        }
        event(&tx, thread, Change::PermissionRequested { request })?;
        Ok(tx.commit()?)
    }

    pub fn decide(
        &mut self,
        thread: &ThreadId,
        run: &RunId,
        id: &RequestId,
        option: &str,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        let request: String = tx.query_row("SELECT p.request FROM permission p JOIN run r ON r.id=p.run_id WHERE p.id=? AND r.id=? AND r.thread_id=? AND p.expired=0 AND p.decision IS NULL",params![id.0,run.0,thread.0],|r|r.get(0)).optional()?.ok_or_else(|| Error::NotFound("pending permission".into()))?;
        let request: PermissionRequest = serde_json::from_str(&request)?;
        if !request.options.iter().any(|o| o.option_id == option) {
            return Err(Error::Invalid("unknown permission option".into()));
        }
        tx.execute(
            "UPDATE permission SET decision=? WHERE id=?",
            params![option, id.0],
        )?;
        event(
            &tx,
            thread,
            Change::PermissionClosed {
                id: id.clone(),
                option: Some(option.into()),
            },
        )?;
        Ok(tx.commit()?)
    }

    fn finish_tx(
        tx: &Transaction<'_>,
        thread: &ThreadId,
        id: &RunId,
        result: Result<()>,
    ) -> Result<()> {
        // Waiting and the final answer commit together. A cleanup error or a
        // process exit after this point cannot undo provider completion.
        let pending = tx
            .prepare("SELECT id FROM permission WHERE run_id=? AND expired=0 AND decision IS NULL")?
            .query_map([&id.0], |r| Ok(RequestId(r.get(0)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in pending {
            tx.execute("UPDATE permission SET expired=1 WHERE id=?", [&id.0])?;
            event(tx, thread, Change::PermissionClosed { id, option: None })?;
        }
        let interrupted = matches!(result, Err(Error::Interrupted));
        let error = result.err().map(|e| e.to_string());
        tx.execute(
            "UPDATE run SET status=CASE WHEN status='waiting' THEN 'completed' WHEN stop_requested OR ?3 THEN 'interrupted' WHEN ?1 IS NULL THEN 'completed' ELSE 'failed' END,
                 error=CASE WHEN status='waiting' THEN NULL WHEN stop_requested THEN 'task cancelled' ELSE ?1 END WHERE id=?2",
            params![error, id.0,interrupted],
        )?;
        run_event(tx, thread, id)?;
        tx.execute("DELETE FROM turn_effect WHERE run_id=?", [&id.0])?;
        Ok(())
    }

    pub fn finish(&mut self, thread: &ThreadId, id: &RunId, result: Result<()>) -> Result<()> {
        let tx = self.db.transaction()?;
        native_children::disconnect(&tx, Some(id))?;
        // A foreground turn may have already completed while its native children continued.
        let unfinished: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM turn_effect WHERE run_id=?)",
            [&id.0],
            |r| r.get(0),
        )?;
        if unfinished {
            Self::finish_tx(&tx, thread, id, result)?;
        }
        tx.execute("DELETE FROM agent_grant WHERE run_id=?", [&id.0])?;
        Ok(tx.commit()?)
    }

    /// Promote the oldest queued message when the previous run has settled.
    fn next_queued(tx: &Transaction, thread: &ThreadId) -> Result<Option<RunId>> {
        let id = tx.query_row(
            "UPDATE run SET status='starting' WHERE id=(SELECT id FROM run WHERE thread_id=?1 AND status='queued' ORDER BY ordinal LIMIT 1)
             AND NOT EXISTS(SELECT 1 FROM run WHERE thread_id=?1 AND status IN ('starting','running','waiting')) RETURNING id",
            [&thread.0], |r| Ok(RunId(r.get(0)?)),
        ).optional()?;
        if let Some(id) = &id {
            run_event(tx, thread, id)?;
        }
        Ok(id)
    }

    /// Reserve runnable continuations under the same transaction as their delivery receipts.
    pub fn ready_jobs(&mut self) -> Result<Vec<(ThreadId, RunId)>> {
        let tx = self.db.transaction()?;
        let threads = tx
            .prepare("SELECT DISTINCT thread_id FROM run WHERE status='queued'")?
            .query_map([], |r| Ok(ThreadId(r.get(0)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut jobs = Vec::new();
        for thread in threads {
            if let Some(run) = Self::next_queued(&tx, &thread)? {
                jobs.push((thread, run));
            }
        }
        tasks::reconcile(&tx, &mut jobs)?;
        tx.commit()?;
        Ok(jobs)
    }

    pub fn grant(&self, run: &RunId) -> Result<String> {
        let token = new_id();
        self.db.execute(
            "INSERT INTO agent_grant(token,run_id) VALUES(?,?)",
            params![token, run.0],
        )?;
        Ok(token)
    }

    pub fn revoke(&self, thread: &ThreadId, run: &RunId) -> Result<bool> {
        Ok(self.db.execute(
            "DELETE FROM agent_grant WHERE run_id=(SELECT id FROM run WHERE thread_id=? AND id=?)",
            params![thread.0, run.0],
        )? != 0)
    }

    pub fn caller(&self, token: &str) -> Result<(ThreadId, RunId, ProjectId)> {
        self.db.query_row("SELECT t.id,r.id,t.project FROM agent_grant g JOIN run r ON r.id=g.run_id JOIN thread t ON t.id=r.thread_id
            WHERE g.token=? AND r.stop_requested=0 AND r.status IN ('starting','running')", [token], |r| Ok((ThreadId(r.get(0)?),RunId(r.get(1)?),ProjectId(r.get(2)?))))
            .optional()?.ok_or(Error::Forbidden)
    }

    pub fn peers(&self, token: &str) -> Result<Vec<ThreadSummary>> {
        let (sender, _, project) = self.caller(token)?;
        Ok(self
            .db
            .prepare("SELECT id,name FROM thread WHERE project=? AND id!=? ORDER BY rowid")?
            .query_map(params![project.0, sender.0], |r| {
                Ok(ThreadSummary {
                    id: ThreadId(r.get(0)?),
                    name: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn peer_receipt(&self, sender: &ThreadId, key: &str) -> Result<Option<PeerReceipt>> {
        let json: Option<String> = self
            .db
            .query_row(
                "SELECT result FROM peer_receipt WHERE sender_thread=? AND request_key=?",
                params![sender.0, key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn send_peer(
        &mut self,
        token: &str,
        target: &ThreadId,
        key: &str,
        text: &str,
    ) -> Result<(PeerReceipt, bool)> {
        let (sender, run_id, project) = self.caller(token)?;
        if sender == *target {
            return Err(Error::Invalid("expected an independent peer".into()));
        }
        if key.trim().is_empty() {
            return Err(Error::Invalid("empty clientRequestId".into()));
        }
        let model: Option<String> = self
            .db
            .query_row(
                "SELECT model FROM thread WHERE id=? AND project=?",
                params![target.0, project.0],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::Forbidden)?;
        if let Some(receipt) = self.peer_receipt(&sender, key)? {
            let original: String = self.db.query_row(
                "SELECT text FROM message WHERE id=?",
                [&receipt.message_id.0],
                |r| r.get(0),
            )?;
            if receipt.thread_id != *target || original != text {
                return Err(Error::Conflict {
                    command: CommandId(key.into()),
                    original_thread: receipt.thread_id,
                    requested_thread: target.clone(),
                });
            }
            return Ok((receipt, false));
        }
        let tx = self.db.transaction()?;
        let origin = MessageOrigin {
            thread_id: sender.clone(),
            run_id,
            request_key: key.into(),
        };
        let (run_id, message_id, _, delivery) =
            start_message(&tx, target, text, model, Some(origin))?;
        let receipt = PeerReceipt {
            thread_id: target.clone(),
            run_id,
            message_id,
            delivery,
        };
        tx.execute(
            "INSERT INTO peer_receipt(sender_thread,request_key,message_id,result) VALUES(?,?,?,?)",
            params![
                sender.0,
                key,
                receipt.message_id.0,
                serde_json::to_string(&receipt)?
            ],
        )?;
        tx.commit()?;
        Ok((receipt, true))
    }

    pub fn threads(&self) -> Result<Vec<ThreadSummary>> {
        Ok(self
            .db
            .prepare("SELECT id,name FROM thread ORDER BY rowid")?
            .query_map([], |r| {
                Ok(ThreadSummary {
                    id: ThreadId(r.get(0)?),
                    name: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn read(&self, id: &ThreadId) -> Result<Thread> {
        let mut thread = self
            .db
            .query_row(
                "SELECT project,name,model,native_thread,provider FROM thread WHERE id=?",
                [&id.0],
                |r| {
                    Ok(Thread {
                        native_children: vec![],
                        tasks: vec![],
                        provider: Provider::from_key(&r.get::<_, String>(4)?)?,
                        permissions: vec![],
                        id: id.clone(),
                        project: ProjectId(r.get(0)?),
                        name: r.get(1)?,
                        model: r.get(2)?,
                        native_thread: r.get::<_, Option<String>>(3)?.map(NativeThreadId),
                        messages: vec![],
                        runs: vec![],
                        cursor: 0,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(id.to_string()))?;
        thread.tasks = tasks::for_parent(&self.db, id)?;
        thread.native_children = native_children::for_thread(&self.db, id)?;
        thread.runs = self
            .db
            .prepare(&format!(
                "SELECT {RUN_FIELDS} FROM run WHERE thread_id=? ORDER BY ordinal"
            ))?
            .query_map([&id.0], run_row)?
            .collect::<rusqlite::Result<_>>()?;
        thread.messages = self
            .db
            .prepare(
                "SELECT m.id,m.run_id,m.role,m.text,m.origin FROM message m JOIN run r ON m.run_id=r.id
            WHERE r.thread_id=? ORDER BY r.ordinal,m.rowid",
            )?
            .query_map([&id.0], |r| {
                Ok(Message {
                    id: MessageId(r.get(0)?),
                    run_id: RunId(r.get(1)?),
                    role: if r.get::<_, String>(2)? == "user" {
                        Role::User
                    } else {
                        Role::Assistant
                    },
                    text: r.get(3)?,
                    origin: r.get::<_,Option<String>>(4)?.map(|s| serde_json::from_str(&s).map_err(|e|rusqlite::Error::FromSqlConversionFailure(4,rusqlite::types::Type::Text,Box::new(e)))).transpose()?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        thread.permissions = self.db.prepare("SELECT request FROM permission p JOIN run r ON p.run_id=r.id WHERE r.thread_id=? AND p.expired=0 AND p.decision IS NULL ORDER BY p.rowid")?.query_map([&id.0], |r|r.get::<_,String>(0))?.map(|row|Ok(serde_json::from_str(&row?)?)).collect::<Result<_>>()?;
        thread.cursor = self.db.query_row(
            "SELECT coalesce(max(sequence),0) FROM event WHERE thread_id=?",
            [&id.0],
            |r| r.get(0),
        )?;
        Ok(thread)
    }

    pub fn find_run(&self, thread: &ThreadId, id: &RunId) -> Result<Run> {
        self.db
            .query_row(
                &format!("SELECT {RUN_FIELDS} FROM run WHERE thread_id=? AND id=?"),
                params![thread.0, id.0],
                run_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(id.to_string()))
    }

    pub fn events(&self, thread: &ThreadId, after: i64) -> Result<Vec<Event>> {
        let mut statement = self.db.prepare("SELECT sequence,change FROM event WHERE thread_id=? AND sequence>? ORDER BY sequence LIMIT 64")?;
        let mut rows = statement.query(params![thread.0, after])?;
        let mut events = vec![];
        while let Some(row) = rows.next()? {
            events.push(Event {
                sequence: row.get(0)?,
                thread_id: thread.clone(),
                change: serde_json::from_str(&row.get::<_, String>(1)?)?,
            });
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrades_v2_messages_and_events_without_attribution() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let conn = Connection::open(dir.path().join("huggler.sqlite3"))?;
        conn.execute_batch(include_str!("../migrations/001.sql"))?;
        conn.execute_batch(include_str!("../migrations/002.sql"))?;
        conn.execute_batch(r#"INSERT INTO project VALUES('p','/old');
            INSERT INTO thread(id,project,name) VALUES('t','p','legacy');
            INSERT INTO run(id,thread_id,ordinal,status) VALUES('r','t',1,'completed');
            INSERT INTO message VALUES('m','r','assistant','legacy answer');
            INSERT INTO event(thread_id,change) VALUES('t','{"type":"message","message":{"id":"m","run_id":"r","role":"assistant","text":"legacy answer"}}');"#)?;
        drop(conn);
        let thread = ThreadId("t".into());
        let db = Database::open(dir.path())?;
        let snapshot = db.read(&thread)?;
        assert_eq!(snapshot.messages[0].text, "legacy answer");
        assert_eq!(snapshot.messages[0].origin, None);
        assert!(
            matches!(&db.events(&thread,0)?[0].change, Change::Message {message} if message == &snapshot.messages[0])
        );
        Ok(())
    }

    #[test]
    fn recovery_finishes_a_persisted_answer_exactly_once() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut db = Database::open(dir.path())?;
        let project = db.project(dir.path())?;
        let thread = ThreadId("conversation".into());
        db.dispatch(&Command {
            id: CommandId("launch".into()),
            thread_id: thread.clone(),
            action: Action::Launch {
                project: project.id,
                name: "Recovery".into(),
                model: None,
            },
        })?;
        let send = Command {
            id: CommandId("send".into()),
            thread_id: thread.clone(),
            action: Action::Send {
                text: "question".into(),
            },
        };
        let (receipt, _) = db.dispatch(&send)?;
        let run = receipt.run_id.as_ref().unwrap();
        db.claim(run)?;
        db.update(
            &thread,
            run,
            Update::Started(Some(NativeTurnId("turn".into()))),
        )?;
        db.update(&thread, run, Update::Finalizing("answer".into()))?;
        let waiting_cursor = db.read(&thread)?.cursor;
        drop(db);

        for _ in 0..2 {
            let mut recovered = Database::open(dir.path())?;
            let snapshot = recovered.read(&thread)?;
            assert_eq!(snapshot.runs[0].status, RunStatus::Completed);
            assert_eq!(snapshot.runs[0].error, None);
            assert_eq!(snapshot.messages.len(), 2);
            assert_eq!(snapshot.messages[1].text, "answer");
            let events = recovered.events(&thread, waiting_cursor)?;
            assert!(
                matches!(events.as_slice(), [Event { change: Change::Run { run }, .. }] if run.status == RunStatus::Completed)
            );
            assert_eq!(recovered.dispatch(&send)?, (receipt.clone(), false));
        }
        Ok(())
    }
}
