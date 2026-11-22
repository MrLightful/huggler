use super::Settings;
use crate::ui::{hint, label, section_title};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use std::collections::BTreeMap;

pub(super) struct EnvironmentRow {
    id: uuid::Uuid,
    name: Entity<InputState>,
    value: Entity<InputState>,
}

impl EnvironmentRow {
    fn new(name: &str, value: &str, window: &mut Window, cx: &mut Context<Settings>) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(name.to_owned())
                    .placeholder("NAME")
            }),
            value: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value.to_owned())
                    .placeholder("Value")
                    .masked(true)
            }),
        }
    }
}

impl Settings {
    fn edit_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.environment_rows.is_none() {
            self.environment_rows = Some(
                self.prefs
                    .provider_environment
                    .get(self.providers.selected.as_str())
                    .into_iter()
                    .flatten()
                    .map(|(name, value)| EnvironmentRow::new(name, value, window, cx))
                    .collect(),
            );
        }
        cx.notify();
    }

    fn add_variable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_environment(window, cx);
        let row = EnvironmentRow::new("", "", window, cx);
        row.name.focus_handle(cx).focus(window);
        self.environment_rows.get_or_insert_default().push(row);
    }

    fn save_environment(&mut self, cx: &mut Context<Self>) {
        let Some(rows) = self.environment_rows.as_ref() else {
            return;
        };
        let mut variables = BTreeMap::new();
        for row in rows {
            let name = row.name.read(cx).value();
            let name = name.trim();
            let value = row.value.read(cx).value();
            if name.is_empty() && value.is_empty() {
                continue;
            }
            let mut bytes = name.bytes();
            let valid_name = bytes
                .next()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
                && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
            if !valid_name || value.contains('\0') {
                self.error = Some("Use letters, numbers, and underscores in variable names. Start with a letter or underscore, and remove null characters from values.".into());
                cx.notify();
                return;
            }
            if variables
                .insert(name.to_owned(), value.to_string())
                .is_some()
            {
                self.error = Some(format!("Enter only one value for {name}."));
                cx.notify();
                return;
            }
        }

        let mut prefs = self.prefs.clone();
        let id = self.providers.selected.as_str();
        if variables.is_empty() {
            prefs.provider_environment.remove(id);
        } else {
            prefs.provider_environment.insert(id.to_owned(), variables);
        }
        if self.persist(prefs, cx) {
            self.environment_rows = None;
        }
    }

    pub(super) fn render_environment(&self, cx: &Context<Self>) -> Div {
        let saved = self
            .prefs
            .provider_environment
            .get(self.providers.selected.as_str());
        let pending = saved != Some(self.selected_provider().environment())
            && (saved.is_some() || !self.selected_provider().environment().is_empty());
        let actions = h_flex()
            .gap_2()
            .flex_wrap()
            .when(saved.is_some() && self.environment_rows.is_none(), |row| {
                row.child(control!(
                    "Edit variables",
                    Button::new("edit-variables")
                        .small()
                        .label("Edit variables")
                        .on_click(
                            cx.listener(|view, _, window, cx| view.edit_environment(window, cx))
                        )
                ))
            })
            .child(control!(
                "Add variable",
                Button::new("add-variable")
                    .small()
                    .outline()
                    .icon(IconName::Plus)
                    .label("Add variable")
                    .on_click(cx.listener(|view, _, window, cx| view.add_variable(window, cx)))
            ));

        v_flex()
            .gap_3()
            .pt_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(control!("Environment", section_title("Environment")))
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .flex_wrap()
                    .child(hint("Variables for this provider’s CLI.", cx))
                    .child(actions),
            )
            .when(self.environment_rows.is_none(), |section| {
                section.children(saved.into_iter().flat_map(BTreeMap::keys).map(|name| {
                    control!(
                        format!("environment-variable/{name}"),
                        label(name.clone()).truncate()
                    )
                }))
            })
            .when_some(self.environment_rows.as_ref(), |section, rows| {
                section
                    .children(rows.iter().enumerate().map(|(index, row)| {
                        let id = row.id;
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(control!(
                                format!("variable-name/{index}"),
                                Input::new(&row.name)
                            )))
                            .child(div().flex_1().min_w_0().child(control!(
                                format!("variable-value/{index}"),
                                Input::new(&row.value).mask_toggle()
                            )))
                            .child(control!(
                                format!("remove-variable/{index}"),
                                Button::new(("remove-variable", index))
                                    .small()
                                    .ghost()
                                    .icon(IconName::Minus)
                                    .tooltip("Remove variable")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if let Some(rows) = &mut view.environment_rows {
                                            rows.retain(|row| row.id != id);
                                            cx.notify();
                                        }
                                    }))
                            ))
                    }))
                    .child(hint(
                        "Values are saved in this Mac’s Huggler preferences.",
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(control!(
                                "Save variables",
                                Button::new("save-variables")
                                    .small()
                                    .primary()
                                    .label("Save variables")
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.save_environment(cx))
                                    )
                            ))
                            .child(control!(
                                "Cancel variable changes",
                                Button::new("cancel-variables")
                                    .small()
                                    .label("Cancel")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.environment_rows = None;
                                        view.error = None;
                                        cx.notify();
                                    }))
                            )),
                    )
            })
            .when(pending, |section| {
                section.child(hint("Restart Huggler to apply environment changes.", cx))
            })
    }
}
