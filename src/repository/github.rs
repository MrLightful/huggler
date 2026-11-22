//! GitHub pull request queries and guarded updates for a repository binding.
use super::{Driver, RepositoryClient, malformed};
use crate::{Error, RepositoryBindingId, Result};
use reqwest::Method;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

const BRANCH_QUERY: &str = r#"
query BranchPullRequests($owner: String!, $repo: String!, $head: String!, $ref: String!) {
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

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Details {
    id: String,
    number: u64,
    url: String,
    state: String,
    is_draft: bool,
    base_ref_name: String,
    head_ref_oid: String,
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
}

#[derive(Deserialize)]
struct BranchQuery {
    repository: Option<BranchRepository>,
}

#[derive(Deserialize)]
struct BranchRepository {
    #[serde(rename = "ref")]
    head: Option<HeadRef>,
    recent: Nodes<Details>,
}

#[derive(Deserialize)]
struct HeadRef {
    #[serde(rename = "associatedPullRequests")]
    open: Nodes<Details>,
    recent: Nodes<Details>,
}

/// Pull requests associated with the bound repository's head branch.
#[derive(Debug)]
pub struct BranchPullRequests {
    pub open: Vec<PullRequest>,
    pub recent: Vec<PullRequest>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PullRequestState {
    Open,
    Merged,
    Closed,
    Unknown,
}

/// A GitHub snapshot tied to the repository and head branch that returned it.
#[derive(Clone, Debug)]
pub struct PullRequest {
    binding: RepositoryBindingId,
    branch: String,
    details: Details,
}

impl PullRequest {
    pub fn number(&self) -> u64 {
        self.details.number
    }

    pub fn url(&self) -> &str {
        &self.details.url
    }

    pub fn repository(&self) -> &str {
        &self.details.repository.name_with_owner
    }

    pub fn base_branch(&self) -> &str {
        &self.details.base_ref_name
    }

    pub fn head_commit(&self) -> &str {
        &self.details.head_ref_oid
    }

    pub fn same_revision(&self, other: &Self) -> bool {
        self.binding == other.binding
            && self.branch == other.branch
            && self.details.id == other.details.id
            && self.base_branch() == other.base_branch()
            && self.head_commit() == other.head_commit()
    }

    pub fn state(&self) -> PullRequestState {
        match self.details.state.as_str() {
            "OPEN" => PullRequestState::Open,
            "MERGED" => PullRequestState::Merged,
            "CLOSED" => PullRequestState::Closed,
            _ => PullRequestState::Unknown,
        }
    }

    pub fn is_draft(&self) -> bool {
        self.details.is_draft
    }

    pub fn has_conflicts(&self) -> bool {
        self.details.mergeable == "CONFLICTING" || self.details.merge_state_status == "DIRTY"
    }

    pub fn failed_checks(&self) -> bool {
        self.checks().is_some_and(CheckRollup::failed)
    }

    pub fn changes_requested(&self) -> bool {
        self.details.review_decision.as_deref() == Some("CHANGES_REQUESTED")
    }

    pub fn uses_merge_queue(&self) -> bool {
        self.details.is_merge_queue_enabled
    }

    pub fn is_queued(&self) -> bool {
        self.details.merge_queue_entry.is_some()
    }

    pub fn auto_merge_enabled(&self) -> bool {
        self.details.auto_merge_request.is_some()
    }

    pub fn allows_merge_method(&self, method: MergeMethod) -> bool {
        let repo = &self.details.repository;
        match method {
            MergeMethod::Merge => repo.merge_commit_allowed,
            MergeMethod::Squash => repo.squash_merge_allowed,
            MergeMethod::Rebase => repo.rebase_merge_allowed && self.details.can_be_rebased,
        }
    }

    pub fn can_merge_now(&self) -> bool {
        self.can_update()
            && matches!(
                self.details.repository.viewer_permission.as_deref(),
                Some("ADMIN" | "MAINTAIN" | "WRITE")
            )
            && self.details.mergeable == "MERGEABLE"
            && matches!(
                self.details.merge_state_status.as_str(),
                "CLEAN" | "HAS_HOOKS"
            )
            && self.checks().is_none_or(|checks| checks.state == "SUCCESS")
    }

    pub fn can_enable_auto_merge(&self) -> bool {
        self.can_update() && self.details.viewer_can_enable_auto_merge
    }

    fn can_update(&self) -> bool {
        self.state() == PullRequestState::Open
            && !self.is_draft()
            && !self.is_queued()
            && !self.auto_merge_enabled()
            && !self.has_conflicts()
            && !self.failed_checks()
            && !self.changes_requested()
    }

    fn checks(&self) -> Option<&CheckRollup> {
        self.details
            .commits
            .nodes
            .last()
            .and_then(|node| node.commit.status_check_rollup.as_ref())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PullRequestUpdate {
    Merge(MergeMethod),
    /// Use None when the merge queue chooses the method.
    EnableAutoMerge(Option<MergeMethod>),
    Enqueue,
}

impl PullRequestUpdate {
    fn allowed(self, pr: &PullRequest) -> bool {
        match self {
            Self::Merge(method) => {
                pr.can_merge_now() && !pr.uses_merge_queue() && pr.allows_merge_method(method)
            }
            Self::EnableAutoMerge(method) => {
                pr.can_enable_auto_merge()
                    && match method {
                        Some(method) => !pr.uses_merge_queue() && pr.allows_merge_method(method),
                        None => pr.uses_merge_queue(),
                    }
            }
            Self::Enqueue => pr.can_merge_now() && pr.uses_merge_queue(),
        }
    }
}

impl RepositoryClient {
    /// Read open requests and the most recently updated retired requests using this head branch.
    /// A fork can return a PR whose base is in another GitHub repository.
    pub async fn github_pull_requests(&self, branch: &str) -> Result<BranchPullRequests> {
        if self.binding.driver != Driver::GitHub {
            return Err(Error::Unsupported(
                "pull request operations require GitHub".into(),
            ));
        }
        if branch.trim().is_empty() {
            return Err(Error::Invalid("expected a head branch name".into()));
        }
        let (owner, repo) = self
            .binding
            .repository
            .split_once('/')
            .ok_or_else(malformed)?;
        let data: BranchQuery = self
            .github_graphql(
                BRANCH_QUERY,
                json!({
                    "owner": owner,
                    "repo": repo,
                    "head": branch,
                    "ref": format!("refs/heads/{branch}"),
                }),
            )
            .await?;
        let repository = data
            .repository
            .ok_or_else(|| Error::NotFound("GitHub repository".into()))?;
        let mut recent = Vec::new();
        let mut open = Vec::new();
        if let Some(head) = repository.head {
            open = head.open.nodes;
            recent = head.recent.nodes;
        }
        recent.extend(repository.recent.nodes);
        recent.retain(|pr| {
            pr.head_repository.as_ref().map_or_else(
                || {
                    !pr.is_cross_repository
                        && pr
                            .repository
                            .name_with_owner
                            .eq_ignore_ascii_case(&self.binding.repository)
                },
                |head| {
                    head.name_with_owner
                        .eq_ignore_ascii_case(&self.binding.repository)
                },
            )
        });
        let snapshot = |details| PullRequest {
            binding: self.binding.id.clone(),
            branch: branch.to_owned(),
            details,
        };
        Ok(BranchPullRequests {
            open: open.into_iter().map(snapshot).collect(),
            recent: recent.into_iter().map(snapshot).collect(),
        })
    }

    /// Revalidate the approved snapshot and action, then ask GitHub to apply it
    /// only to the expected head commit. A failed write can leave its outcome uncertain.
    pub async fn update_github_pull_request(
        &self,
        previous: &PullRequest,
        action: PullRequestUpdate,
    ) -> Result<()> {
        if previous.binding != self.binding.id {
            return Err(Error::Forbidden);
        }
        let fresh = self.github_pull_requests(&previous.branch).await?;
        let [pr] = fresh.open.as_slice() else {
            return Err(changed_pull_request());
        };
        if !previous.same_revision(pr) || !action.allowed(pr) {
            return Err(changed_pull_request());
        }
        let (field, receipt, method) = match action {
            PullRequestUpdate::Merge(method) => ("mergePullRequest", "pullRequest", Some(method)),
            PullRequestUpdate::EnableAutoMerge(method) => {
                ("enablePullRequestAutoMerge", "pullRequest", method)
            }
            PullRequestUpdate::Enqueue => ("enqueuePullRequest", "mergeQueueEntry", None),
        };
        let mut variables = json!({"id": pr.details.id, "head": pr.head_commit()});
        let (declaration, argument) = if let Some(method) = method {
            variables["method"] = json!(method.graphql());
            (
                ", $method: PullRequestMergeMethod!",
                ", mergeMethod: $method",
            )
        } else {
            ("", "")
        };
        let query = format!(
            "mutation UpdatePullRequest($id: ID!, $head: GitObjectID!{declaration}) {{\n\
              {field}(input: {{pullRequestId: $id, expectedHeadOid: $head{argument}}}) {{ {receipt} {{ id }} }}\n\
            }}"
        );
        let outcome = self.github_graphql::<Value>(&query, variables).await;
        if matches!(&outcome, Ok(data) if data[field][receipt]["id"].as_str().is_some_and(|id| !id.is_empty()))
        {
            return Ok(());
        }
        let reason = match outcome {
            Ok(_) => "missing GitHub receipt".to_owned(),
            Err(error) => error.to_string(),
        };
        Err(Error::Integration(format!(
            "Could not confirm the update. Check the pull request before retrying. {reason}"
        )))
    }

    async fn github_graphql<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<T> {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.request(
                Method::POST,
                "/graphql",
                Some(json!({"query": query, "variables": variables})),
            ),
        )
        .await
        .map_err(|_| Error::Integration("GitHub request timed out".into()))??;
        if response.get("errors").is_some() {
            return Err(Error::Integration(
                "GitHub rejected the pull request request".into(),
            ));
        }
        #[derive(Deserialize)]
        struct Response<T> {
            data: T,
        }
        Ok(serde_json::from_value::<Response<T>>(response)
            .map_err(|_| malformed())?
            .data)
    }
}

fn changed_pull_request() -> Error {
    Error::Invalid(
        "The checkout or pull request status changed. Review the current action and try again."
            .into(),
    )
}
