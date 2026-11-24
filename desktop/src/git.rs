//! Checkout operations for the desktop's local Git review.
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs,
    io::{self, Read, Seek, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Output, Stdio},
    time::Duration,
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
    recreated: bool,
}

impl File {
    pub fn can_revert(&self) -> bool {
        self.working_tree.is_some_and(|status| status.can_revert())
    }

    pub fn revert_removes_file(&self) -> bool {
        self.working_tree
            .is_some_and(|status| status.change() == FileChange::Added)
    }

    pub fn can_approve(&self) -> bool {
        !matches!(self.change, FileChange::Conflict | FileChange::Submodule)
    }
}

impl WorkingTreeStatus {
    fn has_staged_change(self) -> bool {
        !matches!(self.status[0], b'.' | b'?')
    }

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
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

pub fn main_worktree(root: &Path) -> Result<PathBuf> {
    let output = execute(git(root).args(["worktree", "list", "--porcelain", "-z"]))?;
    let entry = output.split(|byte| *byte == 0).next().unwrap_or_default();
    let path = entry
        .strip_prefix(b"worktree ")
        .context("Git returned no worktree")?;
    Ok(file_path(path)?.canonicalize()?)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    pub root: PathBuf,
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
    Ok(Branches {
        root,
        branches,
        default,
    })
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
    git_output(cmd.output().context("start Git")?)
}

fn git_output(output: Output) -> Result<Vec<u8>> {
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
    let mut untracked_paths = HashSet::new();
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
        if status == *b"??" {
            untracked_paths.insert(path);
        } else {
            tracked_paths.insert(path);
        }
        let working_tree = WorkingTreeStatus {
            status,
            submodule,
            recreated: false,
        };
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
    files.retain_mut(|file| {
        let bytes = file.path.as_os_str().as_encoded_bytes();
        if file.diff_source == DiffSource::Untracked {
            return !tracked_paths.contains(bytes);
        }
        if let Some(status) = &mut file.working_tree {
            status.recreated = untracked_paths.contains(bytes);
        }
        true
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
        } else if file
            .working_tree
            .is_some_and(WorkingTreeStatus::has_staged_change)
        {
            // The working copy may cancel out a change that is still staged.
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
    let scratch = copy_index(root)?;
    let index = scratch.path().join("index");
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

fn copy_index(root: &Path) -> Result<tempfile::TempDir> {
    let scratch = tempfile::tempdir()?;
    copy_index_to(&index_path(root)?, &scratch.path().join("index"))?;
    Ok(scratch)
}

fn index_path(root: &Path) -> Result<PathBuf> {
    let path =
        execute(git(root).args(["rev-parse", "--path-format=absolute", "--git-path", "index"]))?;
    file_path(path.strip_suffix(b"\n").unwrap_or(&path))
}

fn copy_index_to(source: &Path, destination: &Path) -> Result<()> {
    match fs::copy(source, destination) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
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

pub fn diff_fingerprint(root: &Path, base: &str, head: &str, file: &File) -> Result<String> {
    check_file_path(root, &file.path)?;
    ensure!(file.can_approve(), "This file cannot be approved.");
    // A full index line identifies binary changes. Hash the complete output
    // without loading the patch into memory or writing an object to Git.
    let (command, _index) = diff_command(root, base, file)?;
    let mut fingerprint = hash_patch(root, command, file.diff_source == DiffSource::Untracked)?;
    let empty = execute(git(root).args(["hash-object", "--stdin"]))?;
    let net_empty = empty.strip_suffix(b"\n") == Some(fingerprint.as_bytes());
    if let Some(extra) = supplemental_diff(file, net_empty) {
        // The combined patch can conceal staged changes when unstaged edits cancel them out.
        let staged = hash_patch(root, staged_diff(root, head, &file.path), false)?;
        ensure!(
            empty.strip_suffix(b"\n") != Some(staged.as_bytes()),
            "The staged diff changed."
        );
        fingerprint.push(':');
        fingerprint.push_str(&staged);
        if extra == SupplementalDiff::Recreated {
            // A staged deletion hides the recreated, untracked file from diff-index.
            fingerprint.push(':');
            fingerprint.push_str(&hash_patch(root, untracked_diff(root, &file.path), true)?);
        }
        return Ok(fingerprint);
    }
    ensure!(!net_empty, "The file no longer has a diff.");
    Ok(fingerprint)
}

pub(crate) fn file_patch(
    root: &Path,
    base: &str,
    head: &str,
    file: &File,
    limit: usize,
) -> Result<Vec<u8>> {
    check_file_path(root, &file.path)?;
    let (command, _index) = diff_command(root, base, file)?;
    let mut patch = Vec::new();
    read_patch(
        command,
        file.diff_source == DiffSource::Untracked,
        limit,
        &mut patch,
    )?;
    if let Some(extra) = supplemental_diff(file, patch.is_empty()) {
        append_patch_section(
            staged_diff(root, head, &file.path),
            false,
            b"\nStaged changes\n\n",
            limit,
            &mut patch,
        )?;
        if extra == SupplementalDiff::Recreated {
            append_patch_section(
                untracked_diff(root, &file.path),
                true,
                b"\nRecreated file\n\n",
                limit,
                &mut patch,
            )?;
        }
    }
    Ok(patch)
}

#[derive(PartialEq, Eq)]
enum SupplementalDiff {
    Staged,
    Recreated,
}

fn supplemental_diff(file: &File, combined_empty: bool) -> Option<SupplementalDiff> {
    let status = file.working_tree?;
    if !status.has_staged_change() {
        return None;
    }
    if status.recreated {
        Some(SupplementalDiff::Recreated)
    } else if combined_empty || status.status[1] != b'.' {
        Some(SupplementalDiff::Staged)
    } else {
        None
    }
}

fn append_patch_section(
    command: Command,
    untracked: bool,
    header: &[u8],
    limit: usize,
    patch: &mut Vec<u8>,
) -> Result<()> {
    if patch.len() == limit {
        return Ok(());
    }
    patch.extend_from_slice(&header[..header.len().min(limit - patch.len())]);
    if patch.len() < limit {
        read_patch(command, untracked, limit, patch)?;
    }
    Ok(())
}

fn read_patch(
    mut command: Command,
    untracked: bool,
    limit: usize,
    patch: &mut Vec<u8>,
) -> Result<()> {
    let mut diff = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stream = diff.stdout.take().expect("piped Git output");
    let result = stream
        .take(limit.saturating_sub(patch.len()) as u64)
        .read_to_end(patch);
    let clipped = patch.len() >= limit;
    if result.is_err() || clipped {
        let _ = diff.kill();
    }
    let status = diff.wait()?;
    result?;
    ensure!(
        clipped || status.success() || (untracked && status.code() == Some(1)),
        "Git could not read the file diff."
    );
    Ok(())
}

fn staged_diff(root: &Path, head: &str, path: &Path) -> Command {
    let mut command = git(root);
    command
        .args([
            "--no-optional-locks",
            "diff-index",
            "--cached",
            "--patch",
            "--full-index",
            "--no-renames",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            head,
            "--",
        ])
        .arg(path);
    command
}

fn hash_patch(root: &Path, mut command: Command, untracked: bool) -> Result<String> {
    let mut diff = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stream = diff.stdout.take().context("missing Git diff output")?;
    let hash = execute(git(root).args(["hash-object", "--stdin"]).stdin(stream));
    let status = diff.wait()?;
    ensure!(
        status.success() || (untracked && status.code() == Some(1)),
        "Git could not read the file diff."
    );
    Ok(String::from_utf8(hash?)?.trim_end().to_owned())
}

fn untracked_diff(root: &Path, path: &Path) -> Command {
    let mut command = git(root);
    command
        .args([
            "--no-optional-locks",
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--full-index",
            "--",
            "/dev/null",
        ])
        .arg(path);
    command
}

fn diff_command(
    root: &Path,
    base: &str,
    file: &File,
) -> Result<(Command, Option<tempfile::TempDir>)> {
    if file.diff_source == DiffSource::Untracked {
        return Ok((untracked_diff(root, &file.path), None));
    }
    let mut command = git(root);
    command.arg("--no-optional-locks");
    let index = if file.diff_source == DiffSource::IntentToAdd {
        Some(intent_to_add_index(root, [&*file.path])?)
    } else {
        None
    };
    if let Some(index) = &index {
        command.env("GIT_INDEX_FILE", index.path().join("index"));
    }
    command
        .args([
            "diff-index",
            "--patch",
            "--no-renames",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=dirty",
            "--full-index",
            base,
            "--",
        ])
        .arg(&file.path);
    Ok((command, index))
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

pub struct PreparedRevert {
    file: File,
    head: String,
    fingerprint: String,
}

impl PreparedRevert {
    pub fn prepare(root: &Path, head: String, file: File) -> Result<Self> {
        ensure!(
            file.can_revert(),
            "Resolve this file in Git before reverting it."
        );
        let fingerprint = diff_fingerprint(root, &head, &head, &file)?;
        Ok(Self {
            file,
            head,
            fingerprint,
        })
    }

    pub fn path(&self) -> &Path {
        &self.file.path
    }
}

pub fn revert_file(root: &Path, target: &PreparedRevert) -> Result<()> {
    let PreparedRevert {
        file,
        head,
        fingerprint,
    } = target;
    check_file_path(root, &file.path)?;
    let current = read_changes(root, Some(&file.path))?;
    ensure!(
        current.head == *head
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
    ensure!(
        diff_fingerprint(root, head, head, file)? == *fingerprint,
        "The file changed. Review it and try again."
    );
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

pub fn has_conflicts(root: &Path) -> bool {
    execute(git(root).args(["ls-files", "--unmerged", "-z"])).is_ok_and(|output| !output.is_empty())
}

pub fn has_uncommitted_changes(root: &Path) -> Result<bool> {
    let status = execute(git(root).args([
        "--no-optional-locks",
        "status",
        "--porcelain",
        "-z",
        "--untracked-files=normal",
        "--ignore-submodules=dirty",
    ]))?;
    Ok(!status.is_empty())
}

pub struct PendingCommit {
    head: String,
    branch_ref: String,
    tree: Vec<u8>,
    index: tempfile::TempDir,
}

impl PendingCommit {
    pub fn prepare(root: &Path, branch: &GitHubBranch) -> Result<Option<Self>> {
        Self::snapshot(
            root,
            &branch.local_oid,
            &format!("refs/heads/{}", branch.local_branch),
        )
    }

    fn snapshot(root: &Path, head: &str, branch_ref: &str) -> Result<Option<Self>> {
        ensure!(
            !has_conflicts(root),
            "Resolve the Git conflicts before creating a pull request."
        );
        let dir = execute(git(root).args(["rev-parse", "--absolute-git-dir"]))?;
        let dir = file_path(dir.strip_suffix(b"\n").unwrap_or(&dir))?;
        ensure!(
            [
                "MERGE_HEAD",
                "CHERRY_PICK_HEAD",
                "REVERT_HEAD",
                "rebase-merge",
                "rebase-apply",
                "sequencer"
            ]
            .iter()
            .all(|path| !dir.join(path).exists()),
            "Finish the Git operation in progress before creating a pull request."
        );
        let index = copy_index(root)?;
        let mut pending = Self {
            head: head.to_owned(),
            branch_ref: branch_ref.to_owned(),
            tree: Vec::new(),
            index,
        };
        execute(pending.git(root).args(["add", "-A", "--", "."]))
            .context("Prepare changes for the pull request commit")?;
        pending.tree = execute(pending.git(root).arg("write-tree"))?;
        let base = execute(git(root).args(["rev-parse", &format!("{head}^{{tree}}")]))?;
        Ok((pending.tree != base).then_some(pending))
    }

    fn git(&self, root: &Path) -> Command {
        let mut command = git(root);
        command
            .env("GIT_INDEX_FILE", self.index.path().join("index"))
            .args([
                "-c",
                "core.splitIndex=false",
                "-c",
                "splitIndex.sharedIndexExpire=never",
            ]);
        command
    }

    pub fn message_prompt(&self, root: &Path) -> Result<String> {
        let mut summary = self.git(root);
        summary.args([
            "diff",
            "--cached",
            "--name-status",
            "--no-relative",
            "--no-color",
            &self.head,
            "--",
        ]);
        let mut patch = self.git(root);
        patch.args([
            "diff",
            "--cached",
            "--patch",
            "--no-relative",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            &self.head,
            "--",
        ]);
        let mut subjects = git(root);
        subjects.args(["log", "-8", "--format=%s", &self.head, "--"]);
        let bounded = |command, limit| {
            let mut bytes = Vec::new();
            read_patch(command, false, limit, &mut bytes)?;
            let mut result = String::from_utf8_lossy(&bytes).into_owned();
            if bytes.len() == limit {
                result.push_str("\n[truncated]");
            }
            Ok::<_, anyhow::Error>(result)
        };
        Ok(format!(
            "Write one Git commit message for these changes. Use a concise, specific subject in imperative mood and follow the style of the recent subjects. Use an empty body unless it adds useful context.\n\nRecent commit subjects:\n{}\n\nChanged paths:\n{}\n\nPatch:\n{}",
            bounded(subjects, 2000)?,
            bounded(summary, 8000)?,
            bounded(patch, 50_000)?,
        ))
    }

    pub fn commit(&self, root: &Path, subject: &str, body: &str) -> Result<String> {
        let index_path = index_path(root)?;
        let mut lock_path = index_path.clone().into_os_string();
        lock_path.push(".lock");
        let lock_path = PathBuf::from(lock_path);
        // A normal Git writer honours this lock while the commit and its hooks use the
        // verified temporary index. Failed commits leave the original index in place.
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
            .context("Lock the Git index for the pull request commit")?;
        let index_lock = tempfile::TempPath::try_from_path(lock_path)?;
        self.verify_head(root)?;
        let prepared = Self::snapshot(root, &self.head, &self.branch_ref)?
            .filter(|current| current.tree == self.tree)
            .context("The working changes changed while writing the commit message. Review them and try again.")?;
        let mut command = prepared.git(root);
        command.args(["commit", "--cleanup=whitespace", "-m", subject]);
        if !body.is_empty() {
            command.args(["-m", body]);
        }
        self.verify_head(root)?;
        execute(&mut command).context("Commit changes for the pull request")?;
        fs::copy(prepared.index.path().join("index"), &index_lock)
            .context("The commit was created. Could not copy its Git index")?;
        index_lock
            .persist(&index_path)
            .context("The commit was created. Could not update the Git index")?;
        let revision =
            execute(git(root).args(["show", "-s", "--format=%H%n%T%n%P", "HEAD", "--"]))?;
        let revision = std::str::from_utf8(&revision)?;
        let mut lines = revision.lines();
        let oid = lines
            .next()
            .context("Expected the new pull request commit.")?;
        ensure!(
            lines
                .next()
                .is_some_and(|tree| self.tree.strip_suffix(b"\n") == Some(tree.as_bytes()))
                && lines.next() == Some(self.head.as_str()),
            "The resulting commit differs from the prepared changes. Review it before publishing."
        );
        Ok(oid.to_owned())
    }

    fn verify_head(&self, root: &Path) -> Result<()> {
        ensure!(
            execute(git(root).args(["symbolic-ref", "--quiet", "HEAD"]))?
                == format!("{}\n", self.branch_ref).as_bytes()
                && execute(git(root).args(["rev-parse", "HEAD"]))?
                    == format!("{}\n", self.head).as_bytes(),
            "The branch changed while writing the commit message. Try again."
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubBranch {
    pub owner: String,
    pub repo: String,
    pub head: String,
    local_branch: String,
    pub local_oid: String,
    remote: String,
    set_upstream: bool,
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
    let local_branch = branch
        .strip_prefix("refs/heads/")
        .context("Expected a local branch.")?;
    let tracking = execute(git(root).args([
        "for-each-ref",
        "--format=%(refname)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream:track,nobracket)%00%(objectname)",
        branch,
    ]))?;
    let mut fields = std::str::from_utf8(&tracking)?
        .lines()
        .find_map(|line| {
            let mut fields = line.split('\0');
            (fields.next() == Some(branch)).then_some(fields)
        })
        .context("Commit changes on this branch before creating a pull request.")?;
    let remote_key = format!("branch.{local_branch}.remote");
    let merge_key = format!("branch.{local_branch}.merge");
    let tracking_remote = fields.next().filter(|r| !r.is_empty());
    let upstream_head = fields.next().and_then(|r| r.strip_prefix("refs/heads/"));
    let tracking_status = fields.next();
    let in_sync = tracking_remote.is_some() && tracking_status == Some("");
    // Even if the remote only fetches selected branches, preserve its configured push target.
    let upstream_remote = match tracking_remote {
        Some(remote) => Some(remote.to_owned()),
        None => git_config(root, &remote_key)?,
    }
    .filter(|r| !matches!(r.as_str(), "" | "."));
    let upstream_head = match (upstream_remote.as_ref(), upstream_head) {
        (Some(_), Some(head)) => Some(head.to_owned()),
        (Some(_), None) => git_config(root, &merge_key)?
            .and_then(|reference| reference.strip_prefix("refs/heads/").map(str::to_owned)),
        (None, _) => None,
    };
    let local_oid = fields
        .next()
        .context("Expected a branch commit.")?
        .to_owned();

    let remote = match upstream_remote {
        Some(remote) => remote,
        None => {
            let remotes = execute(git(root).arg("remote"))?;
            let remotes: Vec<_> = std::str::from_utf8(&remotes)?.lines().collect();
            match remotes.as_slice() {
                _ if remotes.contains(&"origin") => "origin".into(),
                [remote] => (*remote).into(),
                [] => bail!("Add a GitHub.com remote before creating a pull request."),
                _ => bail!("Set the branch upstream to choose a GitHub.com remote."),
            }
        }
    };
    let url = execute(git(root).args(["remote", "get-url", "--", &remote]))?;
    let (owner, repo) = github_repository(std::str::from_utf8(&url)?.trim_end())
        .context("Create PR requires a GitHub.com remote.")?;
    Ok(GitHubBranch {
        owner,
        repo,
        head: upstream_head.as_deref().unwrap_or(local_branch).to_owned(),
        local_branch: local_branch.to_owned(),
        local_oid,
        remote,
        set_upstream: upstream_head.is_none(),
        synced: upstream_head.is_some() && in_sync,
    })
}

fn git_config(root: &Path, key: &str) -> Result<Option<String>> {
    let output = git(root)
        .args(["config", "--get", key])
        .output()
        .context("start Git")?;
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    let value = String::from_utf8(git_output(output)?)?;
    Ok(Some(value.trim_end_matches('\n').to_owned()))
}

impl GitHubBranch {
    pub fn cached_default_branch(&self, root: &Path) -> Option<String> {
        let prefix = format!("refs/remotes/{}/", self.remote);
        let output =
            execute(git(root).args(["symbolic-ref", "--quiet", &format!("{prefix}HEAD")])).ok()?;
        std::str::from_utf8(&output)
            .ok()?
            .trim_end()
            .strip_prefix(&prefix)
            .map(str::to_owned)
    }

    pub fn is_pr_head(&self, default: &str, base: &str) -> bool {
        self.local_branch != default && self.head != default && self.head != base
    }

    pub fn pr_base_commit(&self, root: &Path, base: &str) -> Option<String> {
        let base_ref = format!("refs/remotes/{}/{base}", self.remote);
        let commit = execute(git(root).args(["rev-parse", "--verify", &base_ref]))
            .or_else(|_| {
                execute(git(root).args(["rev-parse", "--verify", &format!("refs/heads/{base}")]))
            })
            .ok()?;
        Some(String::from_utf8(commit).ok()?.trim_end().to_owned())
    }

    pub fn has_pr_commits(&self, root: &Path, base_commit: &str) -> bool {
        execute(git(root).args([
            "rev-list",
            "--max-count=1",
            &format!("{base_commit}..{}", self.local_oid),
            "--",
        ]))
        .is_ok_and(|commits| !commits.is_empty())
    }

    pub fn configured_pr_base(&self, root: &Path) -> Result<Option<String>> {
        let key = format!("branch.{}.gh-merge-base", self.local_branch);
        let Some(base) = git_config(root, &key)? else {
            return Ok(None);
        };
        execute(git(root).args(["check-ref-format", &format!("refs/heads/{base}")]))
            .context("Invalid pull request base branch in Git configuration")?;
        Ok(Some(base))
    }

    pub fn pr_head(mut self, default: &str, base: &str) -> Self {
        if (self.head == default || self.head == base) && self.head != self.local_branch {
            self.head.clone_from(&self.local_branch);
            self.set_upstream = true;
            self.synced = false;
        }
        self
    }

    pub fn compare_url(&self, default: &str, base: &str) -> Result<String> {
        ensure!(
            self.synced,
            "GitHub status is required to publish this branch and create a pull request."
        );
        Ok(github_compare(
            &self.owner,
            &self.repo,
            (base != default).then_some(base),
            &self.head,
        ))
    }

    pub fn pr_text(&self, root: &Path) -> Result<(String, String)> {
        let output =
            execute(git(root).args(["show", "-s", "--format=%s%x00%b", &self.local_oid, "--"]))?;
        let output = String::from_utf8_lossy(&output);
        let (title, body) = output
            .split_once('\0')
            .context("Expected a commit message.")?;
        let title = if title.trim().is_empty() {
            &self.head
        } else {
            title.trim()
        };
        Ok((title.to_owned(), body.trim().to_owned()))
    }

    pub fn verify_push_target(&self, root: &Path) -> Result<()> {
        let preferred_remote =
            match git_config(root, &format!("branch.{}.pushRemote", self.local_branch))? {
                Some(remote) => Some(remote),
                None => git_config(root, "remote.pushDefault")?,
            };
        ensure!(
            preferred_remote.is_none_or(|remote| remote == self.remote),
            "Create PR uses remote {}. The checkout has a different Git push remote configured.",
            self.remote
        );
        let urls =
            execute(git(root).args(["remote", "get-url", "--push", "--all", "--", &self.remote]))?;
        let mut urls = std::str::from_utf8(&urls)?.lines();
        let repository = urls.next().and_then(github_repository);
        ensure!(
            urls.next().is_none()
                && repository.is_some_and(|(owner, repo)| {
                    owner.eq_ignore_ascii_case(&self.owner) && repo.eq_ignore_ascii_case(&self.repo)
                }),
            "The push URL must point to the same GitHub repository as the selected remote."
        );
        Ok(())
    }

    pub async fn push(&self, root: &Path) -> Result<()> {
        self.verify_push_target(root)?;
        let refspec = format!("{}:refs/heads/{}", self.local_oid, self.head);
        let mut command = tokio::process::Command::from(git(root));
        command
            .env("GCM_INTERACTIVE", "never")
            .args([
                "push",
                "--no-force",
                "--no-mirror",
                "--no-follow-tags",
                "--recurse-submodules=no",
            ])
            .kill_on_drop(true);
        let output = tokio::time::timeout(
            Duration::from_secs(120),
            command.args(["--", &self.remote, &refspec]).output(),
        )
        .await
        .context("Git push timed out. Check the branch on GitHub before retrying.")??;
        git_output(output).context("Push the selected branch")?;
        Ok(())
    }

    pub fn set_upstream(&self, root: &Path) -> Result<()> {
        if self.set_upstream {
            let remote_key = format!("branch.{}.remote", self.local_branch);
            let merge_key = format!("branch.{}.merge", self.local_branch);
            execute(git(root).args([
                "config",
                "--local",
                "--replace-all",
                &remote_key,
                &self.remote,
            ]))
            .context("Set upstream for the published branch")?;
            let merge_ref = format!("refs/heads/{}", self.head);
            execute(git(root).args(["config", "--local", "--replace-all", &merge_key, &merge_ref]))
                .context("Set upstream for the published branch")?;
        }
        Ok(())
    }

    pub fn same_target(&self, other: &Self) -> bool {
        self.local_branch == other.local_branch
            && self.local_oid == other.local_oid
            && self.remote == other.remote
            && self.head == other.head
            && self.set_upstream == other.set_upstream
            && self.owner.eq_ignore_ascii_case(&other.owner)
            && self.repo.eq_ignore_ascii_case(&other.repo)
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

fn github_compare(owner: &str, repo: &str, base: Option<&str>, head: &str) -> String {
    let mut url = url::Url::parse("https://github.com").unwrap();
    let range = match base {
        Some(base) => format!("{base}...{head}"),
        None => head.to_owned(),
    };
    url.path_segments_mut()
        .unwrap()
        .extend([owner, repo, "compare", &range]);
    url.set_query(Some("expand=1"));
    url.into()
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
        let (mut command, _index) = diff_command(root, &snapshot.base, text).unwrap();
        let patch = String::from_utf8(execute(&mut command).unwrap()).unwrap();
        assert!(patch.contains("+second"));

        let target = PreparedRevert::prepare(root, snapshot.head.clone(), text.clone()).unwrap();
        revert_file(root, &target).unwrap();
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

        let target = PreparedRevert::prepare(root, snapshot.head.clone(), file.clone()).unwrap();
        fs::write(root.join("file"), "newer edit\n").unwrap();
        assert!(revert_file(root, &target).is_err());
        assert_eq!(
            fs::read_to_string(root.join("file")).unwrap(),
            "newer edit\n"
        );
        fs::write(root.join("file"), "modified\n").unwrap();
        execute(git(root).args(["add", "--", "file"])).unwrap();
        assert!(revert_file(root, &target).is_err());
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
        assert!(PreparedRevert::prepare(root, snapshot.head.clone(), file.clone()).is_err());
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
        execute(git(root).args(["switch", "-c", "topic"])).unwrap();
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
        let (mut command, _index) = diff_command(root, &all.base, file).unwrap();
        let patch = String::from_utf8(execute(&mut command).unwrap()).unwrap();
        assert!(patch.contains("-base\n+working tree\n"));
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index_before);

        let local = changed_files(root, ChangeScope::Uncommitted).unwrap();
        assert_eq!(local.files.len(), 1);
        assert_eq!(local.base, all.head);
        assert_eq!(local.files[0].path, restored);
        assert_eq!(local.files[0].change, FileChange::Added);
        assert!(file.revert_removes_file());
        let target = PreparedRevert::prepare(root, all.head.clone(), file.clone()).unwrap();
        revert_file(root, &target).unwrap();
        assert!(!root.join(restored).exists());
        assert!(root.join(committed).exists());
    }

    #[test]
    fn approving_a_binary_diff_distinguishes_same_size_edits() {
        let dir = fixture();
        let root = dir.path();
        let path = root.join("font.bin");
        fs::write(&path, b"\0ab").unwrap();
        let untracked = changed_files(root, ChangeScope::Uncommitted).unwrap();
        let file = &untracked.files[0];
        let original = diff_fingerprint(root, &untracked.base, &untracked.head, file).unwrap();
        fs::write(&path, b"\0ac").unwrap();
        assert_ne!(
            original,
            diff_fingerprint(root, &untracked.base, &untracked.head, file).unwrap()
        );

        commit_fixture(root);
        fs::write(&path, b"\0ad").unwrap();
        let tracked = changed_files(root, ChangeScope::Uncommitted).unwrap();
        let file = &tracked.files[0];
        let original = diff_fingerprint(root, &tracked.base, &tracked.head, file).unwrap();
        fs::write(&path, b"\0ae").unwrap();
        assert_ne!(
            original,
            diff_fingerprint(root, &tracked.base, &tracked.head, file).unwrap()
        );
    }

    #[test]
    fn staged_changes_remain_reviewable_when_the_working_copy_matches_the_base() {
        let dir = fixture();
        let root = dir.path();
        let path = root.join("file");
        fs::write(&path, "base\n").unwrap();
        commit_fixture(root);
        execute(git(root).args(["branch", "-M", "main"])).unwrap();
        execute(git(root).args(["switch", "-c", "topic"])).unwrap();
        fs::write(root.join("branch"), "branch change\n").unwrap();
        commit_fixture(root);
        fs::write(&path, "staged edit\n").unwrap();
        execute(git(root).args(["add", "--", "file"])).unwrap();
        fs::write(&path, "base\n").unwrap();

        for scope in [ChangeScope::All, ChangeScope::Uncommitted] {
            let snapshot = changed_files(root, scope).unwrap();
            let file = snapshot
                .files
                .iter()
                .find(|file| file.path == Path::new("file"))
                .unwrap();
            assert!(diff_fingerprint(root, &snapshot.base, &snapshot.head, file).is_ok());
            let patch = file_patch(root, &snapshot.base, &snapshot.head, file, 1024).unwrap();
            assert!(
                String::from_utf8(patch)
                    .unwrap()
                    .contains("-base\n+staged edit\n")
            );
            for limit in [1, b"\nStaged changes\n\n".len() - 1, 32] {
                let excerpt =
                    file_patch(root, &snapshot.base, &snapshot.head, file, limit).unwrap();
                assert_eq!(excerpt.len(), limit);
            }
        }
    }

    #[test]
    fn approval_and_revert_detect_edits_to_a_recreated_staged_deletion() {
        let dir = fixture();
        let root = dir.path();
        let path = root.join("file");
        fs::write(&path, "base\n").unwrap();
        commit_fixture(root);
        execute(git(root).args(["rm", "--", "file"])).unwrap();
        fs::write(&path, "recreated\n").unwrap();
        let snapshot = changed_files(root, ChangeScope::Uncommitted).unwrap();
        assert_eq!(snapshot.files.len(), 1);
        let file = &snapshot.files[0];
        let target = PreparedRevert::prepare(root, snapshot.head.clone(), file.clone()).unwrap();
        let original = diff_fingerprint(root, &snapshot.base, &snapshot.head, file).unwrap();
        fs::write(&path, "edited\n").unwrap();
        assert_ne!(
            original,
            diff_fingerprint(root, &snapshot.base, &snapshot.head, file).unwrap()
        );
        assert!(revert_file(root, &target).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "edited\n");
    }

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
                github_compare(&owner, &repo, None, "fix/parser#12"),
                "https://github.com/org/project/compare/fix%2Fparser%2312?expand=1"
            );
            assert_eq!(
                github_compare(&owner, &repo, Some("release/v2"), "fix/parser#12"),
                "https://github.com/org/project/compare/release%2Fv2...fix%2Fparser%2312?expand=1"
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
