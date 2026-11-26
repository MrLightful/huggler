mod inline;

use super::*;
use anyhow::{Context as _, ensure};
use huggler::{Attachment, MessageId};
pub(super) use inline::Direction;
use inline::{Insertion, reference, references, replace_range, selection_range};
use std::{
    io::{Read, Write},
    path::Path,
};

const PREVIEW_BYTES: usize = 64 * 1024;
const MAX_SENT_LABEL_WIDTH: Pixels = px(270.);
const CHIP_SIDE_PADDING: f32 = 0.375;

pub(super) const MESSAGE_LINE_HEIGHT: Pixels = px(22.);

fn split_caption_before_last_line<'a>(
    caption: &'a str,
    width: Pixels,
    window: &Window,
    cx: &App,
) -> (Option<&'a str>, &'a str) {
    // Only the last visible line shares space with the attachment chips.
    let (prefix, line) = caption
        .rsplit_once('\n')
        .map_or((None, caption), |(prefix, line)| (Some(prefix), line));
    if line.is_empty() {
        return (prefix, line);
    }
    let line_start = caption.len() - line.len();
    let mut style = window.text_style();
    style.font_family = cx.theme().font_family.clone();
    let split = window
        .text_system()
        .shape_text(
            line.to_owned().into(),
            rems(0.875).to_pixels(window.rem_size()),
            &[style.to_run(line.len())],
            Some(width),
            None,
        )
        .ok()
        .and_then(|lines| {
            let line = lines.first()?;
            let wrap = line.wrap_boundaries().last()?;
            Some(line.runs()[wrap.run_ix].glyphs[wrap.glyph_ix].index)
        })
        .unwrap_or(0);
    let prefix = (split > 0)
        .then(|| &caption[..line_start + split])
        .or(prefix);
    (prefix, &line[split..])
}

fn caption_with_attachments(
    caption: &str,
    _caption_control: impl Fn() -> String,
    width: Pixels,
    chips: impl IntoIterator<Item = Div>,
    window: &Window,
    cx: &App,
) -> Div {
    let (earlier_lines, last_line) = split_caption_before_last_line(caption, width, window, cx);

    v_flex()
        .text_sm()
        .line_height(MESSAGE_LINE_HEIGHT)
        .when_some(earlier_lines, |body, text| {
            let lines = div()
                .min_h(MESSAGE_LINE_HEIGHT)
                .max_w(width)
                .child(text.to_owned());
            body.child(if last_line.is_empty() {
                control!(_caption_control(), lines)
            } else {
                lines
            })
        })
        .child(
            h_flex()
                .max_w(width)
                .flex_wrap()
                .gap_x_1p5()
                .gap_y_1()
                .when(!last_line.is_empty(), |line| {
                    line.child(control!(
                        _caption_control(),
                        div().max_w(width).child(last_line.to_owned())
                    ))
                })
                .children(chips),
        )
}

fn attachment_chip(
    id: impl Into<ElementId>,
    name: &str,
    max_label_width: Pixels,
    cx: &App,
) -> Button {
    Button::new(id)
        .xsmall()
        .outline()
        .rounded(px(5.))
        .px(rems(CHIP_SIDE_PADDING))
        .cursor_pointer()
        .child(crate::file_icons::icon(
            Path::new(name),
            cx.theme().muted_foreground,
        ))
        .child(
            div()
                .text_size(crate::ui::control_text_size())
                .child(AttachmentLabel {
                    text: name.replace(['\r', '\n', '\t'], " ").into(),
                    max_width: max_label_width,
                }),
        )
}

#[derive(IntoElement)]
struct AttachmentLabel {
    text: SharedString,
    max_width: Pixels,
}

impl RenderOnce for AttachmentLabel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let run = style.to_run(self.text.len());
        // GPUI's intrinsic measurement can cache the untruncated label before flex sets its width.
        let text = cx
            .text_system()
            .line_wrapper(style.font(), font_size)
            .truncate_line(self.text, self.max_width, "…", &mut vec![run]);
        div()
            .max_w(self.max_width)
            .overflow_hidden()
            .whitespace_nowrap()
            .child(text)
    }
}

struct DraftFile {
    file: Attachment,
    // Legacy drafts can have files without a reference.
    position: Option<usize>,
}

