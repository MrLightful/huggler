use crate::{Attachment, Error, Result};

impl Attachment {
    pub const MAX_FILES: usize = 8;
    pub const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;
    pub const MAX_TOTAL_BYTES: usize = 50 * 1024 * 1024;
    pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
    const MAX_TOTAL_IMAGE_BYTES: usize = 20 * 1024 * 1024;

    /// Capture a file, using its signature for common images and PDFs and its name for other types.
    pub fn from_bytes(name: String, bytes: Vec<u8>) -> Result<Self> {
        let mime = match signature(&bytes) {
            Some(mime) => mime.to_owned(),
            None => mime_guess::from_path(&name)
                .first_raw()
                .filter(|mime| !is_native_image(mime) && *mime != "application/pdf")
                .filter(|mime| *mime != "text/plain" || preview_text(&bytes).is_some())
                .unwrap_or_else(|| {
                    if preview_text(&bytes).is_some() {
                        "text/plain"
                    } else {
                        "application/octet-stream"
                    }
                })
                .to_owned(),
        };
        let file = Self { name, mime, bytes };
        file.validate()?;
        Ok(file)
    }

    /// Validate a file selection before sending it.
    pub fn validate_batch(files: &[Self]) -> Result<()> {
        if !(1..=Self::MAX_FILES).contains(&files.len()) {
            return Err(Error::Invalid("attach between one and eight files".into()));
        }
        let mut total = 0;
        let mut images = 0;
        for file in files {
            file.validate()?;
            total += file.bytes.len();
            if is_native_image(&file.mime) {
                images += file.bytes.len();
            }
        }
        validate_total_size(total, images)
    }

    /// UTF-8 content suitable for plain-text preview.
    pub fn text(&self) -> Option<&str> {
        (!is_native_image(&self.mime) && self.mime != "application/pdf")
            .then(|| preview_text(&self.bytes))
            .flatten()
    }

    /// Extract a bounded excerpt from a PDF's text layer.
    pub fn pdf_text_excerpt(&self) -> Option<String> {
        if self.mime != "application/pdf" {
            return None;
        }
        let text = pdf_text(&self.bytes, INLINE_TEXT_BYTES);
        (!text.trim().is_empty()).then_some(text)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        validate_file(&self.name, &self.mime, &self.bytes)
    }
}

