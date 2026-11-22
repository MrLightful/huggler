//! GitHub pull requests for the desktop's selected checkout.
use crate::git;
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use smol::process::{Command, Stdio};
use std::{io, path::Path, time::Duration};

const BRANCH_QUERY: &str = r#"
query($owner: String!, $repo: String!, $head: String!, $ref: String!) {
  repository(owner: $owner, name: $repo) {
    ref(qualifiedName: $ref) {
      associatedPullRequests(first: 20, states: [OPEN], orderBy: {field: CREATED_AT, direction: DESC}) {
        nodes { ...PullRequest }
      }
      recent: associatedPullRequests(first: 1, states: [MERGED, CLOSED], orderBy: {field: UPDATED_AT, direction: DESC}) {
        nodes { ...PullRequest }
      }
    }
    recent: pullRequests(first: 1, headRefName: $head, states: [MERGED, CLOSED], orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes { ...PullRequest }
    }
  }
}

fragment PullRequest on PullRequest {
  id number url state isDraft baseRefName headRefOid mergeable mergeStateStatus canBeRebased
  reviewDecision viewerCanEnableAutoMerge
  autoMergeRequest { mergeMethod }
  isMergeQueueEnabled
  mergeQueueEntry { id }
  isCrossRepository
  headRepository { nameWithOwner }
  repository { nameWithOwner viewerPermission mergeCommitAllowed squashMergeAllowed rebaseMergeAllowed }
  commits(last: 1) {
    nodes {
      commit {
        statusCheckRollup {
          state
          contexts(first: 100) {
            nodes {
              ... on CheckRun { conclusion }
              ... on StatusContext { state }
            }
          }
        }
      }
    }
  }
}
"#;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Create,
    Resolve,
    Fix,
    Merge,
    AutoMerge,
    Queue,
    View,
    Draft,
    AutoMergeEnabled,
    Queued,
    Merged,
    Closed,
}

#[derive(Clone, Debug)]
pub enum Status {
    Create,
    Conflicts,
    Found {
        branch: git::GitHubBranch,
        pr: Box<PullRequest>,
    },
    Multiple(Vec<PullRequest>),
    Unavailable(String),
}

