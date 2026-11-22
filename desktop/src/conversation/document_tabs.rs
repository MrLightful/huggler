mod diff;
mod editor;

use super::{markdown_view::Markdown, *};
use crate::{
    changes::{ReviewRow, line_counts},
    file_icons,
    git::PreparedRevert,
};
use anyhow::{Result, ensure};
use diff::Diff;
use editor::{Editor, confirm_discard};
use gpui_component::{
    WindowExt,
    button::ButtonCustomVariant,
    scroll::{ScrollableElement, ScrollbarAxis},
    tooltip::Tooltip,
};
use std::{
    cell::Cell,
    io::Read,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

pub(crate) const DOCUMENT_CONTEXT: &str = "HugglerDocument";
const TEXT_LIMIT: usize = 128 * 1024;
const IMAGE_LIMIT: usize = 8 * 1024 * 1024;
const TAB_LABEL_WIDTH: Pixels = px(144.);
const COPY_FEEDBACK: Duration = Duration::from_secs(2);

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

    fn read(&self, root: &Path, refresh: bool) -> Result<Loaded> {
        let Self::Diff(row) = self else {
            return Ok(Loaded::file(read_file(root, self.path())?));
        };
        let row = if refresh {
            let snapshot = git::changed_files(root, row.scope)?;
            snapshot
                .files
                .into_iter()
                .find(|file| file.path == row.file.path)
                .map(|file| ReviewRow {
                    file,
                    base: snapshot.base,
                    head: snapshot.head,
                    scope: row.scope,
                })
        } else {
            Some(row.clone())
        };
        let Some(row) = row else {
            return Ok(Loaded::file(Preview::Notice(NO_DIFF)));
        };
        let revert = row
            .file
            .can_revert()
            .then(|| PreparedRevert::prepare(root, row.head.clone(), row.file.clone()).ok())
            .flatten()
            .map(|target| Arc::from([target]));
        let bytes = git::file_patch(root, &row.base, &row.head, &row.file, TEXT_LIMIT + 1)?;
        let preview = if bytes.is_empty() {
            Preview::Notice(NO_DIFF)
        } else {
            Preview::text(bytes, Some(&row.file.path), false)
        };
        Ok(Loaded {
            preview,
            review: Some(row),
            revert,
        })
    }
}

#[derive(Default)]
pub(super) struct DocumentTabs {
    tabs: Vec<DocumentTab>,
    editors: HashMap<Arc<Path>, Editor>,
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

    pub(super) fn active_editor_focus(&self, cx: &App) -> Option<FocusHandle> {
        let tab = self.active().filter(|tab| tab.mode == DocumentMode::Raw)?;
        let draft = self.editors.get(tab.document.path())?.draft()?;
        (!draft.is_saving()).then(|| draft.input.focus_handle(cx))
    }

    fn index_of(&self, id: usize) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    fn select(&mut self, index: usize) {
        self.active = Some(index);
        self.reveal.set(Some(index));
    }
}

const NO_DIFF: &str = "This file has no changes in this comparison.";

pub(super) struct DocumentTab {
    id: usize,
    document: Document,
    focus: FocusHandle,
    scroll: ScrollHandle,
    diff_scroll: UniformListScrollHandle,
    mode: DocumentMode,
    copied_at: Option<Instant>,
    loaded: Option<Result<Loaded>>,
    revert_error: Option<String>,
    load: Option<Task<()>>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum DocumentMode {
    #[default]
    Preview,
    Patch,
    Raw,
}

struct Loaded {
    preview: Preview,
    review: Option<ReviewRow>,
    revert: Option<Arc<[PreparedRevert]>>,
}

impl Loaded {
    fn file(preview: Preview) -> Self {
        Self {
            preview,
            review: None,
            revert: None,
        }
    }
}

enum Formatted {
    Markdown(Box<Markdown>),
    Diff(Arc<Diff>),
}

enum Preview {
    Text {
        text: SharedString,
        truncated: bool,
        formatted: Option<Formatted>,
    },
    Image {
        thumbnail: Arc<RenderImage>,
        source: Arc<Image>,
    },
    Notice(&'static str),
}

impl Preview {
    fn text(bytes: Vec<u8>, diff: Option<&Path>, markdown: bool) -> Self {
        let truncated = bytes.len() > TEXT_LIMIT;
        let bytes = &bytes[..bytes.len().min(TEXT_LIMIT)];
        let text = match std::str::from_utf8(bytes) {
            Ok(text) => std::borrow::Cow::Borrowed(text),
            Err(error) if truncated && error.error_len().is_none() => {
                std::str::from_utf8(&bytes[..error.valid_up_to()])
                    .expect("valid UTF-8 prefix")
                    .into()
            }
            Err(_) if diff.is_some() => String::from_utf8_lossy(bytes),
            Err(_) => return Self::Notice("A text preview is unavailable for this file."),
        };
        if diff.is_none() && text.contains('\0') {
            return Self::Notice("A text preview is unavailable for this file.");
        }
        let formatted = if let Some(path) = diff {
            Some(Formatted::Diff(Arc::new(Diff::new(&text, path))))
        } else if markdown {
            Some(Formatted::Markdown(Box::new(Markdown::new(&text))))
        } else {
            None
        };
        Self::Text {
            text: text.into_owned().into(),
            truncated,
            formatted,
        }
    }