#[derive(Default)]
pub(super) struct Attachments {
    files: Vec<DraftFile>,
    next_position: usize,
    pub(super) loading: bool,
    error: Option<String>,
}

impl Attachments {
    pub(super) fn from_files(text: &str, files: Vec<Attachment>) -> Self {
        let next_position = files.len();
        Self {
            files: files
                .into_iter()
                .enumerate()
                .map(|(position, file)| DraftFile {
                    position: text
                        .contains(&reference(position, &file.name))
                        .then_some(position),
                    file,
                })
                .collect(),
            next_position,
            ..Self::default()
        }
    }

    pub(super) fn take(&mut self) -> Self {
        // The editor can still contain undo records and clipboard text from an earlier draft.
        std::mem::replace(
            self,
            Self {
                next_position: self.next_position,
                ..Self::default()
            },
        )
    }

    pub(super) fn restore_references(&self, mut text: String) -> String {
        for entry in &self.files {
            if let Some(position) = entry.position {
                let marker = reference(position, &entry.file.name);
                if !text.contains(&marker) {
                    if !text.is_empty() && !text.ends_with(char::is_whitespace) {
                        text.push(' ');
                    }
                    text.push_str(&marker);
                }
            }
        }
        text
    }

    fn references(&self, text: &str) -> Vec<(std::ops::Range<usize>, usize)> {
        references(
            text,
            self.files
                .iter()
                .filter_map(|entry| Some((entry.position?, entry.file.name.as_str()))),
        )
    }

    fn active<'a>(&'a self, text: &'a str) -> impl Iterator<Item = &'a DraftFile> {
        self.files.iter().filter(|file| file.is_active(text))
    }

    pub(super) fn is_empty(&self, text: &str) -> bool {
        self.active(text).next().is_none()
    }

    pub(super) fn title(&self, text: &str) -> String {
        let mut plain = text.to_owned();
        for file in &self.files {
            if let Some(position) = file.position {
                plain = plain.replace(&reference(position, &file.file.name), "");
            }
        }
        launch_workspace::conversation_title(
            &plain,
            self.active(text).next().map(|file| file.file.name.as_str()),
        )
    }

    pub(super) fn content(&self, text: &str) -> (String, Vec<Attachment>) {
        let mut files = Vec::new();
        let mut positions = HashMap::new();
        for entry in self.active(text) {
            if let Some(position) = entry.position {
                positions.insert(position, files.len());
            }
            files.push(entry.file.clone());
        }
        let mut caption = String::new();
        let mut end = 0;
        for (range, position) in self.references(text) {
            let position = positions[&position];
            caption.push_str(&text[end..range.start]);
            caption.push_str(&reference(position, &files[position].name));
            end = range.end;
        }
        caption.push_str(&text[end..]);
        (caption, files)
    }

    pub(super) fn action(&self, text: &str) -> Action {
        let (text, files) = self.content(text);
        if files.is_empty() {
            Action::Enqueue { text }
        } else {
            Action::EnqueueFiles { text, files }
        }
    }
}

