mod activity;
mod application;
#[cfg(feature = "application")]
mod automation;
mod checkpoints;
mod command_output;
mod context;
#[cfg(test)]
mod crash_tests;
mod creation;
mod guest;
mod history;
mod imports;
mod interrupts;
mod native_children;
mod questions;
mod queue;
mod remote;
#[cfg(feature = "application")]
mod repository;
mod steering;
mod tasks;
mod uploads;
mod usage_limits;
#[cfg(feature = "application")]
mod workspaces;
use crate::{
    contracts::*,
    providers::{Turn, Update},
    worktree::Worktree,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn validate_model_options(driver: &str, options: &ModelOptions) -> Result<()> {
    if !options.is_default() && driver != Provider::Codex.key() {
        return Err(Error::Invalid("model options require Codex".into()));
    }
    Ok(())
}

pub(crate) type RunJob = (ThreadId, RunId);

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DispatchWork {
    pub start: Vec<RunJob>,
    pub stop: Vec<RunId>,
    pub steer: Option<(RunId, String)>,
}

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001.sql"),
    include_str!("../migrations/002.sql"),
    include_str!("../migrations/003.sql"),
    include_str!("../migrations/004.sql"),
    include_str!("../migrations/005.sql"),
    include_str!("../migrations/006.sql"),
    include_str!("../migrations/007.sql"),
    include_str!("../migrations/008.sql"),
    include_str!("../migrations/009.sql"),
    include_str!("../migrations/010.sql"),
    include_str!("../migrations/011.sql"),
    include_str!("../migrations/012.sql"),
    include_str!("../migrations/013.sql"),
    include_str!("../migrations/014.sql"),
    include_str!("../migrations/015.sql"),
    include_str!("../migrations/016.sql"),
    include_str!("../migrations/017.sql"),
    include_str!("../migrations/018.sql"),
    include_str!("../migrations/019.sql"),
    include_str!("../migrations/020.sql"),
    include_str!("../migrations/021.sql"),
    include_str!("../migrations/022.sql"),
    include_str!("../migrations/023.sql"),
    include_str!("../migrations/024.sql"),
    include_str!("../migrations/025.sql"),
    include_str!("../migrations/026.sql"),
    include_str!("../migrations/027.sql"),
    include_str!("../migrations/028.sql"),
    include_str!("../migrations/029.sql"),
    include_str!("../migrations/030.sql"),
    include_str!("../migrations/031.sql"),
    include_str!("../migrations/032.sql"),
    include_str!("../migrations/033.sql"),
    include_str!("../migrations/034.sql"),
    include_str!("../migrations/035.sql"),
    include_str!("../migrations/036.sql"),
    include_str!("../migrations/037.sql"),
    include_str!("../migrations/038.sql"),
    include_str!("../migrations/039.sql"),
    include_str!("../migrations/040.sql"),
    include_str!("../migrations/041.sql"),
    include_str!("../migrations/042.sql"),
    include_str!("../migrations/043.sql"),
];

const RUN_FIELDS: &str = "id, ordinal, status, native_turn, error, queue_held, delivery_slot, usage_limit_reset_at, prompt_suggestion, started_at_ms";

fn run_status(status: &str) -> rusqlite::Result<RunStatus> {
    Ok(match status {
        "queued" => RunStatus::Queued,
        "starting" => RunStatus::Starting,
        "running" => RunStatus::Running,
        "waiting" => RunStatus::Waiting,
        "completed" => RunStatus::Completed,
        "failed" => RunStatus::Failed,
        "interrupted" => RunStatus::Interrupted,
        _ => return Err(rusqlite::Error::InvalidQuery),
    })
}

