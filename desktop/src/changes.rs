use crate::{
    git::{self, File},
    ui::{hint, label},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use std::path::PathBuf;

pub struct Changes {
    root: PathBuf,
    files: Option<anyhow::Result<Vec<File>>>,
    refresh_task: Option<Task<()>>,
}

impl Changes {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: None,
            refresh_task: None,
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let job = cx.background_spawn(async move { git::changed_files(&root) });
        self.refresh_task = Some(cx.spawn(async move |view, cx| {
            let files = job.await;
            let _ = view.update(cx, |panel, cx| {
                panel.files = Some(files);
                cx.notify();
            });
        }));
    }
}

impl Render for Changes {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.files {
            None => hint("Reading checkout…", cx),
            Some(Err(error)) => control!(
                format!("Changes error/{error}"),
                hint("Changes unavailable", cx)
                    .id("changes-error")
                    .tooltip({
                        let text = error.to_string();
                        move |window, cx| {
                            gpui_component::tooltip::Tooltip::new(text.clone()).build(window, cx)
                        }
                    })
            ),
            Some(Ok(files)) if files.is_empty() => {
                control!("Working tree clean", hint("Working tree clean", cx))
            }
            Some(Ok(files)) => div().flex_1().min_h_0().child(
                v_flex()
                    .id("change-files")
                    .h_full()
                    .overflow_y_scroll()
                    .children(files.iter().map(|file| {
                        let path = file.path.to_string_lossy().into_owned();
                        control!(
                            format!("Changed/{path}"),
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .child(div().min_w_0().flex_1().truncate().child(path.clone()))
                                .child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(file.status.clone())
                                )
                                .id(SharedString::from(path.clone()))
                                .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(
                                    path.clone()
                                )
                                .build(window, cx))
                        )
                    })),
            ),
        };
        v_flex()
            .size_full()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .justify_between()
                    .child(label("Changes"))
                    .child(control!(
                        "Refresh changes",
                        Button::new("refresh-changes")
                            .small()
                            .compact()
                            .ghost()
                            .icon(IconName::Redo2)
                            .tooltip("Refresh changed files")
                            .on_click(cx.listener(|view, _, _, cx| view.refresh(cx)))
                    )),
            )
            .child(content)
    }
}
