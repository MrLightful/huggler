mod models;
mod sidebar;

use models::ProviderCatalog;

use crate::{
    FollowUp,
    changes::Changes,
    checks::Checks,
    git,
    publish::Publish,
    questions::Questions,
    settings::{Preferences, Settings, SettingsEvent},
    ui::{choice, field, heading, hint, label, section_title},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Enter, Input, InputEvent, InputState},
    v_flex,
};
use huggler::{
    Action, Command, CommandId, ConversationDetail, ConversationSummary, HistoryCursor, Huggler,
    PermissionRequest, Provider, ProviderInstanceId, Role, RunId, ThreadId,
};
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Conversation,
    Changes,
    Checks,
    Publish,
}

struct Workspace {
    root: PathBuf,
    changes: Entity<Changes>,
    checks: Entity<Checks>,
    publish: Entity<Publish>,
    draft: Entity<InputState>,
    questions: HashMap<huggler::RequestId, Entity<Questions>>,
}

pub struct Desktop {
    settings: Entity<Settings>,
    show_settings: bool,
    focus: FocusHandle,
    tab: Tab,
    workspaces: HashMap<ThreadId, Workspace>,
    runtime: Arc<Huggler>,
    selection_file: PathBuf,
    threads: Vec<ConversationSummary>,
    show_working: bool,
    current: Option<ConversationDetail>,
    checkout_unavailable: bool,
    before: Option<HistoryCursor>,
    search: Entity<InputState>,
    query: String,
    subscription: Option<Task<()>>,
    command_log: Option<CommandLog>,
    thread_updates: Option<Task<()>>,
    history_scroll: ScrollHandle,
    checkout: Entity<InputState>,
    branch: Entity<InputState>,
    launching: Option<ThreadId>,
    name: Entity<InputState>,
    prompt: Entity<InputState>,
    instance: ProviderInstanceId,
    launch_model: Option<String>,
    provider_check: Option<Entity<ProviderCatalog>>,
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
                        view.choose_provider(id);
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
            focus: cx.focus_handle(),
            tab: Tab::Conversation,
            workspaces: HashMap::new(),
            runtime,
            selection_file: state.join("selected-thread"),
            threads: Vec::new(),
            show_working: false,
            current: None,
            checkout_unavailable: false,
            before: None,
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search messages")),
            query: String::new(),
            subscription: None,
            command_log: None,
            thread_updates: None,
            history_scroll: ScrollHandle::new(),
            checkout: cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/project")),
            branch: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Leave blank to use this checkout")
            }),
            launching: None,
            name: cx
                .new(|cx| InputState::new(window, cx).placeholder("e.g. Fix checkout validation")),
            prompt: cx.new(|cx| InputState::new(window, cx).auto_grow(2, 5)),
            instance,
            launch_model: None,
            provider_check: None,
            error: None,
        };
        cx.subscribe(&view.checkout, |view, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                view.launch_model = None;
                view.provider_check = None;
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
            self.checkout.focus_handle(cx).focus(window);
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
        if self.current.as_ref().is_some_and(|thread| thread.id == id) {
            self.focus_workspace(window, cx);
            self.checkout_unavailable = !self.workspaces[&id].root.is_dir();
            return Ok(());
        }
        let snapshot = self.runtime.detail(&id, None, "", 32)?;
        let mut cursor = snapshot.history.sequence;
        if !self.workspaces.contains_key(&id) {
            let root = self.runtime.thread_checkout(&id)?;
            let changes = cx.new(|cx| Changes::new(root.clone(), window, cx));
            let checks = cx.new(|cx| Checks::new(self.runtime.clone(), id.clone(), window, cx));
            let publish = cx.new(|_| Publish::new(root.clone()));
            cx.subscribe_in(&changes, window, |v, _, msg: &FollowUp, w, cx| {
                v.follow_up(msg, w, cx)
            })
            .detach();
            cx.subscribe_in(&checks, window, |v, _, msg: &FollowUp, w, cx| {
                v.follow_up(msg, w, cx)
            })
            .detach();
            let draft = cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(2, 5)
                    .placeholder("Describe a task or ask a question")
            });
            cx.subscribe(&draft, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
            self.workspaces.insert(
                id.clone(),
                Workspace {
                    root,
                    changes,
                    checks,
                    publish,
                    draft,
                    questions: HashMap::new(),
                },
            );
        }
        self.tab = Tab::Conversation;
        self.command_log = None;
        self.checkout_unavailable = !self.workspaces[&id].root.is_dir();
        self.prompt = self.workspaces[&id].draft.clone();
        self.current = Some(snapshot);
        self.before = None;
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
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
                    let result = change.and_then(|_| view.refresh());
                    if let Err(error) = result {
                        view.report(Err(error), cx);
                        return None;
                    }
                    cx.notify();
                    Some(view.current.as_ref()?.history.sequence)
                }) {
                    Ok(Some(next)) => cursor = next,
                    _ => break,
                }
            }
        }));
        fs::write(&self.selection_file, id.0)?;
        Ok(())
    }

    fn refresh(&mut self) -> huggler::Result<()> {
        if let Some(old) = &self.current {
            let thread = match self
                .runtime
                .detail(&old.id, self.before.as_ref(), &self.query, 32)
            {
                Err(huggler::Error::HistoryChanged) => {
                    self.before = None;
                    self.runtime.detail(&old.id, None, &self.query, 32)?
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

    fn search_messages(&mut self, cx: &mut Context<Self>) {
        self.query = self.search.read(cx).value().to_string();
        self.before = None;
        let result = self.refresh();
        self.report(result, cx);
    }

    fn follow_up(&mut self, msg: &FollowUp, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.prompt.read(cx).value();
        let text = if draft.is_empty() {
            msg.0.clone()
        } else {
            format!("{draft}\n\n{}", msg.0)
        };
        self.prompt
            .update(cx, |s, cx| s.set_value(text, window, cx));
        self.tab = Tab::Conversation;
        self.prompt.focus_handle(cx).focus(window);
        cx.notify();
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
                            v.name.focus_handle(cx).focus(window);
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
        self.show_settings = true;
        self.settings.read(cx).focus.focus(window);
        cx.notify();
    }

    fn focus_workspace(&self, window: &mut Window, cx: &App) {
        if self.show_settings {
            self.settings.read(cx).focus.focus(window);
            return;
        }
        match (&self.current, self.tab) {
            (None, _) if self.launching.is_none() => self.checkout.focus_handle(cx).focus(window),
            (Some(_), Tab::Conversation) => self.prompt.focus_handle(cx).focus(window),
            _ => self.focus.focus(window),
        }
    }

    fn new_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_settings = false;
        if self.current.is_none() {
            self.focus_workspace(window, cx);
            cx.notify();
            return;
        }
        self.launch_model = None;
        self.instance = self.settings.read(cx).default_provider();
        self.provider_check = None;
        self.subscription = None;
        self.command_log = None;
        self.current = None;
        self.error = None;
        self.name
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.branch
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.checkout.focus_handle(cx).focus(window);
        cx.notify();
    }

    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launching.is_some() {
            return;
        }
        let name = self.name.read(cx).value().to_string();
        if name.trim().is_empty() {
            self.report(
                Err(huggler::Error::Invalid("enter a conversation name".into())),
                cx,
            );
            return;
        }
        let checkout = self.checkout.read(cx).value();
        if checkout.trim().is_empty() {
            self.report(
                Err(huggler::Error::Invalid("choose a Git checkout".into())),
                cx,
            );
            return;
        }
        let branch = self.branch.read(cx).value();
        let model = self.launch_model.clone();
        let instance = self.instance.clone();
        let runtime = self.runtime.clone();
        let id = ThreadId(Uuid::new_v4().to_string());
        self.launching = Some(id.clone());
        self.focus.focus(window);
        self.error = None;
        cx.notify();
        let thread = id.clone();
        let launch_name = name.clone();
        let launch = cx.background_spawn(async move {
            let source = runtime.open_project(checkout.as_str())?;
            let branch = branch.trim();
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
                thread_id: thread,
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
            anyhow::Ok(())
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = launch.await;
            let _ = view.update_in(cx, |v, w, cx| {
                let still_open = v.launching.as_ref() == Some(&id);
                if still_open {
                    v.launching = None;
                }
                match result {
                    Ok(()) if still_open => {
                        let selected = v.select(id, w, cx);
                        v.report(selected, cx);
                    }
                    Err(error) => {
                        v.error = Some(format!("Could not open {launch_name}. {error}"));
                        if still_open {
                            v.focus_workspace(w, cx);
                        }
                    }
                    Ok(()) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn send(
        &mut self,
        plan: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> huggler::Result<()> {
        let Some(thread) = &self.current else {
            return Ok(());
        };
        let text = self.prompt.read(cx).value().to_string();
        if text.trim().is_empty() {
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
            action: if plan {
                Action::SendPlan { text }
            } else {
                Action::Enqueue { text }
            },
        })?;
        self.prompt
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.refresh()
    }

    fn choose_provider(&mut self, id: &ProviderInstanceId) {
        if self.instance != *id {
            self.instance.clone_from(id);
            self.launch_model = None;
            self.provider_check = None;
        }
    }

    fn launch_form(&self, compact: bool, cx: &Context<Self>) -> Div {
        if self.launching.is_some() {
            return v_flex().p_5().child(label("Opening conversation…"));
        }
        let check = self.provider_check.as_ref().map(|check| check.read(cx));
        let checkout_empty = self.checkout.read(cx).value().trim().is_empty();
        let providers = self.runtime.provider_instances().map(|instance| {
            let id = instance.id.clone();
            let name =
                SharedString::from(self.runtime.output().provider_label(id.as_str()).to_owned());
            control!(
                name.clone(),
                choice(
                    SharedString::from(format!("provider-{id}")),
                    name,
                    id == self.instance
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.choose_provider(&id);
                    cx.notify();
                }))
            )
        });
        let check_button = Button::new("check-provider")
            .label("Check provider")
            .disabled(checkout_empty || check.is_some_and(ProviderCatalog::loading))
            .on_click(cx.listener(|view, _, _, cx| {
                view.catalog(true, cx);
            }));
        let provider_section = v_flex()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(section_title("Coding provider"))
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .flex_wrap()
                    .child(h_flex().gap_1().flex_wrap().children(providers))
                    .child(control!("Check provider", check_button)),
            )
            .child(self.model_control(self.launch_model.as_deref(), checkout_empty, cx))
            .when_some(check, |panel, check| {
                let status = check.status();
                panel.child(control!(
                    format!("provider-status/{status}"),
                    hint(status, cx)
                ))
            });
        let body = v_flex()
            .w_full()
            .max_w(px(720.))
            .p_5()
            .gap_4()
            .when(compact, |form| form.p_3().gap_2())
            .child(heading("New conversation"))
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(field("Git checkout", &self.checkout)),
                    )
                    .child(control!(
                        "Browse",
                        Button::new("browse")
                            .label("Browse…")
                            .on_click(cx.listener(|v, _, w, cx| v.browse(w, cx)))
                    )),
            )
            .child(field("Conversation name", &self.name))
            .child(field("New worktree branch (optional)", &self.branch))
            .child(provider_section);
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .id("launch-form")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(
                h_flex()
                    .px_5()
                    .py_3()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(control!(
                        "Open conversation",
                        Button::new("launch")
                            .primary()
                            .label("Open conversation")
                            .disabled(
                                checkout_empty || self.name.read(cx).value().trim().is_empty()
                            )
                            .on_click(cx.listener(|v, _, w, cx| v.launch(w, cx)))
                    )),
            )
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

    fn conversation(&self, thread: &ConversationDetail, cx: &Context<Self>) -> Div {
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
        let empty_draft = self.prompt.read(cx).value().trim().is_empty();
        let status = last_run.map_or_else(|| "Ready".to_owned(), |run| format!("{:?}", run.status));
        let workspace = &self.workspaces[&thread.id];
        let pending = thread.permissions.len() + thread.questions.len();
        let mut page = v_flex().size_full().min_h_0().child(
            v_flex()
                .px_4()
                .pt_2()
                .gap_1()
                .flex_shrink_0()
                .child(
                    h_flex()
                        .gap_2()
                        .justify_between()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(thread.name.clone()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(provider.clone()),
                        ),
                )
                .child(
                    div()
                        .truncate()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(workspace.root.display().to_string()),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .pb_1()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .children(
                            [
                                (Tab::Conversation, "Conversation"),
                                (Tab::Changes, "Changes"),
                                (Tab::Checks, "Checks"),
                                (Tab::Publish, "Publish"),
                            ]
                            .into_iter()
                            .map(|(tab, label)| {
                                let display = if tab == Tab::Conversation && pending != 0 {
                                    format!("{label} ({pending})")
                                } else {
                                    label.into()
                                };
                                control!(
                                    label,
                                    Button::new(label)
                                        .ghost()
                                        .selected(self.tab == tab)
                                        .compact()
                                        .label(display)
                                        .on_click(cx.listener(move |v, _, w, cx| {
                                            v.tab = tab;
                                            if tab == Tab::Changes
                                                && let Some(t) = &v.current
                                            {
                                                v.workspaces[&t.id]
                                                    .changes
                                                    .update(cx, |panel, cx| panel.refresh(w, cx));
                                            }
                                            if tab == Tab::Publish
                                                && let Some(t) = &v.current
                                            {
                                                v.workspaces[&t.id]
                                                    .publish
                                                    .update(cx, |panel, cx| panel.refresh(cx));
                                            }
                                            cx.notify();
                                        }))
                                )
                            }),
                        ),
                ),
        );
        if self.tab != Tab::Conversation {
            let content = match self.tab {
                Tab::Changes => workspace.changes.clone().into_any_element(),
                Tab::Publish => workspace.publish.clone().into_any_element(),
                _ => workspace.checks.clone().into_any_element(),
            };
            page = page.child(div().flex_1().min_h_0().child(content));
            return page;
        }
        page.child(
            h_flex()
                .px_5()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .on_action(cx.listener(|view, _: &Enter, _, cx| view.search_messages(cx)))
                        .child(control!("Search messages", Input::new(&self.search))),
                )
                .child(control!(
                    "Search",
                    Button::new("search").label("Search").on_click(cx.listener(
                        |view, _, _, cx| {
                            view.search_messages(cx);
                        }
                    ))
                ))
                .when(!self.query.is_empty(), |row| {
                    let clear = Button::new("clear-search")
                        .compact()
                        .label("Clear")
                        .on_click(cx.listener(|v, _, w, cx| {
                            v.search.update(cx, |input, cx| input.set_value("", w, cx));
                            v.search_messages(cx);
                        }));
                    row.child(control!("Clear search", clear))
                })
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
        )
        .child(
            div()
                .id("history")
                .track_scroll(&self.history_scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_5()
                .when(thread.history.messages.is_empty(), |history| {
                    history.child(div().py_4().child(hint(if self.query.is_empty() {
                        "Send a message to start this conversation."
                    } else { "No messages match this search." }, cx)))
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
                    v_flex()
                        .py_3()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(role.to_owned()),
                        )
                        .child(control!(
                            format!("{role}/{}", message.text),
                            message.text.clone(),
                        ))
                        .when(item.truncated, |row| {
                            let id = message.id.clone();
                            row.child(control!(
                                format!("expand/{id}"),
                                Button::new(SharedString::from(format!("full-{id}")))
                                    .label("Show full message")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if let Some(detail) = &mut view.current {
                                            match view.runtime.read_message(&detail.id, &id) {
                                                Ok(message) => {
                                                    if let Some(item) = detail
                                                        .history
                                                        .messages
                                                        .iter_mut()
                                                        .find(|item| item.message.id == id)
                                                    {
                                                        item.message = message;
                                                        item.truncated = false;
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
                .when(self.query.is_empty() && self.before.is_none(), |history| {
                    history.child(self.turn_activity(thread, cx))
                }),
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
                .p_4()
                .gap_2()
                .border_t_1()
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
                                    view.new_conversation(window, cx);
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
                .child(
                    h_flex().justify_between().gap_2()
                        .child(label("Message"))
                        .child(self.model_control(thread.model.as_deref(), false, cx))
                )
                .child(control!("Message", Input::new(&self.prompt)))
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
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(control!(
                            format!("status/{status}"), label(status)
                        )))
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
                        .when(
                            !busy
                                && self.runtime.provider_instances().any(|p| {
                                    p.id == thread.provider_instance && p.driver == Provider::Codex
                                }),
                            |row| {
                                row.child(control!(
                                    "Send plan",
                                    Button::new("send-plan")
                                        .ghost()
                                        .label("Send as plan").disabled(empty_draft)
                                        .tooltip(
                                            "Ask Codex to plan this change in a read-only turn"
                                        )
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            let result = view.send(true, window, cx);
                                            view.report(result, cx);
                                        }))
                                ))
                            },
                        )
                        .child(control!(
                            if busy { "Queue message" } else { "Send" },
                            Button::new("send")
                                .primary()
                                .label(if busy { "Queue message" } else { "Send" }).disabled(empty_draft)
                                .on_click(cx.listener(|view, _, window, cx| {
                                    let result = view.send(false, window, cx);
                                    view.report(result, cx);
                                })),
                        )),
                ),
        )
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

        div()
            .flex()
            .track_focus(&self.focus)
            .on_action(cx.listener(|v, _: &crate::OpenSettings, w, cx| v.open_settings(w, cx)))
            .on_action(
                cx.listener(|v, _: &crate::NewConversation, w, cx| v.new_conversation(w, cx)),
            )
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .w(px(sidebar::WIDTH))
                    .flex_shrink_0()
                    .bg(cx.theme().sidebar)
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .p_3()
                    .gap_3()
                    .child(label("Conversations"))
                    .child(control!(
                        "New conversation",
                        Button::new("new")
                            .w_full()
                            .label("New conversation")
                            .on_click(cx.listener(|view, _, w, cx| view.new_conversation(w, cx))),
                    ))
                    .child(self.conversation_sidebar(cx))
                    .child(
                        div()
                            .pt_2()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .child(control!(
                                "Settings",
                                Button::new("settings")
                                    .w_full()
                                    .justify_start()
                                    .ghost()
                                    .label("Settings")
                                    .selected(self.show_settings)
                                    .on_click(
                                        cx.listener(|view, _, w, cx| view.open_settings(w, cx))
                                    )
                            )),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(if self.show_settings {
                        self.settings.clone().into_any_element()
                    } else {
                        match &self.current {
                            Some(thread) => self.conversation(thread, cx),
                            None => self.launch_form(window.viewport_size().height <= px(480.), cx),
                        }
                        .into_any_element()
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
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}
