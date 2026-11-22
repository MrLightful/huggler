mod attachments;
mod launch_workspace;
mod models;
mod project_actions;
mod sidebar;
mod suggestions;
mod transcript;

use attachments::Attachments;
use gpui_component::list::ListState;
use launch_workspace::{LaunchWorkspace, WorkspaceMode};
use models::{Models, ProviderCatalog};
use transcript::TurnClock;

use crate::{
    changes::Changes,
    git,
    paths::StoragePaths,
    questions::Questions,
    settings::{Preferences, Settings, SettingsEvent},
    ui::{field, hint, label},
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
    ProviderInstanceId, RunId, ThreadId,
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
    changes: Entity<Changes>,
    draft: Entity<InputState>,
    attachments: Attachments,
    questions: HashMap<huggler::RequestId, Entity<Questions>>,
}

pub struct Desktop {
    settings: Entity<Settings>,
    show_settings: bool,
    navigation: sidebar::Sidebar,
    changes_sidebar: sidebar::Sidebar,
    focus: FocusHandle,
    workspaces: HashMap<ThreadId, Workspace>,
    runtime: Arc<Huggler>,
    executor: tokio::runtime::Handle,
    selection_file: PathBuf,
    scratch_root: PathBuf,
    threads: Vec<ConversationSummary>,
    expanded_sections: HashMap<sidebar::Section, bool>,
    current: Option<ConversationDetail>,
    pull_request: Option<crate::github::Status>,
    pull_request_watch: Option<Task<()>>,
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
    launch_workspace: LaunchWorkspace,
    branch: Entity<InputState>,
    launching: Option<ThreadId>,
    name: Entity<InputState>,
    starter: Entity<InputState>,
    starter_attachments: Attachments,
    launch_setup: bool,
    prompt: Entity<InputState>,
    suggestions: suggestions::Suggestions,
    instance: ProviderInstanceId,
    launch_model: Option<String>,
    model_picker: Option<Entity<ListState<Models>>>,
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
                window,
                cx,
            )
        });
        cx.subscribe_in(
            &settings,
            window,
            |view: &mut Self, _, event, window, cx| {
                match event {
                    SettingsEvent::Back => {
                        view.show_settings = false;
                        view.focus_workspace(window, cx);
                    }
                    SettingsEvent::DefaultProvider(id) => {
                        view.choose_provider(id, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
        let instance = settings.read(cx).default_provider();
        let mut view = Self {
            settings,
            show_settings: false,
            navigation: sidebar::Sidebar::new(288.),
            changes_sidebar: sidebar::Sidebar::new(336.),
            focus: cx.focus_handle(),
            workspaces: HashMap::new(),
            runtime,
            executor,
            scratch_root: paths.scratch_dir(),
            selection_file: state.join("selected-thread"),
            threads: Vec::new(),
            expanded_sections: HashMap::new(),
            current: None,
            pull_request: None,
            pull_request_watch: None,
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
            branch: cx.new(|cx| InputState::new(window, cx).placeholder("Generated automatically")),
            launch_workspace: LaunchWorkspace::default(),
            launching: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Name this conversation…")),
            launch_setup: false,
            starter_attachments: Attachments::default(),
            starter: cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(3, 7)
                    .placeholder("Describe a task or ask a question…")
            }),
            prompt: cx.new(|cx| InputState::new(window, cx).auto_grow(2, 5)),
            suggestions: suggestions::Suggestions::default(),
            instance,
            launch_model: None,
            model_picker: None,
            model_catalogs: HashMap::new(),
            checkout_preload: None,
            error: None,
        };
        cx.subscribe(&view.checkout, |view, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                if view.current.is_none() {
                    view.suggestions = Default::default();
                }
                view.launch_model = None;
                view.model_picker = None;
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
        if let Ok(mut revision) = restored {
            let runtime = view.runtime.clone();
            view.thread_updates = Some(cx.spawn(async move |view, cx| {
                loop {
                    let result = runtime.wait_for_conversation_list(revision).await;
                    let done = result.is_err();
                    if view
                        .update(cx, |view, cx| match result {
                            Ok(list) => {
                                revision = list.revision;
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

    fn select(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> huggler::Result<()> {
        self.launching = None;
        self.ensure_workspace(&id, window, cx)?;
        self.workspaces[&id]
            .changes
            .update(cx, |panel, cx| panel.refresh(cx));
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
                            .changes
                            .update(cx, |panel, cx| panel.refresh(cx));
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
                    changes,
                    draft,
                    attachments: Attachments::default(),
                    questions: HashMap::new(),
                },
            );
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
            self.current = Some(thread);
        }
        Ok(())
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        if let Some(paths) = crate::qualification::take_file_selection(cx) {
            if let Some(path) = paths.first() {
                self.choose_checkout(path.clone(), window, cx);
            }
            return;
        }
        let choice = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open checkout".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = choice.await;
            let _ = view.update_in(cx, |v, window, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            v.choose_checkout(path.clone(), window, cx);
                        }
                    }
                    Ok(Err(e)) => v.error = Some(e.to_string()),
                    _ => (),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_checkout(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.checkout.update(cx, |input, cx| {
            input.set_value(path.to_string_lossy().into_owned(), window, cx)
        });
        self.focus_workspace(window, cx);
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.suggestions = Default::default();
        self.model_picker = None;
        self.launch_workspace.close_picker();
        self.show_settings = true;
        self.settings.read(cx).focus.focus(window);
        cx.notify();
    }

    fn focus_workspace(&self, window: &mut Window, cx: &App) {
        if self.show_settings {
            self.settings.read(cx).focus.focus(window);
            return;
        }
        match &self.current {
            None if self.launch_setup && self.launching.is_none() => {
                self.name.focus_handle(cx).focus(window)
            }
            None if self.launching.is_none() => self.starter.focus_handle(cx).focus(window),
            Some(_) => self.prompt.focus_handle(cx).focus(window),
            _ => self.focus.focus(window),
        }
    }

    pub(super) fn focus_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_settings || self.launching.is_some() {
            return;
        }
        let input = match &self.current {
            Some(_) => {
                if window.viewport_size().width < NARROW_VIEWPORT_WIDTH
                    && self.changes_sidebar.is_open(false)
                {
                    self.changes_sidebar.toggle(true);
                }
                &self.prompt
            }
            None if !self.launch_setup => &self.starter,
            None => return,
        };
        self.model_picker = None;
        self.launch_workspace.close_picker();
        input.focus_handle(cx).focus(window);
        cx.notify();
    }

    fn new_conversation(&mut self, setup: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.suggestions = Default::default();
        self.model_picker = None;
        self.launch_workspace.close_picker();
        self.show_settings = false;
        self.launch_setup = setup;
        if self.current.is_none() {
            self.focus_workspace(window, cx);
            cx.notify();
            return;
        }
        self.launch_model = None;
        self.instance = self.settings.read(cx).default_provider();
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

    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launching.is_some() || self.starter_attachments.loading {
            return;
        }
        let message = self.starter.read(cx).value().to_string();
        let explicit_name = self.name.read(cx).value();
        let name = if explicit_name.trim().is_empty() {
            let mut chars = message.trim().lines().next().unwrap_or("").chars();
            let mut title: String = chars.by_ref().take(72).collect();
            if chars.next().is_some() {
                title.push('…');
            }
            if title.is_empty() {
                self.starter_attachments
                    .first_name()
                    .unwrap_or("")
                    .to_owned()
            } else {
                title
            }
        } else {
            explicit_name.trim().to_owned()
        };
        if name.is_empty() {
            return;
        }
        let checkout = self.checkout.read(cx).value();
        let scratch_root = self.scratch_root.clone();
        let branch = self.branch.read(cx).value();
        let workspace_mode = self.launch_workspace.requested_mode();
        let selected_branch = self
            .launch_workspace
            .selected_branch(workspace_mode)
            .cloned();
        let model = self.launch_model.clone();
        let instance = self.instance.clone();
        let runtime = self.runtime.clone();
        let id = ThreadId(Uuid::new_v4().to_string());
        self.launching = Some(id.clone());
        self.launch_workspace.close_picker();
        self.suggestions = Default::default();
        self.focus.focus(window);
        self.error = None;
        cx.notify();
        let thread = id.clone();
        self.starter
            .update(cx, |input, cx| input.set_value("", window, cx));
        let attachments = std::mem::take(&mut self.starter_attachments);
        let first_message = (!message.trim().is_empty() || !attachments.is_empty())
            .then(|| attachments.action(message.clone()));
        let launch_name = name.clone();
        let launch = cx.background_spawn(async move {
            let scratch = checkout.trim().is_empty();
            let source = if scratch {
                fs::create_dir_all(&scratch_root)?;
                runtime.create_scratch(&scratch_root.join(&thread.0))?
            } else {
                runtime.open_project(checkout.trim())?
            };
            let (project, allocated) = launch_workspace::prepare(
                &runtime,
                source,
                if scratch {
                    WorkspaceMode::Current
                } else {
                    workspace_mode
                },
                selected_branch,
                branch.trim(),
                &thread,
            )?;
            if let Err(error) = runtime.dispatch(Command {
                id: CommandId(Uuid::new_v4().to_string()),
                thread_id: thread.clone(),
                action: Action::LaunchInstance {
                    project: project.id.clone(),
                    instance,
                    name,
                    model,
                },
            }) {
                let error = error.into();
                return Err(if allocated {
                    launch_workspace::release_after_failure(&runtime, &project, error)
                } else {
                    error
                });
            }
            let sent = if let Some(action) = first_message {
                runtime
                    .dispatch(Command {
                        id: CommandId(Uuid::new_v4().to_string()),
                        thread_id: thread,
                        action,
                    })
                    .map(|_| ())
            } else {
                Ok(())
            };
            anyhow::Ok(sent)
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = launch.await;
            let _ = view.update_in(cx, |v, w, cx| {
                let still_open = v.launching.as_ref() == Some(&id);
                if still_open {
                    v.launching = None;
                }
                match result {
                    Ok(sent) => {
                        let result = (|| {
                            if sent.is_err() {
                                v.ensure_workspace(&id, w, cx)?;
                                v.workspaces.get_mut(&id).unwrap().attachments = attachments;
                                v.workspaces[&id]
                                    .draft
                                    .update(cx, |input, cx| input.set_value(message, w, cx));
                            }
                            if still_open {
                                v.select(id, w, cx)?;
                            }
                            sent
                        })();
                        v.report(result, cx);
                    }
                    Err(error) => {
                        if v.starter.read(cx).value().is_empty() && v.starter_attachments.is_empty()
                        {
                            v.starter_attachments = attachments;
                            v.starter
                                .update(cx, |input, cx| input.set_value(message, w, cx));
                        }
                        v.error = Some(format!("Could not open {launch_name}. {error}"));
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
        if text.trim().is_empty() && attachments.is_empty() {
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
        self.runtime.dispatch(Command {
            id: CommandId(Uuid::new_v4().to_string()),
            thread_id: thread.id.clone(),
            action: attachments.action(text),
        })?;
        self.workspaces.get_mut(&thread.id).unwrap().attachments = Attachments::default();
        self.prompt
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.refresh()
    }

    fn choose_provider(&mut self, id: &ProviderInstanceId, cx: &mut Context<Self>) {
        if self.instance != *id {
            self.instance.clone_from(id);
            self.launch_model = None;
            if self.current.is_none() {
                self.suggestions = Default::default();
            }
            self.preload_models(cx);
        }
    }

    fn workspace_header(
        &self,
        navigation_open: bool,
        changes_open: bool,
        cx: &Context<Self>,
    ) -> Div {
        let checkout = self.checkout.read(cx).value();
        let (root, title) = match &self.current {
            Some(thread) => (
                self.workspaces[&thread.id].root.as_path(),
                thread.name.as_str(),
            ),
            None => (std::path::Path::new(checkout.as_str()), "Create"),
        };
        let project = root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Workspace");
        let crumb = h_flex()
            .ml_2()
            .flex_1()
            .min_w_0()
            .gap_2()
            .text_sm()
            .child(IconName::Folder)
            .child(
                div()
                    .max_w(relative(0.40))
                    .truncate()
                    .text_color(cx.theme().muted_foreground)
                    .child(project.to_owned()),
            )
            .child(div().text_color(cx.theme().muted_foreground).child("/"))
            .child(div().min_w_0().truncate().child(title.to_owned()));
        h_flex()
            .h_10()
            .flex_shrink_0()
            .px_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .gap_2()
            .child({
                let open = navigation_open;
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
            })
            .when(!self.show_settings, |header| header.child(crumb))
            .when(self.current.is_some() && !self.show_settings, |header| {
                let action = if changes_open {
                    "Hide changes"
                } else {
                    "Show changes"
                };
                header.child(self.project_actions(cx)).child(control!(
                    action,
                    Button::new("toggle-changes")
                        .ghost()
                        .small()
                        .icon(IconName::PanelRight)
                        .selected(changes_open)
                        .tooltip(action)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.changes_sidebar.toggle(changes_open);
                            if !changes_open && let Some(thread) = &view.current {
                                view.workspaces[&thread.id]
                                    .changes
                                    .update(cx, |panel, cx| panel.refresh(cx));
                            }
                            cx.notify();
                        }))
                ))
            })
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

    fn send_icon(id: &'static str, tooltip: &'static str) -> Button {
        Button::new(id)
            .large()
            .w_8()
            .px_0()
            .rounded(px(999.))
            .child(Icon::new(IconName::ArrowUp).with_size(px(16.)))
            .tooltip(tooltip)
    }

    fn launch_form(&self, compact: bool, window: &Window, cx: &Context<Self>) -> AnyElement {
        if self.launching.is_some() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(label("Opening conversation…"))
                .into_any_element();
        }
        let check = self
            .model_catalogs
            .get(&self.model_source(cx))
            .map(|check| check.read(cx));
        let checkout_empty = self.checkout.read(cx).value().trim().is_empty();
        let empty_message =
            self.starter.read(cx).value().trim().is_empty() && self.starter_attachments.is_empty();
        if !self.launch_setup {
            let checkout = self.checkout.read(cx).value();
            let project = std::path::Path::new(checkout.trim())
                .file_name()
                .and_then(|name| name.to_str());
            let heading = project.map_or_else(
                || "What should we build?".into(),
                |name| format!("What should we build in {name}?"),
            );
            let composer = v_flex()
                .on_drop(cx.listener(Self::drop_attachments))
                .w_full()
                .p_3()
                .gap_2()
                .rounded_xl()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted.opacity(0.55))
                .children(self.attachment_list(&self.starter_attachments, cx))
                .child(self.message_input("First message", &self.starter, window, cx))
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(self.model_control(cx))
                        .child(
                            self.send_controls(
                                &self.starter_attachments,
                                "Send first message",
                                Self::send_icon("send-first", "Send (Enter)")
                                    .disabled(empty_message || self.starter_attachments.loading)
                                    .on_click(cx.listener(|v, _, w, cx| v.launch(w, cx))),
                                cx,
                            ),
                        ),
                );
            let body = v_flex()
                .w_full()
                .max_w(px(780.))
                .p_4()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .w_full()
                        .text_center()
                        .text_3xl()
                        .font_weight(FontWeight::MEDIUM)
                        .child(heading),
                )
                .when(!checkout_empty, |body| {
                    body.child(control!(
                        "Start without a project",
                        Button::new("projectless")
                            .small()
                            .ghost()
                            .label("or start without a project")
                            .on_click(cx.listener(|v, _, w, cx| {
                                v.checkout.update(cx, |s, cx| s.set_value("", w, cx));
                                v.branch.update(cx, |s, cx| s.set_value("", w, cx));
                                v.focus_workspace(w, cx);
                            }))
                    ))
                })
                .child(composer)
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .justify_between()
                        .flex_wrap()
                        .child(control!(
                            "Choose project",
                            Button::new("project")
                                .ghost()
                                .small()
                                .icon(IconName::FolderOpen)
                                .child(
                                    div()
                                        .max_w(px(220.))
                                        .truncate()
                                        .child(project.unwrap_or("Choose project").to_owned())
                                )
                                .on_click(cx.listener(|v, _, w, cx| v.browse(w, cx)))
                        ))
                        .when(!checkout_empty, |row| {
                            row.child(self.launch_checkout_controls(cx))
                        }),
                );
            return v_flex()
                .id("starter")
                .flex_1()
                .min_h_0()
                .items_center()
                .overflow_y_scroll()
                .child(body.flex_shrink_0().my_auto())
                .into_any_element();
        }
        let launcher = v_flex()
            .on_drop(cx.listener(Self::drop_attachments))
            .w_full()
            .p_3()
            .gap_3()
            .rounded_xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted.opacity(0.55))
            .children(self.attachment_list(&self.starter_attachments, cx))
            .child(control!(
                "Conversation name",
                Input::new(&self.name).appearance(false)
            ))
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .flex_wrap()
                    .child(self.model_control(cx))
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
                                self.starter_attachments.loading
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
                                    .on_click(cx.listener(|v, _, w, cx| v.browse(w, cx)))
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
            .child(control!(
                "Back to message",
                Button::new("back-to-message")
                    .small()
                    .ghost()
                    .label("Back to message")
                    .on_click(cx.listener(|v, _, w, cx| v.new_conversation(false, w, cx)))
            ))
            .child(launcher)
            .child(context)
            .when(!checkout_empty, |body| {
                body.child(self.launch_checkout_controls(cx))
            })
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
        let empty_draft = attachments.loading
            || (self.prompt.read(cx).value().trim().is_empty() && attachments.is_empty());
        let status = match last_run.map(|run| run.status) {
            Some(huggler::RunStatus::Completed) => None,
            Some(status) => Some(format!("{status:?}")),
            None => Some("Ready".to_owned()),
        };
        let workspace = &self.workspaces[&thread.id];
        let (action_label, action_button) = if can_stop {
            let id = thread.id.clone();
            (
                "Stop thread",
                Button::new("stop-turn")
                    .large()
                    .w(px(36.))
                    .rounded(px(999.))
                    .child(
                        div()
                            .size_3()
                            .rounded_sm()
                            .bg(cx.theme().primary_foreground),
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
        } else {
            let (label, tooltip) = if busy {
                ("Queue message", "Queue (Enter)")
            } else {
                ("Send", "Send (Enter)")
            };
            (
                label,
                Self::send_icon("send", tooltip)
                    .disabled(empty_draft)
                    .on_click(cx.listener(|view, _, window, cx| {
                        let result = view.send(window, cx);
                        view.report(result, cx);
                    })),
            )
        };
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
                    div()
                        .mb_3()
                        .text_center()
                        .text_2xl()
                        .child("What should we build?"),
                )
            })
        .child(
            v_flex()
                .on_drop(cx.listener(Self::drop_attachments))
                .mx_4().mb_3().p_3().rounded_xl().bg(cx.theme().muted.opacity(0.55))
                .gap_2()
                .border_1()
                .border_color(cx.theme().border)
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
                                    let checkout = std::env::current_dir()
                                        .ok()
                                        .filter(|root| root.join(".git").exists())
                                        .unwrap_or_default();
                                    view.checkout.update(cx, |input, cx| {
                                        input.set_value(checkout.to_string_lossy().into_owned(), window, cx)
                                    });
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
                .children(self.attachment_list(attachments, cx))
                .child(self.message_input("Message", &self.prompt, window, cx))
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
                        .child(div().flex_1().min_w_0().when_some(status, |slot, status| slot.child(control!(
                            format!("status/{status}"), label(status)
                        ))))
                        .child(self.send_controls(attachments, action_label, action_button, cx)),
                ),
        )
        .child(div().mx_5().mb_2().text_xs().truncate().text_color(cx.theme().muted_foreground)
            .child(workspace.root.display().to_string()));
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
        let changes_open = self.changes_sidebar.is_open(!narrow);
        let navigation_default = self.current.is_none() || self.show_settings;
        let navigation_open = self.navigation.is_open(navigation_default);
        let left = self.navigation.fraction(navigation_default, window);
        let right = if self.current.is_some() && !self.show_settings {
            self.changes_sidebar.fraction(!narrow, window)
        } else {
            0.
        };
        let left_max =
            SidebarEdge::Left.max_width(viewport, narrow, self.changes_sidebar.width, right);
        let right_max = SidebarEdge::Right.max_width(viewport, narrow, self.navigation.width, left);
        let left_width = self.navigation.width.min(left_max);
        let right_width = self.changes_sidebar.width.min(right_max);
        let conversation_width =
            px(viewport - left_width * left - if narrow { 0. } else { right_width * right });
        div()
            .flex()
            .track_focus(&self.focus)
            .on_action(cx.listener(|v, _: &crate::OpenSettings, w, cx| v.open_settings(w, cx)))
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
                                                    view.open_settings(w, cx)
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
                                .child(self.conversation_sidebar(left_width, cx)),
                        ),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(self.workspace_header(navigation_open, changes_open, cx))
                    .child(if self.show_settings {
                        self.settings.clone().into_any_element()
                    } else {
                        match &self.current {
                            Some(thread) => self
                                .conversation(thread, conversation_width, window, cx)
                                .into_any_element(),
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
                layout.child(SidebarEdge::Left.handle(left_width, narrow, cx))
            })
            .when_some(
                self.current
                    .as_ref()
                    .filter(|_| right > 0. && !self.show_settings),
                |layout, thread| {
                    let tabs = h_flex()
                        .flex_shrink_0()
                        .h_10()
                        .px_2()
                        .gap_1()
                        .child(control!(
                            "Changes tab",
                            label("Changes")
                                .px_2p5()
                                .py_1()
                                .rounded_lg()
                                .bg(cx.theme().sidebar_accent)
                                .text_color(cx.theme().sidebar_accent_foreground)
                        ));
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
                                    .child(tabs)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_h_0()
                                            .child(self.workspaces[&thread.id].changes.clone()),
                                    ),
                            ),
                    )
                },
            )
            .when(
                self.current.is_some() && !self.show_settings && right == 1.,
                |layout| layout.child(SidebarEdge::Right.handle(right_width, narrow, cx)),
            )
            .on_drag_move(
                cx.listener(move |view, event: &DragMoveEvent<SidebarEdge>, _, cx| {
                    let x = f32::from(event.event.position.x - event.bounds.left());
                    match event.drag(cx) {
                        SidebarEdge::Left => view.navigation.width = x.clamp(160., left_max),
                        SidebarEdge::Right => {
                            view.changes_sidebar.width = (viewport - x).clamp(160., right_max)
                        }
                    }
                    cx.notify();
                }),
            )
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}
