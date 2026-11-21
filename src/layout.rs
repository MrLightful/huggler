//! Filesystem locations chosen by the embedding application.
use crate::{Error, Result};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

/// Base directory for newly reserved managed Git checkouts.
#[derive(Clone, Debug)]
pub enum WorktreeRoot {
    /// An absolute directory, or a path relative to `HugglerOptions::state_dir`.
    Shared(PathBuf),
    /// A directory relative to the registered project root. Delegated threads
    /// and forks keep their source thread's project.
    ProjectRelative(PathBuf),
}

/// Locations for Huggler-owned files. Existing reservations retain their absolute paths.
#[derive(Clone, Debug)]
pub struct StorageLayout {
    /// SQLite file, absolute or relative to the state directory. The owner lock
    /// and attachment cache sit beside it. SQLite creates any journal, WAL and
    /// shared-memory files there.
    pub database_path: PathBuf,
    /// Directory shared by isolated delegations, checkpoint forks and allocated workspaces.
    pub worktree_root: WorktreeRoot,
    /// One directory name containing `{id}`, replaced by Huggler's generated UUID.
    pub worktree_name: String,
}

const DEFAULT_DATABASE: &str = "huggler.sqlite3";

impl Default for StorageLayout {
    fn default() -> Self {
        Self {
            database_path: DEFAULT_DATABASE.into(),
            worktree_root: WorktreeRoot::Shared("worktrees".into()),
            worktree_name: "{id}".into(),
        }
    }
}

impl StorageLayout {
    pub(crate) fn validate(&self) -> Result<()> {
        if !self.worktree_name.contains("{id}")
            || Path::new(&self.worktree_name).file_name() != Some(self.worktree_name.as_ref())
            || self.worktree_name.contains('\0')
        {
            return Err(Error::Invalid(
                "worktree_name must be one directory name containing {id}".into(),
            ));
        }
        if let WorktreeRoot::ProjectRelative(path) = &self.worktree_root
            && path
                .components()
                .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
        {
            return Err(Error::Invalid(
                "expected a project-relative worktree root".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn state_files(&self, state_dir: &Path) -> Result<(PathBuf, PathBuf)> {
        let file = self
            .database_path
            .file_name()
            .ok_or_else(|| Error::Invalid("expected a database filename".into()))?;
        let path = state_dir.join(&self.database_path);
        let parent = private_dir(path.parent().unwrap())?;
        let path = parent.join(file);
        // Resolve existing aliases before choosing the corresponding owner lock.
        let database = match fs::symlink_metadata(&path) {
            Ok(_) => {
                let database = path.canonicalize()?;
                let metadata = fs::metadata(&database)?;
                if !metadata.is_file() {
                    return Err(Error::Invalid("expected a regular database file".into()));
                }
                // Hard links can give one database different lock and WAL files.
                #[cfg(unix)]
                if std::os::unix::fs::MetadataExt::nlink(&metadata) > 1 {
                    return Err(Error::Invalid(
                        "expected a database with a single hard link".into(),
                    ));
                }
                database
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => path,
            Err(e) => return Err(e.into()),
        };
        let filename = database
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::Invalid("expected a UTF-8 database filename".into()))?;
        let lower = filename.to_ascii_lowercase();
        if [".lock", "-journal", "-shm", "-wal"]
            .iter()
            .any(|s| lower.ends_with(s))
        {
            return Err(Error::Invalid(
                "database filename overlaps a reserved sidecar name".into(),
            ));
        }
        let lock = if lower == DEFAULT_DATABASE {
            database.with_file_name("runtime.lock")
        } else {
            database.with_file_name(format!("{filename}.runtime.lock"))
        };
        Ok((database, lock))
    }

    pub(crate) fn checkout(&self, state_dir: &Path, project: &Path, id: &str) -> Result<PathBuf> {
        let directory = match &self.worktree_root {
            WorktreeRoot::Shared(path) => state_dir.join(path),
            WorktreeRoot::ProjectRelative(path) => project.join(path),
        };
        let path = private_dir(&directory)?.join(self.worktree_name.replace("{id}", id));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
            Err(error) => Err(error.into()),
            Ok(_) => Err(Error::Invalid(format!(
                "worktree path already exists: {}",
                path.display()
            ))),
        }
    }
}

pub(crate) fn private_dir(path: &Path) -> Result<PathBuf> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.recursive(true).create(path)?;
    Ok(path.canonicalize()?)
}
