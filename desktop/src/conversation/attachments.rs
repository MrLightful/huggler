use super::*;
use anyhow::{Context as _, ensure};
use huggler::Attachment;
use std::{
    io::{Read, Write},
    path::Path,
};

const PREVIEW_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct Attachments {
    files: Vec<Attachment>,
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
            Action::EnqueueFiles {
                text,
                files: self.files.clone(),
            }
        }
    }
}

fn read_file(path: &Path, remaining: usize) -> anyhow::Result<Attachment> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .context("Invalid filename")?;
    let metadata = fs::metadata(path)?;
    ensure!(metadata.is_file(), "Choose a regular file");
    ensure!(
        metadata.len() <= remaining as u64,
        "Attachments can total up to 50 MiB"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((remaining + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= remaining,
        "Attachments can total up to 50 MiB"
    );
    Ok(Attachment::from_bytes(name.to_owned(), bytes)?)
}

#[derive(Clone)]
pub(super) enum FilePreview {
    Loading,
    Ready(Preview),
    Failed(String),
}

#[derive(Clone)]
pub(super) struct Preview {
    name: String,
    mime: String,
    size: usize,
    body: PreviewBody,
}

#[derive(Clone)]
enum PreviewBody {
    Text { text: SharedString, truncated: bool },
    Image(Arc<RenderImage>),
    ImageUnavailable,
    PdfText(SharedString),
    File,
}

impl Preview {
    fn from_file(file: Attachment) -> Self {
        let size = file.bytes.len();
        let body = if matches!(
            file.mime.as_str(),
            "image/jpeg" | "image/png" | "image/webp" | "image/gif"
        ) {
            image_preview(&file.bytes).map_or(PreviewBody::ImageUnavailable, PreviewBody::Image)
        } else if let Some(text) = file.pdf_text_excerpt() {
            PreviewBody::PdfText(text.into())
        } else if let Some(text) = file.text() {
            PreviewBody::Text {
                text: SharedString::from(
                    text[..text.floor_char_boundary(PREVIEW_BYTES)].to_owned(),
                ),
                truncated: text.len() > PREVIEW_BYTES,
            }
        } else {
            PreviewBody::File
        };
        Self {
            name: file.name,
            mime: file.mime,
            size,
            body,
        }
    }

    fn description(&self) -> String {
        let size = match self.size {
            n if n >= 1024 * 1024 => format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0)),
            n if n >= 1024 => format!("{:.1} KiB", n as f64 / 1024.0),
            n => format!("{n} B"),
        };
        format!("{} · {size}", self.mime)
    }
}

