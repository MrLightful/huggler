use super::*;
use crate::github::{Action as PrAction, Checkout, Status};
use crate::open_in::OpenIn;
use anyhow::Context as _;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use huggler::repository::github::{MergeMethod, PullRequestUpdate};
use std::time::Duration;

impl PrAction {
    fn label(self) -> &'static str {
        match self {
            Self::Create => "Create PR",
            Self::Resolve => "Resolve",
            Self::Fix => "Fix",
            Self::Update(PullRequestUpdate::Merge(_)) => "Merge",
            Self::Update(PullRequestUpdate::EnableAutoMerge(_)) => "Auto-merge",
            Self::Update(PullRequestUpdate::Enqueue) => "Queue merge",
            Self::View => "View PR",
            Self::Draft => "View draft",
            Self::AutoMergeEnabled => "Auto-merge on",
            Self::Queued => "Merge queued",
            Self::Merged => "Merged",
            Self::Closed => "PR closed",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Create => "Open PR form",
            Self::Resolve => "Ask this conversation to resolve merge conflicts",
            Self::Fix => "Ask this conversation to fix failed checks or address requested changes",
            Self::Update(PullRequestUpdate::Merge(_)) => "Review and merge the pull request",
            Self::Update(PullRequestUpdate::EnableAutoMerge(_)) => {
                "Merge automatically once GitHub’s requirements are met"
            }
            Self::Update(PullRequestUpdate::Enqueue) => {
                "Review and add the pull request to GitHub’s merge queue"
            }
            Self::View => "Open the pull request on GitHub",
            Self::Draft => "Open the draft pull request on GitHub",
            Self::AutoMergeEnabled => "Auto-merge is enabled. Open the pull request on GitHub.",
            Self::Queued => "The pull request is in GitHub’s merge queue. Open it on GitHub.",
            Self::Merged => "Open the merged pull request on GitHub",
            Self::Closed => "Open the closed pull request on GitHub",
        }
    }
}

