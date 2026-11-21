//! Moderated guest access. The invite can be redeemed once and the host can close the session.
use crate::{runtime::Shared, *};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Redeem {
        name: String,
    },
    View {
        thread: ThreadId,
        before: Option<HistoryCursor>,
    },
    Submit {
        thread: ThreadId,
        key: String,
        text: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Session {
        bearer: String,
        thread: ThreadId,
    },
    View {
        thread: ThreadId,
        messages: Vec<(Role, String)>,
        older: Option<HistoryCursor>,
    },
    Submitted {
        id: SubmissionId,
    },
}
pub struct Invitation {
    pub id: ShareId,
    pub bearer: String,
}
#[derive(Debug)]
pub struct Submission {
    pub id: SubmissionId,
    pub name: String,
    pub text: String,
}
async fn handle(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> std::result::Result<Json<Reply>, StatusCode> {
    let token = remote::bearer(&headers)?;
    shared
        .access(|db| db.guest_request(token, request))
        .map(Json)
        .map_err(remote::status)
}
pub(crate) fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/v1/guest", post(handle))
        .with_state(shared)
}
impl Huggler {
    pub fn invite_guest(&self, thread: &ThreadId) -> Result<Invitation> {
        self.shared.access(|db| db.invite_guest(thread))
    }
    pub fn guest_inbox(&self, thread: &ThreadId) -> Result<Vec<Submission>> {
        self.shared.access(|db| db.guest_inbox(thread))
    }
    pub fn moderate_guest(
        &self,
        thread: &ThreadId,
        id: &SubmissionId,
        accept: bool,
    ) -> Result<Option<RunId>> {
        self.shared
            .schedule(|db| db.moderate_guest(thread, id, accept))
    }
    pub fn close_share(&self, id: &ShareId) -> Result<bool> {
        self.shared.access(|db| db.close_share(id))
    }
}
