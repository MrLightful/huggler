use crate::file_icons;
use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

pub(crate) struct Assets;

const CUSTOM_ASSETS: &[(&str, &[u8])] = &[
    (
        "provider-logos/codex.svg",
        include_bytes!("../assets/provider-logos/codex.svg"),
    ),
    (
        "provider-logos/claude.svg",
        include_bytes!("../assets/provider-logos/claude.svg"),
    ),
    (
        "icons/provider-refresh.svg",
        include_bytes!("../assets/icons/provider-refresh.svg"),
    ),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = CUSTOM_ASSETS
            .iter()
            .chain(file_icons::ASSETS)
            .find(|(name, _)| *name == path)
        {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = gpui_component_assets::Assets.list(path)?;
        assets.extend(
            CUSTOM_ASSETS
                .iter()
                .chain(file_icons::ASSETS)
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(assets)
    }
}
