use crate::{
    contracts::*,
    providers::{
        claude_agent::{self, ClaudeAgentOptions},
        codex::{self, CodexOptions},
    },
    store::Database,
};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    process::Command as Process,
    sync::{Arc, Mutex},
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
    task::JoinSet,
};

#[derive(Clone, Debug)]
pub struct HugglerOptions {
    pub state_dir: PathBuf,
    pub codex: CodexOptions,
    pub claude_agent: ClaudeAgentOptions,
}
impl HugglerOptions {
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            codex: CodexOptions::default(),
            claude_agent: ClaudeAgentOptions::default(),
        }
    }
}

struct Shared {
    db: Mutex<Option<Database>>,
    decisions: Mutex<HashMap<RunId, mpsc::UnboundedSender<(RequestId, String)>>>,
    fault: Mutex<Option<String>>,
    changed: watch::Sender<()>,
    stop: watch::Sender<bool>,
}

impl Shared {
    fn access<T>(&self, f: impl FnOnce(&mut Database) -> Result<T>) -> Result<T> {
        let mut db = self.db.lock().unwrap();
        if *self.stop.borrow() {
            return Err(Error::Closed);
        }
        if let Some(reason) = self.fault.lock().unwrap().as_ref() {
            return Err(Error::Worker(reason.clone()));
        }
        f(db.as_mut().ok_or(Error::Closed)?)
    }

    async fn execute(&self, options: HugglerOptions, thread: ThreadId, run: RunId) {
        let claim = self.access(|db| db.claim(&run));
        let result = match claim {
            Ok(turn) => {
                let update = |change| {
                    self.access(|db| db.update(&thread, &run, change))?;
                    self.changed.send_replace(());
                    Ok(())
                };
                let (tx, decisions) = mpsc::unbounded_channel();
                self.decisions.lock().unwrap().insert(run.clone(), tx);
                let permissions = crate::providers::Permissions {
                    run: run.clone(),
                    decisions,
                    publish: |request| {
                        self.access(|db| db.request_permission(&thread, request))?;
                        self.changed.send_replace(());
                        Ok(())
                    },
                };
                let result = match turn.provider {
                    Provider::Codex => {
                        codex::execute(
                            options.codex,
                            turn,
                            self.stop.subscribe(),
                            update,
                            permissions,
                        )
                        .await
                    }
                    Provider::ClaudeAgent => {
                        claude_agent::execute(
                            &options.claude_agent,
                            turn,
                            serde_json::json!([]),
                            self.stop.subscribe(),
                            update,
                            permissions,
                        )
                        .await
                    }
                };
                self.decisions.lock().unwrap().remove(&run);
                result
            }
            Err(e) => Err(e),
        };
        // close joins this task before dropping the connection. Persist a terminal
        // result even when shutdown interrupted a provider call.
        let mut db = self.db.lock().unwrap();
        if let Err(e) = db.as_mut().unwrap().finish(&thread, &run, result) {
            *self.fault.lock().unwrap() = Some(e.to_string());
        }
        self.changed.send_replace(());
    }
}

/// The owner of a state directory. Share this handle via Arc inside the host.
pub struct Huggler {
    shared: Arc<Shared>,
    options: HugglerOptions,
    handle: Handle,
    workers: Mutex<Option<JoinSet<()>>>,
    closed: watch::Sender<bool>,
}

impl Huggler {
    /// Requires a Tokio runtime with I/O and time enabled.
    pub async fn open(options: HugglerOptions) -> Result<Self> {
        let dir = options.state_dir.clone();
        let db = tokio::task::spawn_blocking(move || Database::open(&dir))
            .await
            .map_err(|e| Error::Worker(e.to_string()))??;
        Ok(Self {
            shared: Arc::new(Shared {
                db: Mutex::new(Some(db)),
                decisions: Mutex::default(),
                fault: Mutex::default(),
                changed: watch::channel(()).0,
                stop: watch::channel(false).0,
            }),
            options,
            handle: Handle::current(),
            workers: Mutex::new(Some(JoinSet::new())),
            closed: watch::channel(false).0,
        })
    }

    /// Resolve the enclosing Git checkout and store its absolute working directory.
    pub fn open_project(&self, path: impl AsRef<Path>) -> Result<Project> {
        let output = Process::new("git")
            .arg("-C")
            .arg(path.as_ref())
            .args(["rev-parse", "--show-toplevel"])
            .output()?;
        if !output.status.success() {
            return Err(Error::Invalid("expected a Git working tree".into()));
        }
        let path = std::str::from_utf8(&output.stdout)
            .map_err(|_| Error::Invalid("workspace path requires UTF-8".into()))?;
        let root = Path::new(path.trim_end_matches(['\r', '\n'])).canonicalize()?;
        self.shared.access(|db| db.project(&root))
    }

