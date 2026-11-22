//! On-demand protocol discovery. Probes never submit a prompt.
use super::{
    claude_agent, codex, muse, opencode, pi,
    rpc::{Rpc, protocol},
};
use crate::{Error, Provider, ProviderInstance, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderReport {
    pub transport: String,
    pub version: Option<String>,
    /// Some(true) means the provider confirmed credentials.
    /// None means the protocol did not verify login.
    pub authenticated: Option<bool>,
    pub models: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_details: BTreeMap<String, ModelDetails>,
    pub resume: bool,
    pub agent_tools: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelDetails {
    pub name: String,
    pub is_default: bool,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
    pub fast_tier: Option<String>,
    pub default_service_tier: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderSkill {
    pub name: String,
    pub path: PathBuf,
    pub description: Option<String>,
    pub scope: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct McpServerStatus {
    pub name: String,
    pub state: McpServerState,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpServerState {
    Connected { tool_count: usize },
    AuthenticationRequired,
    Failed,
    Starting,
    NotStarted,
    Cancelled,
    Disabled,
    Unknown,
}

impl ProviderReport {
    pub(super) fn new(transport: &str) -> Self {
        Self {
            transport: transport.into(),
            version: None,
            authenticated: None,
            models: vec![],
            model_details: BTreeMap::new(),
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
    let mut report = match instance.driver {
        Provider::OpenCode => return opencode::probe(instance, output, cwd).await,
        Provider::Pi => pi::probe(instance, output, cwd).await?,
        Provider::Muse => muse::probe(instance, output, cwd).await?,
        Provider::Codex => {
            let (mut rpc, init) = open_codex_probe(instance, output, cwd).await?;
            let mut report = ProviderReport::new("Codex app-server");
            report.version = init["userAgent"].as_str().and_then(|s| {
                s.split_whitespace()
                    .find_map(|t| t.split_once('/').map(|(_, v)| v.to_owned()))
            });
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
                for model in page["data"].as_array().into_iter().flatten() {
                    let Some(id) = model["id"].as_str() else {
                        continue;
                    };
                    report.models.push(id.into());
                    let mut tiers = ids(&model["serviceTiers"], "id");
                    if tiers.is_empty() {
                        tiers.extend(
                            model["additionalSpeedTiers"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .map(str::to_owned),
                        );
                    }
                    report.model_details.insert(
                        id.into(),
                        ModelDetails {
                            name: model["displayName"]
                                .as_str()
                                .filter(|name| !name.trim().is_empty())
                                .unwrap_or(id)
                                .into(),
                            is_default: model["isDefault"] == true,
                            efforts: ids(&model["supportedReasoningEfforts"], "reasoningEffort"),
                            default_effort: model["defaultReasoningEffort"]
                                .as_str()
                                .map(str::to_owned),
                            fast_tier: tiers
                                .into_iter()
                                .find(|tier| matches!(tier.as_str(), "fast" | "priority")),
                            default_service_tier: model["defaultServiceTier"]
                                .as_str()
                                .map(str::to_owned),
                        },
                    );
                }
                cursor = page["nextCursor"].clone();
                let Some(next) = cursor.as_str() else { break };
                if !seen.insert(next.to_owned()) {
                    return Err(protocol("model/list repeated a cursor"));
                }
            }
            let config = rpc
                .request_frame(4, "config/read", json!({"cwd":cwd,"includeLayers":false}))
                .await?;
            let settings = &config["result"]["config"];
            for (id, details) in &mut report.model_details {
                if let Some(configured) = settings["model"].as_str() {
                    details.is_default = id == configured;
                }
                if let Some(effort) = settings["model_reasoning_effort"].as_str()
                    && details.efforts.iter().any(|supported| supported == effort)
                {
                    details.default_effort = Some(effort.into());
                }
                if let Some(tier) = settings["service_tier"].as_str() {
                    details.default_service_tier = Some(tier.into());
                }
            }
            if let Some(configured) = settings["model"].as_str() {
                report
                    .model_details
                    .entry(configured.into())
                    .or_insert_with(|| ModelDetails {
                        name: configured.into(),
                        is_default: true,
                        default_effort: settings["model_reasoning_effort"]
                            .as_str()
                            .map(str::to_owned),
                        ..Default::default()
                    });
            }
            report.resume = true;
            report.agent_tools = true;
            rpc.finish(Ok(())).await?;
            report
        }
        driver => {
            let mut rpc = Rpc::open(instance.command(), cwd, output)?;
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
            let legacy = &info["models"];
            let option = claude_agent::model_option(&info);
            report.models = ids(&legacy["availableModels"], "modelId");
            let entries = option
                .and_then(|opt| opt["options"].as_array())
                .into_iter()
                .flatten();
            for entry in entries.clone() {
                if let Some(value) = entry["value"].as_str() {
                    report.models.push(value.into());
                }
                report.models.extend(ids(&entry["options"], "value"));
            }
            if let Some(id) = option
                .and_then(|opt| opt["currentValue"].as_str())
                .or_else(|| legacy["currentModelId"].as_str())
                .filter(|id| *id != "default" && !id.is_empty())
            {
                let name = entries
                    .flat_map(|entry| {
                        std::iter::once(entry)
                            .chain(entry["options"].as_array().into_iter().flatten())
                    })
                    .find(|model| model["value"] == id)
                    .or_else(|| {
                        legacy["availableModels"]
                            .as_array()?
                            .iter()
                            .find(|model| model["modelId"] == id)
                    })
                    .and_then(|model| model["name"].as_str())
                    .unwrap_or(id);
                report.model_details.insert(
                    id.into(),
                    ModelDetails {
                        name: name.into(),
                        is_default: true,
                        ..Default::default()
                    },
                );
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

pub(crate) async fn list_skills(
    instance: &ProviderInstance,
    output: &crate::AgentOutput,
    cwd: &Path,
) -> Result<Option<Vec<ProviderSkill>>> {
    if instance.driver != Provider::Codex {
        return Ok(None);
    }
    let (mut rpc, _) = open_codex_probe(instance, output, cwd).await?;
    let frame = rpc
        .request_frame(2, "skills/list", json!({"cwds":[cwd]}))
        .await?;
    let result = if frame["error"]["code"] == -32601 {
        None
    } else if let Some(error) = frame.get("error") {
        return Err(protocol(format!(
            "skills/list: {}",
            error["message"].as_str().unwrap_or("request failed")
        )));
    } else {
        Some(codex_skills(&frame["result"], cwd)?)
    };
    rpc.finish(Ok(())).await?;
    Ok(result)
}

pub(crate) async fn list_mcp_server_status(
    instance: &ProviderInstance,
    output: &crate::AgentOutput,
    cwd: &Path,
) -> Result<Option<Vec<McpServerStatus>>> {
    match instance.driver {
        Provider::Codex => codex::mcp_status::list(instance, output, cwd).await,
        _ => Ok(None),
    }
}

pub(super) async fn open_codex_probe(
    instance: &ProviderInstance,
    output: &crate::AgentOutput,
    cwd: &Path,
) -> Result<(Rpc, Value)> {
    let mut command = instance.command();
    command.arg("app-server");
    let mut rpc = Rpc::open(command, cwd, output)?;
    let init = rpc
        .request(
            1,
            "initialize",
            json!({"clientInfo":output.client_info(),"capabilities":{"experimentalApi":true}}),
        )
        .await?;
    rpc.write(json!({"method":"initialized"})).await?;
    Ok((rpc, init))
}

fn codex_skills(response: &Value, cwd: &Path) -> Result<Vec<ProviderSkill>> {
    let entries = response["data"]
        .as_array()
        .ok_or_else(|| protocol("missing skills/list data"))?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let Some(entry) = entries.iter().find(|entry| {
        entry["cwd"]
            .as_str()
            .is_some_and(|path| Path::new(path) == cwd)
    }) else {
        return Err(protocol("skills/list returned a different checkout"));
    };
    let mut skills: Vec<_> = entry["skills"]
        .as_array()
        .ok_or_else(|| protocol("missing skills for the requested checkout"))?
        .iter()
        .filter(|skill| skill["enabled"] == true)
        .filter_map(|skill| {
            let name = skill["name"].as_str()?;
            if name.is_empty() || name.chars().any(char::is_whitespace) {
                return None;
            }
            let path = PathBuf::from(skill["path"].as_str()?);
            if !path.is_absolute() {
                return None;
            }
            Some(ProviderSkill {
                name: name.into(),
                path,
                description: [
                    skill["shortDescription"].as_str(),
                    skill["interface"]["shortDescription"].as_str(),
                    skill["description"].as_str(),
                ]
                .into_iter()
                .flatten()
                .find(|text| !text.trim().is_empty())
                .map(str::to_owned),
                scope: skill["scope"].as_str().map(str::to_owned),
            })
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    skills.dedup_by(|a, b| a.name == b.name && a.path == b.path);
    Ok(skills)
}
