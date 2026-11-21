use super::*;
use anyhow::{Context as _, ensure};
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use std::process::Command;

impl Desktop {
    pub(super) fn project_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.weak_entity();
        let launcher = Button::new("open-project")
            .small()
            .ghost()
            .icon(IconName::FolderOpen)
            .dropdown_caret(true)
            .tooltip("Open project")
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
                                let output = Command::new("/usr/bin/open")
                                    .args(["-b", bundle])
                                    .arg(root)
                                    .output()
                                    .with_context(|| format!("Could not open {name}"))?;
                                ensure!(
                                    output.status.success(),
                                    "Could not open {name}. {}",
                                    String::from_utf8_lossy(&output.stderr).trim()
                                );
                                Ok(None)
                            });
                        });
                    }));
                }
                menu
            });
        h_flex()
            .gap_1()
            .flex_shrink_0()
            .child(control!("Open project", launcher))
            .child(control!(
                "Create PR",
                Button::new("create-pr")
                    .small()
                    .label("Create PR")
                    .tooltip("Create a pull request on GitHub")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.run_project_action(cx, |root| git::pull_request_url(&root).map(Some));
                    }))
            ))
    }

    fn run_project_action(
        &self,
        cx: &mut Context<Self>,
        action: impl FnOnce(PathBuf) -> anyhow::Result<Option<String>> + Send + 'static,
    ) {
        let Some(thread) = &self.current else { return };
        let id = thread.id.clone();
        let root = self.workspaces[&id].root.clone();
        let job = cx.background_spawn(async move { action(root) });
        cx.spawn(async move |view, cx| {
            let result = job.await;
            let _ = view.update(cx, |view, cx| {
                if view.current.as_ref().is_some_and(|thread| thread.id == id) {
                    if let Ok(Some(url)) = &result {
                        cx.open_url(url);
                    }
                    view.error = result.err().map(|error| error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
