mod advanced;
mod catalog;
mod environment;
mod general;
#[cfg(unix)]
mod machines;
mod models;
mod project_actions;
pub(crate) mod projects;
mod providers;
mod setup;

pub(crate) use setup::default_executable;

use crate::conversation::ProviderCatalog;
use crate::ui::{choice, heading, hint, label, section_title};
use advanced::{RuntimeFields, RuntimePreferences};
use anyhow::{Context as _, Result, ensure};
pub(crate) use general::{FollowUp, SendShortcut};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use huggler::{
    Huggler, HugglerOptions, Provider, ProviderInstanceId,
    workspace::{OrphanedWorktreeOutcome, OrphanedWorktreeReport},
};
pub(crate) use models::ModelPreferences;
use providers::ProviderSettings;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    fn apply(self, window: &mut Window, cx: &mut App) {
        match self {
            Self::System => Theme::sync_system_appearance(Some(window), cx),
            Self::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Self::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    appearance: Appearance,
    #[serde(skip_serializing_if = "SendShortcut::is_default")]
    pub(crate) send_shortcut: SendShortcut,
    #[serde(skip_serializing_if = "FollowUp::is_default")]
    pub(crate) follow_up: FollowUp,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) agent_notifications: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) keep_awake: bool,
    pub codex_executable: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_agent_executable: Option<PathBuf>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    provider_executables: BTreeMap<String, PathBuf>,
    // Which instances this launch supplied, as opposed to the JSON configuration.
    #[serde(skip)]
    provider_presets: HashSet<ProviderInstanceId>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    models: BTreeMap<String, ModelPreferences>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    runtime_overrides: BTreeMap<String, RuntimePreferences>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    provider_environment: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    projects: Vec<projects::SavedProject>,
    #[cfg(unix)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) machines: Vec<crate::machines::SavedMachine>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    worktree_projects: BTreeMap<PathBuf, PathBuf>,
    pub automatic_worktree_cleanup: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            send_shortcut: SendShortcut::default(),
            follow_up: FollowUp::default(),
            agent_notifications: false,
            keep_awake: false,
            codex_executable: None,
            claude_agent_executable: None,
            provider_executables: BTreeMap::new(),
            provider_presets: HashSet::new(),
            models: BTreeMap::new(),
            runtime_overrides: BTreeMap::new(),
            provider_environment: BTreeMap::new(),
            projects: Vec::new(),
            #[cfg(unix)]
            machines: Vec::new(),
            worktree_projects: BTreeMap::new(),
            automatic_worktree_cleanup: false,
        }
    }
}

impl Preferences {
    pub(crate) fn configure_providers(
        &mut self,
        options: &mut HugglerOptions,
        state: &Path,
    ) -> Result<()> {
        self.provider_presets = catalog::register(options, self, state);
        self.apply_runtime_overrides(options)
    }

    fn set_executable(&mut self, id: &str, path: Option<PathBuf>) {
        match id {
            "codex" => self.codex_executable = path,
            "claudeAgent" => self.claude_agent_executable = path,
            _ => match path {
                Some(path) => {
                    self.provider_executables.insert(id.into(), path);
                }
                None => {
                    self.provider_executables.remove(id);
                }
            },
        }
    }

    pub fn load(state: &Path) -> Result<Self> {
        match fs::read(state.join("preferences.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    fn save(&self, state: &Path) -> Result<()> {
        let temp = tempfile::NamedTempFile::new_in(state)?;
        serde_json::to_writer_pretty(temp.as_file(), self)?;
        temp.persist(state.join("preferences.json"))?;
        Ok(())
    }
}

pub enum SettingsEvent {
    Back,
    ProviderModels(ProviderInstanceId, bool),
    #[cfg(unix)]
    OpenMachine(crate::machines::SavedMachine),
    #[cfg(unix)]
    ForgetMachine(crate::machines::SavedMachine),
}

pub(crate) enum SettingsTarget<'a> {
    Overview,
    Provider(&'a ProviderInstanceId),
    #[cfg(target_os = "macos")]
    ProjectActions(PathBuf),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Overview,
    General,
    Appearance,
    Providers,
    Projects,
    ProjectActions,
    #[cfg(unix)]
    Machines,
    Storage,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Settings",
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Providers => "Providers",
            Self::Projects => "Projects",
            Self::ProjectActions => "Project actions",
            Self::Storage => "Storage",
            #[cfg(unix)]
            Self::Machines => "Machines",
        }
    }

