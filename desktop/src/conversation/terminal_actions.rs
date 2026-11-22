use super::{terminal_pane::TerminalLaunch, *};
use crate::settings::projects::ProjectAction;
use gpui_component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};

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
                        view.launch_project_action(&thread, &action, window, cx);
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
                    view.open_settings(SettingsTarget::ProjectActions(root), window, cx);
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
            |action| format!("Run {}", action.name),
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
                                view.launch_project_action(&primary_thread, action, window, cx);
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

    fn launch_project_action(
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
        if !action.command.trim().is_empty() && !action.command.contains('\0') {
            self.open_terminal(thread, TerminalLaunch::Action(&action.command), window, cx);
        }
    }
}
