use super::{Settings, SettingsEvent};
use crate::{
    conversation::ProviderCatalog,
    ui::{hint, label, section_title},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    switch::Switch,
    v_flex,
};
use huggler::ProviderInstanceId;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeSet, HashMap, HashSet},
};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct ModelPreferences {
    order: Vec<String>,
    hidden: BTreeSet<String>,
}

impl ModelPreferences {
    pub(crate) fn enabled(&self, model: &str) -> bool {
        !self.hidden.contains(model)
    }

    pub(crate) fn ordered<'a>(&self, models: &'a [String]) -> Vec<&'a str> {
        let rank: HashMap<_, _> = self.order.iter().map(String::as_str).zip(0..).collect();
        let mut seen = HashSet::new();
        let mut models: Vec<_> = models
            .iter()
            .map(String::as_str)
            .filter(|model| seen.insert(*model))
            .collect();
        models.sort_unstable_by_key(|&model| {
            (
                rank.get(model).copied().unwrap_or(usize::MAX),
                Reverse(model),
            )
        });
        models
    }
}

impl Settings {
    pub(crate) fn model_preferences(&self, id: &ProviderInstanceId) -> ModelPreferences {
        self.prefs
            .models
            .get(id.as_str())
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn request_models(&self, refresh: bool, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::ProviderModels(
            self.providers.selected.clone(),
            refresh,
        ));
    }

    pub(crate) fn set_model_catalog(
        &mut self,
        id: &ProviderInstanceId,
        catalog: Entity<ProviderCatalog>,
        cx: &mut Context<Self>,
    ) {
        if !self.provider_details_open() || self.providers.selected != *id {
            return;
        }
        self.catalog_subscription = Some(cx.observe(&catalog, |_, _, cx| cx.notify()));
        self.model_catalog = Some(catalog);
        cx.notify();
    }

    fn toggle_model(&mut self, model: &str, enabled: bool, cx: &mut Context<Self>) {
        let mut prefs = self.prefs.clone();
        let models = prefs
            .models
            .entry(self.providers.selected.as_str().into())
            .or_default();
        if enabled {
            models.hidden.remove(model);
        } else {
            models.hidden.insert(model.into());
        }
        self.persist(prefs, cx);
    }

    fn move_model(&mut self, model: &str, neighbor: &str, cx: &mut Context<Self>) {
        let Some(Ok(report)) = self
            .model_catalog
            .as_ref()
            .and_then(|catalog| catalog.read(cx).result.as_ref())
        else {
            return;
        };
        let mut prefs = self.prefs.clone();
        let models = prefs
            .models
            .entry(self.providers.selected.as_str().into())
            .or_default();
        let mut order = models.order.clone();
        let ranked: HashSet<_> = models.order.iter().map(String::as_str).collect();
        order.extend(
            models
                .ordered(&report.models)
                .into_iter()
                .filter(|id| !ranked.contains(id))
                .map(str::to_owned),
        );
        if let (Some(from), Some(to)) = (
            order.iter().position(|id| id == model),
            order.iter().position(|id| id == neighbor),
        ) {
            order.swap(from, to);
            models.order = order;
            self.persist(prefs, cx);
        }
    }

    pub(super) fn render_models(&self, cx: &Context<Self>) -> Div {
        let catalog = self.model_catalog.as_ref().map(|catalog| catalog.read(cx));
        let content = match catalog.map(|catalog| &catalog.result) {
            None => hint("Use Refresh to load models.", cx),
            Some(None) => hint("Loading models…", cx),
            Some(Some(Err(error))) => hint(error.to_string(), cx),
            Some(Some(Ok(report))) if report.models.is_empty() => {
                control!("No models", hint("This provider listed no models.", cx))
            }
            Some(Some(Ok(report))) => self.model_list(&report.models, cx),
        };

        v_flex()
            .gap_2()
            .pt_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(control!("Models", section_title("Models")))
            .child(content)
    }

    fn model_list(&self, models: &[String], cx: &Context<Self>) -> Div {
        let prefs = self.model_preferences(&self.providers.selected);
        let (enabled, hidden): (Vec<_>, Vec<_>) = prefs
            .ordered(models)
            .into_iter()
            .partition(|id| prefs.enabled(id));
        let rows = enabled.iter().enumerate().map(|(index, id)| {
            let above = index.checked_sub(1).map(|i| enabled[i]);
            let below = enabled.get(index + 1).copied();
            Self::model_row(id, true, above, below, cx)
        });
        v_flex()
            .gap_2()
            .child(hint(
                "Choose which models appear in the conversation picker and in what order.",
                cx,
            ))
            .child(
                div()
                    .id("provider-models")
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .child(v_flex().children(rows).when(!hidden.is_empty(), |list| {
                        list.child(hint("Hidden from picker", cx).pt_3().pb_1())
                            .children(
                                hidden
                                    .iter()
                                    .map(|id| Self::model_row(id, false, None, None, cx)),
                            )
                    })),
            )
    }

    fn model_row(
        id: &str,
        enabled: bool,
        above: Option<&str>,
        below: Option<&str>,
        cx: &Context<Self>,
    ) -> Div {
        let name = id.to_owned();
        let move_button = |direction: &str, icon, neighbor: Option<&str>| {
            let model = id.to_owned();
            let neighbor = neighbor.map(str::to_owned);
            let key = SharedString::from(format!("model-{direction}/{id}"));
            control!(
                key.clone(),
                Button::new(key)
                    .ghost()
                    .small()
                    .icon(icon)
                    .tooltip(format!("Move {id} {direction}"))
                    .disabled(neighbor.is_none())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(neighbor) = &neighbor {
                            view.move_model(&model, neighbor, cx);
                        }
                    }))
            )
        };
        h_flex()
            .min_h(px(36.))
            .gap_2()
            .pr_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(label(name.clone()).flex_1().min_w_0().truncate())
            .when(enabled, |row| {
                row.child(move_button("up", IconName::ArrowUp, above))
                    .child(move_button("down", IconName::ArrowDown, below))
            })
            .child(control!(
                format!("model-enabled/{id}"),
                Button::new(SharedString::from(format!("toggle-model/{id}")))
                    .ghost()
                    .small()
                    .tooltip(format!(
                        "{} {id} in the model picker",
                        if enabled { "Hide" } else { "Show" }
                    ))
                    .child(
                        Switch::new(SharedString::from(format!("model-switch/{id}")))
                            .small()
                            .checked(enabled)
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.toggle_model(&name, !enabled, cx)
                    }))
            ))
    }
}
