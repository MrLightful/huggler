//! Connect the selected checkout to Huggler's repository client using GitHub CLI credentials.
use crate::git;
use anyhow::{Context, Result, bail, ensure};
use huggler::{
    Huggler, ThreadId,
    repository::{
        Driver, RepositoryClient,
        github::{
            BranchPullRequests, MergeMethod, PullRequest, PullRequestState, PullRequestUpdate,
        },
    },
};
use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Create,
    Resolve,
    Fix,
    Update(PullRequestUpdate),
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
            Self::Found { branch, pr } => primary_action(branch, pr),
            Self::Multiple(_) => Action::View,
        }
    }
}

fn primary_action(branch: &git::GitHubBranch, pr: &PullRequest) -> Action {
    match pr.state() {
        PullRequestState::Merged => return Action::Merged,
        PullRequestState::Closed => return Action::Closed,
        PullRequestState::Unknown => return Action::View,
        PullRequestState::Open => {}
    }
    if pr.has_conflicts() {
        return Action::Resolve;
    }
    if pr.failed_checks() || pr.changes_requested() {
        return Action::Fix;
    }
    if pr.is_draft() {
        return Action::Draft;
    }
    if pr.is_queued() {
        return Action::Queued;
    }
    if pr.auto_merge_enabled() {
        return Action::AutoMergeEnabled;
    }
    if pr.head_commit() != branch.local_oid {
        return Action::View;
    }
    let method = [MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase]
        .into_iter()
        .find(|method| pr.allows_merge_method(*method));
    let update = match (pr.uses_merge_queue(), method) {
        (true, _) if pr.can_merge_now() => PullRequestUpdate::Enqueue,
        (false, Some(method)) if pr.can_merge_now() => PullRequestUpdate::Merge(method),
        (true, _) if pr.can_enable_auto_merge() => PullRequestUpdate::EnableAutoMerge(None),
        (false, Some(method)) if pr.can_enable_auto_merge() => {
            PullRequestUpdate::EnableAutoMerge(Some(method))
        }
        _ => return Action::View,
    };
    Action::Update(update)
}

#[derive(Clone)]
pub(super) struct Checkout {
    pub runtime: Arc<Huggler>,
    pub thread: ThreadId,
    pub root: PathBuf,
}

impl Checkout {
    pub async fn status(&self) -> Status {
        if git::has_conflicts(&self.root) {
            return Status::Conflicts;
        }
        let Ok(branch) = git::github_branch(&self.root) else {
            return Status::Create;
        };
        self.branch_status(branch).await
    }

    async fn branch_status(&self, branch: git::GitHubBranch) -> Status {
        let requests = async {
            let client = self.client(&branch).await?;
            Ok::<_, anyhow::Error>(client.github_pull_requests(&branch.head).await?)
        }
        .await;
        match requests {
            Ok(requests) => select_pull_request(branch, requests),
            Err(error) => Status::Unavailable(format!("{error:#}")),
        }
    }

    pub async fn create_url(&self) -> Result<String> {
        let branch = git::github_branch(&self.root)?;
        ensure!(
            matches!(
                self.branch_status(branch.clone()).await,
                Status::Create | Status::Unavailable(_)
            ),
            "The checkout or pull request status changed. Review the current action and try again."
        );
        self.verify_checkout(&branch)?;
        branch.compare_url()
    }

    pub async fn update(
        &self,
        branch: &git::GitHubBranch,
        pr: &PullRequest,
        action: PullRequestUpdate,
    ) -> Result<()> {
        let client = self.client(branch).await?;
        self.verify_checkout(branch)?;
        client.update_github_pull_request(pr, action).await?;
        Ok(())
    }