impl DraftFile {
    fn is_active(&self, text: &str) -> bool {
        self.position
            .is_none_or(|position| text.contains(&reference(position, &self.file.name)))
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

fn clipboard_attachment(image: Image) -> anyhow::Result<Attachment> {
    let (extension, bytes) = match image.format {
        ImageFormat::Png => ("png", image.bytes),
        ImageFormat::Jpeg => ("jpg", image.bytes),
        ImageFormat::Webp => ("webp", image.bytes),
        ImageFormat::Gif => ("gif", image.bytes),
        ImageFormat::Tiff | ImageFormat::Bmp => {
            let mut reader =
                image::ImageReader::new(std::io::Cursor::new(image.bytes)).with_guessed_format()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(8_192);
            limits.max_image_height = Some(8_192);
            limits.max_alloc = Some(64 * 1024 * 1024);
            reader.limits(limits);
            let mut encoded = std::io::Cursor::new(Vec::new());
            reader
                .decode()?
                .write_to(&mut encoded, image::ImageFormat::Png)?;
            ("png", encoded.into_inner())
        }
        ImageFormat::Svg => ("svg", image.bytes),
    };
    Ok(Attachment::from_bytes(
        format!("Pasted image.{extension}"),
        bytes,
    )?)
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
            image_preview(&file.bytes, true)
                .map_or(PreviewBody::ImageUnavailable, PreviewBody::Image)
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

pub(super) fn image_preview(bytes: &[u8], upscale: bool) -> Option<Arc<RenderImage>> {
    use image::{Frame, ImageReader, Limits};
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(8_192);
    limits.max_image_height = Some(8_192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let image = if upscale || image.width() > 840 || image.height() > 440 {
        image.thumbnail(840, 440)
    } else {
        image
    };
    let mut thumbnail = image.into_rgba8();
    for pixel in thumbnail.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new([Frame::new(thumbnail)])))
}

pub(super) struct SentAttachmentCaption<'a> {
    pub(super) text: &'a str,
    pub(super) author: &'a str,
    pub(super) names: &'a [String],
}

impl Desktop {
    pub(super) fn sent_caption(
        &self,
        thread: &ThreadId,
        message: &MessageId,
        caption: SentAttachmentCaption<'_>,
        width: Pixels,
        window: &Window,
        cx: &Context<Self>,
    ) -> (Div, Option<Div>) {
        let SentAttachmentCaption {
            text: caption,
            author: _author,
            names,
        } = caption;
        let rem = window.rem_size();
        // Account for the icon and gap in rems, plus the two one-pixel borders.
        let chip_extra = rems(2. * CHIP_SIDE_PADDING + 1.).to_pixels(rem) + px(2.);
        let label_width = (width - chip_extra).clamp(px(0.), MAX_SENT_LABEL_WIDTH);
        let chip = |position: usize| {
            self.sent_file_chip(thread, message, position, &names[position], label_width, cx)
        };
        let matches = references(caption, names.iter().map(String::as_str).enumerate());
        let mut preview = self.sent_file_preview(thread, message, names.len(), cx);
        let preview_position = self
            .file_preview
            .as_ref()
            .filter(|((owner, _), _)| owner == message)
            .map(|((_, position), _)| *position);
        if matches.is_empty() {
            return (
                caption_with_attachments(
                    caption,
                    || format!("{_author}/{caption}"),
                    width,
                    (0..names.len()).map(chip),
                    window,
                    cx,
                ),
                preview,
            );
        }

        let make_row = || h_flex().max_w(width).flex_wrap().gap_x_1().gap_y_1();
        let mut body = v_flex().text_sm().line_height(MESSAGE_LINE_HEIGHT);
        let (first, _) = &matches[0];
        let (prefix, first_line) =
            split_caption_before_last_line(&caption[..first.start], width, window, cx);
        if let Some(prefix) = prefix {
            body = body.child(
                div()
                    .max_w(width)
                    .min_h(MESSAGE_LINE_HEIGHT)
                    .child(prefix.to_owned()),
            );
        }
        let mut children = Vec::new();
        let mut rows = Vec::new();
        let add_text = |text: &str, rows: &mut Vec<Div>, children: &mut Vec<Div>| {
            for (index, line) in text.split('\n').enumerate() {
                if index > 0 {
                    rows.push(
                        make_row()
                            .min_h(MESSAGE_LINE_HEIGHT)
                            .children(std::mem::take(children)),
                    );
                }
                if !line.is_empty() {
                    children.push(control!(
                        format!("{_author}/{line}"),
                        div().max_w(width).child(line.to_owned())
                    ));
                }
            }
        };
        add_text(first_line, &mut rows, &mut children);
        let mut referenced = vec![false; names.len()];
        let mut end = first.start;
        for (range, position) in matches {
            add_text(&caption[end..range.start], &mut rows, &mut children);
            children.push(chip(position));
            if preview_position == Some(position)
                && let Some(panel) = preview.take()
            {
                rows.push(make_row().children(std::mem::take(&mut children)));
                rows.push(panel.max_w(width));
            }
            referenced[position] = true;
            end = range.end;
        }
        add_text(&caption[end..], &mut rows, &mut children);
        children.extend(
            (0..names.len())
                .filter(|&position| !referenced[position])
                .map(chip),
        );
        (
            body.children(rows).child(make_row().children(children)),
            preview,
        )
    }

