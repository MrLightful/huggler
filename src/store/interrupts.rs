use super::*;

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
        let message = reason.map_or_else(
            || "Stopped by an agent".into(),
            |s| format!("Stopped by an agent: {s}"),
        );
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

    #[test]
    fn committed_interrupt_survives_worker_loss_and_closes_pending_approval() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut db = Database::open(dir.path())?;
        let project = db.project(dir.path())?;
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
        let run = receipt.run_id.unwrap();
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
