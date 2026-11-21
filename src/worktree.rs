//! Preparation for locally delegated Git worktrees.
use crate::{Error, Result};
use std::{fs, path::Path, process::Command};

fn command(cwd: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        // Checkout preparation belongs to the host, before the provider starts.
        .args(["-c", "core.hooksPath=/dev/null"])
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = command(cwd).args(args).output()?;
    if !output.status.success() {
        return Err(Error::Invalid(format!(
            "git {} in {}: {}",
            args[0],
            cwd.display(),
            String::from_utf8_lossy(&output.stderr).trim_end()
        )));
    }
    String::from_utf8(output.stdout)
        .map(|s| s.strip_suffix('\n').unwrap_or(&s).to_owned())
        .map_err(|_| Error::Invalid("Git worktree output requires UTF-8".into()))
}

fn common_dir(cwd: &Path) -> Result<String> {
    git(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
}

/// Resolve only for a fresh receipt, in the caller's effective checkout.
pub(crate) fn base(cwd: &Path) -> Result<(String, String)> {
    // An empty prefix identifies the checkout root. Split off the hash
    // from the end because the repository directory can contain newlines.
    let revision = git(
        cwd,
        &[
            "rev-parse",
            "--show-prefix",
            "--path-format=absolute",
            "--git-common-dir",
            "--verify",
            "HEAD^{commit}",
        ],
    )?;
    let (repository, commit) = revision
        .strip_prefix('\n')
        .and_then(|s| s.rsplit_once('\n'))
        .ok_or_else(|| Error::Invalid("expected a Git checkout root with a commit".into()))?;
    Ok((repository.into(), commit.into()))
}

pub(crate) fn clean_base(cwd: &Path) -> Result<(String, String)> {
    let revision = base(cwd)?;
    if !git(
        cwd,
        &[
            "status",
            "--porcelain",
            "--untracked-files=normal",
            "--ignore-submodules=none",
        ],
    )?
    .is_empty()
        || base(cwd)? != revision
    {
        return Err(Error::Invalid(
            "checkpoint requires a clean, stable Git checkout".into(),
        ));
    }
    Ok(revision)
}

pub(crate) fn diff(cwd: &Path, base: &str, head: &str) -> Result<String> {
    use std::{io::Read, process::Stdio};
    const LIMIT: u64 = 1024 * 1024;
    let mut child = command(cwd)
        .args([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            base,
            head,
            "--",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .unwrap()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > LIMIT {
        let _ = child.kill();
    }
    let status = child.wait()?;
    read?;
    if bytes.len() as u64 > LIMIT {
        return Err(Error::Invalid("checkpoint diff exceeds 1 MiB".into()));
    }
    if !status.success() {
        return Err(Error::Invalid(
            "Git could not compare the captured commits".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(crate) struct Worktree {
    pub path: String,
    pub common_dir: String,
    pub base_commit: String,
    pub ready: bool,
}

impl Worktree {
    /// Runs before claiming the provider effect. An interrupted creation may
    /// already be on disk when the unclaimed job is recovered.
    pub fn prepare(&self) -> Result<()> {
        let path = Path::new(&self.path);
        if !self.ready && !path.try_exists()? {
            fs::create_dir_all(
                path.parent()
                    .ok_or_else(|| Error::Invalid("invalid worktree path".into()))?,
            )?;
            git(
                Path::new(&self.common_dir),
                &["worktree", "add", "--detach", &self.path, &self.base_commit],
            )?;
            return Ok(());
        }
        if !path.join(".git").is_file()
            || path.canonicalize()? != path
            || common_dir(path)? != self.common_dir
        {
            return Err(Error::Invalid(format!(
                "expected managed Git worktree at {}",
                self.path
            )));
        }
        if !self.ready {
            // Git locks a worktree while populating it. Only adopt a finished
            // checkout of the reserved commit. Never reset an uncertain one.
            let lock = git(
                path,
                &[
                    "rev-parse",
                    "--path-format=absolute",
                    "--git-path",
                    "locked",
                ],
            )?;
            if Path::new(&lock).try_exists()?
                || git(path, &["rev-parse", "--verify", "HEAD^{commit}"])? != self.base_commit
                || !git(path, &["status", "--porcelain", "--untracked-files=no"])?.is_empty()
            {
                return Err(Error::Invalid(format!(
                    "incomplete Git worktree preparation at {}",
                    self.path
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Command as HostCommand, CommandId, ThreadId, store::Database};

    #[test]
    fn worktree_recovery_prepares_the_reserved_commit_before_provider_claim() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let repo = dir.path().canonicalize()?.join("repo");
        fs::create_dir(&repo)?;
        git(&repo, &["init", "-q"])?;
        let commit = [
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "fixture",
        ];
        fs::write(repo.join("tracked"), "base")?;
        git(&repo, &["add", "tracked"])?;
        git(&repo, &commit)?;
        let state = dir.path().join("state");
        let mut db = Database::open(&state)?;
        let project = db.project(&repo)?;
        let parent = ThreadId("caller".into());
        db.dispatch(&HostCommand {
            id: CommandId("launch".into()),
            thread_id: parent.clone(),
            action: Action::Launch {
                project: project.id,
                name: "Parent".into(),
                model: None,
            },
        })?;
        let (receipt, _) = db.dispatch(&HostCommand {
            id: CommandId("send".into()),
            thread_id: parent,
            action: Action::Send {
                text: "Delegate".into(),
            },
        })?;
        let token = db.grant(&receipt.run_id.unwrap())?;
        let (task, _) = db.delegate(
            &token,
            serde_json::from_value(serde_json::json!({
                "clientRequestId":"work", "task":"Review", "isolation":"worktree"
            }))?,
        )?;
        let child = task.child_thread_id;
        let job = (child.clone(), task.child_run_id);
        drop(db);

        fs::write(repo.join("tracked"), "new parent commit")?;
        git(&repo, &["add", "tracked"])?;
        git(&repo, &commit)?;
        let mut db = Database::open(&state)?;
        assert_eq!(db.ready_jobs()?, std::slice::from_ref(&job));
        let worktree = db.worktree(&child)?.unwrap();
        assert!(!worktree.ready && !Path::new(&worktree.path).exists());
        worktree.prepare()?;
        // Lose the runtime after Git succeeds, before storing readiness or claiming the effect.
        drop(db);
        let mut db = Database::open(&state)?;
        assert_eq!(db.ready_jobs()?, std::slice::from_ref(&job));
        let recovered = db.worktree(&child)?.unwrap();
        assert_eq!(recovered.path, worktree.path);
        let tracked = Path::new(&recovered.path).join("tracked");
        assert_eq!(fs::read_to_string(&tracked)?, "base");
        fs::write(&tracked, "local edit")?;
        assert!(recovered.prepare().is_err());
        assert_eq!(fs::read_to_string(&tracked)?, "local edit");
        fs::write(&tracked, "base")?;
        recovered.prepare()?;
        db.worktree_ready(&recovered.path)?;
        assert_eq!(db.claim(&job.1)?.cwd, Path::new(&recovered.path));
        Ok(())
    }
}