    fn sent_file_chip(
        &self,
        thread: &ThreadId,
        message: &MessageId,
        position: usize,
        name: &str,
        max_label_width: Pixels,
        cx: &Context<Self>,
    ) -> Div {
        let key = (message.clone(), position);
        let thread = thread.clone();
        let expanded = self.file_preview.as_ref().is_some_and(|(id, _)| id == &key);
        control!(
            format!("attachment/{message}/{position}/{name}"),
            attachment_chip(
                SharedString::from(format!("attachment-{message}-{position}")),
                name,
                max_label_width,
                cx,
            )
            .when(expanded, |button| button.selected(true))
            .tooltip(format!(
                "{} {name}",
                if expanded { "Hide" } else { "Preview" }
            ))
            .on_click(cx.listener(move |view, _, window, cx| {
                if view.file_preview.as_ref().is_some_and(|(id, _)| id == &key) {
                    view.file_preview = None;
                    cx.notify();
                } else {
                    view.load_file_preview(thread.clone(), key.clone(), window, cx);
                }
            }))
        )
        .flex_shrink_0()
    }

    fn sent_file_preview(
        &self,
        thread: &ThreadId,
        message: &MessageId,
        attachment_count: usize,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let preview = self
            .file_preview
            .as_ref()
            .filter(|((owner, position), _)| owner == message && *position < attachment_count)?;
        let (key, preview) = preview;
        let thread = thread.clone();
        let key = key.clone();
        let panel = match preview {
            FilePreview::Loading => hint("Loading attachment…", cx),
            FilePreview::Failed(error) => div()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(error.clone()),
            FilePreview::Ready(preview) => {
                let body = match &preview.body {
                    PreviewBody::Text { text, truncated } => {
                        let body = if text.is_empty() {
                            "Empty file".into()
                        } else {
                            text.clone()
                        };
                        v_flex()
                            .gap_1()
                            .child(super::transcript::text_preview(
                                SharedString::from(format!("preview-{message}-{}", key.1)),
                                control!(format!("file preview/{body}"), body),
                                px(176.),
                                cx,
                            ))
                            .when(*truncated, |row| {
                                row.child(hint(
                                    "Showing the first 64 KiB. Save the file for the full text.",
                                    cx,
                                ))
                            })
                    }
                    PreviewBody::Image(image) => div().w_full().max_w(px(420.)).child(control!(
                        format!("image preview/{}", preview.name),
                        img(image.clone())
                            .w_full()
                            .h(px(220.))
                            .object_fit(ObjectFit::Contain)
                    )),
                    PreviewBody::ImageUnavailable => hint("Image preview unavailable", cx),
                    PreviewBody::PdfText(text) => v_flex()
                        .w_full()
                        .gap_1()
                        .child(hint("Extracted PDF text", cx))
                        .child(
                            div()
                                .id("pdf-text")
                                .w_full()
                                .max_h(px(176.))
                                .overflow_y_scroll()
                                .rounded_md()
                                .p_2()
                                .bg(cx.theme().muted)
                                .text_sm()
                                .whitespace_normal()
                                .child(control!(
                                    format!("pdf preview/{}", preview.name),
                                    text.clone()
                                )),
                        ),
                    PreviewBody::File => div(),
                };
                let name = preview.name.clone();
                v_flex().items_start().gap_1().w_full().child(body).child(
                    h_flex()
                        .flex_wrap()
                        .gap_2()
                        .items_center()
                        .child(hint(preview.description(), cx))
                        .child(control!(
                            format!("Save {name}"),
                            Button::new("save-attachment")
                                .small()
                                .ghost()
                                .label("Save as…")
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.save_attachment(
                                        thread.clone(),
                                        key.clone(),
                                        &name,
                                        window,
                                        cx,
                                    );
                                }))
                        )),
                )
            }
        };
        Some(
            div()
                .mt_1()
                .w_full()
                .min_w_0()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .child(panel),
        )
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
        self.load_paths(choice, window, cx);
    }

    pub(super) fn drop_attachments(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_paths(Task::ready(Ok(paths.paths().to_vec())), window, cx);
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
        self.load_paths(choice, window, cx);
    }

    fn load_paths(
        &mut self,
        choice: Task<anyhow::Result<Vec<PathBuf>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_attachments(window, cx, async move |remaining, mut budget, cx| {
            let paths = choice.await?;
            ensure!(
                paths.len() <= remaining,
                "Attach up to eight files per message"
            );
            cx.background_spawn(async move {
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
        });
    }

    pub(super) fn replace_partial_attachment_selection(
        &self,
        input: &Entity<InputState>,
        replacement: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !input.focus_handle(cx).is_focused(window) {
            return false;
        }
        let draft = self.composer_attachments();
        if draft.files.is_empty() {
            return false;
        }
        input.update(cx, |input, cx| {
            if input.marked_text_range(window, cx).is_some() {
                return false;
            }
            let Some(selection) =
                selection_range(input, window, cx).filter(|range| !range.is_empty())
            else {
                return false;
            };
            let expanded =
                inline::expand_selection(selection.clone(), &draft.references(&input.value()));
            if expanded == selection {
                return false;
            }
            if replacement.is_none() {
                cx.write_to_clipboard(ClipboardItem::new_string(
                    input.value()[expanded.clone()].to_owned(),
                ));
            }
            replace_range(input, expanded, replacement.unwrap_or_default(), window, cx);
            true
        })
    }

    fn composer_attachments(&self) -> &Attachments {
        match &self.current {
            Some(thread) => &self.workspaces[&thread.id].attachments,
            None => &self.starter_attachments,
        }
    }

    pub(super) fn paste_attachments(
        &mut self,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(clipboard) = cx.read_from_clipboard() else {
            return false;
        };
        if let Some(text) = clipboard.text()
            && self.replace_partial_attachment_selection(input, Some(&text), window, cx)
        {
            return true;
        }
        let images: Vec<_> = clipboard
            .into_entries()
            .filter_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(image),
                _ => None,
            })
            .collect();
        if images.is_empty() {
            return false;
        }
        self.load_attachments(window, cx, async move |remaining, budget, cx| {
            cx.background_spawn(async move {
                ensure!(
                    images.len() <= remaining,
                    "Attach up to eight files per message"
                );
                ensure!(
                    images.iter().map(|image| image.bytes.len()).sum::<usize>() <= budget,
                    "Attachments can total up to 50 MiB"
                );
                images.into_iter().map(clipboard_attachment).collect()
            })
            .await
        });
        true
    }

    fn load_attachments(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        load: impl AsyncFnOnce(usize, usize, &mut AsyncWindowContext) -> anyhow::Result<Vec<Attachment>>
        + 'static,
    ) {
        if self.launching.is_some() {
            return;
        }
        let thread = self.current.as_ref().map(|t| t.id.clone());
        let input = match thread.as_ref() {
            Some(id) => self.workspaces[id].draft.clone(),
            None => self.starter.clone(),
        };
        let text = input.read(cx).value();
        let draft = self.attachments_mut(thread.as_ref());
        if draft.loading {
            return;
        }
        let remaining = Attachment::MAX_FILES.saturating_sub(draft.active(&text).count());
        if remaining == 0 {
            draft.error = Some("Attach up to eight files per message".into());
            cx.notify();
            return;
        }
        let used = draft
            .active(&text)
            .map(|file| file.file.bytes.len())
            .sum::<usize>();
        draft.loading = true;
        draft.error = None;
        let focus = window.focused(cx);
        let insertion = Insertion::capture(&input, window, cx);
        cx.notify();

        cx.spawn_in(window, async move |view, cx| {
            let result = load(remaining, Attachment::MAX_TOTAL_BYTES - used, cx).await;
            let _ = view.update_in(cx, |view, window, cx| {
                if thread
                    .as_ref()
                    .is_some_and(|id| !view.workspaces.contains_key(id))
                {
                    return;
                }
                let active = view.composer_is_active(&input)
                    && (window.focused(cx) == focus || input.focus_handle(cx).is_focused(window));
                let text = input.read(cx).value();
                let draft = view.attachments_mut(thread.as_ref());
                draft.loading = false;
                match result {
                    Ok(files) if !files.is_empty() => {
                        let mut batch: Vec<_> = draft
                            .active(&text)
                            .map(|entry| entry.file.clone())
                            .collect();
                        let previous = batch.len();
                        batch.extend(files);
                        match Attachment::validate_batch(&batch) {
                            Ok(()) => {
                                // Keep a bounded cache of detached files for editor undo.
                                let mut undo_bytes = 0;
                                let mut undo_count = 0;
                                draft.files.retain(|entry| {
                                    if entry.is_active(&text) {
                                        return true;
                                    }
                                    undo_bytes += entry.file.bytes.len();
                                    undo_count += 1;
                                    undo_bytes <= Attachment::MAX_TOTAL_BYTES
                                        && undo_count <= Attachment::MAX_FILES
                                });
                                let references = draft.references(&text);
                                let mut markers = Vec::new();
                                for file in batch.into_iter().skip(previous) {
                                    let position = draft.next_position;
                                    draft.next_position += 1;
                                    markers.push(reference(position, &file.name));
                                    draft.files.push(DraftFile {
                                        file,
                                        position: Some(position),
                                    });
                                }
                                input.update(cx, |input, cx| {
                                    insertion.insert(
                                        input,
                                        &markers.join(" "),
                                        &references,
                                        active,
                                        window,
                                        cx,
                                    );
                                });
                            }
                            Err(error) => draft.error = Some(error.to_string()),
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

    pub(super) fn snap_inline_cursor(
        &self,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.composer_is_active(input) || !input.focus_handle(cx).is_focused(window) {
            return;
        }
        let draft = self.composer_attachments();
        if draft.files.is_empty() {
            return;
        }
        let cursor = input.read(cx).cursor();
        let markers = draft.references(&input.read(cx).value());
        let next = inline::expand_selection(cursor..cursor, &markers).start;
        if next == cursor {
            return;
        }
        input.update(cx, |input, cx| {
            use gpui_component::input::RopeExt;
            if input.marked_text_range(window, cx).is_none()
                && selection_range(input, window, cx).is_some_and(|range| range.is_empty())
            {
                input.set_cursor_position(input.text().offset_to_position(next), window, cx);
            }
        });
    }

    pub(super) fn edit_inline_attachment(
        &self,
        input: &Entity<InputState>,
        direction: Direction,
        delete: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !input.focus_handle(cx).is_focused(window) {
            return false;
        }
        let draft = self.composer_attachments();
        let matches = draft.references(&input.read(cx).value());
        if matches.is_empty() {
            return false;
        }
        input.update(cx, |input, cx| {
            use gpui_component::input::RopeExt;
            if input.marked_text_range(window, cx).is_some() {
                return false;
            }
            let Some(selection) = selection_range(input, window, cx) else {
                return false;
            };
            let range = if selection.is_empty() {
                let Some((marker, _)) = matches
                    .iter()
                    .find(|(marker, _)| direction.touches(marker, selection.start))
                else {
                    return false;
                };
                marker.clone()
            } else if delete {
                let range = inline::expand_selection(selection.clone(), &matches);
                if range == selection {
                    return false;
                }
                range
            } else {
                return false;
            };
            if delete {
                replace_range(input, range, "", window, cx);
            } else {
                let cursor = match direction {
                    Direction::Backward => range.start,
                    Direction::Forward => range.end,
                };
                input.set_cursor_position(input.text().offset_to_position(cursor), window, cx);
            }
            true
        })
    }

    pub(super) fn attach_button(&self, draft: &Attachments, cx: &Context<Self>) -> Div {
        let text = if self.current.is_some() {
            &self.prompt
        } else {
            &self.starter
        }
        .read(cx)
        .value();
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
                        || draft.active(&text).count() >= Attachment::MAX_FILES
                )
                .on_click(cx.listener(|view, _, window, cx| view.pick_attachments(window, cx)))
        )
    }

    pub(super) fn composer_attachment_pills(
        &self,
        input: &Entity<InputState>,
        window: &Window,
        cx: &App,
    ) -> Vec<gpui_component::input::InputInlineElement> {
        let mut style = window.text_style();
        style.font_family = cx.theme().font_family.clone();
        let font_size = crate::ui::control_text_size().to_pixels(window.rem_size());
        self.composer_attachments()
            .files
            .iter()
            .filter_map(|entry| {
                let position = entry.position?;
                let name = &entry.file.name;
                let marker = reference(position, name);
                let label: SharedString = name.replace(['\r', '\n', '\t'], " ").into();
                let name_width = window
                    .text_system()
                    .shape_line(label.clone(), font_size, &[style.to_run(label.len())], None)
                    .width
                    .ceil();
                let chrome =
                    rems(2. * CHIP_SIDE_PADDING + 2.).to_pixels(window.rem_size()) + px(2.);
                let width = name_width.min(px(168.)) + chrome + px(4.);
                let input = input.downgrade();
                let name = name.clone();
                Some(gpui_component::input::InputInlineElement::new(
                    marker.clone(),
                    width,
                    move |range, width, height, selected| {
                        DraftAttachmentPill {
                            input: input.clone(),
                            marker: marker.clone(),
                            range,
                            selected,
                            name: name.clone(),
                            width: (width - px(4.)).max(px(0.)),
                            height,
                            label_width: (width - chrome - px(4.)).max(px(0.)),
                        }
                        .into_any_element()
                    },
                ))
            })
            .collect()
    }

    pub(super) fn attachment_list(&self, draft: &Attachments, cx: &Context<Self>) -> Option<Div> {
        let setup_text =
            (self.current.is_none() && self.launch_setup).then(|| self.starter.read(cx).value());
        let show = |file: &DraftFile| {
            file.position.is_none() || setup_text.as_ref().is_some_and(|text| file.is_active(text))
        };
        if draft.error.is_none() && !draft.loading && !draft.files.iter().any(show) {
            return None;
        }
        Some(
            h_flex()
                .gap_1()
                .flex_wrap()
                .children(
                    draft
                        .files
                        .iter()
                        .enumerate()
                        .filter(|(_, file)| show(file))
                        .map(|(index, entry)| {
                            let file = &entry.file;
                            let title = format!("Remove {}", file.name);
                            control!(
                                title.clone(),
                                attachment_chip(
                                    ("remove-attachment", index),
                                    &file.name,
                                    px(168.),
                                    cx
                                )
                                .child(
                                    Icon::new(IconName::Close)
                                        .with_size(px(11.))
                                        .flex_shrink_0()
                                )
                                .tooltip(title)
                                .on_click(cx.listener(
                                    move |view, _, window, cx| {
                                        let thread = view.current.as_ref().map(|t| t.id.clone());
                                        let draft = view.attachments_mut(thread.as_ref());
                                        let removed = draft.files.remove(index);
                                        draft.error = None;
                                        if let Some(position) = removed.position {
                                            view.starter.update(cx, |input, cx| {
                                                let text = input.value().replace(
                                                    &reference(position, &removed.file.name),
                                                    "",
                                                );
                                                input.set_value(text, window, cx);
                                            });
                                        }
                                        cx.notify();
                                    }
                                ))
                            )
                        }),
                )
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

#[derive(IntoElement)]
struct DraftAttachmentPill {
    input: WeakEntity<InputState>,
    marker: String,
    range: std::ops::Range<usize>,
    selected: bool,
    name: String,
    width: Pixels,
    height: Pixels,
    label_width: Pixels,
}

impl RenderOnce for DraftAttachmentPill {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let title = format!("Remove {}", self.name);
        div().pl(px(2.)).pt(px(1.)).child(control!(
            title.clone(),
            attachment_chip(
                ("inline-attachment", self.range.start),
                &self.name,
                self.label_width,
                cx
            )
            .w(self.width)
            .h((self.height - px(2.)).max(px(0.)))
            .bg(if self.selected {
                cx.theme().selection
            } else {
                cx.theme().background
            })
            .overflow_hidden()
            .child(
                Icon::new(IconName::Close)
                    .with_size(px(11.))
                    .flex_shrink_0()
            )
            .tab_stop(false)
            .tooltip(title)
            .on_click(move |_, window, cx| {
                let _ = self.input.update(cx, |input, cx| {
                    if input.value().get(self.range.clone()) == Some(self.marker.as_str()) {
                        replace_range(input, self.range.clone(), "", window, cx);
                    }
                    input.focus_handle(cx).focus(window);
                });
            })
        ))
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
