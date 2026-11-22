use super::{Page, Settings, catalog, setup};
use crate::ui::{heading, label, section_title};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};
use huggler::{Provider, ProviderInstance, ProviderInstanceId};
use semver::Version;
use std::{collections::HashMap, path::Path};

#[derive(Clone, Copy)]
enum Status {
    Checking,
    Missing,
    Installed(setup::Account),
    Installing,
    Updating,
    ServerConfigured,
}

impl Status {
    fn busy(self) -> bool {
        matches!(self, Self::Checking | Self::Installing | Self::Updating)
    }

    fn detected(self) -> bool {
        !matches!(self, Self::Missing | Self::Installing)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Checking => "Checking setup…",
            Self::Missing => "Not installed",
            Self::Installing => "Installing…",
            Self::Updating => "Updating…",
            Self::Installed(setup::Account::SignedIn) => "Connected",
            Self::Installed(setup::Account::SignedOut) => "Sign-in needed",
            Self::Installed(setup::Account::Unknown) => "Installed",
            Self::ServerConfigured => "Server configured",
        }
    }

    fn dot(self, cx: &App) -> Div {
        let color = match self {
            Self::Installed(setup::Account::SignedIn) | Self::ServerConfigured => {
                cx.theme().success
            }
            Self::Missing | Self::Installed(setup::Account::SignedOut) => cx.theme().danger,
            _ => cx.theme().foreground.opacity(0.6),
        };
        div().size(px(6.)).flex_shrink_0().rounded_full().bg(color)
    }
}

struct SetupState {
    status: Status,
    version: Option<Version>,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProviderPanel {
    Empty,
    Choose,
    Details,
}

pub(super) struct ProviderSettings {
    pub(super) selected: ProviderInstanceId,
    panel: ProviderPanel,
    custom_executable: bool,
    reset_pending: bool,
    reset_needs_restart: bool,
    detected_scroll: ScrollHandle,
    setup: HashMap<ProviderInstanceId, SetupState>,
}

impl ProviderSettings {
    pub(super) fn new(selected: ProviderInstanceId) -> Self {
        Self {
            selected,
            panel: ProviderPanel::Empty,
            custom_executable: false,
            reset_pending: false,
            reset_needs_restart: false,
            detected_scroll: ScrollHandle::default(),
            setup: HashMap::new(),
        }
    }
}

impl Settings {
    pub(crate) fn provider_configured(&self, instance: &ProviderInstance) -> bool {
        !matches!(initial_status(&self.state, instance), Status::Missing)
            && !matches!(
                self.status(&instance.id),
                Status::Installed(setup::Account::SignedOut)
            )
    }

    pub(crate) fn refresh_provider_setup(&mut self, cx: &mut Context<Self>) {
        self.refresh_providers(cx);
    }

    pub(crate) fn open_provider_settings(
        &mut self,
        instance: &ProviderInstanceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_page(Page::Providers, window, cx);
        if self
            .runtime
            .provider_instances()
            .any(|provider| &provider.id == instance)
        {
            self.select_provider(instance, window, cx);
        } else {
            self.providers.panel = ProviderPanel::Choose;
            cx.notify();
        }
    }

    pub(super) fn selected_provider(&self) -> &ProviderInstance {
        self.runtime
            .provider_instances()
            .find(|instance| instance.id == self.providers.selected)
            .expect("settings selection is a registered provider")
    }

    fn provider_name(&self, id: &ProviderInstanceId) -> SharedString {
        self.runtime
            .output()
            .provider_label(id.as_str())
            .to_owned()
            .into()
    }

    pub(super) fn open_providers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_providers(cx);
        let current = &self.providers.selected;
        let selected = if self.provider_detected(current) {
            Some(current.clone())
        } else {
            self.ordered_providers()
                .into_iter()
                .find(|instance| self.provider_detected(&instance.id))
                .map(|instance| instance.id.clone())
        };
        if let Some(id) = selected {
            self.providers.selected = id;
            self.providers.panel = ProviderPanel::Details;
            self.load_provider(window, cx);
        } else {
            self.providers.panel = ProviderPanel::Empty;
        }
    }

    pub(super) fn provider_details_open(&self) -> bool {
        self.page == Page::Providers && self.providers.panel == ProviderPanel::Details
    }

