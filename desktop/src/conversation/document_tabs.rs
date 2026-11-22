use super::*;
use crate::{changes::ReviewRow, file_icons};
use anyhow::{Result, ensure};
use gpui_component::{
    scroll::{ScrollableElement, ScrollbarAxis},
    tooltip::Tooltip,
};
use std::{cell::Cell, io::Read, path::Path, rc::Rc};

pub(crate) const DOCUMENT_CONTEXT: &str = "HugglerDocument";
const TEXT_LIMIT: usize = 128 * 1024;
const IMAGE_LIMIT: usize = 8 * 1024 * 1024;
const TAB_LABEL_WIDTH: Pixels = px(144.);

#[derive(Clone)]
pub(super) enum Document {
    File(Arc<Path>),
    Diff(ReviewRow),
}

impl Document {
    fn path(&self) -> &Path {
        match self {
            Self::File(path) => path,
            Self::Diff(row) => &row.file.path,
        }
    }

    fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::File(a), Self::File(b)) => a == b,
            (Self::Diff(a), Self::Diff(b)) => a.file.path == b.file.path && a.scope == b.scope,
            _ => false,
        }
    }

    fn comparison(&self) -> Option<&'static str> {
        match self {
            Self::File(_) => None,
            Self::Diff(row) => Some(match row.scope {
                git::ChangeScope::All => "All changes",
                git::ChangeScope::Uncommitted => "Uncommitted files",
            }),
        }
    }

    fn read(&self, root: &Path) -> Result<Preview> {
        match self {
            Self::File(path) => read_file(root, path),
            Self::Diff(row) => {
                let bytes = git::file_patch(root, &row.base, &row.head, &row.file, TEXT_LIMIT + 1)?;
                if bytes.is_empty() {
                    return Ok(Preview::Notice(
                        "This file has no changes in this comparison.",
                    ));
                }
                let mut preview = Preview::text(bytes, true);
                if let Preview::Text { text, .. } = &mut preview {
                    // The tab already identifies the file. Keep the hunks and any mode or binary changes.
                    let mut in_header = false;
                    *text = text
                        .split_inclusive('\n')
                        .filter(|line| {
                            if line.starts_with("diff --git ") {
                                in_header = true;
                                return false;
                            }
                            if line.starts_with('@') {
                                in_header = false;
                            }
                            !in_header
                                || !["index ", "--- ", "+++ "]
                                    .iter()
                                    .any(|prefix| line.starts_with(prefix))
                        })
                        .collect::<String>()
                        .into();
                }
                Ok(preview)
            }
        }
    }
}

#[derive(Default)]
pub(super) struct DocumentTabs {
    tabs: Vec<DocumentTab>,
    active: Option<usize>,
    next_id: usize,
    scroll: ScrollHandle,
    reveal: Rc<Cell<Option<usize>>>,
}

impl DocumentTabs {
    pub(super) fn show_chat(&mut self) {
        self.active = None;
    }

    pub(super) fn active(&self) -> Option<&DocumentTab> {
        self.active.and_then(|index| self.tabs.get(index))
    }

    fn index_of(&self, id: usize) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    fn select(&mut self, index: usize, window: &mut Window) {
        self.active = Some(index);
        self.reveal.set(Some(index));
        self.tabs[index].focus.focus(window);
    }
}

pub(super) struct DocumentTab {
    id: usize,
    document: Document,
    focus: FocusHandle,
    scroll: ScrollHandle,
    preview: Option<Result<Preview>>,
    load: Option<Task<()>>,
}

enum Preview {
    Text { text: SharedString, truncated: bool },
    Image(Arc<RenderImage>),
    Notice(&'static str),
}

impl Preview {
    fn text(bytes: Vec<u8>, diff: bool) -> Self {
        let truncated = bytes.len() > TEXT_LIMIT;
        let bytes = &bytes[..bytes.len().min(TEXT_LIMIT)];
        let text = match std::str::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) if truncated && error.error_len().is_none() => {
                std::str::from_utf8(&bytes[..error.valid_up_to()]).expect("valid UTF-8 prefix")
            }
            Err(_) if diff => {
                return Self::Text {
                    text: String::from_utf8_lossy(bytes).into_owned().into(),
                    truncated,
                };
            }
            Err(_) => return Self::Notice("A text preview is unavailable for this file."),
        };
        if !diff && text.contains('\0') {
            return Self::Notice("A text preview is unavailable for this file.");
        }
        Self::Text {
            text: text.to_owned().into(),
            truncated,
        }
    }
}

