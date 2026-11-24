use super::*;

impl DocumentTabs {
    pub(in crate::conversation) fn has_open_traces(&self) -> bool {
        self.tabs
            .iter()
            .any(|tab| matches!(tab.document, Document::Trace(_)))
    }

    pub(in crate::conversation) fn refresh_traces(&mut self, runtime: &Huggler, thread: &ThreadId) {
        for tab in &mut self.tabs {
            let (Document::Trace(trace), Some(content)) = (&mut tab.document, &mut tab.trace)
            else {
                continue;
            };
            if !trace.run_status.terminal() {
                content.payload = read_trace(runtime, thread, trace);
            }
        }
    }
}

fn read_trace(
    runtime: &Huggler,
    thread: &ThreadId,
    trace: &mut ToolTrace,
) -> huggler::Result<ToolTracePayload> {
    let (updated, payload) = runtime.tool_trace(thread, &trace.run_id, &trace.item.id)?;
    *trace = updated;
    Ok(payload)
}

impl DocumentTab {
    pub(super) fn load_trace(
        &mut self,
        thread: &ThreadId,
        runtime: &Arc<Huggler>,
        cx: &mut Context<Desktop>,
    ) {
        let Document::Trace(trace) = &mut self.document else {
            return;
        };
        let payload = read_trace(runtime, thread, trace);
        if let Some(content) = &mut self.trace {
            content.payload = payload;
            return;
        }
        let reader = (trace.item.kind == huggler::ActivityKind::Command).then(|| {
            let (runtime, thread, run, item, id) = (
                runtime.clone(),
                thread.clone(),
                trace.run_id.clone(),
                trace.item.id.clone(),
                self.id,
            );
            cx.spawn(async move |view, cx| {
                let mut revision = None;
                loop {
                    let result = runtime.command_output(&thread, &run, &item, revision).await;
                    let done = result.as_ref().map_or(true, |output| output.closed);
                    if let Ok(output) = &result {
                        revision = Some(output.revision);
                    }
                    let found = view.update(cx, |view, cx| {
                        let content = view
                            .workspaces
                            .get_mut(&thread)
                            .and_then(|workspace| {
                                workspace.documents.tabs.iter_mut().find(|tab| tab.id == id)
                            })
                            .and_then(|tab| tab.trace.as_mut());
                        if let Some(content) = content {
                            content.output = Some(result);
                            cx.notify();
                            return true;
                        }
                        false
                    });
                    if done || !matches!(found, Ok(true)) {
                        return;
                    }
                }
            })
        });
        self.trace = Some(TraceContent {
            payload,
            output: None,
            _reader: reader,
        });
    }

