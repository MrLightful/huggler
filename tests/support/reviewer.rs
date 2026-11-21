use huggler::{HugglerOptions, PermissionRequest, Provider};
use std::{fs, path::Path};

#[derive(Clone, Copy)]
pub enum Reviewer {
    Codex,
    ScriptedAcp,
    ClaudeAgent,
}

impl Reviewer {
    pub fn provider(self) -> Provider {
        match self {
            Self::Codex => Provider::Codex,
            Self::ScriptedAcp | Self::ClaudeAgent => Provider::ClaudeAgent,
        }
    }

    pub fn configure(self, options: &mut HugglerOptions, repo: &Path) {
        match self {
            Self::Codex => {}
            Self::ScriptedAcp => {
                options.claude_agent.executable =
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/peer-acp.py");
            }
            Self::ClaudeAgent => {
                options.claude_agent.executable = std::env::var_os("HUGGLER_CLAUDE_ACP")
                    .unwrap_or_else(|| "claude-agent-acp".into())
                    .into();
                // Release the reviewer's first tool call once the Codex author is idle.
                let settings = repo.join(".claude");
                fs::create_dir(&settings).unwrap();
                fs::write(
                    settings.join("settings.local.json"),
                    r#"{"permissions":{"ask":["mcp__huggler__thread_list","mcp__huggler__thread_send"]}}"#,
                )
                .unwrap();
            }
        }
    }
}

pub fn requested_huggler_tool(request: &PermissionRequest, provider: Provider) -> &str {
    match provider {
        Provider::Codex => {
            let input = &request.tool_call["rawInput"];
            assert_eq!(input["serverName"], "huggler");
            input["_meta"]["tool_name"].as_str().unwrap()
        }
        Provider::ClaudeAgent => {
            let meta = &request.tool_call["_meta"]["claudeCode"];
            assert_eq!(meta["mcpServer"]["name"], "huggler");
            meta["toolName"]
                .as_str()
                .unwrap()
                .strip_prefix("mcp__huggler__")
                .unwrap()
        }
        other => panic!("unexpected reviewer {}", other.key()),
    }
}

pub fn allow_once(request: &PermissionRequest) -> String {
    request
        .options
        .iter()
        .find(|option| option.kind == "allow_once")
        .expect("expected a one-time permission choice")
        .option_id
        .clone()
}
