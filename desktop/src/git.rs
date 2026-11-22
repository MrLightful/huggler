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

fn checkout_command(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["--no-pager", "--literal-pathspecs"])
        .stdin(Stdio::null());
    cmd
}

fn git(root: &Path) -> Command {
    let mut cmd = checkout_command(root);
    // Each conversation already owns a checkout root. Git's normal
    // discovery could walk into a parent repository if this one disappears.
    cmd.args(["--git-dir=.git", "--work-tree=."]);
    cmd
}

pub fn checkout_root(path: &Path) -> Result<PathBuf> {
    let output = execute(checkout_command(path).args(["rev-parse", "--show-toplevel"]))?;
    file_path(output.strip_suffix(b"\n").unwrap_or(&output))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub reference: String,
    pub name: String,
    pub remote: bool,
    pub current: bool,
}

impl Branch {
    pub fn base_label(&self) -> String {
        match self.reference.strip_prefix("refs/remotes/origin/") {
            Some(name) => format!("From {name} (remote)"),
            None => format!("From {}", self.name),
        }
    }
}

pub struct Branches {
    pub branches: Vec<Branch>,
    pub default: Option<Branch>,
}

pub fn branches(path: &Path) -> Result<Branches> {
    let root = checkout_root(path)?;
    let output = execute(git(&root).args([
        "for-each-ref",
        "--format=%(HEAD)%00%(refname)%00%(symref)",
        "refs/heads/",
        "refs/remotes/",
    ]))?;
    let mut branches = Vec::new();
    let mut remote_heads = Vec::new();
    for line in std::str::from_utf8(&output)?.lines() {
        let mut fields = line.split('\0');
        let (Some(head), Some(reference), Some(target)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let remote = reference.starts_with("refs/remotes/");
        if !target.is_empty() {
            if remote && reference.ends_with("/HEAD") {
                remote_heads.push((reference, target));
            }
            continue;
        }
        let name = reference
            .strip_prefix(if remote {
                "refs/remotes/"
            } else {
                "refs/heads/"
            })
            .context("Expected a Git branch ref")?;
        branches.push(Branch {
            reference: reference.to_owned(),
            name: name.to_owned(),
            remote,
            current: head == "*",
        });
    }
    let lookup = |reference: &str| branches.iter().find(|branch| branch.reference == reference);
    let default = remote_heads
        .iter()
        .find(|(head, _)| *head == "refs/remotes/origin/HEAD")
        .and_then(|(_, target)| lookup(target))
        .or_else(|| remote_heads.iter().find_map(|(_, target)| lookup(target)))
        .or_else(|| {
            ["main", "master"].into_iter().find_map(|name| {
                lookup(&format!("refs/remotes/origin/{name}"))
                    .or_else(|| lookup(&format!("refs/heads/{name}")))
            })
        })
        .or_else(|| branches.iter().find(|branch| branch.current))
        .cloned()
        .or_else(|| {
            execute(git(&root).args(["rev-parse", "--verify", "HEAD^{commit}"]))
                .ok()
                .map(|_| Branch {
                    reference: "HEAD".into(),
                    name: "HEAD".into(),
                    remote: false,
                    current: true,
                })
        })
        .or_else(|| branches.first().cloned());
    branches.sort_by_key(|branch| (!branch.current, branch.remote));
    Ok(Branches { branches, default })
}

pub fn workspace_files(root: &Path) -> Result<Vec<PathBuf>> {
    let output = execute(git(root).args([
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
    ]))?;
    parse_file_paths(&output)
}

pub fn files_at(root: &Path, reference: &str) -> Result<Vec<PathBuf>> {
    let output = execute(git(root).args(["ls-tree", "-rz", reference, "--"]))?;
    output
        .split(|byte| *byte == 0)
        .filter(|entry| entry.starts_with(b"100"))
        .filter_map(|entry| {
            let separator = entry.iter().position(|byte| *byte == b'\t')?;
            Some(file_path(&entry[separator + 1..]))
        })
        .collect()
}

fn parse_file_paths(output: &[u8]) -> Result<Vec<PathBuf>> {
    output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(file_path)
        .collect()
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

pub fn switch_branch(root: &Path, branch: &str) -> Result<()> {
    execute(git(root).args([
        "-c",
        "core.hooksPath=/dev/null",
        "switch",
        "--no-guess",
        "--no-overwrite-ignore",
        "--no-recurse-submodules",
        "--",
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

pub fn has_conflicts(root: &Path) -> bool {
    execute(git(root).args(["ls-files", "--unmerged", "-z"])).is_ok_and(|output| !output.is_empty())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubBranch {
    pub owner: String,
    pub repo: String,
    pub head: String,
    local_ref: String,
    pub local_oid: String,
    synced: bool,
}

pub fn github_branch(root: &Path) -> Result<GitHubBranch> {
    ensure!(
        root.join(".git").exists(),
        "Create PR requires a Git checkout."
    );
    let branch = execute(git(root).args(["symbolic-ref", "--quiet", "HEAD"]))
        .map_err(|_| anyhow::anyhow!("Switch to a branch before creating a pull request."))?;
    let branch = std::str::from_utf8(&branch)?.trim_end();
    let tracking = execute(git(root).args([
        "for-each-ref",
        "--format=%(refname)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream:track,nobracket)%00%(objectname)",
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
    let synced = fields.next() == Some("");
    let local_oid = fields.next().context(message)?.to_owned();
    let url = execute(git(root).args(["remote", "get-url", "--", remote]))?;
    let (owner, repo) = github_repository(std::str::from_utf8(&url)?.trim_end())
        .context("Create PR requires a GitHub.com upstream.")?;
    Ok(GitHubBranch {
        owner,
        repo,
        head: head.to_owned(),
        local_ref: branch.to_owned(),
        local_oid,
        synced,
    })
}

impl GitHubBranch {
    pub fn compare_url(&self) -> Result<String> {
        ensure!(
            self.synced,
            "Sync the branch with its upstream before creating a pull request."
        );
        Ok(github_compare(&self.owner, &self.repo, &self.head))
    }
}

fn github_repository(remote: &str) -> Option<(String, String)> {
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
    Some((owner.to_owned(), repo.to_owned()))
}

fn github_compare(owner: &str, repo: &str, branch: &str) -> String {
    let mut url = url::Url::parse("https://github.com").unwrap();
    url.path_segments_mut()
        .unwrap()
        .extend([owner, repo, "compare", branch]);
    url.set_query(Some("expand=1"));
    url.into()
}

#[cfg(test)]
mod tests {
    use super::{github_compare, github_repository};

    #[test]
    fn opens_github_comparisons_for_the_selected_repository_and_branch() {
        for remote in [
            "git@github.com:org/project.git",
            "ssh://git@github.com/org/project.git",
            "https://github.com/org/project",
            "https://sample@example.invalid@github.com/org/project.git",
        ] {
            let (owner, repo) = github_repository(remote).unwrap();
            assert_eq!(
                github_compare(&owner, &repo, "fix/parser#12"),
                "https://github.com/org/project/compare/fix%2Fparser%2312?expand=1"
            );
        }
        for remote in [
            "/tmp/project.git",
            "git@gitlab.com:org/project.git",
            "https://github.com/org",
        ] {
            assert!(github_repository(remote).is_none());
        }
    }
}
