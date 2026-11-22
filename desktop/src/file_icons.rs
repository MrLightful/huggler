use gpui::{AssetSource, Hsla, IntoElement, SharedString, Styled, img};
use gpui_component::{Icon, IconName, Sizable};
use std::{borrow::Cow, path::Path};

macro_rules! file_icons {
    ($($icon:ident => $name:literal),+ $(,)?) => {
        enum FileIcon {
            $($icon,)+
            Generic(IconName),
        }

        impl FileIcon {
            fn render(self, muted: Hsla) -> gpui::AnyElement {
                let asset = match self {
                    $(Self::$icon => concat!("file-icons/", $name, ".svg"),)+
                    Self::Generic(symbol) => {
                        return Icon::new(symbol).small().text_color(muted).into_any_element();
                    }
                };
                img(SharedString::from(asset)).size_3p5().flex_shrink_0().into_any_element()
            }
        }

        const ASSETS: &[(&str, &[u8])] = &[
            $((concat!("file-icons/", $name, ".svg"), include_bytes!(concat!("../assets/file-icons/", $name, ".svg"))),)+
        ];
    };
}

file_icons! {
    Bash => "bash",
    C => "c",
    Cpp => "cpp",
    Css => "css",
    Database => "database",
    Docker => "docker",
    Git => "git",
    Go => "go",
    Html => "html",
    JavaScript => "javascript",
    Json => "json",
    Markdown => "markdown",
    NodeJs => "nodejs",
    Python => "python",
    React => "react",
    Ruby => "ruby",
    Rust => "rust",
    Sass => "sass",
    Svelte => "svelte",
    Swift => "swift",
    TypeScript => "typescript",
    Vue => "vue",
    Yaml => "yml",
    Zig => "zig",
}

pub(crate) struct DesktopAssets;

impl AssetSource for DesktopAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = ASSETS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut assets = gpui_component_assets::Assets.list(path)?;
        assets.extend(
            ASSETS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(assets)
    }
}

pub(crate) fn icon(path: &Path, muted: Hsla) -> gpui::AnyElement {
    resolve(path).render(muted)
}

fn resolve(path: &Path) -> FileIcon {
    use FileIcon::*;

    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match name.as_str() {
        ".gitattributes" | ".gitconfig" | ".gitignore" | ".gitmodules" => return Git,
        "cargo.toml" | "cargo.lock" => return Rust,
        "dockerfile" | "containerfile" | ".dockerignore" => return Docker,
        "go.mod" | "go.sum" | "go.work" => return Go,
        "package.json" | ".nvmrc" | ".node-version" => return NodeJs,
        "package-lock.json" | "npm-shrinkwrap.json" => return Json,
        "readme" | "license" | "copying" | "notice" | "changelog" => {
            return Generic(IconName::BookOpen);
        }
        "makefile" | "justfile" => return Generic(IconName::SquareTerminal),
        ".editorconfig" | ".env" | ".npmrc" => return Generic(IconName::Settings2),
        _ => {}
    }

    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" | "jsonc" if name == "tsconfig.json" || name.starts_with("tsconfig.") => TypeScript,
        "rs" => Rust,
        "c" | "h" => C,
        "cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx" => Cpp,
        "go" => Go,
        "swift" => Swift,
        "py" | "pyi" => Python,
        "rb" => Ruby,
        "zig" => Zig,
        "js" | "mjs" | "cjs" => JavaScript,
        "ts" | "mts" | "cts" => TypeScript,
        "jsx" | "tsx" => React,
        "vue" => Vue,
        "svelte" => Svelte,
        "sh" | "bash" | "zsh" | "fish" => Bash,
        "html" | "htm" => Html,
        "css" | "less" => Css,
        "scss" | "sass" => Sass,
        "json" | "jsonc" => Json,
        "yaml" | "yml" => Yaml,
        "md" | "mdx" => Markdown,
        "sql" | "sqlite" => Database,
        "cs" | "java" | "kt" | "php" | "lua" | "ps1" => Generic(IconName::SquareTerminal),
        "toml" | "ini" | "cfg" | "conf" | "env" | "lock" => Generic(IconName::Settings2),
        "rst" | "adoc" | "pdf" | "doc" | "docx" | "odt" => Generic(IconName::BookOpen),
        "txt" | "log" | "otf" | "ttf" | "woff" | "woff2" => Generic(IconName::CaseSensitive),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "heic" | "ico" | "svg"
        | "tif" | "tiff" => Generic(IconName::Frame),
        "mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" | "mp4" | "mov" | "mkv" | "avi" | "webm" => {
            Generic(IconName::GalleryVerticalEnd)
        }
        "csv" | "tsv" | "xls" | "xlsx" | "ods" | "parquet" => Generic(IconName::ChartPie),
        "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" | "7z" | "rar" => Generic(IconName::Inbox),
        _ => Generic(IconName::File),
    }
}
