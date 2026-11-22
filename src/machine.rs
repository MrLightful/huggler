//! Owner-only desktop operations over an SSH-forwarded Unix socket.
use crate::*;
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Machine {
    pub id: EnvironmentId,
    pub version: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Identify,
    OpenProject {
        checkout: PathBuf,
    },
    List {
        project: ProjectId,
    },
    Launch {
        project: ProjectId,
        thread: ThreadId,
        command: CommandId,
        name: String,
    },
    Detail {
        project: ProjectId,
        thread: ThreadId,
        after: Option<i64>,
        before: Option<HistoryCursor>,
    },
    MessageBody {
        project: ProjectId,
        thread: ThreadId,
        message: MessageId,
    },
    Send {
        project: ProjectId,
        thread: ThreadId,
        command: CommandId,
        text: String,
    },
    Stop {
        project: ProjectId,
        thread: ThreadId,
        command: CommandId,
    },
    ResumeQueue {
        project: ProjectId,
        thread: ThreadId,
        command: CommandId,
    },
    Permission {
        project: ProjectId,
        thread: ThreadId,
        run: RunId,
        request: RequestId,
        option: String,
    },
    Answer {
        project: ProjectId,
        thread: ThreadId,
        run: RunId,
        request: RequestId,
        answers: Option<UserAnswers>,
    },
}

impl Request {
    fn thread_scope(&self) -> Option<(&ProjectId, &ThreadId)> {
        match self {
            Self::Identify | Self::OpenProject { .. } | Self::List { .. } | Self::Launch { .. } => {
                None
            }
            Self::Detail {
                project, thread, ..
            }
            | Self::MessageBody {
                project, thread, ..
            }
            | Self::Send {
                project, thread, ..
            }
            | Self::Stop {
                project, thread, ..
            }
            | Self::ResumeQueue {
                project, thread, ..
            }
            | Self::Permission {
                project, thread, ..
            }
            | Self::Answer {
                project, thread, ..
            } => Some((project, thread)),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Reply {
    Machine(Machine),
    Project(Project),
    Conversations(Vec<ConversationSummary>),
    Detail(Box<ConversationDetail>),
    MessageBody(String),
    Unchanged,
    Receipt(Receipt),
    Responded,
}

#[derive(Serialize, Deserialize)]
struct Call {
    environment: Option<EnvironmentId>,
    request: Request,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("machine connection: {0}")]
    Connection(#[from] reqwest::Error),
    #[error("the machine returned an incompatible response")]
    Protocol,
    #[error("{0}")]
    Rejected(String),
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    environment: Option<EnvironmentId>,
}

impl Client {
    pub fn new(socket: &Path) -> std::result::Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .unix_socket(socket.to_owned())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            http,
            environment: None,
        })
    }

    pub fn for_environment(mut self, environment: EnvironmentId) -> Self {
        self.environment = Some(environment);
        self
    }

