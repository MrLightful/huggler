//! Host-owned instance configuration. Each worker receives only its selected configuration.
use super::{claude_agent::ClaudeAgentOptions, codex::CodexOptions};
use crate::{Error, Provider, ProviderInstanceId, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderInstance {
    pub id: ProviderInstanceId,
    pub driver: Provider,
    pub executable: PathBuf,
    /// ACP invocation arguments, Muse serve options, or arguments before the protocol command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Absolute instance data directory for drivers that support private homes.
    #[serde(default)]
    pub home: Option<PathBuf>,
    /// Absolute path to the separately installed Huggler tool extension for Pi.
    #[serde(default)]
    pub pi_extension: Option<PathBuf>,
    /// An existing loopback OpenCode HTTP server. The host never changes its MCP configuration.
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub request_approvals: bool,
    /// Allow workspace writes on this Codex instance.
    #[serde(default)]
    pub workspace_write: bool,
    /// Environment variable containing the external OpenCode server password.
    #[serde(default)]
    pub password_env: Option<String>,
    #[serde(skip)]
    environment: ProviderEnvironment,
}

#[derive(Clone, Default)]
struct ProviderEnvironment(BTreeMap<String, String>);

impl fmt::Debug for ProviderEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_set().entries(self.0.keys()).finish()
    }
}

impl ProviderInstance {
    pub fn new(
        id: impl Into<ProviderInstanceId>,
        driver: Provider,
        executable: impl Into<PathBuf>,
    ) -> Self {
        Self {
            id: id.into(),
            driver,
            executable: executable.into(),
            args: vec![],
            home: None,
            pi_extension: None,
            server_url: None,
            request_approvals: false,
            workspace_write: false,
            password_env: None,
            environment: ProviderEnvironment::default(),
        }
    }

    /// Set the environment passed to subprocesses launched for this instance.
    pub fn set_environment(&mut self, values: BTreeMap<String, String>) -> Result<()> {
        if values
            .iter()
            .any(|(key, value)| key.is_empty() || key.contains(['=', '\0']) || value.contains('\0'))
        {
            return Err(Error::Invalid(
                "invalid provider environment variable".into(),
            ));
        }
        self.environment = ProviderEnvironment(values);
        Ok(())
    }

    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment.0
    }

    fn home_variable(&self) -> Option<&'static str> {
        match self.driver {
            Provider::Codex => Some("CODEX_HOME"),
            Provider::ClaudeAgent => Some("CLAUDE_CONFIG_DIR"),
            Provider::Pi => Some("PI_CODING_AGENT_DIR"),

            _ => None,
        }
    }

    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.args(&self.args).envs(self.environment());
        if let (Some(key), Some(path)) = (self.home_variable(), &self.home) {
            command.env(key, path);
        }
        command
    }

    pub(crate) fn continuation_key(&self) -> Result<String> {
        let environment_path = |name: &str| {
            self.environment()
                .get(name)
                .map(PathBuf::from)
                .or_else(|| std::env::var_os(name).map(PathBuf::from))
        };
        let home = self
            .home
            .clone()
            .or_else(|| self.home_variable().and_then(environment_path))
            .or_else(|| environment_path("HOME"));
        let identity = serde_json::to_vec(&(
            self.driver,
            &self.executable,
            &self.args,
            home,
            &self.server_url,
            &self.password_env,
        ))?;
        Ok(Sha256::digest(identity)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    fn validate(&self) -> Result<()> {
        let id = self.id.as_str();
        if id.is_empty()
            || id.len() > 100
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(Error::Invalid(format!(
                "invalid provider instance ID {:?}",
                self.id
            )));
        }
        if self.executable.as_os_str().is_empty()
            || (!self.executable.is_absolute() && self.executable.components().count() != 1)
        {
            return Err(Error::Invalid(format!(
                "{} requires an executable name or absolute path",
                self.id
            )));
        }
        if self
            .home
            .as_deref()
            .is_some_and(|p| !Path::is_absolute(p) || self.home_variable().is_none())
        {
            return Err(Error::Invalid(format!(
                "{} does not support that instance home",
                self.id
            )));
        }
        if self
            .pi_extension
            .as_ref()
            .is_some_and(|p| self.driver != Provider::Pi || !p.is_absolute())
        {
            return Err(Error::Invalid(format!(
                "{}: piExtension requires a Pi instance and an absolute file path",
                self.id
            )));
        }
        if Provider::from_key(id).is_ok_and(|driver| driver != self.driver) {
            return Err(Error::Invalid(format!(
                "instance {} names a different driver",
                self.id
            )));
        }
        if self.password_env.as_ref().is_some_and(|k| {
            self.server_url.is_none()
                || k.is_empty()
                || !k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        }) {
            return Err(Error::Invalid(
                "passwordEnv requires an environment variable name and external server".into(),
            ));
        }
        if let Some(url) = &self.server_url {
            let uri = url
                .parse::<axum::http::Uri>()
                .map_err(|_| Error::Invalid("invalid OpenCode server URL".into()))?;
            if self.driver != Provider::OpenCode
                || uri.scheme_str() != Some("http")
                || !matches!(uri.host(), Some("127.0.0.1" | "[::1]"))
                || uri.authority().is_some_and(|a| a.as_str().contains('@'))
                || uri.path() != "/"
                || uri.query().is_some()
            {
                return Err(Error::Invalid(
                    "external OpenCode requires a loopback HTTP origin".into(),
                ));
            }
        }
        Ok(())
    }
}

pub(crate) type Registry = BTreeMap<String, ProviderInstance>;

pub(crate) fn registry(
    codex: &CodexOptions,
    claude: &ClaudeAgentOptions,
    extra: &[ProviderInstance],
) -> Result<Registry> {
    let mut first = ProviderInstance::new("codex", Provider::Codex, &codex.executable);
    first.request_approvals = codex.request_approvals;
    first.workspace_write = codex.workspace_write;
    let second = ProviderInstance::new("claudeAgent", Provider::ClaudeAgent, &claude.executable);
    let mut registry = Registry::new();
    for instance in extra.iter().cloned() {
        instance.validate()?;
        let id = instance.id.0.clone();
        if registry.insert(id.clone(), instance).is_some() {
            return Err(Error::Invalid(format!("duplicate provider instance {id}")));
        }
    }
    for instance in [first, second] {
        if !registry.contains_key(instance.id.as_str()) {
            instance.validate()?;
            registry.insert(instance.id.as_str().to_owned(), instance);
        }
    }
    Ok(registry)
}
