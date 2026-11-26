mod agent_activity;
mod attachments;
mod backlog;
mod document_tabs;
mod file_tree;
mod launch_workspace;
pub(crate) mod markdown_view;
mod mcp_status;
mod models;
mod project_actions;
mod projects;
mod sidebar;
mod suggestions;
#[cfg(target_os = "macos")]
mod terminal;
#[cfg(target_os = "macos")]
mod terminal_actions;
#[cfg(target_os = "macos")]
mod terminal_pane;
mod traces;
mod transcript;

use attachments::Attachments;
use backlog::{Backlog, LaunchDraft};
pub(crate) use document_tabs::DOCUMENT_CONTEXT;
use document_tabs::{Document, DocumentTabs};
use file_tree::{FileAction, FileTree};
use gpui_component::list::ListState;
use launch_workspace::{LaunchWorkspace, WorkspaceMode};
use models::Models;
pub(crate) use models::ProviderCatalog;
use transcript::TurnClock;
#[cfg(unix)]
pub(crate) use transcript::permission_card;

use self::launch_workspace::{LaunchProgress, WorktreeStage};
use crate::{
    changes::{Changes, RevertFinished, ReviewRow},
    git,
    paths::StoragePaths,
    questions::Questions,
    settings::{FollowUp, Preferences, SendShortcut, Settings, SettingsEvent, SettingsTarget},
    ui::{field, heading, hint, label},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use huggler::{
    Action, Command, CommandId, ConversationDetail, ConversationSummary, HistoryCursor, Huggler,
    ModelOptions, Provider, ProviderInstanceId, RunId, SteerOutcome, ThreadId,
};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::Arc,
};
use uuid::Uuid;

const NARROW_VIEWPORT_WIDTH: Pixels = px(900.);

pub(crate) const COMPOSER_CONTEXT: &str = "HugglerComposer";

fn history_at_bottom(scroll: &ScrollHandle) -> bool {
    scroll.max_offset().height + scroll.offset().y <= px(2.)
}

struct Workspace {
    root: PathBuf,
    files: Entity<FileTree>,
    changes: Entity<Changes>,
    documents: DocumentTabs,
    traces: traces::Traces,
    draft: Entity<InputState>,
    attachments: Attachments,
    questions: HashMap<huggler::RequestId, Entity<Questions>>,
    #[cfg(target_os = "macos")]
    terminal: terminal_pane::TerminalPane,
}

impl Workspace {
    fn refresh_after_checkout_update(&self, tab: WorkspaceTab, cx: &mut App) {
        WorkspaceTab::Changes.refresh(self, cx);
        if tab == WorkspaceTab::AllFiles {
            tab.refresh(self, cx);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkspaceTab {
    AllFiles,
    Changes,
    Traces,
}

impl WorkspaceTab {
    fn label(self) -> &'static str {
        match self {
            Self::AllFiles => "All files",
            Self::Changes => "Changes",
            Self::Traces => "Traces",
        }
    }

    fn refresh(self, workspace: &Workspace, cx: &mut App) {
        match self {
            Self::AllFiles => workspace.files.update(cx, |tree, cx| tree.refresh(cx)),
            Self::Changes => workspace.changes.update(cx, |panel, cx| panel.refresh(cx)),
            Self::Traces => {}
        }
    }
}

pub struct Desktop {
    settings: Entity<Settings>,
    show_settings: bool,
    backlog: Backlog,
    #[cfg(unix)]
    remote_project: Option<Entity<crate::machines::RemoteProject>>,
    #[cfg(unix)]
    remote_projects: HashMap<
        (huggler::EnvironmentId, huggler::ProjectId),
        Entity<crate::machines::RemoteProject>,
    >,
    navigation: sidebar::Sidebar,
    right_sidebar: sidebar::Sidebar,
    workspace_tab: WorkspaceTab,
    focus: FocusHandle,
    workspaces: HashMap<ThreadId, Workspace>,
    runtime: Arc<Huggler>,
    executor: tokio::runtime::Handle,
    selection_file: PathBuf,
    scratch_root: PathBuf,
    threads: Vec<ConversationSummary>,
    agent_activity: agent_activity::AgentActivity,
    expanded_sections: HashMap<sidebar::Section, bool>,
    current: Option<ConversationDetail>,
    pull_request: Option<crate::github::Status>,
    pull_request_watch: Option<Task<()>>,
    merged_pr_watch: Option<Task<()>>,
    pull_request_action: HashSet<ThreadId>,
    checkout_unavailable: bool,
    before: Option<HistoryCursor>,
    subscription: Option<Task<()>>,
    command_log: Option<CommandLog>,
    expanded_activity: HashSet<(RunId, Option<String>)>,
    turn_clock: Option<Entity<TurnClock>>,
    full_messages: HashMap<huggler::MessageId, String>,
    file_preview: Option<((huggler::MessageId, usize), attachments::FilePreview)>,
    thread_updates: Option<Task<()>>,
    history_scroll: ScrollHandle,
    checkout: Entity<InputState>,
    picked_checkout: Option<PathBuf>,
    project_picker: Option<Entity<ListState<projects::ProjectPicker>>>,
    checkout_browse: Option<Task<()>>,
    launch_workspace: LaunchWorkspace,
    branch: Entity<InputState>,
    launching: Option<LaunchProgress>,
    name: Entity<InputState>,
    starter: Entity<InputState>,
    starter_attachments: Attachments,
    launch_setup: bool,
    prompt: Entity<InputState>,
    suggestions: suggestions::Suggestions,
    instance: ProviderInstanceId,
    launch_model: Option<String>,
    launch_model_options: ModelOptions,
    model_picker: Option<Entity<ListState<Models>>>,
    show_model_efforts: bool,
    mcp_status: Option<mcp_status::McpStatus>,
    mcp_status_focus: FocusHandle,
    model_catalogs: HashMap<(ProviderInstanceId, PathBuf), Entity<ProviderCatalog>>,
    checkout_preload: Option<Task<()>>,
    error: Option<String>,
}

struct CommandLog {
    run: RunId,
    item: String,
    content: Option<huggler::Result<huggler::CommandOutput>>,
    _reader: Task<()>,
}

impl Desktop {
    pub fn new(
        runtime: Arc<Huggler>,
        executor: tokio::runtime::Handle,
        paths: StoragePaths,
        prefs: Preferences,
        load_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = paths.state_dir();
        let settings = cx.new(|cx| {
            Settings::new(
                prefs,
                state.clone(),
                runtime.clone(),
                load_error,
                executor.clone(),
                window,
                cx,
            )
        });
        let catalog_fallback = state.clone();
        #[cfg(target_os = "macos")]
        cx.on_app_quit(|view, cx| {
            for workspace in view.workspaces.values_mut() {
                workspace.terminal.release(cx);
            }
            std::future::ready(())
        })
        .detach();
        cx.observe(&settings, |view, settings, cx| {
            view.agent_activity
                .update_preferences(settings.read(cx).preferences(), &view.threads);
            cx.notify();
        })
        .detach();
        settings.update(cx, |settings, cx| settings.refresh_provider_setup(cx));
        cx.subscribe_in(
            &settings,
            window,
            move |view: &mut Self, _, event, window, cx| {
                match event {
                    SettingsEvent::Back => {
                        if view.leave_settings(cx) {
                            view.focus_workspace(window, cx);
                        }
                    }
                    #[cfg(unix)]
                    SettingsEvent::OpenMachine(machine) => {
                        view.open_machine(machine.clone(), window, cx)
                    }
                    #[cfg(unix)]
                    SettingsEvent::ForgetMachine(machine) => {
                        let key = (machine.environment.clone(), machine.project.clone());
                        if let Some(removed) = view.remote_projects.remove(&key)
                            && view.remote_project.as_ref() == Some(&removed)
                        {
                            view.remote_project = None;
                        }
                    }
                    SettingsEvent::ProviderModels(id, refresh) => {
                        let (_, checkout) = view.model_source(cx);
                        let directory = if checkout.is_dir() {
                            checkout
                        } else {
                            catalog_fallback.clone()
                        };
                        let catalog = view.catalog_for((id.clone(), directory), *refresh, cx);
                        view.settings.update(cx, |settings, cx| {
                            settings.set_model_catalog(id, catalog, cx)
                        });
                    }
                }
                cx.notify();
            },
        )
        .detach();
        let mut view = Self {
            settings,
            show_settings: false,
            backlog: Backlog::default(),
            #[cfg(unix)]
            remote_project: None,
            #[cfg(unix)]
            remote_projects: HashMap::new(),
            navigation: sidebar::Sidebar::new(288.),
            right_sidebar: sidebar::Sidebar::new(336.),
            workspace_tab: WorkspaceTab::Changes,
            focus: cx.focus_handle(),
            workspaces: HashMap::new(),
            runtime,
            executor,
            scratch_root: paths.scratch_dir(),
            selection_file: state.join("selected-thread"),
            threads: Vec::new(),
            agent_activity: agent_activity::AgentActivity::default(),
            expanded_sections: HashMap::new(),
            current: None,
            pull_request: None,
            pull_request_watch: None,
            merged_pr_watch: None,
            pull_request_action: HashSet::new(),
            checkout_unavailable: false,
            before: None,
            subscription: None,
            command_log: None,
            expanded_activity: HashSet::new(),
            turn_clock: None,
            full_messages: HashMap::new(),
            file_preview: None,
            thread_updates: None,
            history_scroll: ScrollHandle::new(),
            checkout: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(
                        std::env::current_dir()
                            .ok()
                            .filter(|root| root.join(".git").exists())
                            .map(|root| root.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    )
                    .placeholder("/path/to/project")
            }),
            picked_checkout: None,
            project_picker: None,
            checkout_browse: None,
            branch: cx
                .new(|cx| InputState::new(window, cx).placeholder("Generated from the title")),
            launch_workspace: LaunchWorkspace::default(),
            launching: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Name this conversation…")),
            launch_setup: false,
            starter_attachments: Attachments::default(),
            starter: cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(2, 7)
                    .placeholder("Describe a task or ask a question…")
            }),
            prompt: cx.new(|cx| InputState::new(window, cx).auto_grow(2, 5)),
            suggestions: suggestions::Suggestions::default(),
            instance: Provider::Codex.key().into(),
            launch_model: None,
            launch_model_options: ModelOptions::default(),
            model_picker: None,
            show_model_efforts: false,
            mcp_status: None,
            mcp_status_focus: cx.focus_handle(),
            model_catalogs: HashMap::new(),
            checkout_preload: None,
            error: None,
        };
        cx.subscribe(&view.checkout, |view, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                if view.picked_checkout.as_ref().is_some_and(|path| {
                    path.as_os_str() != std::ffi::OsStr::new(input.read(cx).value().as_str())
                }) {
                    view.picked_checkout = None;
                }
                if view.current.is_none() {
                    view.suggestions = Default::default();
                }
                view.launch_model = None;
                view.launch_model_options = ModelOptions::default();
                view.model_picker = None;
                view.mcp_status = None;
                view.launch_workspace = LaunchWorkspace::default();
                view.load_launch_branches(true, cx);
                let delay = cx
                    .background_executor()
                    .timer(std::time::Duration::from_millis(300));
                view.checkout_preload = Some(cx.spawn(async move |view, cx| {
                    delay.await;
                    let _ = view.update(cx, |view, cx| view.preload_models(cx));
                }));
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&view.name, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        Self::observe_composer(&view.starter, window, cx);
        cx.observe_window_activation(window, |view, window, cx| {
            if window.is_window_active() {
                view.watch_pull_request(cx);
            }
        })
        .detach();
        let restored = view.restore(window, cx);
        view.watch_merged_prs(cx);
        view.agent_activity
            .update_preferences(view.settings.read(cx).preferences(), &view.threads);
        if let Ok(mut revision) = restored {
            let runtime = view.runtime.clone();
            view.thread_updates = Some(cx.spawn_in(window, async move |view, cx| {
                loop {
                    let result = runtime.wait_for_conversation_list(revision).await;
                    let done = result.is_err();
                    if view
                        .update_in(cx, |view, window, cx| match result {
                            Ok(list) => {
                                revision = list.revision;
                                view.agent_activity.update_threads(
                                    &view.threads,
                                    &list.threads,
                                    window,
                                    cx,
                                );
                                view.threads = list.threads;
                                cx.notify();
                            }
                            Err(error) => view.report(Err(error), cx),
                        })
                        .is_err()
                        || done
                    {
                        break;
                    }
                }
            }));
        }
        view.report(restored.map(|_| ()), cx);
        view
    }

