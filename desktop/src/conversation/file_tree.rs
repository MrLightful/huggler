use crate::{
    file_icons,
    open_in::{OpenIn, file_menu},
    ui::hint,
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    tooltip::Tooltip,
};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

const FILE_ROW_GROUP: &str = "workspace-file-row";

pub(super) struct FileTree {
    root: PathBuf,
    expanded: HashSet<Arc<Path>>,
    rows: Option<io::Result<Vec<Row>>>,
    scan: Option<Task<()>>,
    scroll: UniformListScrollHandle,
}

pub(super) enum FileAction {
    Open(Arc<Path>),
    AttachFile(Arc<Path>),
    AttachReference(Arc<Path>),
    OpenIn(OpenIn, Arc<Path>),
}

impl EventEmitter<FileAction> for FileTree {}

enum Entry {
    Directory { expanded: bool },
    File,
    Symlink,
    Status(&'static str),
}

struct Row {
    path: Arc<Path>,
    depth: usize,
    entry: Entry,
}

impl FileTree {
    pub(super) fn new(root: PathBuf) -> Self {
        Self {
            root,
            expanded: HashSet::new(),
            rows: None,
            scan: None,
            scroll: UniformListScrollHandle::default(),
        }
    }

    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let expanded = self.expanded.clone();
        let scan = cx.background_spawn(async move {
            let root = root.canonicalize()?;
            let mut rows = Vec::new();
            append_rows(&root, Path::new(""), 0, &expanded, &mut rows)?;
            Ok(rows)
        });
        self.scan = Some(cx.spawn(async move |tree, cx| {
            let rows = scan.await;
            let _ = tree.update(cx, |tree, cx| {
                tree.rows = Some(rows);
                tree.scan = None;
                cx.notify();
            });
        }));
    }

    fn toggle(&mut self, path: &Arc<Path>, cx: &mut Context<Self>) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.clone());
        }
        self.refresh(cx);
    }
}

fn append_rows(
    root: &Path,
    directory: &Path,
    depth: usize,
    expanded_paths: &HashSet<Arc<Path>>,
    rows: &mut Vec<Row>,
) -> io::Result<()> {
    let path = root.join(directory);
    // An expanded directory can be replaced by a symlink during a checkout update.
    if !directory.as_os_str().is_empty() && path.canonicalize()? != path {
        return Err(io::Error::other("Directory moved"));
    }
    let mut children = Vec::new();
    for child in fs::read_dir(path)? {
        let child = child?;
        let name = child.file_name();
        if name == ".git" {
            continue;
        }
        let kind = match child.file_type() {
            Ok(kind) => kind,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if kind.is_dir() || kind.is_file() || kind.is_symlink() {
            children.push((name, kind));
        }
    }
    children.sort_unstable_by(|(left, left_kind), (right, right_kind)| {
        (!left_kind.is_dir(), left).cmp(&(!right_kind.is_dir(), right))
    });

    for (name, kind) in children {
        let path: Arc<Path> = directory.join(name).into();
        let expanded = kind.is_dir() && expanded_paths.contains(&path);
        let entry = if kind.is_dir() {
            Entry::Directory { expanded }
        } else if kind.is_symlink() {
            Entry::Symlink
        } else {
            Entry::File
        };
        rows.push(Row {
            path: path.clone(),
            depth,
            entry,
        });
        if expanded {
            let start = rows.len();
            let status = match append_rows(root, &path, depth + 1, expanded_paths, rows) {
                Ok(()) if rows.len() == start => "Empty folder",
                Ok(()) => continue,
                Err(_) => "Could not read folder",
            };
            rows.push(Row {
                path,
                depth: depth + 1,
                entry: Entry::Status(status),
            });
        }
    }
    Ok(())
}

impl Render for FileTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message: SharedString = match &self.rows {
            None => "Reading files…".into(),
            Some(Err(error)) => format!("Could not read workspace files. {error}").into(),
            Some(Ok(rows)) if rows.is_empty() => "No files in this workspace".into(),
            Some(Ok(rows)) => {
                let list = uniform_list(
                    ("all-files", cx.entity_id()),
                    rows.len(),
                    cx.processor(|tree, range: std::ops::Range<usize>, _, cx| {
                        let Some(Ok(rows)) = &tree.rows else {
                            return Vec::new();
                        };
                        rows.get(range)
                            .unwrap_or_default()
                            .iter()
                            .map(|row| row.render(cx))
                            .collect()
                    }),
                )
                .size_full()
                .with_sizing_behavior(ListSizingBehavior::Auto)
                .track_scroll(self.scroll.clone());
                return div()
                    .size_full()
                    .relative()
                    .child(list)
                    .vertical_scrollbar(&self.scroll)
                    .into_any_element();
            }
        };
        control!(&message, hint(message.clone(), cx).px_3().py_2()).into_any_element()
    }
}

