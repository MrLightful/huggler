use super::{Controls, huggler_window_number};
use anyhow::{Context as _, Result, ensure};
use gpui::*;
use gpui_component::Theme;
use image::{ImageFormat, RgbaImage};
use objc2_app_kit::NSApplication;
use objc2_foundation::MainThreadMarker;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Deserialize)]
pub(crate) struct Screenshot {
    pub(super) name: String,
    pub(super) path: Option<PathBuf>,
    baseline: Option<PathBuf>,
    component: Option<String>,
}

impl Screenshot {
    pub(super) async fn run(&self, handle: AnyWindowHandle, cx: &mut AsyncApp) -> Result<()> {
        let Some(path) = &self.path else {
            handle.update(cx, |_, window, cx| self.component_bounds(window, cx))??;
            return Ok(());
        };
        // The input caret blinks independently of the scripted interaction.
        let caret = handle.update(cx, |_, window, cx| {
            let theme = Theme::global_mut(cx);
            let caret = theme.caret;
            theme.caret = Hsla::transparent_black();
            window.refresh();
            caret
        })?;
        let result = self.capture(handle, cx).await;
        handle.update(cx, |_, window, cx| {
            Theme::global_mut(cx).caret = caret;
            window.refresh();
        })?;

        let image = result?;
        image.save_with_format(path, ImageFormat::Png)?;
        if let Some(baseline) = &self.baseline
            && pixels_changed(&image, baseline)?
        {
            println!("Screenshot changed: {}", self.name);
        }
        Ok(())
    }

    fn component_bounds(&self, window: &Window, cx: &App) -> Result<Option<Bounds<Pixels>>> {
        self.component
            .as_deref()
            .map(|name| {
                let bounds = cx
                    .global::<Controls>()
                    .0
                    .get(name)
                    .copied()
                    .with_context(|| format!("component {name:?} is missing"))?;
                validate_bounds(window.viewport_size(), bounds)?;
                Ok(bounds)
            })
            .transpose()
    }

    async fn capture(&self, handle: AnyWindowHandle, cx: &mut AsyncApp) -> Result<RgbaImage> {
        let number = handle.update(cx, |_, _, _| -> Result<_> {
            let app =
                NSApplication::sharedApplication(MainThreadMarker::new().expect("GPUI thread"));
            huggler_window_number(&app).context("Huggler window is missing")
        })??;
        // Give AppKit and the compositor time to present the GPUI scene and finish transitions.
        cx.background_executor()
            .timer(Duration::from_millis(300))
            .await;
        let (viewport, scale, component) = handle.update(cx, |_, window, cx| -> Result<_> {
            Ok((
                window.viewport_size(),
                window.scale_factor(),
                self.component_bounds(window, cx)?,
            ))
        })??;
        let image = capture_window(number).await?;
        let expected_width = (f32::from(viewport.width) * scale).round() as u32;
        ensure!(
            expected_width.abs_diff(image.width()) <= 1,
            "captured window width is {} pixels, expected {expected_width}",
            image.width()
        );
        ensure!(
            image.pixels().any(|pixel| pixel != image.get_pixel(0, 0)),
            "captured a blank window. Use an unlocked GUI session with Screen Recording access."
        );
        match component {
            Some(bounds) => crop(&image, viewport, scale, bounds),
            None => Ok(image),
        }
    }
}

async fn capture_window(number: isize) -> Result<RgbaImage> {
    let scratch = tempfile::tempdir().context("create screenshot scratch directory")?;
    let file = scratch.path().join("window.png");
    let output = async_process::Command::new("/usr/sbin/screencapture")
        .args(["-x", "-o", "-t", "png", "-l", &number.to_string()])
        .arg(&file)
        .output()
        .await
        .context("run macOS screencapture")?;
    ensure!(
        output.status.success(),
        "macOS could not capture window {number}. Use an unlocked GUI session with Screen Recording access. {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(image::open(&file)
        .context("read captured window")?
        .into_rgba8())
}

fn pixels_changed(current: &RgbaImage, baseline: &Path) -> Result<bool> {
    let previous = image::open(baseline)
        .with_context(|| format!("read baseline screenshot {}", baseline.display()))?
        .into_rgba8();
    Ok(&previous != current)
}

fn validate_bounds(viewport: Size<Pixels>, bounds: Bounds<Pixels>) -> Result<()> {
    ensure!(
        bounds.size.width > px(0.) && bounds.size.height > px(0.),
        "component has empty bounds"
    );
    ensure!(
        bounds.left() >= px(0.)
            && bounds.top() >= px(0.)
            && bounds.right() <= viewport.width
            && bounds.bottom() <= viewport.height,
        "component extends outside the viewport: {bounds:?} in {viewport:?}"
    );
    Ok(())
}

fn crop(
    image: &RgbaImage,
    viewport: Size<Pixels>,
    scale: f32,
    bounds: Bounds<Pixels>,
) -> Result<RgbaImage> {
    validate_bounds(viewport, bounds)?;
    // The native title bar is above the GPUI viewport in the captured window.
    let content_height = (f32::from(viewport.height) * scale).round() as u32;
    let titlebar = image
        .height()
        .checked_sub(content_height)
        .context("window image is shorter than its GPUI viewport")?;
    let left = (f32::from(bounds.left()) * scale).floor() as u32;
    let top = titlebar + (f32::from(bounds.top()) * scale).floor() as u32;
    let right = ((f32::from(bounds.right()) * scale).ceil() as u32).min(image.width());
    let bottom =
        (titlebar + (f32::from(bounds.bottom()) * scale).ceil() as u32).min(image.height());
    ensure!(left < right && top < bottom, "component has empty bounds");
    Ok(image::imageops::crop_imm(image, left, top, right - left, bottom - top).to_image())
}

#[cfg(test)]
mod tests {
    use super::{crop, pixels_changed};
    use anyhow::Result;
    use gpui::{Bounds, point, px, size};
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ColorType, ImageEncoder as _, Rgba, RgbaImage};

    #[test]
    fn component_crop_accounts_for_display_scale_and_native_title_bar() -> Result<()> {
        let source = RgbaImage::from_fn(12, 12, |x, y| Rgba([x as u8, y as u8, 0, 255]));
        let viewport = size(px(6.), px(4.));
        let component = Bounds::new(point(px(1.25), px(0.75)), size(px(2.), px(2.)));
        let cropped = crop(&source, viewport, 2., component)?;

        assert_eq!(cropped.dimensions(), (5, 5));
        assert_eq!(cropped.get_pixel(0, 0), source.get_pixel(2, 5));
        assert_eq!(cropped.get_pixel(4, 4), source.get_pixel(6, 9));
        let offscreen = Bounds::new(point(px(5.), px(0.)), size(px(2.), px(2.)));
        assert!(crop(&source, viewport, 2., offscreen).is_err());
        Ok(())
    }

    #[test]
    fn baseline_comparison_uses_pixels_and_dimensions() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let baseline = directory.path().join("baseline.png");
        let mut current = RgbaImage::from_pixel(8, 8, Rgba([42, 46, 50, 255]));
        PngEncoder::new_with_quality(
            std::fs::File::create(&baseline)?,
            CompressionType::Best,
            FilterType::NoFilter,
        )
        .write_image(current.as_raw(), 8, 8, ColorType::Rgba8.into())?;
        assert!(!pixels_changed(&current, &baseline)?);
        assert!(pixels_changed(&RgbaImage::new(8, 9), &baseline)?);
        current.get_pixel_mut(0, 0)[3] = 200;
        assert!(pixels_changed(&current, &baseline)?);
        Ok(())
    }
}