pub(super) fn image_preview(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    use image::{Frame, ImageReader, Limits};
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(8_192);
    limits.max_image_height = Some(8_192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let mut thumbnail = reader.decode().ok()?.thumbnail(840, 440).into_rgba8();
    for pixel in thumbnail.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new([Frame::new(thumbnail)])))
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
            .map(|(_, preview)| preview);
        let expanded = preview.is_some();
        let button_id = SharedString::from(format!("attachment-{message}-{position}"));
        let preview_key = key.clone();
        let preview_thread = thread.clone();
        v_flex()
            .min_w_0()
            .gap_1()
            .child(h_flex().min_w_0().child(control!(
                format!("attachment/{message}/{position}/{name}"),
                Button::new(button_id.clone())
                    .small()
                    .compact()
                    .ghost()
                    .max_w_full()
                    .min_w_0()
                    .justify_start()
                    .icon(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .child(Icon::new(IconName::File).xsmall().flex_shrink_0())
                    .child(div().min_w_0().max_w(px(270.)).truncate().child(name.to_owned()))
                    .tooltip(format!("{} {}", if expanded { "Hide" } else { "Preview" }, name))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        if expanded {
                            view.file_preview = None;
                            cx.notify();
                        } else {
                            view.load_file_preview(preview_thread.clone(), preview_key.clone(), window, cx);
                        }
                    }))
            )))
            .when_some(preview, |row, preview| row.child(match preview {
                FilePreview::Loading => hint("Loading attachment…", cx),
                FilePreview::Failed(error) => div().text_sm().text_color(cx.theme().danger).child(error.clone()),
                FilePreview::Ready(preview) => {
                    let body = match &preview.body {
                        PreviewBody::Text { text, truncated } => {
                            let body = if text.is_empty() { "Empty file".into() } else { text.clone() };
                            v_flex().gap_1().child(super::transcript::text_preview(
                                SharedString::from(format!("preview-{button_id}")),
                                control!(format!("file preview/{body}"), body),
                                px(176.),
                                cx,
                            )).when(*truncated, |row| row.child(hint("Showing the first 64 KiB. Save the file for the full text.", cx)))
                        }
                        PreviewBody::Image(image) => div().w_full().max_w(px(420.)).child(control!(
                            format!("image preview/{}", preview.name),
                            img(image.clone()).w_full().h(px(220.)).object_fit(ObjectFit::Contain)
                        )),
                        PreviewBody::ImageUnavailable => hint("Image preview unavailable", cx),
                        PreviewBody::PdfText(text) => v_flex()
                            .w_full()
                            .gap_1()
                            .child(hint("Extracted PDF text", cx))
                            .child(div()
                                .id("pdf-text")
                                .w_full().max_h(px(176.)).overflow_y_scroll()
                                .rounded_md().p_2().bg(cx.theme().muted)
                                .text_sm().whitespace_normal()
                                .child(control!(format!("pdf preview/{}", preview.name), text.clone()))),
                        PreviewBody::File => div(),
                    };
                    let name = preview.name.clone();
                    v_flex().items_start().gap_1()
                        .w_full()
                        .child(body)
                        .child(h_flex().flex_wrap().gap_2().items_center()
                            .child(hint(preview.description(), cx))
                            .child(control!(format!("Save {name}"), Button::new("save-attachment")
                                .small().ghost().label("Save as…")
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.save_attachment(thread.clone(), key.clone(), &name, window, cx);
                                })))))
                }
            }))
    }

    fn load_file_preview(
        &mut self,
        thread: ThreadId,
        key: (huggler::MessageId, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_preview = Some((key.clone(), FilePreview::Loading));
        let runtime = self.runtime.clone();
        let read_key = key.clone();
        let read = cx.background_spawn(async move {
            runtime
                .read_attachment_file(&thread, &read_key.0, read_key.1)
                .map(Preview::from_file)
        });
        cx.spawn_in(window, async move |view, cx| {
            let preview = match read.await {
                Ok(preview) => FilePreview::Ready(preview),
                Err(error) => FilePreview::Failed(error.to_string()),
            };
            let _ = view.update(cx, |view, cx| {
                if let Some((current, state)) = &mut view.file_preview
                    && *current == key
                    && matches!(state, FilePreview::Loading)
                {
                    *state = preview;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn save_attachment(
        &mut self,
        thread: ThreadId,
        (message, position): (huggler::MessageId, usize),
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choice = select_save_path(name, cx);
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |view, cx| {
            let result: anyhow::Result<()> = async {
                if let Some(path) = choice.await? {
                    cx.background_spawn(async move {
                        let file = runtime.read_attachment_file(&thread, &message, position)?;
                        let directory = path.parent().context("Missing save directory")?;
                        let mut output = tempfile::NamedTempFile::new_in(directory)?;
                        output.write_all(&file.bytes)?;
                        output.persist(path)?;
                        anyhow::Ok(())
                    })
                    .await?;
                }
                Ok(())
            }
            .await;
            if let Err(error) = result {
                let _ = view.update(cx, |view, cx| {
                    view.error = Some(format!("Could not save attachment. {error}"));
                    cx.notify();
                });
            }
        })
        .detach();
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
        let choice = select_files(cx);
        self.load_attachments(choice, window, cx);
    }

    pub(super) fn drop_attachments(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_attachments(Task::ready(Ok(paths.paths().to_vec())), window, cx);
    }

    pub(super) fn attach_workspace_file(
        &mut self,
        thread: &ThreadId,
        path: Arc<Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.workspaces[thread].root.clone();
        let choice = cx.background_spawn(async move {
            let target = root.canonicalize()?.join(&path);
            ensure!(
                target.symlink_metadata()?.is_file() && target.canonicalize()? == target,
                "The selected workspace file has moved."
            );
            Ok(vec![target])
        });
        self.load_attachments(choice, window, cx);
    }

    fn load_attachments(
        &mut self,
        choice: Task<anyhow::Result<Vec<PathBuf>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.launching.is_some() {
            return;
        }
        let thread = self.current.as_ref().map(|t| t.id.clone());
        let draft = self.attachments_mut(thread.as_ref());
        if draft.loading {
            return;
        }
        if draft.files.len() == Attachment::MAX_FILES {
            draft.error = Some("Attach up to eight files per message".into());
            cx.notify();
            return;
        }
        let remaining = Attachment::MAX_FILES - draft.files.len();
        let used = draft
            .files
            .iter()
            .map(|file| file.bytes.len())
            .sum::<usize>();
        draft.loading = true;
        draft.error = None;
        cx.notify();
        cx.spawn_in(window, async move |view, cx| {
            let result: anyhow::Result<Vec<Attachment>> = async {
                let paths = choice.await?;

                ensure!(
                    paths.len() <= remaining,
                    "Attach up to eight files per message"
                );
                cx.background_spawn(async move {
                    let mut budget = Attachment::MAX_TOTAL_BYTES - used;
                    let mut files = Vec::with_capacity(paths.len());
                    for path in paths {
                        let file =
                            read_file(&path, budget).with_context(|| path.display().to_string())?;
                        budget -= file.bytes.len();
                        files.push(file);
                    }
                    Ok(files)
                })
                .await
            }
            .await;
            let _ = view.update(cx, |view, cx| {
                let draft = match thread.as_ref() {
                    Some(id) => match view.workspaces.get_mut(id) {
                        Some(workspace) => &mut workspace.attachments,
                        None => return,
                    },
                    None => &mut view.starter_attachments,
                };
                draft.loading = false;
                match result {
                    Ok(files) if !files.is_empty() => {
                        let previous = draft.files.len();
                        draft.files.extend(files);
                        if let Err(error) = Attachment::validate_batch(&draft.files) {
                            draft.files.truncate(previous);
                            draft.error = Some(error.to_string());
                        }
                    }
                    Ok(_) => {}
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
                .tooltip("Attach files")
                .disabled(
                    self.launching.is_some()
                        || draft.loading
                        || draft.files.len() == Attachment::MAX_FILES
                )
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
                                let draft = view.attachments_mut(thread.as_ref());
                                draft.files.remove(index);
                                draft.error = None;
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
        prompt: Some("Attach files".into()),
    });
    cx.spawn(async move |_, _| Ok(choice.await??.unwrap_or_default()))
}

fn select_save_path(
    name: &str,
    cx: &mut Context<Desktop>,
) -> Task<anyhow::Result<Option<PathBuf>>> {
    #[cfg(all(feature = "qualification", target_os = "macos"))]
    if let Some(path) = crate::qualification::take_save_path(cx) {
        return Task::ready(Ok(Some(path)));
    }
    let directory = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let choice = cx.prompt_for_new_path(&directory, Some(name));
    cx.spawn(async move |_, _| choice.await?)
}
