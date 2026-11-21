mod attachments;
mod models;
mod project_actions;
mod sidebar;
mod suggestions;

use attachments::Attachments;
use gpui_component::list::ListState;
use models::{Models, ProviderCatalog};

use crate::{
    changes::Changes,
    git,
    questions::Questions,
    settings::{Preferences, Settings, SettingsEvent},
    ui::{field, hint, label},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Enter, Input, InputEvent, InputState},
    v_flex,
};
use huggler::{
    Action, Command, CommandId, ConversationDetail, ConversationSummary, HistoryCursor, Huggler,
    PermissionRequest, ProviderInstanceId, Role, RunId, ThreadId,
};
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};
use uuid::Uuid;

const NARROW_VIEWPORT_WIDTH: Pixels = px(900.);

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
    selection_file: PathBuf,
    scratch_root: PathBuf,
    threads: Vec<ConversationSummary>,
    expanded_sections: HashMap<sidebar::Section, bool>,
    current: Option<ConversationDetail>,
    checkout_unavailable: bool,
    before: Option<HistoryCursor>,
    subscription: Option<Task<()>>,
    command_log: Option<CommandLog>,
    file_preview: Option<((huggler::MessageId, usize), String)>,
    thread_updates: Option<Task<()>>,
    history_scroll: ScrollHandle,
    checkout: Entity<InputState>,
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
    text: String,
    _reader: Task<()>,
}

