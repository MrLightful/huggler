use super::*;
use crate::settings::ModelPreferences;
use gpui_component::{
    IconName, IndexPath, Sizable,
    list::{List, ListDelegate, ListEvent, ListItem, ListState},
    popover::Popover,
    switch::Switch,
};
use huggler::{ModelDetails, ProviderReport};

const AUTOMATIC: &str = "Automatic";
const PICKER_WIDTH: Pixels = px(300.);
const OPTION_PADDING: Pixels = px(12.);

pub(crate) struct ProviderCatalog {
    instance: ProviderInstanceId,
    checkout: PathBuf,
    pub(crate) result: Option<huggler::Result<ProviderReport>>,
}

impl ProviderCatalog {
    pub(super) fn loading(&self) -> bool {
        self.result.is_none()
    }

    fn login_required(&self) -> bool {
        matches!(&self.result, Some(Err(huggler::Error::LoginRequired)))
    }

    fn report(&self) -> Option<&ProviderReport> {
        self.result.as_ref()?.as_ref().ok()
    }

    pub(super) fn status(&self) -> String {
        match &self.result {
            None => "Checking provider…".into(),
            Some(Err(error)) => error.to_string(),
            Some(Ok(report)) => format!(
                "{} · {} · {} · {} models",
                report.transport,
                report.version.as_deref().unwrap_or("version unknown"),
                if report.authenticated == Some(true) {
                    "signed in"
                } else {
                    "login unverified"
                },
                report.models.len(),
            ),
        }
    }

    fn load(&mut self, runtime: Arc<Huggler>, cx: &mut Context<Self>) {
        let instance = self.instance.clone();
        let checkout = self.checkout.clone();
        self.result = None;
        cx.notify();
        let job = cx.background_spawn(async move {
            runtime.probe_provider(instance.as_str(), checkout).await
        });
        cx.spawn(async move |catalog, cx| {
            let result = job.await;
            let _ = catalog.update(cx, |catalog, cx| {
                catalog.result = Some(result);
                cx.notify();
            });
        })
        .detach();
    }
}

pub(super) struct Models {
    catalog: Option<Entity<ProviderCatalog>>,
    current: Option<String>,
    effort: Option<String>,
    query: String,
    matches: Vec<Option<String>>,
    preferences: ModelPreferences,
}

fn effort_label(effort: &str) -> String {
    match effort {
        "xhigh" => "Extra high".into(),
        _ => {
            let mut chars = effort.chars();
            chars
                .next()
                .into_iter()
                .flat_map(char::to_uppercase)
                .chain(chars)
                .collect()
        }
    }
}

fn model_details<'a>(
    report: Option<&'a ProviderReport>,
    model: Option<&str>,
) -> Option<&'a ModelDetails> {
    let models = &report?.model_details;
    match model {
        Some(id) => models.get(id),
        None => models.values().find(|model| model.is_default),
    }
}

fn model_name<'a>(model: Option<&'a str>, details: Option<&'a ModelDetails>) -> &'a str {
    details.map_or_else(|| model.unwrap_or(AUTOMATIC), |details| &details.name)
}

impl Models {
    fn report<'a>(&self, cx: &'a App) -> Option<&'a ProviderReport> {
        self.catalog.as_ref()?.read(cx).report()
    }

    fn filter(&mut self, cx: &App) {
        let report = self.report(cx);
        let models = report.map(|report| self.preferences.ordered(&report.models));
        self.matches = std::iter::once(None)
            .take(usize::from(self.catalog.is_some()))
            .chain(
                models
                    .into_iter()
                    .flatten()
                    .filter(|id| self.preferences.enabled(id))
                    .map(Some),
            )
            .filter(|id| {
                model_name(*id, model_details(report, *id))
                    .to_lowercase()
                    .contains(&self.query)
                    || id.is_some_and(|id| id.to_lowercase().contains(&self.query))
                    || (id.is_none() && "default".contains(&self.query))
            })
            .map(|id| id.map(str::to_owned))
            .collect();
    }
}

