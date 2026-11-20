//! Run-scoped agent tools on the runtime's loopback listener.
use crate::{Error, Result as HostResult, ThreadId, runtime::Shared};
use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION, request::Parts},
    middleware::{self, Next},
    response::Response,
};
use rmcp::{
    ServerHandler,
    handler::server::{tool::Extension, wrapper::Parameters},
    model::CallToolResult,
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::{net::TcpListener, sync::watch};

fn bearer(headers: &HeaderMap) -> HostResult<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(Error::Forbidden)
}

async fn authenticate(
    State(runtime): State<Arc<Shared>>,
    request: Request,
    next: Next,
) -> std::result::Result<Response, StatusCode> {
    let token = bearer(request.headers()).map_err(|_| StatusCode::UNAUTHORIZED)?;
    runtime
        .access(|db| db.caller(token))
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    Ok(next.run(request).await)
}

#[derive(Clone)]
struct AgentServer(Arc<Shared>);

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Send {
    thread_id: String,
    message: String,
    /// Use the same key and message when retrying a delivery.
    client_request_id: String,
    #[serde(default)]
    mode: DeliveryMode,
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum DeliveryMode {
    #[default]
    Auto,
}

fn tool_result(value: HostResult<impl Serialize>) -> CallToolResult {
    match value {
        Ok(value) => CallToolResult::structured(json!(value)),
        Err(error) => CallToolResult::structured_error(json!(error)),
    }
}

#[tool_router]
impl AgentServer {
    #[tool(description = "List other Huggler threads in your project and their addresses.")]
    fn thread_list(&self, Extension(parts): Extension<Parts>) -> CallToolResult {
        tool_result(
            self.0
                .access(|db| Ok(json!({"threads":db.peers(bearer(&parts.headers)?)?}))),
        )
    }

    #[tool(
        description = "Send a message to another Huggler thread. An idle recipient starts a run. A busy recipient queues it. Reuse clientRequestId for an identical retry."
    )]
    fn thread_send(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<Send>,
    ) -> CallToolResult {
        let DeliveryMode::Auto = input.mode;
        tool_result(bearer(&parts.headers).and_then(|token| {
            self.0.send_peer(
                token,
                ThreadId(input.thread_id),
                &input.client_request_id,
                &input.message,
            )
        }))
    }
}

#[tool_handler(name = "huggler")]
impl ServerHandler for AgentServer {}

pub(crate) async fn serve(
    listener: TcpListener,
    runtime: Arc<Shared>,
    mut stop: watch::Receiver<bool>,
) {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .enforce_origin_validation();
    let cancel = config.cancellation_token.clone();
    let server = AgentServer(runtime.clone());
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let router = Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(runtime, authenticate));
    let _ = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = stop.wait_for(|closed| *closed).await;
            cancel.cancel();
        })
        .await;
}
