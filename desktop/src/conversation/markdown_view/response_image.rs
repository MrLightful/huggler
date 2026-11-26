use super::{InlineImage, ResponseImages};
use crate::conversation::{
    attachments::image_preview, document_tabs::open_workspace_file, history_at_bottom,
    transcript::transcript_muted,
};
use anyhow::{Context as _, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use gpui::{prelude::*, *};
use gpui_component::{button::Button, v_flex};
use percent_encoding::percent_decode_str;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use url::Url;

const IMAGE_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Hash)]
enum Source {
    File { checkout: PathBuf, path: PathBuf },
    Embedded(String),
    Web(Url),
}

impl Source {
    fn parse(value: &str, checkout: Option<&Path>) -> Option<Self> {
        if value.is_empty() || value.starts_with("//") {
            return None;
        }
        if let Ok(mut url) = Url::parse(value) {
            url.set_fragment(None);
            return match url.scheme() {
                "data" if url.path().starts_with("image/") && value.len() <= IMAGE_LIMIT * 2 => {
                    Some(Self::Embedded(url.path().to_owned()))
                }
                "http" | "https" if url.username().is_empty() && url.password().is_none() => {
                    Some(Self::Web(url))
                }
                "file" if url.host_str().is_none() => Some(Self::File {
                    checkout: checkout?.to_owned(),
                    path: url.to_file_path().ok()?,
                }),
                _ => None,
            };
        }
        let path = value.split(['?', '#']).next()?;
        let path = percent_decode_str(path).decode_utf8().ok()?;
        Some(Self::File {
            checkout: checkout?.to_owned(),
            path: PathBuf::from(path.as_ref()),
        })
    }

    async fn load(self: Arc<Self>) -> Result<Arc<RenderImage>> {
        if let Self::Web(url) = self.as_ref() {
            let bytes = download(url).await?;
            return tokio::task::spawn_blocking(move || decode(&bytes)).await?;
        }
        tokio::task::spawn_blocking(move || decode(&self.read()?)).await?
    }

    fn read(&self) -> Result<Vec<u8>> {
        match self {
            Self::File { checkout, path } => {
                let file = open_workspace_file(checkout, checkout_relative(checkout, path)?)?;
                ensure!(file.metadata()?.is_file(), "Expected a regular image file");
                let mut bytes = Vec::new();
                file.take((IMAGE_LIMIT + 1) as u64)
                    .read_to_end(&mut bytes)?;
                Ok(bytes)
            }
            Self::Embedded(data) => {
                let (mime, encoded) = data.split_once(',').context("Invalid image data URL")?;
                let bytes = percent_decode_str(encoded).collect::<Vec<_>>();
                if mime.ends_with(";base64") {
                    Ok(STANDARD.decode(bytes)?)
                } else {
                    Ok(bytes)
                }
            }
            Self::Web(_) => unreachable!("web images are downloaded"),
        }
    }
}

fn checkout_relative<'a>(checkout: &Path, path: &'a Path) -> Result<&'a Path> {
    if !path.is_absolute() {
        return Ok(path.strip_prefix(".").unwrap_or(path));
    }
    let canonical = checkout.canonicalize()?;
    if let Ok(relative) = path
        .strip_prefix(checkout)
        .or_else(|_| path.strip_prefix(&canonical))
    {
        return Ok(relative);
    }
    // Resolving a Windows UNC path can authenticate to a network share before
    // its scope is established. Only try alternate root spellings on Unix.
    ensure!(cfg!(unix), "Image is outside the checkout");
    // Accept aliases of the checkout's parent directories (for example /var
    // on macOS). The scoped reader still checks every component below the root.
    let root = path
        .ancestors()
        .skip(1)
        .find(|parent| {
            parent.file_name() == canonical.file_name()
                && parent
                    .canonicalize()
                    .is_ok_and(|parent| parent == canonical)
        })
        .context("Image is outside the checkout")?;
    Ok(path.strip_prefix(root)?)
}

async fn download(url: &Url) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let mut response = client.get(url.clone()).send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= IMAGE_LIMIT,
            "Image exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<Arc<RenderImage>> {
    ensure!(bytes.len() <= IMAGE_LIMIT, "Image exceeds size limit");
    image_preview(bytes, false).context("Unsupported or invalid image")
}

enum Preview {
    Pending,
    Loading,
    Ready(Arc<RenderImage>),
    Unavailable,
}

