//! Run-scoped agent tools on the runtime's loopback listener.
use crate::{
    Error, Result as HostResult, TaskId, ThreadId,
    contracts::agent_commands::{CreateThreads, Delegation},
    runtime::Shared,
};
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

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DelegateTask {
    #[serde(flatten)]
    delegation: Delegation,
    #[serde(default)]
    mode: TaskMode,
    /// Maximum time in milliseconds for wait mode. Defaults to 30 seconds, capped at 45 seconds.
    timeout_ms: Option<u64>,
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum TaskMode {
    #[default]
    Async,
    Wait,
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

#[derive(Deserialize, JsonSchema)]
struct Search {
    query: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Notice {
    client_request_id: String,
    text: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SecretInput {
    client_request_id: String,
    label: String,
}

impl AgentServer {
    fn permitted(&self, name: &str) -> bool {
        let policy = &self.0.agent_policy;
        policy
            .allowed_tools
            .as_ref()
            .is_none_or(|names| names.contains(name))
            && (name != "request_secret" || policy.secret_ttl_seconds.is_some_and(|n| n > 0))
    }
    fn visible(&self, token: &str, name: &str) -> HostResult<bool> {
        if !self.permitted(name) {
            return Ok(false);
        }
        if !self.0.agent_policy.defer_tools
            || matches!(
                name,
                "tool_search" | "orchestrator_capabilities" | "delegate_task" | "task_status"
            )
        {
            return Ok(true);
        }
        self.0.access(|db| db.discovered(token, name, false))
    }
}
#[tool_router]
impl AgentServer {
    #[tool(
        description = "List eligible orchestration tools for this caller. Search for a tool to load its schema."
    )]
    fn orchestrator_capabilities(&self) -> CallToolResult {
        let names: Vec<_> = Self::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name)
            .filter(|n| self.permitted(n))
            .collect();
        tool_result(Ok(
            json!({"tools":names,"deferred":self.0.agent_policy.defer_tools}),
        ))
    }
    #[tool(
        description = "Discover matching tools by name and expose their schemas to the current run."
    )]
    fn tool_search(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<Search>,
    ) -> CallToolResult {
        tool_result(bearer(&parts.headers).and_then(|token| {
            let query = input.query.trim().to_lowercase();
            if query.is_empty() {
                return Err(Error::Invalid("expected a tool name or keyword".into()));
            }
            let mut tools = Vec::new();
            for tool in Self::tool_router().list_all() {
                if self.permitted(&tool.name) && tool.name.contains(&query) {
                    self.0.access(|db| db.discovered(token, &tool.name, true))?;
                    tools.push(tool);
                }
            }
            Ok(json!({"tools":tools}))
        }))
    }
    #[tool(description = "Deliver an interim message to the host user in your own thread.")]
    fn send_to_user(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<Notice>,
    ) -> CallToolResult {
        tool_result(bearer(&parts.headers).and_then(|token| {
            self.0.schedule(|db| {
                db.notify_user(token, &input.client_request_id, &input.text)?;
                Ok((json!({"delivered":true}), []))
            })
        }))
    }
    #[tool(
        description = "Ask the host user for a secret. Poll with the same key and label for the decision and a one-use project reference."
    )]
    fn request_secret(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<SecretInput>,
    ) -> CallToolResult {
        tool_result(bearer(&parts.headers).and_then(|token| {
            self.0.access(|db| {
                let ttl = self
                    .0
                    .agent_policy
                    .secret_ttl_seconds
                    .ok_or(Error::Forbidden)?;
                db.request_secret(token, &input.client_request_id, &input.label, ttl)
            })
        }))
    }
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
        description = "Delegate a task to a new child thread. For each new round of a delegated review, call delegate_task with a fresh clientRequestId and include the original brief, prior findings and fixes to verify. Reuse the key for retries of the same round. Default isolation shared uses your current checkout. Use worktree to start a separate Git checkout at your current commit. Default async completion wakes you when idle. With mode wait, return on completion, a human approval request or timeout."
    )]
    async fn delegate_task(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<DelegateTask>,
    ) -> CallToolResult {
        let result: HostResult<_> = async {
            let token = bearer(&parts.headers)?;
            // Waiting is a property of this call. A retry may observe the same
            // child with a different timeout or return immediately.
            let task = self.0.delegate(token, input.delegation)?;
            match input.mode {
                TaskMode::Async => Ok(json!(task)),
                TaskMode::Wait => {
                    let limit = std::time::Duration::from_millis(
                        input.timeout_ms.unwrap_or(30_000).clamp(1, 45_000),
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
        description = "Send a message to an existing Huggler thread, including a reply to the independent peer identified in a message's origin. An idle recipient starts a run. A busy recipient queues it. For a follow-up addressed to a specific delegated child after its task settles, sending returns a new taskId. Reuse clientRequestId for an identical retry."
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
impl ServerHandler for AgentServer {
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let token = request_token(&context)?;
        let mut tools = Vec::new();
        for tool in Self::tool_router().list_all() {
            if self.visible(token, &tool.name).unwrap_or(false) {
                tools.push(tool);
            }
        }
        Ok(rmcp::model::ListToolsResult::with_all_items(tools))
    }
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        let token = request_token(&context)?;
        if !self.visible(token, &request.name).unwrap_or(false) {
            return Ok(tool_result(HostResult::<()>::Err(Error::Forbidden)).into());
        }
        Self::tool_router()
            .call(rmcp::handler::server::tool::ToolCallContext::new(
                self, request, context,
            ))
            .await
    }
}

fn request_token(
    ctx: &rmcp::service::RequestContext<rmcp::RoleServer>,
) -> Result<&str, rmcp::ErrorData> {
    ctx.extensions
        .get::<Parts>()
        .and_then(|p| bearer(&p.headers).ok())
        .ok_or_else(|| rmcp::ErrorData::invalid_request("missing caller", None))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegation_schema_and_parser_expose_the_same_flat_command() {
        let schema = serde_json::to_value(schemars::schema_for!(DelegateTask)).unwrap();
        let properties = schema["properties"].as_object().unwrap();
        for field in [
            "task",
            "clientRequestId",
            "target",
            "title",
            "role",
            "isolation",
            "mode",
            "timeoutMs",
        ] {
            assert!(properties.contains_key(field), "missing {field}");
        }
        assert_eq!(schema["additionalProperties"], false);
        let mut args =
            json!({"task":"review", "clientRequestId":"key", "mode":"wait", "timeoutMs":100});
        assert!(serde_json::from_value::<DelegateTask>(args.clone()).is_ok());
        args["typo"] = json!(true);
        assert!(serde_json::from_value::<DelegateTask>(args).is_err());
    }
}
