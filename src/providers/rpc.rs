use super::Turn;
use super::registry::ProviderInstance;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::path::Path;
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};
pub(super) struct Rpc {
    text: std::sync::Arc<dyn crate::AgentText>,
    child: Child,
    input: Option<ChildStdin>,
    output: Option<Lines<BufReader<ChildStdout>>>,
}

pub(super) fn protocol(message: impl Into<String>) -> Error {
    Error::Provider(message.into())
}

impl Rpc {
    pub fn spawn(instance: &ProviderInstance, args: &[&str], turn: &Turn) -> Result<Self> {
        let mut command = instance.command();
        command.args(args);
        if turn.provider == crate::Provider::Codex
            && let Some(tools) = &turn.agent_tools
        {
            command.env("HUGGLER_MCP_TOKEN", &tools.token);
        }
        Self::open(command, &turn.cwd, &turn.output)
    }

    pub fn open(
        command: std::process::Command,
        cwd: &Path,
        output: &crate::AgentOutput,
    ) -> Result<Self> {
        let mut command = Command::from(command);
        command
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::MissingExecutable(
                    command
                        .as_std()
                        .get_program()
                        .to_string_lossy()
                        .into_owned(),
                )
            } else {
                Error::Io(e)
            }
        })?;
        Ok(Self {
            text: output.text.clone(),
            input: child.stdin.take(),
            output: Some(BufReader::new(child.stdout.take().unwrap()).lines()),
            child,
        })
    }

    pub fn error(&self, code: i32, message: &str) -> Value {
        json!({"code": code, "message": self.text.label(message)})
    }

    /// A setup request. Reject unsupported callbacks while waiting for the matching response.
    pub async fn request_frame(&mut self, id: u64, method: &str, params: Value) -> Result<Value> {
        self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        loop {
            let frame = self.read().await?;
            if frame.get("method").is_some() {
                if let Some(wire) = frame.get("id") {
                    self.write(json!({"jsonrpc":"2.0","id":wire,"error":self.error(-32601, "Unsupported client request")})).await?;
                }
                continue;
            }
            if frame["id"] != id {
                return Err(protocol(format!("unexpected reply to {method}")));
            }
            return Ok(frame);
        }
    }

    pub async fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value> {
        let frame = self.request_frame(id, method, params).await?;
        if let Some(error) = frame.get("error") {
            return Err(protocol(format!(
                "{method}: {}",
                error["message"].as_str().unwrap_or("request failed")
            )));
        }
        frame
            .get("result")
            .cloned()
            .ok_or_else(|| protocol(format!("missing result for {method}")))
    }

    pub async fn write(&mut self, frame: Value) -> Result<()> {
        let mut line = serde_json::to_vec(&frame)?;
        line.push(b'\n');
        self.input.as_mut().unwrap().write_all(&line).await?;
        Ok(())
    }

    pub fn drain_stdout(&mut self) {
        let mut lines = self.output.take().expect("stdout already drained");
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    }

    pub async fn line(&mut self) -> Result<String> {
        self.output
            .as_mut()
            .expect("stdout already drained")
            .next_line()
            .await?
            .ok_or_else(|| protocol("provider stdout closed before turn completion"))
    }
    pub async fn read(&mut self) -> Result<Value> {
        let line = self.line().await?;
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
        // On failure, stop helpers while the unreaped launcher still owns its PID.
        #[cfg(unix)]
        if outcome.is_err() {
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
