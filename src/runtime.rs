mod questions;
#[cfg(feature = "application")]
mod workspace_setup;
use crate::{
    contracts::{
        agent_commands::{CreateThreads, CreatedPeer, Delegation},
        *,
    },
    providers::{
        ChildInterrupt, Control,
        claude_agent::{self, ClaudeAgentOptions},
        codex::{self, CodexOptions},
    },
    store::{Database, RunJob},
};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
    task::JoinSet,
};

#[derive(Clone, Debug)]
pub struct HugglerOptions {
    /// Base directory for relative storage paths.
    pub state_dir: PathBuf,
    pub storage: crate::StorageLayout,
    pub output: crate::AgentOutput,
    /// Enable caller-scoped orchestration tools over loopback HTTP.
    pub agent_tools: bool,
    pub agent_policy: crate::application::AgentPolicy,
    /// Continue an idle independent Codex thread after a provider-reported quota reset.
    pub auto_resume_usage_limits: bool,
    /// Ask Codex for an optional follow-up after eligible independent conversations.
    pub prompt_suggestions: bool,
    pub instances: Vec<crate::providers::registry::ProviderInstance>,
    pub codex: CodexOptions,
    pub claude_agent: ClaudeAgentOptions,
}
impl HugglerOptions {
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            storage: Default::default(),
            output: Default::default(),
            agent_tools: false,
            agent_policy: Default::default(),
            auto_resume_usage_limits: false,
            prompt_suggestions: false,
            instances: vec![],
            codex: CodexOptions::default(),
            claude_agent: ClaudeAgentOptions::default(),
        }
    }
}