    fn parent(self) -> Option<Self> {
        match self {
            Self::Overview => None,
            Self::ProjectActions => Some(Self::Projects),
            _ => Some(Self::Overview),
        }
    }
}

pub struct Settings {
    page: Page,
    entry_page: Page,
    prefs: Preferences,
    state: PathBuf,
    runtime: Arc<Huggler>,
    input: Entity<InputState>,
    providers: ProviderSettings,
    project_editor: Option<(PathBuf, Entity<InputState>)>,
    action_project: PathBuf,
    action_editor: Option<project_actions::ActionEditor>,
    adding_project: bool,
    #[cfg(unix)]
    machines: machines::Fields,
    model_catalog: Option<Entity<ProviderCatalog>>,
    catalog_subscription: Option<Subscription>,
    advanced: bool,
    runtime_fields: RuntimeFields,
    environment_rows: Option<Vec<environment::EnvironmentRow>>,
    pub focus: FocusHandle,
    error: Option<String>,
    cleanup_running: bool,
    cleanup_status: Option<String>,
}

impl EventEmitter<SettingsEvent> for Settings {}

impl Settings {
    pub(crate) fn preferences(&self) -> &Preferences {
        &self.prefs
    }

    pub fn new(
        prefs: Preferences,
        state: PathBuf,
        runtime: Arc<Huggler>,
        load_error: Option<String>,
        _executor: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe(&input, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        prefs.appearance.apply(window, cx);
        cx.observe_window_appearance(window, |view: &mut Self, window, cx| {
            if view.prefs.appearance == Appearance::System {
                Appearance::System.apply(window, cx);
            }
        })
        .detach();
        let providers = ProviderSettings::new(Provider::Codex.key().into());
        let runtime_fields = RuntimeFields::new(window, cx);
        Self {
            page: Page::default(),
            entry_page: Page::default(),
            prefs,
            state,
            runtime,
            input,
            providers,
            project_editor: None,
            action_project: PathBuf::new(),
            action_editor: None,
            adding_project: false,
            #[cfg(unix)]
            machines: machines::Fields::new(_executor, window, cx),
            model_catalog: None,
            catalog_subscription: None,
            advanced: false,
            runtime_fields,
            environment_rows: None,
            focus: cx.focus_handle(),
            error: load_error,
            cleanup_running: false,
            cleanup_status: None,
        }
    }

    pub(crate) fn opened(
        &mut self,
        target: SettingsTarget<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            SettingsTarget::Overview => {
                self.show_page(Page::Overview, window, cx);
                if self.page != Page::Overview {
                    return;
                }
            }
            SettingsTarget::Provider(instance) => self.open_provider_settings(instance, window, cx),
            #[cfg(target_os = "macos")]
            SettingsTarget::ProjectActions(root) => self.open_project_actions(root, window, cx),
        }
        self.entry_page = self.page;
    }

    pub(crate) fn leave(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.save_provider_edits(cx) {
            return false;
        }
        self.project_editor = None;
        self.action_editor = None;
        self.page = Page::Overview;
        self.entry_page = Page::Overview;
        true
    }

