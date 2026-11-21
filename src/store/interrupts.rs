use super::*;

pub(super) fn set_queue_hold(tx: &Transaction, thread: &ThreadId, hold: bool) -> Result<()> {
    let changed = tx
        .prepare(
            "UPDATE run SET queue_held=?1 WHERE thread_id=?2 AND status='queued' AND queue_held<>?1
         RETURNING id",
        )?
        .query_map(params![hold, thread.0], |r| Ok(RunId(r.get(0)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in changed {
        run_event(tx, thread, &id)?;
    }
    Ok(())
}

pub(super) fn stop_thread(
    output: &crate::AgentOutput,
    tx: &Transaction,
    root: &ThreadId,
) -> Result<Vec<RunId>> {
    let mut stops = Vec::new();
    let reason = output.text.label("Stopped by the host");
    let cancelled = output.text.label("task cancelled");
    for thread in tasks::subtree(tx, root)? {
        tx.execute("DELETE FROM usage_recovery WHERE thread_id=?", [&thread.0])?;
        set_queue_hold(tx, &thread, true)?;
        if &thread != root {
            tasks::dispose(tx, &thread, None, &cancelled)?;
        }
        // When stopping a child directly, its owner can still receive the
        // active task's result. Held secondary messages resume independently.
        tx.execute(
            "UPDATE run SET task_id=NULL WHERE thread_id=? AND queue_held AND
             EXISTS(SELECT 1 FROM delegated_task t WHERE t.id=run.task_id
                    AND (t.child_run<>run.id OR t.delivery='disposed'))",
            [&thread.0],
        )?;
        let runs = tx
            .prepare(
                "UPDATE run SET stop_requested=1, stop_reason=?1
             WHERE thread_id=?2 AND stop_requested=0 AND
               (status IN ('starting','running','waiting') OR
                id IN (SELECT run_id FROM native_child WHERE active))
             RETURNING id",
            )?
            .query_map(params![reason, thread.0], |r| Ok(RunId(r.get(0)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for run in &runs {
            event(
                tx,
                &thread,
                Change::RunStopRequested {
                    run_id: run.clone(),
                },
            )?;
        }
        stops.extend(runs);
    }
    Ok(stops)
}

impl Database {
    /// Returns the terminal status for a settled run, or None while stopping it.
    pub fn interrupt_run(
        &mut self,
        token: &str,
        thread: &ThreadId,
        run: &RunId,
        key: &str,
        reason: Option<&str>,
    ) -> Result<Option<RunStatus>> {
        let (caller, caller_run, project) = self.caller(token)?;
        self.require_project_thread(thread, &project)?;
        if key.trim().is_empty() {
            return Err(Error::Invalid("empty clientRequestId".into()));
        }
        let status = self.find_run(thread, run)?.status;
        let tx = self.db.transaction()?;
        let previous = tx
            .query_row(
                "SELECT r.thread_id,i.run_id,i.reason FROM run_interrupt_receipt i
                 JOIN run r ON r.id=i.run_id WHERE i.sender_thread=? AND i.request_key=?",
                params![caller.0, key],
                |row| {
                    Ok((
                        ThreadId(row.get(0)?),
                        RunId(row.get(1)?),
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((original_thread, original_run, original_reason)) = previous {
            if original_run != *run || original_reason.as_deref() != reason {
                return Err(Error::Conflict {
                    command: CommandId(key.into()),
                    original_thread,
                    requested_thread: thread.clone(),
                });
            }
        } else {
            tx.execute(
                "INSERT INTO run_interrupt_receipt(sender_thread,request_key,run_id,reason) VALUES(?,?,?,?)",
                params![caller.0, key, run.0, reason],
            )?;
        }
        if status.terminal() {
            tx.commit()?;
            return Ok(Some(status));
        }
        if !matches!(status, RunStatus::Starting | RunStatus::Running) {
            return Err(Error::Invalid("run is not interruptible".into()));
        }
        let message = self.output.text.interrupted_by_agent(reason);
        if tx.execute(
            "UPDATE run SET stop_requested=1,stop_reason=? WHERE id=? AND stop_requested=0",
            params![message, run.0],
        )? == 1
        {
            event(
                &tx,
                thread,
                Change::RunInterruptRequested {
                    run_id: run.clone(),
                    caller_thread_id: caller,
                    caller_run_id: caller_run,
                    reason: reason.map(str::to_owned),
                },
            )?;
        }
        tx.commit()?;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_reviewer(dir: &Path) -> Result<(Database, ThreadId, RunId)> {
        let mut db = Database::open(dir)?;
        let project = db.project(dir)?;
        let thread = ThreadId("reviewer".into());
        db.dispatch(&Command {
            id: CommandId("launch".into()),
            thread_id: thread.clone(),
            action: Action::Launch {
                project: project.id,
                name: "Reviewer".into(),
                model: None,
            },
        })?;
        let (receipt, _) = db.dispatch(&Command {
            id: CommandId("start".into()),
            thread_id: thread.clone(),
            action: Action::Send {
                text: "review".into(),
            },
        })?;
        Ok((db, thread, receipt.run_id.unwrap()))
    }

    fn delegate_review(db: &mut Database, token: &str) -> Result<DelegatedTask> {
        let (task, _) = db.delegate(
            token,
            serde_json::from_value(serde_json::json!({
                "clientRequestId": "delegate", "task": "review"
            }))?,
        )?;
        Ok(task)
    }

    fn finalize(db: &mut Database, thread: &ThreadId, run: &RunId, answer: &str) -> Result<()> {
        db.claim(run)?;
        db.update(thread, run, Update::Started(None))?;
        db.update(thread, run, Update::Finalizing(answer.into()))
    }

    #[test]
    fn stop_thread_preserves_committed_and_disposed_followup_results() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut db, parent, parent_run) = start_reviewer(dir.path())?;
        let token = db.grant(&parent_run)?;
        let original = delegate_review(&mut db, &token)?;
        let child = &original.child_thread_id;
        finalize(&mut db, child, &original.child_run_id, "Original review")?;
        db.finish(child, &original.child_run_id, Ok(()))?;
        assert!(db.ready_jobs()?.is_empty());
        db.task_status(&token, &original.task_id)?;

        let (first, _) = db.send_peer(&token, child, "first", "First revision")?;
        let (second, _) = db.send_peer(&token, child, "second", "Second revision")?;
        finalize(&mut db, child, &first.run_id, "First answer")?;
        db.dispatch(&Command {
            id: CommandId("stop".into()),
            thread_id: parent.clone(),
            action: Action::StopThread,
        })?;
        db.finish(child, &first.run_id, Err(Error::Interrupted))?;
        db.finish(&parent, &parent_run, Err(Error::Interrupted))?;
        assert!(db.ready_jobs()?.is_empty());
        let tasks = db.read(&parent)?.tasks;
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].delivery, CompletionDelivery::Acknowledged);
        assert_eq!(Some(&tasks[1].task_id), first.task_id.as_ref());
        assert_eq!(tasks[1].delivery, CompletionDelivery::Disposed);
        assert_eq!(tasks[1].status, RunStatus::Completed);
        assert_eq!(tasks[1].result.as_deref(), Some("First answer"));
        assert_eq!(Some(&tasks[2].task_id), second.task_id.as_ref());
        assert_eq!(tasks[2].delivery, CompletionDelivery::Disposed);
        assert_eq!(tasks[2].status, RunStatus::Interrupted);
        assert_eq!(tasks[2].result_run_id.as_ref(), Some(&second.run_id));
        assert!(db.find_run(child, &second.run_id)?.queue_held);

        let (_, work) = db.dispatch(&Command {
            id: CommandId("resume".into()),
            thread_id: child.clone(),
            action: Action::ResumeQueue,
        })?;
        assert_eq!(work.start, [(child.clone(), second.run_id.clone())]);
        finalize(&mut db, child, &second.run_id, "Second answer")?;
        db.finish(child, &second.run_id, Ok(()))?;
        assert!(db.ready_jobs()?.is_empty());
        assert_eq!(
            db.read(child)?.messages.last().unwrap().text,
            "Second answer"
        );
        assert_eq!(db.read(&parent)?.tasks, tasks);
        Ok(())
    }

    #[test]
    fn stop_thread_delivers_to_an_idle_owner_after_disposing_held_work() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut db, root, root_run) = start_reviewer(dir.path())?;
        let root_token = db.grant(&root_run)?;
        let middle = delegate_review(&mut db, &root_token)?;
        let middle_token = db.grant(&middle.child_run_id)?;
        let leaf = delegate_review(&mut db, &middle_token)?;
        let child = &leaf.child_thread_id;
        finalize(&mut db, child, &leaf.child_run_id, "Reviewed")?;
        db.finish(child, &leaf.child_run_id, Ok(()))?;
        assert!(db.ready_jobs()?.is_empty());
        db.task_status(&middle_token, &leaf.task_id)?;

        let (active, _) = db.send_peer(&root_token, child, "peer", "Peer message")?;
        let (held, _) = db.send_peer(&middle_token, child, "followup", "Revise")?;
        let stop = |id: &str, thread: &ThreadId| Command {
            id: CommandId(id.into()),
            thread_id: thread.clone(),
            action: Action::StopThread,
        };
        db.dispatch(&stop("stop leaf", child))?;
        db.finish(child, &active.run_id, Err(Error::Interrupted))?;
        let parent = &middle.child_thread_id;
        finalize(&mut db, parent, &middle.child_run_id, "Report")?;
        db.finish(parent, &middle.child_run_id, Ok(()))?;
        finalize(&mut db, &root, &root_run, "Waiting for report")?;
        db.finish(&root, &root_run, Ok(()))?;
        assert!(db.ready_jobs()?.is_empty());
        assert!(db.read(&root)?.tasks[0].result_run_id.is_none());

        // Only the held follow-up prevents delivery. No worker remains to
        // trigger another scheduling pass after this stop commits.
        let (_, work) = db.dispatch(&stop("stop middle", parent))?;
        assert!(work.stop.is_empty());
        let task = &db.read(&root)?.tasks[0];
        assert_eq!(task.delivery, CompletionDelivery::Delivered);
        assert_eq!(task.result.as_deref(), Some("Report"));
        let wake = task.wake_run_id.clone().unwrap();
        assert_eq!(work.start, [(root, wake)]);
        assert!(db.find_run(child, &held.run_id)?.queue_held);
        Ok(())
    }

    #[test]
    fn stop_thread_rejects_stale_completion_and_keeps_the_next_run_in_order() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut db, parent, parent_run) = start_reviewer(dir.path())?;
        let token = db.grant(&parent_run)?;
        let task = delegate_review(&mut db, &token)?;
        let target = &task.child_thread_id;
        let first = &task.child_run_id;
        let command = |id: &str, action| Command {
            id: CommandId(id.into()),
            thread_id: target.clone(),
            action,
        };
        let (second, _) = db.send_peer(&token, target, "second", "First revision")?;
        let (_, work) = db.dispatch(&command("stop", Action::StopThread))?;
        assert_eq!(work.stop, std::slice::from_ref(first));
        assert!(matches!(db.claim(first), Err(Error::Interrupted)));
        let (third, _) = db.send_peer(&token, target, "third", "Second revision")?;
        assert!(db.find_run(target, &third.run_id)?.queue_held);
        db.finish(target, first, Err(Error::Interrupted))?;
        assert!(db.ready_jobs()?.is_empty());
        let stopped = db.find_run(target, first)?;
        assert_eq!(stopped.status, RunStatus::Interrupted);
        let result = &db.read(&parent)?.tasks[0];
        assert_eq!(result.result_run_id.as_ref(), Some(first));
        assert_eq!(result.delivery, CompletionDelivery::Pending);

        let (nudge, _) = db.send_peer(&token, target, "nudge", "New request")?;
        assert_eq!(nudge.delivery, PeerDelivery::Started);
        db.claim(&nudge.run_id)?;
        db.update(target, &nudge.run_id, Update::Started(None))?;
        db.update(target, &nudge.run_id, Update::Finalizing("Reply".into()))?;
        db.finish(target, &nudge.run_id, Ok(()))?;
        assert!(db.ready_jobs()?.is_empty());
        assert!(db.find_run(target, &second.run_id)?.queue_held);

        let (_, work) = db.dispatch(&command("resume", Action::ResumeQueue))?;
        assert_eq!(work.start, [(target.clone(), second.run_id.clone())]);
        db.claim(&second.run_id)?;
        db.update(target, &second.run_id, Update::Started(None))?;
        let cursor = db.read(target)?.cursor;
        for update in [
            Update::Started(None),
            Update::Finalizing("late answer".into()),
            Update::ForegroundComplete,
        ] {
            assert!(matches!(
                db.update(target, first, update),
                Err(Error::Interrupted)
            ));
        }
        db.finish(target, first, Ok(()))?;
        assert!(db.ready_jobs()?.is_empty());
        assert_eq!(db.read(target)?.cursor, cursor);
        assert_eq!(db.find_run(target, first)?, stopped);
        assert_eq!(
            db.find_run(target, &second.run_id)?.status,
            RunStatus::Running
        );
        assert_eq!(
            db.find_run(target, &third.run_id)?.status,
            RunStatus::Queued
        );

        db.update(
            target,
            &second.run_id,
            Update::Finalizing("Revision".into()),
        )?;
        db.finish(target, &second.run_id, Ok(()))?;
        assert_eq!(db.ready_jobs()?, [(target.clone(), third.run_id)]);
        Ok(())
    }

    #[test]
    fn committed_interrupt_survives_worker_loss_and_closes_pending_approval() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut db, thread, run) = start_reviewer(dir.path())?;
        db.claim(&run)?;
        db.update(&thread, &run, Update::Started(None))?;
        let request = RequestId("approval".into());
        db.request_permission(
            &thread,
            PermissionRequest {
                id: request.clone(),
                run_id: run.clone(),
                native_child: None,
                tool_call: serde_json::json!({}),
                options: vec![PermissionOption {
                    option_id: "allow".into(),
                    name: "Allow".into(),
                    kind: "allow_once".into(),
                }],
            },
        )?;
        let token = db.grant(&run)?;
        assert_eq!(db.interrupt_run(&token, &thread, &run, "stop", None)?, None);
        assert!(matches!(db.caller(&token), Err(Error::Forbidden)));
        assert!(matches!(
            db.decide(&thread, &run, &request, "allow"),
            Err(Error::NotFound(_))
        ));
        // Lose the worker between committing the request and finishing the turn.
        drop(db);
        let reopened = Database::open(dir.path())?;
        let snapshot = reopened.read(&thread)?;
        assert!(snapshot.permissions.is_empty());
        assert_eq!(snapshot.runs[0].status, RunStatus::Interrupted);
        assert_eq!(
            snapshot.runs[0].error.as_deref(),
            Some("Stopped by an agent")
        );
        Ok(())
    }
}