    fn report(&mut self, result: huggler::Result<()>, cx: &mut Context<Self>) {
        self.error = result.err().map(|error| error.to_string());
        cx.notify();
    }

    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) -> huggler::Result<i64> {
        let list = self.runtime.conversation_list()?;
        self.threads = list.threads;
        let saved = fs::read_to_string(&self.selection_file).ok();
        let selected = self
            .threads
            .iter()
            .find(|t| Some(&t.id.0) == saved.as_ref())
            .or_else(|| self.threads.first());
        if let Some(thread) = selected {
            self.select(thread.id.clone(), window, cx)?;
        } else {
            self.starter.focus_handle(cx).focus(window);
            self.load_launch_branches(false, cx);
            self.preload_models(cx);
        }
        Ok(list.revision)
    }

    #[cfg(unix)]
    fn open_machine(
        &mut self,
        machine: crate::machines::SavedMachine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_settings = false;
        self.backlog.open = false;
        self.error = None;
        let key = (machine.environment.clone(), machine.project.clone());
        let view = self.remote_projects.entry(key).or_insert_with(|| {
            cx.new(|cx| {
                crate::machines::RemoteProject::new(
                    machine.clone(),
                    self.executor.clone(),
                    window,
                    cx,
                )
            })
        });
        view.update(cx, |view, cx| view.set_machine(machine, window, cx));
        self.remote_project = Some(view.clone());
        self.focus.focus(window);
        cx.notify();
    }

    fn local_workspace_visible(&self) -> bool {
        !self.show_settings && !self.backlog.open && !self.remote_open()
    }

    fn remote_open(&self) -> bool {
        #[cfg(unix)]
        return self.remote_project.is_some();
        #[cfg(not(unix))]
        false
    }

    fn select(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> huggler::Result<()> {
        self.launching = None;
        self.project_picker = None;
        self.checkout_browse = None;
        self.ensure_workspace(&id, window, cx)?;
        self.backlog.open = false;
        #[cfg(unix)]
        {
            self.remote_project = None;
        }
        self.mcp_status = None;
        self.workspaces[&id].refresh_after_checkout_update(self.workspace_tab, cx);
        self.workspaces
            .get_mut(&id)
            .expect("selected workspace")
            .documents
            .show_chat();
        if self.current.as_ref().is_some_and(|thread| thread.id == id) {
            self.focus_workspace(window, cx);
            self.checkout_unavailable = !self.workspaces[&id].root.is_dir();
            self.watch_pull_request(cx);
            return Ok(());
        }
        let snapshot = self.runtime.detail(&id, None, "", 32)?;
        let mut cursor = snapshot.history.sequence;
        self.file_preview = None;
        self.command_log = None;
        self.expanded_activity.clear();
        self.full_messages.clear();
        self.checkout_unavailable = !self.workspaces[&id].root.is_dir();
        self.prompt = self.workspaces[&id].draft.clone();
        self.suggestions = Default::default();
        self.model_picker = None;
        self.launch_workspace.close_picker();
        self.current = Some(snapshot);
        self.refresh_traces(None);
        self.pull_request = None;
        self.watch_pull_request(cx);
        self.preload_models(cx);
        self.before = None;
        self.focus_workspace(window, cx);
        self.history_scroll.scroll_to_bottom();
        let runtime = self.runtime.clone();
        let selected = id.clone();
        self.subscription = Some(cx.spawn(async move |view, cx| {
            loop {
                let change = runtime.wait_for_change(&selected, cursor).await;
                match view.update(cx, |view, cx| -> Option<i64> {
                    if view.current.as_ref()?.id != selected {
                        return None;
                    }
                    let old = view.current.as_ref()?;
                    let foreground = old.foreground_run.as_ref().map(|r| r.id.clone());
                    let latest = old.latest_run.as_ref().map(|r| (r.id.clone(), r.status));
                    let result = change.and_then(|_| view.refresh());
                    if let Err(error) = result {
                        view.report(Err(error), cx);
                        return None;
                    }
                    cx.notify();
                    let thread = view.current.as_ref()?;
                    let finished = foreground.is_some()
                        && foreground.as_ref() != thread.foreground_run.as_ref().map(|r| &r.id);
                    let latest_finished = thread.latest_run.as_ref().is_some_and(|r| {
                        r.status.terminal()
                            && latest
                                .as_ref()
                                .is_none_or(|(id, status)| id != &r.id || *status != r.status)
                    });
                    let sequence = thread.history.sequence;
                    if finished || latest_finished {
                        view.workspaces[&selected]
                            .refresh_after_checkout_update(view.workspace_tab, cx);
                        view.watch_pull_request(cx);
                    }
                    Some(sequence)
                }) {
                    Ok(Some(next)) => cursor = next,
                    _ => break,
                }
            }
        }));
        fs::write(&self.selection_file, id.0)?;
        Ok(())
    }

    fn ensure_workspace(
        &mut self,
        id: &ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> huggler::Result<()> {
        if !self.workspaces.contains_key(id) {
            let root = self.runtime.thread_checkout(id)?;
            let approvals_file = self.selection_file.with_file_name("file-approvals.json");
            let changes = cx.new(|cx| Changes::new(root.clone(), approvals_file, id.0.clone(), cx));
            let files = cx.new(|_| FileTree::new(root.clone()));
            let thread = id.clone();
            cx.subscribe_in(&files, window, move |view, _, action, window, cx| {
                if !view
                    .current
                    .as_ref()
                    .is_some_and(|current| current.id == thread)
                {
                    return;
                }
                match action {
                    FileAction::Open(path) => {
                        view.open_document(Document::File(path.clone()), window, cx);
                    }
                    FileAction::AttachFile(path) => {
                        view.return_to_chat(window, cx);
                        view.attach_workspace_file(&thread, path.clone(), window, cx);
                    }
                    FileAction::AttachReference(path) => {
                        view.return_to_chat(window, cx);
                        view.insert_file_reference(path, window, cx);
                    }
                    FileAction::OpenIn(destination, path) => {
                        let destination = *destination;
                        let path = path.clone();
                        view.run_project_action(cx, move |root| {
                            destination.open_file(&root, &path)
                        });
                    }
                }
            })
            .detach();
            let thread = id.clone();
            cx.subscribe_in(
                &changes,
                window,
                move |view, _, row: &ReviewRow, window, cx| {
                    if view
                        .current
                        .as_ref()
                        .is_some_and(|current| current.id == thread)
                    {
                        view.open_document(Document::Diff(row.clone()), window, cx);
                    }
                },
            )
            .detach();
            let thread = id.clone();
            cx.subscribe_in(
                &changes,
                window,
                move |view, _, result: &RevertFinished, window, cx| {
                    view.refresh_reverted_documents(&thread, result, window, cx);
                },
            )
            .detach();
            let draft = cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(2, 5)
                    .placeholder("Describe a task or ask a question")
            });
            Self::observe_composer(&draft, window, cx);
            self.workspaces.insert(
                id.clone(),
                Workspace {
                    root,
                    files,
                    changes,
                    documents: DocumentTabs::default(),
                    traces: traces::Traces::default(),
                    draft,
                    attachments: Attachments::default(),
                    questions: HashMap::new(),
                    #[cfg(target_os = "macos")]
                    terminal: terminal_pane::TerminalPane::default(),
                },
            );
            #[cfg(target_os = "macos")]
            self.resolve_terminal_project(id.clone(), cx);
        }
        Ok(())
    }

    fn refresh(&mut self) -> huggler::Result<()> {
        if let Some(old) = &self.current {
            let thread = match self.runtime.detail(&old.id, self.before.as_ref(), "", 32) {
                Err(huggler::Error::HistoryChanged) => {
                    self.before = None;
                    self.runtime.detail(&old.id, None, "", 32)?
                }
                result => result?,
            };
            if self.before.is_none()
                && history_at_bottom(&self.history_scroll)
                && (thread.history.messages.last().map(|m| &m.message.id)
                    != old.history.messages.last().map(|m| &m.message.id)
                    || thread.streaming_text != old.streaming_text
                    || thread.activity != old.activity)
            {
                self.history_scroll.scroll_to_bottom();
            }
            if self.command_log.as_ref().is_some_and(|log| {
                !thread
                    .activity
                    .iter()
                    .any(|activity| activity.run_id == log.run)
            }) {
                self.command_log = None;
            }
            self.expanded_activity.retain(|(run, group)| {
                thread.activity.iter().any(|activity| {
                    &activity.run_id == run
                        && (group.is_some() || activity.status.producing_output())
                })
            });
            self.full_messages.retain(|id, _| {
                thread
                    .history
                    .messages
                    .iter()
                    .any(|item| &item.message.id == id)
            });
            if thread.latest_run.as_ref().map(|r| (&r.id, r.status))
                != old.latest_run.as_ref().map(|r| (&r.id, r.status))
            {
                self.checkout_unavailable = !self.workspaces[&thread.id].root.is_dir();
            }
            let previous_sequence = old.history.sequence;
            self.current = Some(thread);
            self.refresh_traces(Some(previous_sequence));
        }
        Ok(())
    }

    fn choose_checkout(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.launch_checkout(cx) != path;
        self.picked_checkout = (!path.as_os_str().is_empty()).then(|| path.clone());
        if changed || self.checkout.read(cx).value().as_str() != path.to_string_lossy().as_ref() {
            self.checkout.update(cx, |input, cx| {
                input.set_value(path.to_string_lossy().into_owned(), window, cx)
            });
            self.branch
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.project_picker = None;
        self.error = None;
        self.focus_workspace(window, cx);
        cx.notify();
    }

    fn leave_settings(&mut self, cx: &mut Context<Self>) -> bool {
        if self.show_settings && !self.settings.update(cx, |settings, cx| settings.leave(cx)) {
            return false;
        }
        self.show_settings = false;
        true
    }

    fn open_settings(
        &mut self,
        target: SettingsTarget<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.suggestions = Default::default();
        self.model_picker = None;
        self.mcp_status = None;
        self.project_picker = None;
        self.checkout_browse = None;
        self.launch_workspace.close_picker();
        self.show_settings = true;
        self.settings
            .update(cx, |settings, cx| settings.opened(target, window, cx));
        self.settings.read(cx).focus.focus(window);
        cx.notify();
    }

    fn focus_workspace(&self, window: &mut Window, cx: &App) {
        if self.show_settings {
            self.settings.read(cx).focus.focus(window);
            return;
        }
        if self.backlog.open || self.remote_open() {
            self.focus.focus(window);
            return;
        }
        match &self.current {
            None if self.launch_setup && self.launching.is_none() => {
                self.name.focus_handle(cx).focus(window)
            }
            None if self.launching.is_none() => self.starter.focus_handle(cx).focus(window),
            Some(thread) => {
                let documents = &self.workspaces[&thread.id].documents;
                let focus = match documents.active() {
                    Some(_) => documents
                        .active_editor_focus(cx)
                        .unwrap_or_else(|| self.focus.clone()),
                    None => self.prompt.focus_handle(cx),
                };
                focus.focus(window);
            }
            _ => self.focus.focus(window),
        }
    }

    pub(super) fn focus_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_settings || self.backlog.open || self.launching.is_some() {
            return;
        }
        #[cfg(unix)]
        if let Some(project) = &self.remote_project {
            project.read(cx).focus_message(window, cx);
            return;
        }
        let input = match &self.current {
            Some(_) => {
                self.return_to_chat(window, cx);
                if window.viewport_size().width < NARROW_VIEWPORT_WIDTH
                    && self.right_sidebar.is_open(false)
                {
                    self.right_sidebar.toggle(true);
                }
                &self.prompt
            }
            None if !self.launch_setup => &self.starter,
            None => return,
        };
        self.model_picker = None;
        self.mcp_status = None;
        self.project_picker = None;
        self.launch_workspace.close_picker();
        input.focus_handle(cx).focus(window);
        cx.notify();
    }

    fn new_conversation(&mut self, setup: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.leave_settings(cx) {
            return;
        }
        self.backlog.open = false;
        self.error = None;
        self.suggestions = Default::default();
        self.model_picker = None;
        self.mcp_status = None;
        self.project_picker = None;
        self.checkout_browse = None;
        self.launch_workspace.close_picker();
        self.launch_setup = setup;
        #[cfg(unix)]
        {
            self.remote_project = None;
        }
        if self.current.is_none() {
            self.focus_workspace(window, cx);
            cx.notify();
            return;
        }
        self.launch_model = None;
        self.launch_model_options = ModelOptions::default();
        self.instance = Provider::Codex.key().into();
        self.subscription = None;
        self.command_log = None;
        self.current = None;
        self.pull_request_watch = None;
        self.pull_request = None;
        self.launch_workspace = LaunchWorkspace::default();
        self.load_launch_branches(false, cx);
        self.preload_models(cx);
        self.error = None;
        self.name
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.branch
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.focus_workspace(window, cx);
        cx.notify();
    }

    fn launch_draft(&self, cx: &App) -> LaunchDraft {
        let message = self.starter.read(cx).value().to_string();
        let name = self.name.read(cx).value();
        let name = name.trim();
        let name = if name.is_empty() {
            self.starter_attachments.title(&message)
        } else {
            name.to_owned()
        };
        let checkout = self.launch_checkout(cx);
        let workspace_mode = if checkout.as_os_str().is_empty() {
            WorkspaceMode::Current
        } else {
            self.launch_workspace.requested_mode()
        };
        LaunchDraft {
            name,
            message,
            project_name: self.launch_project_name(cx),
            checkout,
            checkout_root: None,
            workspace_mode,
            selected_branch: self
                .launch_workspace
                .selected_branch(workspace_mode)
                .cloned(),
            branch: self.branch.read(cx).value().trim().to_owned(),
            model: self.launch_model.clone(),
            model_options: self.launch_model_options.clone(),
            instance: self.instance.clone(),
        }
    }

    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launching.is_some()
            || self.checkout_browse.is_some()
            || self.starter_attachments.loading
            || (self.name.read(cx).value().trim().is_empty()
                && self.starter.read(cx).value().trim().is_empty()
                && self
                    .starter_attachments
                    .is_empty(&self.starter.read(cx).value()))
        {
            return;
        }
        let draft = self.launch_draft(cx);
        let attachments = self.starter_attachments.take();
        self.starter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.start_launch(draft, attachments, None, window, cx);
    }

    fn start_launch(
        &mut self,
        draft: LaunchDraft,
        attachments: Attachments,
        saved_draft: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let LaunchDraft {
            name,
            message,
            checkout,
            checkout_root,
            workspace_mode,
            selected_branch,
            branch,
            model,
            model_options,
            instance,
            ..
        } = draft;
        let saved_roots: HashSet<_> = self
            .settings
            .read(cx)
            .saved_projects()
            .iter()
            .map(|project| project.root.clone())
            .collect();
        let scratch_root = self.scratch_root.clone();
        let scratch = checkout.as_os_str().is_empty();
        let runtime = self.runtime.clone();
        let id = ThreadId(saved_draft.unwrap_or_else(Uuid::new_v4).to_string());
        if let Some(draft) = saved_draft {
            self.backlog.sending = Some(draft);
        }
        self.launching = Some(LaunchProgress {
            thread: id.clone(),
            title: name.clone(),
            stage: (workspace_mode == WorkspaceMode::Worktree)
                .then_some(WorktreeStage::ResolveBase),
        });
        self.project_picker = None;
        self.launch_workspace.close_picker();
        self.suggestions = Default::default();
        self.focus.focus(window);
        self.error = None;
        cx.notify();
        let thread = id.clone();
        let first_message = (!message.trim().is_empty() || !attachments.is_empty(&message))
            .then(|| attachments.action(&message));
        let launch_name = name.clone();
        let (report_stage, mut stages) = tokio::sync::mpsc::unbounded_channel();
        let launch = cx.background_spawn(async move {
            let send = || match first_message {
                Some(action) => runtime.dispatch(Command {
                    id: CommandId(saved_draft.map_or_else(
                        || Uuid::new_v4().to_string(),
                        |id| format!("backlog-send-{id}"),
                    )),
                    thread_id: thread.clone(),
                    action,
                }).map(|_| ()),
                None => Ok(()),
            };
            // Launch and enqueue commit separately. A retry uses the existing
            // conversation and the same enqueue ID, even after an app restart.
            if saved_draft.is_some() && backlog::has_draft_conversation(&runtime, &thread)? {
                return Ok((send(), None));
            }
            let source = if scratch {
                fs::create_dir_all(&scratch_root)?;
                let mut path = scratch_root.join(&thread.0);
                if saved_draft.is_some() && path.try_exists()? {
                    path = scratch_root.join(format!("{thread}-{}", Uuid::new_v4()));
                }
                runtime.create_scratch(&path)?
            } else {
                if saved_draft.is_some() && !checkout.try_exists()? {
                    anyhow::bail!("The saved checkout is missing. Restore it and retry.");
                }
                let project = runtime.open_project(&checkout)?;
                if let Some(root) = &checkout_root {
                    anyhow::ensure!(
                        project.root == *root,
                        "The saved checkout now points to a different Git repository. Restore the original checkout and retry."
                    );
                }
                anyhow::ensure!(
                    !saved_roots.contains(&checkout) || project.root == checkout,
                    "The saved project checkout has moved. Choose a Git checkout for this conversation."
                );
                project
            };
            let source_root = source.root.clone();
            let (project, allocated) = launch_workspace::prepare(
                &runtime,
                source,
                workspace_mode,
                selected_branch,
                &branch,
                &name,
                |stage| {
                    let _ = report_stage.send(stage);
                },
            )?;
            if allocated {
                let _ = report_stage.send(Some(WorktreeStage::OpenConversation));
            }
            if let Err(error) = runtime.dispatch(Command {
                id: CommandId(Uuid::new_v4().to_string()),
                thread_id: thread.clone(),
                action: Action::LaunchModelSelection {
                    project: project.id.clone(),
                    instance,
                    name,
                    model,
                    options: model_options,
                },
            }) {
                let error = error.into();
                return Err(if allocated {
                    launch_workspace::release_after_failure(&runtime, &project, error)
                } else {
                    error
                });
            }
            // A saved linked checkout can define its own actions. Git only reports
            // the repository's main checkout from the newly allocated worktree.
            let worktree_project = (allocated
                && saved_roots.contains(&source_root)
                && git::main_worktree(&project.root).is_ok_and(|main| main != source_root))
            .then_some((project.root, source_root));
            anyhow::Ok((send(), worktree_project))
        });
        cx.spawn_in(window, async move |view, cx| {
            while let Some(stage) = stages.recv().await {
                let _ = view.update_in(cx, |v, _, cx| {
                    if let Some(progress) = v.launching.as_mut().filter(|p| p.thread == id) {
                        progress.stage = stage;
                        cx.notify();
                    }
                });
            }
            let result = launch.await;
            let _ = view.update_in(cx, |v, w, cx| {
                let still_open = v.launching.as_ref().is_some_and(|p| p.thread == id);
                if still_open {
                    v.launching = None;
                }
                if saved_draft.is_some() {
                    v.backlog.sending = None;
                }
                match result {
                    Ok((sent, worktree_project)) => {
                        let saved = worktree_project.map(|(worktree, root)| {
                            v.settings.update(cx, |settings, cx| {
                                settings.save_worktree_project(worktree, root, cx)
                            })
                        });
                        let cleanup = saved_draft
                            .filter(|_| sent.is_ok())
                            .map(|draft| v.remove_saved_draft(draft));
                        let failed_draft = saved_draft.filter(|_| sent.is_err());
                        let result = (|| {
                            if sent.is_err() && saved_draft.is_none() {
                                v.ensure_workspace(&id, w, cx)?;
                                v.workspaces.get_mut(&id).unwrap().attachments = attachments;
                                v.workspaces[&id]
                                    .draft
                                    .update(cx, |input, cx| input.set_value(message, w, cx));
                            }
                            if still_open
                                && !v.show_settings
                                && (v.backlog.open == saved_draft.is_some())
                                && (saved_draft.is_none() || sent.is_ok())
                            {
                                v.select(id, w, cx)?;
                            }
                            sent
                        })();
                        if let (Some(draft), Err(error)) = (failed_draft, &result) {
                            v.backlog
                                .errors
                                .insert(draft, format!("Could not send draft. {error}"));
                        } else {
                            v.report(result, cx);
                        }
                        if let Some(Err(error)) = cleanup {
                            v.error = Some(format!(
                                "Message sent. Could not remove its Backlog draft. {error}"
                            ));
                        } else if v.error.is_none()
                            && let Some(Err(error)) = saved
                        {
                            v.error = Some(format!("{error:#}"));
                        }
                    }
                    Err(error) => {
                        if saved_draft.is_none()
                            && v.starter.read(cx).value().is_empty()
                            && v.starter_attachments.is_empty("")
                        {
                            v.starter_attachments = attachments;
                            v.starter
                                .update(cx, |input, cx| input.set_value(message, w, cx));
                        }
                        let error = format!("Could not open {launch_name}. {error}");
                        if let Some(draft) = saved_draft {
                            v.backlog.errors.insert(draft, error);
                        } else {
                            v.error = Some(error);
                        }
                        if still_open {
                            v.focus_workspace(w, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) -> huggler::Result<()> {
        let Some(thread) = &self.current else {
            return Ok(());
        };
        let text = self.prompt.read(cx).value().to_string();
        let attachments = &self.workspaces[&thread.id].attachments;
        if attachments.loading {
            return Ok(());
        }
        if text.trim().is_empty() && attachments.is_empty(&text) {
            return Ok(());
        }
        if thread.foreground_run.is_none() && thread.queued_count == 0 {
            let root = &self.workspaces[&thread.id].root;
            self.checkout_unavailable = !root.is_dir();
            if self.checkout_unavailable {
                // Keep the draft while the user restores the checkout.
                cx.notify();
                return Ok(());
            }
        }
        let steer = attachments.is_empty(&text)
            && self.settings.read(cx).preferences().follow_up == FollowUp::Steer;
        let steering_text = steer.then(|| text.clone());
        let action = if steer {
            Action::SendAuto { text }
        } else {
            attachments.action(&text)
        };
        let receipt = self.runtime.dispatch(Command {
            id: CommandId(Uuid::new_v4().to_string()),
            thread_id: thread.id.clone(),
            action,
        })?;
        if let Some(text) = steering_text.filter(|_| receipt.message_id.is_none()) {
            self.watch_steering(receipt, text, window, cx);
        }
        self.workspaces
            .get_mut(&thread.id)
            .unwrap()
            .attachments
            .take();
        self.prompt
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.refresh()
    }

    fn watch_steering(
        &self,
        receipt: huggler::Receipt,
        text: String,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |_, cx| {
            let mut cursor = receipt.event_sequences.last().copied().unwrap_or_default();
            let thread = receipt.thread_id;
            let command = receipt.command_id;
            let result = loop {
                match runtime.steer_outcome(&thread, &command) {
                    Ok(SteerOutcome::Pending) => {
                        match runtime.wait_for_change(&thread, cursor).await {
                            Ok(sequence) => cursor = sequence,
                            Err(error) => {
                                break format!("Could not check follow-up delivery. {error}");
                            }
                        }
                    }
                    Ok(SteerOutcome::Accepted) => return,
                    Ok(SteerOutcome::Rejected(reason)) => {
                        break format!("Could not steer the turn. {reason}");
                    }
                    Ok(SteerOutcome::Uncertain) => {
                        break "Could not confirm delivery of the follow-up.".into();
                    }
                    Err(error) => break format!("Could not check follow-up delivery. {error}"),
                }
            };
            let _ = cx.update(|window, cx| {
                use gpui_component::{WindowExt, notification::Notification};
                window.push_notification(
                    Notification::warning(result)
                        .autohide(false)
                        .action(move |_, _, _| {
                            let text = text.clone();
                            Button::new("copy-follow-up")
                                .small()
                                .label("Copy message")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                                })
                        }),
                    cx,
                );
            });
        })
        .detach();
    }

    fn choose_provider(&mut self, id: &ProviderInstanceId, cx: &mut Context<Self>) {
        if self.instance != *id {
            self.instance.clone_from(id);
            self.launch_model = None;
            self.launch_model_options = ModelOptions::default();
            self.mcp_status = None;
            if self.current.is_none() {
                self.suggestions = Default::default();
            }
            self.preload_models(cx);
        }
    }

    fn navigation_toggle(&self, open: bool, cx: &Context<Self>) -> Div {
        let action = if open {
            "Hide conversations"
        } else {
            "Show conversations"
        };
        control!(
            action,
            Button::new("toggle-navigation")
                .ghost()
                .small()
                .icon(IconName::PanelLeft)
                .selected(open)
                .tooltip(action)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.navigation.toggle(open);
                    cx.notify();
                }))
        )
    }

    fn workspace_header(&self, navigation_open: bool, right_open: bool, cx: &Context<Self>) -> Div {
        let (project, title) = match &self.current {
            Some(thread) => (
                self.workspaces[&thread.id]
                    .root
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
                thread.name.as_str(),
            ),
            None => (self.launch_project_name(cx), "Create"),
        };
        let crumb = h_flex()
            .ml_2()
            .flex_1()
            .min_w_0()
            .gap_2()
            .text_sm()
            .child(
                Icon::new(IconName::Folder)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .max_w(relative(0.40))
                    .truncate()
                    .text_color(cx.theme().muted_foreground)
                    .child(project.unwrap_or_else(|| "Workspace".into())),
            )
            .child(div().text_color(cx.theme().muted_foreground).child("/"))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title.to_owned()),
            );
        h_flex()
            .h_10()
            .flex_shrink_0()
            .px_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .gap_2()
            .child(self.navigation_toggle(navigation_open, cx))
            .child(crumb)
            .when(self.current.is_some(), |header| {
                let action = match (self.workspace_tab, right_open) {
                    (WorkspaceTab::AllFiles, true) => "Hide all files",
                    (WorkspaceTab::AllFiles, false) => "Show all files",
                    (WorkspaceTab::Changes, true) => "Hide changes",
                    (WorkspaceTab::Changes, false) => "Show changes",
                    (WorkspaceTab::Traces, true) => "Hide traces",
                    (WorkspaceTab::Traces, false) => "Show traces",
                };
                header.child(self.project_actions(cx)).child(control!(
                    action,
                    Button::new("toggle-right-sidebar")
                        .ghost()
                        .small()
                        .icon(IconName::PanelRight)
                        .selected(right_open)
                        .tooltip(action)
                        .on_click(cx.listener(move |view, _, _window, cx| {
                            #[cfg(target_os = "macos")]
                            if right_open
                                && let Some(thread) = &view.current
                                && view.workspaces[&thread.id].terminal.is_focused(_window, cx)
                            {
                                view.focus_workspace(_window, cx);
                            }
                            view.right_sidebar.toggle(right_open);
                            if !right_open && let Some(thread) = &view.current {
                                view.workspace_tab.refresh(&view.workspaces[&thread.id], cx);
                                if view.workspace_tab == WorkspaceTab::Traces {
                                    view.refresh_trace_list();
                                }
                            }
                            cx.notify();
                        }))
                ))
            })
    }

    fn workspace_tabs(&self, width: f32, cx: &Context<Self>) -> Div {
        h_flex().flex_shrink_0().h_10().px_1().gap_0p5().children(
            [
                WorkspaceTab::AllFiles,
                WorkspaceTab::Changes,
                WorkspaceTab::Traces,
            ]
            .map(|tab| {
                let label = tab.label();
                control!(
                    format!("{label} tab"),
                    Button::new(label)
                        .label(label)
                        .ghost()
                        .small()
                        .px_1p5()
                        .when(width < 190., |button| button.xsmall().h_6().px_0p5())
                        .selected(self.workspace_tab == tab)
                        .when(self.workspace_tab == tab, |button| {
                            button
                                .bg(cx.theme().sidebar_accent)
                                .text_color(cx.theme().sidebar_accent_foreground)
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.workspace_tab = tab;
                            if let Some(thread) = &view.current {
                                tab.refresh(&view.workspaces[&thread.id], cx);
                            }
                            if tab == WorkspaceTab::Traces {
                                view.refresh_trace_list();
                            }
                            cx.notify();
                        }))
                )
            }),
        )
    }

    fn send_controls(
        &self,
        attachments: &Attachments,
        _name: &'static str,
        button: Button,
        cx: &Context<Self>,
    ) -> Div {
        h_flex()
            .gap_2()
            .flex_shrink_0()
            .child(self.attach_button(attachments, cx))
            .child(control!(_name, button.primary()))
    }

    fn send_icon(id: &'static str, tooltip: impl Into<SharedString>) -> Button {
        Button::new(id)
            .size_8()
            .px_0()
            .rounded(px(999.))
            .child(Icon::new(IconName::ArrowUp).with_size(px(16.)))
            .tooltip(tooltip)
    }

    fn composer_surface(cx: &App) -> Div {
        v_flex()
            .p_3()
            .gap_2()
            .rounded_xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted.opacity(0.55))
    }

    fn launch_form(&self, compact: bool, window: &Window, cx: &Context<Self>) -> AnyElement {
        if let Some(progress) = &self.launching {
            return progress.render(cx).into_any_element();
        }
        let check = self
            .model_catalogs
            .get(&self.model_source(cx))
            .map(|check| check.read(cx));
        let checkout_empty = self.launch_checkout(cx).as_os_str().is_empty();
        let browsing = self.checkout_browse.is_some();
        let text = self.starter.read(cx).value();
        let empty_message = text.trim().is_empty() && self.starter_attachments.is_empty(&text);
        let draft_unavailable = empty_message || browsing || self.starter_attachments.loading;
        let project_row = h_flex()
            .w_full()
            .justify_between()
            .gap_2()
            .child(self.project_control(window, cx))
            .child(self.save_backlog_button(draft_unavailable, cx));
        if !self.launch_setup {
            let title = self.launch_project_name(cx).map_or_else(
                || "What should we build?".into(),
                |name| format!("What should we build in {name}?"),
            );
            let composer = Self::composer_surface(cx)
                .w_full()
                .child(self.message_input("First message", &self.starter, window, cx))
                .children(self.attachment_list(&self.starter_attachments, cx))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_2()
                        .child(self.model_control(cx))
                        .child(div().flex_1().min_w_0())
                        .child(self.mcp_status_control(window, cx))
                        .child(
                            self.send_controls(
                                &self.starter_attachments,
                                "Send first message",
                                Self::send_icon(
                                    "send-first",
                                    format!(
                                        "Send ({})",
                                        self.settings.read(cx).preferences().send_shortcut.label()
                                    ),
                                )
                                .disabled(draft_unavailable)
                                .on_click(cx.listener(|v, _, w, cx| v.launch(w, cx))),
                                cx,
                            ),
                        ),
                );
            let body = v_flex()
                .on_drop(cx.listener(Self::drop_attachments))
                .w_full()
                .max_w(px(736.))
                .p_4()
                .gap_2()
                .child(heading(title).text_xl().mb_1())
                .child(project_row)
                .child(composer)
                .when(!checkout_empty, |body| {
                    body.child(self.launch_checkout_controls(cx))
                });
            return v_flex()
                .id("starter")
                .flex_1()
                .min_h_0()
                .items_center()
                .overflow_y_scroll()
                .child(body.flex_shrink_0().my_auto())
                .into_any_element();
        }
        let launcher = Self::composer_surface(cx)
            .on_drop(cx.listener(Self::drop_attachments))
            .w_full()
            .children(self.attachment_list(&self.starter_attachments, cx))
            .child(control!(
                "Conversation name",
                Input::new(&self.name).appearance(false)
            ))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(self.model_control(cx))
                    .child(div().flex_1().min_w_0())
                    .child(
                        self.send_controls(
                            &self.starter_attachments,
                            if empty_message {
                                "Open conversation"
                            } else {
                                "Send first message"
                            },
                            (if empty_message {
                                Button::new("launch")
                                    .small()
                                    .label("Open conversation")
                                    .icon(IconName::ArrowRight)
                            } else {
                                Self::send_icon("launch", "Send")
                            })
                            .disabled(
                                browsing
                                    || self.starter_attachments.loading
                                    || (empty_message
                                        && self.name.read(cx).value().trim().is_empty()),
                            )
                            .on_click(cx.listener(|v, _, w, cx| v.launch(w, cx))),
                            cx,
                        ),
                    ),
            );
        let context = h_flex()
            .items_end()
            .gap_3()
            .when(compact, |row| row.flex_col().items_start())
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .when(compact, |field| field.w_full().flex_none())
                    .child(label("Git checkout"))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(control!("Git checkout", Input::new(&self.checkout))),
                            )
                            .child(control!(
                                "Browse",
                                Button::new("browse")
                                    .ghost()
                                    .icon(IconName::FolderOpen)
                                    .tooltip("Browse")
                                    .disabled(browsing)
                                    .on_click(
                                        cx.listener(|v, _, w, cx| v.browse_project(false, w, cx))
                                    )
                            )),
                    ),
            )
            .when(
                !checkout_empty && self.launch_workspace.mode() == WorkspaceMode::Worktree,
                |row| {
                    row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .when(compact, |field| field.w_full().flex_none())
                            .child(field("New worktree branch (optional)", &self.branch)),
                    )
                },
            );
        let body = v_flex()
            .w_full()
            .max_w(px(736.))
            .p_4()
            .gap_3()
            .child(h_flex().child(control!(
                "Back to message",
                Button::new("back-to-message")
                    .small()
                    .ghost()
                    .icon(IconName::ChevronLeft)
                    .label("Back to message")
                    .on_click(cx.listener(|v, _, w, cx| v.new_conversation(false, w, cx)))
            )))
            .child(launcher)
            .when(!checkout_empty, |body| {
                body.child(self.launch_checkout_controls(cx))
            })
            .child(context)
            .child(project_row)
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(control!(
                        "Check provider",
                        Button::new("check-provider")
                            .small()
                            .ghost()
                            .label("Check provider")
                            .disabled(checkout_empty || check.is_some_and(ProviderCatalog::loading))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.catalog(true, cx);
                            }))
                    ))
                    .when_some(check, |row, check| {
                        let status = check.status();
                        row.child(control!(
                            format!("provider-status/{status}"),
                            hint(status, cx)
                        ))
                    }),
            );
        v_flex()
            .id("launch-form")
            .flex_1()
            .min_h_0()
            .items_center()
            .overflow_y_scroll()
            .child(body.flex_shrink_0().my_auto())
            .into_any_element()
    }

    fn conversation(
        &self,
        thread: &ConversationDetail,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let last_run = thread
            .foreground_run
            .as_ref()
            .or(thread.latest_run.as_ref());
        let busy = thread.foreground_run.is_some() || thread.queued_count > 0;
        let can_stop =
            thread.foreground_run.is_some() || (thread.queued_count > 0 && !thread.queue_held);
        let attachments = &self.workspaces[&thread.id].attachments;
        let text = self.prompt.read(cx).value();
        let empty_draft =
            attachments.loading || (text.trim().is_empty() && attachments.is_empty(&text));
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        let status = match last_run.map(|run| run.status) {
            Some(huggler::RunStatus::Running | huggler::RunStatus::Completed) => None,
            Some(status) => Some(format!("{status:?}")),
            None => Some("Ready".to_owned()),
        };
        let message_input = self.message_input("Message", &self.prompt, window, cx);
        // Qualification checks observe conversation state without rendering a status label.
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        let message_input = match status {
            Some(status) => {
                crate::qualification::observe(format!("status/{status}").into(), message_input)
            }
            None => message_input,
        };
        let workspace = &self.workspaces[&thread.id];
        let stop = can_stop.then(|| {
            let id = thread.id.clone();
            control!(
                "Stop thread",
                Button::new("stop-turn")
                    .size_8()
                    .px_0()
                    .rounded(px(999.))
                    .child(
                        div()
                            .size_3()
                            .rounded_sm()
                            .bg(cx.theme().secondary_foreground),
                    )
                    .tooltip("Stop")
                    .on_click(cx.listener(move |view, _, _, cx| {
                        let result = view
                            .runtime
                            .stop_thread(CommandId(Uuid::new_v4().to_string()), id.clone())
                            .map(|_| ());
                        view.report(result, cx);
                    })),
            )
        });
        let preferences = self.settings.read(cx).preferences();
        let (_action_label, action) =
            if busy && attachments.is_empty(&text) && preferences.follow_up == FollowUp::Steer {
                ("Send follow-up", "Send follow-up")
            } else if busy {
                ("Queue message", "Queue")
            } else {
                ("Send", "Send")
            };
        let send = Self::send_icon(
            "send",
            format!("{action} ({})", preferences.send_shortcut.label()),
        )
        .disabled(empty_draft)
        .on_click(cx.listener(|view, _, window, cx| {
            let result = view.send(window, cx);
            view.report(result, cx);
        }));
        let pending = thread.permissions.len() + thread.questions.len();
        let fresh = last_run.is_none()
            && thread.history.messages.is_empty()
            && self.before.is_none()
            && pending == 0
            && !busy;
        // Keep the left rail clear of the centered text in narrow windows.
        let transcript_width = (width - px(24.)).clamp(px(0.), px(800.));
        let width = width.min(px(800.));
        let scroll_to_end_visibility = (!fresh).then(|| {
            window.use_keyed_state(
                SharedString::from(format!("show-scroll-to-end-{}", thread.id)),
                cx,
                |_, _| false,
            )
        });
        let show_scroll_to_end = self.before.is_some()
            || scroll_to_end_visibility
                .as_ref()
                .is_some_and(|visible| *visible.read(cx));
        let page = v_flex()
            .flex_1()
            .min_h_0()
            .when(fresh, |page| page.justify_center())
            .when(!fresh, |page| {
                page.child(self.transcript(thread, transcript_width, window, cx))
            })
            .when_some(scroll_to_end_visibility, |page, visible| {
                let history = self.history_scroll.clone();
                page.on_children_prepainted(move |_, _, cx| {
                    // The scroll handle has clamped the offset against the current layout.
                    let next = !history_at_bottom(&history);
                    if *visible.read(cx) != next {
                        visible.update(cx, |visible, cx| {
                            *visible = next;
                            cx.notify();
                        });
                    }
                })
            })
            .when(pending != 0, |page| {
                page.child(
                    div()
                        .id("permissions")
                        .w(width)
                        .mx_auto()
                        .px_5()
                        .max_h(relative(0.30))
                        .overflow_y_scroll()
                        .children(
                            thread
                                .permissions
                                .iter()
                                .map(|r| Self::permission(thread, r, cx)),
                        )
                        .children(
                            thread
                                .questions
                                .iter()
                                .filter_map(|q| workspace.questions.get(&q.id).cloned()),
                        ),
                )
            });
        let column = v_flex()
            .w(width)
            .mx_auto()
            .relative()
            .when(show_scroll_to_end, |column| column.child(Self::scroll_to_end_button(cx)))
            .when(fresh, |column| {
                column.child(
                    heading("What should we build?").text_xl().mx_4().mb_3(),
                )
            })
        .child(
            Self::composer_surface(cx)
                .on_drop(cx.listener(Self::drop_attachments))
                .mx_4().mb_2()
                .when(self.checkout_unavailable, |composer| {
                    composer
                        .child(control!(
                            "Checkout unavailable",
                            div().text_sm().child(format!(
                                "Restore {} to continue this conversation, or start one in another checkout.",
                                workspace.root.display()
                            ))
                        ))
                        .child(control!(
                            "Choose another checkout",
                            Button::new("replace-checkout")
                                .label("Choose another checkout")
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.new_conversation(false, window, cx);
                                    view.open_project_picker(window, cx);
                                }))
                        ))
                })
                .when_some(
                    last_run.and_then(|run| run.prompt_suggestion.clone()),
                    |row, hint| {
                        row.child(control!(
                            "Suggested reply",
                            Button::new("suggestion")
                                .label(hint.clone())
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    if view.prompt.read(cx).value().is_empty() {
                                        view.prompt.update(cx, |input, cx| {
                                            input.set_value(hint.clone(), window, cx)
                                        });
                                        view.prompt.focus_handle(cx).focus(window);
                                    }
                                }))
                        ))
                    },
                )
                .when_some(last_run.filter(|_| !self.checkout_unavailable).and_then(|r| r.error.clone()), |area, error| {
                    area.child(div().id("run-error").max_h(px(64.)).overflow_y_scroll().text_sm().text_color(cx.theme().danger).child(error))
                })
                .child(message_input)
                .children(self.attachment_list(attachments, cx))
                .when(thread.queued_count > 0, |composer| {
                    let count = thread.queued_count;
                    let noun = if count == 1 { "message" } else { "messages" };
                    let summary = if thread.queue_held {
                        format!("{count} queued {noun} · paused")
                    } else {
                        format!("{count} queued {noun}")
                    };
                    composer.child(
                        h_flex()
                            .gap_2()
                            .child(control!(summary.clone(), div().text_sm().child(summary)))
                            .when(thread.queue_held, |row| {
                                let thread = thread.id.clone();
                                row.child(control!(
                                    "Resume queue",
                                    Button::new("resume-queue").label("Resume queue").on_click(
                                        cx.listener(move |v, _, _, cx| {
                                            let result = v
                                                .runtime
                                                .resume_queue(
                                                    CommandId(Uuid::new_v4().to_string()),
                                                    thread.clone(),
                                                )
                                                .map(|_| ());
                                            v.report(result, cx);
                                        })
                                    )
                                ))
                            }),
                    )
                })
                .child(
                    h_flex()
                        .flex_wrap().gap_2()
                        .child(self.model_control(cx))
                        .child(div().flex_1().min_w_0())
                        .child(self.mcp_status_control(window, cx))
                        .child(h_flex().gap_2().flex_shrink_0()
                            .child(self.attach_button(attachments, cx))
                            .children(stop)
                            .when(!can_stop || !empty_draft, |row| {
                                row.child(control!(_action_label, send.primary()))
                            })),
                ),
        )
        .pb_2();
        page.child(column)
    }

    fn scroll_to_end_button(cx: &Context<Self>) -> Div {
        h_flex()
            .absolute()
            .left_0()
            .right_0()
            .bottom(relative(1.))
            .justify_center()
            .pb_2()
            .child(control!(
                "Scroll to end",
                Button::new("scroll-to-end")
                    .size(px(30.))
                    .p_0()
                    .rounded(px(999.))
                    .border_color(cx.theme().border)
                    .shadow_sm()
                    .block_mouse_except_scroll()
                    .child(Icon::new(IconName::ArrowDown).with_size(px(16.)))
                    .on_click(cx.listener(|view, _, _, cx| view.scroll_to_end(cx)))
            ))
    }
}

