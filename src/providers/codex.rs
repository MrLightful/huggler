//! Codex app-server over newline-delimited JSON-RPC on a managed child's stdio.
mod children;
use super::{
    Permissions, Turn, Update,
    rpc::{Rpc, protocol},
};
use crate::{
    Error, NativeChildTurn, NativeThreadId, NativeTurnId, PermissionOption, PermissionRequest,
    RequestId, Result, UserAnswer, UserAnswers, UserInputRequest,
};
use children::Children;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
};
use tokio::sync::{mpsc, oneshot, watch};

pub(crate) struct ChildInterrupt {
    pub thread: NativeThreadId,
    pub turn: NativeTurnId,
    pub reply: oneshot::Sender<Result<()>>,
}

pub(crate) enum Control {
    Interrupt(ChildInterrupt),
    Answer {
        id: RequestId,
        answers: UserAnswers,
        reply: oneshot::Sender<Result<()>>,
    },
}

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

enum Approval {
    Command,
    Permissions(Value),
    Mcp,
}

struct PendingApproval {
    wire_id: Value,
    approval: Approval,
    child: Option<NativeChildTurn>,
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
                            | "item/tool/requestUserInput"
                            | "item/permissions/requestApproval"
                            | "mcpServer/elicitation/request"
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
                        "item/started"
                            | "item/completed"
                            | "turn/started"
                            | "turn/completed"
                            | "item/commandExecution/requestApproval"
                            | "item/tool/requestUserInput"
                            | "serverRequest/resolved"
                            | "item/permissions/requestApproval"
                            | "mcpServer/elicitation/request"
                    )
                )
            {
                self.early.push_back(frame);
            }
        }
    }

    async fn write_answers(&mut self, wire_id: Value, answers: UserAnswers) -> Result<()> {
        let answers = answers
            .into_iter()
            .map(|(id, answer)| {
                let values = match answer {
                    UserAnswer::Text(text) => vec![text],
                    UserAnswer::Choices(choices) => choices,
                };
                (id, json!({"answers": values}))
            })
            .collect::<serde_json::Map<_, _>>();
        self.rpc
            .write(json!({"id": wire_id, "result": {"answers": answers}}))
            .await
    }

    async fn interrupt_child(
        &mut self,
        request: ChildInterrupt,
        children: &Children,
        replies: &mut HashMap<u64, oneshot::Sender<Result<()>>>,
    ) -> Result<()> {
        if !children.running(&request.thread.0, &request.turn.0) {
            let _ = request
                .reply
                .send(Err(Error::NotFound("running native child turn".into())));
            return Ok(());
        }
        let id = self.next_id;
        self.next_id += 1;
        self.rpc
            .write(json!({"id": id, "method": "turn/interrupt", "params": {
                "threadId": request.thread, "turnId": request.turn
            }}))
            .await?;
        replies.insert(id, request.reply);
        Ok(())
    }

    async fn converse(
        &mut self,
        turn: Turn,
        update: impl Fn(Update) -> Result<()>,
        request_approvals: bool,
        mut permissions: Permissions<impl Fn(PermissionRequest) -> Result<()>>,
        mut controls: mpsc::UnboundedReceiver<Control>,
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
        if let Some(tools) = &turn.agent_tools {
            session["config"]["mcp_servers.huggler"] = json!({
                "url": tools.url, "bearer_token_env_var": "HUGGLER_MCP_TOKEN",
                "required": true
            });
        }
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
            let mode = if turn.plan_mode { "plan" } else { "default" };
            params["collaborationMode"] = json!({"mode":mode,"settings":{"model":model}});
        } else if turn.plan_mode {
            return Err(protocol("Codex plan mode requires a model"));
        }
        let started = self.request("turn/start", params).await?;
        let id = started
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol("missing native turn ID"))?;
        update(Update::Started(Some(NativeTurnId(id.to_owned()))))?;
        let mut answer = None;
        let mut pending = HashMap::<RequestId, PendingApproval>::new();
        let mut questions = HashMap::<RequestId, Value>::new();
        let mut children = Children::default();
        let mut foreground_complete = false;
        let mut interrupt_replies = HashMap::<u64, oneshot::Sender<Result<()>>>::new();
        loop {
            if foreground_complete && !children.active() && interrupt_replies.is_empty() {
                return Ok(());
            }
            let frame = match self.early.pop_front() {
                Some(frame) => frame,
                None => tokio::select! {
                    frame = self.read() => frame?,
                    control = controls.recv() => {
                        match control.ok_or(Error::Closed)? {
                            Control::Interrupt(request) => {
                                self.interrupt_child(request, &children, &mut interrupt_replies).await?;
                            }
                            Control::Answer { id, answers, reply } => {
                                if let Some(wire_id) = questions.remove(&id) {
                                    self.write_answers(wire_id, answers).await?;
                                    update(Update::QuestionClosed { id, answered: true })?;
                                    let _ = reply.send(Ok(()));
                                } else {
                                    let _ = reply.send(Err(Error::NotFound("live user-input request".into())));
                                }
                            }
                        }
                        continue;
                    }
                    decision = permissions.decisions.recv() => {
                        let (request,option) = decision.ok_or(Error::Closed)?;
                        // A turn can end while a host decision is queued.
                        let Some(PendingApproval { wire_id, approval, .. }) = pending.remove(&request) else {
                            continue;
                        };
                        let result = match approval {
                            Approval::Permissions(profile) => json!({"permissions":if option=="accept" {profile} else {json!({})},"scope":"turn"}),
                            Approval::Command => json!({"decision":option}),
                            Approval::Mcp => json!({"action":option,"content":if option=="accept" {json!({})} else {Value::Null}}),
                        };
                        self.rpc.write(json!({"id":wire_id,"result":result})).await?;
                        continue;
                    }
                },
            };
            let p = &frame["params"];
            if let Some(wire_id) = frame.get("id") {
                if frame.get("method").is_none() {
                    if let Some(reply) = wire_id
                        .as_u64()
                        .and_then(|id| interrupt_replies.remove(&id))
                    {
                        let result = if let Some(error) = frame.get("error") {
                            Err(protocol(format!("turn/interrupt: {error}")))
                        } else if frame.get("result").is_some() {
                            Ok(())
                        } else {
                            Err(protocol("missing turn/interrupt result"))
                        };
                        let _ = reply.send(result);
                    }
                    continue;
                }
                let child = p["threadId"]
                    .as_str()
                    .zip(p["turnId"].as_str())
                    .filter(|(t, u)| children.running(t, u))
                    .map(|(t, u)| NativeChildTurn {
                        thread_id: NativeThreadId(t.into()),
                        turn_id: NativeTurnId(u.into()),
                    });
                let root_request =
                    !foreground_complete && p["threadId"] == thread && p["turnId"] == id;
                if frame["method"] == "item/tool/requestUserInput" && root_request {
                    let Some(items) = p["questions"].as_array() else {
                        self.rpc.write(json!({"id":wire_id,"error":{"code":-32602,"message":"Missing questions"}})).await?;
                        continue;
                    };
                    let ids: HashSet<_> = items
                        .iter()
                        .filter_map(|item| item["id"].as_str().filter(|id| !id.is_empty()))
                        .collect();
                    if items.is_empty()
                        || ids.len() != items.len()
                        || questions.values().any(|id| id == wire_id)
                    {
                        self.rpc.write(json!({"id":wire_id,"error":{"code":-32602,"message":"Invalid question IDs"}})).await?;
                        continue;
                    }
                    let id = RequestId(uuid::Uuid::new_v4().to_string());
                    questions.insert(id.clone(), wire_id.clone());
                    update(Update::Question(UserInputRequest {
                        id,
                        run_id: permissions.run.clone(),
                        questions: items.clone(),
                    }))?;
                    continue;
                }
                if !request_approvals || (!root_request && child.is_none()) {
                    self.rpc.write(json!({"id":wire_id,"error":{"code":-32602,"message":"No matching interactive run"}})).await?;
                    continue;
                }
                let (approval, title) = match frame["method"].as_str() {
                    Some("item/permissions/requestApproval") => (
                        Approval::Permissions(p["permissions"].clone()),
                        "Grant permissions for this turn",
                    ),
                    Some("mcpServer/elicitation/request")
                        if turn.agent_tools.is_some()
                            && p["serverName"] == "huggler"
                            && p["mode"] == "form"
                            && p["_meta"]["codex_approval_kind"] == "mcp_tool_call" =>
                    {
                        (
                            Approval::Mcp,
                            p["message"].as_str().unwrap_or("Use Huggler agent tool"),
                        )
                    }
                    Some("item/commandExecution/requestApproval") => {
                        (Approval::Command, "Run command outside the sandbox")
                    }
                    _ => {
                        self.rpc.write(json!({"id":wire_id,"error":{"code":-32601,"message":"Unsupported host request"}})).await?;
                        continue;
                    }
                };
                let request = RequestId(uuid::Uuid::new_v4().to_string());
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
                pending.insert(
                    request.clone(),
                    PendingApproval {
                        wire_id: wire_id.clone(),
                        approval,
                        child: child.clone(),
                    },
                );
                (permissions.publish)(PermissionRequest {
                    id: request,
                    run_id: permissions.run.clone(),
                    native_child: child,
                    tool_call: json!({"title":title,"rawInput":p}),
                    options,
                })?;
                continue;
            }
            if let Some(child) = children.observe(thread, id, &permissions.run, &frame) {
                update(Update::NativeChild(child))?;
                pending.retain(|_, request| {
                    request
                        .child
                        .as_ref()
                        .is_none_or(|owner| children.running(&owner.thread_id.0, &owner.turn_id.0))
                });
            }
            if foreground_complete || p["threadId"] != thread {
                continue;
            }
            match frame["method"].as_str() {
                Some("serverRequest/resolved") => {
                    if let Some(request) = questions
                        .iter()
                        .find(|(_, wire)| **wire == p["requestId"])
                        .map(|(id, _)| id.clone())
                    {
                        questions.remove(&request);
                        update(Update::QuestionClosed {
                            id: request,
                            answered: false,
                        })?;
                    }
                }
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
                    let text = answer
                        .take()
                        .ok_or_else(|| protocol("completed turn has no final answer"))?;
                    update(Update::Finalizing(text))?;
                    if !children.active() && interrupt_replies.is_empty() {
                        return Ok(());
                    }
                    update(Update::ForegroundComplete)?;
                    foreground_complete = true;
                    questions.clear();
                    pending.retain(|_, request| request.child.is_some());
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
    controls: mpsc::UnboundedReceiver<Control>,
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
        answer = client.converse(turn, update,options.request_approvals,permissions,controls) => answer,
    };
    client.rpc.shutdown().await?;
    result
}
