use super::*;
use gpui_component::{
    IndexPath,
    list::{List, ListDelegate, ListEvent, ListItem},
    menu::{DropdownMenu, PopupMenuItem},
    popover::Popover,
    spinner::Spinner,
};
use huggler::{Project, workspace::WorktreeBranch};
use markdown::{ParseOptions, mdast::Node, to_mdast};
use std::time::Duration;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum WorkspaceMode {
    Current,
    #[default]
    Worktree,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum WorktreeStage {
    ResolveBase,
    Checkout,
    OpenConversation,
}

pub(super) struct LaunchProgress {
    pub thread: ThreadId,
    pub title: String,
    pub stage: Option<WorktreeStage>,
}

impl LaunchProgress {
    pub(super) fn render(&self, cx: &App) -> Div {
        let view = v_flex().size_full().items_center().justify_center().p_4();
        let Some(current) = self.stage else {
            return view.child(label("Opening conversation…"));
        };
        let theme = cx.theme();
        let steps = [
            (WorktreeStage::ResolveBase, "Select base branch"),
            (WorktreeStage::Checkout, "Create branch and check out files"),
            (WorktreeStage::OpenConversation, "Open conversation"),
        ];
        view.child(
            v_flex()
                .w_full()
                .max_w(px(420.))
                .gap_4()
                .p_5()
                .rounded_xl()
                .border_1()
                .border_color(theme.border)
                .child(
                    v_flex()
                        .gap_1()
                        .child(control!(
                            "Setting up worktree",
                            crate::ui::heading("Setting up worktree")
                        ))
                        .child(hint(self.title.clone(), cx).truncate()),
                )
                .child(
                    v_flex()
                        .gap_3()
                        .pt_4()
                        .border_t_1()
                        .border_color(theme.border)
                        .children(steps.into_iter().map(|(stage, name)| {
                            let (indicator, status) = match stage.cmp(&current) {
                                std::cmp::Ordering::Less => (
                                    Icon::new(IconName::Check)
                                        .with_size(px(14.))
                                        .into_any_element(),
                                    "Done",
                                ),
                                std::cmp::Ordering::Equal => (
                                    Spinner::new().with_size(px(14.)).into_any_element(),
                                    "In progress",
                                ),
                                std::cmp::Ordering::Greater => (
                                    Icon::new(IconName::Minus)
                                        .with_size(px(14.))
                                        .into_any_element(),
                                    "Waiting",
                                ),
                            };
                            control!(
                                format!("{name}/{status}"),
                                h_flex()
                                    .gap_2()
                                    .child(div().flex_shrink_0().child(indicator))
                                    .child(label(name).flex_1())
                                    .child(hint(status, cx).flex_shrink_0())
                            )
                        })),
                ),
        )
    }
}

pub(super) fn conversation_title(message: &str, attachment: Option<&str>) -> String {
    fn append_inline_text(node: &Node, text: &mut String) {
        match node {
            Node::Break(_) => text.push('\n'),
            Node::Image(image) => text.push_str(&image.alt),
            Node::ImageReference(image) => text.push_str(&image.alt),
            Node::Html(_) => (),
            _ => {
                if let Some(children) = node.children() {
                    for child in children {
                        append_inline_text(child, text);
                    }
                } else {
                    text.push_str(&node.to_string());
                }
            }
        }
    }

    fn first_prose(node: &Node) -> Option<String> {
        match node {
            Node::Paragraph(_) | Node::Heading(_) => {
                let mut text = String::new();
                append_inline_text(node, &mut text);
                (!text.trim().is_empty()).then_some(text)
            }
            Node::Root(_) | Node::List(_) | Node::ListItem(_) => {
                node.children()?.iter().find_map(first_prose)
            }
            _ => None,
        }
    }

    let message = message.trim();
    let prefix = &message[..message.floor_char_boundary(message.len().min(4096))];
    let prose = match to_mdast(prefix, &ParseOptions::gfm()) {
        Ok(root) => first_prose(&root).unwrap_or_default(),
        Err(_) => prefix.to_owned(),
    };
    let first_line = prose
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let line = first_line.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = line
        .match_indices(['.', '!', '?'])
        .find(|&(index, mark)| line[index + mark.len()..].starts_with(' '))
        .map_or(line.len(), |(index, _)| index);
    let mut title = line[..end].to_owned();
    let lower = title.to_ascii_lowercase();
    let prefixes = [
        "please ",
        "can you ",
        "could you ",
        "would you ",
        "i would like you to ",
        "i'd like you to ",
        "i would like to ",
        "i'd like to ",
    ];
    let mut rest = lower.as_str();
    while let Some(stripped) = prefixes.iter().find_map(|prefix| rest.strip_prefix(prefix)) {
        rest = stripped;
    }
    if rest.len() < lower.len() {
        let mut chars = title[lower.len() - rest.len()..].chars();
        if let Some(first) = chars.next() {
            title = first.to_uppercase().chain(chars).collect();
        }
    }
    let title = title.trim_end_matches(['.', '!', '?']).trim_end();
    let title = if title.is_empty() {
        attachment
            .filter(|name| !name.is_empty())
            .unwrap_or("New conversation")
    } else {
        title
    };
    match title.char_indices().nth(72) {
        Some((end, _)) => {
            let prefix = &title[..end];
            let (earliest, _) = prefix.char_indices().nth(48).unwrap();
            let cut = prefix[earliest..]
                .rfind(char::is_whitespace)
                .map(|cut| earliest + cut);
            format!("{}…", &prefix[..cut.unwrap_or(end)])
        }
        None => title.to_owned(),
    }
}

#[cfg(test)]
mod title_tests {
    use super::conversation_title;

    #[test]
    fn derives_the_title_from_the_first_task() {
        for (message, attachment, expected) in [
            (
                "Could you please **fix the parser**? Add a test.",
                None,
                "Fix the parser",
            ),
            (
                "Update [parser.rs](src/parser.rs)  \nKeep the fixtures.",
                None,
                "Update parser.rs",
            ),
            (
                "```rust\nfn main() {}\n```",
                Some("fixture.rs"),
                "fixture.rs",
            ),
        ] {
            assert_eq!(conversation_title(message, attachment), expected);
        }
    }
}

impl WorkspaceMode {
    fn label(self) -> &'static str {
        match self {
            Self::Current => "Current checkout",
            Self::Worktree => "New worktree",
        }
    }
}

