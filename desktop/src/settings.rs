mod advanced;
mod environment;
mod models;
mod providers;
mod setup;

pub(crate) use setup::default_executable;

use crate::conversation::ProviderCatalog;
use crate::ui::{choice, heading, hint, label, section_title};
use advanced::{RuntimeFields, RuntimePreferences};
use anyhow::{Context as _, Result, ensure};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use huggler::{Huggler, Provider, ProviderInstanceId};
pub(crate) use models::ModelPreferences;
use providers::ProviderSettings;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
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
    pub codex_executable: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_agent_executable: Option<PathBuf>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    models: BTreeMap<String, ModelPreferences>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    runtime_overrides: BTreeMap<String, RuntimePreferences>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    provider_environment: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            codex_executable: None,
            claude_agent_executable: None,
            models: BTreeMap::new(),
            runtime_overrides: BTreeMap::new(),
            provider_environment: BTreeMap::new(),
        }
    }
}

impl Preferences {
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
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Overview,
    Appearance,
    Providers,
    Provider,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Settings",
            Self::Appearance => "Appearance",
            Self::Providers => "Providers",
            Self::Provider => "Provider",
        }
    }

    fn parent(self) -> Option<Self> {
        match self {
            Self::Overview => None,
            Self::Appearance | Self::Providers => Some(Self::Overview),
            Self::Provider => Some(Self::Providers),
        }
    }
}

pub struct Settings {
    page: Page,
    prefs: Preferences,
    state: PathBuf,
    runtime: Arc<Huggler>,
    input: Entity<InputState>,
    providers: ProviderSettings,
    model_catalog: Option<Entity<ProviderCatalog>>,
    catalog_subscription: Option<Subscription>,
    advanced: bool,
    runtime_fields: RuntimeFields,
    environment_rows: Option<Vec<environment::EnvironmentRow>>,
    pub focus: FocusHandle,
    error: Option<String>,
}

impl EventEmitter<SettingsEvent> for Settings {}

impl Settings {
    pub fn new(
        prefs: Preferences,
        state: PathBuf,
        runtime: Arc<Huggler>,
        load_error: Option<String>,
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
            prefs,
            state,
            runtime,
            input,
            providers,
            model_catalog: None,
            catalog_subscription: None,
            advanced: false,
            runtime_fields,
            environment_rows: None,
            focus: cx.focus_handle(),
            error: load_error,
        }
    }

    pub fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_page(Page::Overview, window, cx);
    }

    fn show_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == page || !self.leave_page(cx) {
            return;
        }
        if page == Page::Providers && self.page != Page::Provider {
            self.refresh_providers(cx);
        }
        self.page = page;
        self.focus.focus(window);
        cx.notify();
    }

    fn leave_page(&mut self, cx: &mut Context<Self>) -> bool {
        self.page != Page::Provider
            || ((!self.path_edited(cx) || self.save_path(cx))
                && (!self.runtime_edited(cx) || self.save_runtime(cx)))
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
                std::env::var_os(variable).is_none() && self.entered_path(cx) != *saved
            })
    }

    fn executable_preference(&self) -> Option<(&Option<PathBuf>, &'static str)> {
        match self.providers.selected.as_str() {
            "codex" => Some((&self.prefs.codex_executable, "HUGGLER_CODEX")),
            "claudeAgent" => Some((&self.prefs.claude_agent_executable, "HUGGLER_CLAUDE_ACP")),
            _ => None,
        }
    }

    fn load_executable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((saved, variable)) = self.executable_preference() {
            let value = std::env::var_os(variable)
                .map(PathBuf::from)
                .or_else(|| saved.clone())
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
        let path = self.entered_path(cx);
        if let Some(path) = &path
            && let Err(e) = validate_program(path)
        {
            self.error = Some(e.to_string());
            cx.notify();
            return false;
        }
        let mut prefs = self.prefs.clone();
        match self.providers.selected.as_str() {
            "codex" => prefs.codex_executable = path,
            "claudeAgent" => prefs.claude_agent_executable = path,
            _ => return false,
        }
        self.persist(prefs, cx)
    }
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
        control!(
            title,
            Button::new(title)
                .ghost()
                .relative()
                .w_full()
                .h(px(56.))
                .px_4()
                .justify_start()
                .child(
                    h_flex().gap_3().child(Icon::new(icon).small()).child(
                        v_flex().items_start().gap_0p5().child(label(title)).child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().foreground.opacity(0.7))
                                .child(description),
                        ),
                    ),
                )
                .child(
                    div()
                        .absolute()
                        .right_4()
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .child(Icon::new(IconName::ChevronRight).small()),
                )
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.show_page(page, window, cx);
                }))
        )
    }

    fn overview(cx: &Context<Self>) -> Div {
        v_flex()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(Self::section_link(
                Page::Appearance,
                IconName::Palette,
                "Light, dark, or system theme",
                cx,
            ))
            .child(div().border_t_1().border_color(cx.theme().border))
            .child(Self::section_link(
                Page::Providers,
                IconName::SquareTerminal,
                "Install and connect coding providers",
                cx,
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
        let managed = std::env::var_os(variable).is_some();
        let edited = self.path_edited(cx);
        let executable = &instance.executable;
        let path_hint = if managed {
            format!("{variable} sets this path at startup.")
        } else if edited {
            "Save the path to use it the next time Huggler starts.".into()
        } else if saved.clone().unwrap_or_else(|| self.automatic_executable()) != *executable {
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
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.page {
            Page::Overview => Self::overview(cx),
            Page::Appearance => self.appearance(cx),
            Page::Providers => self.render_providers(cx),
            Page::Provider => self.render_provider(cx),
        };
        let body = v_flex()
            .w_full()
            .max_w(px(720.))
            .p_5()
            .gap_3()
            .child(content)
            .when_some(self.error.clone(), |panel, error| {
                panel.child(control!(
                    format!("settings-error/{error}"),
                    div().text_sm().text_color(cx.theme().danger).child(error)
                ))
            });
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .child(
                h_flex()
                    .px_5()
                    .py_3()
                    .flex_shrink_0()
                    .justify_between()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .pl_8()
                            .min_w_0()
                            .gap_2()
                            .when_some(self.page.parent(), |row, parent| {
                                let back = if parent == Page::Providers {
                                    "Back to Providers"
                                } else {
                                    "Back to Settings"
                                };
                                row.child(control!(
                                    back,
                                    Button::new("back-to-settings")
                                        .ghost()
                                        .small()
                                        .icon(IconName::ChevronLeft)
                                        .when(parent == Page::Providers, |button| {
                                            button.label("Providers")
                                        })
                                        .tooltip(back)
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.show_page(parent, window, cx);
                                        }))
                                ))
                            })
                            .when(self.page != Page::Provider, |row| {
                                row.child(heading(self.page.title()).truncate())
                            }),
                    )
                    .child(
                        control!(
                            "Done",
                            Button::new("done").label("Done").on_click(cx.listener(
                                |v, _, _, cx| {
                                    if v.leave_page(cx) {
                                        v.page = Page::Overview;
                                        cx.emit(SettingsEvent::Back);
                                    }
                                }
                            ))
                        )
                        .flex_shrink_0(),
                    ),
            )
            .child(
                div()
                    .id(("settings-body", self.page as usize))
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}
