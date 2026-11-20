use super::*;
use crate::{
    mcp::{Delegation, Isolation},
    worktree,
};

const SELECT_TASK: &str = "SELECT t.id,t.parent_thread,t.parent_run,t.child_thread,t.child_run,
    t.result_run,t.delivery,t.wake_run,CASE WHEN t.cancelled AND t.result_run IS NOT NULL THEN 'interrupted'
        WHEN initial.status='queued' THEN 'queued' ELSE r.status END,
    CASE WHEN t.cancelled AND t.result_run IS NOT NULL THEN 'task cancelled' ELSE coalesce(r.error,m.text) END
    FROM delegated_task t JOIN run initial ON initial.id=t.child_run
    LEFT JOIN run r ON r.id=t.result_run
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
        Some("queued") => RunStatus::Queued,
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

pub(super) fn publish_promoted(tx: &Transaction, run: &RunId) -> Result<()> {
    let task = tx
        .query_row(
            "SELECT id,parent_thread FROM delegated_task WHERE child_run=?",
            [&run.0],
            |r| Ok((TaskId(r.get(0)?), ThreadId(r.get(1)?))),
        )
        .optional()?;
    if let Some((id, owner)) = task {
        publish(tx, &owner, &id)?;
    }
    Ok(())
}

pub(super) fn record_followup(
    tx: &Transaction,
    sender: &ThreadId,
    sender_run: &RunId,
    target: &ThreadId,
    run: &RunId,
) -> Result<Option<TaskId>> {
    let id = TaskId(new_id());
    let inserted = tx.execute(
        "INSERT INTO delegated_task(id,parent_thread,parent_run,child_thread,child_run)
         SELECT ?1,parent_thread,?2,child_thread,?3 FROM delegated_task
         WHERE child_thread=?4 AND parent_thread=?5 AND request_key IS NOT NULL AND result_run IS NOT NULL",
        params![id.0, sender_run.0, run.0, target.0, sender.0],
    )?;
    if inserted == 0 {
        return Ok(None);
    }
    publish(tx, sender, &id)?;
    Ok(Some(id))
}

