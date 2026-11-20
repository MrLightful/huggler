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
    /// Enable caller-scoped orchestration tools over loopback HTTP.
    pub agent_tools: bool,
    pub codex: CodexOptions,
    pub claude_agent: ClaudeAgentOptions,
}
impl HugglerOptions {
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            agent_tools: false,
            codex: CodexOptions::default(),
            claude_agent: ClaudeAgentOptions::default(),
        }
    }
}

pub(crate) struct Shared {
    options: HugglerOptions,
    endpoint: Option<String>,
    handle: Handle,
    workers: Mutex<Option<JoinSet<()>>>,
    db: Mutex<Option<Database>>,
    run_stops: Mutex<HashMap<RunId, watch::Sender<bool>>>,
    decisions: Mutex<HashMap<RunId, mpsc::UnboundedSender<(RequestId, String)>>>,
    fault: Mutex<Option<String>>,
    changed: watch::Sender<()>,
    stop: watch::Sender<bool>,
}

impl Shared {
    pub(crate) fn access<T>(&self, f: impl FnOnce(&mut Database) -> Result<T>) -> Result<T> {
        let mut db = self.db.lock().unwrap();
        if *self.stop.borrow() {
            return Err(Error::Closed);
        }
        if let Some(reason) = self.fault.lock().unwrap().as_ref() {
            return Err(Error::Worker(reason.clone()));
        }
        f(db.as_mut().ok_or(Error::Closed)?)
    }

    // Hold the intake gate through commit and worker registration. Shutdown
    // takes this same gate before taking ownership of all registered workers.
    fn schedule<T>(
        self: &Arc<Self>,
        write: impl FnOnce(&mut Database) -> Result<(T, Option<(ThreadId, RunId)>)>,
    ) -> Result<T> {
        let mut gate = self.workers.lock().unwrap();
        let workers = gate.as_mut().ok_or(Error::Closed)?;
        while let Some(completed) = workers.try_join_next() {
            if let Err(e) = completed {
                return Err(Error::Worker(e.to_string()));
            }
        }
        let (value, job) = self.access(write)?;
        self.changed.send_replace(());
        if let Some((thread, run)) = job {
            self.spawn(workers, thread, run);
        }
        Ok(value)
    }

    pub(crate) fn send_peer(
        self: &Arc<Self>,
        token: &str,
        target: ThreadId,
        key: &str,
        text: &str,
    ) -> Result<PeerReceipt> {
        self.schedule(|db| {
            let (receipt, fresh) = db.send_peer(token, &target, key, text)?;
            let job = (fresh && receipt.delivery == PeerDelivery::Started)
                .then(|| (target, receipt.run_id.clone()));
            Ok((receipt, job))
        })
    }

    pub(crate) fn delegate(
        self: &Arc<Self>,
        token: &str,
        input: crate::mcp::Delegation,
    ) -> Result<DelegatedTask> {
        self.schedule(|db| {
            let (task, fresh) = db.delegate(token, input)?;
            let job = fresh.then(|| (task.child_thread_id.clone(), task.child_run_id.clone()));
            Ok((task, job))
        })
    }

    pub(crate) fn task_status(&self, token: &str, id: &TaskId) -> Result<DelegatedTask> {
        let task = self.access(|db| db.task_status(token, id))?;
        self.changed.send_replace(());
        Ok(task)
    }

    pub(crate) fn cancel_task(self: &Arc<Self>, token: &str, id: &TaskId) -> Result<DelegatedTask> {
        self.schedule(|db| {
            let (task, runs) = db.cancel_task(token, id)?;
            let stops = self.run_stops.lock().unwrap();
            for id in runs {
                if let Some(stop) = stops.get(&id) {
                    stop.send_replace(true);
                }
            }
            Ok((task, None))
        })
    }

    fn spawn(self: &Arc<Self>, workers: &mut JoinSet<()>, thread: ThreadId, run: RunId) {
        let (stop, stopped) = watch::channel(false);
        self.run_stops.lock().unwrap().insert(run.clone(), stop);
        let shared = self.clone();
        workers.spawn_on(
            async move { shared.work(thread, run, stopped).await },
            &self.handle,
        );
    }

    async fn work(self: Arc<Self>, thread: ThreadId, run: RunId, stopped: watch::Receiver<bool>) {
        let result = self.execute(&thread, &run, stopped).await;
        let mut gate = self.workers.lock().unwrap();
        let mut guard = self.db.lock().unwrap();
        let db = guard
            .as_mut()
            .expect("close joins workers before releasing storage");
        self.run_stops.lock().unwrap().remove(&run);
        let finished = db.finish(&thread, &run, result).and_then(|()| {
            if let Some(workers) = gate.as_mut() {
                for (thread, run) in db.ready_jobs()? {
                    self.spawn(workers, thread, run);
                }
            }
            Ok(())
        });
        if let Err(error) = finished {
            *self.fault.lock().unwrap() = Some(error.to_string());
        }
        self.changed.send_replace(());
    }

