mod approvals;

use crate::{
    git::{self, ChangeScope, File, FileChange, LineCounts, PreparedRevert},
    open_in::{OpenIn, file_menu},
    ui::{hint, label},
};
use anyhow::Context as _;
pub(crate) use approvals::ReviewRow;
use approvals::{Approval, Approvals};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct Changes {
    root: PathBuf,
    approvals_file: PathBuf,
    thread: String,
    approvals: Approvals,
    filter: Filter,
    requested_filter: Filter,
    filter_focus: FocusHandle,
    filter_menu: Option<FilterMenu>,
    snapshot: Option<anyhow::Result<Vec<ReviewRow>>>,
    error: Option<String>,
    mutation: Option<Mutation>,
    refresh_task: Option<Task<()>>,
    mutation_task: Option<Task<()>>,
}

impl EventEmitter<ReviewRow> for Changes {}
impl EventEmitter<RevertFinished> for Changes {}

pub(crate) struct RevertFinished {
    pub paths: Vec<PathBuf>,
    pub failure: Option<(PathBuf, String)>,
}

struct FilterMenu {
    view: Entity<PopupMenu>,
    _subscriptions: [Subscription; 2],
}

enum Mutation {
    Approve,
    Revert { bulk: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Uncommitted,
    Approved,
}

impl Filter {
    const OPTIONS: [Self; 3] = [Self::All, Self::Uncommitted, Self::Approved];

    fn scope(self) -> Option<ChangeScope> {
        match self {
            Self::All => Some(ChangeScope::All),
            Self::Uncommitted => Some(ChangeScope::Uncommitted),
            Self::Approved => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::All => "All changes",
            Self::Uncommitted => "Uncommitted files",
            Self::Approved => "Approved",
        }
    }

    fn bulk_approval_title(self) -> &'static str {
        match self {
            Self::Approved => "Undo all approvals",
            Self::All | Self::Uncommitted => "Approve all",
        }
    }
}

