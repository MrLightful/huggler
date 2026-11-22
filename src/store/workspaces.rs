use super::*;
use crate::{
    workspace::{
        OrphanedWorktreeEntry, OrphanedWorktreeOutcome, OrphanedWorktreeReport, SetupStatus,
        WorktreeBranch,
    },
    worktree::{self, git},
};
use std::fs::OpenOptions;

#[derive(Clone)]
struct OrphanedWorktree {
    path: PathBuf,
    common_dir: String,
    base_commit: String,
}

struct WorktreeMutationGuard {
    locks: Vec<PathBuf>,
}

impl WorktreeMutationGuard {
    fn acquire(worktree: &OrphanedWorktree) -> Result<Self> {
        let mut locks = Vec::new();
        for name in ["index", "HEAD"] {
            let path = git(
                &worktree.path,
                &["rev-parse", "--path-format=absolute", "--git-path", name],
            )?;
            let mut lock = PathBuf::from(path).into_os_string();
            lock.push(".lock");
            let lock = PathBuf::from(lock);
            match OpenOptions::new().write(true).create_new(true).open(&lock) {
                Ok(_) => locks.push(lock),
                Err(error) => {
                    for acquired in &locks {
                        let _ = std::fs::remove_file(acquired);
                    }
                    return Err(if error.kind() == std::io::ErrorKind::AlreadyExists {
                        Error::Busy
                    } else {
                        error.into()
                    });
                }
            }
        }
        Ok(Self { locks })
    }
}

impl Drop for WorktreeMutationGuard {
    fn drop(&mut self) {
        for lock in &self.locks {
            let _ = std::fs::remove_file(lock);
        }
    }
}

fn directory_bytes(root: &Path) -> Option<u64> {
    let mut total = 0u64;
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_file() {
            total = total.checked_add(metadata.len())?;
            continue;
        }
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path).ok()? {
                pending.push(entry.ok()?.path());
            }
        }
    }
    Some(total)
}

fn inspect_orphaned_worktree(worktree: &OrphanedWorktree) -> OrphanedWorktreeEntry {
    let bytes = if worktree.path.exists() {
        directory_bytes(&worktree.path)
    } else {
        Some(0)
    };
    let entry = |outcome, reason: &str| OrphanedWorktreeEntry {
        path: worktree.path.clone(),
        outcome,
        reason: reason.into(),
        bytes,
    };
    let metadata = match std::fs::symlink_metadata(&worktree.path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return entry(
                OrphanedWorktreeOutcome::Removable,
                "managed checkout is already missing",
            );
        }
        Err(error) => {
            return entry(
                OrphanedWorktreeOutcome::Kept,
                &format!("checkout could not be inspected: {error}"),
            );
        }
    };
    if metadata.file_type().is_symlink() {
        return entry(OrphanedWorktreeOutcome::Kept, "checkout is a symbolic link");
    }
    if !metadata.is_dir() {
        return entry(OrphanedWorktreeOutcome::Kept, "checkout is not a directory");
    }
    let canonical = match worktree.path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            return entry(
                OrphanedWorktreeOutcome::Kept,
                &format!("checkout could not be resolved: {error}"),
            );
        }
    };
    if canonical != worktree.path {
        return entry(
            OrphanedWorktreeOutcome::Kept,
            "checkout path changed since reservation",
        );
    }
    if !worktree.path.join(".git").is_file() {
        return entry(
            OrphanedWorktreeOutcome::Kept,
            "checkout is not a linked worktree",
        );
    }
    let current_common_dir = match worktree::common_dir(&worktree.path) {
        Ok(path) => path,
        Err(error) => {
            return entry(
                OrphanedWorktreeOutcome::Kept,
                &format!("Git repository could not be inspected: {error}"),
            );
        }
    };
    if current_common_dir != worktree.common_dir {
        return entry(
            OrphanedWorktreeOutcome::Kept,
            "Git repository changed since reservation",
        );
    }
    let head = match git(&worktree.path, &["rev-parse", "--verify", "HEAD^{commit}"]) {
        Ok(head) => head,
        Err(error) => {
            return entry(
                OrphanedWorktreeOutcome::Kept,
                &format!("Git commit could not be inspected: {error}"),
            );
        }
    };
    if head != worktree.base_commit {
        return entry(
            OrphanedWorktreeOutcome::Kept,
            "checkout has commits beyond its recorded base",
        );
    }
    let tracked_flags = match git(&worktree.path, &["ls-files", "-v", "-z"]) {
        Ok(flags) => flags,
        Err(error) => {
            return entry(
                OrphanedWorktreeOutcome::Kept,
                &format!("Git index flags could not be inspected: {error}"),
            );
        }
    };
    if tracked_flags.split('\0').any(|record| {
        record
            .as_bytes()
            .first()
            .is_some_and(|flag| flag.is_ascii_lowercase() || *flag == b'S')
    }) {
        return entry(
            OrphanedWorktreeOutcome::Kept,
            "checkout has tracked files hidden from Git status",
        );
    }
    let status = match git(
        &worktree.path,
        &[
            "status",
            "--porcelain",
            "--ignored",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    ) {
        Ok(status) => status,
        Err(error) => {
            return entry(
                OrphanedWorktreeOutcome::Kept,
                &format!("Git status could not be inspected: {error}"),
            );
        }
    };
    if !status.is_empty() {
        return entry(
            OrphanedWorktreeOutcome::Kept,
            "checkout has tracked, untracked or ignored files",
        );
    }
    entry(
        OrphanedWorktreeOutcome::Removable,
        "checkout is unchanged and unreferenced",
    )
}