    pub(super) fn render_trace(
        &self,
        trace: &ToolTrace,
        cx: &Context<Desktop>,
    ) -> impl IntoElement {
        let title = traces::title(trace);
        let theme = cx.theme();
        let command = trace.item.kind == huggler::ActivityKind::Command;
        let mcp = trace.source == huggler::ToolTraceSource::Mcp;
        let mut details = v_flex().gap_5().px_4().py_4().w_full().max_w(px(760.));
        let content = self.trace.as_ref().expect("opened trace");
        let mut has_fields = false;
        match &content.payload {
            Err(error) => {
                details = details.child(hint(format!("Could not read trace details. {error}"), cx));
            }
            Ok(payload) => {
                if let Some(input) = &payload.input {
                    let fields = mcp.then(|| trace_fields(input)).flatten();
                    has_fields |= fields.is_some();
                    details = details.child(trace_section(
                        "Input",
                        input,
                        fields.filter(|_| self.mode != DocumentMode::Raw),
                        false,
                        cx,
                    ));
                }
                if let Some(result) = &payload.result {
                    let fields = mcp.then(|| trace_fields(result)).flatten();
                    has_fields |= fields.is_some();
                    details = details.child(trace_section(
                        "Result",
                        result,
                        fields.filter(|_| self.mode != DocumentMode::Raw),
                        trace.item.status == huggler::ActivityStatus::Failed,
                        cx,
                    ));
                }
                if !command && payload.input.is_none() && payload.result.is_none() {
                    details = details.child(hint("No input or result recorded", cx));
                }
            }
        }
        if command {
            details = match &content.output {
                None => details.child(hint("Reading command output…", cx)),
                Some(Ok(output)) if !output.text.is_empty() => details.child(
                    trace_section("Output", &output.text, None, false, cx)
                        .when(output.truncated, |section| {
                            section.child(hint("Showing the latest 32 KiB", cx))
                        }),
                ),
                Some(Ok(output)) if !output.closed => {
                    details.child(hint("Waiting for command output…", cx))
                }
                Some(Ok(_)) | Some(Err(huggler::Error::NotFound(_))) => {
                    details.child(hint("No command output recorded", cx))
                }
                Some(Err(error)) => {
                    details.child(hint(format!("Could not read command output. {error}"), cx))
                }
            };
        }
        v_flex()
            .id(("trace-document", self.id))
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .min_w_0()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .size(px(32.))
                            .flex_none()
                            .justify_center()
                            .rounded_md()
                            .bg(theme.muted)
                            .child(traces::icon(trace, cx)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(control!(
                                format!("Tool trace/{title}"),
                                div()
                                    .text_base()
                                    .line_height(px(22.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title.to_owned())
                            ))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(hint(traces::source_label(trace.source), cx).text_xs())
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(traces::status_color(trace, cx))
                                            .child(traces::status(trace)),
                                    ),
                            ),
                    )
                    .when(has_fields, |header| {
                        header.child(self.mode_control(
                            &[
                                (DocumentMode::Preview, "Fields"),
                                (DocumentMode::Raw, "Raw"),
                            ],
                            cx,
                        ))
                    }),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("trace-content")
                            .size_full()
                            .overflow_scroll()
                            .track_scroll(&self.scroll)
                            .child(details),
                    )
                    .scrollbar(&self.scroll, ScrollbarAxis::Both),
            )
    }
}

type TraceFields = serde_json::Map<String, serde_json::Value>;

fn trace_fields(text: &str) -> Option<TraceFields> {
    serde_json::from_str::<TraceFields>(text)
        .ok()
        .filter(|fields| !fields.is_empty())
}

fn trace_section(
    label: &'static str,
    text: &str,
    fields: Option<TraceFields>,
    failed: bool,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let body = match fields {
        Some(fields) => {
            v_flex().children(fields.into_iter().enumerate().map(|(index, (key, value))| {
                let value = match value {
                    serde_json::Value::String(value) if !value.is_empty() => value,
                    serde_json::Value::Array(blocks) if label == "Result" && key == "content" => {
                        text_content(&blocks).unwrap_or_else(|| {
                            serde_json::to_string_pretty(&blocks).unwrap_or_default()
                        })
                    }
                    _ => serde_json::to_string_pretty(&value).unwrap_or_default(),
                };
                h_flex()
                    .items_start()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .when(index > 0, |field| {
                        field.border_t_1().border_color(theme.border)
                    })
                    .child(
                        control!(
                            format!("Trace field/{key}"),
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(key)
                        )
                        .w(relative(0.3))
                        .max_w(px(112.))
                        .flex_none(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_family(theme.mono_font_family.clone())
                            .line_height(px(20.))
                            .child(StyledText::new(value)),
                    )
            }))
        }
        None => div()
            .px_3()
            .py_2()
            .whitespace_nowrap()
            .font_family(theme.mono_font_family.clone())
            .text_sm()
            .line_height(px(20.))
            .child(StyledText::new(text.to_owned())),
    };
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .when(failed, |heading| {
                    heading.child(
                        Icon::new(IconName::TriangleAlert)
                            .xsmall()
                            .text_color(theme.danger),
                    )
                })
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(label),
                ),
        )
        .child(control!(
            format!("Trace {label}/{}", &text[..text.floor_char_boundary(1024)]),
            div()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.sidebar)
                .child(body)
        ))
}

fn text_content(blocks: &[serde_json::Value]) -> Option<String> {
    if blocks.is_empty() {
        return None;
    }
    let texts = blocks
        .iter()
        .map(|block| {
            (block["type"] == "text")
                .then(|| block["text"].as_str())
                .flatten()
        })
        .collect::<Option<Vec<_>>>()?;
    Some(texts.join("\n\n"))
}
