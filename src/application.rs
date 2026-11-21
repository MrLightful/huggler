//! Environment preferences, agent tool eligibility and human-mediated secret references.
use crate::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::Path,
    process::{Command, Stdio},
};

#[derive(Default, Clone, Debug)]
pub struct AgentPolicy {
    /// A host-managed ceiling applied both to discovery and invocation.
    pub allowed_tools: Option<BTreeSet<String>>,
    pub defer_tools: bool,
    /// Maximum seconds between request and consumption. None disables requests.
    pub secret_ttl_seconds: Option<u32>,
}
#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: Theme,
    pub reduce_motion: bool,
}

#[derive(Debug, Serialize)]
pub struct SecretRequest {
    pub id: String,
    pub run: RunId,
    pub label: String,
    pub state: String,
    pub expires: i64,
}
impl Huggler {
    pub fn preferences(&self) -> Result<Preferences> {
        self.shared.access(|db| db.preferences())
    }
    pub fn set_preferences(&self, prefs: &Preferences) -> Result<()> {
        self.shared.access(|db| db.set_preferences(prefs))
    }
    pub fn user_notices(&self, thread: &ThreadId) -> Result<Vec<(RunId, String)>> {
        self.shared.access(|db| db.user_notices(thread))
    }
    pub fn secret_requests(&self, thread: &ThreadId) -> Result<Vec<SecretRequest>> {
        self.shared.access(|db| db.secret_requests(thread))
    }
    /// Approve with exact bytes, or deny with None. Values live only in process memory.
    pub fn answer_secret(&self, thread: &ThreadId, id: &str, value: Option<Vec<u8>>) -> Result<()> {
        self.shared.access(|db| db.answer_secret(thread, id, value))
    }
    /// Read a host-selected 1Password reference using an already authenticated CLI.
    pub fn approve_onepassword(
        &self,
        thread: &ThreadId,
        id: &str,
        executable: &Path,
        reference: &str,
    ) -> Result<()> {
        if !reference.starts_with("op://") || reference.contains(['\n', '\r']) {
            return Err(Error::Invalid(
                "expected a 1Password secret reference".into(),
            ));
        }
        if !self
            .secret_requests(thread)?
            .iter()
            .any(|r| r.id == id && r.state == "requested")
        {
            return Err(Error::Forbidden);
        }
        let output = Command::new(executable)
            .args(["read", "--no-newline", reference])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(Error::LoginRequired);
        }
        self.answer_secret(thread, id, Some(output.stdout))
    }
    /// Redeem the agent's opaque reference once in the project that requested it.
    pub fn consume_secret(&self, project: &ProjectId, reference: &str) -> Result<Vec<u8>> {
        self.shared
            .access(|db| db.consume_secret(project, reference))
    }
}