#[derive(Default)]
pub(super) struct LaunchWorkspace {
    mode: WorkspaceMode,
    base: Option<git::Branch>,
    checkout_branch: Option<git::Branch>,
    branches: Option<anyhow::Result<git::Branches>>,
    picker: Option<Entity<ListState<BranchPicker>>>,
    picker_load: Option<Task<()>>,
    load: Option<Task<()>>,
}

impl LaunchWorkspace {
    pub(super) fn mode(&self) -> WorkspaceMode {
        if matches!(&self.branches, Some(Ok(branches)) if branches.default.is_none()) {
            WorkspaceMode::Current
        } else {
            self.mode
        }
    }

    pub(super) fn requested_mode(&self) -> WorkspaceMode {
        self.mode
    }

    pub(super) fn selected_branch(&self, mode: WorkspaceMode) -> Option<&git::Branch> {
        match mode {
            WorkspaceMode::Current => self.checkout_branch.as_ref(),
            WorkspaceMode::Worktree => self.base.as_ref(),
        }
    }

    fn branches(&self) -> Option<&git::Branches> {
        self.branches.as_ref()?.as_ref().ok()
    }

    fn displayed_branch(&self, mode: WorkspaceMode) -> Option<&git::Branch> {
        self.selected_branch(mode).or_else(|| {
            let branches = self.branches()?;
            match mode {
                WorkspaceMode::Current => branches.branches.iter().find(|branch| branch.current),
                WorkspaceMode::Worktree => branches.default.as_ref(),
            }
        })
    }

    pub(super) fn close_picker(&mut self) {
        self.picker = None;
        self.picker_load = None;
    }
}

pub(super) fn prepare(
    runtime: &Huggler,
    source: Project,
    mode: WorkspaceMode,
    selected: Option<git::Branch>,
    branch: &str,
    title: &str,
    report_stage: impl Fn(Option<WorktreeStage>),
) -> anyhow::Result<(Project, bool)> {
    if mode == WorkspaceMode::Current {
        if let Some(branch) = selected {
            git::switch_branch(&source.root, &branch.name)?;
        }
        return Ok((source, false));
    }
    let base = match selected {
        Some(base) => Some(base),
        None => git::branches(&source.root)?.default,
    };
    let Some(base) = base else {
        report_stage(None);
        return Ok((source, false));
    };
    let branch = if branch.is_empty() {
        WorktreeBranch::FromTitle(title)
    } else {
        WorktreeBranch::Exact(branch)
    };
    report_stage(Some(WorktreeStage::Checkout));
    let project = runtime.allocate_workspace_on_branch(&source.id, &base.reference, branch)?;
    Ok((project, true))
}

