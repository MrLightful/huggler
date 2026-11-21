//! Thread-scoped links between hosts. Serve the router on loopback or behind an authenticated TLS transport.
use crate::{runtime::Shared, *};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub environment: EnvironmentId,
    pub thread: ThreadId,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Operate,
}

/// Transfer the bearer to the approved host through a private channel.
pub struct Grant {
    pub id: String,
    pub address: Address,
    pub bearer: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Write {
    Send {
        text: String,
    },
    Delegate {
        task: String,
        #[serde(default)]
        worktree: bool,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Read {
        target: Address,
        before: Option<HistoryCursor>,
    },
    Write {
        target: Address,
        sender_run: RunId,
        key: String,
        action: Write,
    },
    Result {
        target: Address,
        run: RunId,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    History {
        address: Address,
        history: HistoryPage,
    },
    Accepted {
        address: Address,
        receipt: PeerReceipt,
    },
    Result {
        status: RunStatus,
        text: Option<String>,
    },
}

/// HTTP client for a local peer or the local end of a host-managed secure tunnel.
pub struct Peer {
    url: String,
    client: reqwest::Client,
    bearer: String,
}
impl Peer {
    pub fn loopback(endpoint: SocketAddr, bearer: String) -> Result<Self> {
        if !endpoint.ip().is_loopback() {
            return Err(Error::Invalid("expected a loopback endpoint".into()));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(transport)?;
        Ok(Self {
            url: format!("http://{endpoint}/v1/peer"),
            client,
            bearer,
        })
    }
    pub async fn request(&self, request: Request) -> Result<Reply> {
        let response = self
            .client
            .post(&self.url)
            .bearer_auth(&self.bearer)
            .json(&request)
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::FORBIDDEN => Err(Error::Forbidden),
            StatusCode::NOT_IMPLEMENTED => {
                Err(Error::Unsupported("remote worktree isolation".into()))
            }
            status if !status.is_success() => {
                Err(Error::Invalid(format!("peer returned {status}")))
            }
            _ => response.json().await.map_err(transport),
        }
    }
}
fn transport(error: reqwest::Error) -> Error {
    Error::Io(std::io::Error::other(error.without_url()))
}

pub(crate) fn status(error: Error) -> StatusCode {
    match error {
        Error::Forbidden => StatusCode::FORBIDDEN,
        Error::Unsupported(_) => StatusCode::NOT_IMPLEMENTED,
        Error::Invalid(_) => StatusCode::BAD_REQUEST,
        Error::Conflict { .. } | Error::Busy => StatusCode::CONFLICT,
        Error::Closed => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
pub(crate) fn bearer(headers: &HeaderMap) -> std::result::Result<&str, StatusCode> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::FORBIDDEN)
}
async fn handle(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> std::result::Result<Json<Reply>, StatusCode> {
    let token = bearer(&headers)?;
    let reply = match request {
        Request::Write {
            target,
            sender_run,
            key,
            action,
        } => shared.schedule(|db| db.remote_write(token, &target, &sender_run, &key, &action)),
        read => shared.access(|db| db.remote_read(token, read)),
    };
    reply.map(Json).map_err(status)
}
impl Huggler {
    /// Mount explicitly to enable the versioned remote interface.
    pub fn remote_router(&self) -> Router {
        Router::new()
            .route("/v1/peer", post(handle))
            .with_state(self.shared.clone())
            .merge(crate::guest::router(self.shared.clone()))
    }
    pub fn address(&self, thread: &ThreadId) -> Result<Address> {
        self.shared.access(|db| db.address(thread))
    }
    /// Human host decision binding one sender address to one target.
    pub fn approve_link(
        &self,
        thread: &ThreadId,
        sender: &Address,
        access: Access,
    ) -> Result<Grant> {
        self.shared
            .access(|db| db.approve_link(thread, sender, access))
    }
    pub fn revoke_link(&self, grant_id: &str) -> Result<bool> {
        self.shared.access(|db| db.revoke_link(grant_id))
    }

    /// Return the recorded child, or retry a missing receipt. An operating grant for the
    /// same sender and target can recover the original receipt after bearer rotation.
    pub async fn delegate_remote(
        &self,
        peer: &Peer,
        target: Address,
        parent: &ThreadId,
        run: &RunId,
        key: &str,
        task: String,
    ) -> Result<Address> {
        if let Some(child) = self
            .shared
            .access(|db| db.reserve_remote_task(parent, run, key, &target, &task))?
        {
            return Ok(child);
        }
        let Reply::Accepted { address, receipt } = peer
            .request(Request::Write {
                target: target.clone(),
                sender_run: run.clone(),
                key: key.into(),
                action: Write::Delegate {
                    task,
                    worktree: false,
                },
            })
            .await?
        else {
            return Err(Error::Invalid("expected a delegation receipt".into()));
        };
        if address.environment != target.environment || receipt.thread_id != address.thread {
            return Err(Error::Invalid("unexpected peer address".into()));
        }
        self.shared
            .access(|db| db.record_remote_child(run, key, &address, &receipt.run_id))?;
        Ok(address)
    }
    /// A host follower polls this operation while connected. Settlement wakes the parent once.
    pub async fn follow_remote(
        &self,
        peer: &Peer,
        parent: &ThreadId,
        run: &RunId,
        key: &str,
    ) -> Result<Option<RunId>> {
        let (child, child_run, wake) =
            self.shared.access(|db| db.remote_child(parent, run, key))?;
        if wake.is_some() {
            return Ok(wake);
        }
        let Reply::Result { status, text } = peer
            .request(Request::Result {
                target: child,
                run: child_run,
            })
            .await?
        else {
            return Err(Error::Invalid("expected a remote result".into()));
        };
        if !status.terminal() {
            return Ok(None);
        }
        self.shared
            .schedule(|db| db.finish_remote_task(parent, run, key, status, text))
    }
}
