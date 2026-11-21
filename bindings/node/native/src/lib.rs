mod options;
mod wire;

use huggler::{
    self as core, Action, Command, CommandId, Error, Huggler, MessageId, ProjectId,
    ProviderInstanceId, RequestId, RunId, ThreadId,
};
use napi::{Env, Result, bindgen_prelude::PromiseRaw};
use napi_derive::napi;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    future::poll_fn,
    pin::pin,
    sync::{Arc, Weak},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> core::Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| wire::error(Error::Worker(e.to_string())))?
        .map_err(wire::error)
}

// Core futures can take a synchronous SQLite lock or touch the filesystem
// on any poll. Let Tokio hand off its scheduler while a poll runs, without
// occupying a blocking thread while the observer is asleep.
async fn poll_core<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    poll_fn(|cx| tokio::task::block_in_place(|| future.as_mut().poll(cx))).await
}

// One registry per JS environment. Its cleanup hook runs before NAPI-RS tears
// down Tokio, including when the parent forcibly terminates a Node Worker.
#[derive(Default)]
struct Owners {
    closing: bool,
    runtimes: Vec<Weak<Huggler>>,
}

#[napi]
pub struct NativeEnvironment {
    owners: Arc<Mutex<Owners>>,
}

#[napi]
impl NativeEnvironment {
    #[napi(constructor)]
    pub fn new(env: &Env) -> Result<Self> {
        let owners = Arc::new(Mutex::new(Owners::default()));
        env.add_env_cleanup_hook(owners.clone(), |owners| {
            napi::bindgen_prelude::block_on(async {
                let mut owners = owners.lock().await;
                owners.closing = true;
                for owner in owners.runtimes.drain(..).filter_map(|r| r.upgrade()) {
                    let _ = owner.close().await;
                }
            });
        })?;
        Ok(Self { owners })
    }

