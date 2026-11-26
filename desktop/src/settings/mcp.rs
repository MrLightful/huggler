use super::{Settings, providers::ProviderSection};
use crate::ui::{hint, label, section_title};
use anyhow::{Result, ensure};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    switch::Switch,
    v_flex,
};
use huggler::{
    Provider,
    providers::codex::mcp_settings::{
        McpConnection, McpEdit, McpServer, McpSettings, McpWriteOutcome,
    },
};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Panel {
    settings: Option<McpSettings>,
    pending: Option<Uuid>,
    unavailable: bool,
    overridden: bool,
    error: Option<String>,
    editor: Option<Editor>,
    removing: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transport {
    Http,
    Stdio,
}

struct Editor {
    existing: Option<String>,
    revision: String,
    name: Entity<InputState>,
    url: Entity<InputState>,
    command: Entity<InputState>,
    args: Entity<InputState>,
    transport: Transport,
}

impl Editor {
    fn new(
        server: Option<&McpServer>,
        revision: &str,
        window: &mut Window,
        cx: &mut Context<Settings>,
    ) -> Self {
        let connection = server.and_then(|server| server.connection.as_ref());
        let (transport, url, command, args) = match connection {
            Some(McpConnection::Stdio { command, args }) => (
                Transport::Stdio,
                "",
                command.as_str(),
                shlex::try_join(args.iter().map(String::as_str)).unwrap_or_default(),
            ),
            Some(McpConnection::Http { url }) => (Transport::Http, url.as_str(), "", String::new()),
            None => (Transport::Http, "", "", String::new()),
        };
        let mut field = |value: &str, placeholder: &'static str| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value.to_owned())
                    .placeholder(placeholder)
            })
        };
        Self {
            existing: server.map(|server| server.name.clone()),
            revision: revision.into(),
            name: field(server.map_or("", |server| server.name.as_str()), "docs"),
            url: field(url, "https://example.com/mcp"),
            command: field(command, "Full path or executable name"),
            args: field(&args, "Optional arguments"),
            transport,
        }
    }

    fn read(&self, settings: &McpSettings, cx: &App) -> Result<(String, McpEdit)> {
        let entered_name = self.name.read(cx).value();
        let name = self
            .existing
            .as_deref()
            .unwrap_or_else(|| entered_name.trim());
        let server = settings.servers.iter().find(|server| server.name == name);
        if self.existing.is_some() {
            ensure!(
                server.is_some(),
                "This server was removed. Cancel and add it again."
            );
            ensure!(
                settings.revision == self.revision,
                "Codex configuration changed. Cancel and reopen the editor to review the latest settings."
            );
        } else {
            ensure!(
                McpServer::valid_name(name),
                "Use letters, numbers, hyphens, or underscores in the server name."
            );
            ensure!(server.is_none(), "A server named {name} already exists.");
        }
        let connection = match self.transport {
            Transport::Http => {
                let url = self.url.read(cx).value();
                McpConnection::Http {
                    url: url.trim().into(),
                }
            }
            Transport::Stdio => {
                let command = self.command.read(cx).value();
                let args = self.args.read(cx).value();
                let args = shlex::split(&args).ok_or_else(|| {
                    anyhow::anyhow!("Close any open quotes in the server arguments.")
                })?;
                McpConnection::Stdio {
                    command: command.trim().into(),
                    args,
                }
            }
        };
        connection
            .validate()
            .map_err(|message| anyhow::anyhow!(message))?;
        if self.existing.is_some() {
            Ok((name.into(), McpEdit::Edit(connection)))
        } else {
            Ok((name.into(), McpEdit::Add(connection)))
        }
    }
}

impl Settings {
    pub(super) fn request_mcp_settings(&mut self, cx: &mut Context<Self>) {
        if self.provider_details_open() && self.selected_provider().driver == Provider::Codex {
            self.update_mcp_settings(None, cx);
        }
    }

