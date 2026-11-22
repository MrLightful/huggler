use super::{
    projects::{ProjectAction, SavedProject},
    *,
};

pub(super) struct ActionEditor {
    index: Option<usize>,
    name: Entity<InputState>,
    command: Entity<InputState>,
}

impl ActionEditor {
    fn action(&self, cx: &App) -> Option<ProjectAction> {
        let name = self.name.read(cx).value();
        let command = self.command.read(cx).value();
        let name = name.trim();
        if name.is_empty() || command.trim().is_empty() || command.contains('\0') {
            return None;
        }
        Some(ProjectAction {
            name: name.to_owned(),
            command: command.to_string(),
        })
    }
}

impl Settings {
    pub(crate) fn open_project_actions(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.action_project = root;
        self.show_page(Page::ProjectActions, window, cx);
        self.error = None;
        cx.notify();
    }

    fn action_project(&self) -> Option<&SavedProject> {
        self.prefs
            .projects
            .iter()
            .find(|project| project.root == self.action_project)
    }

    fn edit_project_action(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = index.and_then(|index| self.action_project()?.actions.get(index));
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Run")
                .default_value(action.map_or_else(String::new, |action| action.name.clone()))
        });
        let command = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(3, 8)
                .placeholder("Enter a shell command")
                .default_value(action.map_or_else(String::new, |action| action.command.clone()))
        });
        for input in [&name, &command] {
            cx.subscribe(input, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }
        name.focus_handle(cx).focus(window);
        self.action_editor = Some(ActionEditor {
            index,
            name,
            command,
        });
        cx.notify();
    }

    fn save_project_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.action_editor.as_ref() else {
            return;
        };
        let Some(action) = editor.action(cx) else {
            return;
        };
        let index = editor.index;
        let mut prefs = self.prefs.clone();
        let project_index = prefs
            .projects
            .iter()
            .position(|project| project.root == self.action_project)
            .unwrap_or_else(|| {
                prefs
                    .projects
                    .push(SavedProject::new(self.action_project.clone()));
                prefs.projects.len() - 1
            });
        let actions = &mut prefs.projects[project_index].actions;
        if let Some(index) = index {
            actions[index] = action;
        } else {
            actions.push(action);
        }
        if self.persist_projects(prefs, cx).is_ok() {
            self.action_editor = None;
            self.focus.focus(window);
        }
    }

    fn update_project_actions(
        &mut self,
        update: impl FnOnce(&mut Vec<ProjectAction>),
        cx: &mut Context<Self>,
    ) {
        let mut prefs = self.prefs.clone();
        if let Some(project) = prefs
            .projects
            .iter_mut()
            .find(|project| project.root == self.action_project)
        {
            update(&mut project.actions);
            let _ = self.persist_projects(prefs, cx);
        }
    }

    pub(super) fn project_actions_panel(&self, cx: &Context<Self>) -> Div {
        let project = self.action_project();
        let name = project
            .map(|project| project.name.as_str())
            .or_else(|| {
                self.action_project
                    .file_name()
                    .and_then(|name| name.to_str())
            })
            .unwrap_or("Project");
        let actions = project.map_or(&[][..], |project| project.actions.as_slice());
        let mut panel = v_flex()
            .gap_3()
            .child(heading(name.to_owned()))
            .child(hint(self.action_project.display().to_string(), cx))
            .child(hint(
                "The default action appears on the terminal’s play button.",
                cx,
            ));
        if let Some(editor) = &self.action_editor {
            panel = panel.child(
                v_flex()
                    .gap_2()
                    .child(label("Action name"))
                    .child(control!("Action name", Input::new(&editor.name)))
                    .child(label("Shell command"))
                    .child(control!("Action command", Input::new(&editor.command)))
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(control!(
                                "Cancel action edit",
                                Button::new("cancel-edit")
                                    .small()
                                    .ghost()
                                    .label("Cancel")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.action_editor = None;
                                        view.focus.focus(window);
                                        cx.notify();
                                    }))
                            ))
                            .child(control!(
                                "Save action",
                                Button::new("save-action")
                                    .small()
                                    .primary()
                                    .label("Save action")
                                    .disabled(editor.action(cx).is_none())
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.save_project_action(window, cx)
                                    }))
                            )),
                    ),
            );
        } else {
            panel = panel
                .when(actions.is_empty(), |panel| {
                    panel.child(hint("No actions configured.", cx))
                })
                .children(
                    actions
                        .iter()
                        .enumerate()
                        .map(|(index, action)| self.project_action_row(index, action, cx)),
                )
                .child(h_flex().justify_end().child(control!(
                        "Add action",
                        Button::new("add-action")
                            .small()
                            .icon(IconName::Plus)
                            .label("Add action")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.edit_project_action(None, window, cx)
                            }))
                    )));
        }
        panel
    }

    fn project_action_row(
        &self,
        index: usize,
        action: &ProjectAction,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .id(("project-action", index))
            .gap_1()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(label(action.name.clone()).min_w_0().truncate())
                    .when(index == 0, |row| row.child(hint("Default", cx)))
                    .child(div().flex_1())
                    .when(index > 0, |row| {
                        row.child(control!(
                            format!("Make default/{}", action.name),
                            Button::new("make-default")
                                .ghost()
                                .small()
                                .label("Make default")
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.update_project_actions(
                                        |actions| actions[..=index].rotate_right(1),
                                        cx,
                                    );
                                }))
                        ))
                    })
                    .child(control!(
                        format!("Edit action/{}", action.name),
                        Button::new("edit-action")
                            .ghost()
                            .small()
                            .label("Edit")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.edit_project_action(Some(index), window, cx)
                            }))
                    ))
                    .child(control!(
                        format!("Remove action/{}", action.name),
                        Button::new("remove-action")
                            .ghost()
                            .small()
                            .label("Remove")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.update_project_actions(
                                    |actions| {
                                        actions.remove(index);
                                    },
                                    cx,
                                );
                            }))
                    )),
            )
            .child(
                hint(action.command.clone(), cx)
                    .font_family(cx.theme().mono_font_family.clone())
                    .truncate(),
            )
    }
}