    pub(super) fn provider_scroll_id(&self) -> SharedString {
        match self.providers.panel {
            ProviderPanel::Empty => "settings-provider-empty".into(),
            ProviderPanel::Choose => "settings-provider-picker".into(),
            ProviderPanel::Details => {
                format!("settings-provider/{}", self.providers.selected).into()
            }
        }
    }

    fn provider_detected(&self, id: &ProviderInstanceId) -> bool {
        self.providers
            .setup
            .get(id)
            .is_some_and(|state| state.status.detected())
    }

    fn refresh_providers(&mut self, cx: &mut Context<Self>) {
        let instances: Vec<_> = self.runtime.provider_instances().cloned().collect();
        for instance in instances {
            self.refresh_provider(instance, cx);
        }
    }

    fn refresh_provider(&mut self, instance: ProviderInstance, cx: &mut Context<Self>) {
        if self
            .providers
            .setup
            .get(&instance.id)
            .is_some_and(|state| state.status.busy())
        {
            return;
        }
        let status = initial_status(&self.state, &instance);
        let version = self
            .providers
            .setup
            .get(&instance.id)
            .and_then(|state| state.version.clone());
        self.providers.setup.insert(
            instance.id.clone(),
            SetupState {
                status,
                version: version.filter(|_| matches!(status, Status::Checking)),
                error: None,
            },
        );
        cx.notify();
        if !matches!(status, Status::Checking) {
            return;
        }
        let executor = cx.background_executor().clone();
        let directory = self.state.clone();
        let job = cx.background_spawn(async move {
            let state = read_setup(&directory, &instance, &executor).await;
            (instance.id, state)
        });
        cx.spawn(async move |view, cx| {
            let (id, state) = job.await;
            let _ = view.update(cx, |view, cx| {
                view.providers.setup.insert(id, state);
                cx.notify();
            });
        })
        .detach();
    }

