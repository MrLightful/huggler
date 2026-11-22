use crate::{Runtime, parse, read, read_async, sync};
use huggler::{workspace::WorktreeBranch, *};
use pyo3::{prelude::*, types::PyBytes};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Branch {
    Exact(String),
    FromTitle(String),
}

#[pymethods]
impl Runtime {
    fn preferences(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        read(py, || self.0.preferences())
    }

    fn set_preferences(&self, py: Python<'_>, preferences: &Bound<'_, PyAny>) -> PyResult<()> {
        let preferences = parse(preferences)?;
        sync(py, || self.0.set_preferences(&preferences))
    }

    fn user_notices(&self, py: Python<'_>, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.user_notices(&ThreadId(thread)))
    }

    fn secret_requests(&self, py: Python<'_>, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.secret_requests(&ThreadId(thread)))
    }

    #[pyo3(signature = (thread, id, value=None))]
    fn answer_secret(
        &self,
        py: Python<'_>,
        thread: String,
        id: String,
        value: Option<&[u8]>,
    ) -> PyResult<()> {
        sync(py, || {
            self.0.answer_secret(
                &ThreadId(thread),
                &SecretRequestId(id),
                value.map(<[u8]>::to_vec),
            )
        })
    }

    fn approve_onepassword(
        &self,
        py: Python<'_>,
        thread: String,
        id: String,
        executable: PathBuf,
        reference: &str,
    ) -> PyResult<()> {
        sync(py, || {
            self.0.approve_onepassword(
                &ThreadId(thread),
                &SecretRequestId(id),
                &executable,
                reference,
            )
        })
    }

    fn consume_secret<'py>(
        &self,
        py: Python<'py>,
        project: String,
        reference: &str,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = sync(py, || self.0.consume_secret(&ProjectId(project), reference))?;
        Ok(PyBytes::new(py, &bytes))
    }

    fn clone_project(
        &self,
        py: Python<'_>,
        source: &str,
        destination: PathBuf,
    ) -> PyResult<Py<PyAny>> {
        read(py, || self.0.clone_project(source, &destination))
    }

    fn create_scratch(&self, py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
        read(py, || self.0.create_scratch(&path))
    }

    fn promote_scratch(&self, py: Python<'_>, project: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.promote_scratch(&ProjectId(project)))
    }

    fn allocate_workspace(&self, py: Python<'_>, source: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.allocate_workspace(&ProjectId(source)))
    }

    fn allocate_workspace_at(
        &self,
        py: Python<'_>,
        source: String,
        revision: &str,
    ) -> PyResult<Py<PyAny>> {
        read(py, || {
            self.0.allocate_workspace_at(&ProjectId(source), revision)
        })
    }

    fn allocate_workspace_on_branch(
        &self,
        py: Python<'_>,
        source: String,
        revision: &str,
        branch: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let branch: Branch = parse(branch)?;
        let branch = match &branch {
            Branch::Exact(name) => WorktreeBranch::Exact(name),
            Branch::FromTitle(title) => WorktreeBranch::FromTitle(title),
        };
        read(py, || {
            self.0
                .allocate_workspace_on_branch(&ProjectId(source), revision, branch)
        })
    }

    fn allocated_workspaces(&self, py: Python<'_>, source: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.allocated_workspaces(&ProjectId(source)))
    }

    fn release_workspace(&self, py: Python<'_>, project: String) -> PyResult<()> {
        sync(py, || self.0.release_workspace(&ProjectId(project)))
    }

    fn inspect_orphaned_worktrees(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        read(py, || self.0.inspect_orphaned_worktrees())
    }

    fn cleanup_orphaned_worktrees(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        read(py, || self.0.cleanup_orphaned_worktrees())
    }

    fn workspace_setup_status(&self, py: Python<'_>, project: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.workspace_setup_status(&ProjectId(project)))
    }

    fn setup_workspace<'py>(
        &self,
        py: Python<'py>,
        project: String,
        program: PathBuf,
        args: Vec<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        read_async(py, async move {
            runtime
                .setup_workspace(&ProjectId(project), &program, &args)
                .await
        })
    }

    #[pyo3(signature = (owner, spec, now, signing_key=None))]
    fn create_schedule(
        &self,
        py: Python<'_>,
        owner: String,
        spec: &Bound<'_, PyAny>,
        now: i64,
        signing_key: Option<&[u8]>,
    ) -> PyResult<String> {
        let spec = parse(spec)?;
        sync(py, || {
            self.0
                .create_schedule(&ThreadId(owner), spec, now, signing_key)
        })
        .map(|id| id.0)
    }

    fn schedules(&self, py: Python<'_>, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.schedules(&ThreadId(thread)))
    }

    fn tick_schedule(&self, py: Python<'_>, id: String, now: i64) -> PyResult<Py<PyAny>> {
        read(py, || self.0.tick_schedule(&ScheduleId(id), now))
    }

    fn deliver_webhook(
        &self,
        py: Python<'_>,
        id: String,
        signing_key: &[u8],
        signature: &str,
        body: &[u8],
    ) -> PyResult<Py<PyAny>> {
        read(py, || {
            self.0
                .deliver_webhook(&ScheduleId(id), signing_key, signature, body)
        })
    }

    fn schedule_history(&self, py: Python<'_>, id: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.schedule_history(&ScheduleId(id)))
    }

    fn delete_schedule(&self, py: Python<'_>, id: String) -> PyResult<bool> {
        sync(py, || self.0.delete_schedule(&ScheduleId(id)))
    }
}
