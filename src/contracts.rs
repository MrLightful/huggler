//! Serializable application commands, projections and thread-scoped stream frames.
pub(crate) mod agent_commands;

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
id!(ProviderInstanceId);

impl ProviderInstanceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ProviderInstanceId {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<String> for ProviderInstanceId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, rmcp::schemars::JsonSchema,
)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    #[default]
    Codex,
    ClaudeAgent,
    Cursor,
    Grok,
    Fx,
    Kiro,
    Antigravity,
    AcpRegistry,
    #[serde(rename = "opencode")]
    OpenCode,
    Pi,
    Muse,
}

impl Provider {
    pub fn key(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeAgent => "claudeAgent",
            Self::Cursor => "cursor",
            Self::Grok => "grok",
            Self::Fx => "fx",
            Self::Kiro => "kiro",
            Self::Antigravity => "antigravity",
            Self::AcpRegistry => "acpRegistry",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
            Self::Muse => "muse",
        }
    }

    pub(crate) fn from_key(value: &str) -> rusqlite::Result<Self> {
        match value {
            "codex" => Ok(Self::Codex),
            "claudeAgent" => Ok(Self::ClaudeAgent),
            "cursor" => Ok(Self::Cursor),
            "grok" => Ok(Self::Grok),
            "fx" => Ok(Self::Fx),
            "kiro" => Ok(Self::Kiro),
            "antigravity" => Ok(Self::Antigravity),
            "acpRegistry" => Ok(Self::AcpRegistry),
            "opencode" => Ok(Self::OpenCode),
            "pi" => Ok(Self::Pi),
            "muse" => Ok(Self::Muse),
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

/// Structured questions from a provider turn, keyed by their provider-local IDs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserInputRequest {
    pub id: RequestId,
    pub run_id: RunId,
    pub questions: Vec<UserInputQuestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserInputQuestion {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default)]
    pub is_other: bool,
    #[serde(default)]
    pub is_secret: bool,
    pub options: Option<Vec<UserInputOption>>,
    #[serde(flatten)]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserInputOption {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(flatten)]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(untagged)]
pub enum UserAnswer {
    Text(String),
    Choices(Vec<String>),
}

pub type UserAnswers = std::collections::BTreeMap<String, UserAnswer>;

/// The child requesting a decision within the owning run. ACP addresses sessions only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeChildTurn {
    pub thread_id: NativeThreadId,
    pub turn_id: Option<NativeTurnId>,
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
    LaunchInstance {
        project: ProjectId,
        name: String,
        model: Option<String>,
        instance: ProviderInstanceId,
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
    RequiresAction,
    /// ACP reports that work ended without giving a stop reason.
    Idle,
    /// The adapter has no current work state for this session.
    Unknown,
    Completed,
    Interrupted,
    Failed,
    /// The owning provider process closed before reporting an outcome.
    Disconnected,
}

impl NativeChildStatus {
    pub(crate) fn working(self) -> bool {
        matches!(self, Self::Running | Self::RequiresAction)
    }
}

/// Provider-owned work observed during an application run. Native addresses are
/// scoped to that run's provider session. The parent may itself be a child.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeChild {
    pub run_id: RunId,
    pub parent_thread_id: NativeThreadId,
    pub parent_turn_id: Option<NativeTurnId>,
    pub thread_id: NativeThreadId,
    pub turn_id: Option<NativeTurnId>,
    /// Codex agent path or ACP display title (session ID when untitled).
    pub path: String,
    /// The live adapter can address this child's current work.
    #[serde(default)]
    pub can_interrupt: bool,
    pub status: NativeChildStatus,
    pub result: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub instance: ProviderInstanceId,
    pub reported_runs: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(from = "ThreadSnapshot")]
pub struct Thread {
    pub questions: Vec<UserInputRequest>,
    pub native_children: Vec<NativeChild>,
    pub tasks: Vec<DelegatedTask>,
    pub provider: Provider,
    pub provider_instance: ProviderInstanceId,
    pub permissions: Vec<PermissionRequest>,
    pub id: ThreadId,
    pub project: ProjectId,
    /// Managed checkout used by this thread. None selects the project root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<PathBuf>,
    pub name: String,
    pub model: Option<String>,
    pub native_thread: Option<NativeThreadId>,
    pub messages: Vec<Message>,
    pub runs: Vec<Run>,
    pub cursor: i64,
}

/// Snapshots written before named instances selected the built-in instance of their driver.
#[derive(Deserialize)]
struct ThreadSnapshot {
    #[serde(default)]
    questions: Vec<UserInputRequest>,
    #[serde(default)]
    native_children: Vec<NativeChild>,
    #[serde(default)]
    tasks: Vec<DelegatedTask>,
    provider: Provider,
    provider_instance: Option<ProviderInstanceId>,
    permissions: Vec<PermissionRequest>,
    id: ThreadId,
    project: ProjectId,
    worktree_path: Option<PathBuf>,
    name: String,
    model: Option<String>,
    native_thread: Option<NativeThreadId>,
    messages: Vec<Message>,
    runs: Vec<Run>,
    cursor: i64,
}

impl From<ThreadSnapshot> for Thread {
    fn from(snapshot: ThreadSnapshot) -> Self {
        Self {
            provider_instance: snapshot
                .provider_instance
                .unwrap_or_else(|| snapshot.provider.key().into()),
            provider: snapshot.provider,
            questions: snapshot.questions,
            native_children: snapshot.native_children,
            tasks: snapshot.tasks,
            permissions: snapshot.permissions,
            id: snapshot.id,
            project: snapshot.project,
            worktree_path: snapshot.worktree_path,
            name: snapshot.name,
            model: snapshot.model,
            native_thread: snapshot.native_thread,
            messages: snapshot.messages,
            runs: snapshot.runs,
            cursor: snapshot.cursor,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Change {
    /// Text as received, before the final assistant message is committed.
    OutputDelta {
        run_id: RunId,
        text: String,
    },
    /// Recorded on the thread whose agent created an independent peer.
    PeerCreated {
        caller_run_id: RunId,
        peer_thread_id: ThreadId,
    },
    RunInterruptRequested {
        run_id: RunId,
        caller_thread_id: ThreadId,
        caller_run_id: RunId,
        reason: Option<String>,
    },
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
    #[error("install the provider executable or configure its path: {0}")]
    MissingExecutable(String),
    #[error("sign in with the provider CLI for this instance, then refresh")]
    LoginRequired,
    #[error("OpenCode refused access; check the configured server password and permissions")]
    ServerAccessDenied,
    #[error("incompatible provider: {0}")]
    IncompatibleProvider(String),
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
