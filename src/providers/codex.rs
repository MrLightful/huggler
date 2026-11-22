//! Codex app-server over newline-delimited JSON-RPC on a managed child's stdio.
mod children;
pub(super) mod mcp_status;
use super::registry::ProviderInstance;
use super::{
    ChildInterrupt, CommandFrame, Control, Permissions, Turn, Update,
    rpc::{Rpc, protocol},
};
use crate::{
    ActivityItem, ActivityKind, ActivityStatus, Error, NativeChildTurn, NativeThreadId,
    NativeTurnId, PermissionOption, PermissionRequest, RequestId, Result, UserAnswer, UserAnswers,
    UserInputQuestion, UserInputRequest,
};

fn activity_item(item: &Value, completed: bool) -> Option<ActivityItem> {
    let field = |key: &str| item[key].as_str().unwrap_or_default();
    let (kind, title) = match field("type") {
        "commandExecution" => (ActivityKind::Command, field("command").into()),
        "fileChange" => {
            let paths = item["changes"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut title = paths
                .first()
                .and_then(|path| path["path"].as_str())
                .unwrap_or_default()
                .to_owned();
            if paths.len() > 1 {
                title.push_str(&format!(" (+{} more)", paths.len() - 1));
            }
            (ActivityKind::FileChange, title)
        }
        "mcpToolCall" => (
            ActivityKind::Tool,
            format!("{} / {}", field("server"), field("tool")),
        ),
        "dynamicToolCall" => (ActivityKind::Tool, field("tool").into()),
        "webSearch" => (ActivityKind::Search, field("query").into()),
        "imageView" => (
            ActivityKind::Tool,
            format!("View image · {}", field("path")),
        ),
        "imageGeneration" => (ActivityKind::Tool, "Generate image".into()),
        "collabAgentToolCall" => (ActivityKind::Agent, field("tool").into()),
        "agentMessage" if completed && item["phase"] == "commentary" => {
            (ActivityKind::Commentary, field("text").into())
        }
        "contextCompaction" if completed => (ActivityKind::Compaction, String::new()),
        _ => return None,
    };
    let status = match field("status") {
        "failed" | "declined" => ActivityStatus::Failed,
        "interrupted" | "cancelled" => ActivityStatus::Interrupted,
        _ if item["exitCode"].as_i64().is_some_and(|code| code != 0)
            || item["success"] == false =>
        {
            ActivityStatus::Failed
        }
        _ if completed => ActivityStatus::Completed,
        _ => ActivityStatus::Running,
    };
    Some(ActivityItem {
        id: item["id"].as_str()?.into(),
        kind,
        title,
        status,
        timestamp_ms: None,
    })
}
use children::Children;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone, Debug)]
pub struct CodexOptions {
    pub executable: PathBuf,
    /// Route explicit command or permission requests to the host.
    pub request_approvals: bool,
    /// Allow Codex to edit the turn checkout outside plan mode.
    pub workspace_write: bool,
}