    pub async fn request(&self, request: Request) -> std::result::Result<Reply, ClientError> {
        let response = self
            .http
            .post("http://huggler/v1/machine")
            .json(&Call {
                environment: self.environment.clone(),
                request,
            })
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
            return Err(ClientError::Rejected(
                "The request exceeds the machine's upload limit.".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(ClientError::Protocol);
        }
        let body = response.bytes().await?;
        let reply: std::result::Result<Reply, String> =
            serde_json::from_slice(&body).map_err(|_| ClientError::Protocol)?;
        reply.map_err(ClientError::Rejected)
    }
}

/// Serve the desktop API on a socket inside a directory private to the SSH account.
/// The socket must belong to this runtime's state directory so its database lock
/// also protects socket creation and cleanup.
pub async fn serve(
    runtime: Arc<Huggler>,
    socket: PathBuf,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let parent = socket
        .parent()
        .ok_or_else(|| Error::Invalid("missing socket directory".into()))?;
    let directory = std::fs::symlink_metadata(parent)?;
    if !directory.is_dir()
        || directory.mode() & 0o077 != 0
        || directory.uid() != unsafe { libc::geteuid() }
    {
        return Err(Error::Invalid(
            "the machine socket directory must be owned by this user and have mode 0700".into(),
        ));
    }
    match std::fs::symlink_metadata(&socket) {
        Ok(entry) if entry.file_type().is_socket() => {
            match std::os::unix::net::UnixStream::connect(&socket) {
                Ok(_) => return Err(Error::AlreadyOpen),
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                    std::fs::remove_file(&socket)?
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(_) => return Err(Error::Invalid("the machine socket path is occupied".into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    let listener = tokio::net::UnixListener::bind(&socket)?;
    struct RemoveSocket(PathBuf);
    impl Drop for RemoveSocket {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let cleanup = RemoveSocket(socket);
    std::fs::set_permissions(&cleanup.0, std::fs::Permissions::from_mode(0o600))?;
    let router = Router::new()
        .route("/v1/machine", post(handle))
        .with_state(runtime);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

async fn handle(
    State(runtime): State<Arc<Huggler>>,
    Json(call): Json<Call>,
) -> Json<std::result::Result<Reply, String>> {
    Json(
        execute(runtime, call)
            .await
            .map_err(|error| error.to_string()),
    )
}

async fn execute(runtime: Arc<Huggler>, call: Call) -> Result<Reply> {
    let id = runtime.shared.access(|db| db.environment())?;
    if matches!(call.request, Request::Identify) {
        return Ok(Reply::Machine(Machine {
            id,
            version: env!("CARGO_PKG_VERSION").into(),
        }));
    }
    if call.environment.as_ref() != Some(&id) {
        return Err(Error::Invalid(
            "this SSH target reaches a different Huggler environment".into(),
        ));
    }
    if let Some((project, thread)) = call.request.thread_scope() {
        runtime
            .shared
            .access(|db| db.require_thread_project(thread, project))?;
    }
    match call.request {
        Request::Identify => unreachable!(),
        Request::OpenProject { checkout } => {
            if !checkout.is_absolute() {
                return Err(Error::Invalid(
                    "enter the absolute path to the remote Git checkout".into(),
                ));
            }
            tokio::task::spawn_blocking(move || runtime.open_project(checkout).map(Reply::Project))
                .await
                .map_err(|err| Error::Worker(err.to_string()))?
        }
        Request::List { project } => runtime.shared.access(|db| {
            Ok(Reply::Conversations(
                db.conversation_list_for_project(Some(&project))?.threads,
            ))
        }),
        Request::Launch {
            project,
            thread,
            command,
            name,
        } => runtime
            .launch(command, thread, project, name, None)
            .map(Reply::Receipt),
        Request::Detail {
            thread,
            after,
            before,
            ..
        } => {
            if let Some(after) = after {
                match tokio::time::timeout(
                    Duration::from_secs(5),
                    runtime.wait_for_change(&thread, after),
                )
                .await
                {
                    Ok(result) => {
                        result?;
                    }
                    Err(_) => return Ok(Reply::Unchanged),
                }
            }
            runtime
                .detail(&thread, before.as_ref(), "", 32)
                .map(|detail| Reply::Detail(Box::new(detail)))
        }
        Request::MessageBody {
            thread, message, ..
        } => runtime
            .read_message_body(&thread, &message)
            .map(Reply::MessageBody),
        Request::Send {
            thread,
            command,
            text,
            ..
        } => runtime.send(command, thread, text).map(Reply::Receipt),
        Request::Stop {
            thread, command, ..
        } => runtime.stop_thread(command, thread).map(Reply::Receipt),
        Request::ResumeQueue {
            thread, command, ..
        } => runtime.resume_queue(command, thread).map(Reply::Receipt),
        Request::Permission {
            thread,
            run,
            request,
            option,
            ..
        } => runtime
            .respond_to_permission(&thread, &run, &request, option)
            .map(|()| Reply::Responded),
        Request::Answer {
            thread,
            run,
            request,
            answers,
            ..
        } => {
            match answers {
                Some(answers) => {
                    runtime
                        .respond_to_question(&thread, &run, &request, answers)
                        .await?
                }
                None => runtime.dismiss_question(&thread, &run, &request).await?,
            }
            Ok(Reply::Responded)
        }
    }
}
