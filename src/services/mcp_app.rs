//! Bridge to an embedding host's existing, initialized MCP client connection.
use super::*;
use serde_json::json;
use std::{collections::BTreeSet, sync::Arc};

/// The host handles MCP framing, server identity, credentials, and transport lifecycle.
pub trait Connection: Send + Sync {
    fn request(&self, thread: &ThreadId, method: &str, params: Value) -> Result<Value>;
}

/// One trusted MCP server, with an explicit set of app resources and tools.
pub struct AppHost {
    pub connection: Arc<dyn Connection>,
    pub resources: BTreeSet<String>,
    pub tools: BTreeSet<String>,
}
struct App {
    thread: ThreadId,
    uri: String,
    tools: BTreeSet<String>,
    connection: Arc<dyn Connection>,
}
impl ServiceHost for AppHost {
    fn open(&mut self, thread: &ThreadId, _: &Path, request: Open) -> Result<Box<dyn Resource>> {
        let Open::McpApp { resource } = request else {
            return Err(Error::Unsupported("expected MCP App".into()));
        };
        if !resource.starts_with("ui://") || !self.resources.contains(&resource) {
            return Err(Error::Forbidden);
        }
        Ok(Box::new(App {
            thread: thread.clone(),
            uri: resource,
            tools: self.tools.clone(),
            connection: self.connection.clone(),
        }))
    }
}
impl Resource for App {
    fn call(&mut self, action: Action) -> Result<Output> {
        let (method, params) = match action {
            Action::ReadResource { uri } if uri == self.uri => {
                ("resources/read", json!({"uri":uri}))
            }
            Action::CallTool { name, arguments }
                if self.tools.contains(&name) && arguments.is_object() =>
            {
                ("tools/call", json!({"name":name,"arguments":arguments}))
            }
            _ => return Err(Error::Forbidden),
        };
        self.connection
            .request(&self.thread, method, params)
            .map(Output::Json)
    }
}
