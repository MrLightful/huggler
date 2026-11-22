use super::*;
use crate::settings::projects::{SavedProject, pick_checkout};
use gpui_component::{
    IndexPath,
    list::{List, ListDelegate, ListEvent, ListItem},
    popover::Popover,
    tooltip::Tooltip,
};
use std::path::Path;

pub(super) struct ProjectPicker {
    projects: Vec<SavedProject>,
    selected: PathBuf,
    matches: Vec<usize>,
}

fn project_path_label(path: &Path) -> String {
    if let Some(relative) = dirs::home_dir().and_then(|home| path.strip_prefix(home).ok()) {
        return Path::new("~").join(relative).display().to_string();
    }
    let full = path.to_string_lossy();
    if full.chars().count() <= 42 {
        return full.into_owned();
    }
    let start = full
        .char_indices()
        .rev()
        .nth(39)
        .map_or(0, |(index, _)| index);
    format!("…{}", &full[start..])
}

impl ListDelegate for ProjectPicker {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.matches.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let project = &self.projects[*self.matches.get(ix.row)?];
        let path = project.root.display().to_string();
        let row = h_flex()
            .id("project-row")
            .w_full()
            .min_w_0()
            .gap_2()
            .child(Icon::new(IconName::Folder).small().flex_shrink_0())
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .child(label(project.name.clone()).w_full().truncate())
                    .child(
                        hint(project_path_label(&project.root), cx)
                            .w_full()
                            .text_xs()
                            .truncate(),
                    ),
            )
            .child(
                div()
                    .w_5()
                    .flex_shrink_0()
                    .when(project.root == self.selected, |mark| {
                        mark.child(Icon::new(IconName::Check).small())
                    }),
            )
            .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx));
        Some(
            ListItem::new(ix.row)
                .rounded_md()
                .child(control!(format!("project/{}", project.name), row)),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        hint(
            if self.projects.is_empty() {
                "No saved projects"
            } else {
                "No matching projects"
            },
            cx,
        )
        .p_2()
    }

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        let query = query.to_lowercase();
        self.matches = self
            .projects
            .iter()
            .enumerate()
            .filter_map(|(index, project)| {
                (project.name.to_lowercase().contains(&query)
                    || project
                        .root
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query))
                .then_some(index)
            })
            .collect();
        Task::ready(())
    }

    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
    }
}

impl Desktop {
    pub(super) fn launch_checkout(&self, cx: &App) -> PathBuf {
        if let Some(path) = &self.picked_checkout {
            return path.clone();
        }
        let checkout = self.checkout.read(cx).value();
        let path = Path::new(checkout.as_str());
        if self
            .settings
            .read(cx)
            .saved_projects()
            .iter()
            .any(|project| project.root == path)
        {
            path.into()
        } else {
            PathBuf::from(checkout.trim())
        }
    }

    pub(super) fn launch_project_name(&self, cx: &App) -> Option<String> {
        let root = self.launch_checkout(cx);
        self.settings
            .read(cx)
            .saved_projects()
            .iter()
            .find(|project| project.root == root)
            .map(|project| project.name.clone())
            .or_else(|| {
                root.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
    }

    pub(super) fn project_control(&self, window: &Window, cx: &Context<Self>) -> Div {
        let trigger = Button::new("choose-project")
            .small()
            .ghost()
            .icon(IconName::Folder)
            .dropdown_caret(true)
            .tooltip("Choose project")
            .child(
                div().max_w(px(200.)).truncate().child(
                    self.launch_project_name(cx)
                        .unwrap_or_else(|| "Choose project".into()),
                ),
            )
            .on_click(cx.listener(|view, event: &ClickEvent, window, cx| {
                if event.is_keyboard() {
                    if view.project_picker.is_some() {
                        view.close_project_picker(window, cx);
                    } else {
                        view.open_project_picker(window, cx);
                    }
                }
            }));
        let picker = Popover::new("project-picker")
            .anchor(Corner::TopLeft)
            .p_1()
            .trigger(trigger)
            .open(self.project_picker.is_some())
            .on_open_change(cx.listener(|view, open, window, cx| {
                if *open {
                    view.open_project_picker(window, cx);
                } else {
                    view.close_project_picker(window, cx);
                }
            }));
        let picker = if let Some(list) = &self.project_picker {
            let viewport = window.viewport_size();
            let width = px(336.).min(viewport.width - px(24.));
            let height = (viewport.height / 2. - px(120.)).clamp(px(88.), px(216.));
            picker.child(
                v_flex()
                    .w(width)
                    .child(
                        List::new(list)
                            .search_placeholder("Search projects")
                            .max_h(height),
                    )
                    .child(
                        v_flex()
                            .pt_1()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .child(control!(
                                "Add project",
                                Button::new("add-project")
                                    .small()
                                    .ghost()
                                    .w_full()
                                    .justify_start()
                                    .icon(IconName::Plus)
                                    .label("Add project")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.browse_project(true, window, cx)
                                    }))
                            ))
                            .child(control!(
                                "Start without a project",
                                Button::new("projectless")
                                    .small()
                                    .ghost()
                                    .w_full()
                                    .justify_start()
                                    .icon(IconName::Minus)
                                    .label("Start without a project")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.choose_checkout(PathBuf::new(), window, cx);
                                    }))
                            )),
                    ),
            )
        } else {
            picker
        };
        control!("Choose project", picker)
    }

    pub(super) fn open_project_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.suggestions = Default::default();
        self.model_picker = None;
        self.mcp_status = None;
        self.launch_workspace.close_picker();
        let projects = self.settings.read(cx).saved_projects().to_vec();
        let selected = self.launch_checkout(cx);
        let list = cx.new(|cx| {
            let matches = (0..projects.len()).collect();
            let mut state = ListState::new(
                ProjectPicker {
                    projects,
                    selected,
                    matches,
                },
                window,
                cx,
            )
            .searchable(true);
            state.set_selected_index(Some(IndexPath::default()), window, cx);
            state
        });
        cx.subscribe_in(&list, window, |view, list, event, window, cx| {
            if view.project_picker.as_ref() != Some(list) {
                return;
            }
            match event {
                ListEvent::Confirm(ix) => {
                    let picker = list.read(cx).delegate();
                    if let Some(index) = picker.matches.get(ix.row) {
                        view.choose_checkout(picker.projects[*index].root.clone(), window, cx);
                    }
                }
                ListEvent::Cancel => view.close_project_picker(window, cx),
                ListEvent::Select(_) => (),
            }
        })
        .detach();
        list.focus_handle(cx).focus(window);
        self.project_picker = Some(list);
        cx.notify();
    }

    fn close_project_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project_picker.take().is_some() {
            self.focus_workspace(window, cx);
            cx.notify();
        }
    }

    pub(super) fn browse_project(
        &mut self,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_project_picker(window, cx);
        if self.checkout_browse.is_some() {
            return;
        }
        let checkout = pick_checkout(cx);
        self.checkout_browse = Some(cx.spawn_in(window, async move |view, cx| {
            let result = checkout.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.checkout_browse = None;
                match result {
                    Ok(Some(root)) => {
                        if save
                            && let Err(error) = view
                                .settings
                                .update(cx, |settings, cx| settings.save_project(root.clone(), cx))
                        {
                            view.error = Some(format!("{error:#}"));
                        } else {
                            view.choose_checkout(root, window, cx);
                        }
                    }
                    Ok(None) => (),
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
    }
}
