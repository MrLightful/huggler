use crate::{
    FollowUp, changes::Changes, checks::Checks, git, publish::Publish, questions::Questions,
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use huggler::{
    Action, Command, CommandId, ConversationDetail, HistoryCursor, Huggler, PermissionRequest,
    Provider, ProviderInstanceId, Role, RunId, ThreadId, ThreadSummary,
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
}

pub struct Desktop {
    tab: Tab,
    workspaces: HashMap<ThreadId, Workspace>,
    questions: HashMap<huggler::RequestId, Entity<Questions>>,
    runtime: Arc<Huggler>,
    selection_file: PathBuf,
    threads: Vec<ThreadSummary>,
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
    launching: bool,
    name: Entity<InputState>,
    prompt: Entity<InputState>,
    instance: ProviderInstanceId,
    provider_check: Option<ProviderCheck>,
    error: Option<String>,
}

struct ProviderCheck {
    id: Uuid,
    result: Option<String>,
}

struct CommandLog {
    run: RunId,
    item: String,
    text: String,
    _reader: Task<()>,
}

fn field(label: &'static str, state: &Entity<InputState>) -> Div {
    v_flex()
        .gap_1()
        .child(div().text_sm().child(label))
        .child(control!(label, Input::new(state)))
}

impl Desktop {
    pub fn new(
        runtime: Arc<Huggler>,
        state: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            tab: Tab::Conversation,
            workspaces: HashMap::new(),
            questions: HashMap::new(),
            runtime,
            selection_file: state.join("selected-thread"),
            threads: Vec::new(),
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
            branch: cx.new(|cx| InputState::new(window, cx).placeholder("Use current checkout")),
            launching: false,
            name: cx.new(|cx| InputState::new(window, cx)),
            prompt: cx.new(|cx| InputState::new(window, cx).auto_grow(2, 5)),
            instance: "codex".into(),
            provider_check: None,
            error: None,
        };
        cx.subscribe(&view.checkout, |view, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                view.provider_check = None;
                cx.notify();
            }
        })
        .detach();
        let restored = view.restore(window, cx);
        if restored.is_ok() {
            let runtime = view.runtime.clone();
            let mut last = view.threads.last().map(|t| t.id.clone());
            view.thread_updates = Some(cx.spawn(async move |view, cx| {
                loop {
                    let result = runtime.wait_for_threads_after(last.as_ref()).await;
                    let done = result.is_err();
                    if view
                        .update(cx, |view, cx| match result {
                            Ok(added) => {
                                last = added.last().map(|t| t.id.clone());
                                view.threads.extend(added);
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
        view.report(restored, cx);
        view
    }

    fn report(&mut self, result: huggler::Result<()>, cx: &mut Context<Self>) {
        self.error = result.err().map(|error| error.to_string());
        cx.notify();
    }

    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) -> huggler::Result<()> {
        self.threads = self.runtime.threads()?;
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
        Ok(())
    }

    fn select(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> huggler::Result<()> {
        if self.current.as_ref().is_some_and(|thread| thread.id == id) {
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
            let draft = cx.new(|cx| InputState::new(window, cx).auto_grow(2, 5));
            self.workspaces.insert(
                id.clone(),
                Workspace {
                    root,
                    changes,
                    checks,
                    publish,
                    draft,
                },
            );
        }
        self.tab = Tab::Conversation;
        self.command_log = None;
        self.checkout_unavailable = !self.workspaces[&id].root.is_dir();
        self.prompt = self.workspaces[&id].draft.clone();
        self.questions.clear();
        self.current = Some(snapshot);
        self.before = None;
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.prompt.focus_handle(cx).focus(window);
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

    fn new_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        if self.launching {
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
        let instance = self.instance.clone();
        let runtime = self.runtime.clone();
        self.launching = true;
        self.error = None;
        cx.notify();
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
            let id = ThreadId(Uuid::new_v4().to_string());
            if let Err(error) = runtime.dispatch(Command {
                id: CommandId(Uuid::new_v4().to_string()),
                thread_id: id.clone(),
                action: Action::LaunchInstance {
                    project: project.id.clone(),
                    instance,
                    name,
                    model: None,
                },
            }) {
                if !branch.is_empty() {
                    runtime.release_workspace(&project.id)?;
                }
                return Err(error.into());
            }
            anyhow::Ok(id)
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = launch.await;
            let _ = view.update_in(cx, |v, w, cx| {
                v.launching = false;
                match result {
                    Ok(id) if v.current.is_none() => {
                        let result = v.select(id, w, cx);
                        v.report(result, cx);
                    }
                    Err(error) => v.error = Some(error.to_string()),
                    _ => {}
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

    fn check_provider(&mut self, cx: &mut Context<Self>) {
        let runtime = self.runtime.clone();
        let instance = self.instance.clone();
        let checkout = self.checkout.read(cx).value().to_string();
        let id = Uuid::new_v4();
        self.provider_check = Some(ProviderCheck { id, result: None });
        cx.notify();
        cx.spawn(async move |view, cx| {
            let status = match runtime.probe_provider(instance.as_str(), &checkout).await {
                Ok(report) => format!(
                    "{} · {} · {} · {} models",
                    report.transport,
                    report.version.as_deref().unwrap_or("version unknown"),
                    if report.authenticated == Some(true) {
                        "signed in"
                    } else {
                        "login unverified"
                    },
                    report.models.len(),
                ),
                Err(error) => error.to_string(),
            };
            let _ = view.update(cx, |view, cx| {
                if let Some(check) = &mut view.provider_check
                    && check.id == id
                {
                    check.result = Some(status);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn launch_form(&self, cx: &Context<Self>) -> Div {
        let check = self.provider_check.as_ref();
        v_flex()
            .p_6()
            .gap_4()
            .child(div().text_xl().child("New conversation"))
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
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Provider"))
                    .child(h_flex().flex_wrap().gap_2().children(
                        self.runtime.provider_instances().map(|instance| {
                            let id = instance.id.clone();
                            let label = SharedString::from(
                                self.runtime.output().provider_label(id.as_str()).to_owned(),
                            );
                            control!(
                                label.clone(),
                                Button::new(SharedString::from(format!("provider-{id}")))
                                    .label(label)
                                    .selected(id == self.instance)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.provider_check = None;
                                        view.instance.clone_from(&id);
                                        cx.notify();
                                    })),
                            )
                        }),
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(control!(
                        "Check provider",
                        Button::new("check-provider")
                            .label("Check provider")
                            .disabled(
                                self.checkout.read(cx).value().trim().is_empty()
                                    || check.is_some_and(|c| c.result.is_none())
                            )
                            .on_click(cx.listener(|view, _, _, cx| view.check_provider(cx)))
                    ))
                    .when_some(check, |row, check| {
                        let status = check
                            .result
                            .as_deref()
                            .unwrap_or("Checking provider…")
                            .to_owned();
                        row.child(control!(
                            format!("provider-status/{status}"),
                            div().text_sm().child(status)
                        ))
                    }),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(field("Conversation name", &self.name)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(field("New worktree branch (optional)", &self.branch)),
                    ),
            )
            .child(control!(
                "Open conversation",
                Button::new("launch")
                    .primary()
                    .label(if self.launching {
                        "Opening…"
                    } else {
                        "Open conversation"
                    })
                    .disabled(self.launching)
                    .on_click(cx.listener(|view, _, window, cx| view.launch(window, cx))),
            ))
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
                        .child(control!("Search messages", Input::new(&self.search))),
                )
                .child(control!(
                    "Search",
                    Button::new("search").label("Search").on_click(cx.listener(
                        |view, _, _, cx| {
                            view.query = view.search.read(cx).value().to_string();
                            view.before = None;
                            let result = view.refresh();
                            view.report(result, cx);
                        }
                    ))
                ))
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
                            .filter_map(|q| self.questions.get(&q.id).cloned()),
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
                .child(field("Message", &self.prompt))
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
                        .justify_between()
                        .gap_2()
                        .child(control!(
                            format!("status/{status}"),
                            div().text_sm().child(status).when_some(
                                last_run.filter(|_| !self.checkout_unavailable).and_then(|run| run.error.clone()),
                                |view, error| view.child(format!(" · {error}"))
                            ),
                        ))
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
                                        .label("Send as plan")
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
                                .label(if busy { "Queue message" } else { "Send" })
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
            self.questions
                .retain(|id, _| thread.questions.iter().any(|q| q.id == *id));
            for q in &thread.questions {
                self.questions.entry(q.id.clone()).or_insert_with(|| {
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
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .w(px(200.))
                    .flex_shrink_0()
                    .bg(cx.theme().sidebar)
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .p_3()
                    .gap_3()
                    .child(div().text_sm().child("Conversations"))
                    .child(control!(
                        "New conversation",
                        Button::new("new")
                            .label("New conversation")
                            .on_click(cx.listener(|view, _, w, cx| view.new_conversation(w, cx))),
                    ))
                    .child(
                        div()
                            .id("threads")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .children(self.threads.iter().map(|thread| {
                                let id = thread.id.clone();
                                let selected = self.current.as_ref().is_some_and(|t| t.id == id);
                                control!(
                                    format!("thread/{}", thread.name),
                                    Button::new(SharedString::from(id.0.clone()))
                                        .label(thread.name.clone())
                                        .ghost()
                                        .selected(selected)
                                        .on_click(cx.listener(move |view, _, w, cx| {
                                            let result = view.select(id.clone(), w, cx);
                                            view.report(result, cx);
                                        })),
                                )
                            })),
                    )
                    .child(control!(
                        "Quit",
                        Button::new("quit")
                            .ghost()
                            .label("Quit")
                            .on_click(|_, _, cx| cx.quit()),
                    )),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(match &self.current {
                        Some(thread) => self.conversation(thread, cx),
                        None => div().size_full().child(
                            div()
                                .id("launch-form")
                                .size_full()
                                .overflow_y_scroll()
                                .child(self.launch_form(cx)),
                        ),
                    })
                    .when_some(self.error.clone(), |view, error| {
                        view.child(control!(
                            format!("error/{error}"),
                            div()
                                .px_4()
                                .py_2()
                                .text_color(cx.theme().danger)
                                .child(error),
                        ))
                    }),
            )
    }
}