impl Default for CodexOptions {
    fn default() -> Self {
        Self {
            executable: "codex".into(),
            request_approvals: false,
            workspace_write: false,
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

/// Every exhausted window must have reset before the next prompt is eligible.
fn quota_reset_at(limits: &Value) -> Option<i64> {
    let mut resets = Vec::new();
    for window in [&limits["primary"], &limits["secondary"]] {
        if window["usedPercent"].as_f64().is_some_and(|v| v >= 100.0) {
            resets.push(window["resetsAt"].as_i64()?);
        }
    }
    resets.into_iter().max()
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
                            | "item/fileChange/requestApproval"
                            | "item/tool/requestUserInput"
                            | "item/permissions/requestApproval"
                            | "mcpServer/elicitation/request"
                    )
                ) {
                    return Ok(frame);
                }
                self.rpc.write(json!({"id":frame["id"],"error":self.rpc.error(-32601, "Host request unsupported")})).await?;
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
            if frame["method"] == "account/rateLimits/updated"
                || frame["method"] == "thread/tokenUsage/updated"
                || method == "turn/start"
                    && matches!(
                        frame["method"].as_str(),
                        Some(
                            "item/started"
                                | "item/commandExecution/outputDelta"
                                | "item/agentMessage/delta"
                                | "item/plan/delta"
                                | "item/completed"
                                | "turn/started"
                                | "turn/completed"
                                | "item/commandExecution/requestApproval"
                                | "item/fileChange/requestApproval"
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

    async fn sample_suggestion(
        &mut self,
        text: &dyn crate::AgentText,
        thread: &str,
        turn: &str,
    ) -> Option<String> {
        let request = self.request(
            "thread/sample",
            json!({
                "threadId": thread, "expectedTurnId": turn, "effort": "low",
                "prompt": text.suggestion_prompt()
            }),
        );
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), request)
            .await
            .ok()?
            .ok()?;
        let task_title = text.delegated_title();
        let text = response["text"].as_str()?.trim();
        (!text.is_empty()
            && text.len() <= 1024
            && (task_title.is_empty() || !text.starts_with(task_title)))
        .then(|| text.to_owned())
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
        if !request
            .turn
            .as_ref()
            .is_some_and(|t| children.running(&request.thread.0, &t.0))
        {
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
        options: &ProviderInstance,
        mut permissions: Permissions<impl Fn(PermissionRequest) -> Result<()>>,
        mut controls: mpsc::UnboundedReceiver<Control>,
    ) -> Result<()> {
        self.request("initialize", json!({
            "clientInfo":turn.output.client_info(),
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
            session["config"][format!("mcp_servers.{}", turn.output.name)] = json!({
                "url": tools.url, "bearer_token_env_var": "HUGGLER_MCP_TOKEN",
                "required": true
            });
        }
        let writable = options.workspace_write && !turn.plan_mode;
        if options.request_approvals {
            session["approvalPolicy"] = json!("on-request");
        }
        if options.request_approvals || options.workspace_write {
            session["sandbox"] = json!(if writable {
                "workspace-write"
            } else {
                "read-only"
            });
        }
        if let Some(model) = model {
            session["model"] = json!(model);
        }
        let method = match (&turn.resume, &turn.fork) {
            (_, Some((parent, through))) => {
                session["threadId"] = json!(parent);
                session["lastTurnId"] = json!(through);
                session["excludeTurns"] = json!(true);
                "thread/fork"
            }
            (Some(id), None) => {
                session["threadId"] = json!(id);
                session["excludeTurns"] = json!(true);
                "thread/resume"
            }
            (None, None) => "thread/start",
        };
        let response = self.request(method, session).await?;
        let thread = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol("missing native thread ID"))?;
        if turn.resume.as_ref().is_some_and(|id| id.0 != thread) {
            return Err(protocol("resume returned a different native thread"));
        }
        if let Some((parent, through)) = &turn.fork {
            let fork = &response["thread"];
            // Paginated servers omit turn history when excludeTurns is set.
            let page = self
                .request("thread/turns/list", json!({"threadId":thread,"limit":1}))
                .await?;
            let last = page["data"].as_array().and_then(|turns| turns.first());
            if thread == parent.0
                || fork["forkedFromId"].as_str() != Some(&parent.0)
                || last.and_then(|t| t["id"].as_str()) != Some(&through.0)
            {
                return Err(protocol(
                    "fork returned a different parent or turn boundary",
                ));
            }
        }
        update(Update::Bound(NativeThreadId(thread.to_owned())))?;
        self.request(
            "thread/name/set",
            json!({"threadId":thread,"name":turn.name}),
        )
        .await?;
        self.request("config/read", read_config).await?;
        let mut input = vec![json!({"type":"text","text":turn.text})];
        input.extend(
            turn.files
                .files
                .iter()
                .filter(|file| file.is_image())
                .map(|file| json!({"type":"localImage","path":file.path})),
        );
        let mut params = json!({"threadId":thread, "cwd":turn.cwd, "input":input,
            "approvalPolicy":"never","approvalsReviewer":"user","sandboxPolicy":{"type":"readOnly"},"summary":"auto"});
        if writable {
            params["sandboxPolicy"] = json!({"type":"workspaceWrite", "networkAccess":false});
        }
        if options.request_approvals {
            params["approvalPolicy"] = json!("on-request");
        }
        if let Some(model) = model.or_else(|| response["model"].as_str()) {
            params["model"] = json!(model);
            let mode = if turn.plan_mode { "plan" } else { "default" };
            params["collaborationMode"] = json!({"mode":mode,"settings":{"model":model}});
        } else if turn.plan_mode {
            return Err(protocol("Codex plan mode requires a model"));
        }
        let effort = turn.model_options.effort.as_deref().or_else(|| {
            config
                .pointer("/config/model_reasoning_effort")
                .and_then(Value::as_str)
        });
        if let Some(effort) = effort {
            params["effort"] = json!(effort);
            if let Some(mode) = params.get_mut("collaborationMode") {
                mode["settings"]["reasoning_effort"] = json!(effort);
            }
        }
        if let Some(tier) = &turn.model_options.service_tier {
            params["serviceTierForTurn"] = json!(tier);
        }
        turn.submitting_prompt(&update)?;
        let started = self.request("turn/start", params).await?;
        let id = started
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol("missing native turn ID"))?;
        // Resume can emit the preceding turn's cumulative counters before acknowledging
        // turn/start. Without that baseline, the thread total cannot be attributed to this run.
        let baseline = if turn.resume.is_none() && turn.fork.is_none() {
            Some((0, 0))
        } else {
            self.early
                .iter()
                .filter(|f| {
                    f["method"] == "thread/tokenUsage/updated"
                        && f["params"]["threadId"] == thread
                        && f["params"]["turnId"].as_str().is_some_and(|t| t != id)
                })
                .filter_map(
                    |f| match super::usage(&f["params"]["tokenUsage"]["total"]) {
                        Some(Update::Usage { input, output }) => Some((input, output)),
                        _ => None,
                    },
                )
                .next_back()
        };
        update(Update::Started(Some(NativeTurnId(id.to_owned()))))?;
        let mut answer = None;
        let mut proposed_plan = None;
        let mut pending = HashMap::<RequestId, PendingApproval>::new();
        let mut questions = HashMap::<RequestId, Value>::new();
        let mut children = Children::default();
        let mut foreground_complete = false;
        let mut reset_at = None;
        let mut steers = HashMap::new();
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
                            Control::Steer { command, text } => {
                                if foreground_complete {
                                    update(Update::Steer(command,crate::SteerOutcome::Rejected("foreground turn completed".into())))?;
                                } else {
                                    let wire_id=self.next_id;
                                    self.next_id+=1;
                                    self.rpc.write(json!({"id":wire_id,"method":"turn/steer","params":{"threadId":thread,
                                        "expectedTurnId":id,"input":[{"type":"text","text":text}]}})).await?;
                                    steers.insert(wire_id,command);
                                }
                            }
                            Control::Interrupt(request) => {
                                self.interrupt_child(request, &children, &mut interrupt_replies).await?;
                            }
                            Control::Answer { id, answers, reply } => {
                                if let Some(wire_id) = questions.remove(&id) {
                                    let answered = answers.is_some();
                                    self.write_answers(wire_id, answers.unwrap_or_default()).await?;
                                    update(Update::QuestionClosed { id, answered })?;
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
            if !foreground_complete
                && frame["method"] == "thread/tokenUsage/updated"
                && p["threadId"] == thread
                && p["turnId"] == id
                && let Some((previous_input, previous_output)) = baseline
                && let Some(Update::Usage { input, output }) =
                    super::usage(&p["tokenUsage"]["total"])
                && input >= previous_input
                && output >= previous_output
            {
                update(Update::Usage {
                    input: input - previous_input,
                    output: output - previous_output,
                })?;
            }
            if let Some(wire_id) = frame.get("id") {
                if frame.get("method").is_none() {
                    if let Some(command) = wire_id.as_u64().and_then(|id| steers.remove(&id)) {
                        let outcome = if let Some(error) = frame.get("error") {
                            crate::SteerOutcome::Rejected(error.to_string())
                        } else if frame.pointer("/result/turnId").and_then(Value::as_str)
                            == Some(id)
                        {
                            crate::SteerOutcome::Accepted
                        } else {
                            crate::SteerOutcome::Uncertain
                        };
                        update(Update::Steer(command, outcome))?;
                    }
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
                        turn_id: Some(NativeTurnId(u.into())),
                    });
                let root_request =
                    !foreground_complete && p["threadId"] == thread && p["turnId"] == id;
                if frame["method"] == "item/tool/requestUserInput" && root_request {
                    let Ok(items) = Vec::<UserInputQuestion>::deserialize(&p["questions"]) else {
                        self.rpc.write(json!({"id":wire_id,"error":self.rpc.error(-32602, "Invalid questions")})).await?;
                        continue;
                    };
                    let mut ids = HashSet::new();
                    if items.is_empty()
                        || items.iter().any(|q| q.id.is_empty() || !ids.insert(&q.id))
                        || questions.values().any(|id| id == wire_id)
                    {
                        self.rpc.write(json!({"id":wire_id,"error":self.rpc.error(-32602, "Invalid question IDs")})).await?;
                        continue;
                    }
                    let id = RequestId(uuid::Uuid::new_v4().to_string());
                    questions.insert(id.clone(), wire_id.clone());
                    update(Update::Question(UserInputRequest {
                        id,
                        run_id: permissions.run.clone(),
                        questions: items,
                    }))?;
                    continue;
                }
                if !options.request_approvals || (!root_request && child.is_none()) {
                    self.rpc.write(json!({"id":wire_id,"error":self.rpc.error(-32602, "No matching interactive run")})).await?;
                    continue;
                }
                let (approval, title) = match frame["method"].as_str() {
                    Some("item/permissions/requestApproval") => (
                        Approval::Permissions(p["permissions"].clone()),
                        turn.output.text.label("Grant permissions for this turn"),
                    ),
                    Some("mcpServer/elicitation/request")
                        if turn.agent_tools.is_some()
                            && p["serverName"] == turn.output.name
                            && p["mode"] == "form"
                            && p["_meta"]["codex_approval_kind"] == "mcp_tool_call" =>
                    {
                        (
                            Approval::Mcp,
                            p["message"].as_str().map(str::to_owned).unwrap_or_else(|| {
                                turn.output
                                    .text
                                    .label(&format!("Use {} agent tool", turn.output.title))
                            }),
                        )
                    }
                    Some("item/fileChange/requestApproval") => (
                        Approval::Command,
                        turn.output.text.label("Allow file changes"),
                    ),
                    Some("item/commandExecution/requestApproval") => (
                        Approval::Command,
                        turn.output.text.label("Run command outside the sandbox"),
                    ),
                    _ => {
                        self.rpc.write(json!({"id":wire_id,"error":self.rpc.error(-32601, "Unsupported host request")})).await?;
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
                    name: turn.output.text.label(name),
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
                    request.child.as_ref().is_none_or(|owner| {
                        owner
                            .turn_id
                            .as_ref()
                            .is_some_and(|t| children.running(&owner.thread_id.0, &t.0))
                    })
                });
            }
            if frame["method"] == "account/rateLimits/updated" {
                let limits = &p["rateLimits"];
                if limits["limitId"].is_null() || limits["limitId"] == "codex" {
                    reset_at = quota_reset_at(limits);
                }
                continue;
            }
            if foreground_complete || p["threadId"] != thread {
                continue;
            }
            if p["turnId"] == id
                && let Some(method @ ("item/started" | "item/completed")) = frame["method"].as_str()
                && p["item"]["type"] != "commandExecution"
                && let Some(item) = activity_item(&p["item"], method == "item/completed")
            {
                update(Update::Activity(item))?;
            }
            match frame["method"].as_str() {
                Some("item/commandExecution/outputDelta") if p["turnId"] == id => {
                    if let (Some(item), Some(delta)) = (p["itemId"].as_str(), p["delta"].as_str()) {
                        update(Update::CommandOutput {
                            item: item.into(),
                            frame: CommandFrame::Append(delta.into()),
                        })?;
                    }
                }
                Some(method @ ("item/started" | "item/completed"))
                    if p["turnId"] == id && p["item"]["type"] == "commandExecution" =>
                {
                    if let Some(item) = p["item"]["id"].as_str() {
                        let output = p["item"]["aggregatedOutput"].as_str().map(str::to_owned);
                        update(Update::CommandOutput {
                            item: item.into(),
                            frame: CommandFrame::Snapshot {
                                activity: activity_item(&p["item"], method == "item/completed"),
                                output,
                                finished: method == "item/completed",
                            },
                        })?;
                    }
                }
                Some("item/agentMessage/delta" | "item/plan/delta") if p["turnId"] == id => {
                    if let Some(delta) = p["delta"].as_str() {
                        update(Update::Output(delta.into()))?;
                    }
                }
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
                Some("item/completed") if p["turnId"] == id && p["item"]["type"] == "plan" => {
                    proposed_plan = p["item"]["text"].as_str().map(str::to_owned);
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
                    if p["turn"]["status"] == "failed"
                        && p["turn"]["error"]["codexErrorInfo"] == "usageLimitExceeded"
                    {
                        return Err(Error::UsageLimit {
                            message: p["turn"]["error"]["message"]
                                .as_str()
                                .unwrap_or("usage limit exceeded")
                                .into(),
                            reset_at,
                        });
                    }
                    if p["turn"]["status"] != "completed" {
                        return Err(protocol(format!(
                            "turn {}: {}",
                            p["turn"]["status"], p["turn"]["error"]
                        )));
                    }
                    let text = match (answer.take(), proposed_plan.take()) {
                        (Some(answer), Some(plan)) if !answer.contains(&plan) => {
                            format!("{answer}\n\n{plan}")
                        }
                        (Some(answer), _) => answer,
                        (None, Some(plan)) => plan,
                        (None, None) => return Err(protocol("completed turn has no final answer")),
                    };
                    update(Update::Finalizing(text))?;
                    if !children.active() && interrupt_replies.is_empty() {
                        if turn.sample_suggestion {
                            update(Update::ForegroundComplete)?;
                            if let Some(text) =
                                self.sample_suggestion(&*turn.output.text, thread, id).await
                            {
                                update(Update::PromptSuggestion(text))?;
                            }
                        }
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
    options: &ProviderInstance,
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
        rpc: Rpc::spawn(options, &["app-server"], &turn)?,
        next_id: 1,
        early: VecDeque::new(),
    };
    let result = tokio::select! {
        biased;
        _ = stop.wait_for(|v| *v) => Err(Error::Closed),
        answer = client.converse(turn, update,options,permissions,controls) => answer,
    };
    client.rpc.finish(result).await
}