    fn show_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == page || !self.save_provider_edits(cx) {
            return;
        }
        self.page = page;
        self.project_editor = None;
        self.action_editor = None;
        if page == Page::Providers {
            self.open_providers(window, cx);
        }
        self.focus.focus(window);
        cx.notify();
    }

    fn save_provider_edits(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.provider_details_open() {
            return true;
        }
        let mut prefs = self.prefs.clone();
        let edits = (|| {
            let path = self.apply_path_edit(&mut prefs, cx)?;
            let runtime = self.apply_runtime_edit(&mut prefs, cx)?;
            let environment = self.apply_environment_edits(&mut prefs, cx)?;
            Ok(path || runtime || environment)
        })();
        self.persist_provider_edits(prefs, edits, cx)
    }

    fn persist_provider_edits(
        &mut self,
        prefs: Preferences,
        edits: Result<bool>,
        cx: &mut Context<Self>,
    ) -> bool {
        match edits {
            Ok(true) => self.persist(prefs, cx),
            Ok(false) => true,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                false
            }
        }
    }

    fn persist(&mut self, prefs: Preferences, cx: &mut Context<Self>) -> bool {
        let result = prefs.save(&self.state);
        self.error = result
            .as_ref()
            .err()
            .map(|e| format!("Could not save settings. {e}"));
        cx.notify();
        if result.is_ok() {
            self.prefs = prefs;
            true
        } else {
            false
        }
    }

    fn entered_path(&self, cx: &App) -> Option<PathBuf> {
        let value = self.input.read(cx).value();
        let text = value.trim();
        (!text.is_empty()).then(|| PathBuf::from(text))
    }

    fn path_edited(&self, cx: &App) -> bool {
        self.executable_preference()
            .is_some_and(|(saved, variable)| {
                variable.and_then(std::env::var_os).is_none()
                    && self.entered_path(cx).as_deref() != saved
            })
    }

    fn executable_preference(&self) -> Option<(Option<&Path>, Option<&'static str>)> {
        let id = &self.providers.selected;
        match id.as_str() {
            "codex" => Some((
                self.prefs.codex_executable.as_deref(),
                Some("HUGGLER_CODEX"),
            )),
            "claudeAgent" => Some((
                self.prefs.claude_agent_executable.as_deref(),
                Some("HUGGLER_CLAUDE_ACP"),
            )),
            key if self.prefs.provider_presets.contains(id) => Some((
                self.prefs
                    .provider_executables
                    .get(key)
                    .map(PathBuf::as_path),
                None,
            )),
            _ => None,
        }
    }

    fn load_executable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((saved, variable)) = self.executable_preference() {
            let value = variable
                .and_then(std::env::var_os)
                .map(PathBuf::from)
                .or_else(|| saved.map(Path::to_owned))
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            let automatic = self.automatic_executable();
            let path = if automatic.is_absolute() {
                Some(automatic)
            } else {
                setup::on_path(&automatic).filter(|path| path.is_absolute())
            };
            let placeholder = path.map_or_else(
                || "Find automatically".into(),
                |path| path.to_string_lossy().into_owned(),
            );
            self.input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx);
                input.set_value(value, window, cx);
            });
        }
    }

    fn automatic_executable(&self) -> PathBuf {
        let driver = self.selected_provider().driver;
        if let Some(preset) = catalog::preset(driver) {
            return preset.executable(&self.state);
        }
        let name = match driver {
            Provider::Codex => huggler::providers::codex::CodexOptions::default().executable,
            Provider::ClaudeAgent => {
                huggler::providers::claude_agent::ClaudeAgentOptions::default().executable
            }
            _ => unreachable!("only built-in providers have executable preferences"),
        };
        default_executable(&self.state, driver, &name)
    }

    fn save_path(&mut self, cx: &mut Context<Self>) -> bool {
        let mut prefs = self.prefs.clone();
        let edit = self.apply_path_edit(&mut prefs, cx);
        self.persist_provider_edits(prefs, edit, cx)
    }

    fn apply_path_edit(&self, prefs: &mut Preferences, cx: &App) -> Result<bool> {
        if !self.path_edited(cx) {
            return Ok(false);
        }
        let path = self.entered_path(cx);
        if let Some(path) = &path {
            validate_program(path)?;
        }
        prefs.set_executable(self.providers.selected.as_str(), path);
        Ok(true)
    }

    fn cleanup_worktrees(&mut self, cx: &mut Context<Self>) {
        if self.cleanup_running {
            return;
        }
        self.cleanup_running = true;
        self.cleanup_status = None;
        let runtime = self.runtime.clone();
        let job = cx.background_spawn(async move { runtime.cleanup_orphaned_worktrees() });
        cx.spawn(async move |view, cx| {
            let result = job.await;
            let _ = view.update(cx, |view, cx| {
                view.cleanup_running = false;
                match result {
                    Ok(report) => {
                        view.cleanup_status = Some(cleanup_summary(&report));
                    }
                    Err(error) => {
                        view.error = Some(format!("Could not clean up worktrees. {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn cleanup_summary(report: &OrphanedWorktreeReport) -> String {
    let removed = report
        .entries
        .iter()
        .filter(|entry| entry.outcome == OrphanedWorktreeOutcome::Removed)
        .count();
    let kept = report
        .entries
        .iter()
        .filter(|entry| entry.outcome == OrphanedWorktreeOutcome::Kept)
        .count();
    let failed = report
        .entries
        .iter()
        .filter(|entry| entry.outcome == OrphanedWorktreeOutcome::Failed)
        .count();
    if report.entries.is_empty() {
        return "No unused worktrees found.".into();
    }
    let mut parts = vec![format!("Removed {removed}")];
    if kept > 0 {
        parts.push(format!("kept {kept} for review"));
    }
    if failed > 0 {
        parts.push(format!("retry later {failed}"));
    }
    format!("{}.", parts.join(", "))
}

fn validate_program(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "Enter the full path to the executable.");
    let metadata = fs::metadata(path)
        .with_context(|| format!("Could not open {}. Check the file path.", path.display()))?;
    ensure!(metadata.is_file(), "Choose an executable file.");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        ensure!(
            metadata.permissions().mode() & 0o111 != 0,
            "Choose a file with execute permission."
        );
    }
    Ok(())
}

impl Settings {
    fn section_link(
        page: Page,
        icon: IconName,
        description: &'static str,
        cx: &Context<Self>,
    ) -> Div {
        let title = page.title();
        let summary = control!(
            format!("settings-summary/{title}"),
            h_flex()
                .gap_3()
                .child(
                    h_flex()
                        .size_7()
                        .flex_shrink_0()
                        .justify_center()
                        .rounded_md()
                        .bg(cx.theme().secondary)
                        .child(Icon::new(icon).small()),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .items_start()
                        .gap_0p5()
                        .child(label(title))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .truncate()
                                .child(description),
                        ),
                )
        );
        div()
            .relative()
            .child(control!(
                title,
                Button::new(title)
                    .ghost()
                    .w_full()
                    .h(px(52.))
                    .pl_4()
                    .pr_12()
                    .justify_start()
                    .rounded_none()
                    .cursor_pointer()
                    .overflow_hidden()
                    .child(summary)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.show_page(page, window, cx);
                    }))
            ))
            .child(
                div()
                    .absolute()
                    .right_4()
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .text_color(cx.theme().muted_foreground)
                    .child(control!(
                        format!("settings-chevron/{title}"),
                        Icon::new(IconName::ChevronRight).small()
                    )),
            )
    }

    fn overview(cx: &Context<Self>) -> Div {
        let group = |name, entries: &[(Page, IconName, &'static str)]| {
            v_flex().gap_2().child(section_title(name)).child(
                v_flex()
                    .rounded_lg()
                    .overflow_hidden()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().sidebar)
                    .children(entries.iter().enumerate().map(|(i, (page, icon, detail))| {
                        Self::section_link(*page, icon.clone(), detail, cx).when(i > 0, |row| {
                            row.border_t_1().border_color(cx.theme().border)
                        })
                    })),
            )
        };
        v_flex()
            .gap_5()
            .child(group(
                "Preferences",
                &[
                    (
                        Page::General,
                        IconName::Settings2,
                        "Messaging, notifications, and sleep",
                    ),
                    (
                        Page::Appearance,
                        IconName::Palette,
                        "Light, dark, or system theme",
                    ),
                ],
            ))
            .child(group(
                "Workspace",
                &[
                    (
                        Page::Providers,
                        IconName::SquareTerminal,
                        "Connections, models, and setup",
                    ),
                    (
                        Page::Projects,
                        IconName::Folder,
                        "Manage Git checkouts saved for new conversations",
                    ),
                    #[cfg(unix)]
                    (
                        Page::Machines,
                        IconName::SquareTerminal,
                        "Connect to a Git checkout over SSH",
                    ),
                    (Page::Storage, IconName::Folder, "Clean up unused worktrees"),
                ],
            ))
    }

    fn appearance(&self, cx: &Context<Self>) -> Div {
        let appearance = [
            (Appearance::System, "System"),
            (Appearance::Light, "Light"),
            (Appearance::Dark, "Dark"),
        ];
        let theme_choices = appearance.into_iter().map(|(mode, name)| {
            control!(
                name,
                choice(name, name, self.prefs.appearance == mode).on_click(cx.listener(
                    move |v, _, w, cx| {
                        if v.persist(
                            Preferences {
                                appearance: mode,
                                ..v.prefs.clone()
                            },
                            cx,
                        ) {
                            mode.apply(w, cx);
                        }
                    }
                ))
            )
        });
        v_flex()
            .gap_2()
            .child(section_title("Color theme"))
            .child(h_flex().gap_1().flex_wrap().children(theme_choices))
    }

    fn executable_field(&self, installed: bool, cx: &Context<Self>) -> Div {
        let instance = self.selected_provider();
        if let Some(url) = &instance.server_url {
            return v_flex()
                .gap_1()
                .child(label("Server address"))
                .child(hint(url.clone(), cx));
        }
        let Some((saved, variable)) = self.executable_preference() else {
            return v_flex()
                .gap_1()
                .child(label("CLI executable"))
                .child(hint(instance.executable.display().to_string(), cx));
        };
        let override_key = variable.filter(|key| std::env::var_os(key).is_some());
        let managed = override_key.is_some();
        let edited = self.path_edited(cx);
        let executable = &instance.executable;
        let path_hint = if let Some(variable) = override_key {
            format!("{variable} sets this path at startup.")
        } else if edited {
            "Save the path to use it the next time Huggler starts.".into()
        } else if saved
            .map(Path::to_owned)
            .unwrap_or_else(|| self.automatic_executable())
            != *executable
        {
            format!(
                "Saved. Restart Huggler to apply. Currently using {}.",
                executable.display()
            )
        } else if saved.is_none() && installed {
            "The executable is found automatically.".into()
        } else if saved.is_none() {
            "To use an installation of your own, enter the full path.".into()
        } else {
            "Clear the path to let Huggler find the executable automatically.".into()
        };
        v_flex()
            .gap_1()
            .child(label("CLI executable"))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(control!(
                        "CLI executable",
                        Input::new(&self.input).disabled(managed)
                    )))
                    .when(!managed, |row| {
                        row.child(control!(
                            "Save path",
                            Button::new("save-path")
                                .label("Save path")
                                .disabled(!edited)
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.save_path(cx);
                                }))
                        ))
                    }),
            )
            .child(hint(path_hint, cx))
    }

    fn storage(&self, cx: &Context<Self>) -> Div {
        let automatic_cleanup = self.prefs.automatic_worktree_cleanup;
        v_flex()
            .gap_2()
            .child(section_title("Automatic cleanup"))
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .children([(false, "Off"), (true, "On")].map(|(enabled, name)| {
                        control!(
                            format!("automatic-cleanup/{name}"),
                            choice(name, name, automatic_cleanup == enabled)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if view.persist(
                                    Preferences {
                                        automatic_worktree_cleanup: enabled,
                                        ..view.prefs.clone()
                                    },
                                    cx,
                                ) && enabled
                                {
                                    view.cleanup_worktrees(cx);
                                }
                            }))
                        )
                    })),
            )
            .child(hint(
                "When on, Huggler checks at startup and hourly. Only unused worktrees without files or new commits are removed.",
                cx,
            ))
            .child(h_flex().mt_2().child(control!(
                "Clean up worktrees now",
                Button::new("cleanup-worktrees")
                    .small()
                    .label(if self.cleanup_running {
                        "Cleaning…"
                    } else {
                        "Clean up now"
                    })
                    .disabled(self.cleanup_running)
                    .on_click(cx.listener(|view, _, _, cx| view.cleanup_worktrees(cx)))
            )))
            .when_some(self.cleanup_status.clone(), |controls, status| {
                controls.child(control!(status.clone(), hint(status, cx)))
            })
    }
}

