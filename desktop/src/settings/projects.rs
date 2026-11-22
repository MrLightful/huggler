use super::*;
use crate::git;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SavedProject {
    pub name: String,
    pub root: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ProjectAction>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ProjectAction {
    pub name: String,
    pub command: String,
}

impl SavedProject {
    pub(super) fn new(root: PathBuf) -> Self {
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Project")
            .to_owned();
        Self {
            name,
            root,
            actions: Vec::new(),
        }
    }
}

pub(crate) fn pick_checkout(cx: &mut App) -> impl Future<Output = Result<Option<PathBuf>>> + use<> {
    let executor = cx.background_executor().clone();
    #[cfg(all(feature = "qualification", target_os = "macos"))]
    let scripted = crate::qualification::take_file_selection(cx);
    #[cfg(not(all(feature = "qualification", target_os = "macos")))]
    let scripted: Option<Vec<PathBuf>> = None;
    let choice = scripted.is_none().then(|| {
        cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose Git checkout".into()),
        })
    });
    async move {
        let paths = match choice {
            Some(choice) => choice.await??,
            None => scripted,
        };
        let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
            return Ok(None);
        };
        executor
            .spawn(async move {
                let root = git::checkout_root(&path)
                    .and_then(|root| Ok(root.canonicalize()?))
                    .with_context(|| {
                        format!("Could not open a Git checkout at {}", path.display())
                    })?;
                ensure!(root.to_str().is_some(), "Git checkout path requires UTF-8");
                Ok(Some(root))
            })
            .await
    }
}

impl Settings {
    pub(crate) fn saved_projects(&self) -> &[SavedProject] {
        &self.prefs.projects
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn worktree_project(&self, worktree: &Path) -> Option<&SavedProject> {
        let root = self.prefs.worktree_projects.get(worktree)?;
        self.prefs
            .projects
            .iter()
            .find(|project| project.root == *root)
    }

    pub(crate) fn save_worktree_project(
        &mut self,
        worktree: PathBuf,
        root: PathBuf,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let mut prefs = self.prefs.clone();
        prefs.worktree_projects.insert(worktree, root);
        self.persist_projects(prefs, cx)
    }

    pub(crate) fn save_project(&mut self, root: PathBuf, cx: &mut Context<Self>) -> Result<()> {
        if self
            .prefs
            .projects
            .iter()
            .any(|project| project.root == root)
        {
            return Ok(());
        }
        let mut prefs = self.prefs.clone();
        prefs.projects.push(SavedProject::new(root));
        self.persist_projects(prefs, cx)
    }

    pub(super) fn persist_projects(
        &mut self,
        mut prefs: Preferences,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        prefs.projects.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.root.cmp(&b.root))
        });
        if self.persist(prefs, cx) {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "{}",
                self.error.as_deref().unwrap_or("Could not save projects")
            ))
        }
    }

    fn add_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.adding_project {
            return;
        }
        self.adding_project = true;
        self.error = None;
        let checkout = pick_checkout(cx);
        cx.spawn_in(window, async move |view, cx| {
            let result = checkout.await;
            let _ = view.update_in(cx, |view, _, cx| {
                view.adding_project = false;
                view.error = result
                    .and_then(|root| {
                        if let Some(root) = root {
                            view.save_project(root, cx)?;
                        }
                        Ok(())
                    })
                    .err()
                    .map(|error| format!("{error:#}"));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn rename_project(&mut self, cx: &mut Context<Self>) {
        let Some((root, input)) = &self.project_editor else {
            return;
        };
        let name = input.read(cx).value();
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let mut prefs = self.prefs.clone();
        if let Some(project) = prefs
            .projects
            .iter_mut()
            .find(|project| project.root == *root)
        {
            project.name = name.to_owned();
            if self.persist_projects(prefs, cx).is_ok() {
                self.project_editor = None;
            }
        }
    }

    pub(super) fn projects_panel(&self, cx: &Context<Self>) -> Div {
        v_flex()
            .gap_3()
            .child(h_flex().justify_end().child(control!(
                "Add project",
                Button::new("add-project")
                    .small()
                    .icon(IconName::Plus)
                    .label(if self.adding_project { "Opening checkout…" } else { "Add project" })
                    .disabled(self.adding_project)
                    .on_click(cx.listener(|view, _, window, cx| view.add_project(window, cx)))
            )))
            .when(self.prefs.projects.is_empty(), |panel| {
                panel.child(hint(
                    "Add a Git checkout to use it in new conversations.",
                    cx,
                ))
            })
            .when(!self.prefs.projects.is_empty(), |panel| {
                panel.child(
                    v_flex()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().sidebar)
                        .children(self.prefs.projects.iter().enumerate().map(
                            |(index, project)| {
                                v_flex()
                                    .when(index > 0, |row| {
                                        row.border_t_1().border_color(cx.theme().border)
                                    })
                                    .child(self.project_row(project, cx))
                            },
                        )),
                )
            })
    }

    fn project_row(&self, project: &SavedProject, cx: &Context<Self>) -> impl IntoElement {
        let root = project.root.clone();
        let name = project.name.clone();
        let mut row = v_flex()
            .id(SharedString::from(root.to_string_lossy().into_owned()))
            .gap_1()
            .px_3()
            .py_2();
        if let Some((_, input)) = self
            .project_editor
            .as_ref()
            .filter(|(root, _)| *root == project.root)
        {
            row = row.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(128.))
                            .child(control!("Project name", Input::new(input))),
                    )
                    .child(control!(
                        "Save name",
                        Button::new("save-name")
                            .small()
                            .label("Save name")
                            .disabled(input.read(cx).value().trim().is_empty())
                            .on_click(cx.listener(|view, _, _, cx| view.rename_project(cx)))
                    ))
                    .child(control!(
                        "Cancel rename",
                        Button::new("cancel-rename")
                            .small()
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.project_editor = None;
                                cx.notify();
                            }))
                    )),
            );
        } else {
            row = row.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(label(name.clone()).flex_1().min_w_0().truncate())
                    .child(control!(
                        format!("Project actions/{name}"),
                        Button::new("project-actions")
                            .small()
                            .ghost()
                            .label("Actions")
                            .on_click(cx.listener({
                                let root = root.clone();
                                move |view, _, window, cx| {
                                    view.open_project_actions(root.clone(), window, cx);
                                }
                            }))
                    ))
                    .child(control!(
                        format!("Rename project/{name}"),
                        Button::new("rename-project")
                            .small()
                            .ghost()
                            .label("Rename")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                let input = cx.new(|cx| {
                                    InputState::new(window, cx).default_value(name.clone())
                                });
                                cx.subscribe(&input, |view, _, event, cx| match event {
                                    InputEvent::Change => cx.notify(),
                                    InputEvent::PressEnter { .. } => view.rename_project(cx),
                                    _ => (),
                                })
                                .detach();
                                input.focus_handle(cx).focus(window);
                                view.project_editor = Some((root.clone(), input));
                                cx.notify();
                            }))
                    ))
                    .child(control!(
                        format!("Remove project/{}", project.name),
                        Button::new("remove-project")
                            .small()
                            .ghost()
                            .label("Remove")
                            .tooltip("Remove from saved projects")
                            .on_click(cx.listener({
                                let root = project.root.clone();
                                move |view, _, _, cx| {
                                    let mut prefs = view.prefs.clone();
                                    prefs.projects.retain(|project| project.root != root);
                                    let _ = view.persist_projects(prefs, cx);
                                }
                            }))
                    )),
            );
        }
        row.child(hint(project.root.display().to_string(), cx).truncate())
    }
}
