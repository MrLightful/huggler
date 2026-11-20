pub mod claude_agent;
pub mod codex;
mod rpc;
use crate::{NativeThreadId, NativeTurnId, Provider};
use std::path::PathBuf;

pub(crate) struct AgentTools {
    pub url: String,
    pub token: String,
}

pub(crate) struct Turn {
    pub plan_mode: bool,
    pub agent_tools: Option<AgentTools>,
    pub provider: Provider,
    pub cwd: PathBuf,
    pub name: String,
    pub text: String,
    pub model: Option<String>,
    pub resume: Option<NativeThreadId>,
}

pub(crate) enum Update {
    Question(crate::UserInputRequest),
    QuestionClosed {
        id: crate::RequestId,
        answered: bool,
    },
    Bound(NativeThreadId),
    Started(Option<NativeTurnId>),
    Finalizing(String),
    /// The foreground has returned its answer and this worker has background children.
    ForegroundComplete,
    NativeChild(crate::NativeChild),
}

pub(crate) struct Permissions<F> {
    pub run: crate::RunId,
    pub publish: F,
    pub decisions: tokio::sync::mpsc::UnboundedReceiver<(crate::RequestId, String)>,
}
