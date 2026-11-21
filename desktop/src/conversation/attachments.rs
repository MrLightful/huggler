use super::*;
use anyhow::{Context as _, ensure};
use gpui_component::Icon;
use huggler::TextAttachment;
use std::{io::Read, path::Path};

const MAX_FILES: usize = 8;
const MAX_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct Attachments {
    files: Vec<TextAttachment>,
    pub(super) loading: bool,
    error: Option<String>,
}

impl Attachments {
    pub(super) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub(super) fn first_name(&self) -> Option<&str> {
        self.files.first().map(|file| file.name.as_str())
    }

    pub(super) fn action(&self, text: String) -> Action {
        if self.is_empty() {
            Action::Enqueue { text }
        } else {
            Action::EnqueueAttachments {
                text,
                files: self.files.clone(),
            }
        }
    }
}

fn read_file(path: &Path) -> anyhow::Result<TextAttachment> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .context("Invalid filename")?;
    ensure!(fs::metadata(path)?.is_file(), "Choose a regular file");
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_BYTES, "Choose a file of 64 KiB or less");
    let content = String::from_utf8(bytes).context("Choose a UTF-8 text file")?;
    ensure!(!content.contains('\0'), "Choose a UTF-8 text file");
    Ok(TextAttachment {
        name: name.to_owned(),
        content,
    })
}

impl Desktop {
    pub(super) fn sent_file(
        &self,
        thread: &ThreadId,
        message: &huggler::MessageId,
        position: usize,
        name: &str,
        cx: &Context<Self>,
    ) -> Div {
        let key = (message.clone(), position);
        let thread = thread.clone();
        let preview = self
            .file_preview
            .as_ref()
            .filter(|(id, _)| id == &key)
            .map(|(_, text)| text);
        let expanded = preview.is_some();
        let filename = name.to_owned();
        let button_id = SharedString::from(format!("attachment-{message}-{position}"));
        v_flex()
            .gap_1()
            .min_w_0()
            .child(h_flex().min_w_0().child(control!(
                format!("attachment/{message}/{position}/{name}"),
                Button::new(button_id.clone())
                    .small()
                    .compact()
                    .ghost()
                    .max_w_full()
                    .min_w_0()
                    .justify_start()
                    .icon(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .child(Icon::new(IconName::File).xsmall().flex_shrink_0())
                    .child(div().min_w_0().max_w(px(270.)).truncate().child(filename))
                    .tooltip(format!(
                        "{} {}",
                        if expanded { "Hide" } else { "Preview" },
                        name
                    ))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if expanded {
                            view.file_preview = None;
                        } else {
                            match view.runtime.read_attachment(&thread, &key.0, position) {
                                Ok(text) => view.file_preview = Some((key.clone(), text)),
                                Err(error) => view.error = Some(error.to_string()),
                            }
                        }
                        cx.notify();
                    }))
            )))
            .when_some(preview, |row, text| {
                let body = if text.is_empty() {
                    "Empty file"
                } else {
                    text.as_str()
                };
                row.child(super::transcript::text_preview(
                    SharedString::from(format!("preview-{button_id}")),
                    control!(format!("file preview/{body}"), body.to_owned()),
                    px(176.),
                    cx,
                ))
            })
    }

    fn attachments_mut(&mut self, thread: Option<&ThreadId>) -> &mut Attachments {
        match thread {
            Some(id) => {
                &mut self
                    .workspaces
                    .get_mut(id)
                    .expect("composer workspace")
                    .attachments
            }
            None => &mut self.starter_attachments,
        }
    }

    fn pick_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let thread = self.current.as_ref().map(|t| t.id.clone());
        let draft = self.attachments_mut(thread.as_ref());
        if draft.loading || draft.files.len() >= MAX_FILES {
            return;
        }
        let remaining = MAX_FILES - draft.files.len();
        draft.loading = true;
        draft.error = None;
        cx.notify();
        let choice = select_files(cx);
        cx.spawn_in(window, async move |view, cx| {
            let result: anyhow::Result<Vec<TextAttachment>> = async {
                let paths = choice.await?;

                ensure!(
                    paths.len() <= remaining,
                    "Attach up to eight files per message"
                );
                cx.background_spawn(async move {
                    paths
                        .iter()
                        .map(|path| read_file(path).with_context(|| path.display().to_string()))
                        .collect()
                })
                .await
            }
            .await;
            let _ = view.update(cx, |view, cx| {
                let draft = view.attachments_mut(thread.as_ref());
                draft.loading = false;
                match result {
                    Ok(files) => draft.files.extend(files),
                    Err(error) => draft.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn attach_button(&self, draft: &Attachments, cx: &Context<Self>) -> Div {
        control!(
            "Attach files",
            Button::new("attach-files")
                .ghost()
                .small()
                .icon(IconName::Plus)
                .tooltip("Attach files (UTF-8 text, 64 KiB each)")
                .disabled(draft.loading || draft.files.len() == MAX_FILES)
                .on_click(cx.listener(|view, _, window, cx| view.pick_attachments(window, cx)))
        )
    }

    pub(super) fn attachment_list(&self, draft: &Attachments, cx: &Context<Self>) -> Option<Div> {
        if draft.is_empty() && draft.error.is_none() && !draft.loading {
            return None;
        }
        Some(
            h_flex()
                .gap_1()
                .flex_wrap()
                .children(draft.files.iter().enumerate().map(|(index, file)| {
                    let title = format!("Remove {}", file.name);
                    control!(
                        title.clone(),
                        Button::new(("remove-attachment", index))
                            .small()
                            .compact()
                            .icon(IconName::Close)
                            .child(div().max_w(px(168.)).truncate().child(file.name.clone()))
                            .tooltip(title)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                let thread = view.current.as_ref().map(|t| t.id.clone());
                                view.attachments_mut(thread.as_ref()).files.remove(index);
                                cx.notify();
                            }))
                    )
                }))
                .when(draft.loading, |row| row.child(hint("Selecting files…", cx)))
                .when_some(draft.error.as_ref(), |row, error| {
                    row.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(error.clone()),
                    )
                }),
        )
    }
}

fn select_files(cx: &mut Context<Desktop>) -> Task<anyhow::Result<Vec<PathBuf>>> {
    #[cfg(all(feature = "qualification", target_os = "macos"))]
    if let Some(paths) = crate::qualification::take_file_selection(cx) {
        return Task::ready(Ok(paths));
    }
    let choice = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some("Attach text files".into()),
    });
    cx.spawn(async move |_, _| Ok(choice.await??.unwrap_or_default()))
}
