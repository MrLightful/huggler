mod application;
mod attachment;
mod options;
mod remote;
mod repository;
mod services;
mod text;

use huggler::*;
use pyo3::{
    create_exception,
    exceptions::{PyException, PyRuntimeError, PyValueError},
    prelude::*,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{future::Future, ops::Deref, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

create_exception!(huggler, HugglerError, PyException);

fn error(py: Python<'_>, error: Error) -> PyErr {
    let diagnostic = serde_json::to_value(&error).unwrap_or_default();
    let result = HugglerError::new_err(error.to_string());
    let exception = result.value(py);
    let _ = exception.setattr("code", diagnostic["code"].as_str().unwrap_or("worker"));
    if let Ok(detail) = data(py, &diagnostic["detail"]) {
        let _ = exception.setattr("detail", detail);
    }
    result
}

fn parse<T: DeserializeOwned>(input: &Bound<'_, PyAny>) -> PyResult<T> {
    pythonize::depythonize(input).map_err(|err| PyValueError::new_err(err.to_string()))
}

fn data<T: Serialize>(py: Python<'_>, value: &T) -> PyResult<Py<PyAny>> {
    Ok(pythonize::pythonize(py, value)?.unbind())
}

fn sync<T: Send>(py: Python<'_>, work: impl FnOnce() -> huggler::Result<T> + Send) -> PyResult<T> {
    py.detach(work).map_err(|err| error(py, err))
}

fn read<T: Serialize + Send>(
    py: Python<'_>,
    work: impl FnOnce() -> huggler::Result<T> + Send,
) -> PyResult<Py<PyAny>> {
    data(py, &sync(py, work)?)
}

fn future<'py, T, U>(
    py: Python<'py>,
    work: impl Future<Output = huggler::Result<T>> + Send + 'static,
    convert: impl FnOnce(Python<'_>, T) -> PyResult<U> + Send + 'static,
) -> PyResult<Bound<'py, PyAny>>
where
    T: Send + 'static,
    U: for<'a> IntoPyObject<'a> + Send + 'static,
{
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let result = work.await;
        Python::attach(|py| convert(py, result.map_err(|err| error(py, err))?))
    })
}

fn read_async<T: Serialize + Send + 'static>(
    py: Python<'_>,
    work: impl Future<Output = huggler::Result<T>> + Send + 'static,
) -> PyResult<Bound<'_, PyAny>> {
    future(py, work, |py, value| data(py, &value))
}

struct RuntimeOwner(Option<Huggler>);

impl Deref for RuntimeOwner {
    type Target = Huggler;

    fn deref(&self) -> &Huggler {
        self.0.as_ref().unwrap()
    }
}

impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        let runtime = self.0.take();
        // Huggler's destructor acquires storage locks. A worker holding one
        // can be waiting to enter Python for a host callback.
        pyo3_async_runtimes::tokio::get_runtime().spawn_blocking(move || drop(runtime));
    }
}

#[pyclass(name = "Huggler", module = "huggler", frozen)]
struct Runtime(Arc<RuntimeOwner>);