pub(crate) fn validate_total_size(total: usize, images: usize) -> Result<()> {
    if total > Attachment::MAX_TOTAL_BYTES || images > Attachment::MAX_TOTAL_IMAGE_BYTES {
        return Err(Error::Invalid(
            "attachments can total up to 50 MiB, including up to 20 MiB of images".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_file(name: &str, media_type: &str, bytes: &[u8]) -> Result<()> {
    validate_name(name)?;
    let mime = media_type
        .parse::<mime_guess::Mime>()
        .map_err(|_| Error::Invalid("expected a media type such as application/pdf".into()))?;
    if mime.essence_str() != media_type || media_type.len() > 127 || media_type.contains('*') {
        return Err(Error::Invalid(
            "expected a lowercase media type without parameters".into(),
        ));
    }
    if bytes.len() > Attachment::MAX_FILE_BYTES {
        return Err(Error::Invalid("choose a file of 50 MiB or less".into()));
    }
    if is_native_image(media_type) && bytes.len() > Attachment::MAX_IMAGE_BYTES {
        return Err(Error::Invalid("choose an image of 10 MiB or less".into()));
    }
    if (is_native_image(media_type) || media_type == "application/pdf")
        && signature(bytes) != Some(media_type)
    {
        return Err(Error::Invalid(format!(
            "{name} does not match its media type"
        )));
    }
    if media_type == "text/plain" && std::str::from_utf8(bytes).is_err() {
        return Err(Error::Invalid("text attachment requires UTF-8".into()));
    }
    Ok(())
}

pub(crate) const INLINE_TEXT_BYTES: usize = 64 * 1024;

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty()
        || name.len() > 255
        || name.contains(['\0', '/', '\\'])
        || matches!(name, "." | "..")
    {
        return Err(Error::Invalid("expected a filename up to 255 bytes".into()));
    }
    Ok(())
}

pub(crate) fn is_native_image(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

fn preview_text(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes)
        .ok()
        .filter(|text| !text.contains('\0'))
}

fn signature(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if bytes.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else {
        None
    }
}

/// Files that need a filesystem path or a provider-native input part.
#[derive(Default)]
pub(crate) struct TurnAttachments {
    pub(crate) directory: std::path::PathBuf,
    pub(crate) pending: Vec<(i64, Attachment)>,
    pub(crate) files: Vec<ProviderFile>,
}

pub(crate) struct ProviderFile {
    pub(crate) file: Attachment,
    pub(crate) path: std::path::PathBuf,
}

impl ProviderFile {
    pub(crate) fn is_image(&self) -> bool {
        is_native_image(&self.file.mime)
    }

    pub(crate) fn base64(&self) -> String {
        use base64::{Engine, engine::general_purpose::STANDARD};
        STANDARD.encode(&self.file.bytes)
    }

    pub(crate) fn file_uri(&self) -> Result<String> {
        url::Url::from_file_path(&self.path)
            .map(String::from)
            .map_err(|_| Error::Invalid("could not create an attachment file URL".into()))
    }
}

impl TurnAttachments {
    pub(crate) fn prepare(mut self, mut text: String) -> Result<(Self, String)> {
        let mut pdf_budget = INLINE_TEXT_BYTES;
        for (position, file) in std::mem::take(&mut self.pending) {
            use std::io::Write;
            let folder = crate::layout::private_dir(&self.directory.join(position.to_string()))?;
            let extension = match file.mime.as_str() {
                "image/jpeg" => Some("jpg"),
                "image/png" => Some("png"),
                "image/gif" => Some("gif"),
                "image/webp" => Some("webp"),
                "application/pdf" => Some("pdf"),
                _ => std::path::Path::new(&file.name)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .filter(|extension| {
                        extension.len() <= 16
                            && extension.bytes().all(|b| b.is_ascii_alphanumeric())
                    }),
            };
            let mut path = folder.join("file");
            if let Some(extension) = extension {
                path.set_extension(extension);
            }
            let path_text = path
                .to_str()
                .ok_or_else(|| Error::Invalid("expected a UTF-8 attachment cache path".into()))?;
            let mut output = tempfile::NamedTempFile::new_in(&folder)?;
            output.write_all(&file.bytes)?;
            output.persist(&path).map_err(|e| e.error)?;

            text.push_str(&format!(
                "\n\nAttached file {} is available at {}",
                serde_json::json!(file.name),
                serde_json::json!(path_text)
            ));
            if file.mime == "application/pdf" && pdf_budget > 0 {
                let extracted = pdf_text(&file.bytes, pdf_budget);
                pdf_budget -= extracted.len();
                if !extracted.trim().is_empty() {
                    text.push_str("\nExtracted PDF text\n");
                    text.push_str(&extracted);
                }
            }
            self.files.push(ProviderFile { file, path });
        }
        Ok((self, text))
    }
}

fn pdf_text(bytes: &[u8], limit: usize) -> String {
    let options = lopdf::LoadOptions::with_max_decompressed_size(8 * 1024 * 1024);
    let Ok(pdf) = lopdf::Document::load_mem_with_options(bytes, options) else {
        return String::new();
    };
    let mut text = String::new();
    let pages: Vec<_> = (1..=pdf.page_iter().take(128).count() as u32).collect();
    for chunk in pdf.extract_text_chunks_with_limit(&pages, 512 * 1024) {
        let Ok(chunk) = chunk else {
            continue;
        };
        let remaining = limit - text.len();
        if chunk.len() > remaining {
            text.push_str(&chunk[..chunk.floor_char_boundary(remaining)]);
            break;
        }
        text.push_str(&chunk);
    }
    text
}
