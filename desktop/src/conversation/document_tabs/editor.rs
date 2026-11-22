mod file;

use super::*;
use file::{read_source, uses_crlf, write_source};
use gpui_component::highlighter::Language;

pub(super) enum Editor {
    Loading { _task: Task<()> },
    Ready(Box<Draft>),
    Unavailable(String),
}

impl Editor {
    pub(super) fn draft(&self) -> Option<&Draft> {
        match self {
            Self::Ready(draft) => Some(draft),
            _ => None,
        }
    }

    fn draft_mut(&mut self) -> Option<&mut Draft> {
        match self {
            Self::Ready(draft) => Some(draft),
            _ => None,
        }
    }

    pub(super) fn render(&self, cx: &App) -> Div {
        match self {
            Self::Loading { .. } => hint("Reading working file…", cx).p_4(),
            Self::Unavailable(error) => hint(error.clone(), cx).p_4(),
            Self::Ready(draft) => control!(
                "Raw editor",
                Input::new(&draft.input)
                    .appearance(false)
                    .size_full()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .font_family(cx.theme().mono_font_family.clone())
                    .disabled(draft.is_saving())
            )
            .size_full(),
        }
    }
}

pub(super) struct Draft {
    pub(super) input: Entity<InputState>,
    pub(super) dirty: bool,
    pub(super) error: Option<String>,
    saved: SharedString,
    disk: SharedString,
    crlf: bool,
    save: Option<Task<()>>,
    _input: Subscription,
}

impl Draft {
    pub(super) fn is_saving(&self) -> bool {
        self.save.is_some()
    }

    fn new(
        disk: SharedString,
        thread: ThreadId,
        path: Arc<Path>,
        window: &mut Window,
        cx: &mut Context<Desktop>,
    ) -> Self {
        let crlf = uses_crlf(&disk);
        let saved = if crlf {
            disk.replace("\r\n", "\n").into()
        } else {
            disk.clone()
        };
        let language =
            Language::from_str(path.extension().and_then(|ext| ext.to_str()).unwrap_or(""));
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(language.name())
                .soft_wrap(false)
                .default_value(saved.clone())
        });
        let subscription = cx.subscribe(&input, move |view, input, event, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(draft) = view.document_draft_mut(&thread, &path)
            {
                draft.dirty = input.read(cx).text() != draft.saved.as_str();
                cx.notify();
            }
        });
        Self {
            input,
            dirty: false,
            error: None,
            saved,
            disk,
            crlf,
            save: None,
            _input: subscription,
        }
    }
}

impl Desktop {
    fn document_draft_mut(&mut self, thread: &ThreadId, path: &Path) -> Option<&mut Draft> {
        self.workspaces
            .get_mut(thread)?
            .documents
            .editors
            .get_mut(path)?
            .draft_mut()
    }

    pub(super) fn load_raw_editor(
        &mut self,
        thread: &ThreadId,
        path: Arc<Path>,
        reload: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspaces.get_mut(thread).expect("document workspace");
        if let Some(editor) = workspace.documents.editors.get(&path)
            && (!reload
                || editor
                    .draft()
                    .is_some_and(|draft| draft.dirty || draft.is_saving()))
        {
            if let Some(draft) = editor.draft()
                && self
                    .current
                    .as_ref()
                    .is_some_and(|current| current.id == *thread)
                && workspace.documents.active().is_some_and(|tab| {
                    tab.mode == DocumentMode::Raw && tab.document.path() == &*path
                })
            {
                draft.input.focus_handle(cx).focus(window);
            }
            return;
        }
        let root = workspace.root.clone();
        let thread = thread.clone();
        let source_path = path.clone();
        let job = cx.background_spawn(async move { read_source(&root, &source_path) });
        let editor_path = path.clone();
        let task = cx.spawn_in(window, async move |view, cx| {
            let result = job.await;
            let _ = view.update_in(cx, |view, window, cx| {
                let editor = match result {
                    Ok(text) => {
                        let draft =
                            Draft::new(text, thread.clone(), editor_path.clone(), window, cx);
                        if view
                            .current
                            .as_ref()
                            .is_some_and(|current| current.id == thread)
                            && view.workspaces[&thread]
                                .documents
                                .active()
                                .is_some_and(|tab| {
                                    tab.mode == DocumentMode::Raw
                                        && tab.document.path() == &*editor_path
                                })
                            && !view.show_settings
                            && !window.has_active_dialog(cx)
                        {
                            draft.input.focus_handle(cx).focus(window);
                        }
                        Editor::Ready(Box::new(draft))
                    }
                    Err(error) => {
                        Editor::Unavailable(format!("Could not open Raw editor. {error:#}"))
                    }
                };
                if let Some(workspace) = view.workspaces.get_mut(&thread) {
                    workspace.documents.editors.insert(editor_path, editor);
                    cx.notify();
                }
            });
        });
        workspace
            .documents
            .editors
            .insert(path, Editor::Loading { _task: task });
        cx.notify();
    }

