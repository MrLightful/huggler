//! Muse Session Protocol over a dedicated stdio host.
use super::{
    Turn, Update,
    probe::{ProviderReport, ids},
    rpc::{Rpc, protocol},
};
use crate::{Error, NativeThreadId, NativeTurnId, ProviderInstance, Result};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};
use tokio::sync::watch;

async fn connect(instance: &ProviderInstance, cwd: &Path) -> Result<(Rpc, Value)> {
    let mut cmd = std::process::Command::new(&instance.executable);
    cmd.arg("serve").args(&instance.args);
    let mut rpc = Rpc::open(cmd, cwd)?;
    let init = rpc.request(1, "initialize", json!({
        "clientInfo": {"name": "huggler", "version": env!("CARGO_PKG_VERSION")},
        "capabilities": {"userInputDialogs": false, "requestedCapabilities": ["sessionMcp"]},
    })).await?;
    if init["schema"]["version"] != 1 {
        return Err(Error::IncompatibleProvider(
            "install a Muse serve build with MSP schema v1".into(),
        ));
    }
    rpc.write(json!({"jsonrpc":"2.0","method":"initialized"}))
        .await?;
    Ok((rpc, init))
}

fn supports_mcp(init: &Value) -> bool {
    init["grantedCapabilities"]
        .as_array()
        .is_some_and(|c| c.iter().any(|s| s == "sessionMcp"))
}

pub(super) async fn probe(instance: &ProviderInstance, cwd: &Path) -> Result<ProviderReport> {
    let (mut rpc, init) = connect(instance, cwd).await?;
    let models = rpc.request(2, "model/list", json!({})).await?;
    let mut report = ProviderReport::new("Muse MSP v1");
    report.version = init["serverInfo"]["version"].as_str().map(str::to_owned);
    report.agent_tools = supports_mcp(&init);
    report.resume = init["sessionDurability"] == "durable";
    report.models = ids(&models["models"], "modelId");
    rpc.finish(Ok(())).await?;
    Ok(report)
}

fn command_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

pub(crate) async fn execute(
    instance: &ProviderInstance,
    turn: Turn,
    mut stop: watch::Receiver<bool>,
    update: impl Fn(Update) -> Result<()>,
) -> Result<()> {
    let run = async {
        let (mut rpc, init) = connect(instance, &turn.cwd).await?;
        let mut params = json!({"commandId":command_id()});
        if let Some(model) = &turn.model {
            params["modelId"] = json!(model);
        }
        if let Some(tools) = &turn.agent_tools
            && supports_mcp(&init)
        {
            params["config"] = json!({"mcpServers":{"huggler":{"transport":"streamableHttp","mode":"required","url":tools.url,"headers":{"Authorization":format!("Bearer {}",tools.token)}}}});
        }
        let method = if let Some(saved) = &turn.resume {
            if init["sessionDurability"] != "durable" {
                return Err(protocol(
                    "Muse cannot resume the saved session on this host",
                ));
            }
            params["sessionId"] = json!(saved);
            params["excludeItems"] = json!(true);
            "session/resume"
        } else {
            params["approvalMode"] = json!("denyUnmatched");
            params["workspaceRoot"] = json!(turn.cwd);
            "session/start"
        };
        let response = rpc.request(2, method, params).await?;
        let session = response["session"]["sessionId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| protocol("missing Muse session ID"))?;
        if response["session"]["workspaceRoot"].as_str().map(Path::new) != Some(turn.cwd.as_path())
            || turn.resume.as_ref().is_some_and(|id| id.0 != session)
        {
            return Err(protocol("Muse returned a different session"));
        }
        if turn
            .model
            .as_ref()
            .is_some_and(|id| response["session"]["modelId"] != *id)
        {
            return Err(protocol("Muse declined the requested model"));
        }
        if !response["session"]["activeTurnId"].is_null() {
            return Err(Error::Busy);
        }
        update(Update::Bound(NativeThreadId(session.into())))?;
        let turn_id = command_id();
        turn.submitting_prompt(&update)?;
        rpc.write(json!({"jsonrpc":"2.0","id":3,"method":"turn/start","params":{"sessionId":session,"commandId":turn_id,"input":[{"type":"text","text":turn.text}]}})).await?;
        let mut usage_cursors = HashSet::new();
        let mut totals = (0i64, 0i64);
        let mut accepted = false;
        let mut completed = false;
        let mut items = HashSet::new();
        let mut answer = None;
        while !(completed && accepted) {
            let f = rpc.read().await?;
            if f.get("method").is_none() && f["id"] == 3 {
                if f["result"]["turnId"] != turn_id || f["result"]["startedNewTurn"] != true {
                    return Err(protocol(format!("Muse refused turn/start: {}", f["error"])));
                }
                update(Update::Started(Some(NativeTurnId(turn_id.clone()))))?;
                accepted = true;
                continue;
            }
            let p = &f["params"];
            if matches!(
                f["method"].as_str(),
                Some(
                    "approval/request"
                        | "approval/requested"
                        | "userInput/request"
                        | "userInput/requested"
                )
            ) && p["sessionId"] == session
            {
                return Err(protocol(
                    "Muse requested interactive input unsupported by this adapter",
                ));
            }
            if f.get("id").is_some() && f.get("method").is_some() {
                rpc.write(json!({"jsonrpc":"2.0","id":f["id"],"error":{"code":-32601,"message":"Unsupported client request"}})).await?;
                continue;
            }
            if p["sessionId"] != session {
                continue;
            }
            let item = &p["item"];
            match f["method"].as_str() {
                Some("item/started")
                    if item["turnId"] == turn_id && item["kind"] == "agentMessage" =>
                {
                    if let Some(id) = item["itemId"].as_str() {
                        items.insert(id.to_owned());
                    }
                }
                Some("item/delta")
                    if p["field"] == "text"
                        && p["itemId"].as_str().is_some_and(|id| items.contains(id)) =>
                {
                    if let Some(delta) = p["delta"].as_str() {
                        update(Update::Output(delta.into()))?;
                    }
                }
                Some("item/completed")
                    if item["turnId"] == turn_id && item["kind"] == "agentMessage" =>
                {
                    answer = item["text"].as_str().map(str::to_owned);
                }
                Some("session/tokenUsage") if p["turnId"] == turn_id => {
                    if let Some(cursor) = p["viewCursor"].as_str()
                        && let Some(Update::Usage { input, output }) = super::usage(&p["usage"])
                        && usage_cursors.insert(cursor.to_owned())
                    {
                        totals.0 = totals.0.saturating_add(input);
                        totals.1 = totals.1.saturating_add(output);
                        update(Update::Usage {
                            input: totals.0,
                            output: totals.1,
                        })?;
                    }
                }
                Some("turn/completed") if p["turnId"] == turn_id => match p["terminal"].as_str() {
                    Some("completed") => {
                        if let Some(aggregate) = super::usage(&p["usage"]) {
                            update(aggregate)?;
                        }
                        completed = true;
                    }
                    Some("cancelled") => return Err(Error::Interrupted),
                    _ => return Err(protocol(format!("Muse turn ended: {}", p["terminal"]))),
                },
                _ => {}
            }
        }
        update(Update::Finalizing(answer.ok_or_else(|| {
            protocol("Muse ended without an assistant answer")
        })?))?;
        rpc.finish(Ok(())).await
    };
    tokio::select! {biased; _=stop.wait_for(|s|*s)=>Err(Error::Closed),r=run=>r}
}
