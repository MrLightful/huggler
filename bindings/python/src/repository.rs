use crate::{Runtime, future, parse, read, read_async, sync};
use huggler::{
    repository::{
        self,
        github::{self, BranchPullRequests, PullRequestState, PullRequestUpdate},
    },
    *,
};
use pyo3::prelude::*;
use serde::Deserialize;
use std::sync::Arc;

#[pymethods]
impl Runtime {
    fn bind_repository(
        &self,
        py: Python<'_>,
        project: String,
        driver: &Bound<'_, PyAny>,
        origin: &str,
        repository: &str,
    ) -> PyResult<Py<PyAny>> {
        let driver = parse(driver)?;
        read(py, || {
            self.0
                .bind_repository(&ProjectId(project), driver, origin, repository)
        })
    }

    fn bind_thread_repository(
        &self,
        py: Python<'_>,
        thread: String,
        driver: &Bound<'_, PyAny>,
        origin: &str,
        repository: &str,
    ) -> PyResult<Py<PyAny>> {
        let driver = parse(driver)?;
        read(py, || {
            self.0
                .bind_thread_repository(&ThreadId(thread), driver, origin, repository)
        })
    }

    #[pyo3(signature = (project, binding, authorization=None))]
    fn repository_client(
        &self,
        py: Python<'_>,
        project: String,
        binding: String,
        authorization: Option<&str>,
    ) -> PyResult<RepositoryClient> {
        sync(py, || {
            self.0.repository_client(
                &ProjectId(project),
                &RepositoryBindingId(binding),
                authorization,
            )
        })
        .map(|client| RepositoryClient(Arc::new(client)))
    }

    fn link_repository_item(
        &self,
        py: Python<'_>,
        thread: String,
        binding: String,
        item: &str,
    ) -> PyResult<String> {
        sync(py, || {
            self.0
                .link_repository_item(&ThreadId(thread), &RepositoryBindingId(binding), item)
        })
        .map(|id| id.0)
    }

    fn repository_links(&self, py: Python<'_>, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.repository_links(&ThreadId(thread)))
    }

    fn unlink_repository_item(
        &self,
        py: Python<'_>,
        thread: String,
        link: String,
    ) -> PyResult<bool> {
        sync(py, || {
            self.0
                .unlink_repository_item(&ThreadId(thread), &RepositoryLinkId(link))
        })
    }

    fn watch_repository_item(
        &self,
        py: Python<'_>,
        thread: String,
        link: String,
    ) -> PyResult<String> {
        sync(py, || {
            self.0
                .watch_repository_item(&ThreadId(thread), &RepositoryLinkId(link))
        })
        .map(|id| id.0)
    }

    fn unwatch_repository_item(
        &self,
        py: Python<'_>,
        thread: String,
        watch: String,
    ) -> PyResult<bool> {
        sync(py, || {
            self.0
                .unwatch_repository_item(&ThreadId(thread), &RepositoryWatchId(watch))
        })
    }

    fn sync_repository_watch(
        &self,
        py: Python<'_>,
        watch: String,
        revision: i64,
        state: &str,
    ) -> PyResult<Option<String>> {
        sync(py, || {
            self.0
                .sync_repository_watch(&RepositoryWatchId(watch), revision, state)
        })
        .map(|run| run.map(|id| id.0))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl From<MergeMethod> for github::MergeMethod {
    fn from(value: MergeMethod) -> Self {
        match value {
            MergeMethod::Merge => Self::Merge,
            MergeMethod::Squash => Self::Squash,
            MergeMethod::Rebase => Self::Rebase,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PullUpdate {
    Merge { method: MergeMethod },
    EnableAutoMerge { method: Option<MergeMethod> },
    Enqueue,
}

#[pyclass(module = "huggler", frozen)]
pub(crate) struct RepositoryClient(Arc<repository::RepositoryClient>);

#[pymethods]
impl RepositoryClient {
    fn list<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let client = self.0.clone();
        read_async(py, async move { client.list().await })
    }

    fn checks<'py>(&self, py: Python<'py>, sha: String) -> PyResult<Bound<'py, PyAny>> {
        let client = self.0.clone();
        read_async(py, async move { client.checks(&sha).await })
    }

    fn comment<'py>(
        &self,
        py: Python<'py>,
        number: u64,
        body: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let client = self.0.clone();
        read_async(py, async move { client.comment(number, &body).await })
    }

    fn comment_on_issue<'py>(
        &self,
        py: Python<'py>,
        identifier: String,
        body: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let client = self.0.clone();
        read_async(py, async move {
            client.comment_on_issue(&identifier, &body).await
        })
    }

