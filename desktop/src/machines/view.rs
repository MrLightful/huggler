use super::ssh::{Connection, SavedMachine};
use crate::{
    conversation::{
        markdown_view::{ResponseImages, prose},
        permission_card,
    },
    ui::{field, heading, hint, label},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use huggler::{
    ConversationDetail, ConversationSummary, HistoryItem, MessageId, RequestId, Role, RunId,
    ThreadId,
    machine::{Client, ClientError, Reply, Request},
};
use std::{collections::HashMap, time::Duration};
use tokio::sync::watch;
use uuid::Uuid;

pub(crate) struct RemoteProject {
    machine: SavedMachine,
    executor: tokio::runtime::Handle,
    connection: Option<Task<()>>,
    forward: Option<Connection>,
    threads: Vec<ConversationSummary>,
    selected: Option<ThreadId>,
    selection_changed: watch::Sender<()>,
    detail: Option<Box<ConversationDetail>>,
    name: Entity<InputState>,
    message: Entity<InputState>,
    drafts: HashMap<ThreadId, String>,
    full_messages: HashMap<MessageId, String>,
    questions: HashMap<RequestId, Entity<crate::questions::Questions>>,
    error: Option<String>,
    status: &'static str,
    pending: Option<Request>,
    working: bool,
    older: Vec<huggler::HistoryItem>,
    before: Option<huggler::HistoryCursor>,
    history_scroll: ScrollHandle,
}

impl RemoteProject {
    pub fn new(
        machine: SavedMachine,
        executor: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            machine,
            executor,
            connection: None,
            forward: None,
            threads: vec![],
            selected: None,
            selection_changed: watch::channel(()).0,
            detail: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Conversation name")),
            message: cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(2, 5)
                    .placeholder("Message Codex…")
            }),
            drafts: HashMap::new(),
            full_messages: HashMap::new(),
            questions: HashMap::new(),
            error: None,
            status: "Disconnected",
            pending: None,
            working: false,
            older: vec![],
            before: None,
            history_scroll: ScrollHandle::new(),
        };
        cx.on_app_quit(|view, _| {
            view.disconnect();
            std::future::ready(())
        })
        .detach();
        view.connect(window, cx);
        view
    }

    pub fn set_machine(
        &mut self,
        machine: SavedMachine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if machine != self.machine {
            self.disconnect();
            self.machine = machine;
            self.connect(window, cx);
        }
    }

    fn disconnect(&mut self) {
        self.forward = None;
        self.connection = None;
        self.questions.clear();
        self.status = "Disconnected";
    }

    fn client(&self) -> Option<&Client> {
        self.forward.as_ref().map(|forward| &forward.client)
    }

    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection.is_some() {
            return;
        }
        self.status = "Connecting…";
        self.error = None;
        let machine = self.machine.clone();
        let executor = self.executor.clone();
        let mut selection = self.selection_changed.subscribe();
        self.connection = Some(cx.spawn_in(window, async move |view, cx| {
            let mut was_connected = false;
            loop {
                let target = machine.target.clone();
                let expected = machine.environment.clone();
                let open =
                    executor.spawn(async move { Connection::open(&target, Some(&expected)).await });
                // Dropping a view or disconnecting also cancels a pending OpenSSH bootstrap.
                let abort = AbortOnDrop(open.abort_handle());
                let opened = open.await;
                drop(abort);
                let connection = match opened {
                    Ok(Ok(connection)) => connection,
                    Err(error) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.connection_failed(error.to_string(), cx)
                        });
                        break;
                    }
                    Ok(Err(error)) => {
                        let message = error.to_string();
                        if !was_connected || message.contains("different Huggler environment") {
                            let _ = view
                                .update_in(cx, |view, _, cx| view.connection_failed(message, cx));
                            break;
                        }
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.error = Some(message);
                            cx.notify();
                        });
                        cx.background_executor().timer(Duration::from_secs(2)).await;
                        continue;
                    }
                };
                was_connected = true;
                let client = connection.client.clone();
                if view
                    .update_in(cx, |view, _, cx| {
                        view.forward = Some(connection);
                        view.status = "Connected";
                        if view.pending.is_none() {
                            view.error = None;
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                let mut fresh_connection = true;
                let error = loop {
                    selection.borrow_and_update();
                    let Ok((selected, after)) = view.update_in(cx, |view, _, _| {
                        (
                            view.selected.clone(),
                            view.detail.as_ref().map(|detail| detail.history.sequence),
                        )
                    }) else {
                        return;
                    };
                    let after = if std::mem::take(&mut fresh_connection) {
                        None
                    } else {
                        after
                    };
                    let client = client.clone();
                    let project = machine.project.clone();
                    let fetch_thread = selected.clone();
                    let mut poll = executor.spawn(async move {
                        let list = client
                            .request(Request::List {
                                project: project.clone(),
                            })
                            .await?;
                        let Reply::Conversations(list) = list else {
                            return Err(ClientError::Protocol);
                        };
                        let detail = if let Some(thread) =
                            fetch_thread.filter(|id| list.iter().any(|row| row.id == *id))
                        {
                            let request = Request::Detail {
                                project: project.clone(),
                                thread: thread.clone(),
                                after,
                                before: None,
                            };
                            let response = match client.request(request).await {
                                Err(ClientError::Rejected(_)) if after.is_some() => {
                                    client
                                        .request(Request::Detail {
                                            project,
                                            thread,
                                            after: None,
                                            before: None,
                                        })
                                        .await?
                                }
                                other => other?,
                            };
                            match response {
                                Reply::Detail(detail) => Some(detail),
                                Reply::Unchanged => None,
                                _ => return Err(ClientError::Protocol),
                            }
                        } else {
                            None
                        };
                        Ok((list, detail))
                    });
                    let outcome = tokio::select! {
                        result = &mut poll => result,
                        _ = selection.changed() => {
                            poll.abort();
                            continue;
                        }
                    };
                    match outcome {
                        Ok(Ok((list, detail))) => {
                            if view
                                .update_in(cx, |view, window, cx| {
                                    view.threads = list;
                                    if view.selected == selected {
                                        if let Some(detail) = detail {
                                            view.set_detail(detail, window, cx);
                                        } else if selected.as_ref().is_some_and(|id| {
                                            !view.threads.iter().any(|row| row.id == *id)
                                        }) && view.detail.is_some()
                                        {
                                            view.select(None, window, cx);
                                        }
                                    }
                                    cx.notify();
                                })
                                .is_err()
                            {
                                return;
                            }
                        }
                        Ok(Err(ClientError::Connection(error))) => break error.to_string(),
                        Ok(Err(error)) => {
                            let _ = view.update_in(cx, |view, _, cx| {
                                view.connection_failed(error.to_string(), cx)
                            });
                            return;
                        }
                        Err(error) => break error.to_string(),
                    }
                    tokio::select! {
                        _ = cx.background_executor().timer(Duration::from_millis(350)) => (),
                        _ = selection.changed() => (),
                    }
                };
                if view
                    .update_in(cx, |view, _, cx| {
                        view.forward = None;
                        view.questions.clear();
                        view.status = "Reconnecting…";
                        if view.pending.is_none() {
                            view.error = Some(error);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        }));
        cx.notify();
    }

    fn connection_failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.disconnect();
        self.error = Some(error);
        cx.notify();
    }

    fn select(&mut self, selected: Option<ThreadId>, window: &mut Window, cx: &mut Context<Self>) {
        if selected == self.selected {
            return;
        }
        if let Some(previous) = self.selected.take() {
            let draft = self.message.read(cx).value();
            if !draft.is_empty() {
                self.drafts.insert(previous, draft.to_string());
            }
        }
        let draft = selected
            .as_ref()
            .and_then(|id| self.drafts.remove(id))
            .unwrap_or_default();
        self.message
            .update(cx, |input, cx| input.set_value(draft, window, cx));
        self.selected = selected;
        self.selection_changed.send_replace(());
        self.detail = None;
        self.questions.clear();
        self.older.clear();
        self.before = None;
        self.history_scroll = ScrollHandle::new();
        cx.notify();
    }

    fn set_detail(
        &mut self,
        detail: Box<ConversationDetail>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let follow =
            self.history_scroll.max_offset().height + self.history_scroll.offset().y <= px(2.);
        if self
            .detail
            .as_ref()
            .is_some_and(|current| current.history.sequence != detail.history.sequence)
        {
            self.older.clear();
            self.before = None;
        }
        self.questions
            .retain(|id, _| detail.questions.iter().any(|q| q.id == *id));
        if let Some(client) = self.client().cloned() {
            for question in &detail.questions {
                self.questions
                    .entry(question.id.clone())
                    .or_insert_with(|| {
                        cx.new(|cx| {
                            crate::questions::Questions::remote(
                                client.clone(),
                                self.executor.clone(),
                                self.machine.project.clone(),
                                detail.id.clone(),
                                question.clone(),
                                window,
                                cx,
                            )
                        })
                    });
            }
        }
        self.older.retain(|item| {
            !detail
                .history
                .messages
                .iter()
                .any(|new| new.message.id == item.message.id)
        });
        if self.older.is_empty() {
            self.before = detail.history.older.clone();
        }
        self.detail = Some(detail);
        if follow {
            self.history_scroll.scroll_to_bottom();
        }
    }

    fn older(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(thread), Some(before)) = (
            self.client().cloned(),
            self.selected.clone(),
            self.before.clone(),
        ) else {
            return;
        };
        let request = Request::Detail {
            project: self.machine.project.clone(),
            thread: thread.clone(),
            after: None,
            before: Some(before),
        };
        let fetch = self
            .executor
            .spawn(async move { client.request(request).await });
        self.working = true;
        cx.notify();
        cx.spawn(async move |view, cx| {
            let result = fetch.await;
            let _ = view.update(cx, |view, cx| {
                view.working = false;
                if view.selected == Some(thread) {
                    match result {
                        Ok(Ok(Reply::Detail(detail))) if view.detail.as_ref().is_some_and(|current| current.history.sequence == detail.history.sequence) => {
                            let mut older = detail.history.messages;
                            older.retain(|item| !view.older.iter().any(|old| old.message.id == item.message.id));
                            older.append(&mut view.older);
                            view.older = older;
                            view.before = detail.history.older;
                        }
                        _ => {
                            view.older.clear();
                            view.before = view.detail.as_ref().and_then(|detail| detail.history.older.clone());
                            view.error = Some("History changed or could not load. Try loading older messages again.".into());
                        }
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn expand_message(&self, thread: ThreadId, message: MessageId, cx: &mut Context<Self>) {
        let Some(client) = self.client().cloned() else {
            return;
        };
        let request = Request::MessageBody {
            project: self.machine.project.clone(),
            thread: thread.clone(),
            message: message.clone(),
        };
        let load = self
            .executor
            .spawn(async move { client.request(request).await });
        cx.spawn(async move |view, cx| {
            let result = load.await;
            let _ = view.update(cx, |view, cx| {
                if view.selected.as_ref() != Some(&thread) {
                    return;
                }
                match result {
                    Ok(Ok(Reply::MessageBody(body))) => {
                        view.full_messages.insert(message, body);
                    }
                    Ok(Ok(_)) => view.error = Some(ClientError::Protocol.to_string()),
                    Ok(Err(error)) => view.error = Some(error.to_string()),
                    Err(error) => view.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn response(
        &self,
        id: String,
        run: &RunId,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        if !text.contains("![") {
            return div().child(text.to_owned());
        }
        prose(
            id,
            text,
            ResponseImages {
                run,
                checkout: None,
                executor: &self.executor,
                scroll: &self.history_scroll,
            },
            window,
            cx,
        )
    }

    fn message_row(
        &self,
        thread: &ThreadId,
        item: &HistoryItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let message = &item.message;
        let role = if message.role == Role::User {
            "You"
        } else {
            "Codex"
        };
        let full = self.full_messages.get(&message.id);
        let preview = item
            .files
            .as_ref()
            .map_or(&message.text, |files| &files.caption);
        let body = full.unwrap_or(preview);
        let truncated = item
            .files
            .as_ref()
            .map_or(item.truncated, |files| files.truncated);
        v_flex()
            .gap_1()
            .min_w_0()
            .child(label(role))
            .child(control!(
                format!("remote-message/{body}"),
                if message.role == Role::Assistant {
                    self.response(
                        format!("remote-message-{}", message.id),
                        &message.run_id,
                        body,
                        window,
                        cx,
                    )
                } else {
                    div().child(body.clone())
                }
            ))
            .when(truncated && full.is_none(), |row| {
                let thread = thread.clone();
                let message = message.id.clone();
                row.child(control!(
                    "Show full remote message",
                    Button::new(SharedString::from(format!("full-remote-{message}")))
                        .ghost()
                        .small()
                        .label("Show full message")
                        .disabled(self.forward.is_none())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.expand_message(thread.clone(), message.clone(), cx);
                        })),
                ))
            })
    }

    fn apply(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = self.client().cloned().filter(|_| !self.working) else {
            return;
        };
        self.pending = Some(request.clone());
        self.working = true;
        self.error = None;
        let send = self
            .executor
            .spawn(async move { client.request(request).await });
        cx.spawn_in(window, async move |view, cx| {
            let result = send.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.working = false;
                match result {
                    Ok(Ok(Reply::Receipt(receipt))) => match view.pending.take() {
                        Some(Request::Launch { .. }) => {
                            view.name.update(cx, |input, cx| input.set_value("", window, cx));
                            view.select(Some(receipt.thread_id), window, cx);
                            view.message.focus_handle(cx).focus(window);
                        }
                        Some(Request::Send { text, .. }) => {
                            if view.message.read(cx).value().as_str().trim() == text {
                                view.message.update(cx, |input, cx| input.set_value("", window, cx));
                            }
                        }
                        _ => (),
                    },
                    Ok(Ok(Reply::Responded)) => { view.pending = None; }
                    Ok(Err(ClientError::Rejected(error))) => { view.pending = None; view.error = Some(error); }
                    _ if matches!(view.pending, Some(Request::Permission { .. })) => {
                        view.pending = None;
                        view.error = Some("Huggler could not confirm the approval. Check whether the request is still pending.".into());
                    }
                    _ => view.error = Some("Huggler could not confirm the command. Retry to check the same request.".into()),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn new_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_owned();
        if !name.is_empty() {
            self.apply(
                Request::Launch {
                    project: self.machine.project.clone(),
                    thread: ThreadId(Uuid::new_v4().to_string()),
                    command: fresh_command(),
                    name,
                },
                window,
                cx,
            );
        }
    }

    pub fn focus_message(&self, window: &mut Window, cx: &App) {
        let input = if self.selected.is_some() {
            &self.message
        } else {
            &self.name
        };
        input.focus_handle(cx).focus(window);
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let text = self.message.read(cx).value().trim().to_owned();
        if let Some(thread) = self.selected.clone().filter(|_| !text.is_empty()) {
            self.apply(
                Request::Send {
                    project: self.machine.project.clone(),
                    thread,
                    command: fresh_command(),
                    text,
                },
                window,
                cx,
            );
        }
    }
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn fresh_command() -> huggler::CommandId {
    huggler::CommandId(Uuid::new_v4().to_string())
}

impl Render for RemoteProject {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.forward.is_some() && !self.working && self.pending.is_none();
        let competing_request = self.pending.is_some() || self.working;
        let project = &self.machine.project;
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .px_5()
                    .py_3()
                    .gap_4()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(heading(self.machine.target.host.clone()))
                            .child(hint(self.machine.checkout.display().to_string(), cx)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(control!(
                                format!("machine-status/{}", self.status),
                                hint(self.status, cx)
                            ))
                            .child(if self.connection.is_some() {
                                Button::new("disconnect-machine")
                                    .small()
                                    .label("Disconnect")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.disconnect();
                                        cx.notify();
                                    }))
                            } else {
                                Button::new("connect-machine")
                                    .small()
                                    .label("Connect")
                                    .on_click(
                                        cx.listener(|view, _, window, cx| view.connect(window, cx)),
                                    )
                            }),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(
                        h_flex()
                            .id("remote-conversations")
                            .gap_1()
                            .flex_wrap()
                            .max_h(px(112.))
                            .overflow_y_scroll()
                            .px_5()
                            .py_2()
                            .child(control!(
                                "New remote conversation",
                                Button::new("new-remote-thread")
                                    .small()
                                    .ghost()
                                    .label("New conversation")
                                    .selected(self.selected.is_none())
                                    .disabled(competing_request)
                                    .on_click(cx.listener(|view, _, w, cx| {
                                        view.select(None, w, cx);
                                        view.name.focus_handle(cx).focus(w);
                                    }))
                            ))
                            .children(self.threads.iter().map(|thread| {
                                let id = thread.id.clone();
                                control!(
                                    format!("remote-thread/{}", thread.name),
                                    Button::new(SharedString::from(format!("remote-{id}")))
                                        .small()
                                        .ghost()
                                        .label(thread.name.clone())
                                        .selected(self.selected.as_ref() == Some(&id))
                                        .disabled(competing_request)
                                        .on_click(cx.listener(move |view, _, w, cx| {
                                            view.select(Some(id.clone()), w, cx)
                                        }))
                                )
                            })),
                    )
                    .when_some(self.error.clone(), |panel, error| {
                        panel.child(
                            div()
                                .px_5()
                                .py_2()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(error),
                        )
                    })
                    .when_some(
                        self.pending.clone().filter(|_| !self.working),
                        |panel, request| {
                            panel.child(
                                h_flex()
                                    .px_5()
                                    .gap_2()
                                    .child(control!(
                                        "Retry remote request",
                                        Button::new("retry-machine-request")
                                            .label("Retry request")
                                            .disabled(self.forward.is_none())
                                            .on_click(cx.listener(move |view, _, w, cx| {
                                                view.apply(request.clone(), w, cx)
                                            }))
                                    ))
                                    .child(
                                        Button::new("dismiss-machine-request")
                                            .ghost()
                                            .label("Dismiss")
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.pending = None;
                                                view.error = None;
                                                cx.notify();
                                            })),
                                    ),
                            )
                        },
                    )
                    .map(|panel| match self.detail.as_ref() {
                        Some(detail) => {
                            let thread = detail.id.clone();
                            let status = if detail.queue_held {
                                "Queue paused".to_owned()
                            } else {
                                detail
                                    .foreground_run
                                    .as_ref()
                                    .or(detail.latest_run.as_ref())
                                    .map_or_else(
                                        || "Ready".into(),
                                        |run| format!("{:?}", run.status),
                                    )
                            };
                            let stop = Request::Stop {
                                project: project.clone(),
                                thread: thread.clone(),
                                command: fresh_command(),
                            };
                            let resume = Request::ResumeQueue {
                                project: project.clone(),
                                thread: thread.clone(),
                                command: fresh_command(),
                            };
                            panel
                                .child(
                                    v_flex()
                                        .id("remote-history")
                                        .track_scroll(&self.history_scroll)
                                        .overflow_y_scroll()
                                        .flex_1()
                                        .min_h_0()
                                        .px_5()
                                        .gap_3()
                                        .when(self.before.is_some(), |history| {
                                            history.child(
                                                Button::new("remote-older")
                                                    .small()
                                                    .label("Load older messages")
                                                    .disabled(!ready)
                                                    .on_click(
                                                        cx.listener(|view, _, _, cx| {
                                                            view.older(cx)
                                                        }),
                                                    ),
                                            )
                                        })
                                        .children(
                                            self.older.iter().chain(&detail.history.messages).map(
                                                |item| self.message_row(&thread, item, window, cx),
                                            ),
                                        )
                                        .when(!detail.streaming_text.is_empty(), |history| {
                                            history.child(v_flex().child(label("Codex")).child(
                                                match &detail.foreground_run {
                                                    Some(run) => self.response(
                                                        format!("remote-stream-{}", run.id),
                                                        &run.id,
                                                        &detail.streaming_text,
                                                        window,
                                                        cx,
                                                    ),
                                                    None => {
                                                        div().child(detail.streaming_text.clone())
                                                    }
                                                },
                                            ))
                                        })
                                        .when_some(
                                            detail
                                                .latest_run
                                                .as_ref()
                                                .and_then(|run| run.error.clone()),
                                            |p, error| p.child(hint(error, cx)),
                                        )
                                        .children(
                                            detail
                                                .questions
                                                .iter()
                                                .filter_map(|q| self.questions.get(&q.id).cloned()),
                                        )
                                        .children(detail.permissions.iter().map(|permission| {
                                            permission_card(detail, permission, cx).child(
                                                h_flex().flex_wrap().gap_1().children(
                                                    permission.options.iter().map(|option| {
                                                        let request = Request::Permission {
                                                            project: project.clone(),
                                                            thread: thread.clone(),
                                                            run: permission.run_id.clone(),
                                                            request: permission.id.clone(),
                                                            option: option.option_id.clone(),
                                                        };
                                                        Button::new(SharedString::from(format!(
                                                            "{}-{}",
                                                            permission.id, option.option_id
                                                        )))
                                                        .small()
                                                        .label(option.name.clone())
                                                        .disabled(!ready)
                                                        .on_click(cx.listener(
                                                            move |view, _, w, cx| {
                                                                view.apply(request.clone(), w, cx)
                                                            },
                                                        ))
                                                    }),
                                                ),
                                            )
                                        })),
                                )
                                .child(
                                    v_flex()
                                        .gap_2()
                                        .px_5()
                                        .py_3()
                                        .border_t_1()
                                        .border_color(cx.theme().border)
                                        .child(control!(
                                            "Remote message",
                                            Input::new(&self.message)
                                        ))
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(control!(
                                                    format!("remote-status/{status}"),
                                                    hint(status, cx)
                                                ))
                                                .child(div().flex_grow())
                                                .when(
                                                    detail.foreground_run.is_some()
                                                        || detail.queued_count > 0,
                                                    |row| {
                                                        row.child(
                                                            Button::new("remote-stop")
                                                                .small()
                                                                .label("Stop")
                                                                .disabled(!ready)
                                                                .on_click(cx.listener(
                                                                    move |view, _, w, cx| {
                                                                        view.apply(
                                                                            stop.clone(),
                                                                            w,
                                                                            cx,
                                                                        )
                                                                    },
                                                                )),
                                                        )
                                                    },
                                                )
                                                .when(detail.queue_held, |row| {
                                                    row.child(
                                                        Button::new("remote-resume")
                                                            .small()
                                                            .label("Resume queue")
                                                            .disabled(!ready)
                                                            .on_click(cx.listener(
                                                                move |view, _, w, cx| {
                                                                    view.apply(
                                                                        resume.clone(),
                                                                        w,
                                                                        cx,
                                                                    )
                                                                },
                                                            )),
                                                    )
                                                })
                                                .child(control!(
                                                    "Send remote message",
                                                    Button::new("remote-send")
                                                        .primary()
                                                        .label("Send")
                                                        .disabled(!ready)
                                                        .on_click(cx.listener(|view, _, w, cx| {
                                                            view.send(w, cx)
                                                        }))
                                                )),
                                        ),
                                )
                        }
                        None if self.selected.is_some() => panel
                            .child(div().px_5().py_3().child(hint("Loading conversation…", cx))),
                        None => panel.child(
                            v_flex()
                                .gap_2()
                                .px_5()
                                .py_3()
                                .max_w(px(600.))
                                .child(field("Remote conversation name", &self.name))
                                .child(control!(
                                    "Open remote conversation",
                                    Button::new("remote-launch")
                                        .primary()
                                        .label("Open conversation")
                                        .disabled(!ready)
                                        .on_click(
                                            cx.listener(|view, _, w, cx| view.new_thread(w, cx))
                                        )
                                )),
                        ),
                    }),
            )
    }
}
