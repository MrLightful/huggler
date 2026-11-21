use gpui::App;
use gpui_component::Theme;
use std::{borrow::Cow, rc::Rc};

pub(crate) fn init(cx: &mut App) {
    // GPUI selects the closest weight and style from the faces in a family.
    let faces: [&'static [u8]; 6] = [
        include_bytes!("../assets/dm-sans/DMSans-Regular.ttf"),
        include_bytes!("../assets/dm-sans/DMSans-Medium.ttf"),
        include_bytes!("../assets/dm-sans/DMSans-SemiBold.ttf"),
        include_bytes!("../assets/dm-sans/DMSans-Bold.ttf"),
        include_bytes!("../assets/dm-sans/DMSans-Italic.ttf"),
        include_bytes!("../assets/dm-sans/DMSans-BoldItalic.ttf"),
    ];
    cx.text_system()
        .add_fonts(faces.map(Cow::Borrowed).into())
        .expect("register bundled DM Sans fonts");

    let theme = Theme::global_mut(cx);
    theme.font_family = "DM Sans".into();
    for palette in [&mut theme.light_theme, &mut theme.dark_theme] {
        Rc::make_mut(palette).font_family = Some(theme.font_family.clone());
    }
}