    fn update_mcp_settings(&mut self, edit: Option<(String, McpEdit)>, cx: &mut Context<Self>) {
        let panel = &mut self.providers.mcp;
        if panel.pending.is_some() {
            return;
        }
        let revision = panel
            .settings
            .as_ref()
            .map(|settings| settings.revision.clone());
        let request = Uuid::new_v4();
        panel.pending = Some(request);
        panel.error = None;
        panel.overridden = false;
        panel.removing = None;
        let runtime = self.runtime.clone();
        let instance = self.providers.selected.clone();
        let cwd = self.state.clone();
        let job = cx.background_spawn(async move {
            let outcome = if let Some((name, edit)) = edit {
                Some(
                    runtime
                        .write_mcp_settings(
                            instance.as_str(),
                            &cwd,
                            revision.expect("edits originate from loaded MCP settings"),
                            name,
                            edit,
                        )
                        .await?,
                )
            } else {
                None
            };
            Ok((
                outcome,
                runtime.read_mcp_settings(instance.as_str(), &cwd).await,
            ))
        });
        cx.spawn(async move |view, cx| {
            let result: huggler::Result<_> = job.await;
            let _ = view.update(cx, |view, cx| {
                let panel = &mut view.providers.mcp;
                if panel.pending != Some(request) {
                    return;
                }
                panel.pending = None;
                match result {
                    Ok((outcome, result)) => {
                        panel.overridden = outcome == Some(McpWriteOutcome::Overridden);
                        if outcome.is_some() {
                            panel.editor = None;
                        }
                        match result {
                            Ok(settings) => {
                                panel.unavailable = settings.is_none();
                                panel.settings = settings;
                            }
                            Err(_) => {
                                panel.settings = None;
                                panel.error = Some(
                                    if outcome.is_some() {
                                        "Changes saved. Refresh to reload the server list."
                                    } else {
                                        "Couldn’t load MCP servers. Refresh to try again."
                                    }
                                    .into(),
                                );
                            }
                        }
                    }
                    Err(error) => panel.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_mcp_editor(&mut self, name: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let panel = &mut self.providers.mcp;
        let Some(settings) = panel.settings.as_ref() else {
            return;
        };
        let server = match name {
            Some(name) => {
                let Some(server) = settings.servers.iter().find(|server| server.name == name)
                else {
                    return;
                };
                Some(server)
            }
            None => None,
        };
        let editor = Editor::new(server, &settings.revision, window, cx);
        if editor.existing.is_none() {
            editor.name.focus_handle(cx).focus(window);
        } else {
            match editor.transport {
                Transport::Http => &editor.url,
                Transport::Stdio => &editor.command,
            }
            .focus_handle(cx)
            .focus(window);
        }
        panel.editor = Some(editor);
        panel.removing = None;
        panel.error = None;
        cx.notify();
    }

    fn save_mcp_editor(&mut self, cx: &mut Context<Self>) {
        let panel = &mut self.providers.mcp;
        let (Some(settings), Some(editor)) = (&panel.settings, &panel.editor) else {
            return;
        };
        match editor.read(settings, cx) {
            Ok(edit) => self.update_mcp_settings(Some(edit), cx),
            Err(error) => {
                panel.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    pub(super) fn render_mcp_settings(&self, cx: &Context<Self>) -> Div {
        let panel = &self.providers.mcp;
        let summary = if panel.error.is_some() {
            Some("Needs attention".into())
        } else if panel.pending.is_some() {
            Some("Updating…".into())
        } else if panel.unavailable {
            Some("Unavailable".into())
        } else {
            panel
                .settings
                .as_ref()
                .map(|settings| match settings.servers.len() {
                    1 => "1 server".into(),
                    count => format!("{count} servers"),
                })
        };
        self.provider_section(
            ProviderSection::Mcp,
            summary,
            || self.mcp_settings_body(cx),
            cx,
        )
    }

    fn mcp_settings_body(&self, cx: &Context<Self>) -> Div {
        let panel = &self.providers.mcp;
        let busy = panel.pending.is_some();
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .flex_wrap()
                    .gap_2()
                    .child(hint("Servers in this Codex instance’s user configuration.", cx))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(control!(
                                "Refresh MCP servers",
                                Button::new("refresh-mcp-settings")
                                    .ghost()
                                    .small()
                                    .icon(Icon::default().path("icons/provider-refresh.svg"))
                                    .tooltip("Refresh MCP servers")
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.request_mcp_settings(cx))
                                    )
                            ))
                            .child(control!(
                                "Add MCP server",
                                Button::new("add-mcp-server")
                                    .outline()
                                    .small()
                                    .icon(IconName::Plus)
                                    .label("Add server")
                                    .disabled(
                                        busy || panel.settings.is_none() || panel.editor.is_some()
                                    )
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.open_mcp_editor(None, window, cx)
                                    }))
                            )),
                    ),
            )
            .when(busy, |section| {
                section.child(control!(
                    "Updating MCP settings",
                    hint("Updating MCP settings…", cx)
                ))
            })
            .when(panel.unavailable, |section| {
                section.child(hint("This Codex version does not expose MCP settings.", cx))
            })
            .when(panel.overridden, |section| {
                section.child(control!(
                    "MCP setting overridden",
                    hint("Saved to the user configuration. A higher-priority Codex setting overrides this change.", cx)
                ))
            })
            .when_some(panel.editor.as_ref(), |section, editor| {
                section.child(self.mcp_editor(editor, cx))
            })
            .when_some(panel.error.as_ref(), |section, error| {
                section.child(control!(
                    "MCP settings error",
                    hint(error.clone(), cx).text_color(cx.theme().danger)
                ))
            })
            .when_some(panel.settings.as_ref(), |section, settings| {
                section
                    .when(
                        settings.servers.is_empty() && panel.editor.is_none(),
                        |section| {
                            section.child(control!(
                                "No user MCP servers",
                                hint("No servers configured.", cx)
                            ))
                        },
                    )
                    .children(
                        settings
                            .servers
                            .iter()
                            .map(|server| self.mcp_server_row(server, cx)),
                    )
            })
    }

    fn mcp_server_row(&self, server: &McpServer, cx: &Context<Self>) -> Div {
        let panel = &self.providers.mcp;
        let name = &server.name;
        let reserved = *name == self.runtime.output().name;
        let locked = panel.pending.is_some() || panel.editor.is_some();
        let removing = panel.removing.as_ref() == Some(name);
        let (toggle, enabled) = if server.enabled {
            ("Disable", false)
        } else {
            ("Enable", true)
        };
        let transport = match server.connection {
            Some(McpConnection::Http { .. }) => "Streamable HTTP",
            Some(McpConnection::Stdio { .. }) => "Stdio",
            None => "Custom",
        };
        let state = if server.enabled {
            "Enabled"
        } else {
            "Disabled"
        };
        let toggle_name = name.clone();
        let edit_name = name.clone();
        let remove_name = name.clone();
        let confirm_name = name.clone();
        v_flex()
            .gap_2()
            .py_1p5()
            .px_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .flex_wrap()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(control!(
                                format!("MCP settings/{name}/{state}"),
                                label(name.clone())
                                    .truncate()
                                    .id(SharedString::from(format!("mcp-name/{name}")))
                                    .tooltip({
                                        let name = name.clone();
                                        move |window, cx| {
                                            gpui_component::tooltip::Tooltip::new(name.clone())
                                                .build(window, cx)
                                        }
                                    })
                            ))
                            .child(hint(
                                if reserved {
                                    "Reserved for Huggler’s built-in tools"
                                } else {
                                    transport
                                },
                                cx,
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(control!(
                                format!("Edit MCP/{name}"),
                                Button::new(SharedString::from(format!("mcp-Edit/{name}")))
                                    .ghost()
                                    .small()
                                    .compact()
                                    .label("Edit")
                                    .disabled(locked || reserved || server.connection.is_none())
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.open_mcp_editor(Some(&edit_name), window, cx)
                                    }))
                            ))
                            .child(control!(
                                format!("Remove MCP/{name}"),
                                Button::new(SharedString::from(format!("mcp-remove/{name}")))
                                    .ghost()
                                    .small()
                                    .compact()
                                    .label("Remove")
                                    .tooltip(format!("Remove {name}"))
                                    .disabled(locked || removing)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.providers.mcp.removing = Some(remove_name.clone());
                                        cx.notify();
                                    }))
                            ))
                            .child(control!(
                                format!("{toggle} MCP/{name}"),
                                Button::new(SharedString::from(format!("mcp-toggle/{name}")))
                                    .ghost()
                                    .small()
                                    .w(px(48.))
                                    .tooltip(format!("{toggle} {name}"))
                                    .child(
                                        Switch::new(SharedString::from(format!(
                                            "mcp-switch/{name}"
                                        )))
                                        .small()
                                        .checked(server.enabled)
                                    )
                                    .disabled(locked || reserved)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.update_mcp_settings(
                                            Some((
                                                toggle_name.clone(),
                                                McpEdit::SetEnabled(enabled),
                                            )),
                                            cx,
                                        );
                                    }))
                            )),
                    ),
            )
            .when(removing, |row| {
                row.child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(label(format!("Remove {name}?")))
                        .child(control!(
                            "Cancel MCP removal",
                            Button::new("cancel-mcp-removal")
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.providers.mcp.removing = None;
                                    cx.notify();
                                }))
                        ))
                        .child(control!(
                            "Confirm MCP removal",
                            Button::new("confirm-mcp-removal")
                                .small()
                                .danger()
                                .label("Remove")
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.update_mcp_settings(
                                        Some((confirm_name.clone(), McpEdit::Remove)),
                                        cx,
                                    );
                                }))
                        )),
                )
            })
    }

    fn mcp_editor(&self, editor: &Editor, cx: &Context<Self>) -> Div {
        let panel = &self.providers.mcp;
        let busy = panel.pending.is_some();
        let adding = editor.existing.is_none();
        let field = |caption: &'static str, input: &Entity<InputState>| {
            v_flex()
                .gap_1()
                .child(label(caption))
                .child(control!(caption, Input::new(input).disabled(busy)))
        };
        let transport_button = |transport, caption| {
            control!(
                caption,
                Button::new(caption)
                    .small()
                    .when(editor.transport == transport, |button| button.primary())
                    .label(caption)
                    .disabled(busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(editor) = &mut view.providers.mcp.editor {
                            editor.transport = transport;
                            view.providers.mcp.error = None;
                            cx.notify();
                        }
                    }))
            )
        };
        v_flex()
            .gap_2()
            .py_2()
            .child(section_title(editor.existing.as_ref().map_or_else(
                || "Add MCP server".into(),
                |name| format!("Edit {name}"),
            )))
            .when(adding, |form| {
                form.child(field("Server name", &editor.name)).child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(transport_button(Transport::Http, "Streamable HTTP"))
                        .child(transport_button(Transport::Stdio, "Stdio")),
                )
            })
            .map(|form| match editor.transport {
                Transport::Http => form.child(field("Server URL", &editor.url)),
                Transport::Stdio => form
                    .child(field("Command", &editor.command))
                    .child(field("Arguments", &editor.args)),
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(control!(
                        "Cancel MCP edit",
                        Button::new("cancel-mcp-edit")
                            .small()
                            .label("Cancel")
                            .disabled(busy)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.providers.mcp.editor = None;
                                view.providers.mcp.error = None;
                                cx.notify();
                            }))
                    ))
                    .child(control!(
                        "Save MCP server",
                        Button::new("save-mcp-server")
                            .small()
                            .primary()
                            .label("Save server")
                            .disabled(busy || panel.settings.is_none())
                            .on_click(cx.listener(|view, _, _, cx| view.save_mcp_editor(cx)))
                    )),
            )
    }
}
