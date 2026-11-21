use super::*;
use crate::workspace::SetupStatus;
use std::process::Stdio;

impl Huggler {
    /// Prepare the project's checkout with an explicit host command. For an isolated
    /// thread, register its worktree path with `open_project` and use that project ID.
    /// An active run in the checkout blocks setup. Failed setup gates new runs there
    /// until a successful retry. Shutdown stops an active command before releasing the database.
    pub async fn setup_workspace(
        &self,
        project: &ProjectId,
        program: &Path,
        args: &[String],
    ) -> Result<SetupStatus> {
        let (send, receive) = tokio::sync::oneshot::channel();
        {
            let shared = self.shared.clone();
            let mut gate = self.shared.workers.lock().unwrap();
            let workers = gate.as_mut().ok_or(Error::Closed)?;
            let root = shared.access(|db| db.begin_setup(project))?;
            let mut command = tokio::process::Command::new(program);
            command
                .args(args)
                .current_dir(root)
                .kill_on_drop(true)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(unix)]
            command.process_group(0);

            let mut stop = shared.stop.subscribe();
            let project = project.clone();
            workers.spawn_on(
                async move {
                    let failure = match command.spawn() {
                        Err(err) => Some(err.to_string()),
                        Ok(mut child) => tokio::select! {
                            result = child.wait() => match result {
                                Ok(status) if status.success() => None,
                                Ok(status) => Some(status.to_string()),
                                Err(err) => Some(err.to_string()),
                            },
                            _ = stop.changed() => {
                                #[cfg(unix)]
                                if let Some(id) = child.id() {
                                    // The child owns this group and has not been reaped.
                                    unsafe { libc::kill(-(id as i32), libc::SIGKILL); }
                                }
                                let _ = child.kill().await;
                                Some("runtime closed during setup".into())
                            }
                        },
                    };
                    // Shutdown keeps the database until all registered workers finish.
                    let result = shared
                        .db
                        .lock()
                        .unwrap()
                        .as_mut()
                        .ok_or(Error::Closed)
                        .and_then(|db| db.end_setup(&project, failure));
                    let _ = send.send(result);
                },
                &self.shared.handle,
            );
        }
        receive.await.map_err(|_| Error::Closed)?
    }
}
