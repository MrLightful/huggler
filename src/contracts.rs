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
id!(ProjectId);
id!(NativeThreadId);
id!(NativeTurnId);

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
    Send {
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
    Starting,
    Running,
    Waiting,
    Completed,
    Failed,
}
impl RunStatus {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
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
pub struct Message {
    pub id: MessageId,
    pub run_id: RunId,
    pub role: Role,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: ThreadId,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Thread {
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
    Launched,
    ModelSelected { model: Option<String> },
    Bound { native_thread: NativeThreadId },
    Run { run: Run },
    Message { message: Message },
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
    #[error("state directory already has a runtime owner")]
    AlreadyOpen,
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
    #[error("Codex: {0}")]
    Provider(String),
    #[error("worker: {0}")]
    Worker(String),
}
pub type Result<T> = std::result::Result<T, Error>;
