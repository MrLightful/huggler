use super::{terminal_pane::TerminalLaunch, *};
use crate::settings::projects::ProjectAction;
use gpui_component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};

actions!(huggler_desktop, [RunTerminalCommand, CancelTerminalCommand]);

pub(crate) fn bind_keys(cx: &mut App) {
    let input = Some("HugglerTerminalCommand > Input");
    cx.bind_keys([
        KeyBinding::new("enter", RunTerminalCommand, input),
        KeyBinding::new("escape", CancelTerminalCommand, input),
        KeyBinding::new(
            "shift-enter",
            gpui_component::input::Enter { secondary: true },
            input,
        ),
    ]);
}

pub(super) struct StagedAction {
    name: String,
    command: Entity<InputState>,
}

fn play_icon() -> Icon {
    Icon::default().path("icons/play.svg")
}

fn action_menu(
    mut menu: PopupMenu,
    view: &WeakEntity<Desktop>,
    thread: &ThreadId,
    root: &std::path::Path,
    actions: &[ProjectAction],
) -> PopupMenu {
    for (index, action) in actions.iter().enumerate() {
        let action = action.clone();
        let view = view.clone();
        let thread = thread.clone();
        let name = action.name.clone();
        menu = menu.item(
            PopupMenuItem::element(move |_, _| {
                control!(
                    format!("Terminal action/{name}"),
                    div().w_full().min_w_0().truncate().child(name.clone())
                )
            })
            .icon(play_icon())
            .checked(index == 0)
            .on_click(move |_, window, cx| {
                let (view, thread, action) = (view.clone(), thread.clone(), action.clone());
                window.defer(cx, move |window, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.stage_project_action(&thread, &action, window, cx);
                    });
                });
            }),
        );
    }
    if !actions.is_empty() {
        menu = menu.separator();
    }
    let view = view.clone();
    let root = root.to_path_buf();
    menu.min_w(px(150.)).max_w(px(280.)).scrollable(true).item(
        PopupMenuItem::element(|_, _| {
            control!(
                "Configure project actions",
                div().w_full().child("Configure")
            )
        })
        .icon(IconName::Settings)
        .on_click(move |_, window, cx| {
            let (view, root) = (view.clone(), root.clone());
            window.defer(cx, move |window, cx| {
                let _ = view.update(cx, |view, cx| {
                    view.open_settings(window, cx);
                    view.settings.update(cx, |settings, cx| {
                        settings.open_project_actions(root, window, cx);
                    });
                });
            });
        }),
    )
}

