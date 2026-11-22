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
        "provider-logos/cursor.svg",
        include_bytes!("../assets/provider-logos/cursor.svg"),
    ),
    (
        "provider-logos/grok.svg",
        include_bytes!("../assets/provider-logos/grok.svg"),
    ),
    (
        "provider-logos/vercel.svg",
        include_bytes!("../assets/provider-logos/vercel.svg"),
    ),
    (
        "provider-logos/kiro.svg",
        include_bytes!("../assets/provider-logos/kiro.svg"),
    ),
    (
        "provider-logos/antigravity.svg",
        include_bytes!("../assets/provider-logos/antigravity.svg"),
    ),
    (
        "provider-logos/agent-handshake.svg",
        include_bytes!("../assets/provider-logos/agent-handshake.svg"),
    ),
    (
        "provider-logos/opencode.svg",
        include_bytes!("../assets/provider-logos/opencode.svg"),
    ),
    (
        "provider-logos/pi.svg",
        include_bytes!("../assets/provider-logos/pi.svg"),
    ),
    (
        "provider-logos/meta.svg",
        include_bytes!("../assets/provider-logos/meta.svg"),
    ),
    ("icons/mcp.svg", include_bytes!("../assets/icons/mcp.svg")),
    (
        "icons/provider-refresh.svg",
        include_bytes!("../assets/icons/provider-refresh.svg"),
    ),
    (
        "icons/image-attachment.svg",
        include_bytes!("../assets/icons/image-attachment.svg"),
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
