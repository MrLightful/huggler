use anyhow::{Context as _, Result, ensure};
use gpui::SharedString;
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path},
};

const EDIT_LIMIT: usize = 1024 * 1024;

pub(super) fn uses_crlf(text: &str) -> bool {
    text.contains("\r\n") && text.split("\r\n").all(|line| !line.contains(['\r', '\n']))
}

fn validate_path(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && path.components().all(|component| {
                matches!(component, Component::Normal(name) if !name.eq_ignore_ascii_case(".git"))
            }),
        "Expected a workspace file path."
    );
    Ok(())
}

fn read_text(file: &fs::File) -> Result<String> {
    ensure!(
        file.metadata()?.is_file(),
        "Only regular files can be edited."
    );
    let mut bytes = Vec::new();
    file.take((EDIT_LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= EDIT_LIMIT,
        "This file exceeds the 1 MiB editor limit."
    );
    ensure!(!bytes.contains(&0), "Only UTF-8 text files can be edited.");
    String::from_utf8(bytes).context("Only UTF-8 text files can be edited.")
}

pub(super) fn read_source(root: &Path, path: &Path) -> Result<SharedString> {
    validate_path(path)?;
    let target = storage::Target::open(root, path)?;
    Ok(read_text(&target.file(false)?)?.into())
}

pub(super) fn write_source(root: &Path, path: &Path, expected: &str, contents: &str) -> Result<()> {
    validate_path(path)?;
    ensure!(
        contents.len() <= EDIT_LIMIT,
        "This file exceeds the 1 MiB editor limit."
    );
    ensure!(
        !contents.contains('\0'),
        "Only UTF-8 text files can be edited."
    );
    storage::Target::open(root, path)?.replace(expected, contents)
}

const CHANGED_ON_DISK: &str = "The file changed on disk. Copy your edits, then reload the file.";

#[cfg(unix)]
mod storage {
    use super::*;
    use rustix::fs::{AtFlags, Mode, OFlags, open, openat, renameat, unlinkat};
    use std::ffi::OsStr;
    use std::os::unix::fs::MetadataExt;
    use uuid::Uuid;

    pub(super) struct Target<'a> {
        parent: fs::File,
        name: &'a OsStr,
    }

    const DIR: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    const FILE: OFlags = OFlags::NOFOLLOW
        .union(OFlags::NONBLOCK)
        .union(OFlags::CLOEXEC);

    impl<'a> Target<'a> {
        pub(super) fn open(root: &Path, path: &'a Path) -> Result<Self> {
            let mut parent = fs::File::from(open(root.canonicalize()?, DIR, Mode::empty())?);
            for component in path.parent().expect("file parent").components() {
                parent = fs::File::from(
                    openat(&parent, component.as_os_str(), DIR, Mode::empty())
                        .context("A parent folder is unavailable.")?,
                );
            }
            Ok(Self {
                parent,
                name: path.file_name().expect("file name"),
            })
        }

        pub(super) fn file(&self, writing: bool) -> Result<fs::File> {
            let access = if writing {
                OFlags::RDWR
            } else {
                OFlags::RDONLY
            };
            Ok(fs::File::from(
                openat(&self.parent, self.name, FILE | access, Mode::empty())
                    .context("The working file is unavailable.")?,
            ))
        }

        pub(super) fn replace(&self, expected: &str, contents: &str) -> Result<()> {
            let original = self.file(true)?;
            let metadata = original.metadata()?;
            ensure!(read_text(&original)? == expected, CHANGED_ON_DISK);

            // Keep the final rename anchored to the same directory as the read.
            let name = format!(".huggler-save-{}", Uuid::new_v4());
            let mut replacement = fs::File::from(openat(
                &self.parent,
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )?);
            let result = (|| {
                replacement.write_all(contents.as_bytes())?;
                replacement.set_permissions(metadata.permissions())?;
                replacement.sync_all()?;
                let current = self.file(true)?;
                let identity = current.metadata()?;
                ensure!(
                    metadata.dev() == identity.dev()
                        && metadata.ino() == identity.ino()
                        && read_text(&current)? == expected,
                    CHANGED_ON_DISK
                );
                renameat(&self.parent, name.as_str(), &self.parent, self.name)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = unlinkat(&self.parent, name.as_str(), AtFlags::empty());
            }
            result
        }
    }
}

#[cfg(not(unix))]
mod storage {
    use super::*;
    use std::path::PathBuf;

    pub(super) struct Target(PathBuf);

    impl Target {
        pub(super) fn open(root: &Path, path: &Path) -> Result<Self> {
            let path = root.canonicalize()?.join(path);
            ensure!(path.canonicalize()? == path, "The file path has moved.");
            Ok(Self(path))
        }

        pub(super) fn file(&self, writing: bool) -> Result<fs::File> {
            ensure!(
                self.0.canonicalize()? == self.0 && self.0.symlink_metadata()?.is_file(),
                "The working file is unavailable."
            );
            Ok(fs::OpenOptions::new()
                .read(true)
                .write(writing)
                .open(&self.0)?)
        }

        pub(super) fn replace(&self, expected: &str, contents: &str) -> Result<()> {
            let original = self.file(true)?;
            ensure!(read_text(&original)? == expected, CHANGED_ON_DISK);
            let mut replacement =
                tempfile::NamedTempFile::new_in(self.0.parent().expect("file parent"))?;
            replacement.write_all(contents.as_bytes())?;
            replacement
                .as_file()
                .set_permissions(original.metadata()?.permissions())?;
            replacement.as_file().sync_all()?;
            ensure!(read_text(&self.file(true)?)? == expected, CHANGED_ON_DISK);
            replacement.persist(&self.0)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{read_source, uses_crlf, write_source};
    use std::{fs, path::Path};

    #[test]
    fn saves_only_the_version_opened_in_the_editor() {
        let checkout = tempfile::tempdir().unwrap();
        let root = checkout.path();
        let path = Path::new("file.txt");
        fs::write(root.join(path), "first\r\nsecond\r\n").unwrap();
        let opened = read_source(root, path).unwrap();
        assert!(uses_crlf(&opened));
        assert!(!uses_crlf("first\r\nsecond\n"));

        write_source(root, path, &opened, "first\r\nthird\r\n").unwrap();
        assert_eq!(fs::read(root.join(path)).unwrap(), b"first\r\nthird\r\n");
        assert!(write_source(root, path, &opened, "stale edit").is_err());
        assert_eq!(fs::read(root.join(path)).unwrap(), b"first\r\nthird\r\n");
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn saves_executable_mode_and_refuses_symlink_paths() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let checkout = tempfile::tempdir().unwrap();
        let root = checkout.path();
        let path = Path::new("file");
        fs::write(root.join(path), "hello").unwrap();
        fs::set_permissions(root.join(path), fs::Permissions::from_mode(0o755)).unwrap();
        write_source(root, path, "hello", "updated").unwrap();
        assert_eq!(
            fs::metadata(root.join(path)).unwrap().permissions().mode() & 0o777,
            0o755
        );

        symlink(path, root.join("link")).unwrap();
        symlink(root, root.join("directory")).unwrap();
        for path in [Path::new("link"), Path::new("directory/file")] {
            assert!(read_source(root, path).is_err());
            assert!(write_source(root, path, "updated", "unexpected").is_err());
        }
        assert_eq!(fs::read_to_string(root.join(path)).unwrap(), "updated");
    }
}
