use super::*;
use anyhow::{Context as _, ensure};
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use huggler::Attachment;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
};

/// A snapshot of the launch form. Skill and file mentions are part of the message.
#[derive(Serialize, Deserialize)]
pub(super) struct LaunchDraft {
    pub name: String,
    pub message: String,
    pub checkout: PathBuf,
    #[serde(default)]
    pub checkout_root: Option<PathBuf>,
    pub project_name: Option<String>,
    pub workspace_mode: WorkspaceMode,
    pub selected_branch: Option<git::Branch>,
    pub branch: String,
    pub model: Option<String>,
    pub model_options: ModelOptions,
    pub instance: ProviderInstanceId,
}

#[derive(Serialize, Deserialize)]
struct SavedDraft {
    launch: LaunchDraft,
    files: Vec<SavedFile>,
    created: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize)]
struct SavedFile {
    name: String,
    mime: String,
    size: usize,
}

#[derive(Default)]
pub(super) struct Backlog {
    pub open: bool,
    pub sending: Option<Uuid>,
    entries: Vec<(Uuid, SavedDraft)>,
    pub errors: HashMap<Uuid, String>,
    edit: Option<DraftEdit>,
}

struct DraftEdit {
    id: Uuid,
    name: Entity<InputState>,
    message: Entity<InputState>,
}

impl SavedDraft {
    fn write(
        path: &Path,
        launch: LaunchDraft,
        files: &[Attachment],
        created: chrono::DateTime<chrono::Utc>,
        replace: bool,
    ) -> anyhow::Result<()> {
        let draft = Self {
            launch,
            files: files
                .iter()
                .map(|file| SavedFile {
                    name: file.name.clone(),
                    mime: file.mime.clone(),
                    size: file.bytes.len(),
                })
                .collect(),
            created,
        };
        let directory = path.parent().expect("draft directory");
        fs::create_dir_all(directory)?;
        let mut pending = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(&mut pending, &draft)?;
        pending.write_all(b"\n")?;
        for file in files {
            pending.write_all(&file.bytes)?;
        }
        pending.as_file().sync_all()?;
        if replace {
            pending.persist(path)?;
        } else {
            pending.persist_noclobber(path)?;
        }
        Ok(())
    }

    // A JSON metadata line followed by the original file bytes lets the page
    // list and display drafts without opening attachment bodies.
    fn read_header(reader: &mut impl BufRead) -> anyhow::Result<Self> {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        Ok(serde_json::from_str(&header)?)
    }

    fn read(path: &Path) -> anyhow::Result<(LaunchDraft, Attachments)> {
        let mut reader = BufReader::new(fs::File::open(path)?);
        let draft = Self::read_header(&mut reader)?;
        let mut files = Vec::with_capacity(draft.files.len());
        let mut remaining = Attachment::MAX_TOTAL_BYTES;
        for file in draft.files {
            ensure!(
                file.size <= remaining,
                "Draft attachments exceed the upload limit"
            );
            remaining -= file.size;
            let mut bytes = vec![0; file.size];
            reader.read_exact(&mut bytes)?;
            files.push(Attachment {
                name: file.name,
                mime: file.mime,
                bytes,
            });
        }
        Ok((draft.launch, Attachments::from_files(files)))
    }
}

// The checkout query returns Forbidden when the thread has no checkout record.
pub(super) fn has_draft_conversation(runtime: &Huggler, id: &ThreadId) -> huggler::Result<bool> {
    match runtime.thread_checkout(id) {
        Ok(_) => Ok(true),
        Err(huggler::Error::Forbidden) => Ok(false),
        Err(error) => Err(error),
    }
}

impl Desktop {
    fn backlog_dir(&self) -> PathBuf {
        self.selection_file.with_file_name("backlog")
    }

    fn backlog_path(&self, id: Uuid) -> PathBuf {
        self.backlog_dir()
            .join(id.to_string())
            .with_extension("draft")
    }

