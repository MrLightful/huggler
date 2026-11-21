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

pub fn pull_request_url(root: &Path) -> Result<String> {
    ensure!(
        root.join(".git").exists(),
        "Create PR requires a Git checkout."
    );
    let branch = execute(git(root).args(["symbolic-ref", "--quiet", "HEAD"]))
        .map_err(|_| anyhow::anyhow!("Switch to a branch before creating a pull request."))?;
    let branch = std::str::from_utf8(&branch)?.trim_end();
    let tracking = execute(git(root).args([
        "for-each-ref",
        "--format=%(refname)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream:track,nobracket)",
        branch,
    ]))?;
    let tracking = std::str::from_utf8(&tracking)?.lines().find_map(|line| {
        let mut fields = line.split('\0');
        (fields.next() == Some(branch)).then_some(fields)
    });
    let message = "Push the branch and set its upstream before creating a pull request.";
    let mut fields = tracking.context(message)?;
    let remote = fields
        .next()
        .filter(|r| !r.is_empty() && *r != ".")
        .context(message)?;
    let head = fields
        .next()
        .and_then(|r| r.strip_prefix("refs/heads/"))
        .context(message)?;
    ensure!(
        fields.next() == Some(""),
        "Sync the branch with its upstream before creating a pull request."
    );
    let url = execute(git(root).args(["remote", "get-url", "--", remote]))?;
    github_compare(std::str::from_utf8(&url)?.trim_end(), head)
        .context("Create PR requires a GitHub.com upstream.")
}

fn github_compare(remote: &str, branch: &str) -> Option<String> {
    let path = if let Some(path) = remote.strip_prefix("git@github.com:") {
        path.to_owned()
    } else {
        let url = url::Url::parse(remote).ok()?;
        if url.host_str() != Some("github.com") || !matches!(url.scheme(), "https" | "ssh") {
            return None;
        }
        url.path().trim_start_matches('/').to_owned()
    };
    let (owner, repo) = path.trim_end_matches('/').split_once('/')?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    for part in [owner, repo] {
        if matches!(part, "" | "." | "..")
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return None;
        }
    }
    let mut url = url::Url::parse("https://github.com").ok()?;
    url.path_segments_mut()
        .ok()?
        .extend([owner, repo, "compare", branch]);
    url.set_query(Some("expand=1"));
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::github_compare;

    #[test]
    fn opens_github_comparisons_for_the_selected_repository_and_branch() {
        for remote in [
            "git@github.com:org/project.git",
            "ssh://git@github.com/org/project.git",
            "https://github.com/org/project",
            "https://sample@example.invalid@github.com/org/project.git",
        ] {
            assert_eq!(
                github_compare(remote, "fix/parser#12").as_deref(),
                Some("https://github.com/org/project/compare/fix%2Fparser%2312?expand=1")
            );
        }
        for remote in [
            "/tmp/project.git",
            "git@gitlab.com:org/project.git",
            "https://github.com/org",
        ] {
            assert!(github_compare(remote, "topic").is_none());
        }
    }
}
