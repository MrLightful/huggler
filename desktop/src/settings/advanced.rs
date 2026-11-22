use super::{Preferences, Settings};
use crate::ui::{hint, label};
use anyhow::{Result, ensure};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, InputState},
    v_flex,
};
use huggler::{HugglerOptions, Provider, ProviderInstance};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub(super) struct RuntimePreferences {
    home: Option<PathBuf>,
    args: Vec<String>,
}

impl RuntimePreferences {
    fn input_values(&self) -> (String, String) {
        let home = self
            .home
            .as_ref()
            .map_or_else(String::new, |path| path.display().to_string());
        let args = shlex::try_join(self.args.iter().map(String::as_str)).unwrap_or_default();
        (home, args)
    }
}

impl Preferences {
    pub(crate) fn apply_runtime_overrides(&self, options: &mut HugglerOptions) -> Result<()> {
        let mut codex = ProviderInstance::new("codex", Provider::Codex, &options.codex.executable);
        codex.request_approvals = options.codex.request_approvals;
        codex.workspace_write = options.codex.workspace_write;
        let claude = ProviderInstance::new(
            "claudeAgent",
            Provider::ClaudeAgent,
            &options.claude_agent.executable,
        );
        for mut instance in [codex, claude] {
            let id = instance.id.as_str();
            let runtime = self.runtime_overrides.get(id);
            if let Some(value) = runtime {
                instance.home.clone_from(&value.home);
                instance.args.clone_from(&value.args);
            }
            if runtime.is_some() || self.provider_environment.contains_key(id) {
                options.instances.push(instance);
            }
        }
        for instance in &mut options.instances {
            if instance.server_url.is_none()
                && let Some(values) = self.provider_environment.get(instance.id.as_str())
            {
                instance.set_environment(values.clone())?;
            }
        }
        Ok(())
    }
}

pub(super) struct RuntimeFields {
    home: Entity<InputState>,
    args: Entity<InputState>,
}

impl RuntimeFields {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Settings>) -> Self {
        let home = cx.new(|cx| InputState::new(window, cx));
        let args = cx.new(|cx| InputState::new(window, cx).placeholder("No additional arguments"));
        for field in [&home, &args] {
            cx.subscribe(field, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }
        Self { home, args }
    }

    fn values(&self, cx: &App) -> (String, String) {
        (
            self.home.read(cx).value().trim().to_owned(),
            self.args.read(cx).value().trim().to_owned(),
        )
    }
}

impl Settings {
    fn saved_runtime(&self) -> RuntimePreferences {
        self.prefs
            .runtime_overrides
            .get(self.providers.selected.as_str())
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn load_runtime_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (home, args) = self.saved_runtime().input_values();
        let placeholder = default_config_directory(self.selected_provider().driver).map_or_else(
            || "Use the CLI default".into(),
            |path| path.to_string_lossy().into_owned(),
        );
        self.runtime_fields.home.update(cx, |field, cx| {
            field.set_placeholder(placeholder, window, cx);
            field.set_value(home, window, cx);
        });
        self.runtime_fields
            .args
            .update(cx, |field, cx| field.set_value(args, window, cx));
    }

    pub(super) fn runtime_edited(&self, cx: &App) -> bool {
        if !self.runtime_editable() {
            return false;
        }
        let (home, args) = self.runtime_fields.values(cx);
        parse_runtime(&home, &args).map_or(true, |value| value != self.saved_runtime())
    }

    fn save_runtime(&mut self, cx: &mut Context<Self>) {
        let mut prefs = self.prefs.clone();
        let edit = self.apply_runtime_edit(&mut prefs, cx);
        self.persist_provider_edits(prefs, edit, cx);
    }

    pub(super) fn apply_runtime_edit(&mut self, prefs: &mut Preferences, cx: &App) -> Result<bool> {
        if !self.runtime_editable() {
            return Ok(false);
        }
        let (home, args) = self.runtime_fields.values(cx);
        let saved = parse_runtime(&home, &args).inspect_err(|_| self.advanced = true)?;
        if saved == self.saved_runtime() {
            return Ok(false);
        }
        let id = self.providers.selected.as_str();
        if saved == RuntimePreferences::default() {
            prefs.runtime_overrides.remove(id);
        } else {
            prefs.runtime_overrides.insert(id.to_owned(), saved);
        }
        Ok(true)
    }

