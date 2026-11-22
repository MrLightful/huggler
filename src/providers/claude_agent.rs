//! ACP sessions over stdio, including Claude Agent's subagent extension.
mod children;
use super::registry::ProviderInstance;
use super::{
    Control, Permissions, Turn, Update,
    rpc::{Rpc, protocol},
};
use crate::{
    Error, NativeChildTurn, NativeThreadId, PermissionOption, PermissionRequest, Provider,
    RequestId, Result,
};
use children::{Children, text_chunk};
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf};
use tokio::sync::{mpsc, watch};

struct PendingPermission {
    wire_id: Value,
    child: Option<NativeThreadId>,
}

#[derive(Clone, Debug)]
pub struct ClaudeAgentOptions {
    pub executable: PathBuf,
}

impl Default for ClaudeAgentOptions {
    fn default() -> Self {
        Self {
            executable: "claude-agent-acp".into(),
        }
    }
}

struct Session<'a> {
    rpc: &'a mut Rpc,
    children: Children<'a>,
    sequence: u64,
}
impl Session<'_> {
    async fn send(&mut self, method: &str, params: Value) -> Result<u64> {
        self.sequence += 1;
        self.rpc
            .write(json!({"jsonrpc":"2.0","method":method,"params":params,"id":self.sequence}))
            .await?;
        Ok(self.sequence)
    }

    async fn unsupported(&mut self, id: &Value) -> Result<()> {
        self.rpc.write(json!({"jsonrpc":"2.0","id":id,"error":self.rpc.error(-32601, "Unsupported client method")})).await
    }

    async fn cancel_permission(&mut self, wire_id: Value) -> Result<()> {
        self.rpc
            .write(json!({
                "jsonrpc": "2.0", "id": wire_id, "result": {"outcome": {"outcome": "cancelled"}}
            }))
            .await
    }

    async fn select_permission(&mut self, wire_id: &Value, option: &str) -> Result<()> {
        self.rpc
            .write(json!({
                "jsonrpc": "2.0", "id": wire_id,
                "result": {"outcome": {"outcome": "selected", "optionId": option}}
            }))
            .await
    }

    async fn close_permissions(
        &mut self,
        pending: &mut HashMap<RequestId, PendingPermission>,
        owner: Option<&NativeThreadId>,
    ) -> Result<()> {
        for (_, request) in pending.extract_if(|_, p| p.child.as_ref() == owner) {
            self.cancel_permission(request.wire_id).await?;
        }
        Ok(())
    }

    async fn control(&mut self, control: Control) -> Result<()> {
        match control {
            Control::Interrupt(request) => {
                if request.turn.is_some() || !self.children.can_interrupt(&request.thread.0) {
                    let _ = request.reply.send(Err(Error::NotFound(
                        "interruptible native child session".into(),
                    )));
                } else {
                    self.rpc
                        .write(json!({
                            "jsonrpc": "2.0",
                            "method": "session/cancel",
                            "params": {"sessionId": request.thread}
                        }))
                        .await?;
                    let _ = request.reply.send(Ok(()));
                }
            }
            Control::Steer { .. } => {
                return Err(Error::Unsupported("native steering requires Codex".into()));
            }
            Control::Answer { reply, .. } => {
                let _ = reply.send(Err(Error::NotFound("live user-input request".into())));
            }
        }
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let root = params["sessionId"].as_str().map(str::to_owned);
        let id = self.send(method, params).await?;
        loop {
            let f = self.rpc.read().await?;
            if f.get("method").is_some() {
                if let Some(id) = f.get("id") {
                    self.unsupported(id).await?;
                } else if f["method"] == "session/update"
                    && let Some(root) = &root
                {
                    self.children.announcement(root, &f["params"], true);
                }
            } else {
                return result(&f, id, method);
            }
        }
    }

    async fn configure(&mut self, turn: &Turn) -> Result<(String, bool)> {
        let init = initialize(self.rpc, turn.provider, &turn.output).await?;
        self.sequence = 1;
        let mut servers = vec![];
        if let Some(tools) = &turn.agent_tools
            && init["agentCapabilities"]["mcpCapabilities"]["http"] == true
        {
            servers.push(
                json!({"name":turn.output.name,"type":"http","url":tools.url,
                "headers":[{"name":"Authorization","value":format!("Bearer {}",tools.token)}]}),
            );
        }
        let mut params = json!({"cwd":turn.cwd,"mcpServers":servers});
        let (session, info) = if let Some(native) = &turn.resume {
            if init["agentCapabilities"]["loadSession"] != true {
                return Err(protocol("adapter cannot load the saved session"));
            }
            params["sessionId"] = json!(native);
            (
                native.0.clone(),
                self.request("session/load", params).await?,
            )
        } else {
            let info = self.request("session/new", params).await?;
            (
                info["sessionId"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| protocol("missing ACP session ID"))?
                    .to_owned(),
                info,
            )
        };
        if info
            .get("sessionId")
            .and_then(Value::as_str)
            .is_some_and(|id| id != session)
        {
            return Err(protocol("ACP resume returned a different session"));
        }
        if turn.provider == Provider::ClaudeAgent
            && info.pointer("/modes/currentModeId").and_then(Value::as_str) != Some("default")
        {
            self.request(
                "session/set_mode",
                json!({"sessionId":session,"modeId":"default"}),
            )
            .await?;
        }
        if let Some(model) = &turn.model {
            if model_option(&info).is_some() {
                self.request(
                    "session/set_config_option",
                    json!({"sessionId":session,"configId":"model","value":model}),
                )
                .await?;
            } else {
                self.request(
                    "session/set_model",
                    json!({"sessionId":session,"modelId":model}),
                )
                .await?;
            }
        }
        Ok((
            session,
            init["agentCapabilities"]["promptCapabilities"]["image"] == true,
        ))
    }
}
pub(super) async fn initialize(
    rpc: &mut Rpc,
    driver: Provider,
    output: &crate::AgentOutput,
) -> Result<Value> {
    let version = if matches!(driver, Provider::Kiro | Provider::Antigravity) {
        2
    } else {
        1
    };
    let caps = if driver == Provider::ClaudeAgent {
        json!({"subagents":{}})
    } else {
        json!({})
    };
    let init = setup_request(
        rpc,
        1,
        "initialize",
        json!({
            "protocolVersion": version,
            "clientCapabilities": caps,
            "clientInfo": output.client_info(),
        }),
    )
    .await?;
    if init["protocolVersion"] != version
        && !(driver == Provider::Antigravity && init["protocolVersion"] == 1)
    {
        return Err(Error::IncompatibleProvider(format!(
            "configure an ACP v{version} adapter for {}",
            driver.key()
        )));
    }
    Ok(init)
}

