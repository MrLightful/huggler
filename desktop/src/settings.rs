use crate::ui::{choice, heading, hint, label, section_title};
use anyhow::{Context as _, Result, ensure};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Theme, ThemeMode,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use huggler::{Huggler, ProviderInstanceId};
use serde::{Deserialize, Serialize};
use std::{
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
    pub provider: ProviderInstanceId,
    appearance: Appearance,
    pub codex_executable: Option<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            provider: "codex".into(),
            appearance: Appearance::System,
            codex_executable: None,
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
        let temp = state.join(format!("preferences-{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&temp, serde_json::to_vec_pretty(self)?)?;
        if let Err(error) = fs::rename(&temp, state.join("preferences.json")) {
            let _ = fs::remove_file(temp);
            return Err(error.into());
        }
        Ok(())
    }
}

pub enum SettingsEvent {
    Back,
    DefaultProvider(ProviderInstanceId),
}

pub struct Settings {
    prefs: Preferences,
    state: PathBuf,
    runtime: Arc<Huggler>,
    input: Entity<InputState>,
    env_override: bool,
    active_executable: PathBuf,
    pub focus: FocusHandle,
    error: Option<String>,
}

impl EventEmitter<SettingsEvent> for Settings {}

impl Settings {
    pub fn new(
        mut prefs: Preferences,
        state: PathBuf,
        runtime: Arc<Huggler>,
        load_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if !runtime.provider_instances().any(|p| p.id == prefs.provider) {
            prefs.provider = "codex".into();
        }
        let active_executable = runtime
            .provider_instances()
            .find(|p| p.id.as_str() == "codex")
            .expect("built-in Codex instance")
            .executable
            .clone();
        let env_override = std::env::var_os("HUGGLER_CODEX").map(PathBuf::from);
        let value = env_override
            .as_ref()
            .or(prefs.codex_executable.as_ref())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Use codex from PATH")
                .default_value(value)
        });
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
        Self {
            prefs,
            state,
            runtime,
            input,
            env_override: env_override.is_some(),
            active_executable,
            focus: cx.focus_handle(),
            error: load_error,
        }
    }

    pub fn default_provider(&self) -> ProviderInstanceId {
        self.prefs.provider.clone()
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

    fn save_path(&mut self, cx: &mut Context<Self>) -> bool {
        let path = self.entered_path(cx);
        if let Some(path) = &path
            && let Err(e) = validate_program(path)
        {
            self.error = Some(e.to_string());
            cx.notify();
            return false;
        }
        self.persist(
            Preferences {
                codex_executable: path,
                ..self.prefs.clone()
            },
            cx,
        )
    }
}

fn validate_program(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "Enter the full path to the Codex executable."
    );
    let metadata = fs::metadata(path)
        .with_context(|| format!("Could not open {}. Check the file path.", path.display()))?;
    ensure!(metadata.is_file(), "Choose the Codex executable file.");
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

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let providers = self.runtime.provider_instances().map(|instance| {
            let id = instance.id.clone();
            let name =
                SharedString::from(self.runtime.output().provider_label(id.as_str()).to_owned());
            control!(
                format!("default/{id}"),
                choice(
                    SharedString::from(format!("default-{id}")),
                    name,
                    self.prefs.provider == id
                )
                .on_click(cx.listener(move |v, _, _, cx| {
                    if v.persist(
                        Preferences {
                            provider: id.clone(),
                            ..v.prefs.clone()
                        },
                        cx,
                    ) {
                        cx.emit(SettingsEvent::DefaultProvider(id.clone()));
                    }
                }))
            )
        });
        let managed = self.env_override;
        let edited = !managed && self.entered_path(cx) != self.prefs.codex_executable;
        let saved = self
            .prefs
            .codex_executable
            .as_deref()
            .unwrap_or(Path::new("codex"));
        let path_hint = if managed {
            "HUGGLER_CODEX sets this path when the app starts.".into()
        } else if edited {
            "Save the path to use it the next time Huggler starts.".into()
        } else if saved != self.active_executable {
            format!(
                "Saved. Restart Huggler to apply. Currently using {}.",
                self.active_executable.display()
            )
        } else {
            "Leave blank to find codex on PATH. Path changes apply after restart.".into()
        };
        let path_field = v_flex()
            .gap_1()
            .child(label("Codex executable"))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(control!(
                        "Codex executable",
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
            .child(hint(path_hint, cx));
        let body = v_flex()
            .w_full()
            .max_w(px(720.))
            .p_5()
            .gap_5()
            .child(
                v_flex()
                    .gap_2()
                    .child(section_title("Appearance"))
                    .child(label("Color theme"))
                    .child(h_flex().gap_1().flex_wrap().children(theme_choices)),
            )
            .child(
                v_flex()
                    .gap_3()
                    .pt_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(section_title("Coding providers"))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(label("Default for new conversations"))
                            .child(h_flex().gap_1().flex_wrap().children(providers)),
                    )
                    .child(path_field),
            )
            .when_some(self.error.clone(), |body, error| {
                body.child(control!(
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
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(heading("Settings"))
                    .child(control!(
                        "Done",
                        Button::new("done").label("Done").on_click(cx.listener(
                            move |v, _, _, cx| {
                                if !edited || v.save_path(cx) {
                                    cx.emit(SettingsEvent::Back);
                                }
                            }
                        ))
                    )),
            )
            .child(
                div()
                    .id("settings-body")
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}
