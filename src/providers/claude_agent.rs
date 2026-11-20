//! Claude Agent ACP v1 over the standalone adapter's stdio.
use super::Permissions;
use super::{
    Turn, Update,
    rpc::{Rpc, protocol},
};
use crate::{Error, NativeThreadId, PermissionOption, PermissionRequest, RequestId, Result};
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf};
use tokio::sync::watch;

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
        self.rpc.write(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unsupported client method"}})).await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.send(method, params).await?;
        loop {
            let f = self.rpc.read().await?;
            if f.get("method").is_some() {
                if let Some(id) = f.get("id") {
                    self.unsupported(id).await?;
                }
            } else {
                return result(&f, id, method);
            }
        }
    }

    async fn configure(&mut self, turn: &Turn) -> Result<String> {
        let init = self.request("initialize",json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"huggler","version":env!("CARGO_PKG_VERSION")}})).await?;
        if init["protocolVersion"] != 1 {
            return Err(protocol("ACP v1 required"));
        }
        let mut servers = vec![];
        if let Some(tools) = &turn.agent_tools {
            if init.pointer("/agentCapabilities/mcpCapabilities/http") != Some(&Value::Bool(true)) {
                return Err(protocol("adapter does not support HTTP MCP servers"));
            }
            servers.push(json!({"name":"huggler","type":"http","url":tools.url,
                "headers":[{"name":"Authorization","value":format!("Bearer {}",tools.token)}]}));
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
        if info.pointer("/modes/currentModeId").and_then(Value::as_str) != Some("default") {
            self.request(
                "session/set_mode",
                json!({"sessionId":session,"modeId":"default"}),
            )
            .await?;
        }
        if let Some(model) = &turn.model {
            self.request(
                "session/set_model",
                json!({"sessionId":session,"modelId":model}),
            )
            .await?;
        }
        Ok(session)
    }
}
fn result(frame: &Value, id: u64, method: &str) -> Result<Value> {
    if frame["id"] != id {
        return Err(protocol(format!("unexpected ACP reply during {method}")));
    }
    if let Some(error) = frame.get("error") {
        return Err(protocol(format!(
            "Claude ACP {method}: {}",
            error["message"].as_str().unwrap_or("provider error")
        )));
    }
    frame
        .get("result")
        .cloned()
        .ok_or_else(|| protocol(format!("missing ACP result for {method}")))
}

pub(crate) async fn execute(
    options: &ClaudeAgentOptions,
    turn: Turn,
    mut stop: watch::Receiver<bool>,
    update: impl Fn(Update) -> Result<()>,
    mut permissions: Permissions<impl Fn(PermissionRequest) -> Result<()>>,
) -> Result<()> {
    if *stop.borrow() {
        return Err(Error::Closed);
    }
    let mut rpc = Rpc::spawn(&options.executable, &[], &turn)?;
    let conversation = async {
        let mut client = Session {
            rpc: &mut rpc,
            sequence: 0,
        };
        let session = client.configure(&turn).await?;
        update(Update::Bound(NativeThreadId(session.clone())))?;
        let prompt = client
            .send(
                "session/prompt",
                json!({"sessionId":session,"prompt":[{"type":"text","text":turn.text}]}),
            )
            .await?;
        update(Update::Started(None))?;
        let mut answer = String::new();
        let mut pending = HashMap::new();
        loop {
            let f = tokio::select! {
                frame = client.rpc.read() => frame?,
                decision = permissions.decisions.recv() => {
                    let (request,option) = decision.ok_or(Error::Closed)?;
                    let wire_id = pending.remove(&request).ok_or_else(||protocol("expired permission"))?;
                    client.rpc.write(json!({"jsonrpc":"2.0","id":wire_id,"result":{"outcome":{"outcome":"selected","optionId":option}}})).await?;
                    continue;
                }
            };
            let params = &f["params"];
            match f["method"].as_str() {
                Some("session/request_permission")
                    if params["sessionId"] == session && !f["id"].is_null() =>
                {
                    let options: Vec<PermissionOption> =
                        serde_json::from_value(params["options"].clone())?;
                    let id = RequestId(uuid::Uuid::new_v4().to_string());
                    pending.insert(id.clone(), f["id"].clone());
                    (permissions.publish)(PermissionRequest {
                        id,
                        run_id: permissions.run.clone(),
                        tool_call: params["toolCall"].clone(),
                        options,
                    })?;
                }
                Some("session/update") if params["sessionId"] == session => {
                    let update = &params["update"];
                    if update["sessionUpdate"] == "agent_message_chunk"
                        && update["content"]["type"] == "text"
                    {
                        if let Some(text) = update["content"]["text"].as_str() {
                            answer.push_str(text);
                        }
                    }
                }
                Some(_) if f.get("id").is_some() => client.unsupported(&f["id"]).await?,
                Some(_) => {}
                None => {
                    let response = result(&f, prompt, "session/prompt")?;
                    if response["stopReason"] != "end_turn" {
                        return Err(protocol(format!(
                            "Claude stopped: {}",
                            response["stopReason"]
                        )));
                    }
                    return update(Update::Finalizing(answer));
                }
            }
        }
    };
    let outcome = tokio::select! {
        biased;
        _ = stop.wait_for(|s|*s) => Err(Error::Closed),
        result = conversation => result,
    };
    rpc.shutdown().await?;
    outcome
}
