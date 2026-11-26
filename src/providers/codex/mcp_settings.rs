use crate::providers::{
    probe::open_codex_probe,
    rpc::{Rpc, protocol},
};
use crate::{AgentOutput, Error, Provider, ProviderInstance, Result};
use serde_json::{Value, json};
use std::path::Path;

/// MCP settings in the base Codex user config for one provider instance.
pub struct McpSettings {
    pub servers: Vec<McpServer>,
    pub revision: String,
}

pub struct McpServer {
    pub name: String,
    pub enabled: bool,
    pub connection: Option<McpConnection>,
}

pub enum McpConnection {
    Stdio { command: String, args: Vec<String> },
    Http { url: String },
}

impl McpConnection {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Http { url }
                if !url::Url::parse(url).is_ok_and(|url| {
                    matches!(url.scheme(), "http" | "https") && url.has_host()
                }) =>
            {
                Err("Enter a valid HTTP or HTTPS server URL.")
            }
            Self::Stdio { command, .. } if command.trim().is_empty() || command.contains('\0') => {
                Err("Enter the executable that launches this server.")
            }
            Self::Stdio { args, .. } if args.iter().any(|arg| arg.contains('\0')) => {
                Err("Remove null characters from the server arguments.")
            }
            _ => Ok(()),
        }
    }
}

pub enum McpEdit {
    Add(McpConnection),
    Edit(McpConnection),
    SetEnabled(bool),
    Remove,
}

/// Result of saving the base user configuration.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum McpWriteOutcome {
    Saved,
    Overridden,
}

impl McpServer {
    pub fn valid_name(name: &str) -> bool {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    }

    fn from_config(name: &str, value: &Value) -> Self {
        let connection = if let Some(command) = value["command"].as_str() {
            let args = value["args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            Some(McpConnection::Stdio {
                command: command.into(),
                args,
            })
        } else {
            value["url"]
                .as_str()
                .map(|url| McpConnection::Http { url: url.into() })
        };
        Self {
            name: name.into(),
            enabled: value["enabled"].as_bool().unwrap_or(true),
            connection,
        }
    }
}

pub(crate) async fn read(
    instance: &ProviderInstance,
    output: &AgentOutput,
    cwd: &Path,
) -> Result<Option<McpSettings>> {
    if instance.driver != Provider::Codex {
        return Ok(None);
    }
    let (mut rpc, _) = open_codex_probe(instance, output, cwd).await?;
    let settings = read_user_settings(&mut rpc).await?;
    rpc.finish(Ok(())).await?;
    Ok(settings)
}

async fn read_user_settings(rpc: &mut Rpc) -> Result<Option<McpSettings>> {
    let response = rpc
        .request_frame(2, "config/read", json!({"includeLayers": true}))
        .await?;
    if response["error"]["code"] == -32601 {
        return Ok(None);
    }
    let layer = response["result"]["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|layer| layer["name"]["type"] == "user" && layer["name"]["profile"].is_null());
    let Some(layer) = layer else {
        return Err(protocol("Couldn’t read the Codex user configuration."));
    };
    let revision = layer["version"]
        .as_str()
        .ok_or_else(|| protocol("Missing Codex user configuration revision."))?;
    let servers = layer["config"]["mcp_servers"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| McpServer::from_config(name, value))
        .collect();
    Ok(Some(McpSettings {
        servers,
        revision: revision.into(),
    }))
}

pub(crate) async fn write(
    instance: &ProviderInstance,
    output: &AgentOutput,
    cwd: &Path,
    revision: &str,
    name: &str,
    edit: McpEdit,
) -> Result<McpWriteOutcome> {
    if instance.driver != Provider::Codex
        || revision.is_empty()
        || matches!(&edit, McpEdit::Add(_)) && !McpServer::valid_name(name)
    {
        return Err(Error::Invalid("Invalid Codex MCP settings edit.".into()));
    }
    if name == output.name && !matches!(edit, McpEdit::Remove) {
        return Err(Error::Invalid(format!(
            "The MCP server name {name} is reserved for Huggler’s built-in tools."
        )));
    }
    if let McpEdit::Add(connection) | McpEdit::Edit(connection) = &edit {
        connection
            .validate()
            .map_err(|message| Error::Invalid(message.into()))?;
    }
    let (mut rpc, _) = open_codex_probe(instance, output, cwd)
        .await
        .map_err(|_| protocol("Couldn’t open the Codex user configuration."))?;
    let settings = read_user_settings(&mut rpc)
        .await?
        .ok_or_else(|| protocol("This Codex version does not expose MCP settings."))?;
    if settings.revision != revision {
        return Err(config_conflict());
    }
    let server = settings.servers.iter().find(|server| server.name == name);
    match (&edit, server) {
        (McpEdit::Add(_), Some(_)) => {
            return Err(Error::Invalid(format!(
                "An MCP server named {name} already exists."
            )));
        }
        (McpEdit::Add(_), None) => {}
        (_, None) => return Err(Error::NotFound("Codex MCP server".into())),
        (McpEdit::Edit(connection), Some(server))
            if !server.connection.as_ref().is_some_and(|current| {
                std::mem::discriminant(current) == std::mem::discriminant(connection)
            }) =>
        {
            return Err(Error::Invalid(
                "The MCP server transport cannot be changed by a connection edit.".into(),
            ));
        }
        _ => {}
    }
    // Config key paths use TOML syntax. Quote names that contain dots or spaces.
    let path = format!("mcp_servers.{}", serde_json::to_string(name)?);
    let change = |key_path: String, value: Value| json!({"keyPath": key_path, "value": value, "mergeStrategy": "replace"});
    let edits = match edit {
        McpEdit::Add(McpConnection::Stdio { command, args }) => {
            vec![change(path, json!({"command": command, "args": args}))]
        }
        McpEdit::Add(McpConnection::Http { url }) => vec![change(path, json!({"url": url}))],
        McpEdit::Edit(McpConnection::Stdio { command, args }) => vec![
            change(format!("{path}.command"), json!(command)),
            change(format!("{path}.args"), json!(args)),
        ],
        McpEdit::Edit(McpConnection::Http { url }) => {
            vec![change(format!("{path}.url"), json!(url))]
        }
        McpEdit::SetEnabled(enabled) => vec![change(format!("{path}.enabled"), json!(enabled))],
        McpEdit::Remove => vec![change(path, Value::Null)],
    };
    let response = rpc
        .request_frame(
            3,
            "config/batchWrite",
            json!({"expectedVersion": revision, "edits": edits}),
        )
        .await?;
    rpc.finish(Ok(())).await?;
    if response["error"]["data"]["config_write_error_code"] == "configVersionConflict" {
        return Err(config_conflict());
    }
    match response["result"]["status"].as_str() {
        Some("ok") => Ok(McpWriteOutcome::Saved),
        Some("okOverridden") => Ok(McpWriteOutcome::Overridden),
        // Config parse errors may include lines containing credentials.
        _ => Err(protocol("Couldn’t save the Codex user configuration.")),
    }
}

fn config_conflict() -> Error {
    protocol("Codex configuration changed. Refresh MCP servers and try again.")
}
