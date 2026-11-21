use super::*;
use gpui_component::{
    IconName, IndexPath, Sizable,
    list::{List, ListDelegate, ListEvent, ListItem, ListState},
    popover::Popover,
};
use huggler::ProviderReport;

const DEFAULT: &str = "Provider default";

pub(super) struct ProviderCatalog {
    instance: ProviderInstanceId,
    checkout: PathBuf,
    result: Option<Result<ProviderReport, String>>,
}

impl ProviderCatalog {
    pub(super) fn loading(&self) -> bool {
        self.result.is_none()
    }

    pub(super) fn status(&self) -> String {
        match &self.result {
            None => "Checking provider…".into(),
            Some(Err(error)) => error.clone(),
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
            runtime
                .probe_provider(instance.as_str(), checkout)
                .await
                .map_err(|error| error.to_string())
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
    query: String,
    matches: Vec<Option<String>>,
}

impl Models {
    fn filter(&mut self, cx: &App) {
        let report = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.read(cx).result.as_ref())
            .and_then(|r| r.as_ref().ok());
        self.matches = std::iter::once(None)
            .chain(report.into_iter().flat_map(|r| r.models.iter().map(Some)))
            .filter(|id| {
                id.map_or(DEFAULT, String::as_str)
                    .to_lowercase()
                    .contains(&self.query)
            })
            .map(|id| id.cloned())
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
        _: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let choice = self.matches.get(ix.row)?;
        let title = choice.as_deref().unwrap_or(DEFAULT);
        Some(
            ListItem::new(ix.row)
                .check_icon(IconName::Check)
                .confirmed(self.current == *choice)
                .child(control!(
                    format!("model-option/{title}"),
                    div().truncate().child(title.to_owned())
                )),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        hint("No matching models", cx).p_2()
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
            None => (
                self.instance.clone(),
                PathBuf::from(self.checkout.read(cx).value().trim()),
            ),
        }
    }

    pub(super) fn preload_models(&mut self, cx: &mut Context<Self>) {
        let source = self.model_source(cx);
        if !self.model_catalogs.contains_key(&source) && source.1.is_dir() {
            self.catalog(false, cx);
        }
    }

    pub(super) fn catalog(
        &mut self,
        refresh: bool,
        cx: &mut Context<Self>,
    ) -> Entity<ProviderCatalog> {
        let source = self.model_source(cx);
        if let Some(catalog) = self.model_catalogs.get(&source) {
            if refresh && !catalog.read(cx).loading() {
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

    pub(super) fn model_control(&self, cx: &Context<Self>) -> Div {
        let (instance, model) = match &self.current {
            Some(thread) => (&thread.provider_instance, &thread.model),
            None => (&self.instance, &self.launch_model),
        };
        let provider = self
            .runtime
            .output()
            .provider_label(instance.as_str())
            .to_owned();
        let trigger = Button::new("choose-model")
            .small()
            .ghost()
            .dropdown_caret(true)
            .tooltip("Provider and model")
            .child(div().max_w(px(240.)).truncate().child(format!(
                "{provider} · {}",
                model.as_deref().unwrap_or("Default")
            )));
        let picker = Popover::new("model-picker")
            .anchor(Corner::BottomLeft)
            .trigger(trigger)
            .open(self.model_picker.is_some())
            .on_open_change(cx.listener(|view, open, window, cx| {
                if *open {
                    view.open_models(window, cx);
                } else {
                    view.model_picker = None;
                    cx.notify();
                }
            }));
        let picker = if let Some(list) = &self.model_picker {
            let mut content = v_flex().w(px(288.)).gap_2();
            if self.current.is_none() {
                let providers = self.runtime.provider_instances().map(|instance| {
                    let id = instance.id.clone();
                    let name = self.runtime.output().provider_label(id.as_str()).to_owned();
                    control!(
                        format!("provider/{name}"),
                        Button::new(SharedString::from(format!("provider-{id}")))
                            .small()
                            .ghost()
                            .selected(id == self.instance)
                            .label(name)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                if id != view.instance {
                                    view.choose_provider(&id, cx);
                                    view.open_models(window, cx);
                                }
                            }))
                    )
                });
                content = content.child(h_flex().gap_1().flex_wrap().children(providers));
            } else {
                content = content.child(label(provider));
            }
            content = content.child(
                List::new(list)
                    .search_placeholder("Search models")
                    .h(px(190.)),
            );
            let catalog = &list.read(cx).delegate().catalog;
            let (message, retry) = match catalog {
                None => (Some("Choose a checkout to load models.".into()), false),
                Some(catalog) => match &catalog.read(cx).result {
                    None => (Some("Loading models…".into()), false),
                    Some(Err(error)) => (Some(error.clone()), true),
                    Some(Ok(report)) if report.models.is_empty() => {
                        (Some("This provider listed no models.".into()), true)
                    }
                    Some(Ok(_)) => (None, false),
                },
            };
            if let Some(message) = message {
                content = content.child(control!(
                    format!("model-status/{message}"),
                    hint(message, cx)
                ));
            }
            if retry {
                content = content.child(control!(
                    "Retry models",
                    Button::new("retry-models")
                        .small()
                        .label("Retry")
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.catalog(true, cx);
                        }))
                ));
            }
            picker.child(content)
        } else {
            picker
        };
        control!(
            format!("model/{}", model.as_deref().unwrap_or(DEFAULT)),
            picker
        )
    }

    fn open_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.model_source(cx);
        let catalog = (!source.1.as_os_str().is_empty()).then(|| self.catalog(false, cx));
        let thread_id = self.current.as_ref().map(|t| t.id.clone());
        let current = match &self.current {
            Some(t) => t.model.clone(),
            None => self.launch_model.clone(),
        };
        let list = cx.new(|cx| {
            let mut models = Models {
                catalog: catalog.clone(),
                current,
                query: String::new(),
                matches: Vec::new(),
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
            let same_context = view.model_source(cx) == source
                && view.current.as_ref().map(|t| &t.id) == thread_id.as_ref();
            view.model_picker = None;
            view.focus_workspace(window, cx);
            cx.notify();
            if !same_context {
                return;
            }
            if let Some(id) = &thread_id {
                let result = view
                    .runtime
                    .dispatch(Command {
                        id: CommandId(Uuid::new_v4().to_string()),
                        thread_id: id.clone(),
                        action: Action::SetModel { model },
                    })
                    .and_then(|_| view.refresh());
                view.report(result, cx);
            } else {
                view.launch_model = model;
                cx.notify();
            }
        })
        .detach();
        list.focus_handle(cx).focus(window);
        self.model_picker = Some(list);
        cx.notify();
    }
}
