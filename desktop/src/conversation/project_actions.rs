use super::*;
use crate::github::{self, Action as PrAction, Status};
use anyhow::{Context as _, ensure};
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use std::{process::Command as ProcessCommand, time::Duration};

impl PrAction {
    fn label(self) -> &'static str {
        match self {
            Self::Create => "Create PR",
            Self::Resolve => "Resolve",
            Self::Fix => "Fix",
            Self::Merge => "Merge",
            Self::AutoMerge => "Auto-merge",
            Self::Queue => "Queue merge",
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
            Self::Create => "Create a pull request on GitHub",
            Self::Resolve => "Ask this conversation to resolve merge conflicts",
            Self::Fix => "Ask this conversation to fix failed checks or address requested changes",
            Self::Merge => "Review and merge the pull request",
            Self::AutoMerge => "Merge automatically once GitHub’s requirements are met",
            Self::Queue => "Review and add the pull request to GitHub’s merge queue",
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
                for (name, bundle) in [
                    ("Finder", "com.apple.finder"),
                    ("Terminal", "com.apple.Terminal"),
                    ("VS Code", "com.microsoft.VSCode"),
                ] {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                        let _ = view.update(cx, |view, cx| {
                            view.run_project_action(cx, move |root| {
                                let output = ProcessCommand::new("/usr/bin/open")
                                    .args(["-b", bundle])
                                    .arg(root)
                                    .output()
                                    .with_context(|| format!("Could not open {name}"))?;
                                ensure!(
                                    output.status.success(),
                                    "Could not open {name}. {}",
                                    String::from_utf8_lossy(&output.stderr).trim()
                                );
                                Ok(())
                            });
                        });
                    }));
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
                        let url = pr.url.clone();
                        menu = menu.item(
                            PopupMenuItem::new(format!(
                                "{} #{} → {}",
                                pr.repository(),
                                pr.number,
                                pr.base_ref_name,
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
        let root = self.workspaces[&id].root.clone();
        let executor = cx.background_executor().clone();
        self.pull_request_watch = Some(cx.spawn(async move |view, cx| {
            loop {
                let root = root.clone();
                let status = executor
                    .spawn(async move { github::status(&root).await })
                    .await;
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
                executor.timer(Duration::from_secs(20)).await;
            }
        }));
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
                let root = self.workspaces[&id].root.clone();
                self.pull_request_action.insert(id.clone());
                cx.notify();
                let job = cx.background_spawn(async move { github::create_url(&root).await });
                cx.spawn(async move |view, cx| {
                    let result = job.await;
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
            (
                Status::Found { branch, pr },
                PrAction::Merge | PrAction::AutoMerge | PrAction::Queue,
            ) => {
                let branch = branch.clone();
                let pr = pr.clone();
                let target = format!(
                    "{} #{} into {}",
                    pr.repository(),
                    pr.number,
                    pr.base_ref_name
                );
                let (title, button) = match action {
                    PrAction::Merge => ("Merge pull request?", "Merge"),
                    PrAction::AutoMerge => ("Enable auto-merge?", "Enable auto-merge"),
                    PrAction::Queue => ("Add pull request to the merge queue?", "Queue merge"),
                    _ => unreachable!(),
                };
                let detail = match (pr.uses_merge_queue(), pr.merge_method()) {
                    (true, _) => target,
                    (_, Some(method)) => format!("{}. {target}.", method.description()),
                    _ => return,
                };
                let id = thread.id.clone();
                let root = self.workspaces[&id].root.clone();
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
                            cx.background_spawn(async move {
                                github::update(&root, &branch, &pr, action).await
                            })
                            .await,
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
            (Status::Found { pr, .. }, _) => cx.open_url(&pr.url),
            _ => {}
        }
    }

    fn handoff_pull_request(&mut self, action: PrAction, status: Status, cx: &mut Context<Self>) {
        let id = self.current.as_ref().unwrap().id.clone();
        let root = self.workspaces[&id].root.clone();
        self.pull_request_action.insert(id.clone());
        cx.notify();
        let job = cx.background_spawn(async move { github::handoff(&root, &status, action).await });
        cx.spawn(async move |view, cx| {
            let result = job.await;
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