    fn github_default_branch<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let client = self.0.clone();
        read_async(py, async move { client.github_default_branch().await })
    }

    fn create_github_pull_request<'py>(
        &self,
        py: Python<'py>,
        head: String,
        base: String,
        title: String,
        body: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let client = self.0.clone();
        read_async(py, async move {
            client
                .create_github_pull_request(&head, &base, &title, &body)
                .await
        })
    }

    fn github_pull_requests<'py>(
        &self,
        py: Python<'py>,
        branch: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let client = self.0.clone();
        future(
            py,
            async move { client.github_pull_requests(&branch).await },
            |py, requests| Py::new(py, PullRequests(requests)),
        )
    }

    fn update_github_pull_request<'py>(
        &self,
        py: Python<'py>,
        previous: &PullRequest,
        action: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let client = self.0.clone();
        let previous = previous.0.clone();
        let action = match parse(action)? {
            PullUpdate::Merge { method } => PullRequestUpdate::Merge(method.into()),
            PullUpdate::EnableAutoMerge { method } => {
                PullRequestUpdate::EnableAutoMerge(method.map(Into::into))
            }
            PullUpdate::Enqueue => PullRequestUpdate::Enqueue,
        };
        read_async(py, async move {
            client.update_github_pull_request(&previous, action).await
        })
    }
}

#[pyclass(name = "BranchPullRequests", module = "huggler", frozen)]
pub(crate) struct PullRequests(BranchPullRequests);

#[pymethods]
impl PullRequests {
    #[getter]
    fn open(&self) -> Vec<PullRequest> {
        self.0.open.iter().cloned().map(PullRequest).collect()
    }

    #[getter]
    fn recent(&self) -> Vec<PullRequest> {
        self.0.recent.iter().cloned().map(PullRequest).collect()
    }
}

#[pyclass(module = "huggler", frozen)]
pub(crate) struct PullRequest(github::PullRequest);

#[pymethods]
impl PullRequest {
    #[getter]
    fn number(&self) -> u64 {
        self.0.number()
    }
    #[getter]
    fn url(&self) -> &str {
        self.0.url()
    }
    #[getter]
    fn repository(&self) -> &str {
        self.0.repository()
    }
    #[getter]
    fn base_branch(&self) -> &str {
        self.0.base_branch()
    }
    #[getter]
    fn head_commit(&self) -> &str {
        self.0.head_commit()
    }

    fn same_revision(&self, other: &Self) -> bool {
        self.0.same_revision(&other.0)
    }

    #[getter]
    fn state(&self) -> &'static str {
        match self.0.state() {
            PullRequestState::Open => "open",
            PullRequestState::Merged => "merged",
            PullRequestState::Closed => "closed",
            PullRequestState::Unknown => "unknown",
        }
    }

    #[getter]
    fn is_draft(&self) -> bool {
        self.0.is_draft()
    }
    #[getter]
    fn has_conflicts(&self) -> bool {
        self.0.has_conflicts()
    }
    #[getter]
    fn failed_checks(&self) -> bool {
        self.0.failed_checks()
    }
    #[getter]
    fn changes_requested(&self) -> bool {
        self.0.changes_requested()
    }
    #[getter]
    fn uses_merge_queue(&self) -> bool {
        self.0.uses_merge_queue()
    }
    #[getter]
    fn is_queued(&self) -> bool {
        self.0.is_queued()
    }
    #[getter]
    fn auto_merge_enabled(&self) -> bool {
        self.0.auto_merge_enabled()
    }

    fn allows_merge_method(&self, method: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self
            .0
            .allows_merge_method(parse::<MergeMethod>(method)?.into()))
    }

    #[getter]
    fn can_merge_now(&self) -> bool {
        self.0.can_merge_now()
    }
    #[getter]
    fn can_enable_auto_merge(&self) -> bool {
        self.0.can_enable_auto_merge()
    }
}