impl Database {
    fn orphaned_worktrees(&self) -> Result<Vec<OrphanedWorktree>> {
        let mut result = Vec::new();
        let mut worktrees = self.db.prepare(
            "SELECT w.path,w.common_dir,w.base_commit FROM worktree w
             WHERE NOT EXISTS(SELECT 1 FROM thread t WHERE t.worktree_path=w.path)
             AND NOT EXISTS(SELECT 1 FROM project p WHERE p.root=w.path)
             ORDER BY w.path",
        )?;
        let rows = worktrees.query_map([], |row| {
            Ok(OrphanedWorktree {
                path: PathBuf::from(row.get::<_, String>(0)?),
                common_dir: row.get(1)?,
                base_commit: row.get(2)?,
            })
        })?;
        result.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        Ok(result)
    }

    pub fn inspect_orphaned_worktrees(&self) -> Result<OrphanedWorktreeReport> {
        Ok(OrphanedWorktreeReport {
            entries: self
                .orphaned_worktrees()?
                .iter()
                .map(inspect_orphaned_worktree)
                .collect(),
            bytes_freed: 0,
        })
    }

    pub fn cleanup_orphaned_worktrees(&mut self) -> Result<OrphanedWorktreeReport> {
        let mut report = OrphanedWorktreeReport::default();
        for worktree in self.orphaned_worktrees()? {
            let inspected = inspect_orphaned_worktree(&worktree);
            if inspected.outcome != OrphanedWorktreeOutcome::Removable {
                report.entries.push(inspected);
                continue;
            }
            let existed = worktree.path.exists();
            let guard = if existed {
                match WorktreeMutationGuard::acquire(&worktree) {
                    Ok(guard) => Some(guard),
                    Err(error) => {
                        report.entries.push(OrphanedWorktreeEntry {
                            outcome: OrphanedWorktreeOutcome::Failed,
                            reason: format!("checkout is being changed: {error}"),
                            ..inspected
                        });
                        continue;
                    }
                }
            } else {
                None
            };
            let rechecked = inspect_orphaned_worktree(&worktree);
            if rechecked.outcome != OrphanedWorktreeOutcome::Removable {
                report.entries.push(rechecked);
                continue;
            }
            let result = (|| -> Result<()> {
                let tx = self.db.transaction()?;
                let removed = tx.execute(
                    "DELETE FROM worktree WHERE path=? AND NOT EXISTS(
                       SELECT 1 FROM thread WHERE worktree_path=?
                     ) AND NOT EXISTS(SELECT 1 FROM project WHERE root=?)",
                    params![
                        worktree.path.to_str(),
                        worktree.path.to_str(),
                        worktree.path.to_str()
                    ],
                )?;
                if removed != 1 {
                    return Err(Error::Busy);
                }
                if existed {
                    let path = worktree.path.to_str().ok_or_else(|| {
                        Error::Invalid("managed worktree path requires UTF-8".into())
                    })?;
                    git(
                        Path::new(&worktree.common_dir),
                        &["worktree", "remove", "--", path],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })();
            drop(guard);
            match result {
                Ok(()) => {
                    report.bytes_freed = report
                        .bytes_freed
                        .saturating_add(inspected.bytes.unwrap_or(0));
                    report.entries.push(OrphanedWorktreeEntry {
                        outcome: OrphanedWorktreeOutcome::Removed,
                        reason: if existed {
                            "removed unchanged managed checkout".into()
                        } else {
                            "removed stale managed checkout reservation".into()
                        },
                        ..rechecked
                    });
                }
                Err(error) => report.entries.push(OrphanedWorktreeEntry {
                    outcome: OrphanedWorktreeOutcome::Failed,
                    reason: error.to_string(),
                    ..rechecked
                }),
            }
        }
        Ok(report)
    }

    pub fn allocate_workspace(
        &mut self,
        source: &ProjectId,
        revision: &str,
        branch: Option<WorktreeBranch<'_>>,
    ) -> Result<Project> {
        let root = self.project_root(source)?;
        let (repository, commit) = worktree::base_at(&root, revision)?;
        let named_branch = branch
            .map(|naming| worktree::resolve_new_branch(&root, naming).map(|name| (naming, name)))
            .transpose()?;
        let checkout_path = |id: &str| self.storage.checkout(&self.state_dir, &root, id);
        let path = match &named_branch {
            Some((_, name)) => match checkout_path(&worktree::named_checkout_id(name, 48)) {
                // A custom template can leave only enough room for the usual 36-byte UUID.
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::InvalidFilename => {
                    checkout_path(&worktree::named_checkout_id(name, 23))
                }
                result => result,
            },
            None => checkout_path(&new_id()),
        }?;
        let checkout = Worktree {
            path: path
                .to_str()
                .ok_or_else(|| Error::Invalid("expected UTF-8 workspace".into()))?
                .into(),
            common_dir: repository.clone(),
            base_commit: commit.clone(),
            ready: false,
        };
        // Reserve the path before Git touches the filesystem. After a crash the host can
        // enumerate the reservation and explicitly release a missing or clean checkout.
        let project = Project {
            id: ProjectId(new_id()),
            root: path,
        };
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO project(id,root) VALUES(?,?)",
            params![project.id.0, checkout.path],
        )?;
        tx.execute(
            "INSERT INTO allocated_workspace VALUES(?,?,?)",
            params![project.id.0, repository, commit],
        )?;
        tx.commit()?;
        if let Err(error) = checkout.prepare() {
            if !project.root.try_exists()? {
                self.db
                    .execute("DELETE FROM project WHERE id=?", [&project.id.0])?;
            }
            return Err(error);
        }
        if let Some((naming, name)) = named_branch
            && let Err(error) = worktree::attach_new_branch(&project.root, naming, &name)
        {
            return match self.release_workspace(&project.id) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(Error::Invalid(format!(
                    "{error}. Could not release {}. {cleanup}",
                    project.root.display()
                ))),
            };
        }
        Ok(project)
    }
    pub fn allocated_workspaces(&self, source: &ProjectId) -> Result<Vec<Project>> {
        let repository = worktree::repository(&self.project_root(source)?)?;
        let mut stmt = self.db.prepare("SELECT p.id,p.root FROM allocated_workspace a JOIN project p ON p.id=a.project WHERE a.common_dir=? ORDER BY p.root")?;
        Ok(stmt
            .query_map([repository], |r| {
                Ok(Project {
                    id: ProjectId(r.get(0)?),
                    root: PathBuf::from(r.get::<_, String>(1)?),
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn release_workspace(&mut self, project: &ProjectId) -> Result<()> {
        let root = self.project_root(project)?;
        let (repository, commit): (String, String) = self
            .db
            .query_row(
                "SELECT common_dir,base_commit FROM allocated_workspace WHERE project=?",
                [&project.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::Forbidden)?;
        let tx = self.db.transaction()?;
        let in_use = tx
            .prepare(
                "SELECT 1 FROM thread t JOIN project p ON p.id=t.project
             WHERE t.project=?1 OR t.worktree_path=?2 OR p.root=?2
             UNION ALL SELECT 1 FROM workspace_setup WHERE project=?1 AND state='running'",
            )?
            .exists(params![project.0, root.to_str()])?;
        if in_use {
            return Err(Error::Busy);
        }
        // The allocation row owns this immutable project path, including after a
        // layout change. Unmanaged projects cannot reach the Git removal below.
        if root.try_exists()? {
            if root.canonicalize()? != root
                || !root.join(".git").is_file()
                || worktree::clean_base(&root)? != (repository.clone(), commit)
                || !git(
                    &root,
                    &[
                        "status",
                        "--porcelain",
                        "--ignored",
                        "--untracked-files=all",
                    ],
                )?
                .is_empty()
            {
                return Err(Error::Invalid("checkout changed since allocation".into()));
            }
            git(
                Path::new(&repository),
                &["worktree", "remove", "--", root.to_str().unwrap()],
            )?;
        }
        tx.execute("DELETE FROM project WHERE id=?", [&project.0])?;
        tx.commit()?;
        Ok(())
    }
    pub fn begin_setup(&self, project: &ProjectId) -> Result<PathBuf> {
        let path = self.project_root(project)?;
        let has_runs = self
            .db
            .prepare(
                "SELECT 1 FROM run r JOIN thread t ON t.id=r.thread_id JOIN project p ON p.id=t.project
                 WHERE coalesce(t.worktree_path,p.root)=? AND r.status IN ('queued','starting','running','waiting')",
            )?
            .exists([path.to_str()])?;
        if has_runs {
            return Err(Error::Busy);
        }
        let started = self.db.execute(
            "INSERT INTO workspace_setup(project,state) VALUES(?,'running')
             ON CONFLICT(project) DO UPDATE SET state='running', detail=NULL WHERE state!='running'",
            [&project.0],
        )?;
        if started == 0 {
            return Err(Error::Busy);
        }
        Ok(path)
    }
    pub fn end_setup(&self, project: &ProjectId, failure: Option<String>) -> Result<SetupStatus> {
        self.db.execute(
            "UPDATE workspace_setup SET state=?, detail=? WHERE project=?",
            params![
                if failure.is_some() { "failed" } else { "ready" },
                failure,
                project.0
            ],
        )?;
        Ok(failure.map_or(SetupStatus::Ready, SetupStatus::Failed))
    }
    pub fn setup_status(&self, project: &ProjectId) -> Result<Option<SetupStatus>> {
        self.project_root(project)?;
        Ok(self
            .db
            .query_row(
                "SELECT state,detail FROM workspace_setup WHERE project=?",
                [&project.0],
                |r| match r.get::<_, String>(0)?.as_str() {
                    "ready" => Ok(SetupStatus::Ready),
                    "failed" => Ok(SetupStatus::Failed(r.get(1)?)),
                    _ => Ok(SetupStatus::Running),
                },
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repository(root: &Path) -> Result<(String, String)> {
        fs::create_dir(root)?;
        git(root, &["init", "-q"])?;
        fs::write(root.join(".gitignore"), ".env\n")?;
        fs::write(root.join("tracked"), "base")?;
        git(root, &["add", ".gitignore", "tracked"])?;
        git(
            root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "fixture",
            ],
        )?;
        worktree::base(root)
    }

    fn reserve_orphan(
        db: &Database,
        common_dir: &str,
        base_commit: &str,
        name: &str,
    ) -> Result<PathBuf> {
        let path = db.state_dir.join("worktrees").join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        git(
            Path::new(common_dir),
            &[
                "worktree",
                "add",
                "--detach",
                path.to_str().unwrap(),
                base_commit,
            ],
        )?;
        db.db.execute(
            "INSERT INTO worktree(path,common_dir,base_commit,ready) VALUES(?,?,?,1)",
            params![path.to_str(), common_dir, base_commit],
        )?;
        Ok(path)
    }

    #[test]
    fn cleanup_removes_only_unchanged_orphaned_thread_worktrees() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let repo = dir.path().join("repo");
        let (common_dir, base_commit) = repository(&repo)?;
        let mut db = Database::open(&dir.path().join("state"))?;
        let clean = reserve_orphan(&db, &common_dir, &base_commit, "clean")?;
        let changed = reserve_orphan(&db, &common_dir, &base_commit, "changed")?;
        let committed = reserve_orphan(&db, &common_dir, &base_commit, "committed")?;
        let ignored = reserve_orphan(&db, &common_dir, &base_commit, "ignored")?;
        let hidden = reserve_orphan(&db, &common_dir, &base_commit, "hidden")?;
        fs::write(changed.join("local"), "keep")?;
        fs::write(committed.join("tracked"), "commit")?;
        git(&committed, &["add", "tracked"])?;
        git(
            &committed,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "work",
            ],
        )?;
        fs::write(ignored.join(".env"), "secret")?;
        git(&hidden, &["update-index", "--assume-unchanged", "tracked"])?;
        fs::write(hidden.join("tracked"), "hidden edit")?;

        let inspected = db.inspect_orphaned_worktrees()?;
        assert_eq!(inspected.entries.len(), 5);
        assert_eq!(
            inspected
                .entries
                .iter()
                .find(|entry| entry.path == clean)
                .unwrap()
                .outcome,
            OrphanedWorktreeOutcome::Removable
        );
        assert!(
            inspected
                .entries
                .iter()
                .filter(|entry| {
                    entry.path != clean && entry.outcome == OrphanedWorktreeOutcome::Kept
                })
                .count()
                == 4
        );
        assert_eq!(
            inspected
                .entries
                .iter()
                .find(|entry| entry.path == hidden)
                .unwrap()
                .reason,
            "checkout has tracked files hidden from Git status"
        );

        let report = db.cleanup_orphaned_worktrees()?;
        assert_eq!(report.entries.len(), 5);
        assert!(report.bytes_freed > 0);
        assert!(!clean.exists());
        assert!(changed.exists());
        assert!(committed.exists());
        assert!(ignored.exists());
        assert!(hidden.exists());
        let stored: i64 = db
            .db
            .query_row("SELECT count(*) FROM worktree", [], |row| row.get(0))?;
        assert_eq!(stored, 4);
        Ok(())
    }

    #[test]
    fn cleanup_removes_missing_reservations() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let repo = dir.path().join("repo");
        let (common_dir, base_commit) = repository(&repo)?;
        let mut db = Database::open(&dir.path().join("state"))?;
        let missing = db.state_dir.join("worktrees/missing");
        db.db.execute(
            "INSERT INTO worktree(path,common_dir,base_commit) VALUES(?,?,?)",
            params![missing.to_str(), common_dir, base_commit],
        )?;

        let report = db.cleanup_orphaned_worktrees()?;
        assert_eq!(report.entries.len(), 1);
        assert!(
            report
                .entries
                .iter()
                .all(|entry| entry.outcome == OrphanedWorktreeOutcome::Removed)
        );
        assert!(!missing.exists());
        assert_eq!(
            db.db
                .query_row("SELECT count(*) FROM worktree", [], |row| row
                    .get::<_, i64>(0))?,
            0
        );
        Ok(())
    }

    #[test]
    fn cleanup_defers_while_git_is_changing_a_worktree() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let repo = dir.path().join("repo");
        let (common_dir, base_commit) = repository(&repo)?;
        let mut db = Database::open(&dir.path().join("state"))?;
        let path = reserve_orphan(&db, &common_dir, &base_commit, "locked")?;
        let index = git(
            &path,
            &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        )?;
        let lock = PathBuf::from(format!("{index}.lock"));
        fs::write(&lock, "in progress")?;

        let report = db.cleanup_orphaned_worktrees()?;
        assert_eq!(report.entries[0].outcome, OrphanedWorktreeOutcome::Failed);
        assert!(path.exists());

        fs::remove_file(lock)?;
        let report = db.cleanup_orphaned_worktrees()?;
        assert_eq!(report.entries[0].outcome, OrphanedWorktreeOutcome::Removed);
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn inspection_excludes_worktrees_still_referenced_by_a_thread() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let repo = dir.path().join("repo");
        let (common_dir, base_commit) = repository(&repo)?;
        let mut db = Database::open(&dir.path().join("state"))?;
        let path = reserve_orphan(&db, &common_dir, &base_commit, "referenced")?;
        let project = db.project(&repo)?;
        let thread = ThreadId("thread".into());
        db.dispatch(&Command {
            id: CommandId("launch".into()),
            thread_id: thread.clone(),
            action: Action::Launch {
                project: project.id,
                name: "Thread".into(),
                model: None,
            },
        })?;
        db.db.execute(
            "UPDATE thread SET worktree_path=? WHERE id=?",
            params![path.to_str(), thread.0],
        )?;

        assert!(db.inspect_orphaned_worktrees()?.entries.is_empty());
        assert!(db.cleanup_orphaned_worktrees()?.entries.is_empty());
        assert!(path.exists());
        Ok(())
    }
}