    fn provider_update(&self) -> Option<&'static str> {
        let instance = self.selected_provider();
        self.executable_preference()?;
        let state = self.providers.setup.get(&instance.id)?;
        if !matches!(state.status, Status::Installed(_)) {
            return None;
        }
        setup::Release::for_instance(&self.state, instance)?.update_version(state.version.as_ref()?)
    }

    fn install_provider(&mut self, update: bool, cx: &mut Context<Self>) {
        let instance = self.selected_provider().clone();
        let Some(release) = setup::Release::for_instance(&self.state, &instance) else {
            return;
        };
        let allowed = if update {
            self.provider_update().is_some()
        } else {
            matches!(self.status(&instance.id), Status::Missing)
        };
        if !allowed {
            return;
        }
        let Some(state) = self.providers.setup.get_mut(&instance.id) else {
            return;
        };
        state.status = if update {
            Status::Updating
        } else {
            Status::Installing
        };
        state.error = None;
        cx.notify();
        let directory = self.state.clone();
        let executor = cx.background_executor().clone();
        let job = cx.background_spawn(async move {
            let result = release.install(&directory, update);
            let mut state = read_setup(&directory, &instance, &executor).await;
            state.error = result.err().map(|error| error.to_string());
            (instance.id, state)
        });
        cx.spawn(async move |view, cx| {
            let (id, state) = job.await;
            let _ = view.update(cx, |view, cx| {
                let load_models = view.provider_details_open()
                    && view.providers.selected == id
                    && matches!(state.status, Status::Installed(_));
                view.providers.setup.insert(id, state);
                cx.notify();
                if load_models {
                    view.request_models(true, cx);
                }
            });
        })
        .detach();
    }

    fn status(&self, id: &ProviderInstanceId) -> Status {
        self.providers
            .setup
            .get(id)
            .map_or(Status::Checking, |state| state.status)
    }

    fn load_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_executable(window, cx);
        self.load_runtime_fields(window, cx);
        self.advanced = false;
        self.providers.custom_executable = false;
        self.providers.reset_pending = false;
        self.providers.reset_needs_restart = false;
        self.environment_rows = None;
        self.model_catalog = None;
        self.catalog_subscription = None;
        let instance = self.selected_provider();
        if instance.driver != Provider::OpenCode
            && setup::executable_available(&self.state, instance)
        {
            self.request_models(false, cx);
        }
    }

    fn select_provider(
        &mut self,
        id: &ProviderInstanceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if (self.provider_details_open() && self.providers.selected == *id)
            || !self.save_provider_edits(cx)
        {
            return;
        }
        self.providers.selected.clone_from(id);
        self.providers.panel = ProviderPanel::Details;
        self.error = None;
        self.load_provider(window, cx);
        self.focus.focus(window);
        cx.notify();
    }

    fn ordered_providers(&self) -> Vec<&ProviderInstance> {
        let mut instances: Vec<_> = self.runtime.provider_instances().collect();
        instances.sort_by_key(|instance| match instance.id.as_str() {
            "codex" => 0,
            "claudeAgent" => 1,
            id => catalog::PRESETS
                .iter()
                .position(|preset| preset.driver.key() == id)
                .map_or(catalog::PRESETS.len() + 2, |index| index + 2),
        });
        instances
    }

    pub(super) fn provider_selector(&self, cx: &Context<Self>) -> Div {
        let add_selected = self.providers.panel == ProviderPanel::Choose
            || (self.provider_details_open() && !self.provider_detected(&self.providers.selected));
        let detected = self
            .ordered_providers()
            .into_iter()
            .filter(|instance| self.provider_detected(&instance.id))
            .map(|instance| self.provider_card(instance, cx).flex_none().w(px(168.)))
            .collect::<Vec<_>>();
        let add = control!(
            "Add provider",
            Button::new("add-provider")
                .outline()
                .h(px(72.))
                .w_full()
                .px_3()
                .justify_start()
                .cursor_pointer()
                .selected(add_selected)
                .when(add_selected, |button| button.border_color(cx.theme().ring))
                .child(
                    h_flex()
                        .gap_3()
                        .child(Icon::new(IconName::Plus).size(px(20.)))
                        .child(label("Add provider"))
                )
                .on_click(cx.listener(|view, _, window, cx| {
                    if view.providers.panel != ProviderPanel::Choose && view.save_provider_edits(cx)
                    {
                        view.providers.panel = ProviderPanel::Choose;
                        view.error = None;
                        view.focus.focus(window);
                        cx.notify();
                    }
                }))
        )
        .w(px(168.))
        .flex_shrink_0();
        div().w_full().max_w(px(720.)).px_5().flex_shrink_0().child(
            h_flex()
                .id("provider-selector")
                .w_full()
                .gap_2()
                .py_3()
                .border_b_1()
                .border_color(cx.theme().border)
                .when(!detected.is_empty(), |selector| {
                    selector.child(
                        div()
                            .min_w_0()
                            .relative()
                            .child(
                                h_flex()
                                    .id("detected-providers")
                                    .gap_2()
                                    .overflow_x_scroll()
                                    .track_scroll(&self.providers.detected_scroll)
                                    .children(detected),
                            )
                            .horizontal_scrollbar(&self.providers.detected_scroll),
                    )
                })
                .child(add),
        )
    }

    fn provider_card(&self, instance: &ProviderInstance, cx: &Context<Self>) -> Div {
        let id = instance.id.clone();
        let status = self.status(&id);
        let failed = self
            .providers
            .setup
            .get(&id)
            .is_some_and(|state| state.error.is_some());
        let selected = self.provider_details_open() && self.providers.selected == id;
        let name = self.provider_name(&id);
        let status_label = if failed {
            "Setup failed"
        } else if matches!(status, Status::Missing) {
            "Installation needed"
        } else {
            status.label()
        };
        let summary = h_flex()
            .gap_3()
            .child(provider_logo(instance.driver, px(20.), cx))
            .child(
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(label(name.clone()).truncate())
                    .child(control!(
                        format!("provider-card-status/{id}/{status_label}"),
                        h_flex()
                            .gap_1p5()
                            .child(status.dot(cx).when(failed, |dot| dot.bg(cx.theme().danger)))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(status_label),
                            )
                    )),
            );
        control!(
            format!("provider/{id}"),
            div()
                .relative()
                .child(
                    Button::new(SharedString::from(format!("provider-{id}")))
                        .outline()
                        .h(px(72.))
                        .w_full()
                        .px_3()
                        .justify_start()
                        .overflow_hidden()
                        .cursor_pointer()
                        .selected(selected)
                        .when(selected, |button| button
                            .pr_9()
                            .border_color(cx.theme().ring))
                        .tooltip(name)
                        .child(summary)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.select_provider(&id, window, cx);
                        })),
                )
                .when(selected, |card| {
                    card.child(
                        div()
                            .absolute()
                            .right_3()
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .child(Icon::new(IconName::Check).small()),
                    )
                })
        )
        .flex_1()
        .min_w(px(152.))
        .max_w(px(336.))
    }

    pub(super) fn render_provider_panel(&self, cx: &Context<Self>) -> Div {
        match self.providers.panel {
            ProviderPanel::Empty => control!(
                "No providers detected",
                hint("No providers detected. Add a provider to get started.", cx)
            ),
            ProviderPanel::Choose => self.provider_picker(cx),
            ProviderPanel::Details => self.render_provider(cx),
        }
    }

    fn provider_picker(&self, cx: &Context<Self>) -> Div {
        let choices: Vec<_> = self
            .ordered_providers()
            .into_iter()
            .filter(|instance| !self.provider_detected(&instance.id))
            .collect();
        v_flex()
            .gap_3()
            .child(control!(
                "Choose a provider",
                section_title("Choose a provider")
            ))
            .when(choices.is_empty(), |panel| {
                panel.child(control!(
                    "All providers detected",
                    hint("All available providers are already shown.", cx)
                ))
            })
            .child(
                div().grid().grid_cols(2).gap_2().children(
                    choices
                        .into_iter()
                        .map(|instance| self.provider_card(instance, cx)),
                ),
            )
    }

    fn render_provider(&self, cx: &Context<Self>) -> Div {
        let instance = self.selected_provider();
        let id = &instance.id;
        let status = self.status(id);
        let installed = matches!(status, Status::Installed(_));
        let sign_in = match status {
            Status::Installed(account) if account != setup::Account::SignedIn => {
                sign_in_actions(instance)
            }
            _ => None,
        };
        v_flex()
            .gap_4()
            .child(self.provider_heading(instance, status, cx))
            .children(sign_in)
            .when(self.providers.reset_pending, |panel| {
                panel.child(self.reset_confirmation(cx))
            })
            .when(self.providers.reset_needs_restart, |panel| {
                panel.child(hint(
                    "Defaults restored. Restart Huggler to apply launch changes.",
                    cx,
                ))
            })
            .when(matches!(status, Status::Missing), |page| {
                page.child(self.missing_provider(instance, cx))
            })
            .when(
                installed || matches!(status, Status::ServerConfigured),
                |page| {
                    page.child(self.render_models(cx))
                        .when(installed, |page| page.child(self.render_environment(cx)))
                        .child(self.render_advanced(installed, cx))
                },
            )
    }

    fn provider_heading(
        &self,
        instance: &ProviderInstance,
        status: Status,
        cx: &Context<Self>,
    ) -> Div {
        let setup = self.providers.setup.get(&instance.id);
        let error = setup.and_then(|state| state.error.as_deref());
        h_flex()
            .justify_between()
            .flex_wrap()
            .items_start()
            .gap_3()
            .child(
                h_flex()
                    .min_w_0()
                    .gap_3()
                    .items_start()
                    .child(
                        div()
                            .mt_1()
                            .child(provider_logo(instance.driver, px(24.), cx)),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .min_w_0()
                                    .flex_wrap()
                                    .child(
                                        control!(
                                            "Provider title",
                                            heading(self.provider_name(&instance.id))
                                                .text_2xl()
                                                .truncate()
                                        )
                                        .min_w_0(),
                                    )
                                    .child(self.provider_indicator(status, setup, cx)),
                            )
                            .when_some(error, |heading, error| {
                                heading.child(control!(
                                    format!("provider-setup-error/{error}"),
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().danger)
                                        .child(error.to_owned())
                                ))
                            })
                            .when(
                                error.is_none()
                                    && (status.busy()
                                        || matches!(
                                            status,
                                            Status::Installed(setup::Account::SignedOut)
                                        )),
                                |heading| {
                                    heading.child(control!(
                                        "provider-status-message",
                                        hint(status.label(), cx)
                                    ))
                                },
                            ),
                    ),
            )
            .when(
                !matches!(status, Status::Missing | Status::Installing),
                |row| row.child(self.provider_actions(status, cx)),
            )
    }

    fn provider_indicator(
        &self,
        status: Status,
        setup: Option<&SetupState>,
        cx: &Context<Self>,
    ) -> Div {
        let indicator = if let Some(version) = self.provider_update() {
            let label = format!("Update to v{version}");
            control!(
                label.clone(),
                Button::new("update-provider")
                    .outline()
                    .small()
                    .icon(IconName::ArrowUp)
                    .tooltip(label)
                    .on_click(cx.listener(|view, _, _, cx| view.install_provider(true, cx)))
            )
        } else {
            let failed = setup.is_some_and(|state| state.error.is_some());
            let description = if failed {
                "Setup failed"
            } else {
                status.label()
            };
            control!(
                description,
                status
                    .dot(cx)
                    .size(px(8.))
                    .when(failed, |dot| dot.bg(cx.theme().danger))
                    .id("provider-status")
                    .tooltip(move |window, cx| Tooltip::new(description).build(window, cx))
            )
        };
        h_flex()
            .flex_shrink_0()
            .gap_2()
            .when_some(
                setup.and_then(|state| state.version.as_ref()),
                |row, version| {
                    let version = format!("v{version}");
                    row.child(control!(
                        version.clone(),
                        hint(version, cx).max_w(px(100.)).truncate()
                    ))
                },
            )
            .child(indicator)
    }

    fn provider_actions(&self, status: Status, cx: &Context<Self>) -> Div {
        let has_overrides = self.has_provider_overrides(cx);
        h_flex()
            .gap_1()
            .child(control!(
                "Refresh setup",
                Button::new("refresh-setup")
                    .ghost()
                    .small()
                    .icon(Icon::default().path("icons/provider-refresh.svg"))
                    .tooltip("Refresh")
                    .disabled(status.busy())
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.refresh_providers(cx);
                        view.request_models(true, cx);
                    }))
            ))
            .when(
                self.executable_preference().is_some() || has_overrides,
                |row| {
                    row.child(control!(
                        "Reset to defaults",
                        Button::new("reset-provider")
                            .ghost()
                            .small()
                            .icon(IconName::Undo2)
                            .tooltip("Reset to defaults")
                            .disabled(
                                status.busy() || self.providers.reset_pending || !has_overrides
                            )
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.providers.reset_pending = true;
                                view.providers.reset_needs_restart = false;
                                cx.notify();
                            }))
                    ))
                },
            )
    }

    fn has_provider_overrides(&self, cx: &App) -> bool {
        let id = self.providers.selected.as_str();
        self.executable_preference()
            .is_some_and(|(path, _)| path.is_some())
            || self.path_edited(cx)
            || self.runtime_edited(cx)
            || self.prefs.runtime_overrides.contains_key(id)
            || self.prefs.models.contains_key(id)
            || self.prefs.provider_environment.contains_key(id)
    }

    fn reset_confirmation(&self, cx: &Context<Self>) -> Div {
        v_flex()
            .gap_2()
            .child(section_title("Reset provider settings?"))
            .child(hint(
                "Restore the default settings and model order for this provider.",
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(control!(
                        "Cancel reset",
                        Button::new("cancel-reset")
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.providers.reset_pending = false;
                                cx.notify();
                            }))
                    ))
                    .child(control!(
                        "Restore defaults",
                        Button::new("confirm-reset")
                            .small()
                            .primary()
                            .label("Restore defaults")
                            .on_click(
                                cx.listener(|view, _, window, cx| view.reset_provider(window, cx))
                            )
                    )),
            )
    }

    fn reset_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.providers.selected.as_str();
        let mut prefs = self.prefs.clone();
        if self.executable_preference().is_some() {
            prefs.set_executable(id, None);
        }
        prefs.models.remove(id);
        prefs.runtime_overrides.remove(id);
        prefs.provider_environment.remove(id);
        if self.persist(prefs, cx) {
            self.load_executable(window, cx);
            self.load_runtime_fields(window, cx);
            self.providers.reset_pending = false;
            self.environment_rows = None;
            let instance = self.selected_provider();
            self.providers.reset_needs_restart = !instance.environment().is_empty()
                || self.executable_preference().is_some_and(|(_, key)| {
                    (self.runtime_editable()
                        && (instance.home.is_some() || !instance.args.is_empty()))
                        || (key.and_then(std::env::var_os).is_none()
                            && instance.executable != self.automatic_executable())
                });
        }
    }

    fn missing_provider(&self, instance: &ProviderInstance, cx: &Context<Self>) -> Div {
        let automatic = setup::Release::for_instance(&self.state, instance).is_some();
        let install = if automatic {
            let button = format!("Install {}", self.provider_name(&instance.id));
            Some(control!(
                button.clone(),
                Button::new("install-provider")
                    .primary()
                    .icon(IconName::ArrowDown)
                    .label(button)
                    .on_click(cx.listener(|view, _, _, cx| view.install_provider(false, cx)))
            ))
        } else {
            setup_guide(instance.driver).map(|url| guide_button("Installation guide", url))
        };
        let custom = self.providers.custom_executable;
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .children(install)
                    .child(control!(
                        "Custom executable",
                        Button::new("custom-executable")
                            .outline()
                            .label("Custom executable")
                            .icon(if custom {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.providers.custom_executable =
                                    !view.providers.custom_executable;
                                cx.notify();
                            }))
                    )),
            )
            .when(self.prefs.provider_presets.contains(&instance.id), |page| {
                page.child(hint("Install and sign in with the provider’s CLI, then restart Huggler to find the installation.", cx))
            })
            .when(custom, |page| {
                page.child(
                    v_flex()
                        .gap_2()
                        .max_w(px(480.))
                        .child(self.executable_field(false, cx))
                        .when(self.executable_preference().is_none(), |details| {
                            details.child(hint(
                                "Set the executable in HUGGLER_PROVIDERS, then restart Huggler.",
                                cx,
                            ))
                        }),
                )
            })
    }
}

