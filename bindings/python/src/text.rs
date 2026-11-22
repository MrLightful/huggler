use huggler::{AgentText, Error, RunStatus};
use pyo3::{call::PyCallArgs, exceptions::PyAttributeError, prelude::*};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct PythonText {
    callbacks: Py<PyAny>,
    defaults: Arc<dyn AgentText>,
    delegated_title: String,
    resume_after_limit: String,
    suggestion_prompt: String,
    remote_title: String,
    scheduled_title: String,
}

impl PythonText {
    pub(crate) fn new(
        py: Python<'_>,
        callbacks: Py<PyAny>,
        defaults: Arc<dyn AgentText>,
    ) -> PyResult<Self> {
        // Rust borrows these strings. Resolve the host's values once when opening.
        let constant = |name, default: &str| -> PyResult<String> {
            match callbacks.bind(py).getattr(name) {
                Ok(method) => method.call0()?.extract(),
                Err(err) if err.is_instance_of::<PyAttributeError>(py) => Ok(default.into()),
                Err(err) => Err(err),
            }
        };
        Ok(Self {
            delegated_title: constant("delegated_title", defaults.delegated_title())?,
            resume_after_limit: constant("resume_after_limit", defaults.resume_after_limit())?,
            suggestion_prompt: constant("suggestion_prompt", defaults.suggestion_prompt())?,
            remote_title: constant("remote_title", defaults.remote_title())?,
            scheduled_title: constant("scheduled_title", defaults.scheduled_title())?,
            callbacks,
            defaults,
        })
    }

    fn call<T>(
        &self,
        name: &str,
        invoke: impl FnOnce(&Bound<'_, PyAny>) -> PyResult<T>,
    ) -> Option<T> {
        Python::try_attach(|py| {
            let result = match self.callbacks.bind(py).getattr(name) {
                Ok(method) => invoke(&method),
                Err(err) if err.is_instance_of::<PyAttributeError>(py) => return None,
                Err(err) => Err(err),
            };
            match result {
                Ok(value) => Some(value),
                Err(err) => {
                    err.write_unraisable(py, None);
                    None
                }
            }
        })
        .flatten()
    }

    fn string(&self, name: &str, args: impl for<'a> PyCallArgs<'a>) -> Option<String> {
        self.call(name, |method| method.call1(args)?.extract())
    }
}

impl AgentText for PythonText {
    fn history(&self, host_title: &str, history_json: &str, message: &str) -> String {
        self.string("history", (host_title, history_json, message))
            .unwrap_or_else(|| self.defaults.history(host_title, history_json, message))
    }
    fn peer_message(&self, host_title: &str, origin_json: &str, message: &str) -> String {
        self.string("peer_message", (host_title, origin_json, message))
            .unwrap_or_else(|| self.defaults.peer_message(host_title, origin_json, message))
    }
    fn app_context(&self, context: &str, message: &str) -> String {
        self.string("app_context", (context, message))
            .unwrap_or_else(|| self.defaults.app_context(context, message))
    }
    fn attachment(&self, name: &str, content: &str) -> String {
        self.string("attachment", (name, content))
            .unwrap_or_else(|| self.defaults.attachment(name, content))
    }
    fn with_attachments(&self, message: &str, attachments: &[String]) -> String {
        self.string("with_attachments", (message, attachments.to_vec()))
            .unwrap_or_else(|| self.defaults.with_attachments(message, attachments))
    }
    fn peer_title(&self, prompt: Option<&str>, position: usize) -> String {
        self.string("peer_title", (prompt, position))
            .unwrap_or_else(|| self.defaults.peer_title(prompt, position))
    }
    fn delegated_title(&self) -> &str {
        &self.delegated_title
    }
    fn delegation(&self, role: Option<&str>, task: &str) -> String {
        self.string("delegation", (role, task))
            .unwrap_or_else(|| self.defaults.delegation(role, task))
    }
    fn task_results(&self, host_title: &str, results_json: &str) -> String {
        self.string("task_results", (host_title, results_json))
            .unwrap_or_else(|| self.defaults.task_results(host_title, results_json))
    }
    fn interrupted_by_agent(&self, reason: Option<&str>) -> String {
        self.string("interrupted_by_agent", (reason,))
            .unwrap_or_else(|| {
                let message = self.label("Stopped by an agent");
                match reason {
                    Some(reason) => format!("{message}: {reason}"),
                    None => message,
                }
            })
    }
    fn resume_after_limit(&self) -> &str {
        &self.resume_after_limit
    }
    fn suggestion_prompt(&self) -> &str {
        &self.suggestion_prompt
    }
    fn guest_message(&self, name: &str, message: &str) -> String {
        self.string("guest_message", (name, message))
            .unwrap_or_else(|| self.defaults.guest_message(name, message))
    }
    fn remote_title(&self) -> &str {
        &self.remote_title
    }
    fn remote_result(&self, key: &str, status: RunStatus, result: &str) -> String {
        self.call("remote_result", |method| {
            let status = pythonize::pythonize(method.py(), &status)?;
            method.call1((key, status, result))?.extract()
        })
        .unwrap_or_else(|| self.defaults.remote_result(key, status, result))
    }
    fn scheduled_title(&self) -> &str {
        &self.scheduled_title
    }
    fn repository_update(&self, repo: &str, item: &str, state: &str) -> String {
        self.string("repository_update", (repo, item, state))
            .unwrap_or_else(|| self.defaults.repository_update(repo, item, state))
    }
    fn label(&self, default: &str) -> String {
        self.string("label", (default,))
            .unwrap_or_else(|| self.defaults.label(default))
    }
    fn run_error(&self, error: &Error) -> String {
        self.call("run_error", |method| {
            let diagnostic = pythonize::pythonize(method.py(), error)?;
            method.call1((diagnostic,))?.extract()
        })
        .unwrap_or_else(|| self.label(&error.to_string()))
    }
    fn tool_error(&self, error: &Error) -> Value {
        self.call("tool_error", |method| {
            let error = pythonize::pythonize(method.py(), error)?;
            let returned = method.call1((error,))?;
            Ok(pythonize::depythonize(&returned)?)
        })
        .unwrap_or_else(|| self.defaults.tool_error(error))
    }
}
