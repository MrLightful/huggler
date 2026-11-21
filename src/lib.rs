//! In-process orchestration of durable coding-agent conversations.
pub mod contracts;
pub mod guest;
mod imports;
mod mcp;
pub mod providers;
pub mod remote;
mod runtime;
mod store;
mod worktree;

pub use contracts::*;
pub use providers::probe::ProviderReport;
pub use providers::registry::ProviderInstance;
pub use runtime::{Huggler, HugglerOptions, Subscription};
