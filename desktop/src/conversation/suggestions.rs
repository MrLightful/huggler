use super::*;
use gpui_component::{
    Icon, StyledExt,
    input::{Enter, Escape, IndentInline, MoveDown, MoveUp},
    kbd::Kbd,
    tooltip::Tooltip,
};
use huggler::ProviderSkill;
use std::path::Path;

const LIMIT: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Files,
    Skills,
}

#[derive(Clone, PartialEq, Eq)]
struct Trigger {
    kind: Kind,
    start: usize,
    end: usize,
    query: String,
}

impl Trigger {
    fn at(text: &str, cursor: usize) -> Option<Self> {
        let before = text.get(..cursor)?;
        let start = before
            .char_indices()
            .rev()
            .find(|(_, ch)| ch.is_whitespace())
            .map_or(0, |(ix, ch)| ix + ch.len_utf8());
        let end = cursor
            + text[cursor..]
                .find(char::is_whitespace)
                .unwrap_or(text.len() - cursor);
        let kind = match before.as_bytes().get(start) {
            Some(b'@') => Kind::Files,
            Some(b'/') if !text[start + 1..end].contains(['/', '\\']) => Kind::Skills,
            _ => return None,
        };
        let query = before[start + 1..].to_lowercase();
        Some(Self {
            kind,
            start,
            end,
            query,
        })
    }

    fn utf16_range(&self, text: &str) -> std::ops::Range<usize> {
        let end = self.end + usize::from(text[self.end..].starts_with(' '));
        text[..self.start].encode_utf16().count()..text[..end].encode_utf16().count()
    }
}

struct FileMatch {
    path: String,
    name: String,
    search_path: String,
}

impl FileMatch {
    fn new(path: PathBuf) -> Option<Self> {
        let name = path.file_name()?.to_str()?.to_owned();
        let path = path.to_str()?.to_owned();
        if path.chars().any(char::is_control) {
            return None;
        }
        Some(Self {
            name,
            search_path: path.to_lowercase(),
            path,
        })
    }

    fn reference(&self) -> String {
        let mut label = String::new();
        for char in self.name.chars() {
            if "\\[]*_&<`".contains(char) {
                label.push('\\');
            }
            label.push(char);
        }
        let mut destination = String::new();
        for byte in self.path.bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                destination.push(char::from(byte));
            } else {
                use std::fmt::Write;
                let _ = write!(destination, "%{byte:02X}");
            }
        }
        format!("[{label}]({destination})")
    }
}

fn checkout_files(
    root: &Path,
    starter: bool,
    base: Option<&str>,
) -> anyhow::Result<Vec<FileMatch>> {
    let root = if starter {
        git::checkout_root(root)?
    } else {
        root.to_path_buf()
    };
    let paths = if let Some(base) = base {
        git::files_at(&root, base)?
    } else if root.join(".git").exists() {
        git::workspace_files(&root)?
            .into_iter()
            .filter(|path| fs::symlink_metadata(root.join(path)).is_ok_and(|m| m.is_file()))
            .collect()
    } else {
        let mut files = Vec::new();
        let mut dirs = vec![PathBuf::new()];
        while let Some(dir) = dirs.pop() {
            let entries = match fs::read_dir(root.join(&dir)) {
                Ok(entries) => entries,
                Err(error) if dir.as_os_str().is_empty() => return Err(error.into()),
                Err(_) => continue,
            };
            for entry in entries {
                let Ok(entry) = entry else { continue };
                let path = dir.join(entry.file_name());
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_file() {
                    files.push(path);
                } else if kind.is_dir() && entry.file_name() != ".git" {
                    dirs.push(path);
                }
            }
        }
        files
    };
    let mut files: Vec<_> = paths.into_iter().filter_map(FileMatch::new).collect();
    files.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    Ok(files)
}

struct Choice {
    label: String,
    detail: String,
    replacement: String,
}

enum Catalog {
    Files(Vec<FileMatch>),
    Skills(Vec<ProviderSkill>),
}

