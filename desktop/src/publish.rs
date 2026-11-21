use crate::git::{self, Publication};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use std::path::PathBuf;

pub struct Publish {
    root: PathBuf,
    state: Option<Publication>,
    busy: bool,
    feedback: Option<String>,
    error: Option<String>,
}

enum Action {
    Inspect(Option<String>),
    Push {
        branch: String,
        head: String,
        remote: String,
    },
}

impl Publish {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            state: None,
            busy: false,
            feedback: None,
            error: None,
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let remote = self.state.as_ref().and_then(|s| s.selected.clone());
        self.run(Action::Inspect(remote), cx);
    }

    fn run(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.feedback = None;
        let root = self.root.clone();
        let job = cx.background_spawn(async move {
            let (remote, receipt) = match action {
                Action::Inspect(remote) => (remote, Ok(None)),
                Action::Push {
                    remote,
                    branch,
                    head,
                } => {
                    let pushed = git::push(&root, &branch, &head, &remote)
                        .map(|_| Some(format!("Pushed {branch} to {remote}.")));
                    (Some(remote), pushed)
                }
            };
            (git::publication(&root, remote.as_deref()), receipt)
        });
        cx.spawn(async move |view, cx| {
            let (state, receipt) = job.await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                match receipt {
                    Ok(message) => view.feedback = message,
                    Err(error) => view.error = Some(error.to_string()),
                }
                match state {
                    Ok(state) => view.state = Some(state),
                    Err(error) => {
                        view.state = None;
                        view.error.get_or_insert_with(|| error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn branch_controls(&self, state: &Publication, cx: &Context<Self>) -> Div {
        let panel = v_flex().gap_3();
        let Some(branch) = &state.branch else {
            return panel.child(
                "This checkout has a detached HEAD. Create a branch to publish its commits.",
            );
        };
        let Some(head) = &state.head else {
            return panel.child("Create the first commit in Changes to publish this branch.");
        };
        let panel = panel.child(
            div()
                .font_family(cx.theme().mono_font_family.clone())
                .child(format!("{branch} · {}", &head[..8.min(head.len())])),
        );
        let Some(remote) = &state.selected else {
            return panel.child("Add a Git remote to this checkout, then refresh.");
        };
        let remotes = state.remotes.iter().map(|name| {
            let selected = name.clone();
            let button = Button::new(SharedString::from(format!("remote-{name}")))
                .label(name.clone())
                .selected(name == remote)
                .disabled(self.busy)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.run(Action::Inspect(Some(selected.clone())), cx);
                }));
            control!(format!("remote/{name}"), button)
        });
        let destination = format!("Push to {remote}/{branch}");
        let (branch, head, remote) = (branch.clone(), head.clone(), remote.clone());
        let push = Button::new("push-branch")
            .primary()
            .label("Push branch")
            .disabled(self.busy)
            .on_click(cx.listener(move |view, _, _, cx| {
                view.run(
                    Action::Push {
                        branch: branch.clone(),
                        head: head.clone(),
                        remote: remote.clone(),
                    },
                    cx,
                );
            }));
        let actions = h_flex()
            .gap_2()
            .flex_wrap()
            .child(control!("Push branch", push))
            .when_some(state.compare_url.clone(), |row, url| {
                let compare = Button::new("compare")
                    .label("Open pull request form")
                    .disabled(self.busy || !state.synced)
                    .tooltip(if state.synced {
                        "Review this branch against the repository’s default branch on GitHub"
                    } else {
                        "Push this branch before opening a pull request"
                    })
                    .on_click(move |_, _, cx| cx.open_url(&url));
                row.child(control!("Open pull request form", compare))
            });
        panel
            .child(div().text_sm().child("Remote"))
            .child(h_flex().flex_wrap().gap_1().children(remotes))
            .child(destination)
            .child(actions)
    }
}

impl Render for Publish {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let refresh = Button::new("refresh-branch")
            .label("Refresh")
            .disabled(self.busy)
            .on_click(cx.listener(|view, _, _, cx| view.refresh(cx)));
        v_flex()
            .id("publish-panel")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .child(div().text_lg().child("Publish branch"))
                    .child(control!("Refresh branch", refresh)),
            )
            .when_some(self.state.as_ref(), |panel, state| {
                panel.child(self.branch_controls(state, cx))
            })
            .when(self.busy, |panel| panel.child("Working…"))
            .when_some(self.feedback.clone(), |panel, message| {
                panel.child(control!(message.clone(), message))
            })
            .when_some(self.error.clone(), |panel, message| {
                panel.child(control!(
                    format!("error/{message}"),
                    div().text_color(cx.theme().danger).child(message)
                ))
            })
    }
}
