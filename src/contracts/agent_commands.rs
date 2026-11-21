//! Caller-scoped thread creation and delegation commands shared by the runtime and store.
use super::{ProviderInstanceId, RunId, ThreadId};
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Delegation {
    pub task: String,
    pub client_request_id: String,
    /// Omit to use the parent's configured provider.
    pub target: Option<ProviderTarget>,
    pub title: Option<String>,
    pub role: Option<String>,
    #[serde(default)]
    pub isolation: Isolation,
}

#[derive(Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderTarget {
    #[schemars(with = "String")]
    pub provider_instance_id: ProviderInstanceId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateThreads {
    pub client_request_id: String,
    #[schemars(length(min = 1, max = 20))]
    pub threads: Vec<CreatePeer>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreatePeer {
    pub title: Option<String>,
    /// Omit to inherit the caller's provider.
    pub target: Option<ProviderTarget>,
    /// Omit to create an idle peer.
    pub prompt: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatedPeer {
    pub thread_id: ThreadId,
    pub run_id: Option<RunId>,
}

#[derive(Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Isolation {
    #[default]
    Shared,
    Worktree,
}
