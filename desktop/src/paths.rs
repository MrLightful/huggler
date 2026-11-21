use anyhow::{Context as _, Result};
use huggler::{HugglerOptions, StorageLayout};
use std::path::{Path, PathBuf};

const USERDATA: &str = "userdata";

pub(crate) struct StoragePaths {
    base: PathBuf,
    userdata: bool,
}

impl StoragePaths {
    pub(crate) fn discover() -> Result<Self> {
        Self::resolve(
            std::env::var_os("HUGGLER_STATE_DIR").map(PathBuf::from),
            std::env::var_os("HUGGLER_HOME")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            dirs::home_dir().as_deref(),
            dirs::data_dir().map(|path| path.join(crate::APP_ID)),
        )
    }

    fn resolve(
        state_override: Option<PathBuf>,
        home_override: Option<PathBuf>,
        home: Option<&Path>,
        legacy: Option<PathBuf>,
    ) -> Result<Self> {
        if let Some(base) = state_override {
            return Ok(Self {
                base,
                userdata: false,
            });
        }
        if let Some(base) = home_override {
            return Ok(Self {
                base,
                userdata: true,
            });
        }
        let default = home.filter(|path| path.is_absolute()).map(|home| Self {
            base: home.join(".huggler"),
            userdata: true,
        });

        // Keep existing databases in place so saved worktree and scratch paths still resolve.
        let legacy = legacy.filter(|path| path.is_absolute());
        if let Some(base) = legacy {
            let use_legacy = match &default {
                Some(paths) => !database_exists(&paths.state_dir())? && database_exists(&base)?,
                None => true,
            };
            if use_legacy {
                return Ok(Self {
                    base,
                    userdata: false,
                });
            }
        }
        default.context("cannot locate the home directory. Set HUGGLER_HOME to choose one")
    }

    pub(crate) fn state_dir(&self) -> PathBuf {
        self.base.join(self.state_subdirectory())
    }

    pub(crate) fn scratch_dir(&self) -> PathBuf {
        self.base.join("scratch")
    }

    pub(crate) fn runtime_options(&self) -> HugglerOptions {
        let mut options = HugglerOptions::new(&self.base);
        options.storage.database_path = self
            .state_subdirectory()
            .join(&options.storage.database_path);
        options.output.version = env!("CARGO_PKG_VERSION").into();
        options
    }

    fn state_subdirectory(&self) -> &Path {
        Path::new(if self.userdata { USERDATA } else { "" })
    }
}

fn database_exists(state: &Path) -> Result<bool> {
    let path = state.join(StorageLayout::default().database_path);
    path.try_exists()
        .with_context(|| format!("cannot check for existing state at {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use huggler::{CommandId, Huggler, ThreadId};
    use std::fs;

    #[test]
    fn fresh_default_separates_userdata_from_checkouts() -> Result<()> {
        let home = tempfile::tempdir()?;
        let root = home.path().canonicalize()?;
        let paths = StoragePaths::resolve(None, None, Some(&root), None)?;
        let base = root.join(".huggler");
        assert_eq!(paths.state_dir(), base.join("userdata"));
        assert_eq!(paths.scratch_dir(), base.join("scratch"));

        tokio::runtime::Runtime::new()?.block_on(async {
            let runtime = Huggler::open(paths.runtime_options()).await?;
            assert!(paths.state_dir().join("huggler.sqlite3").is_file());
            fs::create_dir(paths.scratch_dir())?;
            let scratch = runtime.create_scratch(&paths.scratch_dir().join("conversation"))?;
            assert_eq!(scratch.root, base.join("scratch/conversation"));
            runtime.promote_scratch(&scratch.id)?;
            let status = std::process::Command::new("git")
                .current_dir(&scratch.root)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.com",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--allow-empty",
                    "-qm",
                    "Initial commit",
                ])
                .status()?;
            assert!(status.success());
            let checkout = runtime.allocate_workspace(&scratch.id)?;
            assert_eq!(
                checkout.root.parent(),
                Some(base.join("worktrees").as_path())
            );
            runtime.close().await?;
            Ok(())
        })
    }

    #[test]
    fn legacy_database_reopens_in_place_until_new_default_exists() -> Result<()> {
        let home = tempfile::tempdir()?;
        let legacy = home.path().join("legacy");
        let executor = tokio::runtime::Runtime::new()?;
        executor.block_on(async {
            let runtime = Huggler::open(HugglerOptions::new(&legacy)).await?;
            let project = runtime.create_scratch(&legacy.join("original-scratch"))?;
            let thread = ThreadId("existing conversation".into());
            runtime.launch(
                CommandId("create conversation".into()),
                thread.clone(),
                project.id,
                "Existing conversation".into(),
                None,
            )?;
            runtime.close().await?;

            let resolve =
                || StoragePaths::resolve(None, None, Some(home.path()), Some(legacy.clone()));
            let paths = resolve()?;
            assert_eq!(paths.state_dir(), legacy);
            assert_eq!(paths.scratch_dir(), legacy.join("scratch"));
            let runtime = Huggler::open(paths.runtime_options()).await?;
            assert_eq!(runtime.thread_checkout(&thread)?, project.root);
            runtime.close().await?;

            let paths = StoragePaths::resolve(None, None, None, Some(legacy.clone()))?;
            let runtime = Huggler::open(paths.runtime_options()).await?;
            assert_eq!(runtime.thread_checkout(&thread)?, project.root);
            runtime.close().await?;

            let paths = StoragePaths::resolve(None, None, Some(home.path()), None)?;
            let runtime = Huggler::open(paths.runtime_options()).await?;
            assert!(runtime.conversation_list()?.threads.is_empty());
            runtime.close().await?;
            assert_eq!(resolve()?.state_dir(), paths.state_dir());
            Ok(())
        })
    }

    #[test]
    fn relative_home_cannot_create_a_default_under_the_working_directory() {
        let paths = StoragePaths::resolve(
            None,
            None,
            Some(Path::new("relative home")),
            Some(PathBuf::from("relative legacy state")),
        );
        assert!(paths.is_err());
    }

    #[test]
    fn state_override_keeps_flat_layout_and_takes_precedence() -> Result<()> {
        let state = PathBuf::from("state override");
        let home = PathBuf::from("home override");
        let paths = StoragePaths::resolve(Some(state.clone()), Some(home.clone()), None, None)?;
        assert_eq!(paths.state_dir(), state);
        assert_eq!(paths.scratch_dir(), state.join("scratch"));
        assert_eq!(
            paths.runtime_options().storage.database_path,
            Path::new("huggler.sqlite3")
        );

        let paths = StoragePaths::resolve(None, Some(home.clone()), None, None)?;
        assert_eq!(paths.state_dir(), home.join("userdata"));
        assert_eq!(paths.scratch_dir(), home.join("scratch"));
        assert_eq!(
            paths.runtime_options().storage.database_path,
            Path::new("userdata/huggler.sqlite3")
        );
        Ok(())
    }
}