    pub async fn handoff(&self, previous: &Status, action: Action) -> Result<String> {
        match previous {
            Status::Conflicts if action == Action::Resolve && git::has_conflicts(&self.root) => Ok(
                "Resolve the Git merge conflicts in this checkout. Inspect the operation in progress, preserve the intended changes from both sides and any unrelated edits, then run the relevant checks and summarize the result."
                    .into(),
            ),
            Status::Found { branch, pr } => {
                let fresh = self.status().await;
                if let Status::Unavailable(error) = &fresh {
                    bail!("Could not verify the pull request. {error}");
                }
                ensure!(
                    matches!(fresh, Status::Found { branch: current, pr: updated }
                        if &current == branch
                            && pr.same_revision(&updated)
                            && primary_action(&current, &updated) == action),
                    "The checkout or pull request status changed. Review the current action and try again."
                );
                self.verify_checkout(branch)?;
                let target = format!("{} #{} ({})", pr.repository(), pr.number(), pr.url());
                match action {
                    Action::Resolve => Ok(format!(
                        "Resolve the merge conflicts for {target} in this checkout. Verify the PR head ({}) first, reconcile it with the base branch ({}) while preserving unrelated edits, and run the relevant checks. Leave the resolution in this checkout for review and summarize it.",
                        pr.head_commit(), pr.base_branch()
                    )),
                    Action::Fix => Ok(format!(
                        "Fix the failed checks or address requested review changes for {target} in this checkout. Verify the PR head ({}) first, inspect the latest GitHub check logs and reviews, and verify the changes locally. Leave the changes in this checkout for review and summarize what you fixed.",
                        pr.head_commit()
                    )),
                    _ => bail!("Unsupported pull request handoff."),
                }
            }
            _ => bail!("The checkout or pull request status changed. Review the current action and try again."),
        }
    }

    fn verify_checkout(&self, branch: &git::GitHubBranch) -> Result<()> {
        ensure!(
            !git::has_conflicts(&self.root) && git::github_branch(&self.root)? == *branch,
            "The checkout or pull request status changed. Review the current action and try again."
        );
        Ok(())
    }

    async fn client(&self, branch: &git::GitHubBranch) -> Result<RepositoryClient> {
        let authorization = github_authorization().await?;
        let binding = self.runtime.bind_thread_repository(
            &self.thread,
            Driver::GitHub,
            &github_api_origin(),
            &format!("{}/{}", branch.owner, branch.repo),
        )?;
        Ok(self
            .runtime
            .repository_client(&binding.project, &binding.id, Some(&authorization))?)
    }
}

fn select_pull_request(branch: git::GitHubBranch, mut requests: BranchPullRequests) -> Status {
    let pr = match requests.open.len() {
        0 => {
            // A branch name can be reused for another PR after closing or merging.
            requests
                .recent
                .into_iter()
                .find(|pr| pr.head_commit() == branch.local_oid)
        }
        1 => requests.open.pop(),
        _ => return Status::Multiple(requests.open),
    };
    match pr {
        Some(pr) => Status::Found {
            branch,
            pr: Box::new(pr),
        },
        None => Status::Create,
    }
}

fn github_api_origin() -> String {
    #[cfg(all(feature = "qualification", target_os = "macos"))]
    if std::env::var_os("HUGGLER_UI_SCRIPT").is_some()
        && let Ok(origin) = std::env::var("HUGGLER_QUALIFICATION_GITHUB_API")
        && let Ok(url) = url::Url::parse(&origin)
        && url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
    {
        return origin;
    }
    "https://api.github.com".into()
}

async fn github_authorization() -> Result<String> {
    let run = |executable: &Path| {
        let mut command = Command::new(executable);
        command
            .args(["auth", "token", "--hostname", "github.com"])
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env_remove("GH_DEBUG")
            .env_remove("DEBUG")
            .kill_on_drop(true)
            .stdin(Stdio::null());
        async move {
            tokio::time::timeout(Duration::from_secs(10), command.output())
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "GitHub CLI timed out"))?
        }
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
    .context("Read GitHub CLI credentials")?;
    // Never include CLI output in an error or persist the credential.
    ensure!(
        output.status.success() && !output.stdout.is_empty(),
        "Sign in with GitHub CLI to show pull request status."
    );
    let token = std::str::from_utf8(&output.stdout)
        .ok()
        .map(str::trim)
        .filter(|token| !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_graphic()))
        .context("GitHub CLI returned an invalid credential")?;
    Ok(format!("Bearer {token}"))
}