pub(super) fn release_after_failure(
    runtime: &Huggler,
    project: &Project,
    error: anyhow::Error,
) -> anyhow::Error {
    match runtime.release_workspace(&project.id) {
        Ok(()) => error,
        Err(cleanup) => anyhow::anyhow!(
            "{error}. Could not release {}. {cleanup}",
            project.root.display()
        ),
    }
}

struct BranchPicker {
    branches: Vec<git::Branch>,
    selected: Option<String>,
    matches: Vec<usize>,
}

impl ListDelegate for BranchPicker {
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
        let branch = &self.branches[*self.matches.get(ix.row)?];
        let detail = if branch.current {
            "Current"
        } else if branch.remote {
            "Remote"
        } else {
            "Local"
        };
        Some(
            ListItem::new(ix.row)
                .check_icon(IconName::Check)
                .confirmed(self.selected.as_ref() == Some(&branch.reference))
                .child(control!(
                    format!("branch/{}", branch.name),
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_3()
                        .justify_between()
                        .child(div().min_w_0().truncate().child(branch.name.clone()))
                        .child(hint(detail, cx).flex_shrink_0())
                )),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        hint("No matching branches", cx).p_2()
    }

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        let query = query.to_lowercase();
        self.matches = self
            .branches
            .iter()
            .enumerate()
            .filter(|(_, branch)| branch.name.to_lowercase().contains(&query))
            .map(|(index, _)| index)
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
    pub(super) fn load_launch_branches(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let path = PathBuf::from(self.checkout.read(cx).value().trim());
        self.launch_workspace.load = None;
        if path.as_os_str().is_empty() {
            return;
        }
        self.launch_workspace.load = Some(cx.spawn(async move |view, cx| {
            if debounce {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
            }
            let result = cx
                .background_spawn(async move { git::branches(&path) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.launch_workspace.branches = Some(result);
                cx.notify();
            });
        }));
    }

