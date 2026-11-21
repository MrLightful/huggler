//! Trusted-host checkout preparation. Git checkout operations are blocking.
use crate::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq)]
pub enum SetupStatus {
    Running,
    Failed(String),
    Ready,
}

/// Result of inspecting or cleaning one unreferenced managed checkout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrphanedWorktreeOutcome {
    /// Inspection found that cleanup can safely remove this checkout.
    Removable,
    /// Cleanup removed the checkout and its Huggler reservation.
    Removed,
    /// Cleanup retained the checkout because it could contain work.
    Kept,
    /// Cleanup attempted removal but the filesystem, Git or database rejected it.
    Failed,
}

/// One unreferenced checkout and the reason it was removed or retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrphanedWorktreeEntry {
    pub path: PathBuf,
    pub outcome: OrphanedWorktreeOutcome,
    pub reason: String,
    /// Best-effort logical file size. Shared and sparse blocks can consume less disk space.
    pub bytes: Option<u64>,
}

/// Inspection or cleanup results for all currently unreferenced managed checkouts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrphanedWorktreeReport {
    pub entries: Vec<OrphanedWorktreeEntry>,
    /// Best-effort logical bytes belonging to successfully removed directories.
    pub bytes_freed: u64,
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
        self.shared.access(|db| db.allocate_workspace(source))
    }
    /// Enumerate managed checkout reservations in the same Git repository as the source.
    pub fn allocated_workspaces(&self, source: &ProjectId) -> Result<Vec<Project>> {
        self.shared.access(|db| db.allocated_workspaces(source))
    }
    /// Release a managed checkout only after its last thread is removed.
    pub fn release_workspace(&self, project: &ProjectId) -> Result<()> {
        self.shared.access(|db| db.release_workspace(project))
    }
    /// Inspect unreferenced Huggler-managed checkouts without changing them.
    ///
    /// Only a checkout at its recorded base commit with no tracked, untracked or
    /// ignored files is removable. Run this blocking filesystem and Git operation
    /// off the host's UI or async executor thread.
    pub fn inspect_orphaned_worktrees(&self) -> Result<OrphanedWorktreeReport> {
        #[cfg(feature = "application")]
        let _services = self.shared.services.lock().unwrap();
        self.shared.access(|db| db.inspect_orphaned_worktrees())
    }
    /// Remove only unreferenced Huggler-managed checkouts proven unchanged.
    ///
    /// The host owns cleanup policy and scheduling. A typical host calls this at
    /// startup, after deleting a thread, periodically for retries, or on explicit
    /// user request. Changed and uncertain checkouts are retained and reported.
    pub fn cleanup_orphaned_worktrees(&self) -> Result<OrphanedWorktreeReport> {
        #[cfg(feature = "application")]
        let _services = self.shared.services.lock().unwrap();
        self.shared.access(|db| db.cleanup_orphaned_worktrees())
    }
    pub fn workspace_setup_status(&self, project: &ProjectId) -> Result<Option<SetupStatus>> {
        self.shared.access(|db| db.setup_status(project))
    }
}
