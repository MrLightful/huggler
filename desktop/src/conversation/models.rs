use super::*;
use gpui_component::{
    IconName, IndexPath, Sizable, WindowExt,
    list::{List, ListDelegate, ListEvent, ListItem, ListState},
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
        cx.spawn(async move |catalog, cx| {
            let result = runtime
                .probe_provider(instance.as_str(), checkout)
                .await
                .map_err(|error| error.to_string());
            let _ = catalog.update(cx, |catalog, cx| {
                catalog.result = Some(result);
                cx.notify();
            });
        })
        .detach();
    }
}

struct Models {
    catalog: Entity<ProviderCatalog>,
    current: Option<String>,
    query: String,
    matches: Vec<Option<String>>,
}

impl Models {
    fn filter(&mut self, cx: &App) {
        let report = self
            .catalog
            .read(cx)
            .result
            .as_ref()
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
    fn model_source(&self, cx: &App) -> (ProviderInstanceId, PathBuf) {
        match &self.current {
            Some(thread) => (
                thread.provider_instance.clone(),
                self.workspaces[&thread.id].root.clone(),
            ),
            None => (
                self.instance.clone(),
                PathBuf::from(self.checkout.read(cx).value().as_str()),
            ),
        }
    }

    pub(super) fn catalog(
        &mut self,
        refresh: bool,
        cx: &mut Context<Self>,
    ) -> Entity<ProviderCatalog> {
        let (instance, checkout) = self.model_source(cx);
        if let Some(catalog) = &self.provider_check {
            let state = catalog.read(cx);
            if state.instance == instance && state.checkout == checkout {
                if refresh && !state.loading() {
                    catalog.update(cx, |state, cx| state.load(self.runtime.clone(), cx));
                }
                return catalog.clone();
            }
        }
        let catalog = cx.new(|cx| {
            let mut catalog = ProviderCatalog {
                instance,
                checkout,
                result: None,
            };
            catalog.load(self.runtime.clone(), cx);
            catalog
        });
        cx.observe(&catalog, |_, _, cx| cx.notify()).detach();
        self.provider_check = Some(catalog.clone());
        cx.notify();
        catalog
    }

    pub(super) fn model_control(
        &self,
        model: Option<&str>,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Div {
        let title = model.unwrap_or(DEFAULT);
        h_flex()
            .min_w_0()
            .gap_2()
            .child(label("Model"))
            .child(control!(
                format!("model/{title}"),
                Button::new("choose-model")
                    .small()
                    .dropdown_caret(true)
                    .child(div().max_w(px(192.)).truncate().child(title.to_owned()))
                    .tooltip(if disabled {
                        "Choose a checkout to load models".into()
                    } else {
                        format!("Choose model · {title}")
                    })
                    .disabled(disabled)
                    .on_click(cx.listener(|view, _, window, cx| view.open_models(window, cx)))
            ))
    }

    fn open_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let catalog = self.catalog(false, cx);
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
            cx.observe(&catalog, |list: &mut ListState<Models>, _, cx| {
                list.delegate_mut().filter(cx);
                cx.notify();
            })
            .detach();
            ListState::new(models, window, cx).searchable(true)
        });
        cx.subscribe_in(&list, window, move |view, list, event, window, cx| {
            let ListEvent::Confirm(ix) = event else {
                return;
            };
            let models = list.read(cx).delegate();
            let Some(model) = models.matches.get(ix.row).cloned() else {
                return;
            };
            let source = models.catalog.read(cx);
            let (instance, checkout) = view.model_source(cx);
            let same_context = instance == source.instance
                && checkout == source.checkout
                && view.current.as_ref().map(|t| &t.id) == thread_id.as_ref();
            window.close_dialog(cx);
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
        let runtime = self.runtime.clone();
        let provider = self
            .runtime
            .output()
            .provider_label(catalog.read(cx).instance.as_str())
            .to_owned();
        let focus = list.focus_handle(cx);
        window.open_dialog(cx, move |dialog, _, cx| {
            let (message, can_retry) = match &catalog.read(cx).result {
                None => (Some("Loading models…".into()), false),
                Some(Err(error)) => (Some(error.clone()), true),
                Some(Ok(report)) if report.models.is_empty() => {
                    (Some("This provider listed no models.".into()), true)
                }
                Some(Ok(_)) => (None, false),
            };
            dialog
                .title(format!("Choose model · {provider}"))
                .w(px(410.))
                .child(
                    List::new(&list)
                        .search_placeholder("Search models")
                        .h(px(190.)),
                )
                .when_some(message, |dialog, message| {
                    dialog.child(control!(
                        format!("model-status/{message}"),
                        hint(message, cx)
                    ))
                })
                .when(can_retry, |dialog| {
                    let catalog = catalog.clone();
                    let runtime = runtime.clone();
                    dialog.child(control!(
                        "Retry models",
                        Button::new("retry-models")
                            .label("Retry")
                            .on_click(move |_, _, cx| {
                                catalog.update(cx, |state, cx| {
                                    if !state.loading() {
                                        state.load(runtime.clone(), cx);
                                    }
                                });
                            })
                    ))
                })
        });
        focus.focus(window);
    }
}