fn settings_column() -> Div {
    v_flex().w_full().max_w(px(720.)).mx_auto().px_5()
}

impl Settings {
    pub(super) fn panel(&self, navigation_toggle: Div, cx: &Context<Self>) -> Div {
        let content = match self.page {
            Page::Overview => Self::overview(cx),
            Page::General => self.general(cx),
            Page::Appearance => self.appearance(cx),
            Page::Providers => self.render_provider_panel(cx),
            Page::Projects => self.projects_panel(cx),
            Page::ProjectActions => self.project_actions_panel(cx),
            Page::Storage => self.storage(cx),
            #[cfg(unix)]
            Page::Machines => self.machines_panel(cx),
        };
        let scroll_id = match self.page {
            Page::Providers => self.provider_scroll_id(),
            _ => SharedString::from(self.page.title()),
        };
        let body = settings_column().py_4().gap_3().child(content).when_some(
            self.error.clone(),
            |panel, error| {
                panel.child(control!(
                    format!("settings-error/{error}"),
                    div().text_sm().text_color(cx.theme().danger).child(error)
                ))
            },
        );
        let parent = self.page.parent().filter(|_| self.page != self.entry_page);
        let back = match parent {
            Some(Page::Projects) => "Back to Projects",
            Some(_) => "Back to Settings",
            None => "Back to workspace",
        };
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .child(
                h_flex()
                    .px_5()
                    .py_2()
                    .flex_shrink_0()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(navigation_toggle.flex_shrink_0())
                    .child(control!(
                        back,
                        Button::new("settings-back")
                            .ghost()
                            .small()
                            .icon(IconName::ChevronLeft)
                            .tooltip(back)
                            .on_click(cx.listener(move |view, _, window, cx| match parent {
                                Some(page) => view.show_page(page, window, cx),
                                None => cx.emit(SettingsEvent::Back),
                            }))
                    ))
                    .child(heading(self.page.title()).truncate()),
            )
            .when(self.page == Page::Providers, |page| {
                page.child(self.provider_selector(cx))
            })
            .child(
                div()
                    .id(scroll_id)
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}
