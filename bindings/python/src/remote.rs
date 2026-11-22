use crate::{Runtime, future, parse, read, read_async, sync};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use huggler::{remote::Access, *};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use serde::Deserialize;
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum LinkAccess {
    Read,
    Operate,
}

#[pymethods]
impl Runtime {
    fn remote_router(&self) -> RemoteRouter {
        RemoteRouter(self.0.remote_router())
    }

    fn address(&self, py: Python<'_>, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.address(&ThreadId(thread)))
    }

    fn approve_link(
        &self,
        py: Python<'_>,
        thread: String,
        sender: &Bound<'_, PyAny>,
        access: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let sender = parse(sender)?;
        let access = match parse(access)? {
            LinkAccess::Read => Access::Read,
            LinkAccess::Operate => Access::Operate,
        };
        read(py, || {
            self.0.approve_link(&ThreadId(thread), &sender, access)
        })
    }

    fn revoke_link(&self, py: Python<'_>, grant_id: String) -> PyResult<bool> {
        sync(py, || self.0.revoke_link(&GrantId(grant_id)))
    }

    #[allow(clippy::too_many_arguments)]
    fn delegate_remote<'py>(
        &self,
        py: Python<'py>,
        peer: &Peer,
        target: &Bound<'py, PyAny>,
        parent: String,
        run: String,
        key: String,
        task: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        let peer = peer.0.clone();
        let target = parse(target)?;
        read_async(py, async move {
            runtime
                .delegate_remote(&peer, target, &ThreadId(parent), &RunId(run), &key, task)
                .await
        })
    }

    fn follow_remote<'py>(
        &self,
        py: Python<'py>,
        peer: &Peer,
        parent: String,
        run: String,
        key: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.0.clone();
        let peer = peer.0.clone();
        read_async(py, async move {
            runtime
                .follow_remote(&peer, &ThreadId(parent), &RunId(run), &key)
                .await
        })
    }

    fn invite_guest(&self, py: Python<'_>, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.invite_guest(&ThreadId(thread)))
    }

    fn guest_inbox(&self, py: Python<'_>, thread: String) -> PyResult<Py<PyAny>> {
        read(py, || self.0.guest_inbox(&ThreadId(thread)))
    }

    fn moderate_guest(
        &self,
        py: Python<'_>,
        thread: String,
        id: String,
        accept: bool,
    ) -> PyResult<Option<String>> {
        sync(py, || {
            self.0
                .moderate_guest(&ThreadId(thread), &SubmissionId(id), accept)
        })
        .map(|run| run.map(|id| id.0))
    }

    fn close_share(&self, py: Python<'_>, id: String) -> PyResult<bool> {
        sync(py, || self.0.close_share(&ShareId(id)))
    }
}

#[pyclass(module = "huggler", frozen)]
pub(crate) struct Peer(Arc<remote::Peer>);

#[pymethods]
impl Peer {
    #[staticmethod]
    fn loopback(py: Python<'_>, endpoint: &str, bearer: String) -> PyResult<Self> {
        let endpoint: SocketAddr = endpoint
            .parse()
            .map_err(|_| PyValueError::new_err("expected an IP address and port"))?;
        sync(py, || remote::Peer::loopback(endpoint, bearer)).map(|peer| Self(Arc::new(peer)))
    }

    fn request<'py>(
        &self,
        py: Python<'py>,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let peer = self.0.clone();
        let request = parse(request)?;
        read_async(py, async move { peer.request(request).await })
    }
}

#[pyclass(module = "huggler", frozen)]
pub(crate) struct RemoteRouter(Router);

#[pymethods]
impl RemoteRouter {
    fn request<'py>(
        &self,
        py: Python<'py>,
        method: &str,
        path: &str,
        headers: Vec<(Vec<u8>, Vec<u8>)>,
        body: &[u8],
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut request = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            request = request.header(name.as_slice(), value.as_slice());
        }
        let request = request
            .body(Body::from(body.to_vec()))
            .map_err(|_| PyValueError::new_err("invalid HTTP request"))?;
        let router = self.0.clone();
        future(
            py,
            async move {
                let response = router
                    .oneshot(request)
                    .await
                    .unwrap_or_else(|never| match never {});
                let (parts, body) = response.into_parts();
                let body = to_bytes(body, usize::MAX)
                    .await
                    .map_err(std::io::Error::other)?;
                Ok((parts, body))
            },
            |py, (parts, body)| {
                let headers = parts
                    .headers
                    .iter()
                    .map(|(key, value)| {
                        (
                            PyBytes::new(py, key.as_str().as_bytes()).unbind(),
                            PyBytes::new(py, value.as_bytes()).unbind(),
                        )
                    })
                    .collect::<Vec<_>>();
                Ok((
                    parts.status.as_u16(),
                    headers,
                    PyBytes::new(py, &body).unbind(),
                ))
            },
        )
    }
}