impl Render for Desktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        crate::qualification::begin_frame(cx);

        let clock_start = self
            .current
            .as_ref()
            .filter(|_| !self.show_settings && self.before.is_none())
            .and_then(|detail| detail.foreground_run.as_ref())
            .filter(|run| {
                matches!(
                    run.status,
                    huggler::RunStatus::Starting | huggler::RunStatus::Running
                )
            })
            .and_then(|run| run.started_at_ms.map(|start| (run.id.clone(), start)));
        self.turn_clock = match clock_start {
            Some((id, _))
                if self
                    .turn_clock
                    .as_ref()
                    .is_some_and(|clock| clock.read(cx).run == id) =>
            {
                self.turn_clock.take()
            }
            Some((id, start)) => Some(cx.new(|cx| TurnClock::new(id, start, cx))),
            None => None,
        };

        if let Some(thread) = &self.current {
            let workspace = self
                .workspaces
                .get_mut(&thread.id)
                .expect("selected workspace");
            workspace
                .questions
                .retain(|id, _| thread.questions.iter().any(|q| q.id == *id));
            for q in &thread.questions {
                workspace.questions.entry(q.id.clone()).or_insert_with(|| {
                    cx.new(|cx| {
                        Questions::new(
                            self.runtime.clone(),
                            thread.id.clone(),
                            q.clone(),
                            window,
                            cx,
                        )
                    })
                });
            }
        }

        use sidebar::SidebarEdge;
        let viewport = f32::from(window.viewport_size().width);
        let narrow = window.viewport_size().width < NARROW_VIEWPORT_WIDTH;
        let right_open = self.right_sidebar.is_open(!narrow);
        let navigation_default = !self.show_settings || !narrow;
        let navigation_open = self.navigation.is_open(navigation_default);
        let left = self.navigation.fraction(navigation_default, window);
        let remote = self.remote_open();
        let right = if self.current.is_some() && self.local_workspace_visible() {
            self.right_sidebar.fraction(!narrow, window)
        } else {
            0.
        };
        let left_max =
            SidebarEdge::Left.max_width(viewport, narrow, self.right_sidebar.width, right);
        let right_max = SidebarEdge::Right.max_width(viewport, narrow, self.navigation.width, left);
        let left_width = self.navigation.width.min(left_max);
        let right_width = self.right_sidebar.width.min(right_max);
        #[cfg(target_os = "macos")]
        let terminal_max_height =
            (f32::from(window.viewport_size().height) - if narrow { 40. } else { 0. } - 144.)
                .max(100.);
        let conversation_width =
            px(viewport - left_width * left - if narrow { 0. } else { right_width * right });
        div()
            .flex()
            .track_focus(&self.focus)
            .when(
                self.local_workspace_visible()
                    && self.current.as_ref().is_some_and(|thread| {
                        self.workspaces[&thread.id].documents.active().is_some()
                    }),
                |layout| layout.key_context(DOCUMENT_CONTEXT),
            )
            .when(!remote && !self.backlog.open, |layout| {
                layout
                    .on_action(cx.listener(Self::close_active_document))
                    .on_action(cx.listener(Self::save_active_document))
            })
            .on_action(cx.listener(|v, _: &crate::OpenSettings, w, cx| {
                v.open_settings(SettingsTarget::Overview, w, cx)
            }))
            .on_action(
                cx.listener(|v, _: &crate::NewConversation, w, cx| {
                    v.new_conversation(false, w, cx)
                }),
            )
            .on_action(
                cx.listener(|v, _: &crate::NewConversationWithSetup, w, cx| {
                    v.new_conversation(true, w, cx);
                }),
            )
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .when(left > 0., |layout| {
                layout.child(
                    h_flex()
                        .w(px(left_width * left))
                        .flex_shrink_0()
                        .overflow_hidden()
                        .justify_end()
                        .child(
                            v_flex()
                                .h_full()
                                .w(px(left_width))
                                .flex_shrink_0()
                                .bg(cx.theme().sidebar)
                                .border_r_1()
                                .border_color(cx.theme().sidebar_border)
                                .p_3()
                                .gap_3()
                                .child(
                                    h_flex()
                                        .h_7()
                                        .gap_2()
                                        .child(IconName::Inbox)
                                        .child(label("Huggler")),
                                )
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(control!(
                                            "Create",
                                            Button::new("new")
                                                .small()
                                                .compact()
                                                .w_full()
                                                .ghost()
                                                .justify_start()
                                                .icon(IconName::Plus)
                                                .label("Create")
                                                .on_click(cx.listener(|view, _, w, cx| {
                                                    view.new_conversation(false, w, cx)
                                                })),
                                        ))
                                        .child(control!(
                                            "Settings",
                                            Button::new("settings")
                                                .small()
                                                .compact()
                                                .w_full()
                                                .justify_start()
                                                .ghost()
                                                .icon(IconName::Settings)
                                                .label("Settings")
                                                .selected(self.show_settings)
                                                .on_click(cx.listener(|view, _, w, cx| {
                                                    view.open_settings(
                                                        SettingsTarget::Overview,
                                                        w,
                                                        cx,
                                                    )
                                                }))
                                        ))
                                        .child(control!(
                                            "Backlog",
                                            Button::new("backlog")
                                                .small()
                                                .compact()
                                                .w_full()
                                                .justify_start()
                                                .ghost()
                                                .icon(IconName::GalleryVerticalEnd)
                                                .label("Backlog")
                                                .selected(self.backlog.open && !self.show_settings)
                                                .on_click(cx.listener(|view, _, w, cx| {
                                                    view.open_backlog(w, cx)
                                                }))
                                        )),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .h_px()
                                        .flex_shrink_0()
                                        .bg(cx.theme().sidebar_border),
                                )
                                .map(|sidebar| {
                                    #[cfg(unix)]
                                    let sidebar = sidebar.children(
                                        self.settings
                                            .read(cx)
                                            .preferences()
                                            .machines
                                            .iter()
                                            .enumerate()
                                            .map(|(index, machine)| {
                                                let machine = machine.clone();
                                                let name = machine
                                                    .checkout
                                                    .file_name()
                                                    .unwrap_or(machine.checkout.as_os_str())
                                                    .to_string_lossy();
                                                let title =
                                                    format!("{} · {name}", machine.target.host);
                                                control!(
                                                    format!("machine/{title}"),
                                                    Button::new(("machine", index))
                                                        .label(title)
                                                        .small()
                                                        .compact()
                                                        .ghost()
                                                        .justify_start()
                                                        .w_full()
                                                        .on_click(cx.listener(
                                                            move |view, _, w, cx| {
                                                                view.open_machine(
                                                                    machine.clone(),
                                                                    w,
                                                                    cx,
                                                                )
                                                            }
                                                        ))
                                                )
                                            }),
                                    );
                                    sidebar
                                })
                                .child(self.conversation_sidebar(left_width, cx)),
                        ),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .when(self.local_workspace_visible(), |pane| {
                        pane.child(self.workspace_header(navigation_open, right_open, cx))
                            .child(self.document_tab_strip(window, cx))
                    })
                    .when(remote && !self.show_settings && !navigation_open, |pane| {
                        pane.child(
                            div()
                                .px_4()
                                .py_1()
                                .child(self.navigation_toggle(navigation_open, cx)),
                        )
                    })
                    .child(if self.show_settings {
                        let toggle = self.navigation_toggle(navigation_open, cx);
                        self.settings
                            .update(cx, |settings, cx| settings.panel(toggle, cx))
                            .into_any_element()
                    } else if self.backlog.open {
                        self.backlog_page(navigation_open, conversation_width, cx)
                            .into_any_element()
                    } else {
                        match &self.current {
                            #[cfg(unix)]
                            _ if remote => self
                                .remote_project
                                .as_ref()
                                .unwrap()
                                .clone()
                                .into_any_element(),
                            Some(thread) => {
                                let workspace = &self.workspaces[&thread.id];
                                match workspace.documents.active() {
                                    Some(tab) => tab
                                        .render(
                                            &workspace.changes,
                                            &workspace.documents,
                                            window,
                                            cx,
                                        )
                                        .into_any_element(),
                                    None => self
                                        .conversation(thread, conversation_width, window, cx)
                                        .into_any_element(),
                                }
                            }
                            None => {
                                self.launch_form(viewport - left_width * left < 800., window, cx)
                            }
                        }
                    })
                    .when_some(
                        self.error.as_ref().filter(|_| !self.show_settings),
                        |view, error| {
                            view.child(control!(
                                format!("error/{error}"),
                                div()
                                    .px_4()
                                    .py_2()
                                    .text_sm()
                                    .text_color(cx.theme().danger)
                                    .child(error.clone()),
                            ))
                        },
                    ),
            )
            .when(left == 1., |layout| {
                layout.child(SidebarEdge::Left.handle(
                    "Resize conversations",
                    left_width,
                    narrow,
                    cx,
                ))
            })
            .when_some(
                self.current
                    .as_ref()
                    .filter(|_| right > 0. && !self.show_settings),
                |layout, thread| {
                    let workspace = &self.workspaces[&thread.id];
                    let panel = match self.workspace_tab {
                        WorkspaceTab::AllFiles => workspace.files.clone().into_any_element(),
                        WorkspaceTab::Changes => workspace.changes.clone().into_any_element(),
                        WorkspaceTab::Traces => {
                            self.trace_panel(&workspace.traces, cx).into_any_element()
                        }
                    };
                    layout.child(
                        div()
                            .w(px(right_width * right))
                            .overflow_hidden()
                            .flex_shrink_0()
                            // Keep file clicks inside the panel even when it covers the workspace.
                            .occlude()
                            .when(narrow, |panel| {
                                panel.absolute().right_0().top_10().bottom_0()
                            })
                            .child(
                                v_flex()
                                    .w(px(right_width))
                                    .h_full()
                                    .border_l_1()
                                    .border_color(cx.theme().sidebar_border)
                                    .bg(cx.theme().sidebar)
                                    .child(self.workspace_tabs(right_width, cx))
                                    .child(div().flex_1().min_h_0().overflow_hidden().child(panel))
                                    .map(|panel| {
                                        #[cfg(target_os = "macos")]
                                        let panel = panel.child(self.terminal_pane(
                                            thread,
                                            terminal_max_height,
                                            cx,
                                        ));
                                        panel
                                    }),
                            ),
                    )
                },
            )
            .when(
                self.current.is_some() && !self.show_settings && right == 1.,
                |layout| {
                    let resize = match self.workspace_tab {
                        WorkspaceTab::AllFiles => "Resize all files",
                        WorkspaceTab::Changes => "Resize changes",
                        WorkspaceTab::Traces => "Resize traces",
                    };
                    layout.child(SidebarEdge::Right.handle(resize, right_width, narrow, cx))
                },
            )
            .on_drag_move(
                cx.listener(move |view, event: &DragMoveEvent<SidebarEdge>, _, cx| {
                    let x = f32::from(event.event.position.x - event.bounds.left());
                    match event.drag(cx) {
                        SidebarEdge::Left => view.navigation.width = x.clamp(160., left_max),
                        SidebarEdge::Right => {
                            view.right_sidebar.width = (viewport - x).clamp(160., right_max)
                        }
                    }
                    cx.notify();
                }),
            )
            .children(gpui_component::Root::render_notification_layer(window, cx))
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}
