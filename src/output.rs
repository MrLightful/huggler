//! Host-selected presentation for provider connections and agent conversations.
use crate::{Error, RunStatus};
use std::{collections::BTreeMap, fmt::Debug, sync::Arc};

/// Keyed by the built-in tool name in [`AgentOutput::tools`].
#[derive(Clone, Debug, Default)]
pub struct ToolPresentation {
    pub name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
}

/// Settings shared by provider adapters, the agent tool server and the conversation store.
#[derive(Clone, Debug)]
pub struct AgentOutput {
    /// Protocol client name and MCP server registration key. ASCII letters, digits, '-' and '_'.
    pub name: String,
    pub title: String,
    pub version: String,
    /// Optional instructions advertised at MCP initialization.
    pub instructions: Option<String>,
    /// Overrides for built-in orchestration tools. Policy continues to use the built-in keys.
    pub tools: BTreeMap<String, ToolPresentation>,
    /// Display labels keyed by durable provider instance ID.
    pub provider_labels: BTreeMap<String, String>,
    pub text: Arc<dyn AgentText>,
}

impl Default for AgentOutput {
    fn default() -> Self {
        Self {
            name: "huggler".into(),
            title: "Huggler".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            instructions: None,
            tools: BTreeMap::new(),
            provider_labels: BTreeMap::from([
                ("codex".into(), "Codex".into()),
                ("claudeAgent".into(), "Claude Agent".into()),
            ]),
            text: Arc::new(DefaultAgentText),
        }
    }
}

impl AgentOutput {
    pub fn provider_label<'a>(&'a self, id: &'a str) -> &'a str {
        self.provider_labels.get(id).map_or(id, String::as_str)
    }

    pub(crate) fn client_info(&self) -> serde_json::Value {
        serde_json::json!({"name":self.name,"title":self.title,"version":self.version})
    }

    pub(crate) fn tool_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.tools
            .get(key)
            .and_then(|t| t.name.as_deref())
            .unwrap_or(key)
    }
}

/// Override individual methods to change the text the crate generates. These synchronous
/// callbacks can run inside a storage transaction. Keep them fast and free of runtime calls.
pub trait AgentText: Send + Sync + Debug {
    fn history(&self, host_title: &str, history_json: &str, message: &str) -> String {
        format!("Conversation history from {host_title} (JSON)\n{history_json}\n\n{message}")
    }
    fn peer_message(&self, host_title: &str, origin_json: &str, message: &str) -> String {
        format!("Peer message from {host_title} {origin_json}\n\n{message}")
    }
    fn app_context(&self, context: &str, message: &str) -> String {
        format!("MCP App context (host-approved data):\n{context}\n\n{message}")
    }
    fn attachment(&self, name: &str, content: &str) -> String {
        format!("Text attachment {}\n{content}", serde_json::json!(name))
    }
    fn with_attachments(&self, message: &str, attachments: &[String]) -> String {
        format!("{message}\n\n{}", attachments.join("\n\n"))
    }
    /// Used when a peer has no explicit title. Position is one-based.
    fn peer_title(&self, prompt: Option<&str>, position: usize) -> String {
        prompt.map_or_else(|| format!("Peer {position}"), short_title)
    }
    fn delegated_title(&self) -> &str {
        "Delegated task"
    }
    fn delegation(&self, role: Option<&str>, task: &str) -> String {
        match role {
            Some(role) => format!("Role: {role}\n\n{task}"),
            None => task.into(),
        }
    }
    fn task_results(&self, host_title: &str, results_json: &str) -> String {
        format!("{host_title} delegated task results\n{results_json}")
    }
    fn interrupted_by_agent(&self, reason: Option<&str>) -> String {
        let message = self.label("Stopped by an agent");
        match reason {
            Some(reason) => format!("{message}: {reason}"),
            None => message,
        }
    }
    fn resume_after_limit(&self) -> &str {
        "Continue where you left off."
    }
    fn suggestion_prompt(&self) -> &str {
        "Suggest one short follow-up the user might type next. Return only that message, or empty text if none is useful."
    }
    fn guest_message(&self, name: &str, message: &str) -> String {
        format!("Guest {name}: {message}")
    }
    fn remote_title(&self) -> &str {
        "Remote task"
    }
    fn remote_result(&self, key: &str, status: RunStatus, result: &str) -> String {
        format!("Remote task {key} {status:?}\n{result}")
    }
    fn scheduled_title(&self) -> &str {
        "Scheduled task"
    }
    fn repository_update(&self, repo: &str, item: &str, state: &str) -> String {
        format!("Repository update for {repo} item {item}\n{state}")
    }
    /// Localize a crate-authored approval caption, protocol diagnostic or MCP schema annotation.
    fn label(&self, default: &str) -> String {
        default.into()
    }
    /// Format a failed run for its stored history and delegated result.
    fn run_error(&self, error: &Error) -> String {
        self.label(&error.to_string())
    }
    /// Shape the structured error returned by an agent tool.
    fn tool_error(&self, error: &Error) -> serde_json::Value {
        serde_json::json!(error)
    }
}

pub(crate) fn short_title(detail: &str) -> String {
    let detail = detail.trim();
    if detail.chars().count() <= 80 {
        return detail.into();
    }
    let mut title: String = detail.chars().take(77).collect();
    title.push_str("...");
    title
}

#[derive(Debug)]
struct DefaultAgentText;
impl AgentText for DefaultAgentText {}
