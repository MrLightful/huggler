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

    #[cfg(target_os = "macos")]
    fn open_mcp_login(
        &mut self,
        source: &Source,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mcp_status = None;
        if let Some(thread) = self.current.as_ref().map(|thread| thread.id.clone()) {
            self.open_terminal(
                &thread,
                super::terminal_pane::TerminalLaunch::Action(command),
                window,
                cx,
            );
        } else {
            let root = source.1.clone();
            let command = command.to_owned();
            if let Err(error) = cx.open_window(WindowOptions::default(), move |window, cx| {
                window.set_window_title("MCP authentication");
                let shell = super::terminal::TerminalView::start(&root, Some(&command), window, cx)
                    .map_err(|error| format!("{error:#}"));
                if let Ok(shell) = &shell {
                    shell.read(cx).focus(window);
                }
                let view = cx.new(|_| AuthTerminal { shell });
                cx.new(|cx| gpui_component::Root::new(view, window, cx))
            }) {
                self.error = Some(error.to_string());
            }
        }
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
            let refresh_source = source.clone();
            let header = h_flex()
                .justify_between()
                .gap_3()
                .child(crate::ui::section_title("MCP servers"))
                .child(control!(
                    "Refresh MCP status",
                    Button::new("refresh-mcp-status")
                        .small()
                        .ghost()
                        .icon(Icon::default().path("icons/provider-refresh.svg"))
                        .tooltip("Refresh MCP status")
                        .loading(status.result.is_none())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.load_mcp_status(refresh_source.clone(), cx);
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
                        .children(servers.iter().map(|server| {
                            let row = server_row(server, cx);
                            #[cfg(target_os = "macos")]
                            if let Some(command) = self
                                .runtime
                                .provider_instances()
                                .find(|instance| instance.id == source.0)
                                .and_then(|instance| login_command(instance, server))
                            {
                                let source = source.clone();
                                return control!(
                                    format!("Authenticate MCP/{}", server.name),
                                    Button::new(SharedString::from(format!(
                                        "mcp-auth-{}",
                                        server.name
                                    )))
                                    .ghost()
                                    .w_full()
                                    .child(row)
                                    .tooltip("Authenticate in terminal")
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.open_mcp_login(&source, &command, window, cx);
                                    }))
                                );
                            }
                            row
                        })),
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
            .text_size(crate::ui::control_text_size())
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

#[cfg(target_os = "macos")]
struct AuthTerminal {
    shell: Result<Entity<super::terminal::TerminalView>, String>,
}

#[cfg(target_os = "macos")]
impl Render for AuthTerminal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().background)
            .child(match &self.shell {
                Ok(shell) => shell.clone().into_any_element(),
                Err(error) => div().p_3().child(error.clone()).into_any_element(),
            })
    }
}

#[cfg(any(target_os = "macos", test))]
fn login_command(instance: &huggler::ProviderInstance, server: &McpServerStatus) -> Option<String> {
    if instance.driver != huggler::Provider::Codex
        || !matches!(server.state, McpServerState::AuthenticationRequired)
    {
        return None;
    }
    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
    let mut parts = vec!["env".to_owned()];
    parts.extend(
        instance
            .environment()
            .iter()
            .map(|(key, value)| quote(&format!("{key}={value}"))),
    );
    if let Some(home) = &instance.home {
        parts.push(quote(&format!("CODEX_HOME={}", home.to_string_lossy())));
    }
    parts.push(quote(&instance.executable.to_string_lossy()));
    parts.extend(instance.args.iter().map(|arg| quote(arg)));
    parts.extend(["mcp".into(), "login".into(), quote(&server.name)]);
    Some(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::{McpServerState, McpServerStatus, login_command};

    #[test]
    fn login_preserves_instance_configuration_and_quotes_server_names() {
        let mut instance = huggler::ProviderInstance::new(
            "custom",
            huggler::Provider::Codex,
            "/path with spaces/codex",
        );
        instance.args = vec!["--config".into(), "value='quoted'".into()];
        instance.home = Some("/private home".into());
        instance
            .set_environment(std::collections::BTreeMap::from([(
                "CODEX_HOME".into(),
                "/other home".into(),
            )]))
            .unwrap();
        let server = McpServerStatus {
            name: "docs'; touch /tmp/unwanted; '".into(),
            state: McpServerState::AuthenticationRequired,
        };
        let command = login_command(&instance, &server).unwrap();
        assert_eq!(
            shlex::split(&command).unwrap(),
            vec![
                "env",
                "CODEX_HOME=/other home",
                "CODEX_HOME=/private home",
                "/path with spaces/codex",
                "--config",
                "value='quoted'",
                "mcp",
                "login",
                &server.name
            ]
        );
        assert!(
            login_command(
                &instance,
                &McpServerStatus {
                    state: McpServerState::Disabled,
                    ..server
                }
            )
            .is_none()
        );
    }
}
