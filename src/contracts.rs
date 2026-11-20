//! Serializable application commands, projections and thread-scoped stream frames.
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
id!(CommandId);
id!(ThreadId);
id!(RunId);
id!(MessageId);
id!(TaskId);
id!(ProjectId);
id!(RequestId);
id!(NativeThreadId);
id!(NativeTurnId);

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, rmcp::schemars::JsonSchema,
)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    #[default]
    Codex,
    ClaudeAgent,
}

impl Provider {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeAgent => "claudeAgent",
        }
    }
    pub(crate) fn from_key(value: &str) -> rusqlite::Result<Self> {
        match value {
            "codex" => Ok(Self::Codex),
            "claudeAgent" => Ok(Self::ClaudeAgent),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: RequestId,
    pub run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_child: Option<NativeChildTurn>,
    pub tool_call: serde_json::Value,
    pub options: Vec<PermissionOption>,
}

/// Structured questions from a provider turn. Each question has a provider-local
/// string `id`. The provider's option descriptions and previews are retained.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserInputRequest {
    pub id: RequestId,
    pub run_id: RunId,
    pub questions: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(untagged)]
pub enum UserAnswer {
    Text(String),
    Choices(Vec<String>),
}

pub type UserAnswers = std::collections::BTreeMap<String, UserAnswer>;

/// The provider-native turn requesting a decision within the owning run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeChildTurn {
    pub thread_id: NativeThreadId,
    pub turn_id: NativeTurnId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub root: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Launch {
        project: ProjectId,
        name: String,
        model: Option<String>,
    },
    LaunchProvider {
        project: ProjectId,
        name: String,
        model: Option<String>,
        provider: Provider,
    },
    Send {
        text: String,
    },
    /// Send one message in Codex plan mode.
    SendPlan {
        text: String,
    },
    SetModel {
        model: Option<String>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Command {
    pub id: CommandId,
    pub thread_id: ThreadId,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub command_id: CommandId,
    pub thread_id: ThreadId,
    pub message_id: Option<MessageId>,
    pub run_id: Option<RunId>,
    pub event_sequences: Vec<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Starting,
    Running,
    Waiting,
    Interrupted,
    Completed,
    Failed,
}
impl RunStatus {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Interrupted)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Run {
    pub id: RunId,
    pub ordinal: i64,
    pub status: RunStatus,
    pub native_turn: Option<NativeTurnId>,
    pub error: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageOrigin {
    pub thread_id: ThreadId,
    pub run_id: RunId,
    pub request_key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerDelivery {
    Started,
    Queued,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerReceipt {
    /// A new task when the owner sends to a child whose original task has settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    pub thread_id: ThreadId,
    pub message_id: MessageId,
    pub run_id: RunId,
    pub delivery: PeerDelivery,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub run_id: RunId,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<MessageOrigin>,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: ThreadId,
    pub name: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionDelivery {
    Pending,
    Acknowledged,
    Delivered,
    Disposed,
}

/// The initial child run and the run that settled its work can differ after a follow-up.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegatedTask {
    pub task_id: TaskId,
    pub parent_thread_id: ThreadId,
    pub parent_run_id: RunId,
    pub child_thread_id: ThreadId,
    pub child_run_id: RunId,
    pub result_run_id: Option<RunId>,
    pub result: Option<String>,
    pub status: RunStatus,
    pub delivery: CompletionDelivery,
    pub wake_run_id: Option<RunId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeChildStatus {
    Starting,
    Running,
    Completed,
    Interrupted,
    Failed,
    /// The owning provider process closed before reporting an outcome.
    Disconnected,
}

/// Provider-owned work observed during an application run. Native addresses are
/// scoped to that run's provider session. The parent may itself be a child.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeChild {
    pub run_id: RunId,
    pub parent_thread_id: NativeThreadId,
    pub parent_turn_id: NativeTurnId,
    pub thread_id: NativeThreadId,
    pub turn_id: Option<NativeTurnId>,
    pub path: String,
    pub status: NativeChildStatus,
    pub result: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Thread {
    #[serde(default)]
    pub questions: Vec<UserInputRequest>,
    #[serde(default)]
    pub native_children: Vec<NativeChild>,
    #[serde(default)]
    pub tasks: Vec<DelegatedTask>,
    pub provider: Provider,
    pub permissions: Vec<PermissionRequest>,
    pub id: ThreadId,
    pub project: ProjectId,
    pub name: String,
    pub model: Option<String>,
    pub native_thread: Option<NativeThreadId>,
    pub messages: Vec<Message>,
    pub runs: Vec<Run>,
    pub cursor: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Change {
    QuestionRequested {
        request: UserInputRequest,
    },
    QuestionClosed {
        id: RequestId,
        answered: bool,
        /// Present when an agent submitted the answer.
        responder_run: Option<RunId>,
    },
    NativeChild {
        child: NativeChild,
    },
    Task {
        task: DelegatedTask,
    },
    Launched,
    PermissionRequested {
        request: PermissionRequest,
    },
    PermissionClosed {
        id: RequestId,
        option: Option<String>,
    },
    ModelSelected {
        model: Option<String>,
    },
    Bound {
        native_thread: NativeThreadId,
    },
    Run {
        run: Run,
    },
    Message {
        message: Message,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub sequence: i64,
    pub thread_id: ThreadId,
    pub change: Change,
}

fn display_error<E: fmt::Display, S: serde::Serializer>(
    error: &E,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_str(error)
}

#[derive(Debug, Serialize, thiserror::Error)]
#[serde(tag = "code", content = "detail", rename_all = "snake_case")]
pub enum Error {
    #[error("the user-input request was already answered")]
    AlreadyAnswered,
    #[error("state directory already has a runtime owner")]
    AlreadyOpen,
    #[error("agent caller lacks access to this operation")]
    Forbidden,
    #[error("provider turn interrupted")]
    Interrupted,
    #[error("runtime is closed")]
    Closed,
    #[error(
        "command {command} is already committed for {original_thread} with a different intent (requested thread {requested_thread})"
    )]
    Conflict {
        command: CommandId,
        original_thread: ThreadId,
        requested_thread: ThreadId,
    },
    #[error("unknown {0}")]
    NotFound(String),
    #[error("thread already has an active run")]
    Busy,
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("unsupported state schema version {0}")]
    SchemaVersion(i64),
    #[error("storage: {0}")]
    Storage(
        #[from]
        #[serde(serialize_with = "display_error")]
        rusqlite::Error,
    ),
    #[error("invalid stored data: {0}")]
    Data(
        #[from]
        #[serde(serialize_with = "display_error")]
        serde_json::Error,
    ),
    #[error("I/O: {0}")]
    Io(
        #[from]
        #[serde(serialize_with = "display_error")]
        std::io::Error,
    ),
    #[error("provider: {0}")]
    Provider(String),
    #[error("worker: {0}")]
    Worker(String),
}
pub type Result<T> = std::result::Result<T, Error>;
