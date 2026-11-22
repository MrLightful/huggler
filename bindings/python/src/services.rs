use crate::{Runtime, data, parse, read, sync};
use huggler::{
    services::{self, Action, Kind, Open, Output, Resource, ServiceHost, mcp_app},
    *,
};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(FromPyObject)]
#[pyo3(from_item_all)]
struct ServiceBytes {
    mime: String,
    data: Vec<u8>,
}

#[derive(FromPyObject)]
#[pyo3(from_item_all)]
struct TerminalChunk {
    next: u64,
    data: Vec<u8>,
    eof: bool,
}

struct PythonHost(Py<PyAny>);
struct PythonResource(Py<PyAny>);
struct PythonConnection(Py<PyAny>);

fn callback<T>(work: impl for<'py> FnOnce(Python<'py>) -> PyResult<T>) -> huggler::Result<T> {
    Python::try_attach(|py| work(py).map_err(|err| Error::Integration(err.to_string())))
        .unwrap_or(Err(Error::Closed))
}

impl ServiceHost for PythonHost {
    fn open(
        &mut self,
        thread: &ThreadId,
        checkout: &Path,
        request: Open,
    ) -> huggler::Result<Box<dyn Resource>> {
        let resource = callback(|py| {
            self.0
                .bind(py)
                .call_method1("open", (&thread.0, checkout, data(py, &request)?))
                .map(Bound::unbind)
        })?;
        Ok(Box::new(PythonResource(resource)))
    }

    fn devices(&mut self) -> huggler::Result<Vec<String>> {
        callback(|py| self.0.bind(py).call_method0("devices")?.extract())
    }
}

impl Resource for PythonResource {
    fn call(&mut self, action: Action) -> huggler::Result<Output> {
        callback(|py| {
            let output = self
                .0
                .bind(py)
                .call_method1("call", (data(py, &action)?,))?;
            Ok(
                match output.get_item("type")?.extract::<String>()?.as_str() {
                    "done" => Output::Done,
                    "bytes" => {
                        let bytes: ServiceBytes = output.extract()?;
                        Output::Bytes {
                            mime: bytes.mime,
                            data: bytes.data,
                        }
                    }
                    "terminal" => {
                        let chunk: TerminalChunk = output.extract()?;
                        Output::Terminal {
                            next: chunk.next,
                            data: chunk.data,
                            eof: chunk.eof,
                        }
                    }
                    "json" => Output::Json(parse(&output.get_item("value")?)?),
                    "exit_status" => Output::ExitStatus(
                        output
                            .get_item("raw")?
                            .extract::<Option<_>>()?
                            .map(exit_status_from_raw),
                    ),
                    _ => {
                        return Err(pyo3::exceptions::PyValueError::new_err(
                            "unknown service output type",
                        ));
                    }
                },
            )
        })
    }
}

#[cfg(unix)]
fn exit_status_from_raw(raw: i32) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(raw)
}

#[cfg(windows)]
fn exit_status_from_raw(raw: u32) -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(raw)
}

#[cfg(unix)]
fn raw_exit_status(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status.into_raw()
}

#[cfg(windows)]
fn raw_exit_status(status: std::process::ExitStatus) -> u32 {
    // A Windows process exit status always has a code.
    status.code().unwrap() as u32
}

impl Drop for PythonResource {
    fn drop(&mut self) {
        Python::try_attach(|py| {
            if let Err(err) = self.0.bind(py).call_method0("close")
                && !err.is_instance_of::<pyo3::exceptions::PyAttributeError>(py)
            {
                err.write_unraisable(py, None);
            }
        });
    }
}

impl mcp_app::Connection for PythonConnection {
    fn request(&self, thread: &ThreadId, method: &str, params: Value) -> huggler::Result<Value> {
        callback(|py| {
            let output = self
                .0
                .bind(py)
                .call_method1("request", (&thread.0, method, data(py, &params)?))?;
            parse(&output)
        })
    }
}