impl Status {
    pub fn action(&self) -> Action {
        match self {
            Self::Create | Self::Unavailable(_) => Action::Create,
            Self::Conflicts => Action::Resolve,
            Self::Found { branch, pr } => pr.action(branch),
            Self::Multiple(_) => Action::View,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    id: String,
    pub number: u64,
    pub url: String,
    state: String,
    is_draft: bool,
    pub base_ref_name: String,
    pub head_ref_oid: String,
    mergeable: String,
    merge_state_status: String,
    can_be_rebased: bool,
    review_decision: Option<String>,
    viewer_can_enable_auto_merge: bool,
    auto_merge_request: Option<serde::de::IgnoredAny>,
    is_merge_queue_enabled: bool,
    merge_queue_entry: Option<serde::de::IgnoredAny>,
    is_cross_repository: bool,
    head_repository: Option<HeadRepository>,
    repository: Repository,
    commits: Nodes<CommitNode>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeadRepository {
    name_with_owner: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    name_with_owner: String,
    viewer_permission: Option<String>,
    merge_commit_allowed: bool,
    squash_merge_allowed: bool,
    rebase_merge_allowed: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Clone, Debug, Deserialize)]
struct CommitNode {
    commit: Commit,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Commit {
    status_check_rollup: Option<CheckRollup>,
}

#[derive(Clone, Debug, Deserialize)]
struct CheckRollup {
    state: String,
    contexts: Nodes<Check>,
}

#[derive(Clone, Debug, Deserialize)]
struct Check {
    conclusion: Option<String>,
    state: Option<String>,
}

impl CheckRollup {
    fn failed(&self) -> bool {
        matches!(self.state.as_str(), "ERROR" | "FAILURE")
            || self.contexts.nodes.iter().any(|check| {
                matches!(check.state.as_deref(), Some("ERROR" | "FAILURE"))
                    || matches!(
                        check.conclusion.as_deref(),
                        Some(
                            "FAILURE"
                                | "CANCELLED"
                                | "TIMED_OUT"
                                | "ACTION_REQUIRED"
                                | "STARTUP_FAILURE"
                                | "STALE"
                        )
                    )
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    fn graphql(self) -> &'static str {
        match self {
            Self::Merge => "MERGE",
            Self::Squash => "SQUASH",
            Self::Rebase => "REBASE",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Merge => "Create a merge commit",
            Self::Squash => "Squash and merge",
            Self::Rebase => "Rebase and merge",
        }
    }
}

impl PullRequest {
    pub fn repository(&self) -> &str {
        &self.repository.name_with_owner
    }

    pub fn uses_merge_queue(&self) -> bool {
        self.is_merge_queue_enabled
    }

    pub fn merge_method(&self) -> Option<MergeMethod> {
        let repo = &self.repository;
        [
            (repo.merge_commit_allowed, MergeMethod::Merge),
            (repo.squash_merge_allowed, MergeMethod::Squash),
            (
                repo.rebase_merge_allowed && self.can_be_rebased,
                MergeMethod::Rebase,
            ),
        ]
        .into_iter()
        .find_map(|(allowed, method)| allowed.then_some(method))
    }

    fn action(&self, branch: &git::GitHubBranch) -> Action {
        match self.state.as_str() {
            "MERGED" => return Action::Merged,
            "CLOSED" => return Action::Closed,
            "OPEN" => {}
            _ => return Action::View,
        }
        if self.mergeable == "CONFLICTING" || self.merge_state_status == "DIRTY" {
            return Action::Resolve;
        }
        let checks = self
            .commits
            .nodes
            .last()
            .and_then(|node| node.commit.status_check_rollup.as_ref());
        if checks.is_some_and(CheckRollup::failed)
            || self.review_decision.as_deref() == Some("CHANGES_REQUESTED")
        {
            return Action::Fix;
        }
        if self.is_draft {
            return Action::Draft;
        }
        if self.merge_queue_entry.is_some() {
            return Action::Queued;
        }
        if self.auto_merge_request.is_some() {
            return Action::AutoMergeEnabled;
        }
        if self.head_ref_oid != branch.local_oid {
            return Action::View;
        }
        let can_write = matches!(
            self.repository.viewer_permission.as_deref(),
            Some("ADMIN" | "MAINTAIN" | "WRITE")
        );
        let ready = self.mergeable == "MERGEABLE"
            && matches!(self.merge_state_status.as_str(), "CLEAN" | "HAS_HOOKS")
            && checks.is_none_or(|checks| checks.state == "SUCCESS");
        if ready && can_write && self.is_merge_queue_enabled {
            return Action::Queue;
        }
        let has_method = self.merge_method().is_some();
        if ready && can_write && has_method {
            return Action::Merge;
        }
        if self.viewer_can_enable_auto_merge && (self.is_merge_queue_enabled || has_method) {
            return Action::AutoMerge;
        }
        Action::View
    }
}

#[derive(Deserialize)]
struct BranchQuery {
    repository: Option<BranchRepository>,
}

#[derive(Deserialize)]
struct BranchRepository {
    #[serde(rename = "ref")]
    head: Option<HeadRef>,
    recent: Nodes<PullRequest>,
}

#[derive(Deserialize)]
struct HeadRef {
    #[serde(rename = "associatedPullRequests")]
    open: Nodes<PullRequest>,
    recent: Nodes<PullRequest>,
}

pub async fn status(root: &Path) -> Status {
    if git::has_conflicts(root) {
        return Status::Conflicts;
    }
    let Ok(branch) = git::github_branch(root) else {
        return Status::Create;
    };
    branch_status(branch).await
}

async fn branch_status(branch: git::GitHubBranch) -> Status {
    match read_branch(&branch).await {
        Ok(repo) => select_pull_request(branch, repo),
        Err(error) => Status::Unavailable(format!("{error:#}")),
    }
}

pub async fn create_url(root: &Path) -> Result<String> {
    let branch = git::github_branch(root)?;
    ensure!(
        matches!(
            branch_status(branch.clone()).await,
            Status::Create | Status::Unavailable(_)
        ) && !git::has_conflicts(root)
            && git::github_branch(root)? == branch,
        "The checkout or pull request status changed. Review the current action and try again."
    );
    branch.compare_url()
}

async fn read_branch(branch: &git::GitHubBranch) -> Result<BranchRepository> {
    let data: BranchQuery = graphql(
        BRANCH_QUERY,
        &[
            ("owner", &branch.owner),
            ("repo", &branch.repo),
            ("head", &branch.head),
            ("ref", &format!("refs/heads/{}", branch.head)),
        ],
    )
    .await?;
    data.repository.context("GitHub repository unavailable.")
}

fn select_pull_request(branch: git::GitHubBranch, repo: BranchRepository) -> Status {
    let mut fallback = repo.recent.nodes;
    if let Some(mut head) = repo.head {
        match head.open.nodes.len() {
            0 => {}
            1 => {
                return Status::Found {
                    branch,
                    pr: Box::new(head.open.nodes.remove(0)),
                };
            }
            _ => return Status::Multiple(head.open.nodes),
        }
        fallback.splice(..0, head.recent.nodes);
    }
    let name = format!("{}/{}", branch.owner, branch.repo);
    // A retired PR only belongs to this checkout if its head is still the
    // checked-out commit. A branch name can be reused for the next PR.
    let recent = fallback.into_iter().find(|pr| {
        pr.head_ref_oid == branch.local_oid
            && pr.head_repository.as_ref().map_or_else(
                || !pr.is_cross_repository && pr.repository().eq_ignore_ascii_case(&name),
                |head| head.name_with_owner.eq_ignore_ascii_case(&name),
            )
    });
    match recent {
        Some(pr) => Status::Found {
            branch,
            pr: Box::new(pr),
        },
        None => Status::Create,
    }
}

pub async fn update(
    root: &Path,
    branch: &git::GitHubBranch,
    pr: &PullRequest,
    action: Action,
) -> Result<()> {
    verify_action(root, branch, pr, action).await?;
    let (field, receipt) = match action {
        Action::Merge => ("mergePullRequest", "pullRequest"),
        Action::AutoMerge => ("enablePullRequestAutoMerge", "pullRequest"),
        Action::Queue => ("enqueuePullRequest", "mergeQueueEntry"),
        _ => bail!("Unsupported pull request update."),
    };
    let mut variables = vec![("id", pr.id.as_str()), ("head", pr.head_ref_oid.as_str())];
    let (method, argument) = if pr.uses_merge_queue() {
        ("", "")
    } else {
        let merge_method = pr.merge_method().context("No merge method is enabled.")?;
        variables.push(("method", merge_method.graphql()));
        (
            ", $method: PullRequestMergeMethod!",
            ", mergeMethod: $method",
        )
    };
    let query = format!(
        "mutation($id: ID!, $head: GitObjectID!{method}) {{\n\
            {field}(input: {{pullRequestId: $id, expectedHeadOid: $head{argument}}}) {{ {receipt} {{ id }} }}\n\
        }}"
    );
    let result: serde_json::Value = graphql(&query, &variables)
        .await
        .context("Could not confirm the update. Check the pull request before retrying")?;
    ensure!(
        result[field][receipt]["id"].is_string(),
        "GitHub did not confirm the pull request update. Check the pull request before retrying."
    );
    Ok(())
}

pub async fn handoff(root: &Path, previous: &Status, action: Action) -> Result<String> {
    match previous {
        Status::Conflicts if action == Action::Resolve && git::has_conflicts(root) => Ok(
            "Resolve the Git merge conflicts in this checkout. Inspect the operation in progress, preserve the intended changes from both sides and any unrelated edits, then run the relevant checks and summarize the result."
                .into(),
        ),
        Status::Found { branch, pr } => {
            verify_action(root, branch, pr, action).await?;
            let target = format!("{} #{} ({})", pr.repository(), pr.number, pr.url);
            match action {
                Action::Resolve => Ok(format!(
                    "Resolve the merge conflicts for {target} in this checkout. Verify the PR head ({}) first, reconcile it with the base branch ({}) while preserving unrelated edits, and run the relevant checks. Leave the resolution in this checkout for review and summarize it.",
                    pr.head_ref_oid, pr.base_ref_name
                )),
                Action::Fix => Ok(format!(
                    "Fix the failed checks or address requested review changes for {target} in this checkout. Verify the PR head ({}) first, inspect the latest GitHub check logs and reviews, and verify the changes locally. Leave the changes in this checkout for review and summarize what you fixed.",
                    pr.head_ref_oid
                )),
                _ => bail!("Unsupported pull request handoff."),
            }
        }
        _ => bail!("The checkout or pull request status changed. Review the current action and try again."),
    }
}

async fn verify_action(
    root: &Path,
    branch: &git::GitHubBranch,
    pr: &PullRequest,
    action: Action,
) -> Result<()> {
    let fresh = status(root).await;
    if let Status::Unavailable(error) = &fresh {
        bail!("Could not verify the pull request. {error}");
    }
    ensure!(
        matches!(fresh, Status::Found { branch: current, pr: updated }
            if &current == branch
                && updated.id == pr.id
                && updated.base_ref_name == pr.base_ref_name
                && updated.head_ref_oid == pr.head_ref_oid
                && updated.action(&current) == action
                && (action != Action::AutoMerge
                    || updated.uses_merge_queue() == pr.uses_merge_queue())
                && (!matches!(action, Action::Merge | Action::AutoMerge)
                    || pr.uses_merge_queue()
                    || updated.merge_method() == pr.merge_method()))
            && !git::has_conflicts(root)
            && git::github_branch(root)? == *branch,
        "The checkout or pull request status changed. Review the current action and try again."
    );
    Ok(())
}

async fn graphql<T: serde::de::DeserializeOwned>(
    query: &str,
    variables: &[(&str, &str)],
) -> Result<T> {
    let run = |executable: &Path| {
        let mut cmd = Command::new(executable);
        cmd.args(["api", "graphql", "--hostname", "github.com", "-f"])
            .arg(format!("query={query}"))
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env_remove("GH_DEBUG")
            .env_remove("DEBUG")
            .kill_on_drop(true)
            .stdin(Stdio::null());
        for (name, value) in variables {
            cmd.args(["-f", &format!("{name}={value}")]);
        }
        smol::future::race(cmd.output(), async {
            smol::Timer::after(REQUEST_TIMEOUT).await;
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "GitHub CLI timed out",
            ))
        })
    };
    let output = match run(Path::new("gh")).await {
        // Finder-launched apps can have a system-only PATH.
        Err(error) if cfg!(target_os = "macos") && error.kind() == io::ErrorKind::NotFound => {
            let executable = ["/opt/homebrew/bin/gh", "/usr/local/bin/gh"]
                .into_iter()
                .find(|path| Path::new(path).is_file())
                .context("Install GitHub CLI to show pull request status.")?;
            run(Path::new(executable)).await
        }
        result => result,
    }
    .context("GitHub CLI request failed")?;
    ensure!(
        output.status.success(),
        "GitHub CLI could not complete the request. {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    #[derive(Deserialize)]
    struct Response<T> {
        data: T,
    }
    Ok(serde_json::from_slice::<Response<T>>(&output.stdout)
        .context("Read GitHub response")?
        .data)
}