impl Database {
    pub fn delegate(&mut self, token: &str, input: Delegation) -> Result<(DelegatedTask, bool)> {
        let (parent, parent_run, _) = self.caller(token)?;
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
        let provider = input.target.map(|t| t.provider_instance_id);
        let name = input.title.as_deref().unwrap_or("Delegated task");
        let child = creation::create_agent_thread(&tx, &parent, name, provider)?;
        if matches!(input.isolation, Isolation::Worktree) {
            let cwd: String = tx.query_row(
                "SELECT coalesce(t.worktree_path,p.root) FROM thread t JOIN project p ON p.id=t.project WHERE t.id=?",
                [&parent.0], |r| r.get(0),
            )?;
            let (common_dir, base) = worktree::base(Path::new(&cwd))?;
            let path = self.worktree_dir.join(&child.0);
            let path = path
                .to_str()
                .ok_or_else(|| Error::Invalid("worktree path requires UTF-8".into()))?;
            tx.execute(
                "INSERT INTO worktree(path,common_dir,base_commit) VALUES(?,?,?)",
                params![path, common_dir, base],
            )?;
            tx.execute(
                "UPDATE thread SET worktree_path=? WHERE id=?",
                params![path, child.0],
            )?;
        }
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

    pub fn task_awaits_approval(&self, task: &TaskId) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM permission p JOIN run r ON r.id=p.run_id
             WHERE r.task_id=? AND p.expired=0 AND p.decision IS NULL)",
            [&task.0],
            |row| row.get(0),
        )?)
    }

    pub fn cancel_task(
        &mut self,
        token: &str,
        id: &TaskId,
        key: Option<&str>,
    ) -> Result<(DelegatedTask, Vec<RunId>)> {
        let (parent, caller_run, _) = self.caller(token)?;
        let tx = self.db.transaction()?;
        let task = owned(&tx, &parent, id)?;
        let request_key = match key {
            Some(k) if k.trim().is_empty() => {
                return Err(Error::Invalid("empty clientRequestId".into()));
            }
            Some(k) => format!("request:{k}"),
            None => format!("run:{caller_run}"),
        };
        if tx.execute(
            "INSERT INTO task_cancel_receipt(task_id,request_key) VALUES(?,?)
                       ON CONFLICT(task_id,request_key) DO NOTHING",
            params![id.0, request_key],
        )? == 0
        {
            return Ok((task, Vec::new()));
        }
        // The tree belongs to the parent even after a task returns its result.
        // A later turn in any of these threads can have more work in flight.
        let subtree = tx
            .prepare(
                "WITH RECURSIVE family(child) AS (
                    SELECT child_thread FROM delegated_task WHERE id=?1
                    UNION
                    SELECT t.child_thread FROM delegated_task t JOIN family f ON t.parent_thread=f.child
                    WHERE t.request_key IS NOT NULL
                ) SELECT child FROM family",
            )?
            .query_map([&id.0], |r| Ok(ThreadId(r.get(0)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stops = Vec::new();
        for child in subtree {
            let disposed = tx.prepare(
                "UPDATE delegated_task AS t SET delivery='disposed',cancelled=(result_run IS NULL AND
                    (EXISTS (SELECT 1 FROM run WHERE task_id=t.id AND status IN ('starting','running','queued'))
                     OR EXISTS (SELECT 1 FROM native_child c JOIN run r ON r.id=c.run_id
                                WHERE c.active AND r.task_id=t.id)
                     OR EXISTS (SELECT 1 FROM delegated_task d JOIN run r ON r.id=d.parent_run
                                WHERE r.task_id=t.id AND (d.result_run IS NULL OR d.delivery='pending'))))
                 WHERE child_thread=?1 AND delivery='pending'
                   AND (id=?2 OR child_thread<>?3 OR result_run IS NULL)
                 RETURNING id,parent_thread",
            )?.query_map(params![child.0, id.0, task.child_thread_id.0], |r|
                Ok((TaskId(r.get(0)?), ThreadId(r.get(1)?))))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (subtask, owner) in disposed {
                publish(&tx, &owner, &subtask)?;
            }
            let runs = tx.prepare("SELECT id,status FROM run WHERE thread_id=? AND
                (status IN ('starting','running','queued') OR id IN (SELECT run_id FROM native_child WHERE active))")?
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
            SELECT id FROM run WHERE task_id=t.id ORDER BY ordinal DESC LIMIT 1)
            WHERE result_run IS NULL AND NOT EXISTS (
                SELECT 1 FROM run WHERE task_id=t.id AND status IN ('starting','running','waiting','queued'))
            AND NOT EXISTS (SELECT 1 FROM native_child c JOIN run r ON r.id=c.run_id
                WHERE c.active AND r.task_id=t.id)
            AND NOT EXISTS (SELECT 1 FROM delegated_task d JOIN run r ON r.id=d.parent_run WHERE r.task_id=t.id
                AND (d.result_run IS NULL OR d.delivery='pending'))
            RETURNING id,parent_thread")?.query_map([],|r|Ok((TaskId(r.get(0)?),ThreadId(r.get(1)?))))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, parent) in &settled {
            publish(tx, parent, id)?;
        }
        // Group siblings into one wake. If two runs on this parent belong to
        // different outer tasks, each needs its own wake and final response.
        let parents = tx
            .prepare(
                "SELECT ready.parent_thread,r.task_id FROM (
                    SELECT parent_thread,min(rowid) AS first_ready FROM delegated_task
                    WHERE delivery='pending' AND result_run IS NOT NULL GROUP BY parent_thread
                 ) ready JOIN delegated_task t ON t.rowid=ready.first_ready JOIN run r ON r.id=t.parent_run
                 WHERE NOT EXISTS (SELECT 1 FROM run WHERE thread_id=ready.parent_thread
                                   AND status IN ('starting','running','waiting','queued'))",
            )?
            .query_map([], |row| Ok((ThreadId(row.get(0)?), row.get::<_, Option<String>>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (parent, outer_task) in parents {
            let tasks = tx
                .prepare(&format!(
                    "{SELECT_TASK} JOIN run owner ON owner.id=t.parent_run
                WHERE t.parent_thread=?1 AND owner.task_id IS ?2 AND t.delivery='pending'
                AND t.result_run IS NOT NULL ORDER BY t.rowid"
                ))?
                .query_map(params![parent.0, outer_task], task_row)?
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
            tx.execute(
                "UPDATE run SET task_id=? WHERE id=?",
                params![outer_task, wake.0],
            )?;
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
