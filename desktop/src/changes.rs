use crate::{
    git::{self, ChangeScope, ChangesSnapshot, File, FileChange, LineCounts},
    ui::{hint, label},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, IndexPath, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    select::{Select, SelectEvent, SelectItem, SelectState},
    tooltip::Tooltip,
    v_flex,
};
use std::path::PathBuf;

pub struct Changes {
    root: PathBuf,
    scope: ChangeScope,
    scope_selector: Entity<SelectState<Vec<ChangeScope>>>,
    snapshot: Option<anyhow::Result<ChangesSnapshot>>,
    expanded: Option<ExpandedDiff>,
    error: Option<String>,
    busy: bool,
    refresh_task: Option<Task<()>>,
    diff_task: Option<Task<()>>,
    revert_task: Option<Task<()>>,
}

struct ExpandedDiff {
    path: PathBuf,
    result: Option<anyhow::Result<String>>,
}

impl Changes {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let scope_selector = cx.new(|cx| {
            SelectState::new(
                vec![ChangeScope::All, ChangeScope::Uncommitted],
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        cx.subscribe(&scope_selector, |panel, _, event, cx| {
            if let SelectEvent::Confirm(Some(scope)) = event {
                panel.set_scope(*scope, cx);
            }
        })
        .detach();
        Self {
            root,
            scope: ChangeScope::default(),
            scope_selector,
            snapshot: None,
            expanded: None,
            error: None,
            busy: false,
            refresh_task: None,
            diff_task: None,
            revert_task: None,
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.scan(cx);
    }

    fn scan(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let scope = self.scope;
        let job = cx.background_spawn(async move { git::changed_files(&root, scope) });
        self.refresh_task = Some(cx.spawn(async move |view, cx| {
            let snapshot = job.await;
            let _ = view.update(cx, |panel, cx| {
                panel.refresh_task = None;
                panel.update_snapshot(snapshot, cx);
            });
        }));
    }

    fn set_scope(&mut self, scope: ChangeScope, cx: &mut Context<Self>) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.snapshot = None;
        self.diff_task = None;
        self.refresh(cx);
        cx.notify();
    }

    fn update_snapshot(
        &mut self,
        snapshot: anyhow::Result<ChangesSnapshot>,
        cx: &mut Context<Self>,
    ) {
        self.diff_task = None;
        let expanded_path = self.expanded.take().map(|diff| diff.path);
        self.snapshot = Some(snapshot);
        if let Some((file, base)) = expanded_path.and_then(|path| {
            let snapshot = self.snapshot.as_ref()?.as_ref().ok()?;
            let file = snapshot.files.iter().find(|file| file.path == path)?;
            Some((file.clone(), snapshot.base.clone()))
        }) {
            self.load_diff(file, base, cx);
        }
        cx.notify();
    }

    fn toggle_diff(&mut self, file: File, base: String, cx: &mut Context<Self>) {
        if self
            .expanded
            .as_ref()
            .is_some_and(|diff| diff.path == file.path)
        {
            self.expanded = None;
            self.diff_task = None;
            cx.notify();
        } else {
            self.load_diff(file, base, cx);
        }
    }

    fn load_diff(&mut self, file: File, base: String, cx: &mut Context<Self>) {
        self.expanded = Some(ExpandedDiff {
            path: file.path.clone(),
            result: None,
        });
        let root = self.root.clone();
        let job = cx.background_spawn(async move { git::file_diff(&root, &base, &file) });
        self.diff_task = Some(cx.spawn(async move |view, cx| {
            let diff = job.await;
            let _ = view.update(cx, |panel, cx| {
                if let Some(expanded) = &mut panel.expanded {
                    expanded.result = Some(diff);
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn confirm_revert(
        &self,
        file: File,
        base: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let path = file.path.display().to_string();
        let detail = if file.revert_removes_file() {
            "This removes the new file and any staged changes for it."
        } else {
            "This restores the file from the last commit and discards its staged and unstaged changes."
        };
        let view = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let view = view.clone();
            let file = file.clone();
            let base = base.clone();
            dialog
                .title("Revert file?")
                .confirm()
                // The dialog's default Return action must not discard edits.
                .on_ok(|_, _, _| false)
                .child(
                    v_flex()
                        .gap_2()
                        .child(label(path.clone()))
                        .child(hint(detail, cx)),
                )
                .footer(move |_, _, _, _| {
                    vec![
                        control!(
                            "Cancel file revert",
                            Button::new("cancel-file-revert")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx))
                        ),
                        control!(
                            "Confirm file revert",
                            Button::new("confirm-file-revert")
                                .label("Revert file")
                                .danger()
                                .on_click({
                                    let view = view.clone();
                                    let file = file.clone();
                                    let base = base.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        let _ = view.update(cx, |panel, cx| {
                                            panel.revert(file.clone(), base.clone(), cx);
                                        });
                                    }
                                })
                        ),
                    ]
                })
        });
    }

    fn revert(&mut self, file: File, base: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.refresh_task = None;
        let root = self.root.clone();
        let path = file.path.display().to_string();
        let job = cx.background_spawn(async move { git::revert_file(&root, &base, &file) });
        self.revert_task = Some(cx.spawn(async move |view, cx| {
            let result = job.await;
            let _ = view.update(cx, |panel, cx| {
                panel.busy = false;
                panel.revert_task = None;
                panel.error = result
                    .err()
                    .map(|error| format!("Could not revert {path}. {error:#}"));
                panel.snapshot = None;
                panel.diff_task = None;
                panel.scan(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn file_row(
        &self,
        file: &File,
        snapshot: &ChangesSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let path = file.path.to_string_lossy().into_owned();
        let (directory, filename) = path
            .rsplit_once('/')
            .map_or((String::new(), path.clone()), |(directory, filename)| {
                (format!("{directory}/"), filename.to_owned())
            });
        let row = window.use_keyed_state(
            SharedString::from(
                file.path
                    .as_os_str()
                    .as_encoded_bytes()
                    .escape_ascii()
                    .to_string(),
            ),
            cx,
            |_, cx| (cx.focus_handle(), cx.focus_handle()),
        );
        let (row_focus, revert_focus) = row.read(cx).clone();
        let focused = row_focus.is_focused(window) || revert_focus.is_focused(window);
        let diff = self.expanded.as_ref().filter(|diff| diff.path == file.path);
        let open = diff.is_some();
        let background = cx.theme().sidebar;
        let hover = cx.theme().sidebar_accent;
        let ring = cx.theme().ring;
        let muted = cx.theme().muted_foreground;
        let group = "changed-file";
        let disclosure = if open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };
        let row_path = path.clone();
        let full_path = path.clone();
        let can_revert = file.can_revert() && !self.busy;

        v_flex()
            .min_w_0()
            .flex_shrink_0()
            .child(control!(
                format!("Changed/{path}"),
                div()
                    .id(("change-row", row.entity_id()))
                    .group(group)
                    .relative()
                    .rounded_md()
                    .hover(|style| style.bg(hover))
                    .when(open || focused, |row| row.bg(hover))
                    .child(
                        h_flex()
                            .id("file-diff")
                            .track_focus(&row_focus)
                            .tab_stop(true)
                            .h(px(32.))
                            .px_2()
                            .gap_1()
                            .min_w_0()
                            .rounded_md()
                            .border_1()
                            .border_color(transparent_black())
                            .focus(|style| style.border_color(ring))
                            .cursor_pointer()
                            .on_click(cx.listener({
                                let file = file.clone();
                                let base = snapshot.base.clone();
                                move |panel, _, _, cx| {
                                    panel.toggle_diff(file.clone(), base.clone(), cx)
                                }
                            }))
                            .child(
                                h_flex()
                                    .id("changed-file-path")
                                    .min_w_0()
                                    .flex_1()
                                    .text_sm()
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(full_path.clone()).build(window, cx)
                                    })
                                    .when(!directory.is_empty(), |path| {
                                        path.child(
                                            div().min_w_0().overflow_hidden().child(
                                                div()
                                                    .w_full()
                                                    .truncate()
                                                    .text_color(muted)
                                                    .child(directory),
                                            ),
                                        )
                                    })
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_shrink_0()
                                            .max_w_full()
                                            .overflow_hidden()
                                            .child(div().w_full().truncate().child(filename))
                                    ),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .min_w(px(24.))
                                    .when(can_revert, |area| {
                                        area.group_hover(group, |style| style.invisible())
                                            .when(focused, |area| area.invisible())
                                    })
                                    .child(line_counts(file.lines, cx)),
                            )
                            .child(status_icon(file.change, cx).flex_none())
                            .child(control!(
                                format!("Toggle diff/{row_path}"),
                                h_flex()
                                    .id("diff-disclosure")
                                    .w_5()
                                    .h_6()
                                    .flex_none()
                                    .justify_center()
                                    .rounded_sm()
                                    .hover(|style| style.bg(background))
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(if open { "Hide diff" } else { "Show diff" })
                                            .build(window, cx)
                                    })
                                    .child(Icon::new(disclosure).small().text_color(muted))
                            )),
                    )
                    .when(can_revert, |row| {
                        let file = file.clone();
                        let head = snapshot.head.clone();
                        row.child(
                            div()
                                .absolute()
                                .right(px(46.))
                                .top(px(4.))
                                .when(!focused, |area| {
                                    area.opacity(0.)
                                        .group_hover(group, |style| style.opacity(1.))
                                })
                                .child(control!(
                                    format!("Revert/{row_path}"),
                                    div()
                                        .id("revert-file")
                                        .track_focus(&revert_focus)
                                        .tab_stop(true)
                                        .size(px(24.))
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(transparent_black())
                                        .focus(|style| style.border_color(ring))
                                        .hover(|style| style.bg(background))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .tooltip(|window, cx| Tooltip::new("Revert file")
                                            .build(window, cx))
                                        .on_click(cx.listener(move |panel, _, window, cx| {
                                            cx.stop_propagation();
                                            panel.confirm_revert(
                                                file.clone(),
                                                head.clone(),
                                                window,
                                                cx,
                                            );
                                        }))
                                        .child(Icon::new(IconName::Undo2).small()),
                                )),
                        )
                    }),
            ))
            .when_some(diff, |row, diff| {
                row.child(diff_preview(diff.result.as_ref(), &row_path, cx))
            })
    }
}

impl Render for Changes {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = self
            .snapshot
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .into_iter()
            .flat_map(|snapshot| &snapshot.files)
            .fold(LineCounts::default(), |mut total, file| {
                total.added += file.lines.added;
                total.removed += file.lines.removed;
                total
            });
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .flex_wrap()
                    .justify_between()
                    .gap_x_2()
                    .pl_2()
                    .pr_3()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(control!(
                        self.scope.title(),
                        div()
                            .w(px(match self.scope {
                                ChangeScope::All => 112.,
                                ChangeScope::Uncommitted => 154.,
                            }))
                            .child(
                                Select::new(&self.scope_selector)
                                    .small()
                                    .menu_width(px(154.))
                                    .appearance(false)
                                    .rounded_md()
                                    .cursor_pointer()
                            )
                    ))
                    .child(line_counts(counts, cx)),
            )
            .when(self.busy, |panel| {
                panel.child(hint("Reverting file…", cx).px_3().py_1())
            })
            .when_some(self.error.as_ref(), |panel, error| {
                panel.child(
                    hint(error.clone(), cx)
                        .text_color(change_colors(cx).1)
                        .px_3()
                        .py_2(),
                )
            })
            .child(match &self.snapshot {
                None => hint("Reading checkout…", cx)
                    .px_3()
                    .py_2()
                    .into_any_element(),
                Some(Err(error)) => hint(format!("Could not read changes. {error:#}"), cx)
                    .px_3()
                    .py_2()
                    .into_any_element(),
                Some(Ok(snapshot)) if snapshot.files.is_empty() => {
                    let message = if self.scope == ChangeScope::Uncommitted {
                        "Working tree clean"
                    } else {
                        "No changes"
                    };
                    control!(message, hint(message, cx).px_3().py_2()).into_any_element()
                }
                Some(Ok(snapshot)) => v_flex()
                    .id("change-files")
                    .flex_1()
                    .min_h_0()
                    .px_1()
                    .py_1()
                    .overflow_y_scroll()
                    .children(
                        snapshot
                            .files
                            .iter()
                            .map(|file| self.file_row(file, snapshot, window, cx)),
                    )
                    .into_any_element(),
            })
    }
}

impl SelectItem for ChangeScope {
    type Value = Self;

    fn title(&self) -> SharedString {
        match self {
            Self::All => "All changes",
            Self::Uncommitted => "Uncommitted files",
        }
        .into()
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let title = self.title();
        control!(format!("Change filter/{title}"), div().child(title))
    }

    fn value(&self) -> &Self {
        self
    }
}

fn change_colors(cx: &App) -> (Hsla, Hsla, Hsla) {
    if cx.theme().is_dark() {
        (
            rgb(0x4cdb85).into(),
            rgb(0xff7979).into(),
            rgb(0xefd674).into(),
        )
    } else {
        (
            rgb(0x11743a).into(),
            rgb(0xb42636).into(),
            rgb(0x866200).into(),
        )
    }
}

fn line_counts(counts: LineCounts, cx: &App) -> Div {
    let (added, removed, _) = change_colors(cx);
    h_flex()
        .justify_end()
        .gap_1()
        .text_xs()
        .font_family(cx.theme().mono_font_family.clone())
        .when(counts.added > 0, |stats| {
            stats.child(div().text_color(added).child(format!("+{}", counts.added)))
        })
        .when(counts.removed > 0, |stats| {
            stats.child(
                div()
                    .text_color(removed)
                    .child(format!("-{}", counts.removed)),
            )
        })
}

fn status_icon(change: FileChange, cx: &App) -> Div {
    let (added, removed, modified) = change_colors(cx);
    let (icon, color) = match change {
        FileChange::Added => (Some(IconName::Plus), added),
        FileChange::Deleted => (Some(IconName::Minus), removed),
        FileChange::Conflict => (Some(IconName::TriangleAlert), removed),
        FileChange::Submodule => (Some(IconName::Folder), modified),
        FileChange::Modified => (None, modified),
    };
    let frame = div().size(px(16.)).flex().items_center().justify_center();
    if matches!(change, FileChange::Conflict | FileChange::Submodule) {
        return frame.children(icon.map(|icon| Icon::new(icon).xsmall().text_color(color)));
    }
    frame.child(
        div()
            .size(px(11.))
            .rounded(px(2.))
            .border_1()
            .border_color(color)
            .flex()
            .items_center()
            .justify_center()
            .map(|frame| match icon {
                Some(icon) => frame.child(Icon::new(icon).with_size(px(8.)).text_color(color)),
                None => frame.child(div().size(px(2.)).rounded_full().bg(color)),
            }),
    )
}

fn diff_preview(diff: Option<&anyhow::Result<String>>, path: &str, cx: &App) -> Div {
    let (added, removed, _) = change_colors(cx);
    let inner = match diff {
        None => hint("Reading diff…", cx).into_any_element(),
        Some(Err(error)) => hint(format!("{error:#}"), cx).into_any_element(),
        Some(Ok(diff)) if diff.is_empty() => {
            hint("No diff available for this file.", cx).into_any_element()
        }
        Some(Ok(diff)) => {
            let mut offset = 0;
            let highlights: Vec<_> = diff
                .split_inclusive('\n')
                .filter_map(|line| {
                    let start = offset;
                    offset += line.len();
                    let color = if line.starts_with("+++ ") || line.starts_with("--- ") {
                        cx.theme().muted_foreground
                    } else if line.starts_with('+') {
                        added
                    } else if line.starts_with('-') {
                        removed
                    } else if line.starts_with("@@")
                        || line.starts_with("diff ")
                        || line.starts_with("index ")
                    {
                        cx.theme().muted_foreground
                    } else {
                        return None;
                    };
                    Some((
                        start..offset,
                        HighlightStyle {
                            color: Some(color),
                            ..Default::default()
                        },
                    ))
                })
                .collect();
            control!(
                format!("Diff/{path}"),
                div()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .whitespace_nowrap()
                    .child(StyledText::new(diff.clone()).with_highlights(highlights))
            )
            .id("file-diff-preview")
            .max_h(px(240.))
            .overflow_scroll()
            .into_any_element()
        }
    };
    div()
        .mx_1()
        .mb_1()
        .rounded_md()
        .bg(cx.theme().background)
        .border_1()
        .border_color(cx.theme().border)
        .p_2()
        .child(inner)
}