#[pymethods]
impl Runtime {
    #[staticmethod]
    #[pyo3(signature = (state_dir=None, *, config=None, agent_text=None))]
    fn open<'py>(
        py: Python<'py>,
        state_dir: Option<PathBuf>,
        config: Option<&Bound<'py, PyAny>>,
        agent_text: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options = options::build(py, state_dir, config, agent_text)?;
        future(py, Huggler::open(options), |py, runtime| {
            Py::new(py, Self(Arc::new(RuntimeOwner(Some(runtime)))))
        })
    }

    fn __aenter__<'py>(slf: Py<Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(slf) })
    }

    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: &Bound<'py, PyAny>,
        _exc: &Bound<'py, PyAny>,
        _traceback: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.close(py)
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        let locals = pyo3_async_runtimes::tokio::get_current_locals(py)?;
        // An explicit shutdown keeps running if Python stops awaiting it.
        let task =
            pyo3_async_runtimes::tokio::get_runtime().spawn(async move { runtime.close().await });
        pyo3_async_runtimes::tokio::future_into_py_with_locals(py, locals, async move {
            let result = task
                .await
                .map_err(|err| Error::Worker(err.to_string()))
                .and_then(|result| result);
            Python::attach(|py| result.map_err(|err| error(py, err)))
        })
    }

    fn open_project(&self, py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
        read(py, || self.0.open_project(path))
    }

    fn dispatch(&self, py: Python<'_>, command: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let command = parse(command)?;
        read(py, || self.0.dispatch(command))
    }

    #[pyo3(signature = (id, thread, project, name, model=None))]
    fn launch(
        &self,
        py: Python<'_>,
        id: String,
        thread: String,
        project: String,
        name: String,
        model: Option<String>,
    ) -> PyResult<Py<PyAny>> {
        read(py, || {
            self.0.launch(
                CommandId(id),
                ThreadId(thread),
                ProjectId(project),
                name,
                model,
            )
        })
    }

    fn send(
        &self,
        py: Python<'_>,
        id: String,
        thread: String,
        text: String,
    ) -> PyResult<Py<PyAny>> {
        read(py, || self.0.send(CommandId(id), ThreadId(thread), text))
    }

    fn enqueue_files(
        &self,
        py: Python<'_>,
        id: String,
        thread: String,
        text: String,
        files: Vec<Py<attachment::Attachment>>,
    ) -> PyResult<Py<PyAny>> {
        read(py, || {
            self.0.dispatch(Command {
                id: CommandId(id),
                thread_id: ThreadId(thread),
                action: Action::EnqueueFiles {
                    text,
                    files: files.iter().map(|file| file.get().0.clone()).collect(),
                },
            })
        })
    }

    fn stop_thread(&self, py: Python<'_>, id: String, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.stop_thread(CommandId(id), ThreadId(thread)))
    }

    fn resume_queue(&self, py: Python<'_>, id: String, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.resume_queue(CommandId(id), ThreadId(thread)))
    }

    fn steer_outcome(
        &self,
        py: Python<'_>,
        thread: String,
        command: String,
    ) -> PyResult<Py<PyAny>> {
        read(py, || {
            self.0.steer_outcome(&ThreadId(thread), &CommandId(command))
        })
    }

    fn respond_to_permission(
        &self,
        py: Python<'_>,
        thread: String,
        run: String,
        id: String,
        option: String,
    ) -> PyResult<()> {
        sync(py, || {
            self.0
                .respond_to_permission(&ThreadId(thread), &RunId(run), &RequestId(id), option)
        })
    }

    fn respond_to_question<'py>(
        &self,
        py: Python<'py>,
        thread: String,
        run: String,
        id: String,
        answers: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let answers = parse(answers)?;
        let runtime = self.0.clone();
        read_async(py, async move {
            runtime
                .respond_to_question(&ThreadId(thread), &RunId(run), &RequestId(id), answers)
                .await
        })
    }

    fn respond_with_attachments<'py>(
        &self,
        py: Python<'py>,
        thread: String,
        run: String,
        id: String,
        answers: &Bound<'py, PyAny>,
        attachments: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let answers = parse(answers)?;
        let attachments = parse(attachments)?;
        let runtime = self.0.clone();
        read_async(py, async move {
            runtime
                .respond_with_attachments(
                    &ThreadId(thread),
                    &RunId(run),
                    &RequestId(id),
                    answers,
                    &attachments,
                )
                .await
        })
    }

    fn dismiss_question<'py>(
        &self,
        py: Python<'py>,
        thread: String,
        run: String,
        id: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(py, async move {
            runtime
                .dismiss_question(&ThreadId(thread), &RunId(run), &RequestId(id))
                .await
        })
    }

    fn peer_receipt(&self, py: Python<'_>, sender: String, key: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.peer_receipt(&ThreadId(sender), &key))
    }

    #[pyo3(signature = (thread, run, child, turn=None))]
    fn interrupt_native_child<'py>(
        &self,
        py: Python<'py>,
        thread: String,
        run: String,
        child: String,
        turn: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(py, async move {
            let turn = turn.map(NativeTurnId);
            runtime
                .interrupt_native_child(
                    &ThreadId(thread),
                    &RunId(run),
                    &NativeThreadId(child),
                    turn.as_ref(),
                )
                .await
        })
    }

    fn revoke_agent_tools(&self, py: Python<'_>, thread: String, run: String) -> PyResult<bool> {
        sync(py, || {
            self.0.revoke_agent_tools(&ThreadId(thread), &RunId(run))
        })
    }

    fn output(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        options::output(py, self.0.output())
    }

    fn provider_label(&self, id: &str) -> String {
        self.0.output().provider_label(id).into()
    }

    fn provider_instances(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        data(py, &self.0.provider_instances().collect::<Vec<_>>())
    }

    fn provider_environment(&self, py: Python<'_>, id: &str) -> PyResult<Py<PyAny>> {
        let instance = self
            .0
            .provider_instances()
            .find(|instance| instance.id.as_str() == id)
            .ok_or_else(|| error(py, Error::NotFound(id.to_owned())))?;
        data(py, instance.environment())
    }

    fn probe_provider<'py>(
        &self,
        py: Python<'py>,
        id: String,
        cwd: PathBuf,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(py, async move { runtime.probe_provider(&id, cwd).await })
    }

    fn list_provider_skills<'py>(
        &self,
        py: Python<'py>,
        id: String,
        cwd: PathBuf,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(
            py,
            async move { runtime.list_provider_skills(&id, cwd).await },
        )
    }

    fn list_mcp_server_status<'py>(
        &self,
        py: Python<'py>,
        id: String,
        cwd: PathBuf,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(
            py,
            async move { runtime.list_mcp_server_status(&id, cwd).await },
        )
    }

    fn provider_usage(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        read(py, || self.0.provider_usage())
    }

    fn threads(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        read(py, || self.0.threads())
    }

    fn conversation_list(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        read(py, || self.0.conversation_list())
    }

    fn wait_for_conversation_list<'py>(
        &self,
        py: Python<'py>,
        revision: i64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(py, async move {
            runtime.wait_for_conversation_list(revision).await
        })
    }

    #[pyo3(signature = (last=None))]
    fn wait_for_threads_after<'py>(
        &self,
        py: Python<'py>,
        last: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(py, async move {
            runtime
                .wait_for_threads_after(last.map(ThreadId).as_ref())
                .await
        })
    }

    fn read(&self, py: Python<'_>, id: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.read(&ThreadId(id)))
    }

    #[pyo3(signature = (thread, before=None, query="", limit=100))]
    fn detail(
        &self,
        py: Python<'_>,
        thread: String,
        before: Option<&Bound<'_, PyAny>>,
        query: &str,
        limit: usize,
    ) -> PyResult<Py<PyAny>> {
        let before = before.map(parse).transpose()?;
        read(py, || {
            self.0
                .detail(&ThreadId(thread), before.as_ref(), query, limit)
        })
    }

    fn command_items(&self, py: Python<'_>, thread: String, run: String) -> PyResult<Vec<String>> {
        sync(py, || self.0.command_items(&ThreadId(thread), &RunId(run)))
    }

    #[pyo3(signature = (thread, run, item, after=None))]
    fn command_output<'py>(
        &self,
        py: Python<'py>,
        thread: String,
        run: String,
        item: String,
        after: Option<i64>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(py, async move {
            runtime
                .command_output(&ThreadId(thread), &RunId(run), &item, after)
                .await
        })
    }

    fn prepare_upload(
        &self,
        py: Python<'_>,
        thread: String,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> PyResult<String> {
        sync(py, || {
            self.0.prepare_upload(&ThreadId(thread), name, mime, bytes)
        })
        .map(|id| id.0)
    }

    fn discard_upload(&self, py: Python<'_>, thread: String, id: String) -> PyResult<()> {
        sync(py, || {
            self.0.discard_upload(&ThreadId(thread), &UploadId(id))
        })
    }

    fn checkpoint_diff(
        &self,
        py: Python<'_>,
        thread: String,
        from_run: String,
        to_run: String,
    ) -> PyResult<String> {
        sync(py, || {
            self.0
                .checkpoint_diff(&ThreadId(thread), &RunId(from_run), &RunId(to_run))
        })
    }

    fn fork_history(&self, py: Python<'_>, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.fork_history(&ThreadId(thread)))
    }

    fn read_message_body(&self, py: Python<'_>, thread: String, id: String) -> PyResult<String> {
        sync(py, || {
            self.0.read_message_body(&ThreadId(thread), &MessageId(id))
        })
    }

    fn read_attachment(
        &self,
        py: Python<'_>,
        thread: String,
        message: String,
        position: usize,
    ) -> PyResult<String> {
        sync(py, || {
            self.0
                .read_attachment(&ThreadId(thread), &MessageId(message), position)
        })
    }

    fn read_attachment_file(
        &self,
        py: Python<'_>,
        thread: String,
        message: String,
        position: usize,
    ) -> PyResult<attachment::Attachment> {
        sync(py, || {
            self.0
                .read_attachment_file(&ThreadId(thread), &MessageId(message), position)
        })
        .map(attachment::Attachment)
    }

    fn read_message(&self, py: Python<'_>, thread: String, id: String) -> PyResult<Py<PyAny>> {
        read(py, || {
            self.0.read_message(&ThreadId(thread), &MessageId(id))
        })
    }

    fn import_history<'py>(
        &self,
        py: Python<'py>,
        project: String,
        thread: String,
        name: String,
        instance: String,
        source: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let source = parse(source)?;
        let runtime = self.0.clone();
        read_async(py, async move {
            runtime
                .import_history(
                    &ProjectId(project),
                    &ThreadId(thread),
                    &name,
                    &ProviderInstanceId(instance),
                    source,
                )
                .await
        })
    }

    fn import_source(&self, py: Python<'_>, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.import_source(&ThreadId(thread)))
    }

    fn retain_events(&self, py: Python<'_>, thread: String, count: u32) -> PyResult<()> {
        sync(py, || self.0.retain_events(&ThreadId(thread), count))
    }

    fn delete_thread(&self, py: Python<'_>, thread: String) -> PyResult<()> {
        sync(py, || self.0.delete_thread(&ThreadId(thread)))
    }

    fn wait_for_change<'py>(
        &self,
        py: Python<'py>,
        thread: String,
        after: i64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(py, async move {
            runtime.wait_for_change(&ThreadId(thread), after).await
        })
    }

    fn wait<'py>(
        &self,
        py: Python<'py>,
        thread: String,
        run: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(py, async move {
            runtime.wait(&ThreadId(thread), &RunId(run)).await
        })
    }

    #[pyo3(signature = (id, after=None))]
    fn subscribe(
        &self,
        py: Python<'_>,
        id: String,
        after: Option<i64>,
    ) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
        let (snapshot, stream) = sync(py, || self.0.subscribe(&ThreadId(id), after))?;
        let stream = NativeEventStream(Arc::new(Mutex::new(StreamState {
            subscription: stream,
            pending: None,
        })));
        let wrapper = PyModule::import(py, "huggler._events")?
            .getattr("EventStream")?
            .call1((stream,))?;
        Ok((data(py, &snapshot)?, wrapper.unbind()))
    }
}

struct StreamState {
    subscription: Subscription,
    pending: Option<Event>,
}

#[pyclass(module = "huggler._native", frozen)]
struct NativeEventStream(Arc<Mutex<StreamState>>);

#[pymethods]
impl NativeEventStream {
    fn peek<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let stream = self.0.clone();
        read_async(py, async move {
            let mut state = stream.lock().await;
            if let Some(event) = &state.pending {
                return Ok(event.clone());
            }
            let event = state.subscription.next().await?;
            state.pending = Some(event.clone());
            Ok(event)
        })
    }

    fn consume(&self) -> PyResult<()> {
        // Python serializes reads and calls this after peek releases the lock.
        self.0
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("event stream read in progress"))?
            .pending = None;
        Ok(())
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("HugglerError", m.py().get_type::<HugglerError>())?;
    m.add_class::<Runtime>()?;
    m.add_class::<attachment::Attachment>()?;
    m.add_class::<remote::Peer>()?;
    m.add_class::<remote::RemoteRouter>()?;
    m.add_class::<repository::RepositoryClient>()?;
    m.add_class::<repository::PullRequest>()?;
    m.add_class::<repository::PullRequests>()?;
    Ok(())
}
