//! Codex app-server over newline-delimited JSON-RPC on a managed child's stdio.
use super::{
    Permissions, Turn, Update,
    rpc::{Rpc, protocol},
};
use crate::{
    Error, NativeThreadId, NativeTurnId, PermissionOption, PermissionRequest, RequestId, Result,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
};
use tokio::sync::watch;

#[derive(Clone, Debug)]
pub struct CodexOptions {
    pub executable: PathBuf,
    /// Route explicit command or permission requests to the host.
    pub request_approvals: bool,
}

impl Default for CodexOptions {
    fn default() -> Self {
        Self {
            executable: "codex".into(),
            request_approvals: false,
        }
    }
}

struct Client {
    rpc: Rpc,
    next_id: u64,
    early: VecDeque<Value>,
}

impl Client {
    async fn read(&mut self) -> Result<Value> {
        loop {
            let frame = self.rpc.read().await?;
            if frame.get("method").is_some() && frame.get("id").is_some() {
                if matches!(
                    frame["method"].as_str(),
                    Some(
                        "item/commandExecution/requestApproval"
                            | "item/permissions/requestApproval"
                    )
                ) {
                    return Ok(frame);
                }
                self.rpc.write(json!({"id":frame["id"],"error":{"code":-32601,"message":"Host request unsupported"}})).await?;
                continue;
            }
            return Ok(frame);
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.rpc
            .write(json!({"id":id,"method":method,"params":params}))
            .await?;
        loop {
            let frame = self.read().await?;
            if frame.get("id").is_some() && frame.get("method").is_none() {
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
                    Some(
                        "item/completed"
                            | "turn/completed"
                            | "item/commandExecution/requestApproval"
                            | "item/permissions/requestApproval"
                    )
                )
            {
                self.early.push_back(frame);
            }
        }
    }

    async fn converse(
        &mut self,
        turn: Turn,
        update: impl Fn(Update) -> Result<()>,
        request_approvals: bool,
        mut permissions: Permissions<impl Fn(PermissionRequest) -> Result<()>>,
    ) -> Result<()> {
        self.request("initialize", json!({
            "clientInfo":{"name":"huggler","title":"Huggler","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true,"optOutNotificationMethods":["turn/diff/updated"]}
        })).await?;
        self.rpc.write(json!({"method":"initialized"})).await?;
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
        if request_approvals {
            session["approvalPolicy"] = json!("on-request");
            session["sandbox"] = json!("read-only");
        }
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
        if request_approvals {
            params["approvalPolicy"] = json!("on-request");
        }
        if let Some(model) = model.or_else(|| response["model"].as_str()) {
            params["model"] = json!(model);
            params["collaborationMode"] = json!({"mode":"default","settings":{"model":model}});
        }
        let started = self.request("turn/start", params).await?;
        let id = started
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol("missing native turn ID"))?;
        update(Update::Started(Some(NativeTurnId(id.to_owned()))))?;
        let mut answer = None;
        let mut pending = HashMap::new();
        loop {
            let frame = match self.early.pop_front() {
                Some(frame) => frame,
                None => tokio::select! {
                    frame = self.read() => frame?,
                    decision = permissions.decisions.recv() => {
                        let (request,option) = decision.ok_or(Error::Closed)?;
                        let (wire_id, profile): (Value,Option<Value>) = pending.remove(&request).ok_or_else(||protocol("expired approval"))?;
                        let result = match profile {
                            Some(profile) => json!({"permissions":if option=="accept" {profile} else {json!({})},"scope":"turn"}),
                            None => json!({"decision":option}),
                        };
                        self.rpc.write(json!({"id":wire_id,"result":result})).await?;
                        continue;
                    }
                },
            };
            let p = &frame["params"];
            if let Some(wire_id) = frame.get("id") {
                if !request_approvals || p["threadId"] != thread || p["turnId"] != id {
                    self.rpc.write(json!({"id":wire_id,"error":{"code":-32602,"message":"No matching interactive run"}})).await?;
                    continue;
                }
                let profile = (frame["method"] == "item/permissions/requestApproval")
                    .then(|| p["permissions"].clone());
                let request = RequestId(uuid::Uuid::new_v4().to_string());
                let title = if profile.is_some() {
                    "Grant permissions for this turn"
                } else {
                    "Run command outside the sandbox"
                };
                let options = [
                    ("accept", "Allow", "allow_once"),
                    ("decline", "Deny", "reject_once"),
                    ("cancel", "Deny and stop turn", "reject_once"),
                ]
                .into_iter()
                .filter(|(option, _, _)| {
                    p["availableDecisions"]
                        .as_array()
                        .map_or(*option != "cancel", |allowed| {
                            allowed.iter().any(|v| v == *option)
                        })
                })
                .map(|(id, name, kind)| PermissionOption {
                    option_id: id.into(),
                    name: name.into(),
                    kind: kind.into(),
                })
                .collect();
                pending.insert(request.clone(), (wire_id.clone(), profile));
                (permissions.publish)(PermissionRequest {
                    id: request,
                    run_id: permissions.run.clone(),
                    tool_call: json!({"title":title,"rawInput":p}),
                    options,
                })?;
                continue;
            }
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
                    if p["turn"]["status"] == "interrupted" {
                        return Err(Error::Interrupted);
                    }
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
}

pub(crate) async fn execute(
    options: CodexOptions,
    turn: Turn,
    mut stop: watch::Receiver<bool>,
    update: impl Fn(Update) -> Result<()>,
    permissions: Permissions<impl Fn(PermissionRequest) -> Result<()>>,
) -> Result<()> {
    if *stop.borrow() {
        return Err(Error::Closed);
    }
    let mut client = Client {
        rpc: Rpc::spawn(&options.executable, &["app-server"], &turn)?,
        next_id: 1,
        early: VecDeque::new(),
    };
    let result = tokio::select! {
        biased;
        _ = stop.wait_for(|v| *v) => Err(Error::Closed),
        answer = client.converse(turn, update,options.request_approvals,permissions) => answer,
    };
    client.rpc.shutdown().await?;
    result
}
