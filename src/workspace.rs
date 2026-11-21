//! Trusted-host checkout preparation. Filesystem and setup operations are blocking.
use crate::*;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

#[derive(Debug, PartialEq)]
pub enum SetupStatus {
    Running,
    Failed(String),
    Ready,
}

impl Huggler {
    /// Clone into a new directory. The caller owns the clone.
    pub fn clone_project(&self, source: &str, destination: &Path) -> Result<Project> {
        let parent = destination
            .parent()
            .ok_or_else(|| Error::Invalid("expected destination parent".into()))?
            .canonicalize()?;
        fs::create_dir(destination)?;
        let path = destination.canonicalize()?;
        crate::worktree::git(
            &parent,
            &[
                "clone",
                "--",
                source,
                path.to_str()
                    .ok_or_else(|| Error::Invalid("expected UTF-8 path".into()))?,
            ],
        )?;
        self.open_project(path)
    }
    /// Register a newly created scratch directory. Promotion preserves its path and identity.
    pub fn create_scratch(&self, path: &Path) -> Result<Project> {
        fs::create_dir(path)?;
        let path = path.canonicalize()?;
        self.shared.access(|db| db.project(&path))
    }
    pub fn promote_scratch(&self, project: &ProjectId) -> Result<Project> {
        let path = self.shared.access(|db| db.project_root(project))?;
        crate::worktree::git(&path, &["init", "-q"])?;
        self.open_project(path)
    }
    /// Allocate a detached checkout of the project's current HEAD.
    pub fn allocate_workspace(&self, source: &ProjectId) -> Result<Project> {
        self.shared.access(|db| db.allocate_workspace(source))
    }
    /// Release a managed checkout only after its last thread is removed.
    pub fn release_workspace(&self, project: &ProjectId) -> Result<()> {
        self.shared.access(|db| db.release_workspace(project))
    }
    /// Run an explicitly selected setup command on an idle project. Failed setup gates new runs
    /// until a successful retry. Exit status is durable. The host owns command selection.
    pub fn setup_workspace(
        &self,
        project: &ProjectId,
        program: &Path,
        args: &[String],
    ) -> Result<SetupStatus> {
        let root = self.shared.access(|db| db.begin_setup(project))?;
        let status = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let failure = match status {
            Ok(status) if status.success() => None,
            Ok(status) => Some(status.to_string()),
            Err(err) => Some(err.to_string()),
        };
        self.shared.access(|db| db.end_setup(project, failure))
    }
    pub fn workspace_setup_status(&self, project: &ProjectId) -> Result<Option<SetupStatus>> {
        self.shared.access(|db| db.setup_status(project))
    }
}
