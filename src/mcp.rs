//! Run-scoped agent tools on the runtime's loopback listener.
use crate::{Error, Provider, Result as HostResult, RunId, TaskId, ThreadId, runtime::Shared};
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
struct PeerSend {
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

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Delegation {
    pub task: String,
    pub client_request_id: String,
    /// Omit to use the parent's configured provider.
    pub target: Option<Target>,
    pub title: Option<String>,
    pub role: Option<String>,
    #[serde(default)]
    mode: TaskMode,
    /// Maximum time in milliseconds for wait mode. Defaults to 30 seconds, capped at 45 seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default)]
    isolation: Isolation,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Target {
    pub provider_instance_id: Provider,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateThreads {
    pub client_request_id: String,
    #[schemars(length(min = 1, max = 20))]
    pub threads: Vec<CreatePeer>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreatePeer {
    pub title: Option<String>,
    /// Omit to inherit the caller's provider.
    pub target: Option<Target>,
    /// Omit to create an idle peer.
    pub prompt: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatedPeer {
    pub thread_id: ThreadId,
    pub run_id: Option<RunId>,
}

#[derive(Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum TaskMode {
    #[default]
    Async,
    Wait,
}

#[derive(Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Isolation {
    #[default]
    Shared,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskLookup {
    task_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThreadInterrupt {
    thread_id: String,
    run_id: String,
    client_request_id: String,
    reason: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskCancel {
    task_id: String,
    /// Reuse this key for retries on this task, including from a later run.
    client_request_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum QuestionAction {
    List,
    Read,
    Respond,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingRequest {
    action: QuestionAction,
    /// Omit to select the caller's thread.
    thread_id: Option<String>,
    /// Required for read and respond.
    request_id: Option<String>,
    /// Required for respond. Keys are question IDs.
    answers: Option<crate::UserAnswers>,
}

fn tool_result(value: HostResult<impl Serialize>) -> CallToolResult {
    match value {
        Ok(value) => CallToolResult::structured(json!(value)),
        Err(error) => CallToolResult::structured_error(json!(error)),
    }
}

#[tool_router]
impl AgentServer {
    #[tool(
        description = "List, read or answer pending user questions. Omit threadId for your own thread. Use the question IDs from read as keys in answers. Provider permissions require a human responder."
    )]
    async fn pending_request(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<PendingRequest>,
    ) -> CallToolResult {
        let result: HostResult<_> = async {
            let token = bearer(&parts.headers)?;
            let target = input.thread_id.map(ThreadId);
            let request_id = || {
                input
                    .request_id
                    .map(crate::RequestId)
                    .ok_or_else(|| Error::Invalid("requestId is required".into()))
            };
            match input.action {
                QuestionAction::List => self.0.access(|db| {
                    let (thread, _) = db.question_target(token, target.as_ref())?;
                    let ids: Vec<_> = db
                        .open_questions(&thread)?
                        .into_iter()
                        .map(|q| q.id)
                        .collect();
                    Ok(json!({"requestIds": ids}))
                }),
                QuestionAction::Read => self.0.access(|db| {
                    let (thread, _) = db.question_target(token, target.as_ref())?;
                    let (request, _) = db.question(&thread, &request_id()?, None)?;
                    Ok(json!({"requestId": request.id, "questions": request.questions}))
                }),
                QuestionAction::Respond => {
                    let id = request_id()?;
                    let answers = input
                        .answers
                        .ok_or_else(|| Error::Invalid("answers are required".into()))?;
                    self.0
                        .answer_as_agent(token, target.as_ref(), &id, answers)
                        .await?;
                    Ok(json!({"requestId": id}))
                }
            }
        }
        .await;
        tool_result(result)
    }

    #[tool(
        description = "Delegate a task to a new child thread in the same checkout. Default async completion wakes you when idle. With mode wait, return on completion, a human approval request or timeout. Reuse clientRequestId to retry."
    )]
    async fn delegate_task(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(mut input): Parameters<Delegation>,
    ) -> CallToolResult {
        let result: HostResult<_> = async {
            let token = bearer(&parts.headers)?;
            // Waiting is a property of this call. A retry may observe the same
            // child with a different timeout or return immediately.
            let mode = std::mem::take(&mut input.mode);
            let timeout = input.timeout_ms.take();
            let task = self.0.delegate(token, input)?;
            match mode {
                TaskMode::Async => Ok(json!(task)),
                TaskMode::Wait => {
                    let limit = std::time::Duration::from_millis(
                        timeout.unwrap_or(30_000).clamp(1, 45_000),
                    );
                    Ok(json!(
                        self.0.wait_for_task(token, &task.task_id, limit).await?
                    ))
                }
            }
        }
        .await;
        tool_result(result)
    }

    #[tool(
        description = "Read your delegated task. Reading a terminal result acknowledges it and suppresses a pending wake."
    )]
    fn task_status(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<TaskLookup>,
    ) -> CallToolResult {
        tool_result(
            bearer(&parts.headers)
                .and_then(|token| self.0.task_status(token, &TaskId(input.task_id))),
        )
    }

    #[tool(
        description = "Stop current and queued work in your delegated child's subtree. A settled task keeps its result. Reuse clientRequestId to retry the same stop."
    )]
    fn task_cancel(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<TaskCancel>,
    ) -> CallToolResult {
        tool_result(bearer(&parts.headers).and_then(|token| {
            self.0.cancel_task(
                token,
                &TaskId(input.task_id),
                input.client_request_id.as_deref(),
            )
        }))
    }

    #[tool(
        description = "Interrupt one starting or running turn in your project. Specify its threadId and runId. Queued turns can proceed. Reuse clientRequestId for retries. A settled run returns its terminal status."
    )]
    fn thread_interrupt(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<ThreadInterrupt>,
    ) -> CallToolResult {
        tool_result(bearer(&parts.headers).and_then(|token| {
            let thread = ThreadId(input.thread_id);
            let run = crate::RunId(input.run_id);
            let terminal = self.0.interrupt_run(
                token,
                &thread,
                &run,
                &input.client_request_id,
                input.reason.as_deref(),
            )?;
            let status = terminal.map_or_else(|| json!("interrupt_requested"), |s| json!(s));
            Ok(json!({"threadId": thread, "runId": run, "status": status}))
        }))
    }

    #[tool(
        description = "Create 1 to 20 independent peers in your project and checkout. Returns their thread IDs and initial run IDs in input order. Reuse clientRequestId with the same ordered entries to retry or to add a first prompt to an idle peer."
    )]
    fn create_threads(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<CreateThreads>,
    ) -> CallToolResult {
        tool_result(bearer(&parts.headers).and_then(|token| {
            let threads = self.0.create_threads(token, input)?;
            Ok(json!({ "threads": threads }))
        }))
    }

    #[tool(description = "List other Huggler threads in your project and their addresses.")]
    fn thread_list(&self, Extension(parts): Extension<Parts>) -> CallToolResult {
        tool_result(
            self.0
                .access(|db| Ok(json!({"threads":db.peers(bearer(&parts.headers)?)?}))),
        )
    }

    #[tool(
        description = "Send a message to another Huggler thread. An idle recipient starts a run. A busy recipient queues it. Sending to your delegated child after its original task settles returns a new taskId. Reuse clientRequestId for an identical retry."
    )]
    fn thread_send(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<PeerSend>,
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
