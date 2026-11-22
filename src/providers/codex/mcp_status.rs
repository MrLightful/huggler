use crate::{
    AgentOutput, McpServerState, McpServerStatus, ProviderInstance, Result,
    providers::{probe::open_codex_probe, rpc::protocol},
};
use serde::{Deserialize, de::IgnoredAny};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusPage {
    data: Vec<Server>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Server {
    name: String,
    tools: serde_json::Map<String, Value>,
    runtime_status: Option<String>,
    auth_status: Option<String>,
    server_info: Option<IgnoredAny>,
    tools_error: Option<IgnoredAny>,
}

impl From<Server> for McpServerStatus {
    fn from(server: Server) -> Self {
        use McpServerState::*;
        let tool_count = server.tools.len();
        let discovered =
            server.tools_error.is_none() && (server.server_info.is_some() || tool_count > 0);
        let state = match server.runtime_status.as_deref() {
            Some("disabled") => Disabled,
            Some("authenticationRequired") => AuthenticationRequired,
            Some("starting") => Starting,
            Some("notStarted") => NotStarted,
            Some("cancelled") => Cancelled,
            status
                if status != Some("connected")
                    && !discovered
                    && server.auth_status.as_deref() == Some("notLoggedIn") =>
            {
                AuthenticationRequired
            }
            status if status == Some("failed") || server.tools_error.is_some() => Failed,
            status if status == Some("connected") || discovered => Connected { tool_count },
            _ => Unknown,
        };
        Self {
            name: server.name,
            state,
        }
    }
}

pub(crate) async fn list(
    instance: &ProviderInstance,
    output: &AgentOutput,
    cwd: &Path,
) -> Result<Option<Vec<McpServerStatus>>> {
    let (mut rpc, _) = open_codex_probe(instance, output, cwd).await?;
    // An unscoped listing misses trusted project configuration. An ephemeral
    // thread loads the checkout's MCP settings without submitting a turn.
    let session = rpc
        .request(2, "thread/start", json!({"cwd": cwd, "ephemeral": true}))
        .await?;
    let thread = session["thread"]["id"]
        .as_str()
        .ok_or_else(|| protocol("missing native thread ID for MCP server discovery"))?;
    let mut cursor = None;
    let mut seen = HashSet::new();
    let mut servers = Vec::new();
    for id in 3.. {
        let mut frame = rpc
            .request_frame(
                id,
                "mcpServerStatus/list",
                json!({
                    "threadId": thread, "detail": "toolsAndAuthOnly", "cursor": cursor,
                }),
            )
            .await?;
        if frame["error"]["code"] == -32601 {
            rpc.finish(Ok(())).await?;
            return Ok(None);
        }
        if let Some(error) = frame.get("error") {
            return Err(protocol(format!(
                "mcpServerStatus/list: {}",
                error["message"].as_str().unwrap_or("request failed")
            )));
        }
        let page: StatusPage = serde_json::from_value(frame["result"].take())?;
        servers.extend(page.data.into_iter().map(McpServerStatus::from));
        let Some(next) = page.next_cursor else { break };
        if !seen.insert(next.clone()) {
            return Err(protocol("mcpServerStatus/list repeated a cursor"));
        }
        cursor = Some(next);
    }
    rpc.finish(Ok(())).await?;
    servers.sort_by_cached_key(|server| (server.name.to_lowercase(), server.name.clone()));
    Ok(Some(servers))
}
