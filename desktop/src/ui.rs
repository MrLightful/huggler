use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Selectable, Theme,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    v_flex,
};
use std::rc::Rc;

pub(crate) fn init_theme(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    let colors = &mut Rc::make_mut(&mut theme.dark_theme).colors;
    colors.background = Some("#1a1918".into());
    colors.sidebar = Some("#242220".into());
    colors.sidebar_accent = Some("#35322f".into());
    colors.secondary = colors.sidebar_accent.clone();
    colors.secondary_hover = Some("#403c38".into());
    colors.secondary_active = colors.secondary_hover.clone();
    colors.muted = Some("#302d2a".into());
    colors.muted_foreground = Some("#aaa49e".into());
    colors.border = Some("#383532".into());
    colors.sidebar_border = colors.border.clone();
    colors.popover = Some("#292624".into());
    colors.accent = colors.sidebar_accent.clone();
    if theme.is_dark() {
        theme.apply_config(&theme.dark_theme.clone());
    }
}

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