impl Changes {
    pub fn new(
        root: PathBuf,
        approvals_file: PathBuf,
        thread: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let (approvals, error) = match approvals::load(&approvals_file, &thread) {
            Ok(approvals) => (approvals, None),
            Err(error) => (
                Approvals::new(),
                Some(format!("Could not read file approvals. {error:#}")),
            ),
        };
        Self {
            root,
            approvals_file,
            thread,
            approvals,
            filter: Filter::All,
            requested_filter: Filter::All,
            filter_focus: cx.focus_handle(),
            filter_menu: None,
            snapshot: None,
            error,
            mutation: None,
            refresh_task: None,
            mutation_task: None,
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.mutation.is_some() {
            return;
        }
        self.scan(cx);
    }

    fn scan(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let filter = self.requested_filter;
        let approvals = self.approvals.clone();
        let job = cx.background_spawn(async move {
            approvals::visible_rows(&root, filter.scope(), &approvals)
        });
        self.refresh_task = Some(cx.spawn(async move |view, cx| {
            let snapshot = job.await;
            let _ = view.update(cx, |panel, cx| {
                panel.refresh_task = None;
                panel.filter = filter;
                panel.snapshot = Some(snapshot);
                cx.notify();
            });
        }));
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        if self.requested_filter == filter {
            return;
        }
        // Keep the displayed filter and its rows together until the scan finishes.
        self.requested_filter = filter;
        self.refresh(cx);
    }

    fn open_filter_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_menu.is_some() {
            return;
        }
        let panel = cx.weak_entity();
        let view = PopupMenu::build(window, cx, |menu, _, _| {
            let mut menu = menu
                .min_w(px(154.))
                .action_context(self.filter_focus.clone());
            for filter in Filter::OPTIONS {
                let title = filter.title();
                let panel = panel.clone();
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        control!(format!("Change filter/{title}"), div().child(title))
                    })
                    .checked(filter == self.filter)
                    .on_click(move |_, _, cx| {
                        let _ = panel.update(cx, |panel, cx| panel.set_filter(filter, cx));
                    }),
                );
            }
            menu
        });
        let focus = view.focus_handle(cx);
        let dismiss = cx.subscribe(&view, |panel, _, _: &DismissEvent, cx| {
            panel.filter_menu = None;
            cx.notify();
        });
        let blur = cx.on_blur(&focus, window, |panel, _, cx| {
            panel.filter_menu = None;
            cx.notify();
        });
        self.filter_menu = Some(FilterMenu {
            view,
            _subscriptions: [dismiss, blur],
        });
        focus.focus(window);
        cx.notify();
    }

    fn filter_control(&self, cx: &Context<Self>) -> impl IntoElement {
        let title = self.filter.title();
        let open = self.filter_menu.is_some();
        let fill = cx.theme().sidebar_accent;
        let height = px(24.);
        control!(
            title,
            v_flex()
                .w(px(match self.filter {
                    Filter::All => 112.,
                    Filter::Uncommitted => 154.,
                    Filter::Approved => 102.,
                }))
                .child(
                    h_flex()
                        .id("change-filter")
                        .track_focus(&self.filter_focus)
                        .tab_stop(true)
                        .justify_between()
                        .h(height)
                        .px_2()
                        .rounded_md()
                        .text_sm()
                        .cursor_pointer()
                        .hover(|style| style.bg(fill))
                        .focus(|style| style.bg(fill))
                        .when(open, |trigger| trigger.bg(fill))
                        .when(!open, |trigger| {
                            trigger
                                .on_click(cx.listener(|panel, _, window, cx| {
                                    panel.open_filter_menu(window, cx);
                                }))
                                .on_key_down(cx.listener(
                                    |panel, event: &KeyDownEvent, window, cx| {
                                        if !event.keystroke.modifiers.modified()
                                            && matches!(event.keystroke.key.as_str(), "down" | "up")
                                        {
                                            cx.stop_propagation();
                                            panel.open_filter_menu(window, cx);
                                        }
                                    },
                                ))
                        })
                        .child(title)
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground)
                        )
                )
                .when_some(self.filter_menu.as_ref(), |trigger, menu| {
                    trigger.child(
                        deferred(
                            anchored().snap_to_window_with_margin(px(8.)).child(
                                div().occlude().mt(height + px(4.)).child(menu.view.clone()),
                            ),
                        )
                        .with_priority(1),
                    )
                })
        )
    }

    fn rows(&self) -> &[ReviewRow] {
        match &self.snapshot {
            Some(Ok(rows)) => rows,
            _ => &[],
        }
    }

    fn bulk_rows(&self, filter: Filter, eligible: fn(&File) -> bool) -> Vec<ReviewRow> {
        if self.filter != filter || self.requested_filter != filter || self.mutation.is_some() {
            return Vec::new();
        }
        self.rows()
            .iter()
            .filter(|row| eligible(&row.file))
            .cloned()
            .collect()
    }

    fn bulk_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.weak_entity();
        let filter = self.filter;
        let approval = filter.bulk_approval_title();
        let can_approve = self.rows().iter().any(|row| row.file.can_approve());
        let can_revert = self.rows().iter().any(|row| row.file.can_revert());
        control!(
            "File actions",
            Button::new("bulk-file-actions")
                .small()
                .compact()
                .ghost()
                .size(px(24.))
                .p_0()
                .icon(IconName::Ellipsis)
                .tooltip("File actions")
                .disabled(
                    self.mutation.is_some()
                        || self.requested_filter != filter
                        || !(can_approve || can_revert)
                )
                .dropdown_menu_with_anchor(Corner::TopRight, move |menu, _, _| {
                    let approve_view = view.clone();
                    let revert_view = view.clone();
                    menu.item(
                        PopupMenuItem::element(move |_, _| {
                            control!(approval, div().child(approval))
                        })
                        .icon(if filter == Filter::Approved {
                            IconName::Close
                        } else {
                            IconName::Check
                        })
                        .disabled(!can_approve)
                        .on_click(move |_, _, cx| {
                            let _ = approve_view.update(cx, |panel, cx| {
                                let rows = panel.bulk_rows(filter, File::can_approve);
                                panel.toggle_approvals(filter, rows, cx);
                            });
                        }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::element(|_, _| {
                            control!("Revert all", div().child("Revert all"))
                        })
                        .icon(IconName::Undo2)
                        .disabled(!can_revert)
                        .on_click(move |_, window, cx| {
                            let _ = revert_view.update(cx, |panel, cx| {
                                let rows = panel.bulk_rows(filter, File::can_revert);
                                panel.confirm_revert(rows, true, window, cx);
                            });
                        }),
                    )
                })
        )
        .flex_none()
    }

    fn confirm_revert(
        &self,
        rows: Vec<ReviewRow>,
        bulk: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_revert_dialog(rows, bulk, None, window, cx);
    }

    pub(crate) fn confirm_document_revert(
        &self,
        row: ReviewRow,
        prepared: Arc<[PreparedRevert]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_revert_dialog(vec![row], false, Some(prepared), window, cx);
    }

    fn show_revert_dialog(
        &self,
        rows: Vec<ReviewRow>,
        bulk: bool,
        targets: Option<Arc<[PreparedRevert]>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(first) = rows.first().filter(|_| self.mutation.is_none()) else {
            return;
        };
        let (title, target, detail, confirm) = if bulk {
            let count = rows.len();
            let noun = if count == 1 { "file" } else { "files" };
            (
                format!("Revert {count} {noun}?"),
                self.filter.title().to_owned(),
                "This restores tracked files from the last commit and removes new files. Staged and unstaged changes will be discarded.",
                "Revert all",
            )
        } else {
            let detail = if first.file.revert_removes_file() {
                "This removes the new file and any staged changes for it."
            } else {
                "This restores the file from the last commit and discards its staged and unstaged changes."
            };
            (
                "Revert file?".to_owned(),
                first.file.path.display().to_string(),
                detail,
                "Revert file",
            )
        };
        let _confirm_control = if bulk {
            "Confirm revert all"
        } else {
            "Confirm file revert"
        };
        let root = self.root.clone();
        let prepare_on_open = targets.is_none();
        let prepared = cx.new(|_| targets.map(Ok));
        if prepare_on_open {
            let pending = prepared.downgrade();
            let job = cx.background_spawn(async move {
                rows.into_iter()
                    .map(|row| {
                        let path = row.file.path.display().to_string();
                        PreparedRevert::prepare(&root, row.head, row.file).map_err(|error| {
                            format!("Could not prepare to revert {path}. {error:#}")
                        })
                    })
                    .collect::<Result<Arc<[_]>, _>>()
            });
            cx.spawn_in(window, async move |_, cx| {
                let result = job.await;
                let _ = pending.update_in(cx, |prepared, window, cx| {
                    *prepared = Some(result);
                    cx.notify();
                    window.refresh();
                });
            })
            .detach();
        }
        let view = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let view = view.clone();
            let preparation = prepared.read(cx);
            let targets = preparation
                .as_ref()
                .and_then(|result| result.as_ref().ok())
                .cloned();
            dialog
                .title(title.clone())
                .confirm()
                // The dialog's default Return action must not discard edits.
                .on_ok(|_, _, _| false)
                .child(
                    v_flex()
                        .gap_2()
                        .child(label(target.clone()))
                        .child(hint(detail, cx))
                        .when_some(
                            preparation
                                .as_ref()
                                .and_then(|result| result.as_ref().err()),
                            |body, error| {
                                body.child(hint(error.clone(), cx).text_color(cx.theme().danger))
                            },
                        ),
                )
                .footer(move |_, _, _, _| {
                    vec![
                        control!(
                            "Cancel file revert",
                            Button::new("cancel-file-revert")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx))
                        ),
                        if let Some(targets) = &targets {
                            let view = view.clone();
                            let targets = targets.clone();
                            control!(
                                _confirm_control,
                                Button::new("confirm-file-revert")
                                    .label(confirm)
                                    .danger()
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        let _ = view.update(cx, |panel, cx| {
                                            panel.revert(targets.clone(), bulk, cx);
                                        });
                                    })
                            )
                        } else {
                            div().child(
                                Button::new("confirm-file-revert")
                                    .label(confirm)
                                    .danger()
                                    .disabled(true),
                            )
                        },
                    ]
                })
        });
    }

    fn revert(&mut self, targets: Arc<[PreparedRevert]>, bulk: bool, cx: &mut Context<Self>) {
        if self.mutation.is_some() {
            return;
        }
        self.begin_mutation(Mutation::Revert { bulk }, cx);
        let root = self.root.clone();
        let job = cx.background_spawn(async move {
            let mut paths = Vec::with_capacity(targets.len());
            for target in targets.iter() {
                if let Err(error) = git::revert_file(&root, target) {
                    let failure =
                        format!("Could not revert {}. {error:#}", target.path().display());
                    let message = if bulk {
                        format!(
                            "Reverted {} of {} files. {failure}",
                            paths.len(),
                            targets.len()
                        )
                    } else {
                        failure
                    };
                    return RevertFinished {
                        paths,
                        failure: Some((target.path().to_path_buf(), message)),
                    };
                }
                paths.push(target.path().to_path_buf());
            }
            RevertFinished {
                paths,
                failure: None,
            }
        });
        self.mutation_task = Some(cx.spawn(async move |view, cx| {
            let result = job.await;
            let _ = view.update(cx, |panel, cx| {
                panel.snapshot = None;
                panel.finish_mutation(result.failure.as_ref().map(|(_, error)| error.clone()), cx);
                cx.emit(result);
            });
        }));
    }

    fn begin_mutation(&mut self, mutation: Mutation, cx: &mut Context<Self>) {
        self.mutation = Some(mutation);
        self.error = None;
        self.refresh_task = None;
        cx.notify();
    }

    fn finish_mutation(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.mutation = None;
        self.mutation_task = None;
        self.error = error;
        self.scan(cx);
        cx.notify();
    }

    fn save_approvals(
        &mut self,
        updates: BTreeMap<String, Option<Approval>>,
    ) -> anyhow::Result<()> {
        let mut approvals = self.approvals.clone();
        for (key, approval) in &updates {
            if let Some(approval) = approval {
                approvals.insert(key.clone(), approval.clone());
            } else {
                approvals.remove(key);
            }
        }
        approvals::save(&self.approvals_file, &self.thread, &approvals)?;
        self.approvals = approvals;
        // Keep the other rows visible while the next scan reconciles checkout edits.
        if let Some(Ok(rows)) = &mut self.snapshot {
            rows.retain(|row| !updates.contains_key(&approvals::path_key(&row.file.path)));
        }
        Ok(())
    }

    fn toggle_approvals(&mut self, filter: Filter, rows: Vec<ReviewRow>, cx: &mut Context<Self>) {
        if self.filter != filter || self.mutation.is_some() || rows.is_empty() {
            return;
        }
        let Some(scope) = filter.scope() else {
            let updates = rows
                .iter()
                .map(|row| (approvals::path_key(&row.file.path), None))
                .collect();
            let error = self
                .save_approvals(updates)
                .err()
                .map(|error| format!("Could not undo approval. {error:#}"));
            self.finish_mutation(error, cx);
            return;
        };
        self.begin_mutation(Mutation::Approve, cx);
        let root = self.root.clone();
        let job = cx.background_spawn(async move {
            rows.into_iter()
                .map(|row| {
                    let path = &row.file.path;
                    let fingerprint = git::diff_fingerprint(&root, &row.base, &row.head, &row.file)
                        .with_context(|| {
                            format!("Could not read the diff for {}", path.display())
                        })?;
                    Ok((
                        approvals::path_key(path),
                        Some(Approval { scope, fingerprint }),
                    ))
                })
                .collect::<anyhow::Result<_>>()
        });
        self.mutation_task = Some(cx.spawn(async move |view, cx| {
            let updates = job.await;
            let _ = view.update(cx, |panel, cx| {
                let error = updates
                    .and_then(|updates| panel.save_approvals(updates))
                    .err()
                    .map(|error| format!("Could not approve changes. {error:#}"));
                panel.finish_mutation(error, cx);
            });
        }));
    }

    fn open_file_in(&mut self, destination: OpenIn, path: PathBuf, cx: &mut Context<Self>) {
        if self.error.take().is_some() {
            cx.notify();
        }
        let root = self.root.clone();
        let job = cx.background_spawn(async move {
            destination
                .open_file(&root, &path)
                .map_err(|error| format!("Could not open {}. {error:#}", path.display()))
        });
        cx.spawn(async move |view, cx| {
            if let Err(error) = job.await {
                let _ = view.update(cx, |panel, cx| {
                    panel.error = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn file_open_menu(
        &self,
        path: &Path,
        row_id: EntityId,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let view = cx.weak_entity();
        let target = path.to_owned();
        file_menu(
            Button::new(("open-changed-file", row_id))
                .small()
                .compact()
                .ghost()
                .size(px(24.))
                .p_0()
                .child(Icon::new(IconName::ChevronDown).small()),
            path,
            move |destination, cx| {
                let _ = view.update(cx, |panel, cx| {
                    panel.open_file_in(destination, target.clone(), cx);
                });
            },
        )
    }

    fn file_row(&self, row: &ReviewRow, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let file = &row.file;
        let path = file.path.to_string_lossy().into_owned();
        let (directory, filename) = path
            .rsplit_once('/')
            .map_or((String::new(), path.clone()), |(directory, filename)| {
                (format!("{directory}/"), filename.to_owned())
            });
        let row_state = window.use_keyed_state(
            SharedString::from(approvals::path_key(&file.path)),
            cx,
            |_, cx| (cx.focus_handle(), cx.focus_handle()),
        );
        let (approval_focus, revert_focus) = row_state.read(cx).clone();
        let focused = approval_focus.is_focused(window) || revert_focus.is_focused(window);
        let hover = cx.theme().sidebar_accent;
        let muted = cx.theme().muted_foreground;
        let group = "changed-file";
        let _row_path = path.clone();
        let full_path = path.clone();
        let diff = row.clone();
        let ring = cx.theme().ring;
        let filter = self.filter;
        let reviewed = filter == Filter::Approved;
        let action_opacity = if self.mutation.is_none() { 1. } else { 0.5 };
        let can_approve = file.can_approve();
        let can_revert = file.can_revert();
        let has_actions = can_approve || can_revert;
        let action = if reviewed { "Undo approval" } else { "Approve" };
        let action_icon = if reviewed {
            IconName::Close
        } else {
            IconName::Check
        };

        control!(
            format!("Changed/{path}"),
            div()
                .id(("change-row", row_state.entity_id()))
                .group(group)
                .relative()
                .rounded_md()
                .hover(|style| style.bg(hover))
                .when(focused, |row| row.bg(hover))
                .child(
                    h_flex()
                        .h(px(32.))
                        .px_2()
                        .gap_1()
                        .min_w_0()
                        .child(
                            h_flex()
                                .id("changed-file-path")
                                .min_w_0()
                                .flex_1()
                                .h_full()
                                .rounded_sm()
                                .tab_index(0)
                                .focus(move |style| style.border_1().border_color(ring))
                                .cursor_pointer()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(full_path.clone()).build(window, cx)
                                })
                                .on_click(cx.listener(move |_, _, _, cx| cx.emit(diff.clone())))
                                .child(file_path_label(directory, filename, muted)),
                        )
                        .child(
                            h_flex()
                                .flex_none()
                                .gap_1()
                                .when(has_actions, |area| {
                                    area.group_hover(group, |style| style.invisible())
                                        .when(focused, |area| area.invisible())
                                })
                                .child(
                                    div()
                                        .min_w(px(if can_approve && can_revert {
                                            48.
                                        } else {
                                            24.
                                        }))
                                        .child(line_counts(file.lines, cx)),
                                )
                                .child(status_icon(file.change, cx).flex_none()),
                        )
                        .child(self.file_open_menu(&file.path, row_state.entity_id(), cx))
                )
                .when(has_actions, |container| {
                    container.child(
                        h_flex()
                            .absolute()
                            .right(px(36.))
                            .top(px(4.))
                            .opacity(if focused { action_opacity } else { 0. })
                            .group_hover(group, |style| style.opacity(action_opacity))
                            .when(can_approve, |actions| {
                                let row = row.clone();
                                actions.child(control!(
                                    format!("{action}/{_row_path}"),
                                    file_action(
                                        "change-approval",
                                        action_icon,
                                        action,
                                        &approval_focus,
                                        cx
                                    )
                                    .on_click(cx.listener(
                                        move |panel, _, _, cx| {
                                            cx.stop_propagation();
                                            panel.toggle_approvals(filter, vec![row.clone()], cx);
                                        }
                                    ))
                                ))
                            })
                            .when(can_revert, |actions| {
                                let row = row.clone();
                                actions.child(control!(
                                    format!("Revert/{_row_path}"),
                                    file_action(
                                        "revert-file",
                                        IconName::Undo2,
                                        "Revert",
                                        &revert_focus,
                                        cx
                                    )
                                    .on_click(cx.listener(
                                        move |panel, _, window, cx| {
                                            cx.stop_propagation();
                                            panel.confirm_revert(
                                                vec![row.clone()],
                                                false,
                                                window,
                                                cx,
                                            );
                                        }
                                    ))
                                ))
                            }),
                    )
                }),
        )
        .min_w_0()
        .flex_shrink_0()
    }
}

impl Render for Changes {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = self
            .rows()
            .iter()
            .fold(LineCounts::default(), |mut total, row| {
                total.added += row.file.lines.added;
                total.removed += row.file.lines.removed;
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
                    .child(self.filter_control(cx))
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .child(line_counts(counts, cx))
                            .child(self.bulk_menu(cx)),
                    ),
            )
            .when_some(
                self.mutation.as_ref().and_then(|mutation| match mutation {
                    Mutation::Revert { bulk: true } => Some("Reverting files…"),
                    Mutation::Revert { bulk: false } => Some("Reverting file…"),
                    Mutation::Approve => None,
                }),
                |panel, message| panel.child(hint(message, cx).px_3().py_1()),
            )
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
                Some(Ok(rows)) if rows.is_empty() => {
                    let message = match self.filter {
                        Filter::All if self.approvals.is_empty() => "No changes",
                        Filter::Uncommitted if self.approvals.is_empty() => "Working tree clean",
                        Filter::All | Filter::Uncommitted => "No changes to review",
                        Filter::Approved => "No approved files",
                    };
                    control!(message, hint(message, cx).px_3().py_2()).into_any_element()
                }
                Some(Ok(rows)) => v_flex()
                    .id(("change-files", self.filter as usize))
                    .flex_1()
                    .min_h_0()
                    .px_1()
                    .py_1()
                    .overflow_y_scroll()
                    .children(rows.iter().map(|row| self.file_row(row, window, cx)))
                    .into_any_element(),
            })
    }
}

