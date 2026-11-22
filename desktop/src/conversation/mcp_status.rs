use super::*;
use gpui_component::popover::Popover;
use huggler::{McpServerState, McpServerStatus};

type Source = (ProviderInstanceId, PathBuf);

pub(super) struct McpStatus {
    source: Source,
    result: Option<huggler::Result<Option<Vec<McpServerStatus>>>>,
    _load: Task<()>,
}

impl Desktop {
    fn load_mcp_status(&mut self, source: Source, cx: &mut Context<Self>) {
        let (instance, checkout) = source.clone();
        let resolve_project =
            self.current.is_none() && !self.checkout.read(cx).value().trim().is_empty();
        let runtime = self.runtime.clone();
        let job = cx.background_spawn(async move {
            let checkout = if resolve_project {
                git::checkout_root(&checkout)
                    .map_err(|_| huggler::Error::CheckoutUnavailable(checkout))?
            } else {
                checkout
            };
            runtime
                .list_mcp_server_status(instance.as_str(), checkout)
                .await
        });
        let load = cx.spawn(async move |view, cx| {
            let result = job.await;
            let _ = view.update(cx, |view, cx| {
                if let Some(status) = &mut view.mcp_status {
                    status.result = Some(result);
                    cx.notify();
                }
            });
        });
        self.mcp_status = Some(McpStatus {
            source,
            result: None,
            _load: load,
        });
        cx.notify();
    }

    pub(super) fn mcp_status_control(&self, window: &Window, cx: &Context<Self>) -> Div {
        let mut source = self.model_source(cx);
        if source.1.as_os_str().is_empty() {
            source.1 = self.selection_file.parent().unwrap().to_owned();
        }
        let status = self
            .mcp_status
            .as_ref()
            .filter(|status| status.source == source);
        let keyboard_source = source.clone();
        let trigger = Button::new("mcp-status-trigger")
            .small()
            .ghost()
            .icon(Icon::default().path("icons/mcp.svg"))
            .tooltip("MCP server status")
            .on_click(cx.listener(move |view, event, window, cx| {
                if matches!(event, ClickEvent::Keyboard(_)) {
                    view.load_mcp_status(keyboard_source.clone(), cx);
                    view.mcp_status_focus.focus(window);
                }
            }));
        let open_source = source.clone();
        let mut popover = Popover::new("mcp-status-popover")
            .anchor(Corner::BottomRight)
            .trigger(trigger)
            .track_focus(&self.mcp_status_focus)
            .open(status.is_some())
            .on_open_change(cx.listener(move |view, open, window, cx| {
                if *open {
                    view.load_mcp_status(open_source.clone(), cx);
                } else {
                    view.mcp_status = None;
                    if view.mcp_status_focus.contains_focused(window, cx) {
                        view.focus_workspace(window, cx);
                    }
                    cx.notify();
                }
            }));
        if let Some(status) = status {
            let provider = self
                .runtime
                .output()
                .provider_label(source.0.as_str())
                .to_owned();
            let header = h_flex()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("MCP servers"),
                )
                .child(control!(
                    "Refresh MCP status",
                    Button::new("refresh-mcp-status")
                        .small()
                        .ghost()
                        .icon(Icon::default().path("icons/provider-refresh.svg"))
                        .tooltip("Refresh MCP status")
                        .loading(status.result.is_none())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.load_mcp_status(source.clone(), cx);
                        }))
                ));
            let body = match &status.result {
                None => control!("Checking MCP servers…", hint("Checking MCP servers…", cx)),
                Some(Ok(None)) => {
                    let message = format!("{provider} doesn’t expose MCP status.");
                    control!(message.clone(), hint(message, cx))
                }
                Some(Ok(Some(servers))) if servers.is_empty() => {
                    control!(
                        "No MCP servers configured",
                        hint("No MCP servers configured", cx)
                    )
                }
                Some(Ok(Some(servers))) => div().child(
                    div()
                        .id("mcp-status-list")
                        .max_h((window.viewport_size().height * 0.5).min(px(360.)))
                        .overflow_y_scroll()
                        .children(servers.iter().map(|server| server_row(server, cx))),
                ),
                Some(Err(_)) => control!(
                    "Couldn’t check MCP servers",
                    hint("Couldn’t check MCP servers. Refresh to try again.", cx)
                ),
            };
            popover = popover.child(
                v_flex()
                    .track_focus(&self.mcp_status_focus)
                    .w(px(320.).min(window.viewport_size().width - px(40.)))
                    .gap_2()
                    .child(control!("MCP servers", header))
                    .child(body),
            );
        }
        control!("MCP server status", popover)
    }
}

fn server_row(server: &McpServerStatus, cx: &App) -> Div {
    use McpServerState::*;
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let (label, icon, color) = match server.state {
        Connected { tool_count } => (
            format!(
                "{tool_count} {}",
                if tool_count == 1 { "tool" } else { "tools" }
            ),
            IconName::Check,
            theme.green,
        ),
        AuthenticationRequired => ("Needs auth".into(), IconName::TriangleAlert, theme.yellow),
        Failed => ("Error".into(), IconName::CircleX, theme.red),
        Starting => ("Starting".into(), IconName::LoaderCircle, muted),
        NotStarted => ("Not started".into(), IconName::Minus, muted),
        Cancelled => ("Cancelled".into(), IconName::Minus, muted),
        Disabled => ("Disabled".into(), IconName::Minus, muted),
        Unknown => ("Unknown".into(), IconName::Info, muted),
    };
    control!(
        format!("MCP/{}/{}", server.name, label),
        h_flex()
            .w_full()
            .gap_3()
            .px_2()
            .py_1()
            .text_sm()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(server.name.clone())
            )
            .child(div().flex_shrink_0().text_color(muted).child(label))
            .child(Icon::new(icon).with_size(px(14.)).text_color(color))
    )
}
