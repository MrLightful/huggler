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
    ProviderInstanceId, Role, ThreadId, ThreadSummary,
};
use std::{fs, path::PathBuf, sync::Arc};
use uuid::Uuid;

pub struct Desktop {
    runtime: Arc<Huggler>,
    selection_file: PathBuf,
    threads: Vec<ThreadSummary>,
    current: Option<ConversationDetail>,
    before: Option<HistoryCursor>,
    search: Entity<InputState>,
    query: String,
    subscription: Option<Task<()>>,
    thread_updates: Option<Task<()>>,
    history_scroll: ScrollHandle,
    checkout: Entity<InputState>,
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

// Only the installed qualification build needs names for the painted controls.
macro_rules! control {
    ($name:expr, $child:expr $(,)?) => {{
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        let key = SharedString::from($name);
        let element = div().child($child);
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        let element = crate::qualification::observe(key, element);
        element
    }};
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
            runtime,
            selection_file: state.join("selected-thread"),
            threads: Vec::new(),
            current: None,
            before: None,
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search messages")),
            query: String::new(),
            subscription: None,
            thread_updates: None,
            history_scroll: ScrollHandle::new(),
            checkout: cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/project")),
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
            return Ok(());
        }
        let snapshot = self.runtime.detail(&id, None, "", 32)?;
        let mut cursor = snapshot.history.sequence;
        self.current = Some(snapshot);
        self.before = None;
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.prompt
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
            if self.before.is_none()
                && thread.history.messages.last().map(|m| &m.message.id)
                    != old.history.messages.last().map(|m| &m.message.id)
            {
                self.history_scroll.scroll_to_bottom();
            }
            self.current = Some(thread);
        }
        Ok(())
    }

    fn new_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscription = None;
        self.current = None;
        self.error = None;
        self.name
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.checkout.focus_handle(cx).focus(window);
        cx.notify();
    }

    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) -> huggler::Result<()> {
        let name = self.name.read(cx).value().to_string();
        if name.trim().is_empty() {
            return Err(huggler::Error::Invalid("enter a conversation name".into()));
        }
        let root = self.checkout.read(cx).value();
        let project = self.runtime.open_project(root.as_str())?;
        let id = ThreadId(Uuid::new_v4().to_string());
        self.runtime.dispatch(Command {
            id: CommandId(Uuid::new_v4().to_string()),
            thread_id: id.clone(),
            action: Action::LaunchInstance {
                project: project.id,
                instance: self.instance.clone(),
                name,
                model: None,
            },
        })?;
        self.select(id, window, cx)
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) -> huggler::Result<()> {
        let Some(thread) = &self.current else {
            return Ok(());
        };
        let text = self.prompt.read(cx).value().to_string();
        if text.trim().is_empty() {
            return Err(huggler::Error::Invalid("enter a message".into()));
        }
        self.runtime.send(
            CommandId(Uuid::new_v4().to_string()),
            thread.id.clone(),
            text,
        )?;
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
            .child(field("Git checkout", &self.checkout))
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
            .child(field("Conversation name", &self.name))
            .child(control!(
                "Open conversation",
                Button::new("launch")
                    .primary()
                    .label("Open conversation")
                    .on_click(cx.listener(|view, _, window, cx| {
                        let result = view.launch(window, cx);
                        view.report(result, cx);
                    })),
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

    fn conversation(&self, thread: &ConversationDetail, cx: &Context<Self>) -> Div {
        let provider = self
            .runtime
            .output()
            .provider_label(thread.provider_instance.as_str())
            .to_owned();
        let last_run = thread.latest_run.as_ref();
        let status = last_run.map_or_else(|| "Ready".to_owned(), |run| format!("{:?}", run.status));
        v_flex()
            .size_full()
            .child(
                v_flex()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_lg().child(thread.name.clone()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(provider.clone()),
                    ),
            )
            .child(
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
                                                    Err(error) => {
                                                        view.error = Some(error.to_string())
                                                    }
                                                }
                                                cx.notify();
                                            }
                                        })),
                                ))
                            })
                    })),
            )
            .when(!thread.permissions.is_empty(), |view| {
                view.child(
                    div()
                        .id("permissions")
                        .px_5()
                        .max_h(relative(0.45))
                        .overflow_y_scroll()
                        .children(
                            thread
                                .permissions
                                .iter()
                                .map(|r| Self::permission(thread, r, cx)),
                        ),
                )
            })
            .child(
                v_flex()
                    .p_4()
                    .gap_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .when_some(
                        last_run.and_then(|run| run.prompt_suggestion.clone()),
                        |row, hint| {
                            row.child(control!(
                                "Suggested reply",
                                Button::new("suggestion").label(hint.clone()).on_click(
                                    cx.listener(move |view, _, window, cx| {
                                        if view.prompt.read(cx).value().is_empty() {
                                            view.prompt.update(cx, |input, cx| {
                                                input.set_value(hint.clone(), window, cx)
                                            });
                                            view.prompt.focus_handle(cx).focus(window);
                                        }
                                    })
                                )
                            ))
                        },
                    )
                    .child(field("Message", &self.prompt))
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(control!(
                                format!("status/{status}"),
                                div().text_sm().child(status).when_some(
                                    last_run.and_then(|run| run.error.clone()),
                                    |view, error| view.child(format!(" · {error}"))
                                ),
                            ))
                            .child(control!(
                                "Send",
                                Button::new("send")
                                    .primary()
                                    .label("Send")
                                    .disabled(last_run.is_some_and(|r| !r.status.terminal()))
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        let result = view.send(window, cx);
                                        view.report(result, cx);
                                    })),
                            )),
                    ),
            )
    }
}

impl Render for Desktop {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        crate::qualification::begin_frame(cx);

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
                        None => self.launch_form(cx),
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
