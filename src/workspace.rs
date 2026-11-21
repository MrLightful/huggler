//! Trusted-host checkout preparation. Git checkout operations are blocking.
use crate::*;
use std::{fs, path::Path};

#[derive(Debug, PartialEq)]
pub enum SetupStatus {
    Running,
    Failed(String),
    Ready,
}

impl Huggler {
    /// Clone into a new directory. The caller owns the clone.
    pub fn clone_project(&self, source: &str, destination: &Path) -> Result<Project> {
        self.shared.access(|_| Ok(()))?;
        let target = destination
            .to_str()
            .ok_or_else(|| Error::Invalid("expected UTF-8 path".into()))?;
        fs::create_dir(destination)?;
        if let Err(error) = crate::worktree::git(Path::new("."), &["clone", "--", source, target]) {
            // Remove only an empty directory created by this call.
            let _ = fs::remove_dir(destination);
            return Err(error);
        }
        self.open_project(destination)
    }
    /// Register a newly created scratch directory. Promotion preserves its path and identity.
    pub fn create_scratch(&self, path: &Path) -> Result<Project> {
        self.shared.access(|db| {
            fs::create_dir(path)?;
            let result = path
                .canonicalize()
                .map_err(Error::from)
                .and_then(|root| db.project(&root));
            if result.is_err() {
                let _ = fs::remove_dir(path);
            }
            result
        })
    }
    pub fn promote_scratch(&self, project: &ProjectId) -> Result<Project> {
        let path = self.shared.access(|db| db.project_root(project))?;
        crate::worktree::git(&path, &["init", "-q"])?;
        self.open_project(path)
    }
    /// Allocate a detached checkout of the project's current HEAD.
    pub fn allocate_workspace(&self, source: &ProjectId) -> Result<Project> {
        self.allocate_workspace_at(source, "HEAD")
    }
    /// Resolve a Git revision in the source repository and allocate a detached checkout of its commit.
    pub fn allocate_workspace_at(&self, source: &ProjectId, revision: &str) -> Result<Project> {
        self.shared
            .access(|db| db.allocate_workspace(source, revision))
    }
    /// Enumerate managed checkout reservations in the same Git repository as the source.
    pub fn allocated_workspaces(&self, source: &ProjectId) -> Result<Vec<Project>> {
        self.shared.access(|db| db.allocated_workspaces(source))
    }
    /// Release a managed checkout only after its last thread is removed.
    pub fn release_workspace(&self, project: &ProjectId) -> Result<()> {
        self.shared.access(|db| db.release_workspace(project))
    }
    pub fn workspace_setup_status(&self, project: &ProjectId) -> Result<Option<SetupStatus>> {
        self.shared.access(|db| db.setup_status(project))
    }
}