fn read_file(root: &Path, path: &Path) -> Result<Preview> {
    let target = root.canonicalize()?.join(path);
    let parent = target.parent().expect("workspace file has a parent");
    ensure!(
        parent.canonicalize()? == parent,
        "The parent folder has moved."
    );
    let kind = target.symlink_metadata()?.file_type();
    if kind.is_symlink() {
        return Ok(Preview::Text {
            text: format!("Symbolic link → {}", fs::read_link(target)?.display()).into(),
            truncated: false,
        });
    }
    ensure!(
        kind.is_file(),
        "The selected path no longer points to a regular file."
    );
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(target)?;
    ensure!(
        file.metadata()?.is_file(),
        "The selected path no longer points to a regular file."
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take((TEXT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if matches!(
        image::guess_format(&bytes),
        Ok(image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::WebP
            | image::ImageFormat::Gif)
    ) {
        file.take((IMAGE_LIMIT + 1 - bytes.len()) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > IMAGE_LIMIT {
            return Ok(Preview::Notice("This image is too large to preview."));
        }
        return Ok(attachments::image_preview(&bytes).map_or(
            Preview::Notice("Could not preview this image."),
            Preview::Image,
        ));
    }
    if bytes.is_empty() {
        return Ok(Preview::Notice("This file is empty."));
    }
    Ok(Preview::text(bytes, false))
}

impl Desktop {
    pub(super) fn return_to_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(thread) = &self.current {
            self.workspaces
                .get_mut(&thread.id)
                .expect("selected workspace")
                .documents
                .show_chat();
        }
        self.focus_workspace(window, cx);
        cx.notify();
    }

    pub(super) fn open_document(
        &mut self,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.current.as_ref().map(|thread| thread.id.clone()) else {
            return;
        };
        self.suggestions = Default::default();
        self.model_picker = None;
        if window.viewport_size().width < NARROW_VIEWPORT_WIDTH && self.right_sidebar.is_open(false)
        {
            self.right_sidebar.toggle(true);
        }
        let workspace = self
            .workspaces
            .get_mut(&thread)
            .expect("selected workspace");
        let tabs = &mut workspace.documents;
        let index = tabs
            .tabs
            .iter()
            .position(|tab| tab.document.matches(&document))
            .unwrap_or_else(|| {
                let index = tabs.tabs.len();
                tabs.tabs.push(DocumentTab {
                    id: tabs.next_id,
                    document: document.clone(),
                    focus: cx.focus_handle().tab_stop(true),
                    scroll: ScrollHandle::new(),
                    preview: None,
                    load: None,
                });
                tabs.next_id += 1;
                index
            });
        tabs.select(index, window);
        let tab = &mut tabs.tabs[index];
        tab.document = document.clone();
        let id = tab.id;
        let root = workspace.root.clone();
        let job = cx.background_spawn(async move { document.read(&root) });
        tab.load = Some(cx.spawn(async move |view, cx| {
            let result = job.await;
            let _ = view.update(cx, |view, cx| {
                if let Some(tab) = view.workspaces.get_mut(&thread).and_then(|workspace| {
                    workspace.documents.tabs.iter_mut().find(|tab| tab.id == id)
                }) {
                    tab.preview = Some(result);
                    tab.load = None;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn close_document(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = &self.current else {
            return;
        };
        let tabs = &mut self
            .workspaces
            .get_mut(&thread.id)
            .expect("selected workspace")
            .documents;
        let Some(index) = tabs.index_of(id) else {
            return;
        };
        tabs.tabs.remove(index);
        match tabs.active {
            Some(active) if active == index && index > 0 => tabs.select(index - 1, window),
            Some(active) if active == index => self.return_to_chat(window, cx),
            Some(active) => tabs.select(active - usize::from(active > index), window),
            None => self.focus_workspace(window, cx),
        }
        cx.notify();
    }

    pub(super) fn close_active_document(
        &mut self,
        _: &crate::CloseDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self
            .current
            .as_ref()
            .and_then(|thread| self.workspaces[&thread.id].documents.active())
        {
            self.close_document(tab.id, window, cx);
        }
    }

    pub(super) fn document_tab_strip(&self, window: &Window, cx: &Context<Self>) -> Div {
        let documents = self
            .current
            .as_ref()
            .map(|thread| &self.workspaces[&thread.id].documents);
        let chat_active = documents.and_then(DocumentTabs::active).is_none();
        let theme = cx.theme();
        h_flex()
            .h_9()
            .flex_shrink_0()
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(
                control!(
                    "Chat tab",
                    Button::new("chat-tab")
                        .ghost()
                        .small()
                        .h_7()
                        .px_2p5()
                        .label("Chat")
                        .when(chat_active, |button| button
                            .bg(theme.sidebar_accent)
                            .selected(true))
                        .on_click(
                            cx.listener(|view, _, window, cx| view.return_to_chat(window, cx))
                        )
                )
                .flex_none(),
            )
            .when_some(
                documents.filter(|documents| !documents.tabs.is_empty()),
                |strip, documents| {
                    strip
                        .child(div().w_px().h_4().flex_none().bg(theme.border))
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .on_children_prepainted({
                                    let reveal = documents.reveal.clone();
                                    let scroll = documents.scroll.clone();
                                    move |_, window, _| reveal_tab(&reveal, &scroll, window)
                                })
                                .child(
                                    h_flex()
                                        .id("document-tabs")
                                        .h_9()
                                        .gap_1()
                                        .overflow_x_scroll()
                                        .track_scroll(&documents.scroll)
                                        .children(documents.tabs.iter().enumerate().map(
                                            |(index, tab)| {
                                                tab.selector(
                                                    documents.active == Some(index),
                                                    window,
                                                    cx,
                                                )
                                            },
                                        )),
                                )
                                .horizontal_scrollbar(&documents.scroll),
                        )
                },
            )
    }
}

impl DocumentTab {
    fn selector(&self, active: bool, window: &Window, cx: &Context<Desktop>) -> impl IntoElement {
        let id = self.id;
        let path = self.document.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .replace(['\n', '\r', '\t'], " ");
        let full_path = path.display().to_string();
        let diff = self.document.comparison();
        let _key = diff.map_or_else(
            || format!("File tab/{full_path}"),
            |scope| format!("Diff tab/{scope}/{full_path}"),
        );
        let theme = cx.theme();
        let fill = theme.sidebar_accent;
        let ring = theme.ring;
        let muted = theme.muted_foreground;
        let label_color = if active { theme.foreground } else { muted };
        let label_style = TextStyle {
            font_family: theme.font_family.clone(),
            color: label_color,
            ..Default::default()
        };
        let font_size = rems(0.875).to_pixels(window.rem_size());
        let mut runs = vec![label_style.to_run(name.len())];
        let label_width = window
            .text_system()
            .layout_line(&name, font_size, &runs, None)
            .width
            .ceil();
        let truncated = label_width > TAB_LABEL_WIDTH;
        let tooltip = match diff {
            Some(scope) => Some(format!("{full_path} · {scope}")),
            None if full_path != name || truncated => Some(full_path),
            None => None,
        };
        let name = if truncated {
            cx.text_system()
                .line_wrapper(label_style.font(), font_size)
                .truncate_line(name.into(), TAB_LABEL_WIDTH, "…", &mut runs)
        } else {
            name.into()
        };
        h_flex()
            .id(("document-tab", id))
            .h_7()
            .flex_none()
            .rounded_md()
            .hover(move |style| style.bg(fill))
            .when(active, |tab| tab.bg(fill))
            .child(
                control!(
                    _key.clone(),
                    h_flex()
                        .id("select")
                        .track_focus(&self.focus)
                        .h_7()
                        .gap_1p5()
                        .pl_2()
                        .pr_0p5()
                        .rounded_md()
                        .border_1()
                        .border_color(transparent_black())
                        .focus(move |style| style.border_color(ring))
                        .cursor_pointer()
                        .when_some(tooltip, |tab, tooltip| tab.tooltip(move |window, cx| {
                            Tooltip::new(tooltip.clone()).build(window, cx)
                        }))
                        .child(file_icons::icon(path, muted))
                        .child(
                            div()
                                .w(label_width.min(TAB_LABEL_WIDTH))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_sm()
                                .text_color(label_color)
                                .child(name)
                        )
                        .when(diff.is_some(), |tab| tab.child(
                            div().flex_none().text_xs().text_color(muted).child("Diff")
                        ))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            if let Some(thread) = &view.current {
                                let tabs = &mut view
                                    .workspaces
                                    .get_mut(&thread.id)
                                    .expect("selected workspace")
                                    .documents;
                                if let Some(index) = tabs.index_of(id) {
                                    tabs.select(index, window);
                                    cx.notify();
                                }
                            }
                        }))
                )
                .flex_none(),
            )
            .child(
                control!(
                    format!("Close {_key}"),
                    Button::new("close")
                        .ghost()
                        .size(px(24.))
                        .p_0()
                        .child(
                            Icon::new(IconName::Close)
                                .with_size(px(12.))
                                .text_color(muted)
                        )
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.close_document(id, window, cx)
                        }))
                )
                .flex_none(),
            )
    }

    pub(super) fn render(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let path = self.document.path().display().to_string();
        let diff = self.document.comparison();
        let preview = match &self.preview {
            None => hint(
                if diff.is_some() {
                    "Reading diff…"
                } else {
                    "Reading file…"
                },
                cx,
            )
            .px_4()
            .py_3(),
            Some(Err(error)) => hint(format!("Could not open {path}. {error:#}"), cx)
                .px_4()
                .py_3(),
            Some(Ok(Preview::Notice(message))) => hint(*message, cx).px_4().py_3(),
            Some(Ok(Preview::Image(image))) => div().p_4().child(img(image.clone()).max_w_full()),
            Some(Ok(Preview::Text { text, truncated })) => {
                let code = StyledText::new(text.clone());
                let code = if diff.is_some() {
                    code.with_highlights(diff_highlights(text, cx))
                } else {
                    code
                };
                div()
                    .min_w_full()
                    .flex_none()
                    .px_4()
                    .py_3()
                    .whitespace_nowrap()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .line_height(px(18.))
                    .child(control!(
                        format!("Document text/{}", &text[..text.floor_char_boundary(1024)]),
                        code
                    ))
                    .when(*truncated, |body| {
                        body.child(hint("Preview truncated", cx).mt_2())
                    })
            }
        };
        v_flex()
            .id(("document", self.id))
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                h_flex()
                    .h_8()
                    .flex_none()
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        control!(
                            format!("Document/{path}"),
                            div()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(theme.foreground)
                                .child(path.clone())
                        )
                        .min_w_0()
                        .flex_1(),
                    )
                    .when_some(diff, |header, scope| {
                        header.child(hint(scope, cx).text_xs().flex_none())
                    }),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(
                        div()
                            .id("document-content")
                            .size_full()
                            .overflow_scroll()
                            .track_scroll(&self.scroll)
                            .child(preview),
                    )
                    .scrollbar(&self.scroll, ScrollbarAxis::Both),
            )
    }
}

fn reveal_tab(reveal: &Cell<Option<usize>>, scroll: &ScrollHandle, window: &mut Window) {
    let Some(tab) = reveal
        .take()
        .and_then(|index| scroll.bounds_for_item(index))
    else {
        return;
    };
    let viewport = scroll.bounds();
    let offset = scroll.offset();
    let right = viewport.right() - tab.right();
    let left = (viewport.left() - tab.left()).min(right);
    let x = offset.x.clamp(left, right);
    if x != offset.x {
        scroll.set_offset(point(x, offset.y));
        window.refresh();
    }
}

fn diff_highlights(text: &str, cx: &App) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let (added, removed, _) = crate::changes::change_colors(cx);
    let muted = cx.theme().muted_foreground;
    let mut offset = 0;
    let mut in_hunk = false;
    let mut highlights = Vec::new();
    for line in text.split_inclusive('\n') {
        let color = match line.as_bytes().first() {
            Some(b'@') => {
                in_hunk = true;
                Some(muted)
            }
            Some(b'+') if in_hunk => Some(added),
            Some(b'-') if in_hunk => Some(removed),
            Some(b' ') if in_hunk => None,
            _ => Some(muted),
        };
        let end = offset + line.trim_end_matches('\n').len();
        if let Some(color) = color.filter(|_| end > offset) {
            highlights.push((
                offset..end,
                HighlightStyle {
                    color: Some(color),
                    ..Default::default()
                },
            ));
        }
        offset += line.len();
    }
    highlights
}