pub(super) async fn setup_request(
    rpc: &mut Rpc,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value> {
    result(&rpc.request_frame(id, method, params).await?, id, method)
}

pub(super) fn model_option(info: &Value) -> Option<&Value> {
    info["configOptions"]
        .as_array()?
        .iter()
        .find(|o| o["id"] == "model" && o["type"] == "select")
}
fn result(frame: &Value, id: u64, method: &str) -> Result<Value> {
    if frame["id"] != id {
        return Err(protocol(format!("unexpected ACP reply during {method}")));
    }
    if let Some(error) = frame.get("error") {
        if error["code"] == -32000 {
            return Err(Error::LoginRequired);
        }
        return Err(protocol(format!(
            "ACP {method}: {}",
            error["message"].as_str().unwrap_or("provider error")
        )));
    }
    frame
        .get("result")
        .cloned()
        .ok_or_else(|| protocol(format!("missing ACP result for {method}")))
}

pub(crate) async fn execute(
    options: &ProviderInstance,
    turn: Turn,
    mut stop: watch::Receiver<bool>,
    update: impl Fn(Update) -> Result<()>,
    mut permissions: Permissions<impl Fn(PermissionRequest) -> Result<()>>,
    mut controls: mpsc::UnboundedReceiver<Control>,
) -> Result<()> {
    if *stop.borrow() {
        return Err(Error::Closed);
    }
    let mut rpc = Rpc::spawn(options, &[], &turn)?;
    let full_access = options.full_access && !turn.plan_mode;
    let conversation = async {
        let mut client = Session {
            rpc: &mut rpc,
            sequence: 0,
            children: Children::new(&permissions.run),
        };
        let (session, supports_images) = client.configure(&turn).await?;
        let mut content = vec![json!({"type":"text","text":turn.text})];
        if supports_images {
            content.extend(turn.files.files.iter().filter(|file| file.is_image()).map(
                |file| json!({"type":"image","data":file.base64(),"mimeType":file.file.mime}),
            ));
        }
        update(Update::Bound(NativeThreadId(session.clone())))?;
        turn.submitting_prompt(&update)?;
        let prompt = client
            .send(
                "session/prompt",
                json!({"sessionId":session,"prompt":content}),
            )
            .await?;
        update(Update::Started(None))?;
        let mut answer = String::new();
        let mut pending = HashMap::<RequestId, PendingPermission>::new();
        let mut foreground_complete = false;
        loop {
            if foreground_complete && !client.children.active() {
                return Ok(());
            }
            let f = tokio::select! {
                frame = client.rpc.read() => frame?,
                control = controls.recv() => {
                    client.control(control.ok_or(Error::Closed)?).await?;
                    continue;
                }
                decision = permissions.decisions.recv() => {
                    let (request,option) = decision.ok_or(Error::Closed)?;
                    let Some(request) = pending.remove(&request) else {
                        continue;
                    };
                    client.select_permission(&request.wire_id, &option).await?;
                    continue;
                }
            };
            let params = &f["params"];
            match f["method"].as_str() {
                Some("session/request_permission") if !f["id"].is_null() => {
                    let target = params["sessionId"].as_str().unwrap_or("");
                    let child = if client.children.running(target) {
                        Some(NativeThreadId(target.into()))
                    } else if foreground_complete || target != session {
                        client.rpc.write(json!({"jsonrpc":"2.0","id":f["id"],"error":client.rpc.error(-32602, "Unknown active session")})).await?;
                        continue;
                    } else {
                        None
                    };
                    let options: Vec<PermissionOption> =
                        serde_json::from_value(params["options"].clone())?;
                    if full_access {
                        // A persistent provider rule could outlive Full access.
                        if let Some(option) =
                            options.iter().find(|option| option.kind == "allow_once")
                        {
                            client
                                .select_permission(&f["id"], &option.option_id)
                                .await?;
                        } else {
                            client.cancel_permission(f["id"].clone()).await?;
                        }
                        continue;
                    }
                    let id = RequestId(uuid::Uuid::new_v4().to_string());
                    pending.insert(
                        id.clone(),
                        PendingPermission {
                            wire_id: f["id"].clone(),
                            child: child.clone(),
                        },
                    );
                    (permissions.publish)(PermissionRequest {
                        native_child: child.map(|thread_id| NativeChildTurn {
                            thread_id,
                            turn_id: None,
                        }),
                        id,
                        run_id: permissions.run.clone(),
                        tool_call: params["toolCall"].clone(),
                        options,
                    })?;
                }
                Some("$/cancel_request") => {
                    if let Some(id) = pending
                        .iter()
                        .find(|(_, p)| p.wire_id == params["requestId"])
                        .map(|(id, _)| id.clone())
                    {
                        pending.remove(&id);
                        update(Update::PermissionWithdrawn(id))?;
                        client
                            .cancel_permission(params["requestId"].clone())
                            .await?;
                    }
                }
                Some("session/update") => {
                    let Some(target) = params["sessionId"].as_str() else {
                        continue;
                    };
                    let notification = &params["update"];
                    if !foreground_complete
                        && target == session
                        && matches!(
                            notification["sessionUpdate"].as_str(),
                            Some("tool_call" | "tool_call_update")
                        )
                        && let Some(id) = notification["toolCallId"].as_str()
                    {
                        use crate::{ActivityItem, ActivityKind, ActivityStatus};
                        let status = match notification["status"].as_str() {
                            Some("completed") => ActivityStatus::Completed,
                            Some("failed") => ActivityStatus::Failed,
                            Some("pending" | "in_progress") => ActivityStatus::Running,
                            _ => ActivityStatus::Unknown,
                        };
                        update(Update::Activity(ActivityItem {
                            id: id.into(),
                            kind: ActivityKind::Tool,
                            title: notification["title"].as_str().unwrap_or_default().into(),
                            status,
                            timestamp_ms: None,
                        }))?;
                    }
                    if notification["sessionUpdate"] == "subagent_update" {
                        if let Some(child) = client.children.announcement(&session, params, false) {
                            let finished =
                                (!child.status.working()).then(|| child.thread_id.clone());
                            update(Update::NativeChild(child))?;
                            if let Some(id) = finished {
                                client.close_permissions(&mut pending, Some(&id)).await?;
                            }
                        }
                    } else if target == session
                        && let Some(text) = text_chunk(notification)
                    {
                        if !foreground_complete
                            && notification
                                .pointer("/_meta/claudeCode/parentToolUseId")
                                .and_then(Value::as_str)
                                .is_none()
                        {
                            update(Update::Output(text.to_owned()))?;
                            answer.push_str(text);
                        }
                    } else if let Some(child) = client.children.message(target, notification) {
                        update(Update::NativeChild(child))?;
                    }
                }
                Some(_) if f.get("id").is_some() => client.unsupported(&f["id"]).await?,
                Some(_) => {}
                None if foreground_complete => {}
                None => {
                    let response = result(&f, prompt, "session/prompt")?;
                    if response["stopReason"] == "cancelled" {
                        return Err(Error::Interrupted);
                    }
                    if response["stopReason"] != "end_turn" {
                        return Err(protocol(format!("ACP stopped: {}", response["stopReason"])));
                    }
                    let usage = super::usage(&response["usage"]).or_else(|| {
                        (turn.provider == Provider::Grok)
                            .then(|| super::usage(&response["_meta"]["usage"]))
                            .flatten()
                    });
                    if let Some(tokens) = usage {
                        update(tokens)?;
                    }
                    update(Update::Finalizing(std::mem::take(&mut answer)))?;
                    if !client.children.active() {
                        return Ok(());
                    }
                    update(Update::ForegroundComplete)?;
                    foreground_complete = true;
                    client.close_permissions(&mut pending, None).await?;
                }
            }
        }
    };
    let outcome = tokio::select! {
        biased;
        _ = stop.wait_for(|s|*s) => Err(Error::Closed),
        result = conversation => result,
    };
    rpc.finish(outcome).await
}

/// Load visible history before the host creates a durable binding. This operation sends no prompt.
pub(crate) async fn load_history(
    instance: &ProviderInstance,
    output: &crate::AgentOutput,
    cwd: &std::path::Path,
    native: NativeThreadId,
) -> Result<crate::imports::Snapshot> {
    use crate::{
        ImportProvenance, ImportSource, Role,
        imports::{MAX_BYTES, Messages, Snapshot},
    };
    if native.0.trim().is_empty() || instance.driver != Provider::AcpRegistry {
        return Err(Error::Unsupported(
            "native import requires an ACP Registry session".into(),
        ));
    }
    let mut rpc = Rpc::open(instance.command(), cwd, output)?;
    let mut messages = Messages::new();
    let history = async {
        let init = initialize(&mut rpc, instance.driver, output).await?;
        if init["agentCapabilities"]["loadSession"] != true {
            return Err(Error::Unsupported(
                "ACP instance does not advertise session/load".into(),
            ));
        }
        let mut size = 0;
        let mut last_id = None;
        rpc.write(json!({"jsonrpc":"2.0","id":2,"method":"session/load","params":{"cwd":cwd,"sessionId":native,"mcpServers":[]}})).await?;
        loop {
            let frame = rpc.read().await?;
            if frame.get("method").is_none() {
                let info = result(&frame, 2, "session/load")?;
                if info["sessionId"].as_str().is_some_and(|id| id != native.0) {
                    return Err(protocol("ACP returned a different session"));
                }
                break;
            }
            if let Some(id) = frame.get("id") {
                rpc.write(json!({"jsonrpc":"2.0","id":id,"error":rpc.error(-32601, "Unsupported during history import")})).await?;
                continue;
            }
            let params = &frame["params"];
            if frame["method"] != "session/update"
                || params["sessionId"].as_str() != Some(native.0.as_str())
            {
                continue;
            }
            let update = &params["update"];
            let role = match update["sessionUpdate"].as_str() {
                Some("user_message_chunk") => Role::User,
                Some("agent_message_chunk") => Role::Assistant,
                _ => continue,
            };
            if update["content"]["type"] != "text" {
                continue;
            }
            let Some(chunk) = update["content"]["text"].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            size += chunk.len();
            if size > MAX_BYTES {
                return Err(Error::Invalid("ACP history exceeds 8 MiB".into()));
            }
            let id = update["messageId"].as_str();
            if let Some((prev, text)) = messages.last_mut()
                && *prev == role
                && last_id.as_deref() == id
            {
                text.push_str(chunk);
            } else {
                if messages.len() >= 32768 {
                    return Err(Error::Invalid("too many ACP messages".into()));
                }
                messages.push((role, chunk.into()));
            }
            last_id = id.map(str::to_owned);
        }
        Ok(())
    };
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), history)
        .await
        .unwrap_or_else(|_| Err(protocol("ACP history load timed out")));
    rpc.finish(outcome).await?;
    let digest = crate::imports::digest(&serde_json::to_vec(&messages)?);
    Ok(Snapshot {
        provenance: ImportProvenance {
            source: ImportSource::AcpRegistry(native.clone()),
            native_session: Some(native),
            digest,
        },
        messages,
    })
}