impl ListDelegate for Models {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.matches.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let choice = self.matches.get(ix.row)?;
        let details = model_details(self.report(cx), choice.as_deref());
        let name = model_name(choice.as_deref(), details);
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        let control_key = choice.as_ref().map_or_else(
            || "model-option-automatic".into(),
            |id| format!("model-option/{id}"),
        );
        let effort = if self.current == *choice {
            self.effort.as_deref()
        } else {
            None
        };
        let effort = effort.or_else(|| details?.default_effort.as_deref());
        Some(
            ListItem::new(ix.row)
                .check_icon(IconName::Check)
                .confirmed(self.current == *choice)
                .child(control!(
                    control_key.clone(),
                    h_flex()
                        .gap_2()
                        .min_w_0()
                        .child(
                            control!(
                                format!("{control_key}/name/{name}"),
                                div().truncate().child(name.to_owned())
                            )
                            .min_w_0()
                        )
                        .when_some(effort, |row, effort| {
                            row.child(hint(effort_label(effort), cx).flex_shrink_0())
                        })
                        .when(choice.is_none() && details.is_some(), |row| {
                            row.child(hint("Default", cx).flex_shrink_0())
                        })
                )),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        hint(
            if self.catalog.is_some() {
                "No matching models"
            } else {
                "No providers configured"
            },
            cx,
        )
        .p_2()
    }

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.query = query.to_lowercase();
        self.filter(cx);
        Task::ready(())
    }

    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
    }
}

impl Desktop {
    pub(super) fn model_source(&self, cx: &App) -> (ProviderInstanceId, PathBuf) {
        match &self.current {
            Some(thread) => (
                thread.provider_instance.clone(),
                self.workspaces[&thread.id].root.clone(),
            ),
            None => (self.instance.clone(), self.launch_checkout(cx)),
        }
    }

    pub(super) fn preload_models(&mut self, cx: &mut Context<Self>) {
        let source = self.picker_source(cx);
        if !self.model_catalogs.contains_key(&source)
            && source.1.is_dir()
            && self
                .configured_providers(cx)
                .any(|provider| provider.id == source.0)
        {
            self.catalog_for(source, false, cx);
        }
    }

    fn picker_source(&self, cx: &App) -> (ProviderInstanceId, PathBuf) {
        let (instance, checkout) = self.model_source(cx);
        let checkout = if checkout.as_os_str().is_empty() {
            self.selection_file
                .parent()
                .expect("state directory")
                .to_owned()
        } else {
            checkout
        };
        (instance, checkout)
    }

