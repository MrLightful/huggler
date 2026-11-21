//! In-process orchestration of durable coding-agent conversations.
pub mod application;
mod attachments;
#[cfg(feature = "application")]
pub mod automation;
pub mod contracts;
pub mod guest;
mod imports;
mod layout;
mod mcp;
mod output;
pub use output::{AgentOutput, AgentText, ToolPresentation};
pub mod providers;
pub mod remote;
mod runtime;
mod store;
mod worktree;

pub use contracts::*;
pub use layout::{StorageLayout, WorktreeRoot};
pub use providers::probe::{ProviderReport, ProviderSkill};
pub use providers::registry::ProviderInstance;
pub use runtime::{Huggler, HugglerOptions, Subscription};

#[cfg(feature = "application")]
pub mod workspace;

#[cfg(feature = "application")]
pub mod repository;

#[cfg(feature = "application")]
pub mod services;
