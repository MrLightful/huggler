//! Checkout operations for the desktop's local Git review.
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs,
    io::{self, Read, Seek, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineCounts {
    pub added: usize,
    pub removed: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChange {
    Added,
    Modified,
    Deleted,
    Conflict,
    Submodule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
    pub change: FileChange,
    pub lines: LineCounts,
    working_tree: Option<WorkingTreeStatus>,
    diff_source: DiffSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiffSource {
    Index,
    Untracked,
    IntentToAdd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorkingTreeStatus {
    status: [u8; 2],
    submodule: bool,
}

impl File {
    pub fn can_revert(&self) -> bool {
        self.working_tree.is_some_and(|status| status.can_revert())
    }

    pub fn revert_removes_file(&self) -> bool {
        self.working_tree
            .is_some_and(|status| status.change() == FileChange::Added)
    }
}

impl WorkingTreeStatus {
    fn change(self) -> FileChange {
        match &self.status {
            b"DD" | b"AU" | b"UD" | b"UA" | b"DU" | b"AA" | b"UU" => FileChange::Conflict,
            _ if self.submodule => FileChange::Submodule,
            [_, b'D'] | [b'D', _] => FileChange::Deleted,
            b"??" | [b'A', _] | [_, b'A'] => FileChange::Added,
            _ => FileChange::Modified,
        }
    }

    fn can_revert(self) -> bool {
        !matches!(self.change(), FileChange::Conflict | FileChange::Submodule)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChangeScope {
    #[default]
    All,
    Uncommitted,
}

pub struct ChangesSnapshot {
    pub base: String,
    // Revert always uses the commit recorded by status, or the empty tree for an unborn branch.
    pub head: String,
    pub files: Vec<File>,
}

struct WorkingTree {
    head: String,
    has_branch: bool,
    files: Vec<File>,
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

pub fn workspace_files(root: &Path) -> Result<Vec<PathBuf>> {
    let output = execute(git(root).args([
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
    ]))?;
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

fn file_path(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(OsStr::from_bytes(bytes).into())
    }
    #[cfg(not(unix))]
    Ok(std::str::from_utf8(bytes)?.into())
}

fn read_changes(root: &Path, path: Option<&Path>) -> Result<WorkingTree> {
    let mut command = git(root);
    command.args([
        "--no-optional-locks",
        "status",
        "--porcelain=v2",
        "--branch",
        "-z",
        "--no-renames",
        "--ignore-submodules=none",
        "--untracked-files=all",
        "--",
    ]);
    if let Some(path) = path {
        command.arg(path);
    }
    let output = execute(&mut command)?;
    let mut head = None;
    let mut has_branch = false;
    let mut files = Vec::new();
    let mut tracked_paths = HashSet::new();
    for record in output.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        if let Some(oid) = record.strip_prefix(b"# branch.oid ") {
            head = Some(std::str::from_utf8(oid)?.to_owned());
            continue;
        }
        if let Some(branch) = record.strip_prefix(b"# branch.head ") {
            has_branch = branch != b"(detached)";
            continue;
        }
        let (path, status, submodule) = match record.first() {
            Some(b'#') => continue,
            Some(b'?') => {
                let path = record.get(2..).context("invalid untracked path")?;
                (path, *b"??", path.ends_with(b"/"))
            }
            Some(b'1' | b'u') => {
                let fields = if record[0] == b'1' { 9 } else { 11 };
                let mut parts = record.splitn(fields, |b| *b == b' ');
                let status = parts.nth(1).context("missing Git status")?.try_into()?;
                let submodule = parts.next().context("missing Git submodule status")?;
                let mut is_git_link = submodule.starts_with(b"S");
                for mode in parts.by_ref().take(if record[0] == b'1' { 3 } else { 4 }) {
                    is_git_link |= mode == b"160000";
                }
                let path = parts
                    .nth(if record[0] == b'1' { 2 } else { 3 })
                    .context("missing Git status path")?;
                (path, status, is_git_link)
            }
            _ => bail!("invalid Git status record"),
        };
        if status != *b"??" {
            tracked_paths.insert(path);
        }
        let working_tree = WorkingTreeStatus { status, submodule };
        files.push(File {
            path: file_path(path)?,
            change: working_tree.change(),
            lines: LineCounts::default(),
            working_tree: Some(working_tree),
            diff_source: if status == *b"??" {
                DiffSource::Untracked
            } else {
                DiffSource::Index
            },
        });
    }
    // A staged deletion can also appear as an untracked path if it was recreated.
    files.retain(|file| {
        file.diff_source != DiffSource::Untracked
            || !tracked_paths.contains(file.path.as_os_str().as_encoded_bytes())
    });
    let mut head = head.context("missing Git checkout revision")?;
    if head == "(initial)" {
        has_branch = false;
        head = String::from_utf8(execute(git(root).args([
            "hash-object",
            "-t",
            "tree",
            "--stdin",
        ]))?)?
        .trim_end()
        .to_owned();
    }
    Ok(WorkingTree {
        head,
        has_branch,
        files,
    })
}

fn comparison_base(root: &Path, head: &str) -> Option<String> {
    [
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
        "refs/remotes/origin/master",
        "refs/heads/main",
        "refs/heads/master",
    ]
    .iter()
    .find_map(|reference| {
        let output = execute(git(root).args(["merge-base", reference, head])).ok()?;
        Some(String::from_utf8(output).ok()?.trim_end().to_owned())
    })
}

pub fn changed_files(root: &Path, scope: ChangeScope) -> Result<ChangesSnapshot> {
    let WorkingTree {
        head,
        has_branch,
        mut files,
    } = read_changes(root, None)?;
    let base = if scope == ChangeScope::All && has_branch {
        comparison_base(root, &head).unwrap_or_else(|| head.clone())
    } else {
        head.clone()
    };
    if base != head {
        files = branch_changes(root, &base, files)?;
    } else if !files.is_empty() {
        let mut stats = diff_stats(root, &base, None)?;
        for file in &mut files {
            if file.diff_source == DiffSource::Index && file.can_revert() {
                file.lines = stats
                    .remove(&file.path)
                    .map(|(_, lines)| lines)
                    .unwrap_or_default();
            }
        }
    }
    for file in &mut files {
        if file.diff_source == DiffSource::Untracked {
            file.lines.added = untracked_line_count(root, &file.path).unwrap_or_default();
        }
    }
    files.sort_unstable_by(|a, b| a.path.as_os_str().cmp(b.path.as_os_str()));
    Ok(ChangesSnapshot { base, head, files })
}

fn branch_changes(root: &Path, base: &str, files: Vec<File>) -> Result<Vec<File>> {
    let mut stats = diff_stats(root, base, None)?;
    // Git omits untracked worktree contents from diff-index. When a path
    // existed in the comparison base, temporarily make it visible to Git.
    let restored_paths: HashSet<_> = files
        .iter()
        .filter(|file| {
            file.diff_source == DiffSource::Untracked
                && file.change != FileChange::Submodule
                && stats.contains_key(&file.path)
        })
        .map(|file| file.path.clone())
        .collect();
    if !restored_paths.is_empty() {
        let index = intent_to_add_index(root, restored_paths.iter().map(PathBuf::as_path))?;
        stats = diff_stats(root, base, Some(&index.path().join("index")))?;
    }

    let mut rows = Vec::new();
    for mut file in files {
        if restored_paths.contains(&file.path) {
            file.diff_source = DiffSource::IntentToAdd;
        }
        if file.diff_source == DiffSource::Untracked {
            stats.remove(&file.path);
            rows.push(file);
        } else if let Some((change, lines)) = stats.remove(&file.path) {
            if !matches!(file.change, FileChange::Conflict | FileChange::Submodule) {
                file.change = change;
                file.lines = lines;
            }
            rows.push(file);
        }
    }
    rows.extend(stats.into_iter().map(|(path, (change, lines))| File {
        path,
        change,
        lines,
        working_tree: None,
        diff_source: DiffSource::Index,
    }));
    Ok(rows)
}

fn intent_to_add_index<'a>(
    root: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
) -> Result<tempfile::TempDir> {
    let scratch = tempfile::tempdir()?;
    let index = scratch.path().join("index");
    let path = execute(git(root).args(["rev-parse", "--git-path", "index"]))?;
    let original = root.join(file_path(path.strip_suffix(b"\n").unwrap_or(&path))?);
    match fs::copy(original, &index) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut input = tempfile::tempfile()?;
    for path in paths {
        input.write_all(path.as_os_str().as_encoded_bytes())?;
        input.write_all(b"\0")?;
    }
    input.rewind()?;
    execute(
        git(root)
            .env("GIT_INDEX_FILE", &index)
            .args([
                "-c",
                "core.splitIndex=false",
                "-c",
                "splitIndex.sharedIndexExpire=never",
                "add",
                "--sparse",
                "--intent-to-add",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ])
            .stdin(input),
    )?;
    Ok(scratch)
}

fn diff_stats(
    root: &Path,
    base: &str,
    index: Option<&Path>,
) -> Result<HashMap<PathBuf, (FileChange, LineCounts)>> {
    let mut command = git(root);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let output = execute(command.args([
        "--no-optional-locks",
        "diff-index",
        "--no-renames",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--raw",
        "--numstat",
        "-z",
        base,
        "--",
    ]))?;
    let mut changes = HashMap::new();
    let mut records = output
        .split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .peekable();
    while records
        .peek()
        .is_some_and(|record| record.starts_with(b":"))
    {
        let mut fields = records.next().unwrap().split(|b| *b == b' ');
        let old_mode = fields.next();
        let new_mode = fields.next();
        let status = fields.nth(2).context("missing Git diff status")?;
        let change = match status {
            b"U" => FileChange::Conflict,
            _ if old_mode == Some(b":160000") || new_mode == Some(b"160000") => {
                FileChange::Submodule
            }
            b"A" => FileChange::Added,
            b"D" => FileChange::Deleted,
            _ => FileChange::Modified,
        };
        let path = file_path(records.next().context("missing Git diff path")?)?;
        changes.insert(path, change);
    }
    let mut stats = HashMap::new();
    for record in records {
        let mut parts = record.splitn(3, |b| *b == b'\t');
        let added = parts.next().context("missing Git insertion count")?;
        let removed = parts.next().context("missing Git deletion count")?;
        let path = file_path(parts.next().context("missing Git numstat path")?)?;
        let change = changes
            .remove(&path)
            .context("missing Git change for numstat path")?;
        let lines = if added == b"-" || removed == b"-" {
            LineCounts::default()
        } else {
            LineCounts {
                added: std::str::from_utf8(added)?.parse()?,
                removed: std::str::from_utf8(removed)?.parse()?,
            }
        };
        stats.insert(path, (change, lines));
    }
    // Raw output alone also reports files whose stat data changed without a patch.
    stats.extend(changes.into_iter().filter_map(|(path, change)| {
        matches!(change, FileChange::Conflict | FileChange::Submodule)
            .then_some((path, (change, LineCounts::default())))
    }));
    Ok(stats)
}

fn untracked_line_count(root: &Path, path: &Path) -> Result<usize> {
    // Skip large and binary additions so building the list stays inexpensive.
    const MAX_COUNT_BYTES: u64 = 1024 * 1024;
    let path = root.join(path);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.len() > MAX_COUNT_BYTES {
        return Ok(0);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_COUNT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_COUNT_BYTES || bytes.iter().take(8000).any(|b| *b == 0) {
        return Ok(0);
    }
    Ok(bytes.iter().filter(|b| **b == b'\n').count()
        + usize::from(bytes.last().is_some_and(|b| *b != b'\n')))
}

pub fn file_diff(root: &Path, base: &str, file: &File) -> Result<String> {
    const MAX_DIFF_BYTES: u64 = 32 * 1024;
    check_file_path(root, &file.path)?;
    let mut command = git(root);
    command.arg("--no-optional-locks");
    if file.diff_source != DiffSource::Index {
        const MAX_PREVIEW_INPUT_BYTES: u64 = 1024 * 1024;
        if fs::symlink_metadata(root.join(&file.path))?.len() > MAX_PREVIEW_INPUT_BYTES {
            return Ok("File is too large to preview.\n".into());
        }
    }
    let untracked = file.diff_source == DiffSource::Untracked;
    let index = if file.diff_source == DiffSource::IntentToAdd {
        Some(intent_to_add_index(root, [&*file.path])?)
    } else {
        None
    };
    if let Some(index) = &index {
        command.env("GIT_INDEX_FILE", index.path().join("index"));
    }
    if untracked {
        command.args([
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--",
            "/dev/null",
        ]);
    } else {
        command.args([
            "diff-index",
            "--patch",
            "--no-renames",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=dirty",
            base,
            "--",
        ]);
    }
    let mut child = command
        .arg(&file.path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut bytes = Vec::new();
    let result = child
        .stdout
        .take()
        .context("missing Git diff output")?
        .take(MAX_DIFF_BYTES + 1)
        .read_to_end(&mut bytes);
    let truncated = bytes.len() as u64 > MAX_DIFF_BYTES;
    if result.is_err() || truncated {
        let _ = child.kill();
    }
    let status = child.wait()?;
    result?;
    ensure!(
        truncated || status.success() || (untracked && status.code() == Some(1)),
        "Git could not read the file diff."
    );
    if truncated {
        bytes.truncate(MAX_DIFF_BYTES as usize);
    }
    let mut diff = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        diff.push_str("\nPreview truncated.\n");
    }
    Ok(diff)
}

fn check_file_path(root: &Path, path: &Path) -> Result<()> {
    // A changed file can be replaced by a directory or a symlinked parent.
    // Git pathspecs also match descendants, so reject directory replacements.
    ensure!(!path.as_os_str().is_empty(), "Missing file path.");
    let mut full = root.to_path_buf();
    let mut parts = path.components().peekable();
    while let Some(part) = parts.next() {
        let Component::Normal(part) = part else {
            bail!("Expected a checkout-relative file path.");
        };
        full.push(part);
        match fs::symlink_metadata(&full) {
            Ok(metadata) if parts.peek().is_some() => {
                ensure!(
                    metadata.is_dir(),
                    "A parent of this path is no longer a directory."
                );
            }
            Ok(metadata) => ensure!(
                metadata.is_file() || metadata.is_symlink(),
                "This file path now refers to a directory or special file."
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub fn revert_file(root: &Path, head: &str, file: &File) -> Result<()> {
    ensure!(
        file.can_revert(),
        "Resolve this file in Git before reverting it."
    );
    check_file_path(root, &file.path)?;
    let current = read_changes(root, Some(&file.path))?;
    ensure!(
        current.head == head
            && matches!(current.files.as_slice(), [latest]
                if latest.path == file.path && latest.working_tree == file.working_tree && latest.can_revert()),
        "The checkout changed. Review the file and try again."
    );
    let indexed = execute(git(root).args(["ls-files", "-z", "--"]).arg(&file.path))?;
    for path in indexed.split(|b| *b == 0).filter(|path| !path.is_empty()) {
        ensure!(
            file_path(path)? == file.path,
            "The index contains files under this path."
        );
    }
    check_file_path(root, &file.path)?;
    if file
        .working_tree
        .is_some_and(|status| status.status == *b"??")
    {
        ensure!(
            indexed.is_empty(),
            "The file was staged. Review it and try again."
        );
        fs::remove_file(root.join(&file.path)).context("Remove untracked file")?;
    } else {
        execute(
            git(root)
                .args(["restore", "--source", head, "--staged", "--worktree", "--"])
                .arg(&file.path),
        )?;
    }
    Ok(())
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
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        execute(checkout_command(dir.path()).args(["init", "--quiet"])).unwrap();
        dir
    }

    fn commit_fixture(root: &Path) {
        execute(git(root).args(["add", "--", "."])).unwrap();
        execute(git(root).args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "Fixture",
        ]))
        .unwrap();
    }

    #[test]
    fn reviews_and_reverts_exact_paths_in_an_unborn_checkout() {
        let dir = fixture();
        let root = dir.path();
        let staged = Path::new("odd [1]\tname.txt");
        fs::write(root.join(staged), "first\nsecond").unwrap();
        fs::write(root.join("font.bin"), b"\0\xff").unwrap();
        execute(git(root).args(["add", "--"]).arg(staged)).unwrap();

        let snapshot = changed_files(root, ChangeScope::All).unwrap();
        assert_eq!(snapshot.files.len(), 2);
        let binary = &snapshot.files[0];
        let text = &snapshot.files[1];
        assert_eq!(binary.change, FileChange::Added);
        assert_eq!(binary.lines, LineCounts::default());
        assert_eq!(text.path, staged);
        assert_eq!(
            text.lines,
            LineCounts {
                added: 2,
                removed: 0
            }
        );
        assert!(
            file_diff(root, &snapshot.base, text)
                .unwrap()
                .contains("+second")
        );

        revert_file(root, &snapshot.head, text).unwrap();
        assert!(!root.join(staged).exists());
        assert_eq!(
            changed_files(root, ChangeScope::Uncommitted).unwrap().files,
            std::slice::from_ref(binary)
        );
    }

    #[test]
    fn revert_rechecks_status_and_protects_directory_replacements() {
        let dir = fixture();
        let root = dir.path();
        fs::write(root.join("file"), "committed\n").unwrap();
        commit_fixture(root);
        fs::write(root.join("file"), "modified\n").unwrap();
        let snapshot = changed_files(root, ChangeScope::Uncommitted).unwrap();
        let file = &snapshot.files[0];
        assert_eq!(
            file.lines,
            LineCounts {
                added: 1,
                removed: 1
            }
        );

        execute(git(root).args(["add", "--", "file"])).unwrap();
        assert!(revert_file(root, &snapshot.head, file).is_err());
        assert_eq!(fs::read_to_string(root.join("file")).unwrap(), "modified\n");

        fs::remove_file(root.join("file")).unwrap();
        fs::create_dir(root.join("file")).unwrap();
        fs::write(root.join("file/keep"), "unrelated\n").unwrap();
        let snapshot = changed_files(root, ChangeScope::Uncommitted).unwrap();
        let file = snapshot
            .files
            .iter()
            .find(|file| file.path == Path::new("file"))
            .unwrap();
        assert!(revert_file(root, &snapshot.head, file).is_err());
        assert_eq!(
            fs::read_to_string(root.join("file/keep")).unwrap(),
            "unrelated\n"
        );
    }

    #[test]
    fn compares_a_recreated_file_to_the_branch_base_and_reverts_to_head() {
        let dir = fixture();
        let root = dir.path();
        let restored = Path::new("file [1]");
        let committed = Path::new("committed");
        fs::write(root.join(restored), "base\n").unwrap();
        commit_fixture(root);
        execute(git(root).args(["branch", "-M", "main"])).unwrap();
        create_branch(root, "topic").unwrap();
        fs::remove_file(root.join(restored)).unwrap();
        fs::write(root.join(committed), "branch\n").unwrap();
        commit_fixture(root);
        let index_before = fs::read(root.join(".git/index")).unwrap();

        fs::write(root.join(restored), "base\n").unwrap();
        let all = changed_files(root, ChangeScope::All).unwrap();
        assert_eq!(all.files.len(), 1);
        assert_eq!(all.files[0].path, committed);
        assert!(!all.files[0].can_revert());

        fs::write(root.join(restored), "working tree\n").unwrap();
        let all = changed_files(root, ChangeScope::All).unwrap();
        let file = &all.files[1];
        assert_eq!(file.path, restored);
        assert_eq!(file.change, FileChange::Modified);
        assert_eq!(
            file.lines,
            LineCounts {
                added: 1,
                removed: 1
            }
        );
        let patch = file_diff(root, &all.base, file).unwrap();
        assert!(patch.contains("-base\n+working tree\n"));
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index_before);

        let local = changed_files(root, ChangeScope::Uncommitted).unwrap();
        assert_eq!(local.files.len(), 1);
        assert_eq!(local.base, all.head);
        assert_eq!(local.files[0].path, restored);
        assert_eq!(local.files[0].change, FileChange::Added);
        assert!(file.revert_removes_file());
        revert_file(root, &all.head, file).unwrap();
        assert!(!root.join(restored).exists());
        assert!(root.join(committed).exists());
    }

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