impl Catalog {
    fn matches(&self, query: &str) -> Vec<Choice> {
        match self {
            Self::Files(files) => {
                let mut matches: Vec<_> = files
                    .iter()
                    .filter_map(|file| {
                        let offset = file.search_path.find(query)?;
                        let name = file.search_path.rsplit('/').next().unwrap_or_default();
                        let rank = match_rank(name, query).unwrap_or(3);
                        Some(((rank, offset, file.path.len(), &file.path), file))
                    })
                    .collect();
                if matches.len() > LIMIT {
                    matches.select_nth_unstable_by(LIMIT, |a, b| a.0.cmp(&b.0));
                    matches.truncate(LIMIT);
                }
                matches.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                matches
                    .into_iter()
                    .map(|(_, file)| Choice {
                        label: file.name.clone(),
                        detail: Path::new(&file.path)
                            .parent()
                            .and_then(Path::to_str)
                            .unwrap_or_default()
                            .into(),
                        replacement: format!("{} ", file.reference()),
                    })
                    .collect()
            }
            Self::Skills(skills) => {
                let query = query.strip_prefix("skill:").unwrap_or(query);
                let mut matches = Vec::new();
                for same_name in skills.chunk_by(|a, b| a.name == b.name) {
                    for skill in same_name {
                        let rank = match_rank(&skill.name.to_lowercase(), query).or_else(|| {
                            skill.description.as_ref().and_then(|description| {
                                description.to_lowercase().contains(query).then_some(3)
                            })
                        });
                        if let Some(rank) = rank
                            && let Some(choice) = skill_choice(skill, same_name.len() > 1)
                        {
                            matches.push((rank, choice));
                        }
                    }
                }
                matches.sort_by_key(|(rank, _)| *rank);
                matches
                    .into_iter()
                    .take(LIMIT)
                    .map(|(_, choice)| choice)
                    .collect()
            }
        }
    }
}

fn skill_choice(skill: &ProviderSkill, ambiguous: bool) -> Option<Choice> {
    let mention = format!("${}", skill.name);
    let (detail, replacement) = if ambiguous {
        // Codex resolves linked mentions by native path. Its parser ends the link at
        // the first closing parenthesis and does not decode an escaped destination.
        let path = skill.path.to_str()?;
        if path.contains(')') || path.chars().any(char::is_control) {
            return None;
        }
        let detail = skill
            .scope
            .as_ref()
            .map_or_else(|| path.to_owned(), |scope| format!("{scope} · {path}"));
        (detail, format!("[{mention}]({path}) "))
    } else {
        let detail = skill.description.as_ref().or(skill.scope.as_ref());
        (detail.cloned().unwrap_or_default(), format!("{mention} "))
    };
    Some(Choice {
        label: format!("/{}", skill.name),
        detail,
        replacement,
    })
}

fn match_rank(name: &str, query: &str) -> Option<u8> {
    if name == query {
        Some(0)
    } else if name.starts_with(query) {
        Some(1)
    } else {
        name.contains(query).then_some(2)
    }
}

struct Menu {
    trigger: Trigger,
    selected: usize,
    catalog: Result<Catalog, &'static str>,
    choices: Vec<Choice>,
    _load: Option<Task<()>>,
}

impl Menu {
    fn filter(&mut self) {
        self.selected = 0;
        self.choices = self
            .catalog
            .as_ref()
            .map(|catalog| catalog.matches(&self.trigger.query))
            .unwrap_or_default();
    }
}

#[derive(Default)]
pub(super) struct Suggestions {
    menu: Option<Menu>,
    dismissed: Option<Trigger>,
}

impl Desktop {
    pub(super) fn insert_file_reference(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = FileMatch::new(path.to_path_buf()) else {
            self.error = Some("Could not attach this path to the message.".into());
            cx.notify();
            return;
        };
        self.prompt.update(cx, |input, cx| {
            let value = input.value();
            let end = value.encode_utf16().count();
            let separator = if value.ends_with(char::is_whitespace) || value.is_empty() {
                ""
            } else {
                " "
            };
            let reference = format!("{separator}{} ", file.reference());
            input.replace_text_in_range(Some(end..end), &reference, window, cx);
        });
    }