fn file_path_label(directory: String, filename: String, muted: Hsla) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let label = |clipped| {
                h_flex()
                    .text_sm()
                    .when(!directory.is_empty(), |label| {
                        label.when(clipped, |label| label.gap_1()).child(
                            div().min_w_0().overflow_hidden().child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .text_color(muted)
                                    .child(directory.clone()),
                            ),
                        )
                    })
                    .child(
                        div()
                            .min_w_0()
                            .flex_shrink_0()
                            .max_w_full()
                            .overflow_hidden()
                            .child(div().w_full().truncate().child(filename.clone())),
                    )
            };
            // Fit against the row's actual text width, before reserving any gap.
            let available = size(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
            let mut path = label(false).into_any_element();
            let mut measured = path.layout_as_root(available, window, cx);
            if measured.width > bounds.size.width {
                path = label(true)
                    .w(bounds.size.width)
                    .overflow_hidden()
                    .into_any_element();
                measured = path.layout_as_root(available, window, cx);
            }
            let origin = point(bounds.left(), bounds.center().y - measured.height / 2.);
            path.prepaint_at(origin, window, cx);
            path
        },
        |_, mut path, window, cx| path.paint(window, cx),
    )
    .size_full()
}

fn file_action(
    id: &'static str,
    icon: IconName,
    title: &'static str,
    focus: &FocusHandle,
    cx: &App,
) -> Stateful<Div> {
    let ring = cx.theme().ring;
    let background = cx.theme().sidebar;
    h_flex()
        .id(id)
        .track_focus(focus)
        .tab_stop(true)
        .size(px(24.))
        .rounded_sm()
        .border_1()
        .border_color(transparent_black())
        .focus(|style| style.border_color(ring))
        .hover(|style| style.bg(background))
        .justify_center()
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new(title).build(window, cx))
        .child(Icon::new(icon).small())
}

pub(crate) fn change_colors(cx: &App) -> (Hsla, Hsla, Hsla) {
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

pub(crate) fn line_counts(counts: LineCounts, cx: &App) -> Div {
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