impl Desktop {
    pub(super) fn project_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.weak_entity();
        let launcher = Button::new("open-project")
            .small()
            .compact()
            .ghost()
            .child(
                h_flex()
                    .gap_1p5()
                    .child(Icon::new(IconName::FolderOpen).small())
                    .child(Icon::new(IconName::ChevronDown).xsmall()),
            )
            .tooltip("Open in")
            .dropdown_menu_with_anchor(Corner::TopRight, move |mut menu, _, _| {
                for destination in OpenIn::OPTIONS {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(destination.label()).on_click(
                        move |_, _, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.run_project_action(cx, move |root| {
                                    destination.open_workspace(&root)
                                });
                            });
                        },
                    ));
                }
                menu
            });
        h_flex()
            .gap_2()
            .flex_shrink_0()
            .child(control!("Open in", launcher))
            .child(self.pull_request_control(cx))
    }

    fn pull_request_control(&self, cx: &Context<Self>) -> impl IntoElement {
        let thread = self.current.as_ref().unwrap();
        let busy = self.pull_request_action.contains(&thread.id);
        let action = self
            .pull_request
            .as_ref()
            .map_or(PrAction::Create, Status::action);
        let tooltip = match &self.pull_request {
            _ if busy => "Updating pull request…".into(),
            None => "Checking pull request status…".into(),
            Some(Status::Unavailable(reason)) => format!("Create PR on GitHub. {reason}"),
            _ => action.tooltip().into(),
        };
        let label = if busy {
            "Updating PR…"
        } else if self.pull_request.is_none() {
            "Checking PR…"
        } else {
            action.label()
        };
        let button = Button::new("pull-request-action")
            .small()
            .label(label)
            .tooltip(tooltip)
            .disabled(busy || self.pull_request.is_none());
        let button = if let Some(Status::Multiple(requests)) = &self.pull_request {
            let requests = requests.clone();
            button
                .dropdown_menu_with_anchor(Corner::TopRight, move |mut menu, _, _| {
                    for pr in &requests {
                        let url = pr.url().to_owned();
                        menu = menu.item(
                            PopupMenuItem::new(format!(
                                "{} #{} → {}",
                                pr.repository(),
                                pr.number(),
                                pr.base_branch(),
                            ))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                        );
                    }
                    menu
                })
                .into_any_element()
        } else {
            button
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.activate_pull_request(action, window, cx);
                }))
                .into_any_element()
        };
        control!(label, button)
    }

    pub(super) fn watch_pull_request(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = &self.current else { return };
        let id = thread.id.clone();
        let checkout = self.pull_request_checkout(&id);
        let executor = self.executor.clone();
        let background = cx.background_executor().clone();
        self.pull_request_watch = Some(cx.spawn(async move |view, cx| {
            loop {
                let read = checkout.clone();
                let status = executor
                    .spawn(async move { read.status().await })
                    .await
                    .unwrap_or_else(|err| Status::Unavailable(err.to_string()));
                let still_selected = view.update(cx, |view, cx| {
                    if view.current.as_ref().is_none_or(|thread| thread.id != id) {
                        return false;
                    }
                    view.pull_request = Some(status);
                    cx.notify();
                    true
                });
                if !matches!(still_selected, Ok(true)) {
                    break;
                }
                background.timer(Duration::from_secs(20)).await;
            }
        }));
    }

    fn pull_request_checkout(&self, id: &ThreadId) -> Checkout {
        Checkout {
            runtime: self.runtime.clone(),
            thread: id.clone(),
            root: self.workspaces[id].root.clone(),
        }
    }

    fn activate_pull_request(
        &mut self,
        action: PrAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = &self.current else { return };
        let Some(status) = &self.pull_request else {
            return;
        };
        if self.pull_request_action.contains(&thread.id) || status.action() != action {
            return;
        }
        match (status, action) {
            (Status::Create | Status::Unavailable(_), PrAction::Create) => {
                let id = thread.id.clone();
                let checkout = self.pull_request_checkout(&id);
                self.pull_request_action.insert(id.clone());
                cx.notify();
                let job = self
                    .executor
                    .spawn(async move { checkout.create_url().await });
                cx.spawn(async move |view, cx| {
                    let result = job
                        .await
                        .context("Run pull request action")
                        .and_then(|result| result);
                    let _ = view.update(cx, |view, cx| {
                        let result = result.map(|url| {
                            if view.current.as_ref().is_some_and(|thread| thread.id == id) {
                                cx.open_url(&url);
                            }
                        });
                        view.finish_pull_request_action(id, Some(result), cx);
                    });
                })
                .detach();
            }
            (Status::Conflicts | Status::Found { .. }, PrAction::Resolve | PrAction::Fix) => {
                self.handoff_pull_request(action, status.clone(), cx);
            }
            (Status::Found { branch, pr }, PrAction::Update(update)) => {
                let branch = branch.clone();
                let pr = pr.clone();
                let target = format!(
                    "{} #{} into {}",
                    pr.repository(),
                    pr.number(),
                    pr.base_branch(),
                );
                let (title, button, method) = match update {
                    PullRequestUpdate::Merge(method) => {
                        ("Merge pull request?", "Merge", Some(method))
                    }
                    PullRequestUpdate::EnableAutoMerge(method) => {
                        ("Enable auto-merge?", "Enable auto-merge", method)
                    }
                    PullRequestUpdate::Enqueue => {
                        ("Add pull request to the merge queue?", "Queue merge", None)
                    }
                };
                let detail = match method {
                    Some(method) => {
                        let description = match method {
                            MergeMethod::Merge => "Create a merge commit",
                            MergeMethod::Squash => "Squash and merge",
                            MergeMethod::Rebase => "Rebase and merge",
                        };
                        format!("{description}. {target}.")
                    }
                    None => target,
                };
                let id = thread.id.clone();
                let checkout = self.pull_request_checkout(&id);
                let executor = self.executor.clone();
                self.pull_request_action.insert(id.clone());
                cx.notify();
                #[cfg(all(feature = "qualification", target_os = "macos"))]
                let scripted = crate::qualification::take_prompt_answer(cx);
                #[cfg(not(all(feature = "qualification", target_os = "macos")))]
                let scripted: Option<usize> = None;
                let choice = scripted.is_none().then(|| {
                    window.prompt(
                        PromptLevel::Warning,
                        title,
                        Some(&detail),
                        &["Cancel", button],
                        cx,
                    )
                });
                cx.spawn(async move |view, cx| {
                    let answer = match choice {
                        Some(choice) => choice.await.ok(),
                        None => scripted,
                    };
                    let result = if answer == Some(1) {
                        Some(
                            executor
                                .spawn(async move { checkout.update(&branch, &pr, update).await })
                                .await
                                .context("Run pull request action")
                                .and_then(|result| result),
                        )
                    } else {
                        None
                    };
                    let _ = view.update(cx, |view, cx| {
                        view.finish_pull_request_action(id, result, cx);
                    });
                })
                .detach();
            }
            (Status::Found { pr, .. }, _) => cx.open_url(pr.url()),
            _ => {}
        }
    }

    fn handoff_pull_request(&mut self, action: PrAction, status: Status, cx: &mut Context<Self>) {
        let id = self.current.as_ref().unwrap().id.clone();
        let checkout = self.pull_request_checkout(&id);
        self.pull_request_action.insert(id.clone());
        cx.notify();
        let job = self
            .executor
            .spawn(async move { checkout.handoff(&status, action).await });
        cx.spawn(async move |view, cx| {
            let result = job
                .await
                .context("Run pull request action")
                .and_then(|result| result);
            let _ = view.update(cx, |view, cx| {
                let result = result.and_then(|text| {
                    view.runtime.dispatch(Command {
                        id: CommandId(Uuid::new_v4().to_string()),
                        thread_id: id.clone(),
                        action: Action::Enqueue { text },
                    })?;
                    Ok(())
                });
                view.finish_pull_request_action(id, Some(result), cx);
            });
        })
        .detach();
    }

    fn finish_pull_request_action(
        &mut self,
        id: ThreadId,
        result: Option<anyhow::Result<()>>,
        cx: &mut Context<Self>,
    ) {
        self.pull_request_action.remove(&id);
        if self.current.as_ref().is_some_and(|thread| thread.id == id) {
            if let Some(result) = result {
                self.error = result.err().map(|error| format!("{error:#}"));
                self.pull_request = None;
                self.watch_pull_request(cx);
            }
            cx.notify();
        }
    }

    fn run_project_action(
        &self,
        cx: &mut Context<Self>,
        action: impl FnOnce(PathBuf) -> anyhow::Result<()> + Send + 'static,
    ) {
        let Some(thread) = &self.current else { return };
        let id = thread.id.clone();
        let root = self.workspaces[&id].root.clone();
        let job = cx.background_spawn(async move { action(root) });
        cx.spawn(async move |view, cx| {
            let result = job.await;
            let _ = view.update(cx, |view, cx| {
                if view.current.as_ref().is_some_and(|thread| thread.id == id) {
                    view.error = result.err().map(|error| error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