fn run_row(row: &Row<'_>) -> rusqlite::Result<Run> {
    Ok(Run {
        id: RunId(row.get(0)?),
        ordinal: row.get(1)?,
        status: run_status(&row.get::<_, String>(2)?)?,
        native_turn: row.get::<_, Option<String>>(3)?.map(NativeTurnId),
        error: row.get(4)?,
        queue_held: row.get(5)?,
        delivery_slot: row.get(6)?,
        usage_limit_reset_at: row.get(7)?,
        prompt_suggestion: row.get(8)?,
        started_at_ms: row.get(9)?,
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
    let sequence = tx.last_insert_rowid();
    if matches!(
        change,
        Change::Launched
            | Change::SettlementChanged { .. }
            | Change::Run { .. }
            | Change::NativeChild { .. }
            | Change::QuestionRequested { .. }
            | Change::QuestionClosed { .. }
            | Change::PermissionRequested { .. }
            | Change::PermissionClosed { .. }
    ) {
        mark_conversation_changed(tx, thread)?;
    }
    Ok(sequence)
}

fn mark_conversation_changed(tx: &Transaction<'_>, thread: &ThreadId) -> Result<()> {
    tx.execute(
        "UPDATE thread_order SET activity=(SELECT max(activity)+1 FROM thread_order) WHERE id=?",
        [&thread.0],
    )?;
    Ok(())
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
        "INSERT INTO message(id,run_id,role,text,origin,timestamp_ms)
         SELECT ?1,id,?3,?4,?5,CASE WHEN status<>'completed' THEN CAST(unixepoch('subsec')*1000 AS INTEGER) END
         FROM run WHERE id=?2",
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

fn set_settled(tx: &Transaction<'_>, thread: &ThreadId, settled: bool) -> Result<Option<i64>> {
    if tx.execute(
        "UPDATE thread SET settled=?1 WHERE id=?2 AND settled!=?1",
        params![settled, thread.0],
    )? == 0
    {
        return Ok(None);
    }
    event(tx, thread, Change::SettlementChanged { settled }).map(Some)
}

fn start_message(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    text: &str,
    model: Option<String>,
    origin: Option<MessageOrigin>,
    enqueue: bool,
) -> Result<(RunId, MessageId, Vec<i64>, PeerDelivery)> {
    if text.trim().is_empty() {
        return Err(Error::Invalid("empty message".into()));
    }
    if tx
        .prepare(
            "SELECT 1 FROM thread t JOIN project p ON p.id=t.project
         JOIN project checkout ON checkout.root=coalesce(t.worktree_path,p.root)
         JOIN workspace_setup s ON s.project=checkout.id WHERE t.id=? AND s.state!='ready'",
        )?
        .exists([&thread.0])?
    {
        return Err(Error::Invalid(
            "workspace setup must succeed before starting a run".into(),
        ));
    }
    let (busy, held): (bool, bool) = tx.query_row(
        "SELECT count(*) FILTER (WHERE queue_held=0)>0, count(*) FILTER (WHERE queue_held=1)>0
         FROM run WHERE thread_id=? AND status IN ('starting','running','waiting','queued')",
        [&thread.0],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if (busy || held) && origin.is_none() && !enqueue {
        return Err(Error::Busy);
    }
    let (status, delivery) = if busy || (held && enqueue) {
        ("queued", PeerDelivery::Queued)
    } else {
        ("starting", PeerDelivery::Started)
    };
    tx.execute("DELETE FROM usage_recovery WHERE thread_id=?", [&thread.0])?;
    let id = RunId(new_id());
    tx.execute(
        "INSERT INTO run(id,thread_id,status,queue_held,ordinal,task_id,started_at_ms)
         SELECT ?1,?2,?3,?4,COALESCE(MAX(ordinal),0)+1,
            (SELECT id FROM delegated_task WHERE NOT ?4 AND child_thread=?2 AND request_key IS NOT NULL
             AND result_run IS NULL AND delivery='pending'),
            CASE WHEN ?3='starting' THEN CAST(unixepoch('subsec')*1000 AS INTEGER) END
         FROM run WHERE thread_id=?2",
        params![id.0, thread.0, status, delivery == PeerDelivery::Queued && held],
    )?;
    let mut events = Vec::new();
    events.extend(set_settled(tx, thread, false)?);
    let started = run_event(tx, thread, &id)?;
    let (msg, seq) = message(tx, thread, &id, Role::User, origin, text.to_owned())?;
    tx.execute(
        "INSERT INTO turn_effect(run_id,model,state,model_effort,model_service_tier)
         SELECT ?1,?2,'pending',CASE WHEN model IS ?2 THEN model_effort END,
             CASE WHEN model IS ?2 THEN model_service_tier END FROM thread WHERE id=?3",
        params![id.0, model, thread.0],
    )?;
    events.extend([started, seq]);
    Ok((id, msg, events, delivery))
}

fn start_durable_message(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    text: &str,
    model: Option<String>,
    fresh_session: bool,
) -> Result<(RunId, Option<RunJob>)> {
    let (run, _, _, delivery) = start_message(tx, thread, text, model, None, true)?;
    tx.execute(
        "UPDATE turn_effect SET durable_launch=1, fresh_session=? WHERE run_id=?",
        params![fresh_session, run.0],
    )?;
    let job = (delivery == PeerDelivery::Started).then(|| (thread.clone(), run.clone()));
    Ok((run, job))
}

struct StateLock(File);

impl Drop for StateLock {
    fn drop(&mut self) {
        // A concurrent spawn can briefly inherit this descriptor before exec.
        // Release ownership even when another descriptor still refers to the file.
        let _ = self.0.unlock();
    }
}

pub(crate) struct Database {
    output: std::sync::Arc<crate::AgentOutput>,
    db: Connection,
    state_dir: PathBuf,
    attachment_dir: PathBuf,
    storage: crate::StorageLayout,
    secrets: std::collections::HashMap<SecretRequestId, application::Secret>,
    // Drop after SQLite closes.
    _lock: StateLock,
}

impl Database {
    #[cfg(test)]
    pub fn open(dir: &Path) -> Result<Self> {
        Self::open_with_registry(
            dir,
            Default::default(),
            Default::default(),
            &crate::providers::registry::registry(&Default::default(), &Default::default(), &[])?,
        )
    }

    pub fn open_with_registry(
        dir: &Path,
        storage: crate::StorageLayout,
        output: std::sync::Arc<crate::AgentOutput>,
        registry: &crate::providers::registry::Registry,
    ) -> Result<Self> {
        storage.validate()?;
        let state_dir = crate::layout::private_dir(dir)?;
        let (database_path, lock_path) = storage.state_files(&state_dir)?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.try_lock().map_err(|err| match err {
            std::fs::TryLockError::WouldBlock => Error::AlreadyOpen,
            std::fs::TryLockError::Error(e) => Error::Io(e),
        })?;
        let lock = StateLock(lock);
        use sha2::{Digest, Sha256};
        let attachment_dir = database_path.with_file_name(format!(
            ".huggler-attachments-{:x}",
            Sha256::digest(database_path.as_os_str().as_encoded_bytes())
        ));
        let mut db = Connection::open(database_path)?;
        db.execute_batch(
            "PRAGMA foreign_keys=OFF; PRAGMA journal_mode=WAL; PRAGMA temp_store=MEMORY;",
        )?;
        let tx = db.transaction()?;
        let mut version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let latest = MIGRATIONS.len() as i64;
        // The attachment preview also used version 41 before the numbered
        // transcript migrations. Bring that schema through both migrations.
        if version == 41
            && uploads::uses_preview_schema(&tx)?
            && !tx
                .prepare("SELECT 1 FROM sqlite_schema WHERE type='table' AND name='run_activity'")?
                .exists([])?
        {
            version = 40;
            tx.pragma_update(None, "user_version", version)?;
        }
        if !(0..=latest).contains(&version) {
            return Err(Error::SchemaVersion(version));
        }
        for migration in MIGRATIONS.iter().skip(version as usize) {
            tx.execute_batch(migration)?;
        }
        let attachments_migrated = uploads::migrate(&tx)?;
        if (version < latest || attachments_migrated)
            && tx.prepare("PRAGMA foreign_key_check")?.exists([])?
        {
            return Err(Error::Invalid(
                "invalid foreign key after state migration".into(),
            ));
        }
        tx.commit()?;
        db.execute_batch("PRAGMA foreign_keys=ON")?;
        uploads::clean_cache(&db, &attachment_dir)?;
        let tx = db.transaction()?;
        tx.execute_batch("CREATE TEMP TABLE configured_provider(id TEXT PRIMARY KEY, driver TEXT NOT NULL, scope TEXT NOT NULL)")?;
        for instance in registry.values() {
            tx.execute(
                "INSERT INTO configured_provider VALUES(?,?,?)",
                params![
                    instance.id.as_str(),
                    instance.driver.key(),
                    instance.continuation_key()?
                ],
            )?;
        }
        tx.execute_batch(
            "CREATE TEMP TABLE agent_grant(token TEXT PRIMARY KEY, run_id TEXT UNIQUE NOT NULL)",
        )?;
        tx.execute(
            "UPDATE workspace_setup SET state='failed',detail=? WHERE state='running'",
            [output.text.label("runtime closed during setup")],
        )?;
        tx.execute_batch("CREATE TEMP TABLE discovered_tool(run TEXT, name TEXT, PRIMARY KEY(run,name));
            CREATE TEMP TRIGGER forget_tools AFTER DELETE ON agent_grant BEGIN DELETE FROM discovered_tool WHERE run=OLD.run_id; END;
            UPDATE secret_request SET state='expired' WHERE state IN ('requested','approved');")?;
        // Queue unclaimed task launches, wakes and peer messages for a new worker.
        // A claimed effect may already have reached the provider.
        tx.execute("UPDATE run SET status='queued' WHERE status='starting' AND stop_requested=0
            AND id IN (SELECT run_id FROM turn_effect WHERE state='pending')
            AND (id IN (SELECT run_id FROM turn_effect WHERE durable_launch=1) OR id IN (SELECT child_run FROM delegated_task) OR id IN (SELECT wake_run FROM delegated_task)
                OR id IN (SELECT run_id FROM message WHERE origin IS NOT NULL))", [])?;
        let lost = tx
            .prepare(
                "SELECT r.id,r.thread_id,e.state FROM turn_effect e
                      JOIN run r ON r.id=e.run_id WHERE r.status != 'queued' ORDER BY r.rowid",
            )?
            .query_map([], |row| {
                Ok((
                    RunId(row.get(0)?),
                    ThreadId(row.get(1)?),
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, thread, state) in lost {
            let reason = if state == "claimed" {
                "runtime exited after provider claim; delivery is uncertain"
            } else {
                "runtime exited before provider claim; prompt was not sent"
            };
            Self::finish_tx(
                &output,
                &tx,
                &thread,
                &id,
                Err(Error::Provider(reason.into())),
            )?;
        }
        native_children::disconnect(&tx, None)?;
        tx.commit()?;
        Ok(Self {
            db,
            secrets: Default::default(),
            state_dir,
            attachment_dir,
            storage,
            output,
            _lock: lock,
        })
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

    pub fn dispatch(&mut self, cmd: &Command) -> Result<(Receipt, DispatchWork)> {
        let tx = self.db.transaction()?;
        let action = uploads::receipt_action(&cmd.action)?;
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
            return Ok((receipt, DispatchWork::default()));
        }
        let thread = &cmd.thread_id;
        let mut work = DispatchWork::default();
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
                Action::Launch { .. }
                    | Action::Fork { .. }
                    | Action::LaunchProvider { .. }
                    | Action::LaunchInstance { .. }
                    | Action::LaunchModelSelection { .. }
            )
        {
            return Err(Error::NotFound(thread.to_string()));
        }
        // Resolve automatic delivery in the receipt transaction. A retry uses
        // the original intent even if a different turn is now active.
        let automatic_steer = match &cmd.action {
            Action::SendAuto { text } => {
                steering::active_codex_turn(&tx, thread)?.map(|active| Action::Steer {
                    active,
                    text: text.clone(),
                })
            }
            _ => None,
        };
        match automatic_steer.as_ref().unwrap_or(&cmd.action) {
            Action::Launch {
                project,
                name,
                model,
            }
            | Action::LaunchInstance {
                project,
                name,
                model,
                ..
            }
            | Action::LaunchModelSelection {
                project,
                name,
                model,
                ..
            }
            | Action::LaunchProvider {
                project,
                name,
                model,
                ..
            } => {
                let instance = match &cmd.action {
                    Action::LaunchInstance { instance, .. }
                    | Action::LaunchModelSelection { instance, .. } => instance.as_str(),
                    Action::LaunchProvider { provider, .. } => provider.key(),
                    _ => Provider::Codex.key(),
                };
                if name.trim().is_empty() {
                    return Err(Error::Invalid("empty thread name".into()));
                }
                if selection.is_some() {
                    return Err(Error::Invalid("thread already exists".into()));
                }
                let driver = creation::instance_driver(&tx, instance)?;
                let options = match &cmd.action {
                    Action::LaunchModelSelection { options, .. } => options.clone(),
                    _ => ModelOptions::default(),
                };
                validate_model_options(&driver, &options)?;
                let n = tx.execute("INSERT INTO thread(id,name,model,provider,provider_instance,model_effort,model_service_tier,project)
                    SELECT ?,?,?,?,?,?,?,id FROM project WHERE id=?",
                    params![thread.0,name,model,driver,instance,options.effort,options.service_tier,project.0])?;
                if n == 0 {
                    return Err(Error::NotFound(project.to_string()));
                }
                receipt
                    .event_sequences
                    .push(event(&tx, thread, Change::Launched)?);
            }
            Action::CaptureCheckpoint { through } => {
                checkpoints::capture(&tx, thread, through)?;
                receipt.run_id = Some(through.clone());
            }
            Action::Fork {
                source,
                through,
                name,
                instance,
                model,
                mode,
                text,
            } => {
                if selection.is_some() || name.trim().is_empty() {
                    return Err(Error::Invalid(
                        "fork requires a new thread and a name".into(),
                    ));
                }
                let driver = creation::instance_driver(&tx, instance.as_str())?;
                let history = context::capture(&tx, source, through, *mode, true)?;
                tx.execute("INSERT INTO thread(id,project,name,provider_instance,provider,model,worktree_path)
                    SELECT ?1,project,?2,?3,?4,?5,worktree_path FROM thread WHERE id=?6",
                    params![thread.0,name,instance.as_str(),driver,model,source.0])?;
                if *mode == ForkMode::Checkpoint {
                    let (common_dir, commit) = checkpoints::revision(&tx, source, through)?;
                    let project: String = tx.query_row(
                        "SELECT p.root FROM project p JOIN thread t ON t.project=p.id WHERE t.id=?",
                        [&source.0],
                        |r| r.get(0),
                    )?;
                    let path =
                        self.storage
                            .checkout(&self.state_dir, Path::new(&project), &new_id())?;
                    checkpoints::reserve(&tx, thread, &path, &common_dir, &commit)?;
                }
                context::save_fork(&tx, thread, &history, instance)?;
                receipt
                    .event_sequences
                    .push(event(&tx, thread, Change::Launched)?);
                let (run, msg, events, _) =
                    start_message(&tx, thread, text, model.clone(), None, false)?;
                work.start.push((thread.clone(), run.clone()));
                receipt.run_id = Some(run);
                receipt.message_id = Some(msg);
                receipt.event_sequences.extend(events);
            }
            Action::MergeBack {
                source,
                through,
                text,
            } => {
                if !tx
                    .prepare("SELECT 1 FROM context_fork WHERE thread_id=? AND source=?")?
                    .exists(params![source.0, thread.0])?
                {
                    return Err(Error::Forbidden);
                }
                if text.trim().is_empty() {
                    return Err(Error::Invalid("empty merge prompt".into()));
                }
                let history = context::capture(&tx, source, through, ForkMode::Portable, false)?;
                let prompt = self.output.text.history(
                    &self.output.title,
                    &serde_json::to_string(&history)?,
                    text,
                );
                let (run, msg, events, _) =
                    start_message(&tx, thread, &prompt, selection.flatten(), None, false)?;
                work.start.push((thread.clone(), run.clone()));
                receipt.run_id = Some(run);
                receipt.message_id = Some(msg);
                receipt.event_sequences = events;
            }
            Action::SetSettled { settled } => {
                if *settled {
                    if tx.prepare("SELECT 1 FROM run WHERE thread_id=?1 AND status IN ('queued','starting','running','waiting')
                        UNION ALL SELECT 1 FROM native_child c JOIN run r ON r.id=c.run_id WHERE r.thread_id=?1 AND c.active
                        UNION ALL SELECT 1 FROM permission p JOIN run r ON r.id=p.run_id WHERE r.thread_id=?1 AND p.expired=0 AND p.decision IS NULL
                        UNION ALL SELECT 1 FROM user_question u JOIN run r ON r.id=u.run_id WHERE r.thread_id=?1 AND u.state IN ('pending','answering')")?.exists([&thread.0])? {
                        return Err(Error::Busy);
                    }
                    tx.execute("DELETE FROM usage_recovery WHERE thread_id=?", [&thread.0])?;
                }
                receipt
                    .event_sequences
                    .extend(set_settled(&tx, thread, *settled)?);
            }
            Action::SetModel { model } => {
                if selection.as_ref() != Some(model) {
                    tx.execute(
                        "UPDATE thread SET model=?,model_effort=NULL,model_service_tier=NULL WHERE id=?",
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
            Action::SetModelSelection { model, options } => {
                let driver: String =
                    tx.query_row("SELECT provider FROM thread WHERE id=?", [&thread.0], |r| {
                        r.get(0)
                    })?;
                validate_model_options(&driver, options)?;
                if tx.execute(
                    "UPDATE thread SET model=?1,model_effort=?2,model_service_tier=?3 WHERE id=?4
                     AND (model IS NOT ?1 OR model_effort IS NOT ?2 OR model_service_tier IS NOT ?3)",
                    params![model, options.effort, options.service_tier, thread.0],
                )? != 0 {
                    receipt.event_sequences.push(event(&tx, thread, Change::ModelSelectionChanged {
                        model: model.clone(), options: options.clone(),
                    })?);
                }
            }
            Action::Send { text }
            | Action::SendAuto { text }
            | Action::SendPlan { text }
            | Action::Enqueue { text }
            | Action::SendAttachments { text, .. }
            | Action::EnqueueAttachments { text, .. }
            | Action::EnqueueFiles { text, .. } => {
                let owned;
                let files = match &cmd.action {
                    Action::SendAttachments { uploads, .. } => {
                        if !(1..=Attachment::MAX_FILES).contains(&uploads.len()) {
                            return Err(Error::Invalid(
                                "send between one and eight uploads".into(),
                            ));
                        }
                        owned = uploads::claim(&tx, thread, uploads, false)?;
                        owned.as_slice()
                    }
                    Action::EnqueueAttachments { files, .. } => {
                        if !(1..=Attachment::MAX_FILES).contains(&files.len()) {
                            return Err(Error::Invalid("send between one and eight files".into()));
                        }
                        for file in files {
                            uploads::validate_text(&file.name, file.content.len())?;
                        }
                        owned = files
                            .iter()
                            .map(|file| Attachment {
                                name: file.name.clone(),
                                mime: "text/plain".into(),
                                bytes: file.content.as_bytes().to_vec(),
                            })
                            .collect();
                        owned.as_slice()
                    }
                    Action::EnqueueFiles { files, .. } => files.as_slice(),
                    _ => &[],
                };
                let combined;
                let prompt = if files.is_empty() {
                    text
                } else {
                    combined = uploads::prompt(&*self.output.text, text, files);
                    &combined
                };
                let (id, msg, events, delivery) = start_message(
                    &tx,
                    thread,
                    prompt,
                    selection.flatten(),
                    None,
                    matches!(
                        cmd.action,
                        Action::Enqueue { .. }
                            | Action::SendAuto { .. }
                            | Action::EnqueueAttachments { .. }
                            | Action::EnqueueFiles { .. }
                    ),
                )?;
                if !files.is_empty() {
                    uploads::save(&tx, &msg, text, files)?;
                }
                if matches!(cmd.action, Action::SendPlan { .. })
                    && tx.execute(
                        "UPDATE turn_effect SET plan_mode=1 WHERE run_id=? AND
                     EXISTS(SELECT 1 FROM thread WHERE id=? AND provider='codex')",
                        params![id.0, thread.0],
                    )? == 0
                {
                    return Err(Error::Invalid("plan mode requires Codex".into()));
                }
                if delivery == PeerDelivery::Started {
                    work.start.push((thread.clone(), id.clone()));
                }
                receipt.run_id = Some(id);
                receipt.message_id = Some(msg);
                receipt.event_sequences = events;
            }
            Action::Steer { active, text } => {
                receipt
                    .event_sequences
                    .push(steering::request(&tx, thread, active, &cmd.id, text)?);
                receipt.run_id = Some(active.clone());
                work.steer = Some((active.clone(), text.clone()));
            }
            Action::Restart { active, text } => {
                if tx.execute("UPDATE run SET stop_requested=1,stop_reason=?
                    WHERE thread_id=? AND id=? AND stop_requested=0 AND status IN ('starting','running')",
                    params![self.output.text.label("Restarted by the host"),thread.0,active.0])? == 0 {
                    return Err(Error::Invalid("expected an active foreground run".into()));
                }
                let first = tx
                    .query_row(
                        "SELECT id FROM run WHERE thread_id=? AND status='queued'
                    ORDER BY coalesce(delivery_slot,ordinal) LIMIT 1",
                        [&thread.0],
                        |r| Ok(RunId(r.get(0)?)),
                    )
                    .optional()?;
                receipt.event_sequences.push(event(
                    &tx,
                    thread,
                    Change::RunStopRequested {
                        run_id: active.clone(),
                    },
                )?);
                let (id, msg, events, _) =
                    start_message(&tx, thread, text, selection.flatten(), None, true)?;
                receipt.event_sequences.extend(events);
                if let Some(before) = first {
                    receipt.event_sequences.extend(queue::move_before(
                        &tx,
                        thread,
                        &id,
                        Some(&before),
                    )?);
                }
                if tx.execute(
                    "UPDATE run SET queue_held=0 WHERE id=? AND queue_held=1",
                    [&id.0],
                )? != 0
                {
                    receipt.event_sequences.push(run_event(&tx, thread, &id)?);
                }
                receipt.run_id = Some(id);
                receipt.message_id = Some(msg);
                work.stop.push(active.clone());
            }
            Action::EditQueued { run, text } => {
                let (msg, seq) = queue::edit(&tx, thread, run, text)?;
                receipt.run_id = Some(run.clone());
                receipt.message_id = Some(msg);
                receipt.event_sequences.push(seq);
            }
            Action::MoveQueued { run, before } => {
                receipt.event_sequences = queue::move_before(&tx, thread, run, before.as_ref())?;
                receipt.run_id = Some(run.clone());
            }
            Action::CancelQueued { run } => {
                queue::require_queued(&tx, thread, run)?;
                Self::finish_tx(&self.output, &tx, thread, run, Err(Error::Interrupted))?;
                receipt.event_sequences.push(tx.last_insert_rowid());
                receipt.run_id = Some(run.clone());
                tasks::reconcile(&self.output, &tx, &mut work.start)?;
            }
            Action::StopThread | Action::ResumeQueue => {
                let cursor: i64 = tx.query_row(
                    "SELECT coalesce(max(sequence),0) FROM event WHERE thread_id=?",
                    [&thread.0],
                    |r| r.get(0),
                )?;
                if cmd.action == Action::StopThread {
                    work.stop = interrupts::stop_thread(&self.output, &tx, thread)?;
                    // Disposing a held task can release its owner's completion
                    // even when every worker has already exited.
                    tasks::reconcile(&self.output, &tx, &mut work.start)?;
                } else {
                    interrupts::set_queue_hold(&tx, thread, false)?;
                    if let Some(run) = Self::next_queued(&tx, thread)? {
                        receipt.run_id = Some(run.clone());
                        work.start.push((thread.clone(), run));
                    }
                }
                receipt.event_sequences = tx.prepare(
                    "SELECT sequence FROM event WHERE thread_id=? AND sequence>? ORDER BY sequence",
                )?.query_map(params![thread.0, cursor], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
            }
        }
        tx.execute(
            "INSERT INTO receipt(command_id,action,result) VALUES(?,?,?)",
            params![cmd.id.0, action, serde_json::to_string(&receipt)?],
        )?;
        tx.commit()?;
        Ok((receipt, work))
    }

    pub fn worktree(&self, thread: &ThreadId) -> Result<Option<Worktree>> {
        Ok(self
            .db
            .query_row(
                "SELECT w.path,w.common_dir,w.base_commit,w.ready FROM thread t
                 JOIN worktree w ON w.path=t.worktree_path WHERE t.id=?",
                [&thread.0],
                |r| {
                    Ok(Worktree {
                        path: r.get(0)?,
                        common_dir: r.get(1)?,
                        base_commit: r.get(2)?,
                        ready: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn worktree_ready(&self, path: &str) -> Result<()> {
        self.db
            .execute("UPDATE worktree SET ready=1 WHERE path=?", [path])?;
        Ok(())
    }

    pub fn claim(&mut self, id: &RunId) -> Result<Turn> {
        let tx = self.db.transaction()?;
        if tx.execute(
            "UPDATE turn_effect SET state='claimed' WHERE run_id=?1 AND state='pending'
             AND EXISTS(SELECT 1 FROM run WHERE id=?1 AND status='starting' AND stop_requested=0)",
            [&id.0],
        )? != 1
        {
            return Err(Error::Interrupted);
        }
        let (mut turn, message) = tx.query_row(
            "SELECT coalesce(t.worktree_path,p.root),t.name,m.text,e.model,t.native_thread,t.provider,m.origin,e.plan_mode,t.provider_instance,m.id,e.model_effort,e.model_service_tier FROM run r
             JOIN turn_effect e ON e.run_id=r.id
             JOIN thread t ON t.id=r.thread_id JOIN project p ON p.id=t.project
             JOIN message m ON m.run_id=r.id WHERE r.id=? AND m.role='user'",
            [&id.0],
            |r| {
                Ok((Turn {
                    files: Default::default(),
                    output: self.output.clone(),
                    has_app_context: false,
                    sample_suggestion: false,
                    fork: None,
                    plan_mode: r.get(7)?,
                    instance: ProviderInstanceId(r.get(8)?),
                    provider: Provider::from_key(&r.get::<_, String>(5)?)?,
                    agent_tools: None,
                    cwd: r.get::<_, String>(0)?.into(),
                    name: r.get(1)?,
                    text: match r.get::<_, Option<String>>(6)? {
                        Some(origin) => self.output.text.peer_message(&self.output.title, &origin, &r.get::<_, String>(2)?),
                        None => r.get(2)?,
                    },
                    model: r.get(3)?,
                    model_options: ModelOptions { effort: r.get(10)?, service_tier: r.get(11)? },
                    resume: r.get::<_, Option<String>>(4)?.map(NativeThreadId),
                }, MessageId(r.get(9)?)))
            },
        )?;
        let scope: String = tx
            .query_row(
                "SELECT scope FROM configured_provider WHERE id=? AND driver=?",
                params![turn.instance.as_str(), turn.provider.key()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "restore the configured {} instance {}",
                    turn.provider.key(),
                    turn.instance
                ))
            })?;
        let thread = tx
            .query_row(
                "UPDATE thread SET native_thread=NULL, continuation_key=NULL, session_reset=session_reset+1
             WHERE id=(SELECT r.thread_id FROM run r JOIN turn_effect e ON e.run_id=r.id
                       WHERE r.id=? AND e.fresh_session=1) RETURNING id",
                [&id.0],
                |row| Ok(ThreadId(row.get(0)?)),
            )
            .optional()?;
        if let Some(thread) = thread {
            turn.resume = None;
            event(&tx, &thread, Change::SessionReset)?;
        } else if !tx
            .prepare(
                "SELECT 1 FROM thread t JOIN run r ON r.thread_id=t.id
                               WHERE r.id=? AND t.session_reset>0",
            )?
            .exists([&id.0])?
        {
            imports::prepare(&tx, id, &mut turn)?;
            context::prepare(&tx, id, &scope, &mut turn)?;
        }
        #[cfg(feature = "application")]
        if let Some(context) = tx.query_row(
            "UPDATE app_context SET claimed_run=?1 WHERE thread=(SELECT thread_id FROM run WHERE id=?1) RETURNING content",
            [&id.0], |r|r.get::<_,String>(0)).optional()? {
            turn.has_app_context = true;
            turn.text = self.output.text.app_context(&context, &turn.text);
        }
        tx.execute("UPDATE run SET session_generation=(SELECT session_reset FROM thread WHERE id=run.thread_id) WHERE id=?", [&id.0])?;
        if tx.execute(
            "UPDATE thread SET continuation_key=?1 WHERE id=(SELECT thread_id FROM run WHERE id=?2)
            AND (continuation_key IS NULL OR native_thread IS NULL OR continuation_key=?1)",
            params![scope, id.0],
        )? != 1
        {
            return Err(Error::Invalid(format!(
                "restore the original configuration of provider instance {} to resume this session",
                turn.instance
            )));
        }
        turn.files.pending = uploads::load_turn_files(&tx, &message)?;
        turn.files.directory = self.attachment_dir.join(message.0);
        tx.commit()?;
        Ok(turn)
    }

    pub fn can_sample_suggestion(&self, run: &RunId) -> Result<bool> {
        Ok(self.db.prepare(
            "SELECT 1 FROM run r JOIN message m ON m.run_id=r.id
             WHERE r.id=?1 AND m.role='user' AND m.origin IS NULL AND r.task_id IS NULL
             AND NOT EXISTS(SELECT 1 FROM delegated_task WHERE child_thread=r.thread_id AND request_key IS NOT NULL)
             AND NOT EXISTS(SELECT 1 FROM delegated_task WHERE parent_thread=r.thread_id AND wake_run=r.id)
             AND NOT EXISTS(SELECT 1 FROM run WHERE thread_id=r.thread_id AND status='queued')"
        )?.exists([&run.0])?)
    }

    pub fn usage(&self) -> Result<Vec<crate::ProviderUsage>> {
        let mut statement = self.db.prepare(
            "SELECT t.provider_instance, count(*), sum(u.input_tokens), sum(u.output_tokens)
             FROM provider_usage u JOIN run r ON r.id=u.run_id JOIN thread t ON t.id=r.thread_id
             GROUP BY t.provider_instance ORDER BY t.provider_instance",
        )?;
        Ok(statement
            .query_map([], |r| {
                Ok(crate::ProviderUsage {
                    instance: ProviderInstanceId(r.get(0)?),
                    reported_runs: r.get(1)?,
                    input_tokens: r.get(2)?,
                    output_tokens: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn update(&mut self, thread: &ThreadId, id: &RunId, update: Update) -> Result<()> {
        let tx = self.db.transaction()?;
        match update {
            Update::SubmittingAppContext => {
                tx.execute("DELETE FROM app_context WHERE thread=? AND claimed_run=?", params![thread.0, id.0])?;
            }
            Update::Steer(command,outcome) => steering::settle(&tx, thread,id,&command,outcome)?,
            Update::Activity(item) => activity::record(&tx, thread, id, item)?,
            Update::CommandOutput { item, frame } => command_output::record(&tx,thread,id,&item,frame)?,
            Update::Output(text) => {
                event(&tx, thread, Change::OutputDelta { run_id: id.clone(), text })?;
            }
            Update::Usage { input, output } => {
                tx.execute(
                    "INSERT INTO provider_usage(run_id,input_tokens,output_tokens) VALUES(?,?,?)
                     ON CONFLICT(run_id) DO UPDATE SET input_tokens=excluded.input_tokens,
                                                       output_tokens=excluded.output_tokens",
                    params![id.0, input, output],
                )?;
            }
            Update::PermissionWithdrawn(request) => {
                if tx.execute("UPDATE permission SET expired=1 WHERE id=? AND run_id=? AND expired=0 AND decision IS NULL",
                    params![request.0, id.0])? != 0 {
                    event(&tx, thread, Change::PermissionClosed { id: request, option: None })?;
                }
            }
            Update::Question(request) => questions::publish(&tx, thread, request)?,
            Update::QuestionClosed {
                id: request,
                answered,
            } => questions::close(&tx, thread, id, &request, answered)?,
            Update::NativeChild(child) => native_children::record(&tx, thread, child)?,
            Update::PromptSuggestion(text) => {
                if tx.execute(
                    "UPDATE run SET prompt_suggestion=?3 WHERE thread_id=?1 AND id=?2 AND status='completed'
                     AND id=(SELECT id FROM run WHERE thread_id=?1 ORDER BY coalesce(delivery_slot,ordinal) DESC LIMIT 1)",
                    params![thread.0,id.0,text],
                )? == 1 {
                    run_event(&tx, thread, id)?;
                }
            }
            Update::ForegroundComplete => {
                // Finalizing committed the answer. Ignore any later callback once its run settles.
                if !tx.prepare("SELECT 1 FROM run WHERE id=? AND thread_id=? AND status='waiting'")?
                    .exists(params![id.0, thread.0])? {
                    return Err(Error::Interrupted);
                }
                Self::finish_tx(&self.output, &tx, thread, id, Ok(()))?;
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
                if tx.execute(
                    "UPDATE run SET status='running',native_turn=? WHERE id=? AND thread_id=?
                     AND status='starting' AND stop_requested=0",
                    params![turn.map(|t| t.0), id.0, thread.0],
                )? != 1 {
                    return Err(Error::Interrupted);
                }
                run_event(&tx, thread, id)?;
            }
            Update::Finalizing(text) => {
                if tx.execute(
                    "UPDATE run SET status='waiting',finished_at_ms=CAST(unixepoch('subsec')*1000 AS INTEGER)
                     WHERE id=? AND thread_id=? AND status='running' AND stop_requested=0",
                    params![id.0, thread.0],
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
        let request: String = tx
            .query_row(
                "SELECT p.request FROM permission p JOIN run r ON r.id=p.run_id
                 WHERE p.id=? AND r.id=? AND r.thread_id=? AND r.stop_requested=0
                 AND p.expired=0 AND p.decision IS NULL",
                params![id.0, run.0, thread.0],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound("pending permission".into()))?;
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

    // A child approval can outlive the foreground. Close requests for the
    // owner that settled, keeping any that belong to its current running turn.
    fn expire_permissions(
        tx: &Transaction<'_>,
        thread: &ThreadId,
        id: &RunId,
        child: Option<&NativeThreadId>,
        keep_turn: Option<&NativeTurnId>,
    ) -> Result<()> {
        let pending = tx
            .prepare(
                "UPDATE permission SET expired=1 WHERE run_id=?1 AND expired=0 AND decision IS NULL
                AND json_extract(request,'$.native_child.thread_id') IS ?2
                AND (?3 IS NULL OR json_extract(request,'$.native_child.turn_id') IS NOT ?3)
                RETURNING id",
            )?
            .query_map(
                params![id.0, child.map(|t| &t.0), keep_turn.map(|t| &t.0)],
                |r| Ok(RequestId(r.get(0)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in pending {
            event(tx, thread, Change::PermissionClosed { id, option: None })?;
        }
        Ok(())
    }

    fn finish_tx(
        output: &crate::AgentOutput,
        tx: &Transaction<'_>,
        thread: &ThreadId,
        id: &RunId,
        result: Result<()>,
    ) -> Result<()> {
        Self::expire_permissions(tx, thread, id, None, None)?;
        questions::expire_run(tx, thread, id)?;
        steering::disconnect(tx, thread, id)?;
        // Waiting and the final answer commit together. A cleanup error or a
        // process exit after this point cannot undo provider completion.
        if let Err(Error::UsageLimit { reset_at, .. }) = &result {
            usage_limits::record(tx, thread, id, *reset_at)?;
        }
        let interrupted = matches!(result, Err(Error::Interrupted));
        let error = result.err().map(|e| output.text.run_error(&e));
        tx.execute(
            "UPDATE run SET queue_held=0, status=CASE WHEN status='waiting' THEN 'completed' WHEN stop_requested OR ?3 THEN 'interrupted' WHEN ?1 IS NULL THEN 'completed' ELSE 'failed' END,
                 finished_at_ms=coalesce(finished_at_ms,CAST(unixepoch('subsec')*1000 AS INTEGER)),
                 error=CASE WHEN status='waiting' THEN NULL WHEN stop_requested THEN coalesce(stop_reason,?4) ELSE ?1 END WHERE id=?2",
            params![error, id.0,interrupted,output.text.label("task cancelled")],
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
            "SELECT EXISTS(SELECT 1 FROM run WHERE id=? AND thread_id=? AND status IN ('starting','running','waiting'))",
            params![id.0, thread.0],
            |r| r.get(0),
        )?;
        if unfinished {
            Self::finish_tx(&self.output, &tx, thread, id, result)?;
        }
        tx.execute("DELETE FROM agent_grant WHERE run_id=?", [&id.0])?;
        Ok(tx.commit()?)
    }

    /// Promote the oldest queued message when the previous run has settled.
    fn next_queued(tx: &Transaction, thread: &ThreadId) -> Result<Option<RunId>> {
        let id = tx.query_row(
            "UPDATE run SET status='starting',started_at_ms=CAST(unixepoch('subsec')*1000 AS INTEGER)
             WHERE id=(SELECT id FROM run WHERE thread_id=?1 AND status='queued' AND queue_held=0 ORDER BY coalesce(delivery_slot,ordinal) LIMIT 1)
             AND NOT EXISTS(SELECT 1 FROM run WHERE thread_id=?1 AND status IN ('starting','running','waiting')) RETURNING id",
            [&thread.0], |r| Ok(RunId(r.get(0)?)),
        ).optional()?;
        if let Some(id) = &id {
            run_event(tx, thread, id)?;
            tasks::publish_promoted(tx, id)?;
        }
        Ok(id)
    }

    /// Reserve runnable continuations under the same transaction as their delivery receipts.
    pub fn ready_jobs(&mut self) -> Result<Vec<RunJob>> {
        let tx = self.db.transaction()?;
        let threads = tx
            .prepare("SELECT DISTINCT thread_id FROM run WHERE status='queued' AND queue_held=0")?
            .query_map([], |r| Ok(ThreadId(r.get(0)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut jobs = Vec::new();
        for thread in threads {
            if let Some(run) = Self::next_queued(&tx, &thread)? {
                jobs.push((thread, run));
            }
        }
        tasks::reconcile(&self.output, &tx, &mut jobs)?;
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

    fn require_project_thread(&self, thread: &ThreadId, project: &ProjectId) -> Result<()> {
        self.db
            .query_row(
                "SELECT 1 FROM thread WHERE id=? AND project=?",
                params![thread.0, project.0],
                |_| Ok(()),
            )
            .optional()?
            .ok_or(Error::Forbidden)
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
        let (sender, sender_run, project) = self.caller(token)?;
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
                "SELECT original_text FROM peer_receipt WHERE message_id=?",
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
            environment_id: None,
            thread_id: sender.clone(),
            run_id: sender_run.clone(),
            request_key: key.into(),
        };
        let (run_id, message_id, _, delivery) =
            start_message(&tx, target, text, model, Some(origin), false)?;
        let task_id = tasks::record_followup(&tx, &sender, &sender_run, target, &run_id)?;
        let receipt = PeerReceipt {
            task_id,
            thread_id: target.clone(),
            run_id,
            message_id,
            delivery,
        };
        tx.execute(
            "INSERT INTO peer_receipt(sender_thread,request_key,message_id,result,original_text) VALUES(?,?,?,?,?)",
            params![
                sender.0,
                key,
                receipt.message_id.0,
                serde_json::to_string(&receipt)?,text
            ],
        )?;
        tx.commit()?;
        Ok((receipt, true))
    }

    pub fn thread_position(&self, id: &ThreadId) -> Result<i64> {
        self.db
            .query_row(
                "SELECT position FROM thread JOIN thread_order USING(id) WHERE id=?",
                [&id.0],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(id.to_string()))
    }

    pub fn thread_cursor_position(&self, id: &ThreadId) -> Result<i64> {
        self.db
            .query_row(
                "SELECT position FROM thread_order WHERE id=?",
                [&id.0],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(id.to_string()))
    }

    pub fn threads_after(&self, position: i64) -> Result<Vec<ThreadSummary>> {
        Ok(self
            .db
            .prepare("SELECT id,name FROM thread JOIN thread_order USING(id) WHERE position>? ORDER BY position")?
            .query_map([position], |r| {
                Ok(ThreadSummary {
                    id: ThreadId(r.get(0)?),
                    name: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn conversation_revision(&self) -> Result<i64> {
        Ok(self
            .db
            .prepare_cached("SELECT coalesce(max(activity),0) FROM thread_order")?
            .query_row([], |row| row.get(0))?)
    }

    pub fn conversation_list(&self) -> Result<ConversationList> {
        self.conversation_list_for_project(None)
    }

    pub fn conversation_list_for_project(
        &self,
        project: Option<&ProjectId>,
    ) -> Result<ConversationList> {
        let mut statement = self.db.prepare_cached(
            "WITH queued AS (
                SELECT thread_id, max(queue_held) AS held FROM run
                WHERE status='queued' GROUP BY thread_id
             )
             SELECT t.id, t.name,
                    coalesce(fg.status, CASE WHEN q.thread_id IS NOT NULL THEN 'queued' END,
                        latest.status),
                    t.id IN (SELECT r.thread_id FROM permission p JOIN run r ON r.id=p.run_id
                             WHERE p.expired=0 AND p.decision IS NULL),
                    t.id IN (SELECT r.thread_id FROM user_question u JOIN run r ON r.id=u.run_id
                             WHERE u.state IN ('pending','answering')),
                    coalesce(q.held,0),
                    t.id IN (SELECT r.thread_id FROM native_child c JOIN run r ON r.id=c.run_id
                             WHERE c.active), t.settled,
                    CASE WHEN latest.status='completed' AND latest.started_at_ms IS NOT NULL
                         THEN latest.id END
             FROM thread_order o JOIN thread t ON t.id=o.id
             LEFT JOIN run fg ON fg.thread_id=t.id AND fg.status IN ('starting','running','waiting')
             LEFT JOIN queued q ON q.thread_id=t.id
             LEFT JOIN run latest ON fg.id IS NULL AND q.thread_id IS NULL AND latest.id=(
                 SELECT id FROM run WHERE thread_id=t.id ORDER BY coalesce(delivery_slot,ordinal) DESC LIMIT 1
             )
             WHERE ?1 IS NULL OR t.project=?1
             ORDER BY o.activity DESC, o.position DESC",
        )?;
        let threads = statement
            .query_map([project.map(|id| &id.0)], |row| {
                let active_children = row.get(6)?;
                Ok(ConversationSummary {
                    id: ThreadId(row.get(0)?),
                    name: row.get(1)?,
                    status: row
                        .get::<_, Option<String>>(2)?
                        .as_deref()
                        .map(run_status)
                        .transpose()?,
                    needs_approval: row.get(3)?,
                    needs_answer: row.get(4)?,
                    queue_held: row.get(5)?,
                    active_children,
                    settled: row.get(7)?,
                    completed_run: if active_children {
                        None
                    } else {
                        row.get::<_, Option<String>>(8)?.map(RunId)
                    },
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ConversationList {
            revision: self.conversation_revision()?,
            threads,
        })
    }

    #[cfg(all(unix, feature = "machine"))]
    pub fn require_thread_project(&self, thread: &ThreadId, project: &ProjectId) -> Result<()> {
        let found: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM thread WHERE id=? AND project=?)",
            params![thread.0, project.0],
            |row| row.get(0),
        )?;
        if !found {
            return Err(Error::NotFound(format!(
                "conversation {thread} in this project"
            )));
        }
        Ok(())
    }

    pub fn read(&self, id: &ThreadId) -> Result<Thread> {
        let mut thread = self
            .db
            .query_row(
                "SELECT project,name,model,native_thread,provider,worktree_path,provider_instance,settled FROM thread WHERE id=?",
                [&id.0],
                |r| {
                    Ok(Thread {
                        settled:r.get(7)?,
                        provider_instance:ProviderInstanceId(r.get(6)?),
                        questions: vec![],
                        native_children: vec![],
                        tasks: vec![],
                        provider: Provider::from_key(&r.get::<_, String>(4)?)?,
                        permissions: vec![],
                        id: id.clone(),
                        project: ProjectId(r.get(0)?),
                        worktree_path: r.get::<_, Option<String>>(5)?.map(PathBuf::from),
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
        thread.questions = self.open_questions(id)?;
        thread.native_children = native_children::for_thread(&self.db, id)?;
        thread.runs = self
            .db
            .prepare(&format!(
                "SELECT {RUN_FIELDS} FROM run WHERE thread_id=? ORDER BY coalesce(delivery_slot,ordinal)"
            ))?
            .query_map([&id.0], run_row)?
            .collect::<rusqlite::Result<_>>()?;
        thread.messages = self
            .db
            .prepare(
                "SELECT m.id,m.run_id,m.role,m.text,m.origin FROM message m JOIN run r ON m.run_id=r.id
            WHERE r.thread_id=? ORDER BY coalesce(r.delivery_slot,r.ordinal),m.rowid",
            )?
            .query_map([&id.0], history::message_row)?
            .collect::<rusqlite::Result<_>>()?;
        thread.permissions = self.open_permissions(id)?;
        thread.cursor = self.sequence(id)?;
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
        self.check_replay(thread, after)?;
        let mut statement = self.db.prepare("SELECT sequence,change FROM event WHERE thread_id=? AND sequence>? ORDER BY sequence LIMIT 64")?;
        let mut rows = statement.query(params![thread.0, after])?;
        let mut events = vec![];
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let change = row.get_ref(1)?.as_str().map_err(rusqlite::Error::from)?;
            if !events.is_empty() && bytes + change.len() > 256 * 1024 {
                break;
            }
            bytes += change.len();
            events.push(Event {
                sequence: row.get(0)?,
                thread_id: thread.clone(),
                change: serde_json::from_str(change)?,
            });
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn reopen_while_a_spawn_still_holds_a_duplicate_lock_descriptor() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let db = Database::open(dir.path())?;
        // Model the descriptor inherited between fork and exec on another thread.
        let inherited = db._lock.0.try_clone()?;
        assert!(matches!(
            Database::open(dir.path()),
            Err(Error::AlreadyOpen)
        ));
        drop(db);
        let _reopened = Database::open(dir.path())?;
        drop(inherited);
        Ok(())
    }

    #[test]
    fn upgrades_text_attachments_and_staged_uploads_in_shared_state() -> Result<()> {
        for version in [40, 41, 42] {
            check_text_attachment_upgrade(version)?;
        }
        Ok(())
    }

    fn check_text_attachment_upgrade(version: i64) -> Result<()> {
        let dir = tempfile::tempdir()?;
        let conn = Connection::open(dir.path().join("huggler.sqlite3"))?;
        for migration in &MIGRATIONS[..40] {
            conn.execute_batch(migration)?;
        }
        conn.execute_batch(
            "INSERT INTO project(id,root) VALUES('p','/old');
             INSERT INTO thread(id,project,name,provider,provider_instance) VALUES('t','p','legacy','codex','codex');
             INSERT INTO run(id,thread_id,ordinal,status) VALUES('r','t',1,'completed');
             INSERT INTO message(id,run_id,role,text,attachment_caption) VALUES('m','r','user','legacy prompt','caption');
             INSERT INTO message_attachment VALUES('m',0,'notes.txt','déjà vu');
             INSERT INTO pending_upload VALUES('upload','t','docs/staged.txt','legacy upload');",
        )?;
        if version >= 41 {
            conn.execute_batch(
                "CREATE TABLE run_activity (
                    run_id TEXT NOT NULL REFERENCES run(id) ON DELETE CASCADE,
                    item_id TEXT NOT NULL, item TEXT NOT NULL, sequence INTEGER NOT NULL,
                    PRIMARY KEY (run_id,item_id)
                 ) STRICT;
                 INSERT INTO run_activity VALUES('r','item','{}',1);",
            )?;
        }
        if version == 42 {
            conn.execute_batch(
                "ALTER TABLE message ADD COLUMN timestamp_ms INTEGER;
                 ALTER TABLE run ADD COLUMN started_at_ms INTEGER;
                 ALTER TABLE run ADD COLUMN finished_at_ms INTEGER;
                 UPDATE message SET timestamp_ms=42;",
            )?;
        }
        conn.pragma_update(None, "user_version", version)?;
        drop(conn);

        let mut db = Database::open(dir.path())?;
        assert_eq!(
            db.db
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
            MIGRATIONS.len() as i64
        );
        if version >= 41 {
            assert!(
                db.db
                    .prepare("SELECT 1 FROM run_activity WHERE run_id='r'")?
                    .exists([])?
            );
        }
        if version == 42 {
            assert_eq!(
                db.db
                    .query_row("SELECT timestamp_ms FROM message WHERE id='m'", [], |r| r
                        .get::<_, i64>(
                        0
                    ))?,
                42
            );
        }
        // A build using the previous text columns can still stage an upload.
        db.db.execute(
            "INSERT INTO pending_upload(id,thread_id,name,content) VALUES('old-writer','t','old.txt','old writer')",
            [],
        )?;
        let thread = ThreadId("t".into());
        let message = MessageId("m".into());
        let file = db.read_attachment_file(&thread, &message, 0)?;
        assert_eq!(file.name, "notes.txt");
        assert_eq!(file.mime, "text/plain");
        assert_eq!(file.bytes, "déjà vu".as_bytes());
        assert_eq!(db.read_message_body(&thread, &message)?, "caption");
        let (receipt, _) = db.dispatch(&Command {
            id: CommandId("claim-legacy".into()),
            thread_id: thread.clone(),
            action: Action::SendAttachments {
                text: "Read this".into(),
                uploads: vec![UploadId("upload".into()), UploadId("old-writer".into())],
            },
        })?;
        let file = db.read_attachment_file(&thread, receipt.message_id.as_ref().unwrap(), 0)?;
        assert_eq!(file.name, "docs/staged.txt");
        assert_eq!(file.text(), Some("legacy upload"));
        assert_eq!(
            db.read_attachment_file(&thread, receipt.message_id.as_ref().unwrap(), 1)?
                .text(),
            Some("old writer")
        );
        assert!(
            db.claim(receipt.run_id.as_ref().unwrap())?
                .files
                .pending
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn recovers_binary_attachments_from_the_earlier_version_41() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let conn = Connection::open(dir.path().join("huggler.sqlite3"))?;
        for migration in &MIGRATIONS[..40] {
            conn.execute_batch(migration)?;
        }
        conn.execute_batch(
            "DROP TABLE pending_upload;
             CREATE TABLE pending_upload(id TEXT PRIMARY KEY, thread_id TEXT NOT NULL REFERENCES thread(id),
                 name TEXT NOT NULL, mime TEXT NOT NULL, content BLOB NOT NULL) STRICT;
             CREATE INDEX pending_upload_thread ON pending_upload(thread_id);
             DROP TABLE message_attachment;
             CREATE TABLE message_attachment(message_id TEXT NOT NULL REFERENCES message(id) ON DELETE CASCADE,
                 position INTEGER NOT NULL, name TEXT NOT NULL, mime TEXT NOT NULL, content BLOB NOT NULL,
                 inline_text INTEGER NOT NULL, PRIMARY KEY(message_id,position)) STRICT;
             INSERT INTO project(id,root) VALUES('p','/old');
             INSERT INTO thread(id,project,name,provider,provider_instance) VALUES('t','p','preview','codex','codex');
             INSERT INTO run(id,thread_id,ordinal,status) VALUES('r','t',1,'completed');
             INSERT INTO message(id,run_id,role,text,attachment_caption) VALUES('m','r','user','preview','caption');
             INSERT INTO message_attachment VALUES('m',0,'prior.bin','application/octet-stream',x'00ff',0);
             INSERT INTO pending_upload VALUES('upload','t','staged.bin','application/octet-stream',x'017f');
             PRAGMA user_version=41;",
        )?;
        drop(conn);
        let mut db = Database::open(dir.path())?;
        assert_eq!(
            db.db
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
            MIGRATIONS.len() as i64
        );
        let thread = ThreadId("t".into());
        assert_eq!(
            db.read_attachment_file(&thread, &MessageId("m".into()), 0)?
                .bytes,
            [0, 255]
        );
        let (receipt, _) = db.dispatch(&Command {
            id: CommandId("claim-preview".into()),
            thread_id: thread.clone(),
            action: Action::SendAttachments {
                text: String::new(),
                uploads: vec![UploadId("upload".into())],
            },
        })?;
        assert_eq!(
            db.read_attachment_file(&thread, receipt.message_id.as_ref().unwrap(), 0)?
                .bytes,
            [1, 127]
        );
        Ok(())
    }

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
            INSERT INTO event(thread_id,change) VALUES
                ('t','{"type":"message","message":{"id":"m","run_id":"r","role":"assistant","text":"legacy answer"}}'),
                ('t','{"type":"run","run":{"id":"r","ordinal":1,"status":"completed","native_turn":null,"error":null}}');"#)?;
        drop(conn);
        let thread = ThreadId("t".into());
        let db = Database::open(dir.path())?;
        let list = db.conversation_list()?;
        assert!(list.revision > 0);
        assert_eq!(list.threads[0].id, thread);
        assert_eq!(list.threads[0].status, Some(RunStatus::Completed));
        let snapshot = db.read(&thread)?;
        assert!(snapshot.worktree_path.is_none());
        assert_eq!(snapshot.messages[0].text, "legacy answer");
        assert_eq!(snapshot.messages[0].origin, None);
        let events = db.events(&thread, 0)?;
        assert!(
            matches!(&events[0].change, Change::Message {message} if message == &snapshot.messages[0])
        );
        assert!(!snapshot.runs[0].queue_held);
        assert!(matches!(&events[1].change, Change::Run {run} if run == &snapshot.runs[0]));
        Ok(())
    }

    #[test]
    fn v12_native_threads_keep_history_and_bind_to_their_original_driver() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let conn = Connection::open(dir.path().join("huggler.sqlite3"))?;
        for migration in &MIGRATIONS[..12] {
            conn.execute_batch(migration)?;
        }
        conn.execute_batch(r#"INSERT INTO project VALUES('p','/old');
            INSERT INTO thread(rowid,id,project,name,native_thread,provider) VALUES
                (3,'a','p','first','native-a','codex'),(9,'b','p','second','native-b','claudeAgent');
            INSERT INTO run(id,thread_id,ordinal,status) VALUES('r','a',1,'completed');
            INSERT INTO message(id,run_id,role,text) VALUES('m','r','user','original');
            INSERT INTO event(sequence,thread_id,change) VALUES(42,'a','{"type":"message","message":{"id":"m","run_id":"r","role":"user","text":"original"}}');
            INSERT INTO receipt VALUES('send','{"type":"send","text":"original"}',
                '{"command_id":"send","thread_id":"a","message_id":"m","run_id":"r","event_sequences":[42]}');"#)?;
        drop(conn);

        let mut db = Database::open(dir.path())?;
        for (id, rowid, native, driver) in [
            ("a", 3, "native-a", Provider::Codex),
            ("b", 9, "native-b", Provider::ClaudeAgent),
        ] {
            let thread = ThreadId(id.into());
            assert_eq!(db.thread_position(&thread)?, rowid);
            let snapshot = db.read(&thread)?;
            assert_eq!(snapshot.native_thread, Some(NativeThreadId(native.into())));
            assert_eq!(snapshot.provider, driver);
            assert_eq!(snapshot.provider_instance.as_str(), driver.key());
            let mut wire = serde_json::to_value(&snapshot)?;
            wire.as_object_mut().unwrap().remove("provider_instance");
            assert_eq!(
                serde_json::from_value::<Thread>(wire.clone())?
                    .provider_instance
                    .as_str(),
                driver.key()
            );
            wire["provider_instance"] = "another-instance".into();
            assert_eq!(
                serde_json::from_value::<Thread>(wire)?
                    .provider_instance
                    .as_str(),
                "another-instance"
            );
        }
        let thread = ThreadId("a".into());
        let snapshot = db.read(&thread)?;
        assert_eq!(snapshot.messages[0].text, "original");
        assert_eq!(snapshot.runs[0].status, RunStatus::Completed);
        assert!(matches!(&db.events(&thread, 41)?[..],
            [Event { sequence: 42, change: Change::Message { message }, .. }] if message == &snapshot.messages[0]));
        let send = |id: &str, text: &str| Command {
            id: CommandId(id.into()),
            thread_id: thread.clone(),
            action: Action::Send { text: text.into() },
        };
        let (prior, work) = db.dispatch(&send("send", "original"))?;
        assert_eq!(work, DispatchWork::default());
        assert_eq!(prior.run_id.as_ref(), Some(&snapshot.runs[0].id));
        assert_eq!(prior.message_id.as_ref(), Some(&snapshot.messages[0].id));
        assert_eq!(prior.event_sequences, [42]);
        let (next, _) = db.dispatch(&send("continue", "follow up"))?;
        let turn = db.claim(next.run_id.as_ref().unwrap())?;
        assert_eq!(turn.resume, snapshot.native_thread);
        assert!(
            db.db
                .prepare(
                    "SELECT 1 FROM thread t JOIN configured_provider c
            ON t.provider_instance=c.id AND t.continuation_key=c.scope WHERE t.id='a'"
                )?
                .exists([])?
        );
        assert!(
            db.db
                .prepare("SELECT 1 FROM thread WHERE id='b' AND continuation_key IS NULL")?
                .exists([])?
        );
        assert!(!db.db.prepare("PRAGMA foreign_key_check")?.exists([])?);
        Ok(())
    }

    #[test]
    fn v7_tasks_keep_their_runs_and_retry_receipts() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let conn = Connection::open(dir.path().join("huggler.sqlite3"))?;
        for migration in &MIGRATIONS[..7] {
            conn.execute_batch(migration)?;
        }
        conn.execute_batch(r#"INSERT INTO project VALUES('p','/old');
            INSERT INTO thread(id,project,name) VALUES('owner','p','parent'),('child','p','child'),('later','p','sibling');
            INSERT INTO run(id,thread_id,ordinal,status) VALUES
                ('sender','owner',1,'completed'),('first','child',1,'completed'),('revision','child',2,'completed'),
                ('sibling-run','later',1,'completed');
            INSERT INTO message(id,run_id,role,text) VALUES
                ('input','revision','user','revise'),('answer','revision','assistant','revised'),
                ('sibling-answer','sibling-run','assistant','pending at restart');
            INSERT INTO delegated_task(id,parent_thread,parent_run,request_key,intent,child_thread,child_run,result_run,delivery)
                VALUES('task','owner','sender','original',
                       '{"task":"review","clientRequestId":"original","target":null,"title":null,"role":null,"mode":"async","isolation":"shared"}',
                       'child','first','revision','disposed'),
                      ('sibling','owner','sender','sibling','{}','later','sibling-run',NULL,'pending');
            INSERT INTO task_cancel_receipt VALUES('task','request:stop');
            INSERT INTO peer_receipt VALUES('owner','old-send','input',
                '{"threadId":"child","messageId":"input","runId":"revision","delivery":"queued"}');"#)?;
        drop(conn);
        let mut db = Database::open(dir.path())?;
        let owner = ThreadId("owner".into());
        let child = ThreadId("child".into());
        let (receipt, _) = db.dispatch(&Command {
            id: CommandId("resume".into()),
            thread_id: owner.clone(),
            action: Action::Send {
                text: "resume".into(),
            },
        })?;
        let token = db.grant(&receipt.run_id.unwrap())?;
        db.ready_jobs()?;
        let tasks = db.read(&owner)?.tasks;
        assert_eq!(tasks[0].result.as_deref(), Some("revised"));
        assert_eq!(tasks[0].delivery, CompletionDelivery::Disposed);
        assert_eq!(tasks[1].result.as_deref(), Some("pending at restart"));

        let retry = |task| {
            serde_json::from_value(serde_json::json!({
                "clientRequestId": "original", "task": task,
            }))
        };
        let (original, fresh) = db.delegate(&token, retry("review")?)?;
        assert!(!fresh);
        assert_eq!(original, tasks[0]);
        assert!(matches!(
            db.delegate(&token, retry("changed")?),
            Err(Error::Conflict { .. })
        ));

        let (old, fresh) = db.send_peer(&token, &child, "old-send", "revise")?;
        assert!(!fresh && old.task_id.is_none());
        let (new, fresh) = db.send_peer(&token, &child, "original", "revise again")?;
        assert!(fresh && new.task_id.is_some());
        let (_, stops) = db.cancel_task(&token, &tasks[0].task_id, Some("stop"))?;
        assert!(stops.is_empty());
        assert_eq!(
            db.read(&owner)?.tasks[2].delivery,
            CompletionDelivery::Pending
        );
        let violations: i64 =
            db.db
                .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                    r.get(0)
                })?;
        assert_eq!(violations, 0);
        Ok(())
    }

    #[test]
    fn manual_settlement_is_durable_idempotent_and_cleared_by_a_turn() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut db = Database::open(dir.path())?;
        let project = db.project(dir.path())?.id;
        let thread = ThreadId("conversation".into());
        let command = |id: &str, action| Command {
            id: CommandId(id.into()),
            thread_id: thread.clone(),
            action,
        };
        db.dispatch(&command(
            "launch",
            Action::Launch {
                project,
                name: "Manual settlement".into(),
                model: None,
            },
        ))?;
        let settle = Action::SetSettled { settled: true };
        let (receipt, _) = db.dispatch(&command("settle", settle.clone()))?;
        assert_eq!(receipt.event_sequences.len(), 1);
        let settled = db.conversation_list()?;
        assert!(settled.threads[0].settled);
        let (noop, _) = db.dispatch(&command("again", settle.clone()))?;
        assert!(noop.event_sequences.is_empty());
        assert_eq!(db.conversation_list()?, settled);
        drop(db);
        let mut db = Database::open(dir.path())?;
        assert!(db.read(&thread)?.settled);
        assert!(
            db.dispatch(&command(
                "empty",
                Action::Send {
                    text: String::new()
                }
            ))
            .is_err()
        );
        assert!(db.read(&thread)?.settled);
        let (sent, _) = db.dispatch(&command(
            "follow-up",
            Action::Send {
                text: "Continue".into(),
            },
        ))?;
        let changes = db.events(&thread, receipt.event_sequences[0])?;
        assert!(matches!(
            changes[0].change,
            Change::SettlementChanged { settled: false }
        ));
        assert_eq!(
            changes.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            sent.event_sequences
        );
        assert!(!db.read(&thread)?.settled);
        assert!(matches!(
            db.dispatch(&command("busy", settle.clone())),
            Err(Error::Busy)
        ));
        db.dispatch(&command(
            "enqueue",
            Action::Enqueue {
                text: "Next".into(),
            },
        ))?;
        db.dispatch(&command("pause", Action::StopThread))?;
        assert!(matches!(
            db.dispatch(&command("queued", settle)),
            Err(Error::Busy)
        ));
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
        let running_list = db.conversation_list()?;
        db.update(&thread, run, Update::Output("partial".into()))?;
        assert_eq!(db.conversation_list()?, running_list);
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
            assert_eq!(
                recovered.dispatch(&send)?,
                (receipt.clone(), DispatchWork::default())
            );
        }
        Ok(())
    }
}
