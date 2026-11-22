mod turn_rail;

use super::attachments::{MESSAGE_LINE_HEIGHT, caption_with_attachments};
use super::markdown_view::prose;
use super::*;
use gpui_component::{Icon, tooltip::Tooltip};
use huggler::{
    ActivityCounts, ActivityItem, ActivityKind, ActivityStatus, HistoryItem, PermissionRequest,
    Role, RunActivity, RunStatus,
};
use std::time::{Duration, Instant};
use turn_rail::TurnPreview;

const GUTTER: Pixels = px(20.);
// Markdown must measure at the final width so wrapped lines clear the footer.
const MAX_RESPONSE_WIDTH: Pixels = px(700.);
pub(super) struct TurnClock {
    pub(super) run: RunId,
    opened: Instant,
    elapsed_at_open_ms: i64,
    _ticker: Task<()>,
}

impl TurnClock {
    pub(super) fn new(run: RunId, started_at_ms: i64, cx: &mut Context<Self>) -> Self {
        let ticker = cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if view.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self {
            run,
            opened: Instant::now(),
            elapsed_at_open_ms: (chrono::Utc::now().timestamp_millis() - started_at_ms).max(0),
            _ticker: ticker,
        }
    }
}

impl Render for TurnClock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let time =
            format_duration(self.elapsed_at_open_ms + self.opened.elapsed().as_millis() as i64);
        control!("elapsed-time", div().child(time))
            .font_family(cx.theme().mono_font_family.clone())
            .text_sm()
            .text_color(transcript_muted(cx))
    }
}

