use crate::{data, error, parse, text::PythonText};
use huggler::{
    AgentOutput, HugglerOptions, ToolPresentation, WorktreeRoot,
    providers::registry::ProviderInstance,
};
use pyo3::{exceptions::PyValueError, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    name: Option<String>,
    slug: Option<String>,
    storage: StorageConfig,
    output: OutputConfig,
    agent_tools: bool,
    agent_policy: PolicyConfig,
    auto_resume_usage_limits: bool,
    prompt_suggestions: bool,
    instances: Vec<InstanceConfig>,
    codex: CodexConfig,
    claude_agent: ClaudeConfig,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct StorageConfig {
    database_path: Option<PathBuf>,
    worktree_root: Option<RootConfig>,
    worktree_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RootConfig {
    Shared(PathBuf),
    ProjectRelative(PathBuf),
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct OutputConfig {
    name: Option<String>,
    title: Option<String>,
    version: Option<String>,
    instructions: Option<String>,
    tools: Option<BTreeMap<String, ToolConfig>>,
    provider_labels: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolConfig {
    name: Option<String>,
    title: Option<String>,
    description: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PolicyConfig {
    allowed_tools: Option<BTreeSet<String>>,
    defer_tools: bool,
    secret_ttl_seconds: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceConfig {
    id: String,
    driver: huggler::Provider,
    executable: PathBuf,
    #[serde(default)]
    args: Vec<String>,
    home: Option<PathBuf>,
    pi_extension: Option<PathBuf>,
    server_url: Option<String>,
    #[serde(default)]
    request_approvals: bool,
    #[serde(default)]
    workspace_write: bool,
    password_env: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CodexConfig {
    executable: Option<PathBuf>,
    request_approvals: Option<bool>,
    workspace_write: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ClaudeConfig {
    executable: Option<PathBuf>,
}

pub(crate) fn build(
    py: Python<'_>,
    state_dir: Option<PathBuf>,
    config: Option<&Bound<'_, PyAny>>,
    agent_text: Option<Py<PyAny>>,
) -> PyResult<HugglerOptions> {
    let config: Config = config.map(parse).transpose()?.unwrap_or_default();
    let mut options = state_dir.map(HugglerOptions::new).unwrap_or_default();
    match (config.name, config.slug) {
        (Some(name), Some(slug)) => {
            options = options
                .with_name_and_slug(&name, &slug)
                .map_err(|err| error(py, err))?
        }
        (Some(name), None) => options = options.with_name(&name).map_err(|err| error(py, err))?,
        (None, Some(_)) => return Err(PyValueError::new_err("slug requires a name")),
        (None, None) => {}
    }
    if let Some(path) = config.storage.database_path {
        options.storage.database_path = path;
    }
    if let Some(root) = config.storage.worktree_root {
        options.storage.worktree_root = match root {
            RootConfig::Shared(path) => WorktreeRoot::Shared(path),
            RootConfig::ProjectRelative(path) => WorktreeRoot::ProjectRelative(path),
        };
    }
    if let Some(name) = config.storage.worktree_name {
        options.storage.worktree_name = name;
    }

    let output = &mut options.output;
    if let Some(name) = config.output.name {
        output.name = name;
    }
    if let Some(title) = config.output.title {
        output.title = title;
    }
    if let Some(version) = config.output.version {
        output.version = version;
    }
    output.instructions = config.output.instructions;
    if let Some(tools) = config.output.tools {
        output.tools = tools
            .into_iter()
            .map(|(key, tool)| {
                (
                    key,
                    ToolPresentation {
                        name: tool.name,
                        title: tool.title,
                        description: tool.description,
                    },
                )
            })
            .collect();
    }
    if let Some(labels) = config.output.provider_labels {
        output.provider_labels = labels;
    }
    if let Some(callbacks) = agent_text {
        output.text = Arc::new(PythonText::new(py, callbacks, output.text.clone())?);
    }

    options.agent_tools = config.agent_tools;
    options.agent_policy.allowed_tools = config.agent_policy.allowed_tools;
    options.agent_policy.defer_tools = config.agent_policy.defer_tools;
    options.agent_policy.secret_ttl_seconds = config.agent_policy.secret_ttl_seconds;
    options.auto_resume_usage_limits = config.auto_resume_usage_limits;
    options.prompt_suggestions = config.prompt_suggestions;
    for input in config.instances {
        let mut instance = ProviderInstance::new(input.id, input.driver, input.executable);
        instance.args = input.args;
        instance.home = input.home;
        instance.pi_extension = input.pi_extension;
        instance.server_url = input.server_url;
        instance.request_approvals = input.request_approvals;
        instance.workspace_write = input.workspace_write;
        instance.password_env = input.password_env;
        instance
            .set_environment(input.environment)
            .map_err(|err| error(py, err))?;
        options.instances.push(instance);
    }
    if let Some(executable) = config.codex.executable {
        options.codex.executable = executable;
    }
    if let Some(request_approvals) = config.codex.request_approvals {
        options.codex.request_approvals = request_approvals;
    }
    if let Some(workspace_write) = config.codex.workspace_write {
        options.codex.workspace_write = workspace_write;
    }
    if let Some(executable) = config.claude_agent.executable {
        options.claude_agent.executable = executable;
    }
    Ok(options)
}

pub(crate) fn output(py: Python<'_>, output: &AgentOutput) -> PyResult<Py<PyAny>> {
    #[derive(Serialize)]
    struct Presentation<'a> {
        name: &'a str,
        title: &'a str,
        version: &'a str,
        instructions: &'a Option<String>,
        tools: BTreeMap<&'a str, ToolConfig>,
        provider_labels: &'a BTreeMap<String, String>,
    }

    data(
        py,
        &Presentation {
            name: &output.name,
            title: &output.title,
            version: &output.version,
            instructions: &output.instructions,
            tools: output
                .tools
                .iter()
                .map(|(key, tool)| {
                    (
                        key.as_str(),
                        ToolConfig {
                            name: tool.name.clone(),
                            title: tool.title.clone(),
                            description: tool.description.clone(),
                        },
                    )
                })
                .collect(),
            provider_labels: &output.provider_labels,
        },
    )
}