pub(crate) struct Shared {
    pub(crate) output: Arc<crate::AgentOutput>,
    pub(crate) tool_catalog: crate::mcp::ToolCatalog,
    #[cfg(feature = "application")]
    pub(crate) services: Mutex<crate::services::Registry>,
    pub(crate) agent_policy: crate::application::AgentPolicy,
    prompt_suggestions: bool,
    registry: crate::providers::registry::Registry,
    endpoint: Option<String>,
    handle: Handle,
    workers: Mutex<Option<JoinSet<()>>>,
    db: Mutex<Option<Database>>,
    run_stops: Mutex<HashMap<RunId, watch::Sender<bool>>>,
    controls: Mutex<HashMap<(ThreadId, RunId), mpsc::UnboundedSender<Control>>>,
    decisions: Mutex<HashMap<RunId, mpsc::UnboundedSender<(RequestId, String)>>>,
    fault: Mutex<Option<String>>,
    changed: watch::Sender<()>,
    limit_reset: tokio::sync::Notify,
    stop: watch::Sender<bool>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskWaitResult {
    #[serde(flatten)]
    task: DelegatedTask,
    awaiting_approval: bool,
    timed_out: bool,
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
    pub(crate) fn schedule<T, Jobs: IntoIterator<Item = RunJob>>(
        self: &Arc<Self>,
        write: impl FnOnce(&mut Database) -> Result<(T, Jobs)>,
    ) -> Result<T> {
        let mut gate = self.workers.lock().unwrap();
        let workers = gate.as_mut().ok_or(Error::Closed)?;
        while let Some(completed) = workers.try_join_next() {
            if let Err(e) = completed {
                return Err(Error::Worker(e.to_string()));
            }
        }
        let (value, jobs) = self.access(write)?;
        self.changed.send_replace(());
        for (thread, run) in jobs {
            self.spawn(workers, thread, run);
        }
        Ok(value)
    }

    pub(crate) fn create_threads(
        self: &Arc<Self>,
        token: &str,
        input: CreateThreads,
    ) -> Result<Vec<CreatedPeer>> {
        self.schedule(|db| db.create_threads(token, input))
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
        input: Delegation,
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

    pub(crate) async fn wait_for_task(
        &self,
        token: &str,
        id: &TaskId,
        limit: std::time::Duration,
    ) -> Result<TaskWaitResult> {
        let mut changes = self.changed.subscribe();
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let (task, awaiting_approval) = self.access(|db| {
                let task = db.task_status(token, id)?;
                let approval = !task.status.terminal() && db.task_awaits_approval(id)?;
                Ok((task, approval))
            })?;
            if task.status.terminal() {
                self.changed.send_replace(());
            }
            let ready = task.status.terminal() || awaiting_approval;
            if ready || tokio::time::Instant::now() >= deadline {
                return Ok(TaskWaitResult {
                    task,
                    awaiting_approval,
                    timed_out: !ready,
                });
            }
            // Recheck ownership and the latest committed result even on a timeout.
            if let Ok(closed) = tokio::time::timeout_at(deadline, changes.changed()).await {
                closed.map_err(|_| Error::Closed)?;
            }
        }
    }

    pub(crate) fn cancel_task(
        self: &Arc<Self>,
        token: &str,
        id: &TaskId,
        key: Option<&str>,
    ) -> Result<DelegatedTask> {
        self.schedule(|db| {
            let (task, runs) = db.cancel_task(token, id, key)?;
            self.stop_runs(&runs);
            Ok((task, None))
        })
    }

    fn stop_runs(&self, runs: &[RunId]) {
        let stops = self.run_stops.lock().unwrap();
        for id in runs {
            if let Some(stop) = stops.get(id) {
                stop.send_replace(true);
            }
        }
    }

    pub(crate) fn interrupt_run(
        self: &Arc<Self>,
        token: &str,
        thread: &ThreadId,
        run: &RunId,
        key: &str,
        reason: Option<&str>,
    ) -> Result<Option<RunStatus>> {
        self.schedule(|db| {
            let terminal = db.interrupt_run(token, thread, run, key, reason)?;
            if terminal.is_none()
                && let Some(stop) = self.run_stops.lock().unwrap().get(run)
            {
                stop.send_replace(true);
            }
            Ok((terminal, None))
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
        self.run_stops.lock().unwrap().remove(&run);
        let limited = matches!(result, Err(Error::UsageLimit { .. }));
        if let Err(error) = self.advance(|db| db.finish(&thread, &run, result)) {
            *self.fault.lock().unwrap() = Some(error.to_string());
            self.changed.send_replace(());
        } else if limited {
            self.limit_reset.notify_one();
        }
    }

    async fn recover_limits(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        loop {
            let delay = match self.schedule(Database::resume_limited) {
                Ok(seconds) => seconds,
                Err(Error::Closed) => return,
                Err(error) => {
                    *self.fault.lock().unwrap() = Some(error.to_string());
                    self.changed.send_replace(());
                    return;
                }
            };
            tokio::select! {
                _ = stop.wait_for(|stopped| *stopped) => return,
                _ = self.limit_reset.notified() => (),
                // Recheck wall time periodically while a recovery is armed.
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay.unwrap_or(0).min(3600))), if delay.is_some() => (),
            }
        }
    }

    fn advance(self: &Arc<Self>, finish: impl FnOnce(&mut Database) -> Result<()>) -> Result<()> {
        let mut gate = self.workers.lock().unwrap();
        let mut guard = self.db.lock().unwrap();
        let db = guard
            .as_mut()
            .expect("close joins workers before releasing storage");
        finish(db)?;
        if let Some(workers) = gate.as_mut() {
            for (thread, run) in db.ready_jobs()? {
                self.spawn(workers, thread, run);
            }
        }
        self.changed.send_replace(());
        Ok(())
    }

    async fn execute(
        self: &Arc<Self>,
        thread: &ThreadId,
        run: &RunId,
        stopped: watch::Receiver<bool>,
    ) -> Result<()> {
        if let Some(worktree) = self.access(|db| db.worktree(thread))? {
            let worktree = tokio::task::spawn_blocking(move || {
                worktree.prepare()?;
                Ok::<_, Error>(worktree)
            })
            .await
            .map_err(|e| Error::Worker(e.to_string()))??;
            if !worktree.ready {
                self.access(|db| db.worktree_ready(&worktree.path))?;
            }
        }
        let turn = self.access(|db| {
            let mut turn = db.claim(run)?;
            turn.sample_suggestion = self.prompt_suggestions
                && turn.provider == Provider::Codex
                && db.can_sample_suggestion(run)?;
            if let Some(url) = &self.endpoint
                && db.permits_agent_tools(run)?
            {
                turn.agent_tools = Some(crate::providers::AgentTools {
                    url: url.clone(),
                    token: db.grant(run)?,
                });
            }
            Ok(turn)
        })?;
        let update = |change| {
            if matches!(change, crate::providers::Update::ForegroundComplete) {
                return self.advance(|db| db.update(thread, run, change));
            }
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
        let (tx, controls) = mpsc::unbounded_channel();
        let owner = (thread.clone(), run.clone());
        self.controls.lock().unwrap().insert(owner.clone(), tx);
        let instance = self
            .registry
            .get(turn.instance.as_str())
            .ok_or_else(|| Error::NotFound(format!("provider instance {}", turn.instance)))?;
        let result = match turn.provider {
            Provider::Codex => {
                codex::execute(instance, turn, stopped, update, permissions, controls).await
            }
            Provider::Pi => crate::providers::pi::execute(instance, turn, stopped, update).await,
            Provider::Muse => {
                crate::providers::muse::execute(instance, turn, stopped, update).await
            }
            Provider::OpenCode => {
                crate::providers::opencode::execute(instance, turn, stopped, update).await
            }
            _ => {
                claude_agent::execute(instance, turn, stopped, update, permissions, controls).await
            }
        };
        self.controls.lock().unwrap().remove(&owner);
        self.decisions.lock().unwrap().remove(run);
        result
    }
}

/// The owner of a Huggler database. Share this handle via Arc inside the host.
pub struct Huggler {
    pub(crate) shared: Arc<Shared>,
    closed: watch::Sender<bool>,
}

impl Huggler {
    /// Requires a Tokio runtime with I/O and time enabled.
    pub async fn open(options: HugglerOptions) -> Result<Self> {
        let dir = options.state_dir;
        let storage = options.storage;
        let registry = crate::providers::registry::registry(
            &options.codex,
            &options.claude_agent,
            &options.instances,
        )?;
        let tool_catalog = crate::mcp::catalog(&options.output)?;
        let output = Arc::new(options.output);
        let database_output = output.clone();
        let configured = registry.clone();
        let mut db = tokio::task::spawn_blocking(move || {
            Database::open_with_registry(&dir, storage, database_output, &configured)
        })
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
            #[cfg(feature = "application")]
            services: Mutex::default(),
            agent_policy: options.agent_policy,
            output,
            tool_catalog,
            prompt_suggestions: options.prompt_suggestions,
            db: Mutex::new(Some(db)),
            decisions: Mutex::default(),
            run_stops: Mutex::default(),
            controls: Mutex::default(),
            fault: Mutex::default(),
            changed: watch::channel(()).0,
            limit_reset: tokio::sync::Notify::new(),
            stop: watch::channel(false).0,
            registry,
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
            if options.auto_resume_usage_limits {
                gate.as_mut()
                    .unwrap()
                    .spawn(shared.clone().recover_limits(shared.stop.subscribe()));
            }
            for (thread, run) in jobs {
                shared.spawn(gate.as_mut().unwrap(), thread, run);
            }
        }
        Ok(runtime)
    }

    /// Resolve the enclosing Git checkout and store its absolute working directory.
    pub fn open_project(&self, path: impl AsRef<Path>) -> Result<Project> {
        let output = crate::worktree::command(path.as_ref())
            .args(["rev-parse", "--show-toplevel"])
            .output()?;
        if !output.status.success() {
            return Err(Error::Invalid("expected a Git working tree".into()));
        }
        let path = std::str::from_utf8(&output.stdout)
            .map_err(|_| Error::Invalid("workspace path requires UTF-8".into()))?;
        let root = Path::new(path.strip_suffix('\n').unwrap_or(path)).canonicalize()?;
        self.shared.access(|db| db.project(&root))
    }

    /// Copy a selected transcript or load an ACP Registry session into a new thread.
    /// Repeating an identical snapshot at the same destination preserves its history.
    pub async fn import_history(
        &self,
        project: &ProjectId,
        thread: &ThreadId,
        name: &str,
        instance: &ProviderInstanceId,
        source: ImportSource,
    ) -> Result<ImportProvenance> {
        let cwd = self.shared.access(|db| db.project_root(project))?;
        let provider = self
            .shared
            .registry
            .get(instance.as_str())
            .ok_or_else(|| Error::NotFound(instance.to_string()))?;
        let snapshot = match source {
            ImportSource::AcpRegistry(native) => {
                claude_agent::load_history(provider, &self.shared.output, &cwd, native).await?
            }
            file => tokio::task::spawn_blocking(move || crate::imports::read_file(file))
                .await
                .map_err(|e| Error::Worker(e.to_string()))??,
        };
        self.shared
            .schedule(|db| Ok((db.import(project, thread, name, instance, snapshot)?, [])))
    }

    pub fn import_source(&self, thread: &ThreadId) -> Result<Option<ImportProvenance>> {
        self.shared.access(|db| db.import_source(thread))
    }

    /// Commit a host command. Reusing the same command
    /// identity and payload returns the original receipt, including after reopen.
    pub fn dispatch(&self, command: Command) -> Result<Receipt> {
        self.shared.schedule(|db| {
            let (receipt, work) = db.dispatch(&command)?;
            self.shared.stop_runs(&work.stop);
            if let Some((run, text)) = work.steer {
                let sent = self
                    .shared
                    .controls
                    .lock()
                    .unwrap()
                    .get(&(command.thread_id.clone(), run.clone()))
                    .is_some_and(|tx| {
                        tx.send(Control::Steer {
                            command: command.id.clone(),
                            text,
                        })
                        .is_ok()
                    });
                if !sent {
                    db.update(
                        &command.thread_id,
                        &run,
                        crate::providers::Update::Steer(
                            command.id.clone(),
                            SteerOutcome::Rejected(
                                "foreground session closed before delivery".into(),
                            ),
                        ),
                    )?;
                }
            }
            Ok((receipt, work.start))
        })
    }

    /// Read the durable acknowledgement of a steering command, including after reopen.
    pub fn steer_outcome(&self, thread: &ThreadId, command: &CommandId) -> Result<SteerOutcome> {
        self.shared.access(|db| db.steer_outcome(thread, command))
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
    /// Request interruption of current work on the thread and its owned descendants.
    /// Observe worker outcomes with `wait` or `subscribe`.
    pub fn stop_thread(&self, id: CommandId, thread: ThreadId) -> Result<Receipt> {
        self.dispatch(Command {
            id,
            thread_id: thread,
            action: Action::StopThread,
        })
    }

    /// Release the held queue on this thread. Descendants have their own queues.
    pub fn resume_queue(&self, id: CommandId, thread: ThreadId) -> Result<Receipt> {
        self.dispatch(Command {
            id,
            thread_id: thread,
            action: Action::ResumeQueue,
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

    /// Interrupt a live native child in its owning run. Supply the exact turn for
    /// Codex and `None` for ACP session cancellation. See [`NativeChild::can_interrupt`].
    /// Observe the child for its outcome after the adapter accepts the request (Codex)
    /// or the notification is written (ACP).
    pub async fn interrupt_native_child<'a>(
        &self,
        thread: &ThreadId,
        run: &RunId,
        child: &NativeThreadId,
        turn: impl Into<Option<&'a NativeTurnId>>,
    ) -> Result<()> {
        let (reply, response) = tokio::sync::oneshot::channel();
        self.shared
            .controls
            .lock()
            .unwrap()
            .get(&(thread.clone(), run.clone()))
            .ok_or_else(|| Error::NotFound("live native child session".into()))?
            .send(Control::Interrupt(ChildInterrupt {
                thread: child.clone(),
                turn: turn.into().cloned(),
                reply,
            }))
            .map_err(|_| Error::Closed)?;
        response.await.map_err(|_| Error::Closed)?
    }

    /// Withdraw a live run's ability to call agent tools.
    pub fn revoke_agent_tools(&self, thread: &ThreadId, run: &RunId) -> Result<bool> {
        let revoked = self.shared.access(|db| db.revoke(thread, run))?;
        if revoked {
            self.shared.changed.send_replace(());
        }
        Ok(revoked)
    }

    /// Effective host-selected presentation, including provider display labels.
    pub fn output(&self) -> &crate::AgentOutput {
        &self.shared.output
    }

    /// Enumerate configured instances in stable ID order without starting a provider.
    pub fn provider_instances(&self) -> impl Iterator<Item = &crate::ProviderInstance> {
        self.shared.registry.values()
    }

    /// Check a single configured installation in the supplied working directory. ACP may
    /// create an empty native session to discover its model catalog and detect a login challenge.
    pub async fn probe_provider(
        &self,
        id: &str,
        cwd: impl AsRef<Path>,
    ) -> Result<crate::ProviderReport> {
        self.discover_provider(id, cwd.as_ref(), async |instance, output, cwd| {
            crate::providers::probe::inspect(&instance, &output, &cwd).await
        })
        .await
    }

    /// List enabled skills in a working directory, ordered by name and path. None
    /// indicates an unsupported adapter or provider version. Discovery does not submit a prompt.
    pub async fn list_provider_skills(
        &self,
        id: &str,
        cwd: impl AsRef<Path>,
    ) -> Result<Option<Vec<crate::ProviderSkill>>> {
        self.discover_provider(id, cwd.as_ref(), async |instance, output, cwd| {
            crate::providers::probe::list_skills(&instance, &output, &cwd).await
        })
        .await
    }

    async fn discover_provider<T: Send + 'static, F: Future<Output = Result<T>> + Send>(
        &self,
        id: &str,
        cwd: &Path,
        discover: impl FnOnce(crate::ProviderInstance, Arc<crate::AgentOutput>, PathBuf) -> F
        + Send
        + 'static,
    ) -> Result<T> {
        let instance = self
            .shared
            .registry
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("provider instance {id}")))?
            .clone();
        let cwd = cwd.canonicalize().map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
                Error::CheckoutUnavailable(cwd.to_owned())
            }
            _ => Error::Io(error),
        })?;
        if !cwd.is_dir() {
            return Err(Error::CheckoutUnavailable(cwd));
        }
        let output = self.shared.output.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut gate = self.shared.workers.lock().unwrap();
            let workers = gate.as_mut().ok_or(Error::Closed)?;
            let mut stop = self.shared.stop.subscribe();
            workers.spawn_on(
                async move {
                    let provider_id = instance.id.clone();
                    let probe = discover(instance, output, cwd);
                    let deadline = std::time::Duration::from_secs(15);
                    let result = tokio::select! {
                        _ = stop.wait_for(|stopped| *stopped) => Err(Error::Closed),
                        result = tokio::time::timeout(deadline, probe) => result.unwrap_or_else(|_| {
                            Err(Error::Provider(format!(
                                "{} timed out during discovery. Check its executable, server and login",
                                provider_id
                            )))
                        }),
                    };
                    let _ = tx.send(result);
                },
                &self.shared.handle,
            );
        }
        rx.await.map_err(|_| Error::Closed)?
    }

    /// Totals for application runs with a provider usage report, including archived instance IDs.
    pub fn provider_usage(&self) -> Result<Vec<ProviderUsage>> {
        self.shared.access(|db| db.usage())
    }

    pub fn threads(&self) -> Result<Vec<ThreadSummary>> {
        self.shared.access(|db| db.threads_after(0))
    }

    /// Read current activity across conversations without loading messages or transcripts.
    pub fn conversation_list(&self) -> Result<ConversationList> {
        self.shared.access(|db| db.conversation_list())
    }

    /// Wait until the list may have changed. Pass the revision from the previous snapshot.
    pub async fn wait_for_conversation_list(&self, revision: i64) -> Result<ConversationList> {
        let mut changed = self.shared.changed.subscribe();
        loop {
            let snapshot = self.shared.access(|db| {
                (db.conversation_revision()? != revision)
                    .then(|| db.conversation_list())
                    .transpose()
            })?;
            if let Some(snapshot) = snapshot {
                return Ok(snapshot);
            }
            changed.changed().await.map_err(|_| Error::Closed)?;
        }
    }

    /// Wait for conversations created after the last thread seen in creation order.
    /// The last seen ID remains a valid cursor after deletion. Pass `None` for an empty list.
    /// An unknown thread ID returns `Error::NotFound`.
    pub async fn wait_for_threads_after(
        &self,
        last: Option<&ThreadId>,
    ) -> Result<Vec<ThreadSummary>> {
        let mut changed = self.shared.changed.subscribe();
        let position = match last {
            Some(id) => self.shared.access(|db| db.thread_cursor_position(id))?,
            None => 0,
        };
        loop {
            let threads = self.shared.access(|db| db.threads_after(position))?;
            if !threads.is_empty() {
                return Ok(threads);
            }
            changed.changed().await.map_err(|_| Error::Closed)?;
        }
    }

    pub fn read(&self, id: &ThreadId) -> Result<Thread> {
        self.shared.access(|db| db.read(id))
    }

    /// Read up to 100 message previews, applying an optional ASCII case-insensitive literal search.
    /// A change to the conversation invalidates its history cursor.
    pub fn detail(
        &self,
        thread: &ThreadId,
        before: Option<&HistoryCursor>,
        query: &str,
        limit: usize,
    ) -> Result<ConversationDetail> {
        self.shared
            .access(|db| db.detail(thread, before, query, limit))
    }

    /// Codex command item IDs observed on one application run, in discovery order.
    pub fn command_items(&self, thread: &ThreadId, run: &RunId) -> Result<Vec<String>> {
        self.shared.access(|db| db.command_items(thread, run))
    }

    /// Read one bounded command tail. Pass its revision to wait for newer output or closure.
    pub async fn command_output(
        &self,
        thread: &ThreadId,
        run: &RunId,
        item: &str,
        after: Option<i64>,
    ) -> Result<CommandOutput> {
        let mut changed = self.shared.changed.subscribe();
        loop {
            let output = self
                .shared
                .access(|db| db.command_output(thread, run, item))?;
            if after.is_some_and(|r| r < 0 || r > output.revision) {
                return Err(Error::Invalid("cursor outside command output".into()));
            }
            if after != Some(output.revision) || output.closed {
                return Ok(output);
            }
            changed.changed().await.map_err(|_| Error::Closed)?;
        }
    }

    /// Stage up to 64 KiB of UTF-8 plain text for one thread.
    pub fn prepare_upload(
        &self,
        thread: &ThreadId,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<UploadId> {
        self.shared
            .access(|db| db.prepare_upload(thread, name, mime, bytes))
    }
    /// Discard an unclaimed upload. Repeated cleanup is safe.
    pub fn discard_upload(&self, thread: &ThreadId, id: &UploadId) -> Result<()> {
        self.shared.access(|db| db.discard_upload(thread, id))
    }

    /// Unified Git diff between two explicit checkpoints on the same thread, up to 1 MiB.
    pub fn checkpoint_diff(&self, thread: &ThreadId, from: &RunId, to: &RunId) -> Result<String> {
        let (repository, base, head) = self
            .shared
            .access(|db| db.checkpoint_pair(thread, from, to))?;
        crate::worktree::diff(Path::new(&repository), &base, &head)
    }

    /// Read the immutable bounded prefix captured at fork creation.
    pub fn fork_history(&self, thread: &ThreadId) -> Result<Option<ForkHistory>> {
        self.shared.access(|db| db.fork_history(thread))
    }

    /// Expand the original message body, excluding separately attached files.
    pub fn read_message_body(&self, thread: &ThreadId, id: &MessageId) -> Result<String> {
        self.shared.access(|db| db.read_message_body(thread, id))
    }

    /// Read one captured text file by its zero-based position in history.
    pub fn read_attachment(
        &self,
        thread: &ThreadId,
        message: &MessageId,
        position: usize,
    ) -> Result<String> {
        self.shared
            .access(|db| db.read_attachment(thread, message, position))
    }

    /// Expand a message preview to its complete stored text.
    pub fn read_message(&self, thread: &ThreadId, id: &MessageId) -> Result<Message> {
        self.shared.access(|db| db.read_message(thread, id))
    }

    /// Keep the most recent `count` replay events. Receipts and projections outlive the replay window.
    pub fn retain_events(&self, thread: &ThreadId, count: u32) -> Result<()> {
        self.shared.access(|db| db.retain_events(thread, count))
    }

    /// Remove an idle thread and its local history. Foreign keys protect cross-thread
    /// references (including delegated tasks and fork sources). Git files stay on disk.
    #[cfg(feature = "application")]
    pub fn delete_thread(&self, thread: &ThreadId) -> Result<()> {
        let mut services = self.shared.services.lock().unwrap();
        self.shared
            .schedule(|db| Ok((db.delete_thread(thread)?, [])))?;
        services.close_thread(thread);
        Ok(())
    }

    /// Coalesced notification for snapshot consumers. Returns the current high-water mark.
    pub async fn wait_for_change(&self, thread: &ThreadId, after: i64) -> Result<i64> {
        let mut changed = self.shared.changed.subscribe();
        loop {
            let cursor = self.shared.access(|db| db.sequence(thread))?;
            if after < 0 || after > cursor {
                return Err(Error::Invalid("cursor outside thread snapshot".into()));
            }
            if cursor > after {
                return Ok(cursor);
            }
            changed.changed().await.map_err(|_| Error::Closed)?;
        }
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
        let snapshot = self.shared.access(|db| {
            if let Some(cursor) = after
                && cursor >= 0
            {
                db.check_replay(id, cursor)?;
            }
            db.read(id)
        })?;
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
            #[cfg(feature = "application")]
            shared.services.lock().unwrap().clear();
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
