pub mod claude_agent;
pub mod codex;
pub(crate) mod muse;
pub(crate) mod opencode;
pub(crate) mod pi;
pub mod probe;
pub mod registry;
mod rpc;
use crate::{
    NativeThreadId, NativeTurnId, Provider, ProviderInstanceId, RequestId, Result, UserAnswers,
};
use std::path::PathBuf;
use tokio::sync::oneshot;

pub(crate) struct ChildInterrupt {
    pub thread: NativeThreadId,
    pub turn: Option<NativeTurnId>,
    pub reply: oneshot::Sender<Result<()>>,
}

pub(crate) enum Control {
    Steer {
        command: crate::CommandId,
        text: String,
    },
    Interrupt(ChildInterrupt),
    Answer {
        id: RequestId,
        answers: Option<UserAnswers>,
        reply: oneshot::Sender<Result<()>>,
    },
}

pub(crate) struct AgentTools {
    pub url: String,
    pub token: String,
}

pub(crate) struct Turn {
    pub output: std::sync::Arc<crate::AgentOutput>,
    pub has_app_context: bool,
    pub sample_suggestion: bool,
    pub fork: Option<(NativeThreadId, NativeTurnId)>,
    pub plan_mode: bool,
    pub agent_tools: Option<AgentTools>,
    pub provider: Provider,
    pub instance: ProviderInstanceId,
    pub cwd: PathBuf,
    pub name: String,
    pub text: String,
    pub model: Option<String>,
    pub resume: Option<NativeThreadId>,
}

impl Turn {
    /// Commit consumption just before the first write that could deliver the prompt.
    pub fn submitting_prompt(&self, update: &impl Fn(Update) -> Result<()>) -> Result<()> {
        if self.has_app_context {
            update(Update::SubmittingAppContext)?;
        }
        Ok(())
    }
}

pub(crate) enum CommandFrame {
    Append(String),
    Snapshot {
        output: Option<String>,
        finished: bool,
    },
}

pub(crate) enum Update {
    SubmittingAppContext,
    CommandOutput {
        item: String,
        frame: CommandFrame,
    },
    Steer(crate::CommandId, crate::SteerOutcome),
    Output(String),
    Usage {
        input: i64,
        output: i64,
    },
    PermissionWithdrawn(RequestId),
    Question(crate::UserInputRequest),
    QuestionClosed {
        id: crate::RequestId,
        answered: bool,
    },
    Bound(NativeThreadId),
    Started(Option<NativeTurnId>),
    Finalizing(String),
    /// Settle the foreground while this worker finishes background work.
    ForegroundComplete,
    PromptSuggestion(String),
    NativeChild(crate::NativeChild),
}

pub(crate) struct Permissions<F> {
    pub run: crate::RunId,
    pub publish: F,
    pub decisions: tokio::sync::mpsc::UnboundedReceiver<(crate::RequestId, String)>,
}

/// Latest reported input and output token totals for this run.
pub(super) fn usage(value: &serde_json::Value) -> Option<Update> {
    Some(Update::Usage {
        input: value["inputTokens"].as_u64()?.try_into().ok()?,
        output: value["outputTokens"].as_u64()?.try_into().ok()?,
    })
}