    async fn execute(
        &self,
        thread: &ThreadId,
        run: &RunId,
        stopped: watch::Receiver<bool>,
    ) -> Result<()> {
        let turn = self.access(|db| {
            let mut turn = db.claim(run)?;
            if let Some(url) = &self.endpoint {
                turn.agent_tools = Some(crate::providers::AgentTools {
                    url: url.clone(),
                    token: db.grant(run)?,
                });
            }
            Ok(turn)
        })?;
        let update = |change| {
            self.access(|db| db.update(thread, run, change))?;
            self.changed.send_replace(());
            Ok(())
        };
        let (tx, decisions) = mpsc::unbounded_channel();
        self.decisions.lock().unwrap().insert(run.clone(), tx);
        let permissions = crate::providers::Permissions {
            run: run.clone(),
            decisions,
            publish: |request| {
                self.access(|db| db.request_permission(thread, request))?;
                self.changed.send_replace(());
                Ok(())
            },
        };
        let result = match turn.provider {
            Provider::Codex => {
                codex::execute(
                    self.options.codex.clone(),
                    turn,
                    stopped,
                    update,
                    permissions,
                )
                .await
            }
            Provider::ClaudeAgent => {
                claude_agent::execute(
                    &self.options.claude_agent,
                    turn,
                    stopped,
                    update,
                    permissions,
                )
                .await
            }
        };
        self.decisions.lock().unwrap().remove(run);
        result
    }
}

/// The owner of a state directory. Share this handle via Arc inside the host.
pub struct Huggler {
    shared: Arc<Shared>,
    closed: watch::Sender<bool>,
}

impl Huggler {
    /// Requires a Tokio runtime with I/O and time enabled.
    pub async fn open(options: HugglerOptions) -> Result<Self> {
        let dir = options.state_dir.clone();
        let mut db = tokio::task::spawn_blocking(move || Database::open(&dir))
            .await
            .map_err(|e| Error::Worker(e.to_string()))??;
        let listener = if options.agent_tools {
            Some(tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?)
        } else {
            None
        };
        let endpoint = listener
            .as_ref()
            .map(|l| l.local_addr().map(|addr| format!("http://{addr}/mcp")))
            .transpose()?;
        let jobs = db.ready_jobs()?;
        let shared = Arc::new(Shared {
            db: Mutex::new(Some(db)),
            decisions: Mutex::default(),
            run_stops: Mutex::default(),
            fault: Mutex::default(),
            changed: watch::channel(()).0,
            stop: watch::channel(false).0,
            options,
            endpoint,
            handle: Handle::current(),
            workers: Mutex::new(Some(JoinSet::new())),
        });
        let runtime = Self {
            shared,
            closed: watch::channel(false).0,
        };
        if let Some(listener) = listener {
            let shared = &runtime.shared;
            shared
                .workers
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .spawn(crate::mcp::serve(
                    listener,
                    shared.clone(),
                    shared.stop.subscribe(),
                ));
        }
        {
            let shared = &runtime.shared;
            let mut gate = shared.workers.lock().unwrap();
            for (thread, run) in jobs {
                shared.spawn(gate.as_mut().unwrap(), thread, run);
            }
        }
        Ok(runtime)
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
        self.shared.schedule(|db| {
            let (receipt, fresh) = db.dispatch(&command)?;
            let job = receipt
                .run_id
                .clone()
                .filter(|_| fresh)
                .map(|run| (command.thread_id, run));
            Ok((receipt, job))
        })
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
    /// Read the original delivery receipt, including after the sender settles.
    pub fn peer_receipt(&self, sender: &ThreadId, key: &str) -> Result<Option<PeerReceipt>> {
        self.shared.access(|db| db.peer_receipt(sender, key))
    }

    /// Withdraw a live run's ability to call agent tools.
    pub fn revoke_agent_tools(&self, thread: &ThreadId, run: &RunId) -> Result<bool> {
        self.shared.access(|db| db.revoke(thread, run))
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
        let mut gate = self.shared.workers.lock().unwrap();
        let Some(mut workers) = gate.take() else {
            return;
        };
        {
            let _db = self.shared.db.lock().unwrap();
            self.shared.stop.send_replace(true);
            for stop in self.shared.run_stops.lock().unwrap().values() {
                stop.send_replace(true);
            }
        }
        self.shared.changed.send_replace(());
        let shared = self.shared.clone();
        let closed = self.closed.clone();
        self.shared.handle.spawn(async move {
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