    fn load_backlog(&mut self) -> anyhow::Result<()> {
        self.backlog.entries.clear();
        let directory = match fs::read_dir(self.backlog_dir()) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let mut failed = 0;
        for entry in directory {
            let Ok(entry) = entry else {
                failed += 1;
                continue;
            };
            let path = entry.path();
            let Some(id) = path
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(|name| Uuid::parse_str(name).ok())
                .filter(|_| {
                    path.extension()
                        .is_some_and(|extension| extension == "draft")
                })
            else {
                continue;
            };
            match fs::File::open(path)
                .map(BufReader::new)
                .map_err(anyhow::Error::from)
                .and_then(|mut reader| SavedDraft::read_header(&mut reader))
            {
                Ok(draft) => self.backlog.entries.push((id, draft)),
                Err(_) => failed += 1,
            }
        }
        self.backlog
            .entries
            .sort_unstable_by_key(|(_, draft)| std::cmp::Reverse(draft.created));
        ensure!(failed == 0, "Could not read {failed} saved draft(s)");
        Ok(())
    }

    pub(super) fn open_backlog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.leave_settings(cx) {
            return;
        }
        self.suggestions = Default::default();
        self.project_picker = None;
        self.model_picker = None;
        self.mcp_status = None;
        self.launch_workspace.close_picker();
        #[cfg(unix)]
        {
            self.remote_project = None;
        }
        self.backlog.open = true;
        self.error = self
            .load_backlog()
            .err()
            .map(|error| format!("Could not load Backlog. {error:#}"));
        self.focus.focus(window);
        cx.notify();
    }

    fn save_to_backlog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        if self.launching.is_some()
            || self.checkout_browse.is_some()
            || self.starter_attachments.loading
            || self.launch_workspace.loading_branches()
        {
            return Ok(());
        }
        let mut launch = self.launch_draft(cx);
        // Save the branch shown in the form even when it was chosen by default.
        if !launch.checkout.as_os_str().is_empty() {
            launch.checkout = std::path::absolute(&launch.checkout)?;
            launch.checkout_root = self
                .launch_workspace
                .branches()
                .map(|branches| branches.root.canonicalize())
                .transpose()?;
            launch.workspace_mode = self.launch_workspace.mode();
            launch.selected_branch = self
                .launch_workspace
                .displayed_branch(launch.workspace_mode)
                .cloned();
        }
        let files = self.starter_attachments.files();
        ensure!(
            !launch.message.trim().is_empty() || !files.is_empty(),
            "Add a message or attachment first"
        );
        SavedDraft::write(
            &self.backlog_path(Uuid::new_v4()),
            launch,
            files,
            chrono::Utc::now(),
            false,
        )?;
        self.starter_attachments = Attachments::default();
        self.starter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.name
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.branch
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.open_backlog(window, cx);
        Ok(())
    }

    pub(super) fn save_backlog_button(&self, disabled: bool, cx: &Context<Self>) -> AnyElement {
        control!(
            "Save to Backlog",
            Button::new("save-to-backlog")
                .small()
                .ghost()
                .label("Save to Backlog")
                .disabled(disabled || self.launch_workspace.loading_branches())
                .on_click(cx.listener(|view, _, window, cx| {
                    if let Err(error) = view.save_to_backlog(window, cx) {
                        view.error = Some(format!("Could not save draft. {error:#}"));
                        cx.notify();
                    }
                }))
        )
        .into_any_element()
    }

    fn send_backlog(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if self.backlog.sending.is_some() || self.launching.is_some() {
            return;
        }
        self.backlog.errors.remove(&id);
        match SavedDraft::read(&self.backlog_path(id)).context("Could not read the saved draft") {
            Ok((draft, attachments)) => self.start_launch(draft, attachments, Some(id), window, cx),
            Err(error) => {
                self.backlog.errors.insert(id, format!("{error:#}"));
                cx.notify();
            }
        }
    }

    pub(super) fn remove_saved_draft(&mut self, id: Uuid) -> std::io::Result<()> {
        match fs::remove_file(self.backlog_path(id)) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        self.backlog.entries.retain(|(entry, _)| *entry != id);
        self.backlog.errors.remove(&id);
        Ok(())
    }

    fn start_draft_edit(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if self.backlog.sending == Some(id) || self.backlog.edit.is_some() {
            return;
        }
        match has_draft_conversation(&self.runtime, &ThreadId(id.to_string())) {
            Ok(false) => (),
            Ok(true) => {
                self.backlog.errors.insert(
                    id,
                    "This draft already has a conversation. Retry delivery with the saved message."
                        .into(),
                );
                cx.notify();
                return;
            }
            Err(error) => {
                self.backlog.errors.insert(id, error.to_string());
                cx.notify();
                return;
            }
        }
        let Some((_, draft)) = self.backlog.entries.iter().find(|(entry, _)| *entry == id) else {
            return;
        };
        let name =
            cx.new(|cx| InputState::new(window, cx).default_value(draft.launch.name.clone()));
        let message = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(3, 12)
                .default_value(draft.launch.message.clone())
        });
        cx.observe(&message, |_, _, cx| cx.notify()).detach();
        message.focus_handle(cx).focus(window);
        self.backlog.edit = Some(DraftEdit { id, name, message });
        self.backlog.errors.remove(&id);
        cx.notify();
    }

    fn save_draft_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let Some(edit) = &self.backlog.edit else {
            return Ok(());
        };
        let id = edit.id;
        let name = edit.name.read(cx).value().trim().to_owned();
        let message = edit.message.read(cx).value().to_string();
        let (_, draft) = self
            .backlog
            .entries
            .iter()
            .find(|(entry, _)| *entry == id)
            .context("Draft is no longer available")?;
        let path = self.backlog_path(id);
        let (mut launch, attachments) = SavedDraft::read(&path)?;
        ensure!(
            !message.trim().is_empty() || !attachments.is_empty(),
            "Add a message or keep an attachment"
        );
        launch.name = if name.is_empty() {
            launch_workspace::conversation_title(&message, attachments.first_name())
        } else {
            name
        };
        launch.message = message;
        SavedDraft::write(&path, launch, attachments.files(), draft.created, true)?;
        self.backlog.edit = None;
        self.open_backlog(window, cx);
        Ok(())
    }

    fn confirm_delete_draft(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if self.backlog.sending == Some(id) {
            return;
        }
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        let scripted = crate::qualification::take_prompt_answer(cx);
        #[cfg(not(all(feature = "qualification", target_os = "macos")))]
        let scripted: Option<usize> = None;
        let prompt = scripted.is_none().then(|| {
            window.prompt(
                PromptLevel::Warning,
                "Delete this draft?",
                Some("This removes the saved message and its attachments from Backlog."),
                &["Cancel", "Delete"],
                cx,
            )
        });
        cx.spawn(async move |view, cx| {
            let answer = match prompt {
                Some(prompt) => prompt.await.ok(),
                None => scripted,
            };
            if answer == Some(1) {
                let _ = view.update(cx, |view, cx| {
                    if view.backlog.sending != Some(id)
                        && let Err(error) = view.remove_saved_draft(id)
                    {
                        view.error = Some(format!("Could not delete draft. {error}"));
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn draft_editor(&self, edit: &DraftEdit, cx: &Context<Self>) -> Div {
        let id = edit.id;
        let can_save = !edit.message.read(cx).value().trim().is_empty()
            || self
                .backlog
                .entries
                .iter()
                .any(|(entry, draft)| *entry == id && !draft.files.is_empty());
        v_flex()
            .gap_2()
            .child(field("Draft title", &edit.name))
            .child(field("Draft message", &edit.message))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(control!(
                        "Cancel draft edit",
                        Button::new("cancel-draft-edit")
                            .small()
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.backlog.edit = None;
                                view.backlog.errors.remove(&id);
                                view.focus.focus(window);
                                cx.notify();
                            }))
                    ))
                    .child(control!(
                        "Save draft changes",
                        Button::new("save-draft-edits")
                            .small()
                            .primary()
                            .label("Save changes")
                            .disabled(!can_save)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                if let Err(error) = view.save_draft_edit(window, cx) {
                                    view.backlog
                                        .errors
                                        .insert(id, format!("Could not save changes. {error:#}"));
                                    cx.notify();
                                }
                            }))
                    )),
            )
    }

    pub(super) fn backlog_page(
        &self,
        navigation_open: bool,
        width: Pixels,
        cx: &Context<Self>,
    ) -> Div {
        let count = self.backlog.entries.len();
        let empty = count == 0;
        let draft_count = if count == 1 {
            "1 saved draft".into()
        } else {
            format!("{count} saved drafts")
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .h_10()
                    .flex_shrink_0()
                    .px_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .gap_2()
                    .child(self.navigation_toggle(navigation_open, cx))
                    .child(label("Backlog")),
            )
            .child(
                v_flex()
                    .id("backlog-list")
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(px(736.))
                            .flex_shrink_0()
                            .px_4()
                            .pt_5()
                            .pb_8()
                            .gap_3()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .justify_between()
                                    .pb_3()
                                    .child(heading(if empty { "Saved drafts".into() } else { draft_count }))
                                    .child(control!(
                                        "Create a draft",
                                        Button::new("create-draft")
                                            .small()
                                            .outline()
                                            .icon(IconName::Plus)
                                            .label("New draft")
                                            .on_click(cx.listener(|view, _, window, cx| {
                                                view.new_conversation(false, window, cx)
                                            }))
                                    )),
                            )
                            .when(empty, |list| {
                                list.child(
                                    v_flex()
                                        .gap_1()
                                        .py_6()
                                        .child(label("No saved drafts"))
                                        .child(hint(
                                            "Use Save to Backlog in the composer to keep a draft for later.",
                                            cx,
                                        )),
                                )
                            })
                            .children(
                                self.backlog
                                    .entries
                                    .iter()
                                    .map(|(id, draft)| self.backlog_row(*id, draft, width < px(490.), cx)),
                            ),
                    ),
            )
    }

    fn backlog_row(&self, id: Uuid, draft: &SavedDraft, compact: bool, cx: &Context<Self>) -> Div {
        let launch = &draft.launch;
        let sending = self.backlog.sending == Some(id);
        let error = self.backlog.errors.get(&id);
        let action = if sending {
            "Sending…"
        } else if error.is_some() {
            "Retry"
        } else {
            "Send"
        };
        let scratch = launch.checkout.as_os_str().is_empty();
        let workspace = if scratch {
            "Scratch workspace".into()
        } else {
            let project = launch
                .project_name
                .clone()
                .or_else(|| {
                    launch
                        .checkout
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "Git checkout".into());
            format!("{project} · {}", launch.workspace_mode.label())
        };
        let branch = (!scratch)
            .then(|| {
                let mut options = Vec::new();
                if let Some(branch) = &launch.selected_branch {
                    options.push(match launch.workspace_mode {
                        WorkspaceMode::Worktree => branch.base_label(),
                        WorkspaceMode::Current => branch.name.clone(),
                    });
                } else if launch.workspace_mode == WorkspaceMode::Worktree {
                    options.push("Default base branch".into());
                }
                if launch.workspace_mode == WorkspaceMode::Worktree && !launch.branch.is_empty() {
                    options.push(format!("New branch {}", launch.branch));
                }
                options
            })
            .filter(|options| !options.is_empty())
            .map(|options| options.join(" · "));
        let provider = self
            .runtime
            .output()
            .provider_label(launch.instance.as_str());
        let model = launch.model.as_deref().unwrap_or("Automatic");
        let mut model_options = Vec::new();
        if let Some(effort) = &launch.model_options.effort {
            model_options.push(format!("{} effort", models::effort_label(effort)));
        }
        if let Some(tier) = &launch.model_options.service_tier {
            model_options.push(match tier.as_str() {
                "priority" | "fast" => "Fast mode".into(),
                "default" => "Standard speed".into(),
                _ => format!("Service tier {tier}"),
            });
        }
        let group = SharedString::from(format!("draft-{id}"));
        let edit = self.backlog.edit.as_ref().filter(|edit| edit.id == id);
        let menu_view = cx.weak_entity();
        let can_edit = self.backlog.edit.is_none();
        let reveal = sending;
        let controls = h_flex()
            .gap_1()
            .flex_shrink_0()
            .child(control!(
                format!("Send draft/{}", launch.name),
                Button::new((ElementId::from(id), "send"))
                    .small()
                    .primary()
                    .icon(IconName::ArrowUp)
                    .label(action)
                    .opacity(if reveal { 1. } else { 0. })
                    .group_hover(group.clone(), |style| style.opacity(1.))
                    .focus(|style| style.opacity(1.))
                    .disabled(self.backlog.sending.is_some() || self.launching.is_some())
                    .on_click(
                        cx.listener(move |view, _, window, cx| view.send_backlog(id, window, cx))
                    )
            ))
            .child(control!(
                format!("Draft actions/{}", launch.name),
                Button::new((ElementId::from(id), "actions"))
                    .small()
                    .ghost()
                    .icon(IconName::Ellipsis)
                    .tooltip("Draft actions")
                    .opacity(0.)
                    .group_hover(group.clone(), |style| style.opacity(1.))
                    .focus(|style| style.opacity(1.))
                    .disabled(sending)
                    .dropdown_menu_with_anchor(Corner::BottomRight, move |menu, _, _| {
                        let edit_view = menu_view.clone();
                        let delete_view = menu_view.clone();
                        menu.item(
                            PopupMenuItem::element(|_, _| {
                                control!("Edit draft", div().child("Edit draft"))
                            })
                            .disabled(!can_edit)
                            .on_click(move |_, window, cx| {
                                let _ = edit_view
                                    .update(cx, |view, cx| view.start_draft_edit(id, window, cx));
                            }),
                        )
                        .item(
                            PopupMenuItem::element(|_, _| {
                                control!("Delete draft", div().child("Delete draft"))
                            })
                            .on_click(move |_, window, cx| {
                                let _ = delete_view.update(cx, |view, cx| {
                                    view.confirm_delete_draft(id, window, cx)
                                });
                            }),
                        )
                    })
            ));
        let card =
            v_flex()
                .id(id)
                .gap_1()
                .p_4()
                .rounded_xl()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted.opacity(0.42))
                .group(group)
                .child(if let Some(edit) = edit {
                    self.draft_editor(edit, cx).mb_2()
                } else if launch.message.trim().is_empty() {
                    div().text_base().mb_2().child("Attachment-only draft")
                } else {
                    div()
                        .text_base()
                        .line_height(px(24.))
                        .mb_2()
                        .child(launch.message.clone())
                })
                .when(!draft.files.is_empty(), |card| {
                    card.child(h_flex().flex_wrap().gap_1().mb_2().children(
                        draft.files.iter().map(|file| {
                            h_flex()
                                .gap_1()
                                .max_w_full()
                                .rounded_md()
                                .border_1()
                                .border_color(cx.theme().border)
                                .px_2()
                                .py_1()
                                .child(crate::file_icons::icon(
                                    Path::new(&file.name),
                                    cx.theme().muted_foreground,
                                ))
                                .child(hint(file.name.clone(), cx).min_w_0())
                        }),
                    ))
                })
                .child(
                    h_flex()
                        .items_end()
                        .when(compact, |footer| footer.flex_col())
                        .gap_2()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .pt_2()
                        .child(
                            v_flex()
                                .when(compact, |details| details.w_full())
                                .when(!compact, |details| details.flex_1())
                                .min_w_0()
                                .gap_1()
                                .when(launch.name != launch.message.trim(), |details| {
                                    details.child(draft_option("Title", launch.name.clone(), cx))
                                })
                                .child(draft_option("Workspace", workspace, cx))
                                .when_some(branch, |details, branch| {
                                    details.child(draft_option("Branch", branch, cx))
                                })
                                .child(draft_option("Model", format!("{provider} · {model}"), cx))
                                .when(!model_options.is_empty(), |details| {
                                    details.child(draft_option(
                                        "Options",
                                        model_options.join(" · "),
                                        cx,
                                    ))
                                }),
                        )
                        .when(edit.is_none(), |footer| footer.child(controls)),
                )
                .when_some(error, |row, error| {
                    row.child(control!(
                        format!("error/{error}"),
                        h_flex()
                            .items_start()
                            .gap_2()
                            .child(Icon::new(IconName::TriangleAlert).small().flex_shrink_0())
                            .child(div().min_w_0().text_sm().child(error.clone()))
                    ))
                });
        control!(format!("Draft card/{}", launch.name), card)
    }
}

fn draft_option(name: &'static str, value: String, cx: &App) -> Div {
    h_flex()
        .items_start()
        .gap_2()
        .child(label(name).w(px(76.)).flex_shrink_0())
        .child(hint(value, cx).min_w_0())
}