    fn copy_content(&self, diff: bool) -> Option<(&'static str, CopyContent)> {
        match self {
            Self::Text {
                text, truncated, ..
            } => {
                let tooltip = if *truncated {
                    "Copy excerpt"
                } else if diff {
                    "Copy diff"
                } else {
                    "Copy file"
                };
                Some((tooltip, CopyContent::Text(text.clone())))
            }
            Self::Image { source, .. } => Some(("Copy image", CopyContent::Image(source.clone()))),
            Self::Notice(_) => None,
        }
    }
}

enum CopyContent {
    Text(SharedString),
    Image(Arc<Image>),
    Draft(Entity<InputState>),
}

impl CopyContent {
    fn clipboard_item(&self, cx: &App) -> ClipboardItem {
        match self {
            Self::Text(text) => ClipboardItem::new_string(text.to_string()),
            Self::Image(image) => ClipboardItem::new_image(image),
            Self::Draft(input) => ClipboardItem::new_string(input.read(cx).value().to_string()),
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
            formatted: None,
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
    let format = match image::guess_format(&bytes) {
        Ok(image::ImageFormat::Png) => Some(ImageFormat::Png),
        Ok(image::ImageFormat::Jpeg) => Some(ImageFormat::Jpeg),
        Ok(image::ImageFormat::WebP) => Some(ImageFormat::Webp),
        Ok(image::ImageFormat::Gif) => Some(ImageFormat::Gif),
        _ => None,
    };
    if let Some(format) = format {
        file.take((IMAGE_LIMIT + 1 - bytes.len()) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > IMAGE_LIMIT {
            return Ok(Preview::Notice("This image is too large to preview."));
        }
        return Ok(attachments::image_preview(&bytes).map_or(
            Preview::Notice("Could not preview this image."),
            |thumbnail| Preview::Image {
                thumbnail,
                source: Arc::new(Image::from_bytes(format, bytes)),
            },
        ));
    }
    if bytes.is_empty() {
        return Ok(Preview::Notice("This file is empty."));
    }
    let markdown = path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            ["md", "markdown", "mdx"]
                .iter()
                .any(|name| ext.eq_ignore_ascii_case(name))
        });
    Ok(Preview::text(bytes, None, markdown))
}

impl Desktop {
    fn active_document_mut(&mut self, id: usize) -> Option<&mut DocumentTab> {
        let thread = self.current.as_ref()?;
        let tabs = &mut self.workspaces.get_mut(&thread.id)?.documents;
        tabs.tabs.get_mut(tabs.active?).filter(|tab| tab.id == id)
    }

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
                    diff_scroll: UniformListScrollHandle::new(),
                    mode: DocumentMode::default(),
                    copied_at: None,
                    loaded: None,
                    revert_error: None,
                    load: None,
                });
                tabs.next_id += 1;
                index
            });
        tabs.select(index);
        let tab = &mut tabs.tabs[index];
        tab.document = document;
        tab.revert_error = None;
        tab.load(&thread, &workspace.root, false, cx);
        let path = tab.document.path();
        let editor_path = (tab.mode == DocumentMode::Raw || tabs.editors.contains_key(path))
            .then(|| Arc::from(path));
        self.focus_workspace(window, cx);
        if let Some(path) = editor_path {
            self.load_raw_editor(&thread, path, true, window, cx);
        }
        cx.notify();
    }

    fn open_active_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.current.as_ref().map(|current| current.id.clone()) else {
            return;
        };
        if let Some(tab) = self.workspaces[&thread].documents.active()
            && tab.mode == DocumentMode::Raw
        {
            self.load_raw_editor(&thread, tab.document.path().into(), false, window, cx);
        }
    }

    pub(super) fn refresh_reverted_documents(
        &mut self,
        thread: &ThreadId,
        result: &RevertFinished,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get_mut(thread) else {
            return;
        };
        if !result.paths.is_empty() {
            workspace.files.update(cx, FileTree::refresh);
        }
        for tab in &mut workspace.documents.tabs {
            let path = tab.document.path();
            let error = result.failure.as_ref().filter(|(failed, _)| failed == path);
            if error.is_none() && !result.paths.iter().any(|changed| changed == path) {
                continue;
            }
            tab.revert_error = error.map(|(_, message)| message.clone());
            tab.load(thread, &workspace.root, true, cx);
        }
        let refresh: Vec<_> = result
            .paths
            .iter()
            .filter(|path| workspace.documents.editors.contains_key(path.as_path()))
            .map(|path| Arc::<Path>::from(path.as_path()))
            .collect();
        for path in refresh {
            self.load_raw_editor(thread, path, true, window, cx);
        }
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
        let path = tabs.tabs[index].document.path();
        let last = !tabs
            .tabs
            .iter()
            .enumerate()
            .any(|(other, tab)| other != index && tab.document.path() == path);
        if last && let Some(draft) = tabs.editors.get(path).and_then(Editor::draft) {
            if draft.is_saving() || window.has_active_dialog(cx) {
                return;
            }
            if draft.dirty {
                let thread = thread.id.clone();
                let view = cx.weak_entity();
                confirm_discard(
                    "Discard unsaved changes?",
                    format!("Changes to {} will be discarded.", path.display()),
                    "Discard changes",
                    move |window, cx| {
                        let _ = view.update(cx, |view, cx| {
                            if view
                                .current
                                .as_ref()
                                .is_some_and(|current| current.id == thread)
                            {
                                view.remove_document(id, window, cx);
                            }
                        });
                    },
                    window,
                    cx,
                );
                return;
            }
        }
        self.remove_document(id, window, cx);
    }

    fn remove_document(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
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
        let tab = tabs.tabs.remove(index);
        if !tabs
            .tabs
            .iter()
            .any(|other| other.document.path() == tab.document.path())
        {
            tabs.editors.remove(tab.document.path());
        }
        match tabs.active {
            Some(active) if active == index && index > 0 => tabs.select(index - 1),
            Some(active) if active == index => return self.return_to_chat(window, cx),
            Some(active) => tabs.select(active - usize::from(active > index)),
            None => {}
        }
        self.focus_workspace(window, cx);
        self.open_active_editor(window, cx);
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
            .relative()
            .h_9()
            .items_end()
            .flex_shrink_0()
            .px_2()
            .gap_0p5()
            .bg(theme.sidebar)
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h_px()
                    .bg(theme.border),
            )
            .child(tab_surface(chat_active, cx).child(control!(
                "Chat tab",
                Button::new("chat-tab")
                    .custom(ButtonCustomVariant::new(cx).foreground(if chat_active {
                        theme.foreground
                    } else {
                        theme.muted_foreground
                    }))
                    .small()
                    .h_full()
                    .px_3()
                    .label("Chat")
                    .on_click(cx.listener(|view, _, window, cx| view.return_to_chat(window, cx)))
            )))
            .when_some(
                documents.filter(|documents| !documents.tabs.is_empty()),
                |strip, documents| {
                    strip.child(
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
                                    .h_8()
                                    .gap_0p5()
                                    .overflow_x_scroll()
                                    .track_scroll(&documents.scroll)
                                    .children(documents.tabs.iter().enumerate().map(
                                        |(index, tab)| {
                                            tab.selector(
                                                documents.active == Some(index),
                                                documents
                                                    .editors
                                                    .get(tab.document.path())
                                                    .and_then(Editor::draft)
                                                    .is_some_and(|draft| draft.dirty),
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

pub(super) fn tab_surface(active: bool, cx: &App) -> Div {
    let theme = cx.theme();
    let hover = theme.muted;
    h_flex()
        .h_8()
        .flex_none()
        .rounded_t_md()
        .border_t_1()
        .border_l_1()
        .border_r_1()
        .border_color(transparent_black())
        .when(active, |tab| {
            tab.bg(theme.background).border_color(theme.border)
        })
        .when(!active, |tab| tab.hover(move |style| style.bg(hover)))
}

impl DocumentTab {
    fn selector(
        &self,
        active: bool,
        dirty: bool,
        window: &Window,
        cx: &Context<Desktop>,
    ) -> impl IntoElement {
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
        let ring = theme.ring;
        let muted = theme.muted_foreground;
        let label_color = if active { theme.foreground } else { muted };
        let label_style = TextStyle {
            font_family: theme.font_family.clone(),
            color: label_color,
            ..Default::default()
        };
        let font_size = crate::ui::control_text_size().to_pixels(window.rem_size());
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
        tab_surface(active, cx)
            .id(("document-tab", id))
            .pr_0p5()
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
                                .text_size(crate::ui::control_text_size())
                                .text_color(label_color)
                                .child(name)
                        )
                        .when(diff.is_some(), |tab| tab.child(
                            div().flex_none().text_xs().text_color(muted).child("Diff")
                        ))
                        .when(dirty, |tab| {
                            tab.child(div().size(px(5.)).flex_none().rounded_full().bg(muted))
                        })
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                        .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                            if let Some(thread) = &view.current {
                                let tabs = &mut view
                                    .workspaces
                                    .get_mut(&thread.id)
                                    .expect("selected workspace")
                                    .documents;
                                if let Some(index) = tabs.index_of(id) {
                                    tabs.select(index);
                                    if !event.is_keyboard() {
                                        view.focus_workspace(window, cx);
                                    }
                                    view.open_active_editor(window, cx);
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

    fn load(&mut self, thread: &ThreadId, root: &Path, refresh: bool, cx: &mut Context<Desktop>) {
        let document = self.document.clone();
        let thread = thread.clone();
        let root = root.to_path_buf();
        let id = self.id;
        let job = cx.background_spawn(async move { document.read(&root, refresh) });
        self.copied_at = None;
        self.load = Some(cx.spawn(async move |view, cx| {
            let result = job.await;
            let _ = view.update(cx, |view, cx| {
                if let Some(tab) = view.workspaces.get_mut(&thread).and_then(|workspace| {
                    workspace.documents.tabs.iter_mut().find(|tab| tab.id == id)
                }) {
                    tab.loaded = Some(result);
                    tab.load = None;
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn render(
        &self,
        changes: &Entity<Changes>,
        documents: &DocumentTabs,
        window: &mut Window,
        cx: &mut Context<Desktop>,
    ) -> impl IntoElement {
        let path = self.document.path();
        let path_label = path.display().to_string();
        let scope = self.document.comparison();
        let theme = cx.theme();
        let (border, muted, foreground) = (theme.border, theme.muted_foreground, theme.foreground);
        let loaded = match &self.loaded {
            Some(Ok(loaded)) => Some(loaded),
            _ => None,
        };
        let preview = loaded.map(|loaded| &loaded.preview);
        let modes: &[(DocumentMode, &str)] = if scope.is_some() {
            &[
                (DocumentMode::Preview, "Unified"),
                (DocumentMode::Patch, "Patch"),
                (DocumentMode::Raw, "Raw"),
            ]
        } else {
            &[
                (DocumentMode::Preview, "Preview"),
                (DocumentMode::Raw, "Raw"),
            ]
        };
        let editor = documents.editors.get(path);
        let draft = editor.and_then(Editor::draft);
        let raw = self.mode == DocumentMode::Raw;
        let copy = if raw {
            draft.map(|draft| ("Copy file", CopyContent::Draft(draft.input.clone())))
        } else {
            preview.and_then(|preview| preview.copy_content(scope.is_some()))
        };
        let revert = loaded.and_then(|loaded| loaded.review.as_ref().zip(loaded.revert.as_ref()));
        let truncated = matches!(
            preview,
            Some(Preview::Text {
                truncated: true,
                ..
            })
        );
        let content = if raw {
            editor
                .map_or_else(
                    || hint("Reading working file…", cx).p_4(),
                    |editor| editor.render(cx),
                )
                .into_any_element()
        } else {
            match &self.loaded {
                None => hint(
                    if scope.is_some() {
                        "Reading diff…"
                    } else {
                        "Reading file…"
                    },
                    cx,
                )
                .p_4()
                .into_any_element(),
                Some(Err(error)) => hint(format!("Could not open {path_label}. {error:#}"), cx)
                    .p_4()
                    .into_any_element(),
                Some(Ok(loaded)) => self.render_preview(&loaded.preview, window, cx),
            }
        };
        v_flex()
            .id(("document", self.id))
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                h_flex()
                    .flex_none()
                    .flex_wrap()
                    .px_3()
                    .py_1p5()
                    .gap_x_3()
                    .gap_y_1()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        h_flex()
                            .min_w(px(120.))
                            .min_h_7()
                            .flex_1()
                            .gap_2()
                            .child(file_icons::icon(path, muted))
                            .child(control!(
                                format!("Document/{path_label}"),
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(crate::ui::control_text_size())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(foreground)
                                    .child(path_label)
                            )),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .max_w_full()
                            .flex_wrap()
                            .gap_1()
                            .child(self.mode_control(modes, cx))
                            .children(
                                copy.map(|(tooltip, content)| {
                                    self.copy_button(tooltip, content, cx)
                                }),
                            )
                            .when_some(
                                draft.filter(|draft| raw || draft.dirty),
                                |actions, draft| {
                                    actions.child(control!(
                                        "Save document",
                                        Button::new("save-document")
                                            .small()
                                            .label(if draft.is_saving() {
                                                "Saving…"
                                            } else {
                                                "Save"
                                            })
                                            .tooltip_with_action(
                                                "Save working file",
                                                &crate::SaveDocument,
                                                Some(DOCUMENT_CONTEXT),
                                            )
                                            .disabled(!draft.dirty || draft.is_saving())
                                            .on_click(cx.listener(|view, _, window, cx| {
                                                view.save_active_document(
                                                    &crate::SaveDocument,
                                                    window,
                                                    cx,
                                                );
                                            }))
                                    ))
                                },
                            )
                            .when_some(revert, |actions, (row, target)| {
                                let changes = changes.clone();
                                let row = row.clone();
                                let target = target.clone();
                                actions.child(control!(
                                    "Revert document",
                                    Button::new("revert-document")
                                        .ghost()
                                        .small()
                                        .icon(IconName::Undo2)
                                        .tooltip("Revert file")
                                        .disabled(
                                            self.load.is_some()
                                                || draft.is_some_and(
                                                    |draft| draft.dirty || draft.is_saving()
                                                )
                                        )
                                        .on_click(move |_, window, cx| {
                                            changes.update(cx, |changes, cx| {
                                                changes.confirm_document_revert(
                                                    row.clone(),
                                                    target.clone(),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        })
                                ))
                            }),
                    ),
            )
            .when_some(scope, |pane, scope| {
                pane.child(
                    h_flex()
                        .px_3()
                        .py_1p5()
                        .gap_3()
                        .border_b_1()
                        .border_color(border)
                        .child(hint(scope, cx).text_xs().flex_1())
                        .when_some(
                            loaded.and_then(|loaded| loaded.review.as_ref()),
                            |summary, row| summary.child(line_counts(row.file.lines, cx)),
                        ),
                )
            })
            .when_some(self.revert_error.as_ref(), |pane, error| {
                pane.child(control!(
                    "Document revert failed",
                    hint(error.clone(), cx).p_3().text_color(cx.theme().danger)
                ))
            })
            .when_some(
                draft.and_then(|draft| draft.error.as_ref()),
                |pane, error| {
                    pane.child(
                        h_flex()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .child(control!(
                                "Document save failed",
                                hint(error.clone(), cx)
                                    .flex_1()
                                    .text_color(cx.theme().danger)
                            ))
                            .child(control!(
                                "Reload document",
                                Button::new("reload-document")
                                    .ghost()
                                    .small()
                                    .label("Reload")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.reload_active_editor(window, cx)
                                    }))
                            )),
                    )
                },
            )
            .when(truncated && !raw, |pane| {
                pane.child(hint("Showing the first 128 KiB", cx).px_3().py_1())
            })
            .child(div().relative().flex_1().min_h_0().min_w_0().child(content))
    }

    fn copy_button(
        &self,
        tooltip: &'static str,
        content: CopyContent,
        cx: &Context<Desktop>,
    ) -> Div {
        let id = self.id;
        let copied = self
            .copied_at
            .is_some_and(|at| at.elapsed() < COPY_FEEDBACK);
        control!(
            "Copy document",
            Button::new("copy-document")
                .ghost()
                .small()
                .icon(if copied {
                    IconName::Check
                } else {
                    IconName::Copy
                })
                .tooltip(tooltip)
                .disabled(self.load.is_some())
                .on_click(cx.listener(move |desktop, _, _, cx| {
                    let Some(tab) = desktop.active_document_mut(id) else {
                        return;
                    };
                    cx.write_to_clipboard(content.clipboard_item(cx));
                    tab.copied_at = Some(Instant::now());
                    cx.notify();
                    cx.spawn(async |view, cx| {
                        cx.background_executor().timer(COPY_FEEDBACK).await;
                        let _ = view.update(cx, |_, cx| cx.notify());
                    })
                    .detach();
                }))
        )
    }

    fn mode_control(&self, modes: &[(DocumentMode, &str)], cx: &Context<Desktop>) -> Div {
        let id = self.id;
        h_flex()
            .flex_none()
            .rounded_md()
            .p_0p5()
            .bg(cx.theme().muted)
            .children(
                modes
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, (mode, label))| {
                        control!(
                            format!("Document mode/{label}"),
                            Button::new(("document-mode", index))
                                .ghost()
                                .xsmall()
                                .h_6()
                                .label(label.to_owned())
                                .selected(self.mode == mode)
                                .when(self.mode == mode, |button| button.bg(cx.theme().background))
                                .on_click(cx.listener(move |desktop, _, window, cx| {
                                    if let Some(tab) = desktop.active_document_mut(id) {
                                        tab.mode = mode;
                                        desktop.focus_workspace(window, cx);
                                        desktop.open_active_editor(window, cx);
                                        cx.notify();
                                    }
                                }))
                        )
                    }),
            )
    }

    fn render_preview(&self, preview: &Preview, window: &mut Window, cx: &mut App) -> AnyElement {
        let body = match preview {
            Preview::Notice(message) => {
                return control!(*message, hint(*message, cx).p_4()).into_any_element();
            }
            Preview::Image { thumbnail, .. } => {
                div().p_4().child(img(thumbnail.clone()).max_w_full())
            }
            Preview::Text {
                formatted: Some(Formatted::Diff(diff)),
                ..
            } if self.mode == DocumentMode::Preview => {
                return Diff::render(diff, &self.diff_scroll, window, cx).into_any_element();
            }
            Preview::Text {
                text,
                formatted: Some(Formatted::Markdown(markdown)),
                ..
            } if self.mode == DocumentMode::Preview => div()
                .w_full()
                .px_6()
                .pb_10()
                .pt_4()
                .flex()
                .justify_center()
                .child(div().w_full().max_w(px(720.)).min_w_0().child(control!(
                    "Rendered Markdown",
                    markdown.render(
                        format!("markdown-document-{}", self.id),
                        text,
                        true,
                        window,
                        cx
                    )
                ))),
            Preview::Text { text, .. } => div()
                .min_w_full()
                .flex_none()
                .px_4()
                .py_3()
                .whitespace_nowrap()
                .font_family(cx.theme().mono_font_family.clone())
                .text_sm()
                .line_height(px(22.))
                .child(control!(
                    format!("Document text/{}", &text[..text.floor_char_boundary(1024)]),
                    StyledText::new(text.clone())
                )),
        };
        let scroll = &self.scroll;
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .id("document-content")
                    .size_full()
                    .overflow_scroll()
                    .track_scroll(scroll)
                    .child(body),
            )
            .scrollbar(scroll, ScrollbarAxis::Both)
            .into_any_element()
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