    pub(in crate::conversation) fn save_active_document(
        &mut self,
        _: &crate::SaveDocument,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.current.as_ref().map(|current| current.id.clone()) else {
            return;
        };
        let workspace = self
            .workspaces
            .get_mut(&thread)
            .expect("selected workspace");
        let Some(path) = workspace
            .documents
            .active()
            .map(|tab| Arc::<Path>::from(tab.document.path()))
        else {
            return;
        };
        let Some(draft) = workspace
            .documents
            .editors
            .get_mut(&path)
            .and_then(Editor::draft_mut)
        else {
            return;
        };
        if !draft.dirty || draft.is_saving() {
            return;
        }
        let value = draft.input.read(cx).value();
        let disk: SharedString = if draft.crlf {
            value.replace("\r\n", "\n").replace('\n', "\r\n").into()
        } else {
            value.clone()
        };
        let root = workspace.root.clone();
        let saved_path = path.clone();
        let expected = draft.disk.clone();
        let contents = disk.clone();
        let job =
            cx.background_spawn(
                async move { write_source(&root, &saved_path, &expected, &contents) },
            );
        draft.error = None;
        draft.save = Some(cx.spawn(async move |view, cx| {
            let result = job.await;
            let _ = view.update(cx, |view, cx| {
                let Some(draft) = view.document_draft_mut(&thread, &path) else {
                    return;
                };
                draft.save = None;
                match result {
                    Ok(()) => {
                        draft.disk = disk;
                        draft.saved = value;
                        draft.dirty = draft.input.read(cx).text() != draft.saved.as_str();
                        view.refresh_working_file_views(&thread, &path, cx);
                        if view
                            .current
                            .as_ref()
                            .is_some_and(|current| current.id == thread)
                        {
                            view.watch_pull_request(cx);
                        }
                    }
                    Err(error) => draft.error = Some(format!("Could not save. {error:#}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn refresh_working_file_views(
        &mut self,
        thread: &ThreadId,
        path: &Path,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspaces.get_mut(thread).expect("document workspace");
        workspace.refresh_after_checkout_update(self.workspace_tab, cx);
        for tab in &mut workspace.documents.tabs {
            if tab.document.path() == path {
                tab.revert_error = None;
                tab.load(thread, &workspace.root, true, cx);
            }
        }
    }

    pub(super) fn reload_active_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.current.as_ref().map(|current| current.id.clone()) else {
            return;
        };
        let Some(tab) = self.workspaces[&thread].documents.active() else {
            return;
        };
        let path: Arc<Path> = tab.document.path().into();
        let draft = self.workspaces[&thread]
            .documents
            .editors
            .get(&path)
            .and_then(Editor::draft);
        if draft.is_some_and(Draft::is_saving) || window.has_active_dialog(cx) {
            return;
        }
        if draft.is_some_and(|draft| draft.dirty) {
            let view = cx.weak_entity();
            confirm_discard(
                "Reload and discard edits?",
                format!("Reloading {} discards your unsaved edits.", path.display()),
                "Reload file",
                move |window, cx| {
                    let _ = view.update(cx, |view, cx| {
                        if let Some(workspace) = view.workspaces.get_mut(&thread) {
                            workspace.documents.editors.remove(&path);
                            view.load_raw_editor(&thread, path.clone(), true, window, cx);
                            view.refresh_working_file_views(&thread, &path, cx);
                        }
                    });
                },
                window,
                cx,
            );
        } else {
            self.load_raw_editor(&thread, path.clone(), true, window, cx);
            self.refresh_working_file_views(&thread, &path, cx);
        }
    }

    pub(crate) fn can_close_with_edits(&self, window: &mut Window, cx: &mut App) -> bool {
        let drafts = || {
            self.workspaces.values().flat_map(|workspace| {
                workspace
                    .documents
                    .editors
                    .values()
                    .filter_map(Editor::draft)
            })
        };
        let saving = drafts().any(Draft::is_saving);
        if !saving && !drafts().any(|draft| draft.dirty) {
            return true;
        }
        if !window.has_active_dialog(cx) {
            if saving {
                window.open_dialog(cx, |dialog, _, cx| {
                    dialog
                        .title("Saving changes…")
                        .child(hint("Wait for the save to finish before quitting.", cx))
                });
            } else {
                confirm_discard(
                    "Quit with unsaved changes?",
                    "Quitting discards your unsaved edits.".into(),
                    "Quit and discard",
                    |_, cx| cx.quit(),
                    window,
                    cx,
                );
            }
        }
        false
    }
}

pub(super) fn confirm_discard(
    title: &'static str,
    detail: String,
    confirm: &'static str,
    action: impl Fn(&mut Window, &mut App) + Clone + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |dialog, _, cx| {
        let action = action.clone();
        dialog
            .title(title)
            .confirm()
            .on_ok(|_, _, _| false)
            .child(hint(detail.clone(), cx))
            .footer(move |_, _, _, _| {
                let action = action.clone();
                vec![
                    control!(
                        "Keep editing",
                        Button::new("keep-editing")
                            .label("Keep editing")
                            .on_click(|_, window, cx| window.close_dialog(cx))
                    ),
                    control!(
                        "Discard document edits",
                        Button::new("discard-edits")
                            .label(confirm)
                            .danger()
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                action(window, cx);
                            })
                    ),
                ]
            })
    });
}
