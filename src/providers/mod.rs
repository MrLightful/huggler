pub mod claude_agent;
pub mod codex;
mod rpc;
use crate::{NativeThreadId, NativeTurnId, Provider};
use std::path::PathBuf;

pub(crate) struct Turn {
    pub provider: Provider,
    pub cwd: PathBuf,
    pub name: String,
    pub text: String,
    pub model: Option<String>,
    pub resume: Option<NativeThreadId>,
}

pub(crate) enum Update {
    Bound(NativeThreadId),
    Started(Option<NativeTurnId>),
    Finalizing(String),
}

pub(crate) struct Permissions<F> {
    pub run: crate::RunId,
    pub publish: F,
    pub decisions: tokio::sync::mpsc::UnboundedReceiver<(crate::RequestId, String)>,
}
