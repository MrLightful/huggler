//! Project-bound SCM and issue tracker reads, comments, links and watch delivery.
use crate::*;
use reqwest::{Client, Method, Url, header::HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Driver {
    GitHub,
    GitLab,
    Bitbucket,
    AzureDevOps,
    Forgejo,
    GitCafe,
    CursorOrigin,
    Linear,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    pub id: RepositoryBindingId,
    pub project: ProjectId,
    pub driver: Driver,
    /// Service API origin, e.g. https://api.github.com.
    pub origin: String,
    /// Owner/repo, Azure org/project/repo, GitLab namespace path, or Linear team key.
    pub repository: String,
}

fn segment(value: &str) -> bool {
    !matches!(value, "" | "." | "..")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

impl Huggler {
    pub fn bind_repository(
        &self,
        project: &ProjectId,
        driver: Driver,
        origin: &str,
        repository: &str,
    ) -> Result<Binding> {
        let url =
            Url::parse(origin).map_err(|_| Error::Invalid("invalid service origin".into()))?;
        let loopback =
            url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "[::1]"));
        let parts = repository.split('/').count();
        let valid_parts = match driver {
            Driver::Linear => parts == 1,
            Driver::AzureDevOps => parts == 3,
            Driver::GitLab => parts >= 2,
            _ => parts == 2,
        };
        if (!loopback && url.scheme() != "https")
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !valid_parts
            || repository.len() > 512
            || !repository.split('/').all(segment)
        {
            return Err(Error::Invalid(
                "invalid API origin or repository path".into(),
            ));
        }
        self.shared.access(|db| {
            db.bind_repository(
                project,
                driver,
                url.origin().ascii_serialization(),
                repository,
            )
        })
    }

    /// Create a client for a trusted host. Pass the service's complete Authorization header.
    /// Credentials are scoped to the persisted origin. Redirects are disabled.
    pub fn repository_client(
        &self,
        project: &ProjectId,
        binding: &RepositoryBindingId,
        authorization: Option<&str>,
    ) -> Result<RepositoryClient> {
        let binding = self
            .shared
            .access(|db| db.repository_binding(project, binding))?;
        let raw = authorization
            .filter(|s| !s.trim().is_empty())
            .ok_or(Error::LoginRequired)?;
        let mut authorization = HeaderValue::from_str(raw)
            .map_err(|_| Error::Invalid("invalid authorization header".into()))?;
        authorization.set_sensitive(true);
        let http = Client::builder()
            .user_agent(&self.output().name)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(transport)?;
        Ok(RepositoryClient {
            binding,
            authorization,
            http,
        })
    }
    /// Link a pull-request number or a Linear issue identifier to its project's thread.
    pub fn link_repository_item(
        &self,
        thread: &ThreadId,
        binding: &RepositoryBindingId,
        item: &str,
    ) -> Result<RepositoryLinkId> {
        self.shared
            .access(|db| db.link_repository_item(thread, binding, item))
    }
    pub fn repository_links(&self, thread: &ThreadId) -> Result<Vec<LinkedItem>> {
        self.shared.access(|db| db.repository_links(thread))
    }
    pub fn unlink_repository_item(
        &self,
        thread: &ThreadId,
        link: &RepositoryLinkId,
    ) -> Result<bool> {
        self.shared
            .access(|db| db.unlink_repository_item(thread, link))
    }
    pub fn watch_repository_item(
        &self,
        thread: &ThreadId,
        link: &RepositoryLinkId,
    ) -> Result<RepositoryWatchId> {
        self.shared
            .access(|db| db.watch_repository_item(thread, link))
    }
    pub fn unwatch_repository_item(
        &self,
        thread: &ThreadId,
        watch: &RepositoryWatchId,
    ) -> Result<bool> {
        self.shared
            .access(|db| db.unwatch_repository_item(thread, watch))
    }
    /// Submit a trusted observation. The first revision establishes a baseline. A newer,
    /// changed state queues one wake. Removing a watch invalidates its observation handle.
    pub fn sync_repository_watch(
        &self,
        watch: &RepositoryWatchId,
        revision: i64,
        state: &str,
    ) -> Result<Option<RunId>> {
        if revision < 0 || state.trim().is_empty() || state.len() > 4096 {
            return Err(Error::Invalid("invalid watch observation".into()));
        }
        self.shared
            .schedule(|db| db.sync_repository_watch(watch, revision, state))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct LinkedItem {
    pub link: RepositoryLinkId,
    pub binding: RepositoryBindingId,
    pub item: String,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Item {
    pub key: String,
    pub title: String,
    pub state: String,
}
pub struct RepositoryClient {
    binding: Binding,
    authorization: HeaderValue,
    http: Client,
}
fn transport(_: reqwest::Error) -> Error {
    Error::Integration("repository service transport failed".into())
}
fn malformed() -> Error {
    Error::Integration("unexpected repository service response".into())
}

impl RepositoryClient {
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.binding.origin))
            .header("Authorization", self.authorization.clone())
            .header("Accept", "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(transport)?;
        match response.status().as_u16() {
            401 => return Err(Error::LoginRequired),
            403 => return Err(Error::Forbidden),
            200..=299 => (),
            code => {
                return Err(Error::Integration(format!(
                    "repository service returned {code}"
                )));
            }
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport)? {
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                return Err(Error::Integration(
                    "repository response exceeds 4 MiB".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| malformed())
    }

    /// Fetch the first service page of pull requests, or the first page of issues in a Linear team.
    pub async fn list(&self) -> Result<Vec<Item>> {
        use Driver::*;
        let repo = &self.binding.repository;
        let (path, rows, id, state) = match self.binding.driver {
            GitHub => (format!("/repos/{repo}/pulls"), "", "number", "/state"),
            Forgejo => (
                format!("/api/v1/repos/{repo}/pulls"),
                "",
                "number",
                "/state",
            ),
            GitLab => (
                format!(
                    "/api/v4/projects/{}/merge_requests",
                    repo.replace('/', "%2F")
                ),
                "",
                "iid",
                "/state",
            ),
            Bitbucket => (
                format!("/2.0/repositories/{repo}/pullrequests"),
                "/values",
                "id",
                "/state",
            ),
            AzureDevOps => {
                let (project, name) = repo.rsplit_once('/').ok_or_else(malformed)?;
                (
                    format!(
                        "/{project}/_apis/git/repositories/{name}/pullrequests?api-version=7.1"
                    ),
                    "/value",
                    "pullRequestId",
                    "/status",
                )
            }
            GitCafe => (
                format!("/api/repos/{repo}/pulls?limit=50&sort=newest&state=open"),
                "/items",
                "number",
                "/state",
            ),
            CursorOrigin => (
                format!("/v1/origin/repos/{repo}/pulls?state=open&pageSize=50&sortBy=updated"),
                "/pullRequests",
                "number",
                "/state",
            ),
            Linear => (
                "/graphql".into(),
                "/data/issues/nodes",
                "identifier",
                "/state/name",
            ),
        };
        let response = if self.binding.driver == Linear {
            self.linear_query(
                "query Issues($team: String!) { issues(first: 50, filter: {team: {key: {eq: $team}}}) { nodes { identifier title state { name } } } }",
                json!({"team": repo}),
            ).await?
        } else {
            self.request(Method::GET, &path, None).await?
        };
        if response.get("errors").is_some() {
            return Err(malformed());
        }
        response
            .pointer(rows)
            .and_then(Value::as_array)
            .ok_or_else(malformed)?
            .iter()
            .map(|row| {
                let key = if self.binding.driver == Linear {
                    row[id].as_str().ok_or_else(malformed)?.to_owned()
                } else if self.binding.driver == CursorOrigin && row[id].is_string() {
                    row[id]
                        .as_str()
                        .and_then(|s| s.parse::<u64>().ok())
                        .ok_or_else(malformed)?
                        .to_string()
                } else {
                    row[id].as_u64().ok_or_else(malformed)?.to_string()
                };
                Ok(Item {
                    key,
                    title: row["title"].as_str().ok_or_else(malformed)?.into(),
                    state: row
                        .pointer(state)
                        .and_then(Value::as_str)
                        .ok_or_else(malformed)?
                        .into(),
                })
            })
            .collect()
    }

    /// Read GitHub check runs for a commit in this repository.
    pub async fn checks(&self, sha: &str) -> Result<Vec<Value>> {
        if self.binding.driver != Driver::GitHub {
            return Err(Error::Unsupported("check runs require GitHub".into()));
        }
        if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid("expected full commit SHA".into()));
        }
        let path = format!(
            "/repos/{}/commits/{sha}/check-runs",
            self.binding.repository
        );
        self.request(Method::GET, &path, None).await?["check_runs"]
            .as_array()
            .cloned()
            .ok_or_else(malformed)
    }
    /// Post a conversation comment to a GitHub, GitLab, Bitbucket or Forgejo pull request.
    /// A transport error can leave delivery uncertain.
    pub async fn comment(&self, number: u64, body: &str) -> Result<u64> {
        use Driver::*;
        let repo = &self.binding.repository;
        let (path, payload) = match self.binding.driver {
            GitHub => (
                format!("/repos/{repo}/issues/{number}/comments"),
                json!({"body":body}),
            ),
            Forgejo => (
                format!("/api/v1/repos/{repo}/issues/{number}/comments"),
                json!({"body":body}),
            ),
            GitLab => (
                format!(
                    "/api/v4/projects/{}/merge_requests/{number}/notes",
                    repo.replace('/', "%2F")
                ),
                json!({"body":body}),
            ),
            Bitbucket => (
                format!("/2.0/repositories/{repo}/pullrequests/{number}/comments"),
                json!({"content":{"raw":body}}),
            ),
            _ => {
                return Err(Error::Unsupported(
                    "pull request comments unavailable for this service".into(),
                ));
            }
        };
        if number == 0 || body.trim().is_empty() {
            return Err(Error::Invalid("expected an item number and comment".into()));
        }
        self.request(Method::POST, &path, Some(payload)).await?["id"]
            .as_u64()
            .ok_or_else(malformed)
    }
    async fn linear_query(&self, query: &str, variables: Value) -> Result<Value> {
        let response = self
            .request(
                Method::POST,
                "/graphql",
                Some(json!({ "query":query, "variables":variables })),
            )
            .await?;
        if response.get("errors").is_some() {
            return Err(Error::Integration("Linear rejected the request".into()));
        }
        Ok(response)
    }

    /// Resolve a Linear issue, check its current team, and post one comment.
    /// A transport error during the mutation can leave delivery uncertain.
    pub async fn comment_on_issue(&self, identifier: &str, body: &str) -> Result<()> {
        if self.binding.driver != Driver::Linear {
            return Err(Error::Unsupported("issue comments require Linear".into()));
        }
        if identifier.trim().is_empty() || body.trim().is_empty() {
            return Err(Error::Invalid(
                "expected an issue identifier and comment".into(),
            ));
        }
        let response = self
            .linear_query(
                "query CommentTarget($id: String!) { issue(id: $id) { id team { key } } }",
                json!({"id":identifier}),
            )
            .await?;
        let issue = response.pointer("/data/issue").ok_or_else(malformed)?;
        if issue.is_null() {
            return Err(Error::NotFound(identifier.into()));
        }
        let team = issue
            .pointer("/team/key")
            .and_then(Value::as_str)
            .ok_or_else(malformed)?;
        if !team.eq_ignore_ascii_case(&self.binding.repository) {
            return Err(Error::Forbidden);
        }
        let id = issue["id"].as_str().ok_or_else(malformed)?;
        let response = self.linear_query(
            "mutation CommentOnIssue($input: CommentCreateInput!) { commentCreate(input: $input) { success } }",
            json!({"input":{"issueId":id,"body":body}}),
        ).await?;
        match response
            .pointer("/data/commentCreate/success")
            .and_then(Value::as_bool)
        {
            Some(true) => Ok(()),
            Some(false) => Err(Error::Integration("Linear declined the comment".into())),
            None => Err(malformed()),
        }
    }
}