impl Desktop {
    pub(super) fn resolve_terminal_project(&self, thread: ThreadId, cx: &mut Context<Self>) {
        let root = self.workspaces[&thread].root.clone();
        let lookup = cx.background_spawn(async move { git::main_worktree(&root).ok() });
        cx.spawn(async move |view, cx| {
            let main_worktree = lookup.await;
            let _ = view.update(cx, |view, cx| {
                if let Some(workspace) = view.workspaces.get_mut(&thread) {
                    workspace.terminal.main_worktree = main_worktree;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn terminal_action_picker(
        &self,
        thread: &ThreadId,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let workspace = &self.workspaces[thread];
        let settings = self.settings.read(cx);
        let projects = settings.saved_projects();
        let project = projects
            .iter()
            .find(|project| project.root == workspace.root)
            .or_else(|| settings.worktree_project(&workspace.root));
        let root = project
            .map(|project| &project.root)
            .or(workspace.terminal.main_worktree.as_ref())?;
        let actions = project
            .or_else(|| projects.iter().find(|project| project.root == *root))
            .map(|project| project.actions.clone())
            .unwrap_or_default();
        let primary = actions.first().cloned();
        let label = primary.as_ref().map_or_else(
            || "No default project action".into(),
            |action| format!("Prefill {}", action.name),
        );
        let root = root.clone();
        let primary_thread = thread.clone();
        let menu_thread = thread.clone();
        let view = cx.weak_entity();
        let theme = cx.theme();

        Some(
            h_flex()
                .id(SharedString::from(format!("terminal-actions-{}", thread.0)))
                .flex_shrink_0()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .child(control!(
                    label.clone(),
                    Button::new("default-action")
                        .ghost()
                        .small()
                        .compact()
                        .icon(play_icon())
                        .tooltip(label)
                        .disabled(primary.is_none())
                        .on_click(cx.listener(move |view, _, window, cx| {
                            if let Some(action) = &primary {
                                view.stage_project_action(&primary_thread, action, window, cx);
                            }
                        }))
                ))
                .child(
                    div()
                        .border_l_1()
                        .border_color(theme.border)
                        .child(control!(
                            "Terminal project actions",
                            Button::new("action-menu")
                                .ghost()
                                .small()
                                .compact()
                                .icon(IconName::ChevronDown)
                                .tooltip("Project actions")
                                .dropdown_menu_with_anchor(Corner::TopRight, move |menu, _, _| {
                                    action_menu(menu, &view, &menu_thread, &root, &actions)
                                },)
                        )),
                ),
        )
    }

    fn stage_project_action(
        &mut self,
        thread: &ThreadId,
        action: &ProjectAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .current
            .as_ref()
            .is_none_or(|current| current.id != *thread)
        {
            return;
        }
        let command = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(1, 2)
                .default_value(action.command.clone())
        });
        cx.subscribe(&command, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        command.focus_handle(cx).focus(window);
        self.workspaces
            .get_mut(thread)
            .unwrap()
            .terminal
            .staged_action = Some(StagedAction {
            name: action.name.clone(),
            command,
        });
        cx.notify();
    }

    fn launch_staged_action(
        &mut self,
        thread: &ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = &mut self.workspaces.get_mut(thread).unwrap().terminal;
        let Some(action) = &pane.staged_action else {
            return;
        };
        let command = action.command.read(cx).value();
        if !command.trim().is_empty() && !command.contains('\0') {
            pane.staged_action = None;
            self.open_terminal(thread, TerminalLaunch::Action(&command), window, cx);
        }
    }

    fn cancel_staged_action(
        &mut self,
        thread: &ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = &mut self.workspaces.get_mut(thread).unwrap().terminal;
        pane.staged_action = None;
        pane.focus_active(window, cx);
        cx.notify();
    }
}

impl StagedAction {
    pub(super) fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.command.focus_handle(cx).is_focused(window)
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &App) {
        self.command.focus_handle(cx).focus(window);
    }

    pub(super) fn render(&self, thread: &ThreadId, cx: &Context<Desktop>) -> impl IntoElement {
        let command = self.command.read(cx).value();
        let run_thread = thread.clone();
        let cancel_thread = thread.clone();
        let run_key_thread = thread.clone();
        let cancel_key_thread = thread.clone();
        v_flex()
            .id("staged-terminal-command")
            .key_context("HugglerTerminalCommand")
            .flex_shrink_0()
            .gap_1()
            .px_2()
            .py_1()
            .bg(cx.theme().sidebar)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(hint(self.name.clone(), cx).truncate())
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        control!(
                            "Prefilled terminal command",
                            Input::new(&self.command).small()
                        )
                        .min_w_0()
                        .flex_1(),
                    )
                    .child(control!(
                        "Run terminal command",
                        Button::new("run-command")
                            .small()
                            .compact()
                            .primary()
                            .icon(play_icon())
                            .tooltip("Run command in a new terminal")
                            .disabled(command.trim().is_empty() || command.contains('\0'))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.launch_staged_action(&run_thread, window, cx)
                            }))
                    ))
                    .child(control!(
                        "Cancel terminal command",
                        Button::new("cancel-command")
                            .small()
                            .compact()
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip("Cancel command")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.cancel_staged_action(&cancel_thread, window, cx)
                            }))
                    )),
            )
            .on_action(
                cx.listener(move |view, _: &RunTerminalCommand, window, cx| {
                    view.launch_staged_action(&run_key_thread, window, cx);
                }),
            )
            .on_action(
                cx.listener(move |view, _: &CancelTerminalCommand, window, cx| {
                    view.cancel_staged_action(&cancel_key_thread, window, cx);
                }),
            )
    }
}
