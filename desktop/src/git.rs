//! Working-tree operations for the desktop's local Git review.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
    pub status: String,
}

fn git(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        // Each conversation already owns a checkout root. Git's normal
        // discovery could walk into a parent repository if this one disappears.
        .args([
            "--git-dir=.git",
            "--work-tree=.",
            "--no-pager",
            "--literal-pathspecs",
        ])
        .stdin(Stdio::null());
    cmd
}

fn execute(cmd: &mut Command) -> Result<Vec<u8>> {
    let output = cmd.output().context("start Git")?;
    if !output.status.success() {
        let details = if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        };
        bail!(
            "Git exited with {}. {}",
            output.status,
            String::from_utf8_lossy(details).trim_end()
        );
    }
    Ok(output.stdout)
}

pub fn validate_new_branch(root: &Path, branch: &str) -> Result<()> {
    let checked = execute(git(root).args(["check-ref-format", "--branch", branch]))?;
    // Git's --branch syntax also expands previous-checkout expressions.
    ensure!(
        checked.strip_suffix(b"\n") == Some(branch.as_bytes()),
        "Enter a literal branch name."
    );
    Ok(())
}

pub fn create_branch(root: &Path, branch: &str) -> Result<()> {
    execute(git(root).args([
        "-c",
        "core.hooksPath=/dev/null",
        "switch",
        "--no-track",
        "-c",
        branch,
    ]))?;
    Ok(())
}

fn file_path(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(OsStr::from_bytes(bytes).into())
    }
    #[cfg(not(unix))]
    Ok(std::str::from_utf8(bytes)?.into())
}

pub fn changed_files(root: &Path) -> Result<Vec<File>> {
    let output = execute(git(root).args([
        "--no-optional-locks",
        "status",
        "--porcelain=v1",
        "-z",
        "--no-renames",
        "--untracked-files=all",
    ]))?;
    output
        .split(|b| *b == 0)
        .filter(|record| !record.is_empty())
        .map(|record| {
            ensure!(
                record.len() > 3 && record[2] == b' ',
                "invalid Git status record"
            );
            Ok(File {
                path: file_path(&record[3..])?,
                status: String::from_utf8_lossy(&record[..2]).trim().to_owned(),
            })
        })
        .collect()
}