impl Desktop {
    pub(super) fn permission(
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
        let input = request
            .tool_call
            .get("rawInput")
            .unwrap_or(&request.tool_call);
        let details = input["command"]
            .as_str()
            .or_else(|| input["reason"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{input:#}"));
        v_flex()
            .gap_2()
            .p_3()
            .border_1()
            .rounded_lg()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .small()
                            .text_color(transcript_muted(cx)),
                    )
                    .child(control!(format!("permission-title/{title}"), label(title))),
            )
            .child(text_preview(
                SharedString::from(format!("details-{}", request.id)),
                div().child(details),
                px(96.),
                cx,
            ))
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
                        if let Ok(output) = &result {
                            revision = Some(output.revision);
                            done = output.closed;
                        }
                        log.content = Some(result);
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
            content: None,
            _reader: reader,
        });
        cx.notify();
    }

    pub(super) fn transcript(
        &self,
        thread: &ConversationDetail,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let provider = self
            .runtime
            .output()
            .provider_label(thread.provider_instance.as_str());
        let mut history = v_flex()
            .id("history")
            .track_scroll(&self.history_scroll)
            .size_full()
            .items_center()
            .pt_4()
            .pb_4()
            .gap_6()
            .overflow_y_scroll();
        let pager = h_flex()
            .w(width)
            .mx_auto()
            .px(GUTTER)
            .pt_2()
            .gap_2()
            .when_some(thread.history.older.clone(), |row, older| {
                row.child(control!(
                    "Older",
                    Button::new("older")
                        .ghost()
                        .small()
                        .label("Older")
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.page_history(Some(older.clone()), cx);
                        }))
                ))
            })
            .when(self.before.is_some(), |row| {
                row.child(control!(
                    "Newest",
                    Button::new("newest")
                        .ghost()
                        .small()
                        .label("Newest")
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.page_history(None, cx);
                        }))
                ))
            });
        let messages = &thread.history.messages;
        let content_width = (width - GUTTER * 2.).max(px(0.));
        let response_width = content_width.min(MAX_RESPONSE_WIDTH);
        let column = || v_flex().w(width).px(GUTTER).gap_3().flex_shrink_0();
        let mut previews = Vec::new();
        let mut shown = HashSet::new();
        let foreground = thread
            .foreground_run
            .as_ref()
            .filter(|_| self.before.is_none());
        for messages in messages.chunk_by(|a, b| a.message.run_id == b.message.run_id) {
            let run = &messages[0].message.run_id;
            let activity = thread
                .activity
                .iter()
                .find(|activity| &activity.run_id == run);
            let streaming = foreground.is_some_and(|active| &active.id == run);
            previews.push(TurnPreview::new(
                messages,
                activity,
                streaming.then_some(thread.streaming_text.as_str()),
            ));
            let mut rows = column();
            for item in messages {
                if item.message.role == Role::Assistant
                    && let Some(activity) = activity
                    && shown.insert(run)
                {
                    rows = rows.child(self.activity_rows(
                        thread,
                        activity,
                        response_width,
                        window,
                        cx,
                    ));
                }
                rows =
                    rows.child(self.message_row(thread, item, provider, content_width, window, cx));
            }
            if let Some(activity) = activity
                && shown.insert(run)
            {
                rows = rows.child(self.activity_rows(thread, activity, response_width, window, cx));
            }
            if streaming {
                rows = rows.children(self.streaming(thread, response_width, window, cx));
            }
            history = history.child(rows);
        }
        let mut tail = Vec::new();
        if messages.is_empty() && thread.foreground_run.is_none() {
            tail.push(hint("Send a message to start this conversation.", cx));
        }
        if self.before.is_none() {
            for activity in &thread.activity {
                if shown.insert(&activity.run_id) {
                    tail.push(self.activity_rows(thread, activity, response_width, window, cx));
                }
            }
            if let Some(run) = foreground
                && !messages.iter().any(|item| item.message.run_id == run.id)
            {
                tail.extend(self.streaming(thread, response_width, window, cx));
            }
            for child in &thread.native_children {
                let status = match child.status {
                    huggler::NativeChildStatus::RequiresAction => "Needs input",
                    huggler::NativeChildStatus::Starting => "Starting",
                    huggler::NativeChildStatus::Running => "Running",
                    huggler::NativeChildStatus::Idle => "Idle",
                    huggler::NativeChildStatus::Unknown => "Status unavailable",
                    huggler::NativeChildStatus::Completed => "Completed",
                    huggler::NativeChildStatus::Interrupted => "Interrupted",
                    huggler::NativeChildStatus::Failed => "Failed",
                    huggler::NativeChildStatus::Disconnected => "Disconnected",
                };
                tail.push(activity_row(IconName::Bot, &child.path, Some(status), cx));
            }
        }
        if !tail.is_empty() {
            history = history.child(column().children(tail));
        }
        v_flex()
            .flex_1()
            .min_h_0()
            .when(
                thread.history.older.is_some() || self.before.is_some(),
                |column| column.child(pager),
            )
            .child(div().relative().flex_1().min_h_0().child(history).when(
                !previews.is_empty(),
                |viewport| {
                    viewport.child(turn_rail::render(
                        previews,
                        &self.history_scroll,
                        window,
                        cx,
                    ))
                },
            ))
    }

    pub(super) fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        if self.before.is_some() {
            self.page_history(None, cx);
        } else {
            self.history_scroll.scroll_to_bottom();
            cx.notify();
        }
    }

    fn page_history(&mut self, before: Option<HistoryCursor>, cx: &mut Context<Self>) {
        self.before = before;
        let result = self.refresh();
        if self.before.is_none() {
            self.history_scroll.scroll_to_bottom();
        } else {
            self.history_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.report(result, cx);
    }

    fn message_row(
        &self,
        thread: &ConversationDetail,
        item: &HistoryItem,
        provider: &str,
        content_width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let message = &item.message;
        let outgoing = message.role == Role::User && message.origin.is_none();
        let author = match &message.origin {
            Some(sender) => self
                .threads
                .iter()
                .find(|t| t.id == sender.thread_id)
                .map_or(sender.thread_id.0.as_str(), |t| t.name.as_str()),
            None if outgoing => "You",
            None => provider,
        };
        let full = self.full_messages.get(&message.id);
        let caption = full.map(String::as_str).unwrap_or_else(|| {
            item.files
                .as_ref()
                .map_or(message.text.as_str(), |files| files.caption.as_str())
        });
        let truncated = full.is_none()
            && item
                .files
                .as_ref()
                .map_or(item.truncated, |files| files.truncated);
        let id = &message.id;
        let bubble_width = (content_width * 0.88).min(px(620.));
        let message_padding = rems(1.);
        let mut body = v_flex()
            .min_w_0()
            .gap_2()
            .when(outgoing, |body| {
                body.max_w(bubble_width)
                    .px(message_padding)
                    .py_3()
                    .rounded_xl()
                    .bg(cx
                        .theme()
                        .accent
                        .blend(cx.theme().foreground.opacity(0.035)))
            })
            .when(!outgoing, |body| body.w_full());
        if let Some(files) = &item.files {
            let width = if outgoing {
                (bubble_width - message_padding.to_pixels(window.rem_size()) * 2.).max(px(0.))
            } else {
                content_width.min(MAX_RESPONSE_WIDTH)
            };
            body = body.child(caption_with_attachments(
                caption,
                || format!("{author}/{caption}"),
                width,
                self.sent_file_chips(&thread.id, id, &files.names, width, window, cx),
                window,
                cx,
            ));
        } else if !caption.is_empty() {
            body = body.child(control!(
                format!("{author}/{caption}"),
                if outgoing || message.origin.is_some() {
                    div()
                        .text_sm()
                        .line_height(MESSAGE_LINE_HEIGHT)
                        .child(caption.to_owned())
                } else {
                    prose(format!("message-{id}"), caption, window, cx)
                }
            ));
        }
        if truncated {
            let id = id.clone();
            body = body.child(h_flex().child(control!(
                    format!("expand/{id}"),
                    Button::new(SharedString::from(format!("full-{id}")))
                        .ghost()
                        .small()
                        .compact()
                        .icon(IconName::ChevronDown)
                        .label("Show full message")
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if let Some(thread) = &view.current {
                                match view.runtime.read_message_body(&thread.id, &id) {
                                    Ok(body) => {
                                        view.full_messages.insert(id.clone(), body);
                                    }
                                    Err(error) => view.error = Some(error.to_string()),
                                }
                            }
                            cx.notify();
                        }))
                )));
        }
        let preview = item
            .files
            .as_ref()
            .and_then(|files| self.sent_file_preview(&thread.id, id, files.names.len(), cx));
        let group = format!("message-hover-{id}");
        let copy = if let Some(files) = item.files.as_ref().filter(|_| caption.is_empty()) {
            files.names.join("\n")
        } else {
            caption.to_owned()
        };
        let thread = thread.id.clone();
        let id = id.clone();
        let copy_button = copy_button(&group).on_click(cx.listener(move |view, _, _, cx| {
            let result = if truncated {
                view.runtime.read_message_body(&thread, &id)
            } else {
                Ok(copy.clone())
            };
            match result {
                Ok(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                Err(error) => view.report(Err(error), cx),
            }
        }));
        let metadata = message_footer(
            &group,
            message.origin.is_some().then_some(author),
            item.timestamp_ms,
            !outgoing,
            item.turn_duration_ms,
            copy_button,
            cx,
        );
        if outgoing {
            v_flex()
                .w(content_width)
                .group(group)
                .child(
                    h_flex()
                        .w(content_width)
                        .justify_end()
                        .child(body.flex_shrink_0()),
                )
                .when_some(preview, |column, preview| {
                    column.child(h_flex().justify_end().child(preview.w(bubble_width)))
                })
                .child(metadata.justify_end())
        } else {
            v_flex()
                .w(content_width.min(MAX_RESPONSE_WIDTH))
                .group(group)
                .child(body)
                .children(preview)
                .child(metadata)
        }
    }

    fn streaming(
        &self,
        thread: &ConversationDetail,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let run = thread.foreground_run.as_ref()?;
        if self.before.is_some() {
            return None;
        }
        let status = match run.status {
            RunStatus::Starting => "Starting",
            RunStatus::Waiting => "Finishing",
            _ => "Working",
        };
        let indicator = if let Some(clock) = &self.turn_clock {
            h_flex()
                .gap_2()
                .text_color(transcript_muted(cx))
                .child(Icon::new(IconName::LoaderCircle).xsmall())
                .child(clock.clone())
        } else {
            activity_row(IconName::LoaderCircle, status, None, cx)
        };
        Some(
            v_flex()
                .gap_1()
                .w(width)
                .when(!thread.streaming_text.is_empty(), |row| {
                    row.child(control!(
                        format!("writing/{}", thread.streaming_text),
                        prose(
                            format!("stream-{}", run.id),
                            &thread.streaming_text,
                            window,
                            cx
                        )
                    ))
                })
                .child(indicator),
        )
    }

    fn activity_rows(
        &self,
        thread: &ConversationDetail,
        activity: &RunActivity,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut rows = v_flex().gap_2().min_w_0().w(width);
        if activity.status.producing_output() || activity.truncated {
            rows =
                rows.child(self.activity_group(thread, activity, &activity.items, 0, window, cx));
        } else {
            let mut command_index = 0;
            for group in activity.items.chunk_by(|a, b| {
                a.kind != ActivityKind::Compaction && b.kind != ActivityKind::Compaction
            }) {
                if group[0].kind == ActivityKind::Compaction {
                    rows = rows.child(compaction_row(cx));
                } else {
                    rows = rows.child(self.activity_group(
                        thread,
                        activity,
                        group,
                        command_index,
                        window,
                        cx,
                    ));
                    command_index += group
                        .iter()
                        .filter(|item| item.kind == ActivityKind::Command)
                        .count();
                }
            }
        }
        let status = match activity.status {
            RunStatus::Failed => Some((IconName::TriangleAlert, "Run failed")),
            RunStatus::Interrupted => Some((IconName::CircleX, "Run stopped")),
            _ => None,
        };
        if let Some((icon, label)) = status {
            rows = rows.child(activity_row(icon, label, None, cx));
        }
        rows
    }

    fn activity_group(
        &self,
        thread: &ConversationDetail,
        activity: &RunActivity,
        group: &[ActivityItem],
        command_start: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let run = &activity.run_id;
        let live = activity.status.producing_output();
        let key = (run.clone(), (!live).then(|| group[0].id.clone()));
        let expanded = self.expanded_activity.contains(&key);
        let counts = activity.counts.unwrap_or_else(|| {
            let messages = group
                .iter()
                .filter(|item| item.kind == ActivityKind::Commentary)
                .count() as u32;
            ActivityCounts {
                tools: group.len() as u32 - messages,
                messages,
                failed: group
                    .iter()
                    .filter(|item| item.status == ActivityStatus::Failed)
                    .count() as u32,
            }
        });
        let mut labels = Vec::new();
        if counts.tools != 0 {
            labels.push(if counts.tools == 1 {
                "1 tool call".into()
            } else {
                format!("{} tool calls", counts.tools)
            });
        }
        if counts.messages != 0 {
            labels.push(if counts.messages == 1 {
                "1 message".into()
            } else {
                format!("{} messages", counts.messages)
            });
        }
        let summary = labels.join(" · ");
        let failed = counts.failed;
        let mut group_row = v_flex().gap_1().when(!summary.is_empty(), |row| {
            row.child(h_flex().child(control!(
                    format!("activity/{summary}"),
                    Button::new(SharedString::from(format!(
                        "activity-{run}-{}",
                        key.1.as_deref().unwrap_or("live")
                    )))
                    .ghost()
                    .small()
                    .compact()
                    .icon(disclosure(expanded))
                    .child(
                        h_flex()
                            .gap_2()
                            .text_color(transcript_muted(cx))
                            .child(summary)
                            .when(failed > 0, |row| row.child(
                                div()
                                    .text_xs()
                                    .text_color(transcript_error(cx))
                                    .child(format!("{failed} failed"))
                            ))
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if !view.expanded_activity.remove(&key) {
                            view.expanded_activity.insert(key.clone());
                        }
                        cx.notify();
                    }))
                )))
        });
        if expanded || live {
            let latest_tool = group.iter().rposition(|item| {
                !matches!(
                    item.kind,
                    ActivityKind::Commentary | ActivityKind::Compaction
                )
            });
            let latest_message = thread
                .streaming_text
                .is_empty()
                .then(|| {
                    group
                        .iter()
                        .rposition(|item| item.kind == ActivityKind::Commentary)
                })
                .flatten();
            let mut commands = command_start;
            let mut entries = v_flex()
                .ml_3()
                .pl_3()
                .py_1()
                .gap_2()
                .border_l_1()
                .border_color(cx.theme().border);
            if expanded && activity.truncated {
                entries = entries.child(control!(
                    "Earlier activity omitted",
                    hint("Earlier activity omitted", cx)
                ));
            }
            for (index, item) in group.iter().enumerate() {
                if item.kind == ActivityKind::Command {
                    commands += 1;
                }
                if !expanded && latest_tool != Some(index) && latest_message != Some(index) {
                    continue;
                }
                if item.kind == ActivityKind::Compaction {
                    entries = entries.child(compaction_row(cx));
                } else if item.kind == ActivityKind::Command {
                    entries = entries.child(self.command_row(
                        &thread.id,
                        run,
                        activity.status,
                        item,
                        commands,
                        cx,
                    ));
                } else if item.kind == ActivityKind::Commentary {
                    entries = entries.child(control!(
                        format!("commentary/{}", item.title),
                        prose(
                            format!("commentary-{run}-{}", item.id),
                            &item.title,
                            window,
                            cx
                        )
                    ));
                } else {
                    let (icon, fallback) = match item.kind {
                        ActivityKind::FileChange => (IconName::File, "File change"),
                        ActivityKind::Search => (IconName::Search, "Web search"),
                        ActivityKind::Agent => (IconName::Bot, "Agent tool"),
                        _ => (IconName::Settings2, "Tool call"),
                    };
                    let title = if item.title.is_empty() {
                        fallback
                    } else {
                        &item.title
                    };
                    entries = entries.child(control!(
                        format!("activity-item/{title}"),
                        activity_row(icon, title, item_status(item.status, activity.status), cx),
                    ));
                }
            }
            group_row = group_row.child(entries);
        }
        group_row
    }

    fn command_row(
        &self,
        thread: &ThreadId,
        run: &RunId,
        run_status: RunStatus,
        item: &ActivityItem,
        index: usize,
        cx: &Context<Self>,
    ) -> Div {
        let log = self
            .command_log
            .as_ref()
            .filter(|log| &log.run == run && log.item == item.id);
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        let qualification_label = if self
            .current
            .as_ref()
            .and_then(|thread| {
                thread
                    .foreground_run
                    .as_ref()
                    .or(thread.latest_run.as_ref())
            })
            .is_some_and(|latest| &latest.id == run)
        {
            format!("Command {index} output")
        } else {
            format!("command/{run}/{index}")
        };
        let title = if item.title.is_empty() {
            format!("Command {index} output")
        } else {
            item.title.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        let row_id = SharedString::from(format!("command-{run}-{}", item.id));
        let (thread, run, item_id) = (thread.clone(), run.clone(), item.id.clone());
        v_flex()
            .gap_1()
            .min_w_0()
            .child(
                h_flex()
                    .gap_1()
                    .min_w_0()
                    .child(control!(
                        qualification_label,
                        Button::new(row_id.clone())
                            .ghost()
                            .small()
                            .compact()
                            .max_w_full()
                            .min_w_0()
                            .justify_start()
                            .icon(disclosure(log.is_some()))
                            .child(Icon::new(IconName::SquareTerminal).xsmall())
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .child(title)
                            )
                            .tooltip(if log.is_some() {
                                "Hide output"
                            } else {
                                "Show output"
                            })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.toggle_command(
                                    thread.clone(),
                                    run.clone(),
                                    item_id.clone(),
                                    cx,
                                );
                            }))
                    ))
                    .children(
                        item_status(item.status, run_status)
                            .map(|status| hint(status, cx).flex_shrink_0()),
                    ),
            )
            .when_some(log, |row, log| {
                row.child(match &log.content {
                    None => hint("Loading output…", cx),
                    Some(Err(error)) => {
                        hint(error.to_string(), cx).text_color(transcript_error(cx))
                    }
                    Some(Ok(output)) if output.text.is_empty() => hint(
                        if output.closed {
                            "No output"
                        } else {
                            "Waiting for output…"
                        },
                        cx,
                    ),
                    Some(Ok(output)) => {
                        let text = if output.truncated {
                            format!("… earlier output omitted\n{}", output.text)
                        } else {
                            output.text.clone()
                        };
                        text_preview(
                            row_id,
                            control!(format!("command-output/{text}"), text),
                            px(176.),
                            cx,
                        )
                    }
                })
            })
    }
}

