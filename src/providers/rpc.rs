use super::Turn;
use crate::{Error, Result};
use serde_json::Value;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};
pub(super) struct Rpc {
    child: Child,
    input: Option<ChildStdin>,
    output: Lines<BufReader<ChildStdout>>,
}

pub(super) fn protocol(message: impl Into<String>) -> Error {
    Error::Provider(message.into())
}

impl Rpc {
    pub fn spawn(executable: &Path, args: &[&str], turn: &Turn) -> Result<Self> {
        let mut command = Command::new(executable);
        command
            .args(args)
            .current_dir(&turn.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if turn.provider == crate::Provider::Codex
            && let Some(tools) = &turn.agent_tools
        {
            command.env("HUGGLER_MCP_TOKEN", &tools.token);
        }
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| protocol(format!("launch {}: {e}", executable.display())))?;
        Ok(Self {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
        })
    }

    pub async fn write(&mut self, frame: Value) -> Result<()> {
        let mut line = serde_json::to_vec(&frame)?;
        line.push(b'\n');
        self.input.as_mut().unwrap().write_all(&line).await?;
        Ok(())
    }

    pub async fn read(&mut self) -> Result<Value> {
        let line = self
            .output
            .next_line()
            .await?
            .ok_or_else(|| protocol("provider stdout closed before turn completion"))?;
        serde_json::from_str(&line).map_err(|e| protocol(format!("invalid JSON-RPC frame: {e}")))
    }

    #[cfg(unix)]
    fn kill_process_group(&self) {
        if let Some(pid) = self.child.id() {
            // spawn() gives this unreaped child its own process group.
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
    }

    pub async fn finish(&mut self, outcome: Result<()>) -> Result<()> {
        // On a host stop, a launcher can exit on EOF and leave its helpers running.
        #[cfg(unix)]
        if matches!(outcome, Err(Error::Closed)) {
            self.kill_process_group();
        }
        self.input.take();
        match timeout(Duration::from_secs(2), self.child.wait()).await {
            Ok(status) => {
                status?;
            }
            Err(_) => {
                #[cfg(unix)]
                self.kill_process_group();
                self.child.kill().await?;
            }
        }
        outcome
    }
}

#[cfg(unix)]
impl Drop for Rpc {
    fn drop(&mut self) {
        // The executor can drop a worker before it finishes shutting down.
        self.kill_process_group();
    }
}
