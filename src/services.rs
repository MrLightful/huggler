//! Thread-owned resources provided by an embedding host. These are trusted host APIs.
//! Adapters apply their URL, HTML, device and MCP server policies before external I/O.
use crate::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub mod mcp_app;
#[cfg(unix)]
pub mod terminal;

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Terminal,
    Preview,
    Device,
    Html,
    McpApp,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Open {
    Terminal {
        executable: PathBuf,
        args: Vec<String>,
    },
    Preview {
        url: String,
    },
    Device {
        id: String,
    },
    Html {
        document: String,
    },
    McpApp {
        resource: String,
    },
}
impl Open {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Terminal { .. } => Kind::Terminal,
            Self::Preview { .. } => Kind::Preview,
            Self::Device { .. } => Kind::Device,
            Self::Html { .. } => Kind::Html,
            Self::McpApp { .. } => Kind::McpApp,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// None starts at the oldest retained byte, including after an expired cursor.
    Output {
        after: Option<u64>,
    },
    /// Inspect process termination independently of output pipe EOF.
    ExitStatus,
    Navigate {
        url: String,
    },
    Capture,
    ReadResource {
        uri: String,
    },
    CallTool {
        name: String,
        arguments: Value,
    },
}
impl Action {
    fn supports(&self, kind: Kind) -> bool {
        match self {
            Self::Output { .. } | Self::ExitStatus => kind == Kind::Terminal,
            Self::Navigate { .. } => kind == Kind::Preview,
            Self::Capture => matches!(kind, Kind::Preview | Kind::Html | Kind::Device),
            Self::ReadResource { .. } | Self::CallTool { .. } => kind == Kind::McpApp,
        }
    }
}
#[derive(Debug)]
pub enum Output {
    Done,
    ExitStatus(Option<std::process::ExitStatus>),
    Bytes {
        mime: String,
        data: Vec<u8>,
    },
    Json(Value),
    /// Byte cursor immediately after the returned chunk. A client can disconnect and resume.
    Terminal {
        next: u64,
        data: Vec<u8>,
        eof: bool,
    },
}

/// A factory installed for one service kind. Callbacks are synchronous and must not reenter
/// this runtime's service registry. Hosts can marshal UI work to their own executor.
pub trait ServiceHost: Send {
    fn open(
        &mut self,
        thread: &ThreadId,
        checkout: &Path,
        request: Open,
    ) -> Result<Box<dyn Resource>>;
    fn devices(&mut self) -> Result<Vec<String>> {
        Err(Error::Unsupported("device enumeration".into()))
    }
}
/// Own the native handle here. Drop releases it on close, host loss, deletion and shutdown.
pub trait Resource: Send {
    fn call(&mut self, action: Action) -> Result<Output>;
}
struct Active {
    thread: ThreadId,
    kind: Kind,
    resource: Box<dyn Resource>,
}
#[derive(Default)]
pub(crate) struct Registry {
    hosts: HashMap<Kind, Box<dyn ServiceHost>>,
    resources: HashMap<ServiceHandleId, Active>,
}
impl Registry {
    pub(crate) fn close_thread(&mut self, thread: &ThreadId) {
        self.resources.retain(|_, r| &r.thread != thread);
    }
    pub(crate) fn clear(&mut self) {
        self.resources.clear();
        self.hosts.clear();
    }
    fn host(&mut self, kind: Kind) -> Result<&mut Box<dyn ServiceHost>> {
        self.hosts
            .get_mut(&kind)
            .ok_or_else(|| Error::Unsupported(format!("{kind:?} host unavailable")))
    }
    fn active(&mut self, thread: &ThreadId, id: &ServiceHandleId) -> Result<&mut Active> {
        self.resources
            .get_mut(id)
            .filter(|r| &r.thread == thread)
            .ok_or(Error::Forbidden)
    }
}
impl Huggler {
    /// Installing or removing a host closes all resources opened on its previous instance.
    pub fn set_service_host(&self, kind: Kind, host: Option<Box<dyn ServiceHost>>) -> Result<()> {
        let mut registry = self.shared.services.lock().unwrap();
        self.shared.access(|_| Ok(()))?;
        registry.resources.retain(|_, r| r.kind != kind);
        registry.hosts.remove(&kind);
        if let Some(host) = host {
            registry.hosts.insert(kind, host);
        }
        Ok(())
    }
    /// Resolve a thread's working directory, including managed delegation worktrees.
    pub fn thread_checkout(&self, thread: &ThreadId) -> Result<PathBuf> {
        self.shared.access(|db| db.service_checkout(thread))
    }
    pub fn devices(&self, thread: &ThreadId) -> Result<Vec<String>> {
        let mut registry = self.shared.services.lock().unwrap();
        self.shared.access(|db| db.thread_position(thread))?;
        registry.host(Kind::Device)?.devices()
    }
    pub fn open_service(&self, thread: &ThreadId, request: Open) -> Result<ServiceHandleId> {
        let mut registry = self.shared.services.lock().unwrap();
        let checkout = self.shared.access(|db| db.service_checkout(thread))?;
        let kind = request.kind();
        let resource = registry.host(kind)?.open(thread, &checkout, request)?;
        let id = ServiceHandleId(uuid::Uuid::new_v4().to_string());
        registry.resources.insert(
            id.clone(),
            Active {
                thread: thread.clone(),
                kind,
                resource,
            },
        );
        Ok(id)
    }
    pub fn service_handles(&self, thread: &ThreadId, kind: Kind) -> Result<Vec<ServiceHandleId>> {
        let registry = self.shared.services.lock().unwrap();
        self.shared.access(|db| db.thread_position(thread))?;
        let mut ids: Vec<_> = registry
            .resources
            .iter()
            .filter(|(_, r)| r.kind == kind && &r.thread == thread)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(ids)
    }
    pub fn service_call(
        &self,
        thread: &ThreadId,
        id: &ServiceHandleId,
        action: Action,
    ) -> Result<Output> {
        let mut registry = self.shared.services.lock().unwrap();
        self.shared.access(|_| Ok(()))?;
        let r = registry.active(thread, id)?;
        if !action.supports(r.kind) {
            return Err(Error::Unsupported(format!(
                "operation unavailable for {:?}",
                r.kind
            )));
        }
        r.resource.call(action)
    }
    /// A trusted host explicitly approves JSON data from a live app for this thread's next
    /// provider turn. Replaces the previous pending context. Raw tool/resource results stay local.
    pub fn approve_app_context(
        &self,
        thread: &ThreadId,
        id: &ServiceHandleId,
        context: &Value,
    ) -> Result<()> {
        let mut registry = self.shared.services.lock().unwrap();
        if registry.active(thread, id)?.kind != Kind::McpApp {
            return Err(Error::Forbidden);
        }
        let text = serde_json::to_string(context)?;
        if text.len() > 16384 {
            return Err(Error::Invalid("app context exceeds 16 KiB".into()));
        }
        self.shared.access(|db| db.set_app_context(thread, &text))
    }
    pub fn close_service(&self, thread: &ThreadId, id: &ServiceHandleId) -> Result<()> {
        let mut registry = self.shared.services.lock().unwrap();
        self.shared.access(|_| Ok(()))?;
        registry.active(thread, id)?;
        registry.resources.remove(id);
        Ok(())
    }
}