fn provider_logo(provider: Provider, size: Pixels, cx: &App) -> AnyElement {
    let logo = match provider {
        Provider::Codex => "provider-logos/codex.svg",
        Provider::ClaudeAgent => "provider-logos/claude.svg",
        _ => {
            return Icon::new(IconName::SquareTerminal)
                .size(size)
                .into_any_element();
        }
    };
    svg()
        .path(logo)
        .size(size)
        .flex_shrink_0()
        .text_color(cx.theme().foreground)
        .into_any_element()
}

fn setup_guide(driver: Provider) -> Option<&'static str> {
    match driver {
        Provider::Codex => Some(setup::CODEX_GUIDE),
        Provider::ClaudeAgent => Some(setup::CLAUDE_GUIDE),
        _ => catalog::preset(driver).map(|preset| preset.guide),
    }
}

fn initial_status(directory: &Path, instance: &ProviderInstance) -> Status {
    if instance.server_url.is_some() {
        Status::ServerConfigured
    } else if setup::executable_available(directory, instance) {
        Status::Checking
    } else {
        Status::Missing
    }
}

async fn read_setup(
    directory: &Path,
    instance: &ProviderInstance,
    executor: &BackgroundExecutor,
) -> SetupState {
    let initial = initial_status(directory, instance);
    let (status, version) = if matches!(initial, Status::Checking) {
        let (account, version) = futures_lite::future::zip(
            setup::account(instance, executor),
            setup::version(instance, executor),
        )
        .await;
        (Status::Installed(account), version)
    } else {
        (initial, None)
    };
    SetupState {
        status,
        version,
        error: None,
    }
}

fn sign_in_actions(instance: &ProviderInstance) -> Option<Div> {
    match instance.driver {
        Provider::Codex => Some(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(control!(
                    "Copy sign-in command",
                    Button::new("copy-sign-in-command")
                        .small()
                        .icon(IconName::Copy)
                        .label("Copy sign-in command")
                        .on_click({
                            let command = setup::codex_login_command(instance);
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(command.clone()))
                            }
                        })
                ))
                .child(guide_button("Codex sign-in guide", setup::CODEX_AUTH_GUIDE)),
        ),
        Provider::ClaudeAgent => Some(h_flex().child(guide_button(
            "Claude credentials guide",
            setup::CLAUDE_AUTH_GUIDE,
        ))),
        driver => {
            setup_guide(driver).map(|url| h_flex().child(guide_button("Provider setup guide", url)))
        }
    }
}

fn guide_button(title: &'static str, url: &'static str) -> Div {
    control!(
        title,
        Button::new(title)
            .ghost()
            .small()
            .icon(IconName::ExternalLink)
            .label(title)
            .on_click(move |_, _, cx| cx.open_url(url))
    )
}

fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    crate::ui::hint(text, cx).text_color(cx.theme().foreground.opacity(0.75))
}