fn load(
    source: Arc<Source>,
    executor: &tokio::runtime::Handle,
    scroll: ScrollHandle,
    cx: &mut Context<Preview>,
) {
    let task = executor.spawn(source.load());
    cx.spawn(async move |state, cx| {
        let image = task.await.ok().and_then(Result::ok);
        let _ = state.update(cx, |preview, cx| {
            if history_at_bottom(&scroll) {
                scroll.scroll_to_bottom();
            }
            *preview = image.map_or(Preview::Unavailable, Preview::Ready);
            cx.notify();
        });
    })
    .detach();
}

pub(super) fn render(
    id: &str,
    InlineImage {
        destination,
        alt,
        link,
    }: InlineImage<'_>,
    images: ResponseImages<'_>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let label = if alt.trim().is_empty() { "Image" } else { alt };
    let Some(source) = Source::parse(destination, images.checkout).map(Arc::new) else {
        return status(label, "Image unavailable", cx);
    };
    // Keep the user's web load when streaming becomes a saved reply. Local
    // previews get a fresh read, since the file may have changed during the run.
    let scope = match source.as_ref() {
        Source::Web(_) => images.run.0.as_str(),
        _ => id,
    };
    let key = SharedString::from(format!("response-image-{scope}-{}", gpui::hash(&source)));
    let preview = window.use_keyed_state(key, cx, |_, cx| {
        if matches!(source.as_ref(), Source::Web(_)) {
            Preview::Pending
        } else {
            load(source.clone(), images.executor, images.scroll.clone(), cx);
            Preview::Loading
        }
    });
    match preview.read(cx) {
        Preview::Pending => {
            let Source::Web(url) = source.as_ref() else {
                unreachable!("only web images need a click to load")
            };
            let host = url.host_str().unwrap_or_default();
            let caption = format!("{label} · {host}");
            let executor = images.executor.clone();
            let scroll = images.scroll.clone();
            v_flex()
                .items_start()
                .gap_1()
                .child(div().text_color(transcript_muted(cx)).child(caption))
                .child(control!(
                    format!("load-response-image/{label}"),
                    Button::new(SharedString::from(id.to_owned()))
                        .outline()
                        .label("Load image")
                        .on_click(move |_, _, cx| {
                            preview.update(cx, |preview, cx| {
                                *preview = Preview::Loading;
                                load(source.clone(), &executor, scroll.clone(), cx);
                                cx.notify();
                            });
                        })
                ))
        }
        Preview::Loading => status(label, "Loading image…", cx),
        Preview::Ready(image) => v_flex()
            .gap_1()
            .items_start()
            .child(
                control!(
                    format!("response-image/{label}"),
                    linked_image(id, image.clone(), link)
                )
                .w_full(),
            )
            .when(!alt.trim().is_empty(), |image| {
                image.child(
                    div()
                        .text_color(transcript_muted(cx))
                        .child(label.to_owned()),
                )
            }),
        Preview::Unavailable => status(label, "Image unavailable", cx),
    }
}

fn linked_image(id: &str, image: Arc<RenderImage>, link: Option<&str>) -> impl IntoElement {
    let size = image.size(0);
    let width = size.width.0 as f32;
    let height = size.height.0 as f32;
    let mut frame = div().relative().w_full().max_w(px(width));
    frame.style().aspect_ratio = Some(width / height);
    frame
        .id(SharedString::from(id.to_owned()))
        .child(img(image).absolute().size_full())
        .when_some(link.map(str::to_owned), |frame, url| {
            frame.cursor_pointer().on_click(move |_, _, cx| {
                #[cfg(all(feature = "qualification", target_os = "macos"))]
                if crate::qualification::record_open_url(&url, cx) {
                    return;
                }
                cx.open_url(&url);
            })
        })
}

fn status(label: &str, status: &str, cx: &App) -> Div {
    control!(
        format!("response-image-status/{label}/{status}"),
        div()
            .text_color(transcript_muted(cx))
            .child(if label == "Image" {
                status.to_owned()
            } else {
                format!("{label} · {status}")
            })
    )
}

#[cfg(test)]
mod tests {
    use super::Source;

    #[test]
    fn remote_responses_only_use_self_contained_or_web_images() {
        for path in ["chart.png", "/tmp/chart.png", "file:///tmp/chart.png"] {
            assert!(Source::parse(path, None).is_none());
        }
        assert!(matches!(
            Source::parse("https://example.com/chart.png", None),
            Some(Source::Web(_))
        ));
        assert!(matches!(
            Source::parse("data:image/png;base64,AA==", None),
            Some(Source::Embedded(_))
        ));
    }
}
