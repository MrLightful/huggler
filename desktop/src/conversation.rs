use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use huggler::{
    Action, Command, CommandId, Huggler, PermissionRequest, Provider, Role, Thread, ThreadId,
    ThreadSummary,
};
use std::{fs, path::PathBuf, sync::Arc};
use uuid::Uuid;

pub struct Desktop {
    runtime: Arc<Huggler>,
    selection_file: PathBuf,
    threads: Vec<ThreadSummary>,
    current: Option<Thread>,
    subscription: Option<Task<()>>,
    history_scroll: ScrollHandle,
    checkout: Entity<InputState>,
    name: Entity<InputState>,
    prompt: Entity<InputState>,
    provider: Provider,
    error: Option<String>,
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

fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "Codex",
        Provider::ClaudeAgent => "Claude Agent",
    }
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
            subscription: None,
            history_scroll: ScrollHandle::new(),
            checkout: cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/project")),
            name: cx.new(|cx| InputState::new(window, cx)),
            prompt: cx.new(|cx| InputState::new(window, cx).auto_grow(2, 5)),
            provider: Provider::Codex,
            error: None,
        };
        let restored = view.restore(window, cx);
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
        let (snapshot, mut stream) = self.runtime.subscribe(&id, None)?;
        self.current = Some(snapshot);
        self.prompt
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.prompt.focus_handle(cx).focus(window);
        self.history_scroll.scroll_to_bottom();
        // Replacing the GPUI task drops the old stream. Re-read once per burst,
        // using the snapshot cursor to skip events already reflected in the view.
        self.subscription = Some(cx.spawn(async move |view, cx| {
            loop {
                let event = stream.next().await;
                let done = event.is_err();
                if view
                    .update(cx, |view, cx| match event {
                        Ok(event)
                            if view.current.as_ref().is_some_and(|thread| {
                                thread.id == event.thread_id && thread.cursor < event.sequence
                            }) =>
                        {
                            if let Err(error) = view.refresh() {
                                view.error = Some(error.to_string());
                            }
                            cx.notify();
                        }
                        Err(error) => view.report(Err(error), cx),
                        _ => {}
                    })
                    .is_err()
                    || done
                {
                    break;
                }
            }
        }));
        fs::write(&self.selection_file, id.0)?;
        Ok(())
    }

    fn refresh(&mut self) -> huggler::Result<()> {
        if let Some(old) = &self.current {
            let thread = self.runtime.read(&old.id)?;
            if thread.messages.len() != old.messages.len() {
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
            action: Action::LaunchProvider {
                project: project.id,
                provider: self.provider,
                name,
                model: None,
            },
        })?;
        self.threads = self.runtime.threads()?;
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

    fn launch_form(&self, cx: &Context<Self>) -> Div {
        v_flex()
            .p_6()
            .gap_4()
            .child(div().text_xl().child("New conversation"))
            .child(field("Git checkout", &self.checkout))
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Provider"))
                    .child(h_flex().gap_2().children(
                        [Provider::Codex, Provider::ClaudeAgent].map(|provider| {
                            let label = provider_name(provider);
                            control!(
                                label,
                                Button::new(label)
                                    .label(label)
                                    .selected(provider == self.provider)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.provider = provider;
                                        cx.notify();
                                    })),
                            )
                        }),
                    )),
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

    fn permission(thread: &Thread, request: &PermissionRequest, cx: &Context<Self>) -> Div {
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

    fn conversation(&self, thread: &Thread, cx: &Context<Self>) -> Div {
        let provider = provider_name(thread.provider);
        let last_run = thread.runs.last();
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
                            .child(provider),
                    ),
            )
            .child(
                div()
                    .id("history")
                    .track_scroll(&self.history_scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_5()
                    .children(thread.messages.iter().map(|message| {
                        let role = if message.role == Role::User {
                            "You"
                        } else {
                            provider
                        };
                        v_flex()
                            .py_3()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(role),
                            )
                            .child(control!(
                                format!("{role}/{}", message.text),
                                message.text.clone(),
                            ))
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