impl Row {
    fn render(&self, cx: &Context<FileTree>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let label = h_flex().h_full().flex_1().min_w_0().gap_1().child(
            div()
                .w(px(12.) * self.depth)
                .max_w(relative(0.5))
                .flex_shrink_0(),
        );
        let row = h_flex().h_6().w_full().min_w_0().px_2();
        let path = self.path.clone();
        let symbol = |name| Icon::new(name).small().text_color(muted).into_any_element();
        let (icon, chevron) = match self.entry {
            Entry::Directory { expanded: true } => {
                (symbol(IconName::FolderOpen), Some(IconName::ChevronDown))
            }
            Entry::Directory { expanded: false } => {
                (symbol(IconName::Folder), Some(IconName::ChevronRight))
            }
            Entry::File => (file_icons::icon(&path, muted), None),
            Entry::Symlink => (symbol(IconName::ExternalLink), None),
            Entry::Status(message) => {
                return control!(
                    message,
                    row.child(label.child(hint(message, cx).pl_4().truncate()))
                )
                .into_any_element();
            }
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let is_directory = chevron.is_some();
        let disclosure = div().w_3().flex_shrink_0().when_some(chevron, |div, icon| {
            div.child(Icon::new(icon).xsmall().text_color(muted))
        });
        let tooltip = path.display().to_string();
        let accent = cx.theme().sidebar_accent;
        let label = label
            .id("file-label")
            .tab_index(0)
            .focus(move |style| style.bg(accent))
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(disclosure)
            .child(icon)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(crate::ui::control_text_size())
                    .when(is_directory, |name| name.font_weight(FontWeight::MEDIUM))
                    .child(SharedString::from(name.into_owned())),
            )
            // Expanding with the mouse should leave keyboard focus where it was.
            .when(is_directory, |label| {
                label.capture_any_mouse_down(|_, window, _| window.prevent_default())
            })
            .on_click(cx.listener({
                let path = path.clone();
                move |tree, _, _, cx| {
                    if is_directory {
                        tree.toggle(&path, cx);
                    } else {
                        cx.emit(FileAction::Open(path.clone()));
                    }
                }
            }));
        let attach_file = matches!(self.entry, Entry::File);
        let attach_path = path.clone();
        let attach = row_action("attach-to-chat", IconName::Plus)
            .tooltip("Attach to chat")
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(if attach_file {
                    FileAction::AttachFile(attach_path.clone())
                } else {
                    FileAction::AttachReference(attach_path.clone())
                });
            }));
        let tree = cx.weak_entity();
        let open_path = path.clone();
        let open = file_menu(
            row_action("open-workspace-file", IconName::ChevronDown),
            &path,
            move |destination, cx| {
                let _ = tree.update(cx, |_, cx| {
                    cx.emit(FileAction::OpenIn(destination, open_path.clone()));
                });
            },
        );
        row.id(path)
            .group(FILE_ROW_GROUP)
            .rounded_md()
            .hover(move |style| style.bg(accent))
            .child(
                control!(format!("Workspace file/{}", self.path.display()), label)
                    .flex_1()
                    .min_w_0()
                    .h_full(),
            )
            .child(control!(format!("Attach to chat/{}", self.path.display()), attach).flex_none())
            .child(open)
            .into_any_element()
    }
}

fn row_action(id: &'static str, icon: IconName) -> Button {
    Button::new(id)
        .with_size(px(18.))
        .ghost()
        .icon(icon)
        .opacity(0.)
        .group_hover(FILE_ROW_GROUP, |style| style.opacity(1.))
        .focus(|style| style.opacity(1.))
}