    pub(super) fn observe_composer(
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.observe_in(input, window, |view, input, window, cx| {
            view.update_suggestions(&input, window, cx);
        })
        .detach();
        cx.subscribe_in(
            input,
            window,
            |view, input, event, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::Focus | InputEvent::Blur => view.update_suggestions(input, window, cx),
                _ => {}
            },
        )
        .detach();
    }

    fn composer_is_active(&self, input: &Entity<InputState>) -> bool {
        !self.show_settings
            && if self.current.is_some() {
                input == &self.prompt
            } else {
                !self.launch_setup && self.launching.is_none() && input == &self.starter
            }
    }

    fn trigger(input: &Entity<InputState>, window: &mut Window, cx: &mut App) -> Option<Trigger> {
        input.update(cx, |input, cx| {
            if !input.focus_handle(cx).is_focused(window)
                || input.marked_text_range(window, cx).is_some()
                || input
                    .selected_text_range(false, window, cx)
                    .is_some_and(|s| !s.range.is_empty())
            {
                return None;
            }
            Trigger::at(&input.value(), input.cursor())
        })
    }

    fn update_suggestions(
        &mut self,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.composer_is_active(input) {
            return;
        }
        let Some(trigger) = Self::trigger(input, window, cx) else {
            self.suggestions.dismissed = None;
            if self.suggestions.menu.take().is_some() {
                cx.notify();
            }
            return;
        };
        if self.suggestions.dismissed.as_ref() == Some(&trigger) {
            return;
        }
        self.suggestions.dismissed = None;
        if let Some(menu) = &mut self.suggestions.menu
            && menu.trigger.kind == trigger.kind
            && menu.trigger.start == trigger.start
        {
            if menu.trigger != trigger {
                menu.trigger = trigger;
                menu.filter();
                cx.notify();
            }
            return;
        }
        let (instance, checkout) = self.model_source(cx);
        let mut menu = Menu {
            selected: 0,
            choices: Vec::new(),
            catalog: Err(match trigger.kind {
                Kind::Files => "Searching files…",
                Kind::Skills => "Loading skills…",
            }),
            _load: None,
            trigger,
        };
        if menu.trigger.kind == Kind::Files && checkout.as_os_str().is_empty() {
            menu.catalog = Err("Choose a checkout to search files");
        } else {
            let kind = menu.trigger.kind;
            let starter = self.current.is_none();
            let mode = self.launch_workspace.requested_mode();
            let worktree = starter && mode == WorkspaceMode::Worktree;
            let selected = self
                .launch_workspace
                .selected_branch(mode)
                .filter(|_| starter)
                .map(|branch| branch.reference.clone());
            let state_dir = self.selection_file.parent().unwrap().to_owned();
            let runtime = self.runtime.clone();
            let job = cx.background_spawn(async move {
                match kind {
                    Kind::Files => {
                        let base = if worktree && selected.is_none() {
                            git::branches(&checkout)
                                .map_err(|_| "Could not read workspace files")?
                                .default
                                .map(|branch| branch.reference)
                        } else {
                            selected
                        };
                        checkout_files(&checkout, starter, base.as_deref())
                            .map(Catalog::Files)
                            .map_err(|_| "Could not read workspace files")
                    }
                    Kind::Skills => {
                        let cwd = if checkout.as_os_str().is_empty() {
                            state_dir
                        } else if starter {
                            git::checkout_root(&checkout)
                                .map_err(|_| "Could not locate checkout")?
                        } else {
                            checkout
                        };
                        runtime
                            .list_provider_skills(instance.as_str(), cwd)
                            .await
                            .map_err(|_| "Could not load provider skills")?
                            .map(Catalog::Skills)
                            .ok_or("Skill discovery is unavailable for this provider")
                    }
                }
            });
            // Closing or replacing the menu cancels delivery to the old menu.
            menu._load = Some(cx.spawn(async move |view, cx| {
                let result = job.await;
                let _ = view.update(cx, |view, cx| {
                    if let Some(menu) = &mut view.suggestions.menu {
                        menu.catalog = result;
                        menu.filter();
                        cx.notify();
                    }
                });
            }));
        }
        self.suggestions.menu = Some(menu);
        cx.notify();
    }

    fn move_suggestion(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let Some(menu) = &mut self.suggestions.menu else {
            return;
        };
        let count = menu.choices.len();
        if count != 0 {
            let step = if backwards { count - 1 } else { 1 };
            menu.selected = (menu.selected + step) % count;
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn accept_suggestion(
        &mut self,
        input: &Entity<InputState>,
        choice: Option<(Trigger, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(menu) = &self.suggestions.menu else {
            return false;
        };
        let replacement = match choice {
            Some((trigger, replacement)) if trigger == menu.trigger => replacement,
            Some(_) => return false,
            None => {
                let Some(choice) = menu.choices.get(menu.selected) else {
                    return false;
                };
                choice.replacement.clone()
            }
        };
        if !self.composer_is_active(input)
            || Self::trigger(input, window, cx).as_ref() != Some(&menu.trigger)
        {
            return false;
        }
        let menu = self.suggestions.menu.take().unwrap();
        input.update(cx, |input, cx| {
            let range = menu.trigger.utf16_range(&input.value());
            input.replace_text_in_range(Some(range), &replacement, window, cx);
        });
        cx.stop_propagation();
        cx.notify();
        true
    }

    pub(super) fn message_input(
        &self,
        _name: &'static str,
        input: &Entity<InputState>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        let mut field = div()
            .key_context(COMPOSER_CONTEXT)
            .relative()
            .capture_action({
                let input = input.clone();
                cx.listener(move |view, action: &Enter, window, cx| {
                    if action.secondary
                        || !input.focus_handle(cx).is_focused(window)
                        || view.accept_suggestion(&input, None, window, cx)
                    {
                        return;
                    }
                    cx.stop_propagation();
                    if input == view.starter {
                        if !input.read(cx).value().trim().is_empty()
                            || !view.starter_attachments.is_empty()
                        {
                            view.launch(window, cx);
                        }
                    } else {
                        let result = view.send(window, cx);
                        view.report(result, cx);
                    }
                })
            })
            .child(control!(_name, Input::new(input).appearance(false)));
        if input.read(cx).text().len() == 0 && !input.focus_handle(cx).is_focused(window) {
            let shortcut = window
                .highest_precedence_binding_for_action(&crate::FocusMessage)
                .and_then(|binding| {
                    binding
                        .keystrokes()
                        .first()
                        .map(|key| Kbd::format(key.as_keystroke()))
                });
            field = field.when_some(shortcut, |field, shortcut| {
                let text = SharedString::from(format!("{shortcut} to focus"));
                field.child(
                    control!(text.clone(), hint(text, cx).text_xs())
                        .absolute()
                        .top_1()
                        .right_2(),
                )
            });
        }
        let Some(menu) = &self.suggestions.menu else {
            return field;
        };
        let icon = match menu.trigger.kind {
            Kind::Files => IconName::File,
            Kind::Skills => IconName::BookOpen,
        };
        let mut popup = v_flex()
            .id("message-suggestions")
            .w(px(328.))
            .p_1()
            .popover_style(cx)
            .occlude();
        if menu.choices.is_empty() {
            let status = menu
                .catalog
                .as_ref()
                .err()
                .copied()
                .unwrap_or(match menu.trigger.kind {
                    Kind::Files => "No matching files",
                    Kind::Skills => "No matching skills",
                });
            popup = popup.child(control!(
                status,
                div().text_sm().px_2().py_1().child(status)
            ));
        } else {
            for (index, choice) in menu.choices.iter().enumerate() {
                let input = input.clone();
                let selection = (menu.trigger.clone(), choice.replacement.clone());
                let detail = choice.detail.clone();
                popup = popup.child(control!(
                    format!("suggestion/{}", choice.label),
                    h_flex()
                        .id(index)
                        .h_8()
                        .min_w_0()
                        .gap_2()
                        .px_2()
                        .rounded_md()
                        .text_sm()
                        .cursor_pointer()
                        .when(index == menu.selected, |row| row.bg(cx.theme().list_active))
                        .on_mouse_move(cx.listener(move |view, _, _, cx| {
                            if let Some(menu) = &mut view.suggestions.menu
                                && menu.selected != index
                            {
                                menu.selected = index;
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.accept_suggestion(&input, Some(selection.clone()), window, cx);
                        }))
                        .when(!detail.is_empty(), |row| {
                            row.tooltip(move |window, cx| {
                                Tooltip::new(detail.clone()).build(window, cx)
                            })
                        })
                        .child(Icon::new(icon.clone()).small().flex_shrink_0())
                        .child(
                            div()
                                .max_w(relative(0.65))
                                .truncate()
                                .child(choice.label.clone())
                        )
                        .child(
                            div()
                                .text_color(cx.theme().popover_foreground.opacity(0.72))
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(choice.detail.clone())
                        )
                ));
            }
        }
        field = field
            .capture_action(cx.listener(|view, _: &MoveUp, _, cx| view.move_suggestion(true, cx)))
            .capture_action(
                cx.listener(|view, _: &MoveDown, _, cx| view.move_suggestion(false, cx)),
            )
            .capture_action({
                let input = input.clone();
                cx.listener(move |view, _: &IndentInline, window, cx| {
                    view.accept_suggestion(&input, None, window, cx);
                })
            })
            .capture_action(cx.listener(|view, _: &Escape, _, cx| {
                if let Some(menu) = view.suggestions.menu.take() {
                    view.suggestions.dismissed = Some(menu.trigger);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                div().absolute().left_0().top_0().child(
                    deferred(
                        anchored()
                            .anchor(Corner::BottomLeft)
                            .offset(point(px(0.), px(-6.)))
                            .child(popup),
                    )
                    .with_priority(1),
                ),
            );
        field
    }
}
