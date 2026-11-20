//! Codex app-server over newline-delimited JSON-RPC on a managed child's stdio.
use crate::{Error, NativeThreadId, NativeTurnId, Result};
use serde_json::{Value, json};
use std::{collections::VecDeque, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::watch,
    time::timeout,
};

#[derive(Clone, Debug)]
pub struct CodexOptions {
    pub executable: PathBuf,
}

impl Default for CodexOptions {
    fn default() -> Self {
        Self {
            executable: "codex".into(),
        }
    }
}

pub(crate) struct Turn {
    pub cwd: PathBuf,
    pub name: String,
    pub text: String,
    pub model: Option<String>,
    pub resume: Option<NativeThreadId>,
}

pub(crate) enum Update {
    Bound(NativeThreadId),
    Started(NativeTurnId),
    Finalizing(String),
}

struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    early: VecDeque<Value>,
}

fn protocol(message: impl Into<String>) -> Error {
    Error::Provider(message.into())
}

impl Client {
    fn spawn(options: &CodexOptions, turn: &Turn) -> Result<Self> {
        let mut command = Command::new(&options.executable);
        command
            .arg("app-server")
            .current_dir(&turn.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| protocol(format!("launch {}: {e}", options.executable.display())))?;
        Ok(Self {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
            next_id: 1,
            early: VecDeque::new(),
        })
    }

    async fn write(&mut self, frame: Value) -> Result<()> {
        let mut line = serde_json::to_vec(&frame)?;
        line.push(b'\n');
        self.input.as_mut().unwrap().write_all(&line).await?;
        Ok(())
    }

    async fn read(&mut self) -> Result<Value> {
        loop {
            let line = self
                .output
                .next_line()
                .await?
                .ok_or_else(|| protocol("app-server stdout closed before turn completion"))?;
            let frame: Value = serde_json::from_str(&line)
                .map_err(|e| protocol(format!("invalid JSON-RPC frame: {e}")))?;
            if frame.get("method").is_some() && frame.get("id").is_some() {
                self.write(json!({"id":frame["id"],"error":{"code":-32601,"message":"Host request unsupported"}})).await?;
                continue;
            }
            return Ok(frame);
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({"id":id,"method":method,"params":params}))
            .await?;
        loop {
            let frame = self.read().await?;
            if frame.get("id").is_some() {
                if frame["id"] != id {
                    return Err(protocol(format!("unexpected reply to {method}")));
                }
                if let Some(error) = frame.get("error") {
                    return Err(protocol(format!("{method}: {error}")));
                }
                return frame
                    .get("result")
                    .cloned()
                    .ok_or_else(|| protocol(format!("missing {method} result")));
            }
            if method == "turn/start"
                && matches!(
                    frame["method"].as_str(),
                    Some("item/completed" | "turn/completed")
                )
            {
                self.early.push_back(frame);
            }
        }
    }

    async fn converse(&mut self, turn: Turn, update: impl Fn(Update) -> Result<()>) -> Result<()> {
        self.request("initialize", json!({
            "clientInfo":{"name":"huggler","title":"Huggler","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true,"optOutNotificationMethods":["turn/diff/updated"]}
        })).await?;
        self.write(json!({"method":"initialized"})).await?;
        let read_config = json!({"cwd":turn.cwd,"includeLayers":false});
        let config = self.request("config/read", read_config.clone()).await?;
        let model = turn
            .model
            .as_deref()
            .or_else(|| config.pointer("/config/model").and_then(Value::as_str));
        let mut session = json!({"cwd":turn.cwd, "config":{
            "tools.update_plan.enabled":true, "mcp_optional_startup_grace_ms":10000,
            "features.concurrent_reasoning_summaries":true
        }});
        if let Some(model) = model {
            session["model"] = json!(model);
        }
        let method = match &turn.resume {
            Some(id) => {
                session["threadId"] = json!(id);
                session["excludeTurns"] = json!(true);
                "thread/resume"
            }
            None => "thread/start",
        };
        let response = self.request(method, session).await?;
        let thread = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol("missing native thread ID"))?;
        if turn.resume.as_ref().is_some_and(|id| id.0 != thread) {
            return Err(protocol("resume returned a different native thread"));
        }
        update(Update::Bound(NativeThreadId(thread.to_owned())))?;
        self.request(
            "thread/name/set",
            json!({"threadId":thread,"name":turn.name}),
        )
        .await?;
        self.request("config/read", read_config).await?;
        let mut params = json!({"threadId":thread, "cwd":turn.cwd, "input":[{"type":"text","text":turn.text}],
            "approvalPolicy":"never","approvalsReviewer":"user","sandboxPolicy":{"type":"readOnly"},"summary":"auto"});
        if let Some(model) = model.or_else(|| response["model"].as_str()) {
            params["model"] = json!(model);
            params["collaborationMode"] = json!({"mode":"default","settings":{"model":model}});
        }
        let started = self.request("turn/start", params).await?;
        let id = started
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol("missing native turn ID"))?;
        update(Update::Started(NativeTurnId(id.to_owned())))?;
        let mut answer = None;
        loop {
            let frame = match self.early.pop_front() {
                Some(frame) => frame,
                None => self.read().await?,
            };
            let p = &frame["params"];
            if p["threadId"] != thread {
                continue;
            }
            match frame["method"].as_str() {
                Some("item/completed")
                    if p["turnId"] == id
                        && p["item"]["type"] == "agentMessage"
                        && p["item"]["phase"] != "commentary" =>
                {
                    answer = p["item"]["text"].as_str().map(str::to_owned);
                }
                Some("turn/completed") if p["turn"]["id"] == id => {
                    if p["turn"]["status"] != "completed" {
                        return Err(protocol(format!(
                            "turn {}: {}",
                            p["turn"]["status"], p["turn"]["error"]
                        )));
                    }
                    let text =
                        answer.ok_or_else(|| protocol("completed turn has no final answer"))?;
                    return update(Update::Finalizing(text));
                }
                _ => {}
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.input.take();
        match timeout(Duration::from_secs(2), self.child.wait()).await {
            Ok(status) => {
                status?;
            }
            Err(_) => {
                #[cfg(unix)]
                if let Some(pid) = self.child.id() {
                    // spawn() gives the managed app-server its own process group.
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                self.child.kill().await?;
            }
        }
        Ok(())
    }
}

pub(crate) async fn execute(
    options: CodexOptions,
    turn: Turn,
    mut stop: watch::Receiver<bool>,
    update: impl Fn(Update) -> Result<()>,
) -> Result<()> {
    if *stop.borrow() {
        return Err(Error::Closed);
    }
    let mut client = Client::spawn(&options, &turn)?;
    let result = tokio::select! {
        biased;
        _ = stop.wait_for(|v| *v) => Err(Error::Closed),
        answer = client.converse(turn, update) => answer,
    };
    client.shutdown().await?;
    result
}
