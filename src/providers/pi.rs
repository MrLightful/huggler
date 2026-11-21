//! Pi process RPC. Resume requires both the session file and its embedded identity.
use super::{
    Turn, Update,
    probe::ProviderReport,
    rpc::{Rpc, protocol},
};
use crate::{Error, NativeThreadId, ProviderInstance, Result};
use serde_json::{Value, json};
use std::{path::Path, process::Command};
use tokio::sync::watch;

async fn extension_request(rpc: &mut Rpc, frame: &Value) -> Result<()> {
    // Pi's notification/status methods are one-way. A dialog lacking a host renderer is cancelled.
    if matches!(
        frame["method"].as_str(),
        Some("select" | "confirm" | "input" | "editor")
    ) {
        rpc.write(json!({"type":"extension_ui_response","id":frame["id"],"cancelled":true}))
            .await?;
    }
    Ok(())
}

async fn request(rpc: &mut Rpc, method: &str, mut params: Value) -> Result<Value> {
    let id = uuid::Uuid::new_v4().to_string();
    params["type"] = json!(method);
    params["id"] = json!(id);
    rpc.write(params).await?;
    loop {
        let f = rpc.read().await?;
        if f["type"] == "extension_ui_request" {
            extension_request(rpc, &f).await?;
            continue;
        }
        if f["type"] != "response" {
            continue;
        }
        if f["id"] != id || f["command"] != method {
            return Err(protocol(format!("unexpected Pi response during {method}")));
        }
        if f["success"] != true {
            return Err(protocol(format!("Pi {method}: {}", f["error"])));
        }
        return Ok(f["data"].clone());
    }
}

fn command(instance: &ProviderInstance) -> Result<Command> {
    if let Some(path) = &instance.pi_extension
        && !path.is_file()
    {
        return Err(protocol(format!(
            "install the Pi extension or update piExtension: {}",
            path.display()
        )));
    }
    let mut cmd = instance.command();
    cmd.args(["--mode", "rpc"])
        .env_remove("HUGGLER_MCP_TOKEN")
        .env_remove("HUGGLER_MCP_URL")
        .env_remove("HUGGLER_MCP_PRESENTATION");
    Ok(cmd)
}

pub(super) async fn probe(
    instance: &ProviderInstance,
    output: &crate::AgentOutput,
    cwd: &Path,
) -> Result<ProviderReport> {
    let mut rpc = Rpc::open(command(instance)?, cwd, output)?;
    request(&mut rpc, "get_state", json!({})).await?;
    let catalog = request(&mut rpc, "get_available_models", json!({})).await?;
    let mut report = ProviderReport::new("Pi RPC");
    report.models = catalog["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| Some(format!("{}/{}", m["provider"].as_str()?, m["id"].as_str()?)))
        .collect();
    report.resume = true;
    report.agent_tools = instance.pi_extension.is_some();
    rpc.finish(Ok(())).await?;
    Ok(report)
}

pub(crate) async fn execute(
    instance: &ProviderInstance,
    turn: Turn,
    mut stop: watch::Receiver<bool>,
    update: impl Fn(Update) -> Result<()>,
) -> Result<()> {
    let mut cmd = command(instance)?;
    if let (Some(tools), Some(path)) = (&turn.agent_tools, &instance.pi_extension) {
        cmd.arg("--extension")
            .arg(path)
            .env("HUGGLER_MCP_URL", &tools.url)
            .env("HUGGLER_MCP_TOKEN", &tools.token)
            .env("HUGGLER_MCP_PRESENTATION", json!({
                "clientInfo": turn.output.client_info(),
                "httpError": turn.output.text.label(&format!("{} MCP returned HTTP {{status}}", turn.output.title)),
                "unexpectedReply": turn.output.text.label("Unexpected MCP reply"),
                "toolFailed": turn.output.text.label(&format!("{} tool failed", turn.output.title)),
            }).to_string());
    }
    if let Some(saved) = &turn.resume {
        let (file, _): (String, String) = serde_json::from_str(&saved.0)?;
        cmd.arg("--session").arg(file);
    }
    let mut rpc = Rpc::open(cmd, &turn.cwd, &turn.output)?;
    let result = tokio::select! {
        biased;
        _=stop.wait_for(|s|*s)=>Err(Error::Closed),
        r=converse(&mut rpc,&turn,update)=>r,
    };
    rpc.finish(result).await
}

async fn converse(rpc: &mut Rpc, turn: &Turn, update: impl Fn(Update) -> Result<()>) -> Result<()> {
    let state = request(rpc, "get_state", json!({})).await?;
    let file = state["sessionFile"]
        .as_str()
        .filter(|p| Path::new(p).is_absolute())
        .ok_or_else(|| protocol("Pi needs a durable session file"))?;
    let id = state["sessionId"]
        .as_str()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| protocol("missing Pi session ID"))?;
    let native = NativeThreadId(serde_json::to_string(&(file, id))?);
    if turn.resume.as_ref().is_some_and(|s| s != &native) {
        return Err(protocol("Pi opened a different session file or identity"));
    }
    update(Update::Bound(native))?;
    if let Some(model) = &turn.model {
        let (provider, id) = model
            .split_once('/')
            .ok_or_else(|| protocol("Pi model must be provider/model"))?;
        request(rpc, "set_model", json!({"provider":provider,"modelId":id})).await?;
    }
    turn.submitting_prompt(&update)?;
    rpc.write(json!({"type":"prompt","message":turn.text}))
        .await?;
    update(Update::Started(None))?;
    let mut totals = (0i64, 0i64);
    let mut reported = false;
    loop {
        let f = rpc.read().await?;
        match f["type"].as_str() {
            Some("response") if f["command"] == "prompt" && f["success"] != true => {
                return Err(protocol(format!("Pi prompt: {}", f["error"])));
            }
            Some("extension_ui_request") => extension_request(rpc, &f).await?,
            Some("message_update") if f["assistantMessageEvent"]["type"] == "text_delta" => {
                if let Some(text) = f["assistantMessageEvent"]["delta"].as_str() {
                    update(Update::Output(text.into()))?;
                }
            }
            Some("message_end") if f["message"]["role"] == "assistant" => {
                let u = &f["message"]["usage"];
                if let (Some(input), Some(output)) = (u["input"].as_i64(), u["output"].as_i64())
                    && input >= 0
                    && output >= 0
                {
                    totals.0 = totals.0.saturating_add(input);
                    totals.1 = totals.1.saturating_add(output);
                    reported = true;
                }
            }
            Some("agent_end") if f["willRetry"] != true => {
                let msg = f["messages"]
                    .as_array()
                    .and_then(|items| items.last())
                    .filter(|m| m["role"] == "assistant")
                    .ok_or_else(|| protocol("Pi ended without an assistant answer"))?;
                match msg["stopReason"].as_str() {
                    Some("stop") => {}
                    Some("aborted") => return Err(Error::Interrupted),
                    _ => return Err(protocol(format!("Pi stopped: {}", msg["stopReason"]))),
                }
                let answer = msg["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["type"] == "text")
                    .filter_map(|c| c["text"].as_str())
                    .collect::<String>();
                if answer.is_empty() {
                    return Err(protocol("Pi ended without assistant text"));
                }
                if reported {
                    update(Update::Usage {
                        input: totals.0,
                        output: totals.1,
                    })?;
                }
                return update(Update::Finalizing(answer));
            }
            _ => {}
        }
    }
}
