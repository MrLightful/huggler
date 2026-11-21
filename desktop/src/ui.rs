use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Selectable,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    v_flex,
};

pub fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_xl()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

pub fn section_title(text: impl Into<SharedString>) -> Div {
    div()
        .text_base()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

pub fn label(text: impl Into<SharedString>) -> Div {
    div()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

pub fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

pub fn field(name: &'static str, state: &Entity<InputState>) -> Div {
    v_flex()
        .gap_1()
        .child(label(name))
        .child(control!(name, Input::new(state)))
}

pub fn choice(id: impl Into<ElementId>, title: impl Into<SharedString>, selected: bool) -> Button {
    Button::new(id)
        .label(title)
        .selected(selected)
        .when(selected, |button| button.primary())
}