fn disclosure(expanded: bool) -> IconName {
    if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    }
}

fn compaction_row(cx: &App) -> Div {
    h_flex()
        .gap_3()
        .text_xs()
        .text_color(transcript_muted(cx))
        .child(Icon::new(IconName::Minimize).xsmall())
        .child(control!("Context compacted", "Context compacted"))
        .child(div().flex_1().h_px().bg(cx.theme().border))
}

fn format_duration(ms: i64) -> String {
    let seconds = ms / 1_000;
    let tenths = (ms % 1_000) / 100;
    if seconds >= 60 {
        format!("{}m {}.{tenths}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}.{tenths}s")
    }
}

fn copy_button(group: &str) -> Button {
    Button::new(SharedString::from(format!("copy-{group}")))
        .ghost()
        .small()
        .compact()
        .icon(IconName::Copy)
        .tooltip("Copy")
}

fn message_footer(
    group: &str,
    author: Option<&str>,
    timestamp_ms: Option<i64>,
    show_duration: bool,
    duration_ms: Option<i64>,
    copy: Button,
    cx: &App,
) -> Div {
    let local = timestamp_ms
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|time| time.with_timezone(&chrono::Local));
    let time = local.as_ref().map_or_else(
        || "Time unavailable".into(),
        |time| time.format("%H:%M").to_string(),
    );
    let time = control!(format!("message-time/{time}"), div().child(time))
        .id(SharedString::from(format!("time-{group}")))
        .when_some(local, |time, date| {
            let date = date.format("%b %-d, %Y · %H:%M").to_string();
            time.tooltip(move |window, cx| Tooltip::new(date.clone()).build(window, cx))
        });
    h_flex()
        .mt_1()
        .gap_2()
        .text_xs()
        .text_color(transcript_muted(cx))
        .invisible()
        .group_hover(SharedString::from(group.to_owned()), |style| {
            style.visible()
        })
        .when_some(author, |footer, author| {
            footer
                .child(div().min_w_0().truncate().child(author.to_owned()))
                .child("·")
        })
        .child(time)
        .when(show_duration, |footer| {
            let duration = duration_ms.map_or_else(
                || "Duration unavailable".into(),
                |ms| {
                    let seconds = ms / 1_000;
                    if seconds >= 60 {
                        format!("{}m, {}s", seconds / 60, seconds % 60)
                    } else {
                        format!("{seconds}s")
                    }
                },
            );
            footer.child("·").child(control!(
                format!("message-duration/{duration}"),
                div().child(duration)
            ))
        })
        .child(copy.flex_shrink_0())
}