    /// Commit a launch, model selection or message. Reusing the same command
    /// identity and payload returns the original receipt, including after reopen.
    pub fn dispatch(&self, command: Command) -> Result<Receipt> {
        let mut gate = self.workers.lock().unwrap();
        let workers = gate.as_mut().ok_or(Error::Closed)?;
        while let Some(completed) = workers.try_join_next() {
            if let Err(e) = completed {
                return Err(Error::Worker(e.to_string()));
            }
        }
        let (receipt, fresh) = self.shared.access(|db| db.dispatch(&command))?;
        if fresh {
            self.shared.changed.send_replace(());
            if let Some(run) = receipt.run_id.clone() {
                let shared = self.shared.clone();
                let options = self.options.clone();
                workers.spawn_on(
                    async move { shared.execute(options, command.thread_id, run).await },
                    &self.handle,
                );
            }
        }
        Ok(receipt)
    }

    pub fn launch(
        &self,
        id: CommandId,
        thread: ThreadId,
        project: ProjectId,
        name: String,
        model: Option<String>,
    ) -> Result<Receipt> {
        self.dispatch(Command {
            id,
            thread_id: thread,
            action: Action::Launch {
                project,
                name,
                model,
            },
        })
    }
    pub fn send(&self, id: CommandId, thread: ThreadId, text: String) -> Result<Receipt> {
        self.dispatch(Command {
            id,
            thread_id: thread,
            action: Action::Send { text },
        })
    }
    /// Answer one live provider request on its owning thread and run.
    pub fn respond_to_permission(
        &self,
        thread: &ThreadId,
        run: &RunId,
        id: &RequestId,
        option: String,
    ) -> Result<()> {
        let decisions = self.shared.decisions.lock().unwrap();
        let tx = decisions
            .get(run)
            .filter(|t| !t.is_closed())
            .ok_or_else(|| Error::NotFound("live permission responder".into()))?;
        self.shared
            .access(|db| db.decide(thread, run, id, &option))?;
        self.shared.changed.send_replace(());
        tx.send((id.clone(), option))
            .map_err(|_| Error::NotFound("live permission responder".into()))
    }
    pub fn threads(&self) -> Result<Vec<ThreadSummary>> {
        self.shared.access(|db| db.threads())
    }

    pub fn read(&self, id: &ThreadId) -> Result<Thread> {
        self.shared.access(|db| db.read(id))
    }

    /// Wait for one specific application run. Dropping or timing out the future
    /// releases the observer while the provider continues.
    pub async fn wait(&self, thread: &ThreadId, run: &RunId) -> Result<Run> {
        let mut changed = self.shared.changed.subscribe();
        loop {
            let state = self.shared.access(|db| db.find_run(thread, run))?;
            if state.status.terminal() {
                return Ok(state);
            }
            changed.changed().await.map_err(|_| Error::Closed)?;
        }
    }

    /// Subscribe from a consistent snapshot, or replay events strictly after a saved cursor.
    pub fn subscribe(&self, id: &ThreadId, after: Option<i64>) -> Result<(Thread, Subscription)> {
        let changed = self.shared.changed.subscribe();
        let snapshot = self.read(id)?;
        let cursor = after.unwrap_or(snapshot.cursor);
        if cursor < 0 || cursor > snapshot.cursor {
            return Err(Error::Invalid("cursor outside thread snapshot".into()));
        }
        let stream = Subscription {
            thread: id.clone(),
            shared: self.shared.clone(),
            changed,
            cursor,
            buffer: VecDeque::new(),
        };
        Ok((snapshot, stream))
    }

    fn start_close(&self) {
        let mut gate = self.workers.lock().unwrap();
        let Some(mut workers) = gate.take() else {
            return;
        };
        self.shared.stop.send_replace(true);
        self.shared.changed.send_replace(());
        let shared = self.shared.clone();
        let closed = self.closed.clone();
        self.handle.spawn(async move {
            while let Some(result) = workers.join_next().await {
                if let Err(e) = result {
                    *shared.fault.lock().unwrap() = Some(e.to_string());
                }
            }
            shared.db.lock().unwrap().take();
            closed.send_replace(true);
        });
    }

    /// Stop intake and wait for managed children, workers and the state lock.
    pub async fn close(&self) -> Result<()> {
        self.start_close();
        self.closed
            .subscribe()
            .wait_for(|value| *value)
            .await
            .map_err(|_| Error::Closed)?;
        if let Some(reason) = self.shared.fault.lock().unwrap().as_ref() {
            return Err(Error::Worker(reason.clone()));
        }
        Ok(())
    }
}
impl Drop for Huggler {
    fn drop(&mut self) {
        self.start_close();
    }
}

pub struct Subscription {
    shared: Arc<Shared>,
    thread: ThreadId,
    changed: watch::Receiver<()>,
    cursor: i64,
    buffer: VecDeque<Event>,
}
impl Subscription {
    pub async fn next(&mut self) -> Result<Event> {
        loop {
            if *self.shared.stop.borrow() {
                return Err(Error::Closed);
            }
            if self.buffer.is_empty() {
                self.buffer = self
                    .shared
                    .access(|db| db.events(&self.thread, self.cursor))?
                    .into();
            }
            if let Some(event) = self.buffer.pop_front() {
                self.cursor = event.sequence;
                return Ok(event);
            }
            self.changed.changed().await.map_err(|_| Error::Closed)?;
        }
    }
}