    pub(super) fn launch_checkout_controls(&self, cx: &Context<Self>) -> Div {
        let mode = self.launch_workspace.mode();
        let can_create_worktree = self
            .launch_workspace
            .branches()
            .is_some_and(|branches| branches.default.is_some());
        let view = cx.weak_entity();
        let workspace = Button::new("launch-workspace")
            .small()
            .ghost()
            .icon(IconName::Folder)
            .label(mode.label())
            .dropdown_caret(true)
            .dropdown_menu_with_anchor(Corner::BottomLeft, move |mut menu, _, _| {
                for option in [WorkspaceMode::Current, WorkspaceMode::Worktree] {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            control!(option.label(), div().child(option.label()))
                        })
                        .checked(mode == option)
                        .disabled(option == WorkspaceMode::Worktree && !can_create_worktree)
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.launch_workspace.mode = option;
                                view.launch_workspace.close_picker();
                                view.suggestions = Default::default();
                                view.focus_workspace(window, cx);
                                cx.notify();
                            });
                        }),
                    );
                }
                menu
            });
        h_flex()
            .min_w_0()
            .gap_2()
            .flex_wrap()
            .child(control!("Choose workspace", workspace))
            .child(self.launch_branch_control(mode, cx))
    }

    fn launch_branch_control(&self, mode: WorkspaceMode, cx: &Context<Self>) -> Div {
        let branch = self.launch_workspace.displayed_branch(mode);
        let can_pick = match mode {
            WorkspaceMode::Worktree => branch.is_some(),
            WorkspaceMode::Current => self
                .launch_workspace
                .branches()
                .is_some_and(|branches| branches.branches.iter().any(|branch| !branch.remote)),
        };
        let control_label = match mode {
            WorkspaceMode::Worktree => "Choose base branch",
            WorkspaceMode::Current => "Choose checkout branch",
        };
        let label = match (mode, branch) {
            (WorkspaceMode::Worktree, Some(branch)) => branch.base_label(),
            (WorkspaceMode::Current, Some(branch))
                if self.launch_workspace.checkout_branch.is_some() =>
            {
                format!("Switch to {}", branch.name)
            }
            (WorkspaceMode::Current, Some(branch)) => branch.name.clone(),
            (_, None) if matches!(self.launch_workspace.branches, Some(Err(_))) => {
                "Branches unavailable".into()
            }
            (_, None) if self.launch_workspace.branches.is_none() => "Loading branches…".into(),
            (_, None) if can_pick => "Choose branch".into(),
            _ => return div(),
        };
        let trigger = Button::new("choose-branch")
            .small()
            .ghost()
            .dropdown_caret(true)
            .disabled(!can_pick)
            .tooltip(control_label)
            .child(div().max_w(px(218.)).truncate().child(label.clone()))
            .on_click(cx.listener(|view, event: &ClickEvent, window, cx| {
                if event.is_keyboard() {
                    if view.launch_workspace.picker.is_some() {
                        view.launch_workspace.close_picker();
                        cx.notify();
                    } else {
                        view.open_launch_branches(window, cx);
                    }
                }
            }));
        // A disabled button can still receive its popover's mouse handler.
        if !can_pick {
            return control!(control_label, trigger);
        }
        let picker = Popover::new("launch-branch-picker")
            .anchor(Corner::BottomRight)
            .trigger(trigger)
            .open(self.launch_workspace.picker.is_some())
            .on_open_change(cx.listener(|view, open, window, cx| {
                if *open {
                    view.open_launch_branches(window, cx);
                } else {
                    view.launch_workspace.close_picker();
                    cx.notify();
                }
            }));
        let picker = if let Some(list) = &self.launch_workspace.picker {
            picker.child(
                v_flex().w(px(312.)).child(
                    List::new(list)
                        .search_placeholder("Search branches")
                        .h(px(196.)),
                ),
            )
        } else {
            picker
        };
        control!(label, control!(control_label, picker))
    }

    fn open_launch_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model_picker = None;
        let path = PathBuf::from(self.checkout.read(cx).value().trim());
        self.launch_workspace.load = None;
        let job = cx.background_spawn(async move { git::branches(&path) });
        self.launch_workspace.picker_load = Some(cx.spawn_in(window, async move |view, cx| {
            let result = job.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.launch_workspace.branches = Some(result);
                view.show_launch_branches(window, cx);
                cx.notify();
            });
        }));
    }

    fn show_launch_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branches) = self.launch_workspace.branches() else {
            return;
        };
        let mode = self.launch_workspace.mode();
        let mut choices: Vec<_> = branches
            .branches
            .iter()
            .filter(|branch| mode == WorkspaceMode::Worktree || !branch.remote)
            .cloned()
            .collect();
        if mode == WorkspaceMode::Worktree
            && let Some(default) = &branches.default
            && !choices
                .iter()
                .any(|branch| branch.reference == default.reference)
        {
            choices.insert(0, default.clone());
        }
        let list = cx.new(|cx| {
            let matches = (0..choices.len()).collect();
            let picker = BranchPicker {
                branches: choices,
                selected: self
                    .launch_workspace
                    .displayed_branch(mode)
                    .map(|branch| branch.reference.clone()),
                matches,
            };
            ListState::new(picker, window, cx).searchable(true)
        });
        cx.subscribe_in(&list, window, move |view, list, event, window, cx| {
            if view.launch_workspace.picker.as_ref() != Some(list) {
                return;
            }
            let picker = list.read(cx).delegate();
            match event {
                ListEvent::Confirm(ix) => {
                    let Some(index) = picker.matches.get(ix.row) else {
                        return;
                    };
                    let branch = &picker.branches[*index];
                    match mode {
                        WorkspaceMode::Worktree => {
                            view.launch_workspace.base = Some(branch.clone());
                        }
                        WorkspaceMode::Current => {
                            view.launch_workspace.checkout_branch =
                                (!branch.current).then(|| branch.clone());
                        }
                    }
                    view.suggestions = Default::default();
                }
                ListEvent::Cancel => (),
                ListEvent::Select(_) => return,
            }
            view.launch_workspace.close_picker();
            view.focus_workspace(window, cx);
            cx.notify();
        })
        .detach();
        list.focus_handle(cx).focus(window);
        self.launch_workspace.picker = Some(list);
    }
}
