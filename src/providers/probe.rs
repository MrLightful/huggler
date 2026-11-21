//! On-demand protocol discovery. Probes never submit a prompt.
use super::{
    claude_agent, muse, opencode, pi,
    rpc::{Rpc, protocol},
};
use crate::{Error, Provider, ProviderInstance, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderReport {
    pub transport: String,
    pub version: Option<String>,
    /// Some(true) means the provider confirmed credentials.
    /// None means the protocol did not verify login.
    pub authenticated: Option<bool>,
    pub models: Vec<String>,
    pub resume: bool,
    pub agent_tools: bool,
}

impl ProviderReport {
    pub(super) fn new(transport: &str) -> Self {
        Self {
            transport: transport.into(),
            version: None,
            authenticated: None,
            models: vec![],
            resume: false,
            agent_tools: false,
        }
    }
}

pub(super) fn ids(items: &Value, field: &str) -> Vec<String> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item[field].as_str().map(str::to_owned))
        .collect()
}

pub(crate) async fn inspect(
    instance: &ProviderInstance,
    output: &crate::AgentOutput,
    cwd: &Path,
) -> Result<ProviderReport> {
    let mut command = instance.command();
    let mut report = match instance.driver {
        Provider::OpenCode => return opencode::probe(instance, output, cwd).await,
        Provider::Pi => pi::probe(instance, output, cwd).await?,
        Provider::Muse => muse::probe(instance, output, cwd).await?,
        Provider::Codex => {
            command.arg("app-server");
            let mut rpc = Rpc::open(command, cwd, output)?;
            let mut report = ProviderReport::new("Codex app-server");
            let init = rpc
                .request(1, "initialize", json!({"clientInfo":output.client_info()}))
                .await?;
            report.version = init["userAgent"].as_str().and_then(|s| {
                s.split_whitespace()
                    .find_map(|t| t.split_once('/').map(|(_, v)| v.to_owned()))
            });
            rpc.write(json!({"method":"initialized"})).await?;
            let account = rpc
                .request(2, "account/read", json!({"refreshToken":false}))
                .await?;
            if account["account"].is_null() && account["requiresOpenaiAuth"] == true {
                return Err(Error::LoginRequired);
            }
            report.authenticated = (!account["account"].is_null()).then_some(true);
            let mut cursor = Value::Null;
            let mut seen = std::collections::HashSet::new();
            loop {
                let page = rpc
                    .request(3, "model/list", json!({"cursor":cursor}))
                    .await?;
                report.models.extend(ids(&page["data"], "id"));
                cursor = page["nextCursor"].clone();
                let Some(next) = cursor.as_str() else { break };
                if !seen.insert(next.to_owned()) {
                    return Err(protocol("model/list repeated a cursor"));
                }
            }
            report.resume = true;
            report.agent_tools = true;
            rpc.finish(Ok(())).await?;
            report
        }
        driver => {
            let mut rpc = Rpc::open(command, cwd, output)?;
            let init = claude_agent::initialize(&mut rpc, driver, output).await?;
            let mut report = ProviderReport::new(&format!("ACP v{}", init["protocolVersion"]));
            report.version = init["agentInfo"]["version"].as_str().map(str::to_owned);
            report.resume = init["agentCapabilities"]["loadSession"] == true;
            report.agent_tools = init["agentCapabilities"]["mcpCapabilities"]["http"] == true;
            let info = claude_agent::setup_request(
                &mut rpc,
                2,
                "session/new",
                json!({"cwd":cwd,"mcpServers":[]}),
            )
            .await?;
            if info["sessionId"].as_str().is_none_or(str::is_empty) {
                return Err(protocol("missing ACP session ID"));
            }
            // A successful session/new can precede a login challenge at prompt time.
            report.models = ids(&info["models"]["availableModels"], "modelId");
            if let Some(opt) = claude_agent::model_option(&info) {
                for entry in opt["options"].as_array().into_iter().flatten() {
                    if let Some(value) = entry["value"].as_str() {
                        report.models.push(value.into());
                    }
                    report.models.extend(ids(&entry["options"], "value"));
                }
            }
            rpc.finish(Ok(())).await?;
            report
        }
    };
    report.models.sort();
    report.models.dedup();
    if report.version.is_none() && instance.driver != Provider::Muse {
        let mut version = instance.command();
        version.arg("--version");
        if let Ok(mut rpc) = Rpc::open(version, cwd, output)
            && let Ok(Ok(line)) =
                tokio::time::timeout(std::time::Duration::from_secs(2), rpc.line()).await
            && line.len() <= 128
            && !line.starts_with('{')
            && line.contains(|c: char| c.is_ascii_digit())
        {
            report.version = Some(line);
        }
    }
    Ok(report)
}