#[pymethods]
impl Runtime {
    #[pyo3(signature = (kind, host=None))]
    fn set_service_host(
        &self,
        py: Python<'_>,
        kind: &Bound<'_, PyAny>,
        host: Option<Py<PyAny>>,
    ) -> PyResult<()> {
        let kind = parse(kind)?;
        let host = host.map(|host| Box::new(PythonHost(host)) as Box<dyn ServiceHost>);
        sync(py, || self.0.set_service_host(kind, host))
    }

    fn install_terminal_host(&self, py: Python<'_>) -> PyResult<()> {
        #[cfg(unix)]
        return sync(py, || {
            self.0.set_service_host(
                Kind::Terminal,
                Some(Box::new(services::terminal::TerminalHost)),
            )
        });
        #[cfg(not(unix))]
        sync(py, || {
            Err(Error::Unsupported(
                "built-in terminal host requires Unix".into(),
            ))
        })
    }

    fn install_mcp_app_host(
        &self,
        py: Python<'_>,
        connection: Py<PyAny>,
        resources: BTreeSet<String>,
        tools: BTreeSet<String>,
    ) -> PyResult<()> {
        let host = mcp_app::AppHost {
            connection: Arc::new(PythonConnection(connection)),
            resources,
            tools,
        };
        sync(py, || {
            self.0.set_service_host(Kind::McpApp, Some(Box::new(host)))
        })
    }

    fn thread_checkout(&self, py: Python<'_>, thread: String) -> PyResult<PathBuf> {
        sync(py, || self.0.thread_checkout(&ThreadId(thread)))
    }

    fn devices(&self, py: Python<'_>, thread: String) -> PyResult<Vec<String>> {
        sync(py, || self.0.devices(&ThreadId(thread)))
    }

    fn open_service(
        &self,
        py: Python<'_>,
        thread: String,
        request: &Bound<'_, PyAny>,
    ) -> PyResult<String> {
        let request = parse(request)?;
        sync(py, || self.0.open_service(&ThreadId(thread), request)).map(|id| id.0)
    }

    fn service_handles(
        &self,
        py: Python<'_>,
        thread: String,
        kind: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let kind = parse(kind)?;
        read(py, || self.0.service_handles(&ThreadId(thread), kind))
    }

    fn service_call(
        &self,
        py: Python<'_>,
        thread: String,
        id: String,
        action: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let action = parse(action)?;
        let output = sync(py, || {
            self.0
                .service_call(&ThreadId(thread), &ServiceHandleId(id), action)
        })?;
        let result = PyDict::new(py);
        match output {
            Output::Done => result.set_item("type", "done")?,
            Output::ExitStatus(status) => {
                result.set_item("type", "exit_status")?;
                result.set_item("code", status.and_then(|status| status.code()))?;
                result.set_item("success", status.map(|status| status.success()))?;
                result.set_item("raw", status.map(raw_exit_status))?;
            }
            Output::Bytes { mime, data } => {
                result.set_item("type", "bytes")?;
                result.set_item("mime", mime)?;
                result.set_item("data", PyBytes::new(py, &data))?;
            }
            Output::Json(value) => {
                result.set_item("type", "json")?;
                result.set_item("value", data(py, &value)?)?;
            }
            Output::Terminal { next, data, eof } => {
                result.set_item("type", "terminal")?;
                result.set_item("next", next)?;
                result.set_item("data", PyBytes::new(py, &data))?;
                result.set_item("eof", eof)?;
            }
        }
        Ok(result.into_any().unbind())
    }

    fn approve_app_context(
        &self,
        py: Python<'_>,
        thread: String,
        id: String,
        context: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let context = parse(context)?;
        sync(py, || {
            self.0
                .approve_app_context(&ThreadId(thread), &ServiceHandleId(id), &context)
        })
    }

    fn close_service(&self, py: Python<'_>, thread: String, id: String) -> PyResult<()> {
        sync(py, || {
            self.0
                .close_service(&ThreadId(thread), &ServiceHandleId(id))
        })
    }
}
