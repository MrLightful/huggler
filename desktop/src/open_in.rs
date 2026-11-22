use anyhow::{Context as _, Result, ensure};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenIn {
    Finder,
    Terminal,
    VsCode,
}

impl OpenIn {
    pub(crate) const OPTIONS: [Self; 3] = [Self::Finder, Self::Terminal, Self::VsCode];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Finder => "Finder",
            Self::Terminal => "Terminal",
            Self::VsCode => "VS Code",
        }
    }

    pub(crate) fn open_workspace(self, path: &Path) -> Result<()> {
        self.open(path, false)
    }

    pub(crate) fn open_file(self, root: &Path, path: &Path) -> Result<()> {
        let (target, reveal) = self.file_target(root, path)?;
        self.open(&target, reveal)
    }

    fn file_target(self, root: &Path, path: &Path) -> Result<(PathBuf, bool)> {
        ensure!(root.is_dir(), "The checkout is unavailable.");
        let path = root.join(path);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if self == Self::Terminal && !metadata.is_dir() => {
                Ok((path.parent().context("Missing folder")?.to_owned(), false))
            }
            Ok(_) => Ok((path, self == Self::Finder)),
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                // Deleted rows can outlive their containing directory.
                let parent = path
                    .ancestors()
                    .skip(1)
                    .take_while(|ancestor| ancestor.starts_with(root))
                    .find(|ancestor| ancestor.is_dir())
                    .context("Could not find a folder for this file")?;
                Ok((parent.to_owned(), false))
            }
            Err(error) => Err(error.into()),
        }
    }

    fn open(self, path: &Path, reveal: bool) -> Result<()> {
        let name = self.label();
        let bundle = match self {
            Self::Finder => "com.apple.finder",
            Self::Terminal => "com.apple.Terminal",
            Self::VsCode => "com.microsoft.VSCode",
        };
        let mut command = Command::new("/usr/bin/open");
        command.args(["-b", bundle]);
        if reveal {
            command.arg("-R");
        }
        let output = command
            .arg(path)
            .output()
            .with_context(|| format!("Could not open {name}"))?;
        ensure!(
            output.status.success(),
            "Could not open {name}. {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_a_changed_file_or_its_containing_folder() {
        let checkout = tempfile::tempdir().unwrap();
        let root = checkout.path();
        let relative = Path::new("src/nested/file with spaces.txt");
        let file = root.join(relative);
        let folder = file.parent().unwrap();
        fs::create_dir_all(folder).unwrap();
        fs::write(&file, "changed").unwrap();

        assert_eq!(
            OpenIn::Finder.file_target(root, relative).unwrap(),
            (file.clone(), true)
        );
        assert_eq!(
            OpenIn::VsCode.file_target(root, relative).unwrap(),
            (file.clone(), false)
        );
        assert_eq!(
            OpenIn::Terminal.file_target(root, relative).unwrap(),
            (folder.to_owned(), false)
        );

        fs::remove_dir_all(folder).unwrap();
        for destination in OpenIn::OPTIONS {
            assert_eq!(
                destination.file_target(root, relative).unwrap(),
                (root.join("src"), false)
            );
        }
    }
}
