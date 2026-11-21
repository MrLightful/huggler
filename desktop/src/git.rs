//! Working-tree operations for the desktop's local Git review.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
    pub group: ChangeGroup,
    pub status: char,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeGroup {
    Conflict,
    Unstaged,
    Staged,
}

pub struct Review {
    pub branch: String,
    pub merging: bool,
    pub files: Vec<File>,
    pub selected: Option<File>,
    pub patch: String,
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

pub struct Publication {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub remotes: Vec<String>,
    pub selected: Option<String>,
    pub synced: bool,
    pub compare_url: Option<String>,
}

/// Read local refs only. The first network operation is the user's explicit push.
pub fn publication(root: &Path, selection: Option<&str>) -> Result<Publication> {
    let status = execute(git(root).args([
        "status",
        "--porcelain=v2",
        "--branch",
        "--untracked-files=no",
    ]))?;
    let header = |prefix: &[u8]| -> Result<&str> {
        let value = status
            .split(|b| *b == b'\n')
            .take_while(|line| line.starts_with(b"# "))
            .find_map(|line| line.strip_prefix(prefix))
            .context("missing Git branch header")?;
        Ok(std::str::from_utf8(value)?)
    };
    let branch = header(b"# branch.head ")?;
    let branch = (branch != "(detached)").then(|| branch.to_owned());
    let head = header(b"# branch.oid ")?;
    let head = (head != "(initial)").then(|| head.to_owned());
    let names = execute(git(root).arg("remote"))?;
    let remotes: Vec<_> = std::str::from_utf8(&names)?
        .lines()
        .map(str::to_owned)
        .collect();
    let mut upstream = String::new();
    let mut synced = false;
    if let Some(branch) = &branch {
        let fields = execute(git(root).args([
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream:track,nobracket)",
            &format!("refs/heads/{branch}"),
        ]))?;
        if !fields.is_empty() {
            let mut parts = std::str::from_utf8(&fields)?
                .trim_end_matches('\n')
                .split('\0');
            upstream = parts.next().unwrap_or_default().to_owned();
            synced =
                parts.next() == Some(&format!("refs/heads/{branch}")) && parts.next() == Some("");
        }
    }
    let selected = selection
        .into_iter()
        .chain([upstream.as_str(), "origin"])
        .find_map(|candidate| remotes.iter().find(|name| *name == candidate))
        .or(remotes.first())
        .cloned();
    synced &= selected.as_ref() == Some(&upstream);
    let compare_url = if let (Some(remote), Some(branch)) = (&selected, &branch) {
        let urls = execute(git(root).args(["remote", "get-url", "--push", "--all", "--", remote]))?;
        let mut lines = std::str::from_utf8(&urls)?.lines();
        lines
            .next()
            .filter(|_| lines.next().is_none())
            .and_then(|url| github_compare(url, branch))
    } else {
        None
    };
    Ok(Publication {
        branch,
        head,
        remotes,
        selected,
        synced,
        compare_url,
    })
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

pub fn push(root: &Path, branch: &str, head: &str, remote: &str) -> Result<()> {
    let current = publication(root, Some(remote))?;
    ensure!(
        current.branch.as_deref() == Some(branch)
            && current.head.as_deref() == Some(head)
            && current.selected.as_deref() == Some(remote),
        "The branch or destination changed. Review it before pushing."
    );
    let reference = format!("refs/heads/{branch}");
    execute(git(root).args([
        "-c",
        &format!("remote.{remote}.mirror=false"),
        "-c",
        "push.followTags=false",
        "push",
        "--recurse-submodules=no",
        "--set-upstream",
        "--",
        remote,
        &format!("{reference}:{reference}"),
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

pub fn read(root: &Path, selection: Option<&File>) -> Result<Review> {
    let output = execute(git(root).args([
        "status",
        "--porcelain=v1",
        "-bz",
        "--no-renames",
        "--untracked-files=all",
    ]))?;
    let mut records = output.split(|b| *b == 0);
    let header = records
        .next()
        .and_then(|s| s.strip_prefix(b"## "))
        .context("missing Git branch")?;
    let mut files = Vec::new();
    for record in records.filter(|r| !r.is_empty()) {
        ensure!(
            record.len() > 3 && record[2] == b' ',
            "invalid Git status record"
        );
        let path = file_path(&record[3..])?;
        if matches!(
            &record[..2],
            b"DD" | b"AU" | b"UD" | b"UA" | b"DU" | b"AA" | b"UU"
        ) {
            files.push(File {
                path,
                group: ChangeGroup::Conflict,
                status: 'U',
            });
            continue;
        }
        for (column, group) in [(0, ChangeGroup::Staged), (1, ChangeGroup::Unstaged)] {
            let status = record[column] as char;
            if status != ' ' && !(group == ChangeGroup::Staged && status == '?') {
                files.push(File {
                    path: path.clone(),
                    group,
                    status,
                });
            }
        }
    }
    let selected = selection
        .and_then(|old| {
            files
                .iter()
                .find(|f| f.path == old.path && f.group == old.group)
                .or_else(|| files.iter().find(|f| f.path == old.path))
        })
        .or_else(|| files.first())
        .cloned();
    let patch = selected.as_ref().map_or_else(String::new, |file| {
        preview(root, file).unwrap_or_else(|error| format!("Preview unavailable: {error}"))
    });
    // Git keeps this file in the individual worktree's metadata, including
    // when the merge result has the same tree as its first parent.
    let merge_head = execute(git(root).args(["rev-parse", "--git-path", "MERGE_HEAD"]))?;
    let merge_head = file_path(merge_head.strip_suffix(b"\n").unwrap_or(&merge_head))?;
    Ok(Review {
        branch: String::from_utf8_lossy(header).into_owned(),
        merging: root.join(merge_head).try_exists()?,
        files,
        selected,
        patch,
    })
}

fn preview(root: &Path, file: &File) -> Result<String> {
    const LIMIT: u64 = 64 * 1024;
    let mut cmd = git(root);
    cmd.args([
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
    ]);
    if file.group == ChangeGroup::Staged {
        cmd.arg("--cached");
    }
    let new_file = file.status == '?';
    if new_file {
        let kind = root.join(&file.path).symlink_metadata()?.file_type();
        if !(kind.is_symlink() || kind.is_file()) {
            return Ok("Preview unavailable for this file type.".into());
        }
        cmd.args(["--no-index", "--", "/dev/null"]);
    } else {
        cmd.arg("--");
    }
    let mut child = cmd
        .arg(&file.path)
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
    let truncated = bytes.len() > LIMIT as usize;
    if truncated || read.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    read?;
    if !(truncated || status.success() || new_file && status.code() == Some(1)) {
        bail!("Git could not preview {}", file.path.display());
    }
    bytes.truncate(LIMIT as usize);
    let mut patch = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        patch.push_str("\n… preview limited to 64 KiB");
    }
    if patch.is_empty() {
        patch.push_str("No textual changes.");
    }
    Ok(patch)
}

pub enum Operation {
    Refresh,
    Stage(File),
    Unstage(File),
    Commit(String),
}

impl Operation {
    pub fn apply(&self, root: &Path) -> Result<Option<String>> {
        let mut cmd = git(root);
        match self {
            Self::Refresh => return Ok(None),
            Self::Stage(file) => {
                cmd.args(["add", "--"]).arg(&file.path);
            }
            Self::Unstage(file) => {
                // git reset's default tree is empty on an unborn branch.
                cmd.args(["reset", "-q", "--"]).arg(&file.path);
            }
            Self::Commit(message) => {
                ensure!(!message.trim().is_empty(), "Enter a commit message.");
                cmd.args(["commit", "-m", message]);
            }
        }
        let bytes = execute(&mut cmd)?;
        Ok(matches!(self, Self::Commit(_)).then(|| {
            String::from_utf8_lossy(&bytes)
                .lines()
                .next()
                .unwrap_or("Committed")
                .to_owned()
        }))
    }
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
