use super::*;
use crate::open_in::OpenIn;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};

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
                                    destination.open_workspace(&root).map(|()| None)
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
            .child(control!(
                "Create PR",
                Button::new("create-pr")
                    .small()
                    .label("Create PR")
                    .tooltip("Open PR form")
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