impl Desktop {
    pub fn new(
        runtime: Arc<Huggler>,
        state: PathBuf,
        prefs: Preferences,
        load_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
            navigation: sidebar::Sidebar::new(256.),
            changes_sidebar: sidebar::Sidebar::new(300.),
            focus: cx.focus_handle(),
            workspaces: HashMap::new(),
            runtime,
            scratch_root: state.join("scratch"),
            selection_file: state.join("selected-thread"),
            threads: Vec::new(),
            expanded_sections: HashMap::new(),
            current: None,
            checkout_unavailable: false,
            before: None,
            subscription: None,
            command_log: None,
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
            branch: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Leave blank to use this checkout")
            }),
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
            return Ok(());
        }
        let snapshot = self.runtime.detail(&id, None, "", 32)?;
        let mut cursor = snapshot.history.sequence;
        self.file_preview = None;
        self.command_log = None;
        self.checkout_unavailable = !self.workspaces[&id].root.is_dir();
        self.prompt = self.workspaces[&id].draft.clone();
        self.suggestions = Default::default();
        self.model_picker = None;
        self.current = Some(snapshot);
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
                    if finished || latest_finished {
                        view.workspaces[&selected]
                            .changes
                            .update(cx, |panel, cx| panel.refresh(cx));
                    }
                    Some(thread.history.sequence)
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
            let changes = cx.new(|_| Changes::new(root.clone()));
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
            let at_bottom =
                self.history_scroll.max_offset().height + self.history_scroll.offset().y <= px(2.);
            if self.before.is_none()
                && at_bottom
                && (thread.history.messages.last().map(|m| &m.message.id)
                    != old.history.messages.last().map(|m| &m.message.id)
                    || thread.streaming_text != old.streaming_text
                    || thread.command_items != old.command_items)
            {
                self.history_scroll.scroll_to_bottom();
            }
            if self.command_log.as_ref().is_some_and(|log| {
                thread
                    .foreground_run
                    .as_ref()
                    .or(thread.latest_run.as_ref())
                    .is_none_or(|run| run.id != log.run)
            }) {
                self.command_log = None;
            }
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
                            v.checkout.update(cx, |s, cx| {
                                s.set_value(path.to_string_lossy().into_owned(), window, cx)
                            });
                            v.focus_workspace(window, cx);
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

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.suggestions = Default::default();
        self.model_picker = None;
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
        input.focus_handle(cx).focus(window);
        cx.notify();
    }

    fn new_conversation(&mut self, setup: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.suggestions = Default::default();
        self.model_picker = None;
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
        let model = self.launch_model.clone();
        let instance = self.instance.clone();
        let runtime = self.runtime.clone();
        let id = ThreadId(Uuid::new_v4().to_string());
        self.launching = Some(id.clone());
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
            let branch = if scratch { "" } else { branch.trim() };
            let project = if branch.is_empty() {
                source
            } else {
                git::validate_new_branch(&source.root, branch)?;
                let project = runtime.allocate_workspace(&source.id)?;
                if let Err(error) = git::create_branch(&project.root, branch) {
                    runtime.release_workspace(&project.id)?;
                    return Err(error);
                }
                project
            };
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
                if !branch.is_empty() {
                    runtime.release_workspace(&project.id)?;
                }
                return Err(error.into());
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
            return Err(huggler::Error::Invalid("enter a message".into()));
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

    fn workspace_header(&self, changes_open: bool, cx: &Context<Self>) -> Div {
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
                let open = self.navigation.is_open(true);
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
            .rounded(px(999.))
            .icon(IconName::ArrowUp)
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
                .capture_action(cx.listener(|view, action: &Enter, window, cx| {
                    if action.secondary {
                        cx.stop_propagation();
                        if !view.starter.read(cx).value().trim().is_empty()
                            || !view.starter_attachments.is_empty()
                        {
                            view.launch(window, cx);
                        }
                    }
                }))
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
                                Self::send_icon("send-first", "Send first message (⌘Enter)")
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
                .child(h_flex().w_full().justify_between().child(control!(
                            "Choose project",
                            Button::new("project")
                                .ghost()
                                .small()
                                .icon(IconName::FolderOpen)
                                .label(project.unwrap_or("Choose project").to_owned())
                                .on_click(cx.listener(|v, _, w, cx| v.browse(w, cx)))
                        )));
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
                                Self::send_icon("launch", "Send first message")
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
                                    .tooltip("Browse for a checkout")
                                    .on_click(cx.listener(|v, _, w, cx| v.browse(w, cx)))
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(compact, |field| field.w_full().flex_none())
                    .child(field("New worktree branch (optional)", &self.branch)),
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

    fn permission(
        thread: &ConversationDetail,
        request: &PermissionRequest,
        cx: &Context<Self>,
    ) -> Div {
        let mut title = request.tool_call["title"]
            .as_str()
            .unwrap_or("Tool permission")
            .to_owned();
        if let Some(origin) = &request.native_child {
            let path = thread
                .native_children
                .iter()
                .find(|child| child.run_id == request.run_id && child.thread_id == origin.thread_id)
                .map_or(origin.thread_id.0.as_str(), |child| child.path.as_str());
            title.push_str(&format!(" ({path})"));
        }
        let details = request
            .tool_call
            .get("rawInput")
            .unwrap_or(&request.tool_call);
        v_flex()
            .gap_2()
            .p_3()
            .border_1()
            .rounded_md()
            .border_color(cx.theme().border)
            .child(control!(format!("permission-title/{title}"), title))
            .child(
                div()
                    .id(SharedString::from(format!("details-{}", request.id)))
                    .max_h(px(96.))
                    .overflow_y_scroll()
                    .text_sm()
                    .child(format!("{details:#}")),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .children(request.options.iter().map(|choice| {
                        let thread = thread.id.clone();
                        let run = request.run_id.clone();
                        let request = request.id.clone();
                        let option = choice.option_id.clone();
                        control!(
                            format!("permission/{}", choice.kind),
                            Button::new(SharedString::from(format!("{request}/{option}")))
                                .label(choice.name.clone())
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    let result = view
                                        .runtime
                                        .respond_to_permission(
                                            &thread,
                                            &run,
                                            &request,
                                            option.clone(),
                                        )
                                        .and_then(|_| view.refresh());
                                    view.report(result, cx);
                                })),
                        )
                    })),
            )
    }

    fn toggle_command(
        &mut self,
        thread: ThreadId,
        run: RunId,
        item: String,
        cx: &mut Context<Self>,
    ) {
        let previous = self.command_log.take();
        if previous
            .as_ref()
            .is_some_and(|log| log.run == run && log.item == item)
        {
            cx.notify();
            return;
        }
        let runtime = self.runtime.clone();
        let command = item.clone();
        let owner = run.clone();
        let reader = cx.spawn(async move |view, cx| {
            let mut revision = None;
            loop {
                let result = runtime
                    .command_output(&thread, &owner, &command, revision)
                    .await;
                let mut done = true;
                if view
                    .update(cx, |view, cx| {
                        let Some(log) = &mut view.command_log else {
                            return;
                        };
                        if log.run != owner || log.item != command {
                            return;
                        }
                        match result {
                            Ok(output) => {
                                revision = Some(output.revision);
                                done = output.closed;
                                log.text = if output.truncated {
                                    format!("… earlier output omitted\n{}", output.text)
                                } else {
                                    output.text
                                };
                            }
                            Err(error) => log.text = error.to_string(),
                        }
                        cx.notify();
                    })
                    .is_err()
                    || done
                {
                    break;
                }
            }
        });
        self.command_log = Some(CommandLog {
            run,
            item,
            text: String::new(),
            _reader: reader,
        });
        cx.notify();
    }

    fn turn_activity(&self, thread: &ConversationDetail, cx: &Context<Self>) -> Div {
        let mut activity = v_flex().gap_1();
        for child in &thread.native_children {
            activity = activity.child(
                div()
                    .text_sm()
                    .child(format!("{} · {:?}", child.path, child.status)),
            );
        }
        if !thread.streaming_text.is_empty() {
            activity = activity
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Writing"),
                )
                .child(control!(
                    format!("writing/{}", thread.streaming_text),
                    thread.streaming_text.clone()
                ));
        }
        if let Some(run) = thread
            .foreground_run
            .as_ref()
            .or(thread.latest_run.as_ref())
        {
            for (i, item) in thread.command_items.iter().enumerate() {
                let run = run.id.clone();
                let thread = thread.id.clone();
                let item = item.clone();
                let log = self
                    .command_log
                    .as_ref()
                    .filter(|log| log.run == run && log.item == item);
                let label = format!("Command {} output", i + 1);
                activity = activity
                    .child(control!(
                        label.clone(),
                        Button::new(SharedString::from(format!("command-{i}")))
                            .ghost()
                            .label(label)
                            .selected(log.is_some())
                            .on_click(cx.listener(move |view, _, _, cx| view.toggle_command(
                                thread.clone(),
                                run.clone(),
                                item.clone(),
                                cx
                            )))
                    ))
                    .when_some(log, |panel, log| {
                        panel.child(
                            div()
                                .p_2()
                                .bg(cx.theme().sidebar)
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_sm()
                                .child(control!(
                                    format!("command-output/{}", log.text),
                                    log.text.clone()
                                )),
                        )
                    });
            }
        }
        activity
    }

    fn conversation(
        &self,
        thread: &ConversationDetail,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        let provider = self
            .runtime
            .output()
            .provider_label(thread.provider_instance.as_str())
            .to_owned();
        let last_run = thread
            .foreground_run
            .as_ref()
            .or(thread.latest_run.as_ref());
        let busy = thread.foreground_run.is_some() || thread.queued_count > 0;
        let attachments = &self.workspaces[&thread.id].attachments;
        let empty_draft = attachments.loading
            || (self.prompt.read(cx).value().trim().is_empty() && attachments.is_empty());
        let status = match last_run.map(|run| run.status) {
            Some(huggler::RunStatus::Completed) => None,
            Some(status) => Some(format!("{status:?}")),
            None => Some("Ready".to_owned()),
        };
        let workspace = &self.workspaces[&thread.id];
        let pending = thread.permissions.len() + thread.questions.len();
        let page = v_flex().flex_1().min_h_0();
        let fresh = last_run.is_none()
            && thread.history.messages.is_empty()
            && self.before.is_none()
            && pending == 0
            && !busy;
        let column = v_flex()
            .w_full()
            .max_w(px(800.))
            .mx_auto()
            .flex_1()
            .min_h_0()
            .when(fresh, |column| column.justify_center())
            .when(fresh, |column| {
                column.child(
                    div()
                        .mb_3()
                        .text_center()
                        .text_2xl()
                        .child("What should we build?"),
                )
            });
        let column = column.when(!fresh, |column| column
        .when(thread.history.older.is_some() || self.before.is_some(), |column| column.child(
            h_flex()
                .px_5()
                .gap_2()
                .when_some(thread.history.older.clone(), |row, older| {
                    row.child(control!(
                        "Older",
                        Button::new("older").label("Older").on_click(cx.listener(
                            move |view, _, _, cx| {
                                view.before = Some(older.clone());
                                let result = view.refresh();
                                view.report(result, cx);
                            }
                        ))
                    ))
                })
                .when(self.before.is_some(), |row| {
                    row.child(control!(
                        "Newest",
                        Button::new("newest").label("Newest").on_click(cx.listener(
                            |view, _, _, cx| {
                                view.before = None;
                                let result = view.refresh();
                                view.report(result, cx);
                            }
                        ))
                    ))
                }),
        ))
        .child(
            div()
                .id("history")
                .track_scroll(&self.history_scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_5()
                .when(thread.history.messages.is_empty(), |history| {
                    history.child(div().py_4().child(hint("Send a message to start this conversation.", cx)))
                })
                .children(thread.history.messages.iter().map(|item| {
                    let message = &item.message;
                    let role = match (&message.origin, message.role) {
                        (Some(sender), _) => self
                            .threads
                            .iter()
                            .find(|t| t.id == sender.thread_id)
                            .map_or(sender.thread_id.0.as_str(), |t| t.name.as_str()),
                        (None, Role::User) => "You",
                        (None, Role::Assistant) => provider.as_str(),
                    };
                    let caption = item.files.as_ref().map_or(message.text.as_str(), |files| &files.caption);
                    v_flex()
                        .py_3()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(role.to_owned()),
                        )
                        .when(!caption.is_empty(), |row| row.child(control!(
                            format!("{role}/{caption}"), caption.to_owned(),
                        )))
                        .when_some(item.files.as_ref(), |row, files| row.children(
                            files.names.iter().enumerate().map(|(index, name)| self.sent_file(&thread.id, &message.id, index, name, cx))
                        ))
                        .when(item.files.as_ref().map_or(item.truncated, |files| files.truncated), |row| {
                            let id = message.id.clone();
                            row.child(control!(
                                format!("expand/{id}"),
                                Button::new(SharedString::from(format!("full-{id}")))
                                    .label("Show full message")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if let Some(detail) = &mut view.current {
                                            match view.runtime.read_message_body(&detail.id, &id) {
                                                Ok(message) => {
                                                    if let Some(item) = detail
                                                        .history
                                                        .messages
                                                        .iter_mut()
                                                        .find(|item| item.message.id == id)
                                                    {
                                                        if let Some(files) = &mut item.files {
                                                            files.caption = message;
                                                            files.truncated = false;
                                                        } else {
                                                            item.message.text = message;
                                                            item.truncated = false;
                                                        }
                                                    }
                                                }
                                                Err(error) => view.error = Some(error.to_string()),
                                            }
                                            cx.notify();
                                        }
                                    })),
                            ))
                        })
                }))
                .when(self.before.is_none(), |history| {
                    history.child(self.turn_activity(thread, cx))
                }),
        )
        )
        .when(pending != 0, |view| {
            view.child(
                div()
                    .id("permissions")
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
        })
        .child(
            v_flex()
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
                        .when(
                            thread.foreground_run.is_some()
                                || (thread.queued_count > 0 && !thread.queue_held),
                            |row| {
                                let id = thread.id.clone();
                                row.child(control!(
                                    "Stop thread",
                                    Button::new("stop-turn")
                                        .label("Stop thread")
                                        .tooltip("Stop this thread and its delegated work")
                                        .on_click(cx.listener(move |v, _, _, cx| {
                                            let result = v
                                                .runtime
                                                .stop_thread(
                                                    CommandId(Uuid::new_v4().to_string()),
                                                    id.clone(),
                                                )
                                                .map(|_| ());
                                            v.report(result, cx);
                                        }))
                                ))
                            },
                        )
                        .child(self.send_controls(
                            attachments,
                            if busy { "Queue message" } else { "Send" },
                            Self::send_icon("send", if busy { "Queue message" } else { "Send" }).disabled(empty_draft)
                                .on_click(cx.listener(|view, _, window, cx| {
                                    let result = view.send(window, cx);
                                    view.report(result, cx);
                                })),
                            cx,
                        )),
                ),
        )
        .child(div().mx_5().mb_2().text_xs().truncate().text_color(cx.theme().muted_foreground)
            .child(workspace.root.display().to_string()));
        page.child(column)
    }
}

impl Render for Desktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        crate::qualification::begin_frame(cx);

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
        let left = self.navigation.fraction(true, window);
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
                                .border_color(cx.theme().border)
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
                                .child(div().w_full().h_px().flex_shrink_0().bg(cx.theme().border))
                                .child(self.conversation_sidebar(left_width, cx)),
                        ),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(self.workspace_header(changes_open, cx))
                    .child(if self.show_settings {
                        self.settings.clone().into_any_element()
                    } else {
                        match &self.current {
                            Some(thread) => {
                                self.conversation(thread, window, cx).into_any_element()
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
                layout.child(SidebarEdge::Left.handle(left_width, narrow, cx))
            })
            .when_some(
                self.current
                    .as_ref()
                    .filter(|_| right > 0. && !self.show_settings),
                |layout, thread| {
                    layout.child(
                        div()
                            .w(px(right_width * right))
                            .overflow_hidden()
                            .flex_shrink_0()
                            .when(narrow, |panel| {
                                panel.absolute().right_0().top_10().bottom_0().occlude()
                            })
                            .child(
                                div()
                                    .w(px(right_width))
                                    .h_full()
                                    .border_l_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().sidebar)
                                    .child(self.workspaces[&thread.id].changes.clone()),
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