fn item_status(status: ActivityStatus, run: RunStatus) -> Option<&'static str> {
    match status {
        ActivityStatus::Running if !run.terminal() => Some("Running"),
        ActivityStatus::Running | ActivityStatus::Unknown if run == RunStatus::Interrupted => {
            Some("Stopped")
        }
        ActivityStatus::Running if run.terminal() => Some("Unconfirmed"),
        ActivityStatus::Failed => Some("Failed"),
        ActivityStatus::Interrupted => Some("Stopped"),
        _ => None,
    }
}

fn activity_row(icon: IconName, title: &str, status: Option<&str>, cx: &App) -> Div {
    h_flex()
        .w_full()
        .gap_2()
        .min_w_0()
        .text_sm()
        .text_color(transcript_muted(cx))
        .child(Icon::new(icon).xsmall().flex_shrink_0())
        .child(
            control!(
                format!("activity-label/{title}"),
                div().truncate().child(title.to_owned())
            )
            .min_w_0(),
        )
        .children(status.map(|status| {
            div()
                .text_xs()
                .whitespace_nowrap()
                .flex_shrink_0()
                .child(status.to_owned())
        }))
}

pub(super) fn text_preview(
    id: impl Into<ElementId>,
    body: Div,
    max_height: Pixels,
    cx: &App,
) -> Div {
    let panel = div()
        .id(id)
        .w_full()
        .min_w_0()
        .max_h(max_height)
        .overflow_y_scroll()
        .rounded_md()
        .bg(cx.theme().muted)
        .p_3()
        .text_xs()
        .line_height(px(18.))
        .font_family(cx.theme().mono_font_family.clone())
        .child(body);
    div().w_full().child(panel)
}

pub(super) fn transcript_muted(cx: &App) -> Hsla {
    cx.theme()
        .muted_foreground
        .blend(cx.theme().foreground.opacity(0.24))
}

fn transcript_error(cx: &App) -> Hsla {
    cx.theme().danger.blend(cx.theme().foreground.opacity(0.38))
}