    pub(super) fn runtime_editable(&self) -> bool {
        matches!(self.providers.selected.as_str(), "codex" | "claudeAgent")
    }

    pub(super) fn render_advanced(&self, installed: bool, cx: &Context<Self>) -> Div {
        v_flex()
            .pt_3()
            .gap_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(h_flex().child(control!(
                    "Advanced settings",
                    Button::new("advanced-settings")
                        .ghost()
                        .label("Advanced settings")
                        .icon(if self.advanced {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.advanced = !view.advanced;
                            cx.notify();
                        }))
                )))
            .when(self.advanced, |section| {
                section.child(self.executable_field(installed, cx)).child(
                    if self.runtime_editable() {
                        self.runtime_editor(cx)
                    } else {
                        self.configured_runtime(cx)
                    },
                )
            })
    }

    fn runtime_editor(&self, cx: &Context<Self>) -> Div {
        let instance = self.selected_provider();
        let home = home_label(instance.driver)
            .expect("both built-in providers support a config directory");
        let saved = self.saved_runtime();
        let pending = instance.home != saved.home || instance.args != saved.args;
        v_flex()
            .gap_3()
            .child(crate::ui::field(home, &self.runtime_fields.home))
            .child(crate::ui::field(
                "Launch arguments",
                &self.runtime_fields.args,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(control!(
                        "Save runtime settings",
                        Button::new("save-runtime-settings")
                            .small()
                            .label("Save runtime settings")
                            .disabled(!self.runtime_edited(cx))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.save_runtime(cx);
                            }))
                    ))
                    .when(pending, |row| {
                        row.child(hint("Restart Huggler to apply runtime settings.", cx))
                    }),
            )
    }

    fn configured_runtime(&self, cx: &Context<Self>) -> Div {
        let instance = self.selected_provider();
        v_flex()
            .gap_3()
            .when_some(
                instance.home.as_ref().zip(home_label(instance.driver)),
                |fields, (path, home)| {
                    fields.child(
                        v_flex()
                            .gap_1()
                            .child(label(home))
                            .child(hint(path.display().to_string(), cx)),
                    )
                },
            )
            .when(!instance.args.is_empty(), |fields| {
                fields.child(
                    v_flex()
                        .gap_1()
                        .child(label("Launch arguments"))
                        .child(hint(
                            shlex::try_join(instance.args.iter().map(String::as_str))
                                .unwrap_or_default(),
                            cx,
                        )),
                )
            })
            .when_some(instance.password_env.as_ref(), |fields, variable| {
                fields.child(
                    v_flex()
                        .gap_1()
                        .child(label("Server password variable"))
                        .child(hint(variable.clone(), cx)),
                )
            })
            .when(self.executable_preference().is_none(), |fields| {
                fields.child(hint("Edit HUGGLER_PROVIDERS to change this instance.", cx))
            })
    }
}

fn default_config_directory(provider: Provider) -> Option<PathBuf> {
    let (variable, directory) = match provider {
        Provider::Codex => ("CODEX_HOME", ".codex"),
        Provider::ClaudeAgent => ("CLAUDE_CONFIG_DIR", ".claude"),
        _ => return None,
    };
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(directory)))
        .filter(|path| path.is_absolute())
}

fn home_label(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Codex => Some("Codex config directory"),
        Provider::ClaudeAgent => Some("Claude config directory"),
        Provider::Pi => Some("Pi config directory"),
        _ => None,
    }
}

fn parse_runtime(home: &str, args: &str) -> Result<RuntimePreferences> {
    ensure!(
        !home.contains('\0') && !args.contains('\0'),
        "Remove null characters from the runtime settings."
    );
    let home = if home.is_empty() {
        None
    } else {
        let path = PathBuf::from(home);
        ensure!(
            path.is_absolute(),
            "Enter the full path to the config directory."
        );
        ensure!(
            !path.exists() || path.is_dir(),
            "Choose a config directory."
        );
        Some(path)
    };
    let args = shlex::split(args)
        .ok_or_else(|| anyhow::anyhow!("Check the quotes in Launch arguments."))?;
    Ok(RuntimePreferences { home, args })
}
