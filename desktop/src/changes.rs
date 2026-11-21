use crate::{
    FollowUp,
    git::{self, ChangeGroup, File, Operation, Review},
    ui::{field, hint, label},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, InputState},
    v_flex,
};
use std::path::PathBuf;

pub struct Changes {
    root: PathBuf,
    message: Entity<InputState>,
    review: Option<Review>,
    busy: bool,
    error: Option<String>,
    receipt: Option<String>,
}

impl EventEmitter<FollowUp> for Changes {}

impl Changes {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message =
            cx.new(|cx| InputState::new(window, cx).placeholder("Summarize the staged changes"));
        cx.subscribe(&message, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        Self {
            root,
            message,
            busy: false,
            review: None,
            error: None,
            receipt: None,
        }
    }

    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(
            Operation::Refresh,
            self.review.as_ref().and_then(|r| r.selected.clone()),
            window,
            cx,
        );
    }

    fn run(
        &mut self,
        op: Operation,
        selected: Option<File>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let root = self.root.clone();
        let job = cx.background_spawn(async move {
            let result = op.apply(&root);
            let review = git::read(&root, selected.as_ref());
            (op, result, review)
        });
        cx.spawn_in(window, async move |view, cx| {
            let (op, result, review) = job.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.busy = false;
                match result {
                    Ok(Some(receipt)) => {
                        view.receipt = Some(receipt);
                        if let Operation::Commit(sent) = op
                            && view.message.read(cx).value().as_str() == sent
                        {
                            view.message
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                    }
                    Ok(None) => {}
                    Err(err) => view.error = Some(err.to_string()),
                }
                match review {
                    Ok(review) => view.review = Some(review),
                    Err(err) => {
                        view.review = None;
                        view.error.get_or_insert_with(|| err.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn file_list(&self, review: &Review, cx: &Context<Self>) -> impl IntoElement {
        let mut list = div()
            .id("change-files")
            .max_h(relative(0.28))
            .overflow_y_scroll();
        for (group, section) in [
            (ChangeGroup::Conflict, "Conflicts"),
            (ChangeGroup::Unstaged, "Unstaged"),
            (ChangeGroup::Staged, "Staged"),
        ] {
            let files: Vec<_> = review.files.iter().filter(|f| f.group == group).collect();
            if files.is_empty() {
                continue;
            }
            list = list.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{section} ({})", files.len())),
            );
            for (i, file) in files.into_iter().enumerate() {
                let name = file.path.to_string_lossy();
                let status = match file.status {
                    '?' => "Untracked".to_owned(),
                    'M' => "Modified".to_owned(),
                    'A' => "Added".to_owned(),
                    'D' => "Deleted".to_owned(),
                    'T' => "Type changed".to_owned(),
                    'U' => "Unmerged".to_owned(),
                    other => other.to_string(),
                };
                let selection = file.clone();
                list = list.child(control!(
                    format!("{section}/{name}"),
                    Button::new(SharedString::from(format!("{section}-{i}")))
                        .ghost()
                        .compact()
                        .w_full()
                        .justify_start()
                        .child(div().min_w_0().truncate().child(name.to_string()))
                        .child(hint(status, cx))
                        .tooltip(name.to_string())
                        .selected(review.selected.as_ref() == Some(file))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |view, _, w, cx| view.run(
                            Operation::Refresh,
                            Some(selection.clone()),
                            w,
                            cx
                        )))
                ));
            }
        }
        list
    }
}

impl Render for Changes {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let review = self.review.as_ref();
        let staged = review.map_or(0, |r| {
            r.files
                .iter()
                .filter(|f| f.group == ChangeGroup::Staged)
                .count()
        });
        let conflicts =
            review.is_some_and(|r| r.files.iter().any(|f| f.group == ChangeGroup::Conflict));
        let merging = review.is_some_and(|r| r.merging);
        let theme = cx.theme();
        v_flex()
            .size_full()
            .min_h_0()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(
                        div().flex_1().min_w_0().child(label("Working tree")).child(
                            hint(
                                review.map_or("Reading checkout…", |r| &r.branch).to_owned(),
                                cx,
                            )
                            .truncate(),
                        ),
                    )
                    .child(control!(
                        "Refresh changes",
                        Button::new("refresh")
                            .label("Refresh")
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, _, w, cx| v.refresh(w, cx)))
                    )),
            )
            .when_some(review, |panel, review| {
                panel
                    .child(self.file_list(review, cx))
                    .when(conflicts, |panel| panel.child(hint("Resolve each conflict in the checkout and stage its resolution before committing.", cx)))
                    .when(merging && !conflicts, |panel| panel.child(hint("Merge in progress. Enter a message to commit the merge.", cx)))
                    .when(review.files.is_empty() && !merging, |panel| {
                        panel.child(control!("Working tree clean", "Working tree clean"))
                    })
            })
            .when_some(review.and_then(|r| r.selected.as_ref()), |panel, file| {
                let file = file.clone();
                let path = file.path.display().to_string();
                let toggle = match file.group {
                    ChangeGroup::Staged => "Unstage file",
                    ChangeGroup::Unstaged => "Stage file",
                    ChangeGroup::Conflict => "Stage resolution",
                };
                panel.child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(label(path.clone()).truncate()),
                        )
                        .child(control!(
                            toggle,
                            Button::new("toggle-stage")
                                .label(toggle)
                                .disabled(self.busy)
                                .on_click(cx.listener(move |v, _, w, cx| {
                                    let op = if file.group == ChangeGroup::Staged {
                                        Operation::Unstage(file.clone())
                                    } else {
                                        Operation::Stage(file.clone())
                                    };
                                    v.run(op, Some(file.clone()), w, cx);
                                }))
                        ))
                        .child(control!(
                            "Request revision",
                            Button::new("revision")
                                .ghost()
                                .compact()
                                .label("Request revision")
                                .disabled(self.busy)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(FollowUp(format!("Please revise {path}. ")));
                                }))
                        )),
                )
            })
            .child(
                div()
                    .id("patch")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .when(review.is_some_and(|r| r.selected.is_some()), |patch| {
                        patch.bg(theme.sidebar).rounded_md()
                    })
                    .p_2()
                    .font_family(theme.mono_font_family.clone())
                    .text_sm()
                    .when_some(review.filter(|r| r.selected.is_some()), |body, r| {
                        let mut offset = 0;
                        let highlights = r.patch.split_inclusive('\n').filter_map(|line| {
                            let start = offset;
                            offset += line.len();
                            let color = match line.as_bytes().first() {
                                Some(b'+') => theme.success,
                                Some(b'-') => theme.danger,
                                Some(b'@') => theme.accent_foreground,
                                _ => return None,
                            };
                            Some((
                                start..offset,
                                HighlightStyle {
                                    color: Some(color),
                                    ..Default::default()
                                },
                            ))
                        });
                        body.child(control!(
                            format!("patch/{}", r.patch),
                            StyledText::new(r.patch.clone()).with_highlights(highlights)
                        ))
                    }),
            )
            .when_some(self.receipt.clone(), |p, msg| {
                p.child(div().text_sm().child(msg))
            })
            .when_some(self.error.clone(), |p, msg| {
                p.child(control!(
                    format!("error/{msg}"),
                    div().text_sm().text_color(theme.danger).child(msg)
                ))
            })
            .child(
                h_flex()
                    .items_end()
                    .flex_shrink_0()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(field("Commit message", &self.message)),
                    )
                    .child(control!(
                        if merging { "Commit merge" } else { "Commit staged" },
                        Button::new("commit")
                            .primary()
                            .label(if merging { "Commit merge".to_owned() } else { format!("Commit staged ({staged})") })
                            .disabled(
                                self.busy
                                    || (staged == 0 && !merging)
                                    || conflicts
                                    || self.message.read(cx).value().trim().is_empty()
                            )
                            .on_click(cx.listener(|v, _, w, cx| {
                                let msg = v.message.read(cx).value().to_string();
                                v.run(Operation::Commit(msg), None, w, cx);
                            }))
                    )),
            )
    }
}