    fn configured_providers<'a>(
        &'a self,
        cx: &'a App,
    ) -> impl Iterator<Item = &'a huggler::ProviderInstance> {
        let (_, checkout) = self.picker_source(cx);
        self.runtime.provider_instances().filter(move |instance| {
            self.settings.read(cx).provider_configured(instance)
                && !self
                    .model_catalogs
                    .get(&(instance.id.clone(), checkout.clone()))
                    .is_some_and(|catalog| catalog.read(cx).login_required())
        })
    }

    pub(super) fn catalog(
        &mut self,
        refresh: bool,
        cx: &mut Context<Self>,
    ) -> Entity<ProviderCatalog> {
        self.catalog_for(self.model_source(cx), refresh, cx)
    }

    pub(super) fn catalog_for(
        &mut self,
        source: (ProviderInstanceId, PathBuf),
        refresh: bool,
        cx: &mut Context<Self>,
    ) -> Entity<ProviderCatalog> {
        if let Some(catalog) = self.model_catalogs.get(&source) {
            let state = catalog.read(cx);
            if !state.loading() && (refresh || state.login_required()) {
                catalog.update(cx, |state, cx| state.load(self.runtime.clone(), cx));
            }
            return catalog.clone();
        }
        let catalog = cx.new(|cx| {
            let mut catalog = ProviderCatalog {
                instance: source.0.clone(),
                checkout: source.1.clone(),
                result: None,
            };
            catalog.load(self.runtime.clone(), cx);
            catalog
        });
        cx.observe(&catalog, |_, _, cx| cx.notify()).detach();
        self.model_catalogs.insert(source, catalog.clone());
        cx.notify();
        catalog
    }

    fn model_selection(&self) -> (Option<&str>, &ModelOptions) {
        match &self.current {
            Some(thread) => (thread.model.as_deref(), &thread.model_options),
            None => (self.launch_model.as_deref(), &self.launch_model_options),
        }
    }

    pub(super) fn model_control(&self, cx: &Context<Self>) -> Div {
        let (model, options) = self.model_selection();
        let source = self.picker_source(cx);
        let catalog = self
            .model_catalogs
            .get(&source)
            .map(|catalog| catalog.read(cx));
        let details = model_details(catalog.and_then(ProviderCatalog::report), model);
        let effort = options
            .effort
            .as_deref()
            .or_else(|| details?.default_effort.as_deref());
        let model_label = model.map(|id| {
            if self.runtime.provider_instances().any(|provider| {
                provider.id == source.0
                    && matches!(provider.driver, Provider::OpenCode | Provider::Pi)
            }) {
                id.split_once('/').map_or(id, |(_, name)| name)
            } else {
                id
            }
        });
        let name = model_name(model_label, details);
        let trigger = Button::new("choose-model")
            .small()
            .ghost()
            .dropdown_caret(true)
            .tooltip("Choose model and effort")
            .child(
                h_flex()
                    .gap_1()
                    .min_w_0()
                    .child(div().max_w(px(176.)).truncate().child(name.to_owned()))
                    .when_some(effort, |row, effort| {
                        row.child(hint(effort_label(effort), cx).flex_shrink_0())
                    }),
            );
        let picker = Popover::new("model-picker")
            .anchor(Corner::BottomLeft)
            .trigger(trigger)
            .open(self.model_picker.is_some())
            .on_open_change(cx.listener(|view, open, window, cx| {
                if *open {
                    view.open_models(window, cx);
                } else {
                    let picker = view.model_picker.take();
                    view.show_model_efforts = false;
                    // The list handles Escape and returns focus to the composer.
                    // The setup-only popover has no list to handle it.
                    if picker.is_some_and(|list| list.read(cx).delegate().catalog.is_none()) {
                        view.focus_workspace(window, cx);
                    }
                    cx.notify();
                }
            }));
        let picker = if let Some(list) = &self.model_picker {
            let models = list.read(cx).delegate();
            let configured: Vec<_> = self.configured_providers(cx).collect();
            let active_configured = configured.iter().any(|provider| provider.id == source.0);
            let provider_choices = configured.iter().map(|instance| {
                let id = instance.id.clone();
                let name = self.runtime.output().provider_label(id.as_str()).to_owned();
                control!(
                    format!("provider/{name}"),
                    Button::new(SharedString::from(format!("provider-{id}")))
                        .small()
                        .ghost()
                        .selected(id == source.0)
                        .label(name)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            if id != view.instance {
                                view.choose_provider(&id, cx);
                                view.open_models(window, cx);
                            }
                        }))
                )
            });
            let mut content = v_flex().w(PICKER_WIDTH).gap_1();
            if self.current.is_none() {
                content = content.child(
                    div()
                        .id("model-picker-providers")
                        .max_h(px(96.))
                        .overflow_y_scroll()
                        .child(h_flex().gap_1().flex_wrap().children(provider_choices)),
                );
            }
            let instance = source.0;
            let edit = control!(
                "Edit",
                Button::new("edit-provider-settings")
                    .small()
                    .ghost()
                    .icon(IconName::Settings)
                    .tooltip("Edit provider settings")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.open_settings(window, cx);
                        view.settings.update(cx, |settings, cx| {
                            settings.open_provider_settings(&instance, window, cx);
                        });
                    }))
            )
            .flex_shrink_0();
            let (message, retry) = match (active_configured, catalog.map(|catalog| &catalog.result))
            {
                (false, _) if self.current.is_none() && !configured.is_empty() => {
                    (Some("Choose a provider to load models.".into()), false)
                }
                (false, _) => (Some("Connect a provider in Settings.".into()), false),
                (_, None | Some(None)) => (Some("Loading models…".into()), false),
                (_, Some(Some(Err(error)))) => (Some(error.to_string()), true),
                (_, Some(Some(Ok(report)))) if report.models.is_empty() => {
                    (Some("This provider listed no models.".into()), true)
                }
                _ => (None, false),
            };
            let status = message.map(|message| {
                h_flex()
                    .gap_2()
                    .px_1()
                    .justify_between()
                    .child(control!(
                        format!("model-status/{message}"),
                        hint(message, cx).min_w_0()
                    ))
                    .when(retry, |row| {
                        row.child(control!(
                            "Retry models",
                            Button::new("retry-models")
                                .small()
                                .ghost()
                                .label("Retry")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.catalog_for(view.picker_source(cx), true, cx);
                                }))
                        ))
                    })
            });
            if active_configured {
                let height = (models.matches.len().max(1) as f32 * 32. + 36.).min(168.);
                content = content
                    .child(
                        h_flex()
                            .items_start()
                            .gap_1()
                            .child(
                                control!(
                                    "Model search and results",
                                    List::new(list)
                                        .search_placeholder("Search models")
                                        .h(px(height))
                                )
                                .flex_1()
                                .min_w_0(),
                            )
                            .child(edit.mt_1()),
                    )
                    .children(status);
            } else {
                content = content.child(
                    h_flex()
                        .gap_1()
                        .child(v_flex().flex_1().min_w_0().children(status))
                        .child(edit),
                );
            }
            if active_configured
                && let Some(details) = details
                && (!details.efforts.is_empty() || details.fast_tier.is_some())
            {
                content = content.child(self.model_options(details, options, cx));
            }
            picker.child(content)
        } else {
            picker
        };
        control!(
            model.map_or_else(|| "model-automatic".into(), |id| format!("model/{id}")),
            picker
        )
    }

    fn model_options(
        &self,
        details: &ModelDetails,
        options: &ModelOptions,
        cx: &Context<Self>,
    ) -> Div {
        let current_effort = options
            .effort
            .as_deref()
            .or(details.default_effort.as_deref());
        let mut section = v_flex()
            .gap_1()
            .border_t_1()
            .border_color(cx.theme().border)
            .pt_1();
        if !details.efforts.is_empty() {
            section = section.child(control!(
                "Effort",
                Button::new("model-effort")
                    .small()
                    .ghost()
                    .w_full()
                    .px(OPTION_PADDING)
                    .justify_start()
                    .child(
                        h_flex()
                            .w(PICKER_WIDTH - OPTION_PADDING * 2.)
                            .justify_between()
                            .gap_2()
                            .child("Effort")
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(hint(
                                        current_effort
                                            .map(effort_label)
                                            .unwrap_or_else(|| "Default".into()),
                                        cx
                                    ))
                                    .child(
                                        Icon::new(if self.show_model_efforts {
                                            IconName::ChevronUp
                                        } else {
                                            IconName::ChevronDown
                                        })
                                        .size(px(14.))
                                    )
                            )
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.show_model_efforts = !view.show_model_efforts;
                        cx.notify();
                    }))
            ));
        }
        if self.show_model_efforts {
            let efforts = std::iter::once(None).chain(details.efforts.iter().map(Some));
            section = section.child(h_flex().flex_wrap().gap_1().children(efforts.map(|effort| {
                let key = effort.map_or("default", String::as_str);
                let id = SharedString::from(format!("effort/{key}"));
                let selected = options.effort.as_ref() == effort;
                let label =
                    effort.map_or_else(|| "Use default".into(), |value| effort_label(value));
                let effort = effort.cloned();
                control!(
                    id.clone(),
                    Button::new(id)
                        .small()
                        .outline()
                        .selected(selected)
                        .label(label)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.update_model_options(
                                |options| options.effort.clone_from(&effort),
                                cx,
                            );
                            view.show_model_efforts = false;
                            cx.notify();
                        }))
                )
            })));
        }
        if let Some(tier) = &details.fast_tier {
            let speed = options
                .service_tier
                .as_deref()
                .or(details.default_service_tier.as_deref());
            let enabled = speed.is_some_and(|tier| matches!(tier, "fast" | "priority"));
            let tier = tier.clone();
            section = section.child(control!(
                if enabled {
                    "Fast mode on"
                } else {
                    "Fast mode off"
                },
                Button::new("model-speed")
                    .small()
                    .ghost()
                    .w_full()
                    .px(OPTION_PADDING)
                    .justify_start()
                    .child(
                        h_flex()
                            .w(PICKER_WIDTH - OPTION_PADDING * 2.)
                            .justify_between()
                            .gap_2()
                            .child("Fast mode")
                            .child(Switch::new("model-fast-mode").small().checked(enabled))
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.update_model_options(
                            |options| {
                                options.service_tier = Some(if enabled {
                                    "default".into()
                                } else {
                                    tier.clone()
                                });
                            },
                            cx,
                        );
                    }))
            ));
        }
        section
    }

    fn update_model_options(
        &mut self,
        update: impl FnOnce(&mut ModelOptions),
        cx: &mut Context<Self>,
    ) {
        let (model, options) = self.model_selection();
        let mut options = options.clone();
        update(&mut options);
        self.set_model_selection(model.map(str::to_owned), options, cx);
    }

    fn set_model_selection(
        &mut self,
        model: Option<String>,
        options: ModelOptions,
        cx: &mut Context<Self>,
    ) {
        if let Some(thread) = &self.current {
            let result = self
                .runtime
                .dispatch(Command {
                    id: CommandId(Uuid::new_v4().to_string()),
                    thread_id: thread.id.clone(),
                    action: Action::SetModelSelection { model, options },
                })
                .and_then(|_| self.refresh());
            self.report(result, cx);
        } else {
            self.launch_model = model;
            self.launch_model_options = options;
            cx.notify();
        }
        if let Some(list) = &self.model_picker {
            let effort = self.model_selection().1.effort.clone();
            list.update(cx, |list, cx| {
                list.delegate_mut().effort = effort;
                cx.notify();
            });
        }
    }

    fn open_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_model_efforts = false;
        let source = self.picker_source(cx);
        let configured = self
            .configured_providers(cx)
            .any(|provider| provider.id == source.0);
        let catalog = configured.then(|| self.catalog_for(source.clone(), false, cx));
        let thread_id = self.current.as_ref().map(|t| t.id.clone());
        let (current, options) = self.model_selection();
        let current = current.map(str::to_owned);
        let effort = options.effort.clone();
        let preferences = self.settings.read(cx).model_preferences(&source.0);
        let list = cx.new(|cx| {
            let mut models = Models {
                catalog: catalog.clone(),
                current,
                effort,
                query: String::new(),
                matches: Vec::new(),
                preferences,
            };
            models.filter(cx);
            if let Some(catalog) = &catalog {
                cx.observe(catalog, |list: &mut ListState<Models>, _, cx| {
                    list.delegate_mut().filter(cx);
                    cx.notify();
                })
                .detach();
            }
            ListState::new(models, window, cx).searchable(true)
        });
        cx.subscribe_in(&list, window, move |view, list, event, window, cx| {
            if view.model_picker.as_ref() != Some(list) {
                return;
            }
            if matches!(event, ListEvent::Cancel) {
                view.model_picker = None;
                view.focus_workspace(window, cx);
                cx.notify();
                return;
            }
            let ListEvent::Confirm(ix) = event else {
                return;
            };
            let models = list.read(cx).delegate();
            let Some(model) = models.matches.get(ix.row).cloned() else {
                return;
            };
            let same_context = view.picker_source(cx) == source
                && view.current.as_ref().map(|t| &t.id) == thread_id.as_ref();
            view.model_picker = None;
            view.focus_workspace(window, cx);
            cx.notify();
            if same_context && view.model_selection().0 != model.as_deref() {
                let options = ModelOptions::default();
                view.set_model_selection(model, options, cx);
            }
        })
        .detach();
        if configured {
            list.focus_handle(cx).focus(window);
        }
        self.model_picker = Some(list);
        cx.notify();
    }
}