    #[napi]
    pub fn open<'a>(&self, env: &'a Env, options: String) -> Result<PromiseRaw<'a, NativeRuntime>> {
        let owners = self.owners.clone();
        env.spawn_future(async move {
            let options = wire::parse::<options::Options>(&options)
                .and_then(options::Options::into_core)
                .map_err(wire::error)?;
            let mut owners = owners.lock().await;
            if owners.closing {
                return Err(wire::error(Error::Closed));
            }
            let runtime = Arc::new(
                poll_core(Huggler::open(options))
                    .await
                    .map_err(wire::error)?,
            );
            owners.runtimes.retain(|r| r.strong_count() > 0);
            owners.runtimes.push(Arc::downgrade(&runtime));
            Ok(NativeRuntime { runtime })
        })
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Operation {
    OpenProject {
        path: String,
    },
    ListProviderInstances,
    ReadProviderUsage,
    Launch {
        command_id: CommandId,
        thread_id: ThreadId,
        project_id: ProjectId,
        provider_instance_id: ProviderInstanceId,
        name: String,
        model: Option<String>,
    },
    Send {
        command_id: CommandId,
        thread_id: ThreadId,
        text: String,
        mode: Option<SendMode>,
    },
    StopThread {
        command_id: CommandId,
        thread_id: ThreadId,
    },
    ListThreads,
    ReadThread {
        thread_id: ThreadId,
    },
    ReadHistory {
        thread_id: ThreadId,
        before: Option<String>,
        query: String,
        limit: usize,
    },
    ReadMessage {
        thread_id: ThreadId,
        message_id: MessageId,
    },
    RespondToPermission {
        thread_id: ThreadId,
        run_id: RunId,
        request_id: RequestId,
        option: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SendMode {
    Default,
    Plan,
}

fn dispatch(
    rt: &Huggler,
    id: CommandId,
    thread_id: ThreadId,
    action: Action,
) -> core::Result<Value> {
    rt.dispatch(Command {
        id,
        thread_id,
        action,
    })
    .map(wire::receipt)
}

impl Operation {
    fn execute(self, rt: &Huggler) -> core::Result<Value> {
        let value = match self {
            Self::OpenProject { path } => json!(rt.open_project(path)?),
            Self::ListProviderInstances => json!(rt.provider_instances().collect::<Vec<_>>()),
            Self::ReadProviderUsage => {
                let usage = rt
                    .provider_usage()?
                    .into_iter()
                    .map(|usage| {
                        json!({
                            "providerInstanceId": usage.instance,
                            "reportedRuns": usage.reported_runs.to_string(),
                            "inputTokens": usage.input_tokens.to_string(),
                            "outputTokens": usage.output_tokens.to_string(),
                        })
                    })
                    .collect();
                Value::Array(usage)
            }
            Self::Launch {
                command_id,
                thread_id,
                project_id,
                provider_instance_id,
                name,
                model,
            } => {
                let action = Action::LaunchInstance {
                    project: project_id,
                    instance: provider_instance_id,
                    name,
                    model,
                };
                dispatch(rt, command_id, thread_id, action)?
            }
            Self::Send {
                command_id,
                thread_id,
                text,
                mode,
            } => {
                let action = match mode {
                    None | Some(SendMode::Default) => Action::Send { text },
                    Some(SendMode::Plan) => Action::SendPlan { text },
                };
                dispatch(rt, command_id, thread_id, action)?
            }
            Self::StopThread {
                command_id,
                thread_id,
            } => wire::receipt(rt.stop_thread(command_id, thread_id)?),
            Self::ListThreads => json!(rt.threads()?),
            Self::ReadThread { thread_id } => wire::thread(rt.read(&thread_id)?),
            Self::ReadHistory {
                thread_id,
                before,
                query,
                limit,
            } => {
                let cursor = before.as_deref().map(wire::parse).transpose()?;
                wire::detail(rt.detail(&thread_id, cursor.as_ref(), &query, limit)?)?
            }
            Self::ReadMessage {
                thread_id,
                message_id,
            } => wire::message(rt.read_message(&thread_id, &message_id)?),
            Self::RespondToPermission {
                thread_id,
                run_id,
                request_id,
                option,
            } => {
                rt.respond_to_permission(&thread_id, &run_id, &request_id, option)?;
                Value::Null
            }
        };
        Ok(value)
    }
}

#[napi]
pub struct NativeRuntime {
    runtime: Arc<Huggler>,
}

#[napi]
impl NativeRuntime {
    #[napi]
    pub fn execute<'a>(&self, env: &'a Env, request: String) -> Result<PromiseRaw<'a, String>> {
        let rt = self.runtime.clone();
        env.spawn_future(blocking(move || {
            Ok(wire::parse::<Operation>(&request)?
                .execute(&rt)?
                .to_string())
        }))
    }

    #[napi]
    pub async fn probe(&self, instance: String, cwd: String) -> Result<String> {
        let report = poll_core(self.runtime.probe_provider(&instance, cwd))
            .await
            .map_err(wire::error)?;
        Ok(json!({
            "version": report.version,
            "transport": report.transport,
            "authenticated": report.authenticated,
            "models": report.models,
            "resume": report.resume,
            "agentTools": report.agent_tools,
        })
        .to_string())
    }

    #[napi]
    pub async fn answer(
        &self,
        thread: String,
        run: String,
        request: String,
        answers: String,
    ) -> Result<()> {
        let answers = wire::parse(&answers).map_err(wire::error)?;
        poll_core(self.runtime.respond_to_question(
            &ThreadId(thread),
            &RunId(run),
            &RequestId(request),
            answers,
        ))
        .await
        .map_err(wire::error)
    }

    #[napi]
    pub fn wait<'a>(
        &self,
        env: &'a Env,
        thread: String,
        run: String,
        cancel: &NativeCancel,
    ) -> Result<PromiseRaw<'a, Option<String>>> {
        let rt = self.runtime.clone();
        let cancelled = cancel.signal.clone();
        env.spawn_future(async move {
            let thread = ThreadId(thread);
            let run = RunId(run);
            tokio::select! {
                biased;
                _ = cancelled.cancelled() => Ok(None),
                r = poll_core(rt.wait(&thread, &run)) => Ok(Some(wire::run(r.map_err(wire::error)?).to_string())),
            }
        })
    }

    #[napi]
    pub fn subscribe<'a>(
        &self,
        env: &'a Env,
        thread: String,
        after: Option<String>,
    ) -> Result<PromiseRaw<'a, SubscriptionStart>> {
        let rt = self.runtime.clone();
        env.spawn_future(blocking(move || {
            let after = after
                .map(|s| {
                    s.parse::<i64>()
                        .map_err(|_| Error::Invalid("invalid event cursor".into()))
                })
                .transpose()?;
            let (snapshot, stream) = rt.subscribe(&ThreadId(thread), after)?;
            Ok(SubscriptionStart {
                snapshot: wire::thread(snapshot).to_string(),
                stream: NativeSubscription {
                    _owner: rt,
                    stream: Mutex::new(stream),
                },
            })
        }))
    }

    #[napi]
    pub async fn close(&self) -> Result<()> {
        poll_core(self.runtime.close()).await.map_err(wire::error)
    }
}

#[derive(Default)]
#[napi]
pub struct NativeCancel {
    signal: CancellationToken,
}

#[napi]
impl NativeCancel {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            signal: CancellationToken::new(),
        }
    }

    #[napi]
    pub fn cancel(&self) {
        self.signal.cancel();
    }
}

#[napi(object, object_from_js = false)]
pub struct SubscriptionStart {
    pub snapshot: String,
    pub stream: NativeSubscription,
}

#[napi]
pub struct NativeSubscription {
    // An iterator can outlive its JS host handle. Explicit close still wakes it.
    _owner: Arc<Huggler>,
    stream: Mutex<core::Subscription>,
}

#[napi]
impl NativeSubscription {
    #[napi]
    pub async fn next(&self, cancel: &NativeCancel) -> Result<Option<String>> {
        let mut stream = self
            .stream
            .try_lock()
            .map_err(|_| wire::error(Error::Invalid("next already pending".into())))?;
        let cancelled = cancel.signal.clone();
        tokio::select! {
            biased;
            _ = cancelled.cancelled() => Ok(None),
            e = poll_core(stream.next()) => Ok(Some(wire::event(e.map_err(wire::error)?).to_string())),
        }
    }
}
