use super::*;
use crate::mcp::Delegation;

const SELECT_TASK: &str = "SELECT t.id,t.parent_thread,t.parent_run,t.child_thread,t.child_run,
    t.result_run,t.delivery,t.wake_run,CASE WHEN t.cancelled AND t.result_run IS NOT NULL THEN 'interrupted' ELSE r.status END,
    CASE WHEN t.cancelled AND t.result_run IS NOT NULL THEN 'task cancelled' ELSE coalesce(r.error,m.text) END
    FROM delegated_task t LEFT JOIN run r ON r.id=t.result_run
    LEFT JOIN message m ON m.run_id=r.id AND m.role='assistant'";

fn task_row(row: &Row<'_>) -> rusqlite::Result<DelegatedTask> {
    let delivery = match row.get::<_, String>(6)?.as_str() {
        "pending" => CompletionDelivery::Pending,
        "acknowledged" => CompletionDelivery::Acknowledged,
        "delivered" => CompletionDelivery::Delivered,
        "disposed" => CompletionDelivery::Disposed,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let status = match row.get::<_, Option<String>>(8)?.as_deref() {
        None => RunStatus::Running,
        Some("completed") => RunStatus::Completed,
        Some("failed") => RunStatus::Failed,
        Some("interrupted") => RunStatus::Interrupted,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(DelegatedTask {
        task_id: TaskId(row.get(0)?),
        parent_thread_id: ThreadId(row.get(1)?),
        parent_run_id: RunId(row.get(2)?),
        child_thread_id: ThreadId(row.get(3)?),
        child_run_id: RunId(row.get(4)?),
        result_run_id: row.get::<_, Option<String>>(5)?.map(RunId),
        delivery,
        wake_run_id: row.get::<_, Option<String>>(7)?.map(RunId),
        status,
        result: row.get(9)?,
    })
}

pub(super) fn for_parent(db: &Connection, parent: &ThreadId) -> Result<Vec<DelegatedTask>> {
    Ok(db
        .prepare(&format!(
            "{SELECT_TASK} WHERE t.parent_thread=? ORDER BY t.rowid"
        ))?
        .query_map([&parent.0], task_row)?
        .collect::<rusqlite::Result<_>>()?)
}

fn owned(db: &Connection, parent: &ThreadId, id: &TaskId) -> Result<DelegatedTask> {
    db.query_row(
        &format!("{SELECT_TASK} WHERE t.parent_thread=? AND t.id=?"),
        params![parent.0, id.0],
        task_row,
    )
    .optional()?
    .ok_or(Error::Forbidden)
}

fn publish(tx: &Transaction, parent: &ThreadId, id: &TaskId) -> Result<DelegatedTask> {
    let task = owned(tx, parent, id)?;
    event(tx, parent, Change::Task { task: task.clone() })?;
    Ok(task)
}

impl Database {
    pub fn delegate(&mut self, token: &str, input: Delegation) -> Result<(DelegatedTask, bool)> {
        let (parent, parent_run, project) = self.caller(token)?;
        if input.client_request_id.trim().is_empty() || input.task.trim().is_empty() {
            return Err(Error::Invalid("expected a task and clientRequestId".into()));
        }
        let intent = serde_json::to_string(&input)?;
        let tx = self.db.transaction()?;
        let previous = tx
            .query_row(
                "SELECT id,intent FROM delegated_task WHERE parent_thread=? AND request_key=?",
                params![parent.0, input.client_request_id],
                |r| Ok((TaskId(r.get(0)?), r.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((id, original)) = previous {
            if intent != original {
                return Err(Error::Conflict {
                    command: CommandId(input.client_request_id),
                    original_thread: parent.clone(),
                    requested_thread: parent,
                });
            }
            return Ok((owned(&tx, &parent, &id)?, false));
        }
        let id = TaskId(new_id());
        let child = ThreadId(new_id());
        let provider = input.target.map(|t| t.provider_instance_id.key());
        let name = input.title.as_deref().unwrap_or("Delegated task");
        // The configured provider is inherited unless the caller selects another instance.
        tx.execute(
            "INSERT INTO thread(id,project,name,provider,model)
            SELECT ?1,?2,?3,coalesce(?4,provider),CASE WHEN ?4 IS NULL OR ?4=provider THEN model END
            FROM thread WHERE id=?5",
            params![child.0, project.0, name, provider, parent.0],
        )?;
        event(&tx, &child, Change::Launched)?;
        let model = tx.query_row("SELECT model FROM thread WHERE id=?", [&child.0], |r| {
            r.get(0)
        })?;
        let prompt = match input.role {
            Some(role) => format!("Role: {role}\n\n{}", input.task),
            None => input.task,
        };
        let (run, _, _, _) = start_message(&tx, &child, &prompt, model, None)?;
        tx.execute("INSERT INTO delegated_task(id,parent_thread,parent_run,request_key,intent,child_thread,child_run)
            VALUES(?,?,?,?,?,?,?)", params![id.0,parent.0,parent_run.0,input.client_request_id,intent,child.0,run.0])?;
        let task = publish(&tx, &parent, &id)?;
        tx.commit()?;
        Ok((task, true))
    }

    pub fn task_status(&mut self, token: &str, id: &TaskId) -> Result<DelegatedTask> {
        let (parent, _, _) = self.caller(token)?;
        let tx = self.db.transaction()?;
        let mut task = owned(&tx, &parent, id)?;
        if task.result_run_id.is_some() && task.delivery == CompletionDelivery::Pending {
            tx.execute(
                "UPDATE delegated_task SET delivery='acknowledged' WHERE id=?",
                [&id.0],
            )?;
            task = publish(&tx, &parent, id)?;
        }
        tx.commit()?;
        Ok(task)
    }

    pub fn child_awaits_approval(&self, thread: &ThreadId) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM permission p JOIN run r ON r.id=p.run_id
             WHERE r.thread_id=? AND p.expired=0 AND p.decision IS NULL)",
            [&thread.0],
            |row| row.get(0),
        )?)
    }

    pub fn cancel_task(&mut self, token: &str, id: &TaskId) -> Result<(DelegatedTask, Vec<RunId>)> {
        let (parent, _, _) = self.caller(token)?;
        let tx = self.db.transaction()?;
        owned(&tx, &parent, id)?;
        // A finished task retains its result. Its pending notification can still be disposed.
        let subtree = tx
            .prepare(
                "WITH RECURSIVE family(id,child_thread,open) AS (
            SELECT id,child_thread,result_run IS NULL FROM delegated_task WHERE id=?1
            UNION ALL SELECT t.id,t.child_thread,t.result_run IS NULL FROM delegated_task t JOIN family f
            ON f.open AND t.parent_thread=f.child_thread
        ) SELECT t.id,t.parent_thread,t.child_thread,t.result_run IS NULL FROM delegated_task t
        JOIN family f ON f.id=t.id",
            )?
            .query_map([&id.0], |r| {
                Ok((
                    TaskId(r.get(0)?),
                    ThreadId(r.get(1)?),
                    ThreadId(r.get(2)?),
                    r.get::<_, bool>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stops = Vec::new();
        for (subtask, owner, child, active) in subtree {
            if tx.execute(
                "UPDATE delegated_task SET delivery='disposed',cancelled=(result_run IS NULL AND
                    (EXISTS (SELECT 1 FROM run WHERE thread_id=child_thread AND status IN ('starting','running','queued'))
                     OR EXISTS (SELECT 1 FROM delegated_task d WHERE d.parent_thread=delegated_task.child_thread
                                AND (d.result_run IS NULL OR d.delivery='pending'))))
                 WHERE id=? AND delivery='pending'",
                [&subtask.0],
            )? != 0
            {
                publish(&tx, &owner, &subtask)?;
            }
            if !active {
                continue;
            }
            let runs = tx.prepare("SELECT id,status FROM run WHERE thread_id=? AND status IN ('starting','running','queued')")?
                .query_map([&child.0],|r|Ok((RunId(r.get(0)?),r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for (run, status) in runs {
                tx.execute("UPDATE run SET stop_requested=1 WHERE id=?", [&run.0])?;
                if status == "queued" {
                    Self::finish_tx(&tx, &child, &run, Err(Error::Interrupted))?;
                } else {
                    stops.push(run);
                }
            }
        }
        let result = owned(&tx, &parent, id)?;
        tx.commit()?;
        Ok((result, stops))
    }
}

/// Settle a child only after its queue and descendants settle. Group available
/// sibling results into one wake when their parent has no work ahead of them.
pub(super) fn reconcile(tx: &Transaction, jobs: &mut Vec<(ThreadId, RunId)>) -> Result<()> {
    loop {
        let settled = tx.prepare("UPDATE delegated_task AS t SET result_run=(
            SELECT id FROM run WHERE thread_id=t.child_thread ORDER BY ordinal DESC LIMIT 1)
            WHERE result_run IS NULL AND NOT EXISTS (
                SELECT 1 FROM run WHERE thread_id=t.child_thread AND status IN ('starting','running','waiting','queued'))
            AND NOT EXISTS (SELECT 1 FROM delegated_task d WHERE d.parent_thread=t.child_thread
                AND (d.result_run IS NULL OR d.delivery='pending'))
            RETURNING id,parent_thread")?.query_map([],|r|Ok((TaskId(r.get(0)?),ThreadId(r.get(1)?))))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, parent) in &settled {
            publish(tx, parent, id)?;
        }
        let parents = tx.prepare("SELECT DISTINCT parent_thread FROM delegated_task t
            WHERE delivery='pending' AND result_run IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM run WHERE thread_id=t.parent_thread AND status IN ('starting','running','waiting','queued'))")?
            .query_map([],|r|Ok(ThreadId(r.get(0)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for parent in parents {
            let tasks = tx
                .prepare(&format!(
                    "{SELECT_TASK} WHERE t.parent_thread=? AND t.delivery='pending'
                AND t.result_run IS NOT NULL ORDER BY t.rowid"
                ))?
                .query_map([&parent.0], task_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let results: Vec<_> = tasks.iter().map(|t|serde_json::json!({
                "taskId":t.task_id,"childThreadId":t.child_thread_id,"resultRunId":t.result_run_id,
                "status":t.status,"result":t.result
            })).collect();
            let prompt = format!(
                "Huggler delegated task results\n{}",
                serde_json::to_string(&results)?
            );
            let model = tx.query_row("SELECT model FROM thread WHERE id=?", [&parent.0], |r| {
                r.get(0)
            })?;
            let (wake, _, _, _) = start_message(tx, &parent, &prompt, model, None)?;
            for task in tasks {
                tx.execute(
                    "UPDATE delegated_task SET delivery='delivered',wake_run=? WHERE id=?",
                    params![wake.0, task.task_id.0],
                )?;
                publish(tx, &parent, &task.task_id)?;
            }
            jobs.push((parent, wake));
        }
        // Disposing a cancelled descendant can make its ancestor immediately eligible.
        if settled.is_empty() {
            return Ok(());
        }
    }
}
