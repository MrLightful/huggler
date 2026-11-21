//! OpenCode sessions on a dedicated managed process or a configured loopback server.
mod events;
use super::{
    Turn, Update,
    probe::ProviderReport,
    rpc::{Rpc, protocol},
};
use crate::{Error, NativeThreadId, ProviderInstance, Result};
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use tokio::{sync::watch, time::timeout};

/// The v2 API has separate controls for admitted inbox items and active executions.
enum Outstanding {
    None,
    Queued(String),
    Active,
}

struct Server {
    origin: String,
    password: Option<String>,
    directory: String,
    client: Client,
    version: String,
    v2: bool,
    managed: Option<Rpc>,
}

fn http_error(_: reqwest::Error) -> Error {
    protocol("OpenCode HTTP exchange failed; check the server address and process")
}

impl Server {
    async fn connect(instance: &ProviderInstance, cwd: &Path) -> Result<Self> {
        let (origin, password, managed) = if let Some(url) = &instance.server_url {
            let secret = instance
                .password_env
                .as_ref()
                .map(|key| {
                    std::env::var(key).map_err(|_| {
                        protocol(format!("set {key} for the OpenCode server password"))
                    })
                })
                .transpose()?;
            (url.trim_end_matches('/').to_owned(), secret, None)
        } else {
            let secret = uuid::Uuid::new_v4().to_string();
            let mut command = instance.command();
            command
                .args(["serve", "--hostname", "127.0.0.1", "--port", "0"])
                .env("OPENCODE_PASSWORD", &secret)
                .env("OPENCODE_SERVER_PASSWORD", &secret)
                .env("OPENCODE_SERVER_USERNAME", "opencode");
            let mut rpc = Rpc::open(command, cwd)?;
            let port = loop {
                let line = rpc.line().await?;
                if let Some((_, port)) = line.split_once("server listening on http://127.0.0.1:") {
                    break port
                        .trim()
                        .parse::<u16>()
                        .ok()
                        .filter(|p| *p != 0)
                        .ok_or_else(|| protocol("invalid managed OpenCode listening port"))?;
                }
            };
            rpc.drain_stdout();
            (format!("http://127.0.0.1:{port}"), Some(secret), Some(rpc))
        };
        let mut server = Self {
            origin,
            password,
            managed,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .connect_timeout(Duration::from_secs(5))
                .build()
                .map_err(http_error)?,
            directory: cwd
                .to_str()
                .ok_or_else(|| protocol("OpenCode needs a UTF-8 working directory"))?
                .to_owned(),
            version: String::new(),
            v2: false,
        };
        let response = server.request(Method::GET, "/api/info", None).await?;
        let info = if response.status().is_success() {
            response.json::<Value>().await.unwrap_or_default()
        } else {
            Value::Null
        };
        let version = if let Some(v) = info["version"].as_str() {
            server.v2 = true;
            v.to_owned()
        } else {
            let health = server.request(Method::GET, "/global/health", None).await?;
            let data = health.json::<Value>().await.unwrap_or_default();
            data["version"]
                .as_str()
                .filter(|_| data["healthy"] == true)
                .unwrap_or_default()
                .to_owned()
        };
        if version.split('.').next() != Some(if server.v2 { "2" } else { "1" }) {
            return Err(Error::IncompatibleProvider(
                "expected OpenCode v1 health or v2 server info".into(),
            ));
        }
        server.version = version;
        Ok(server)
    }

    async fn request(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Response> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.origin));
        if let Some(password) = &self.password {
            request = request.basic_auth("opencode", Some(password));
        }
        if !path.starts_with("/api/") {
            request = request.query(&[("directory", &self.directory)]);
        }
        if path == "/api/event" {
            request = request.header("Accept", "text/event-stream");
        }
        if path == "/api/model" || path.starts_with("/api/experimental/mcp/") {
            request = request.query(&[("location[directory]", &self.directory)]);
        }
        if let Some(value) = body {
            request = request.json(value);
        }
        let response = request.send().await.map_err(http_error)?;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(Error::ServerAccessDenied);
        }
        Ok(response)
    }

    async fn checked(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Response> {
        let response = self.request(method, path, body).await?;
        Self::check_status(path, response)
    }

    fn check_status(path: &str, response: Response) -> Result<Response> {
        if !response.status().is_success() {
            return Err(protocol(format!(
                "OpenCode {path} returned HTTP {}",
                response.status()
            )));
        }
        Ok(response)
    }

    /// Sending the request can start work even if the connection drops before the body arrives.
    async fn submit_prompt(
        &self,
        path: &str,
        body: &Value,
        outstanding: &mut Outstanding,
    ) -> Result<Value> {
        let response = match self.request(Method::POST, path, Some(body)).await {
            Err(Error::ServerAccessDenied) => {
                *outstanding = Outstanding::None;
                return Err(Error::ServerAccessDenied);
            }
            response => response?,
        };
        if matches!(
            response.status(),
            StatusCode::BAD_REQUEST
                | StatusCode::NOT_FOUND
                | StatusCode::CONFLICT
                | StatusCode::UNPROCESSABLE_ENTITY
        ) {
            *outstanding = Outstanding::None;
        }
        Self::check_status(path, response)?
            .json()
            .await
            .map_err(http_error)
    }

    async fn json(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        self.checked(method, path, body)
            .await?
            .json()
            .await
            .map_err(http_error)
    }

    async fn session(&self, turn: &Turn) -> Result<(String, String)> {
        let base = if self.v2 { "/api/session" } else { "/session" };
        let model = turn
            .model
            .as_ref()
            .map(|s| {
                s.split_once('/')
                    .ok_or_else(|| protocol("OpenCode model must be provider/model"))
            })
            .transpose()?;
        let model = model.map(|(provider, id)| json!({"providerID":provider,"id":id}));
        let data = if let Some(native) = &turn.resume {
            valid_id(&native.0)?;
            self.json(Method::GET, &format!("{base}/{}", native.0), None)
                .await?
        } else {
            let mut body = json!({"title":turn.name});
            if self.v2 {
                body["location"] = json!({"directory":self.directory});
                let mut rules = vec![json!({"action":"*","resource":"*","effect":"deny"})];
                if self.managed.is_some() && turn.agent_tools.is_some() {
                    rules.push(json!({"action":"huggler_*","resource":"*","effect":"allow"}));
                }
                body["permissions"] = json!(rules);
                if let Some(model) = &model {
                    body["model"] = model.clone();
                }
            } else {
                let mut rules = vec![json!({"permission":"*","pattern":"*","action":"deny"})];
                if self.managed.is_some() && turn.agent_tools.is_some() {
                    rules.push(json!({"permission":"huggler_*","pattern":"*","action":"allow"}));
                }
                body["permission"] = json!(rules);
            }
            self.json(Method::POST, base, Some(&body)).await?
        };
        let session = if self.v2 { &data["data"] } else { &data };
        let id = valid_id(session["id"].as_str().unwrap_or(""))?;
        let cwd = if self.v2 {
            &session["location"]["directory"]
        } else {
            &session["directory"]
        };
        if cwd != &self.directory || turn.resume.as_ref().is_some_and(|n| n.0 != id) {
            return Err(protocol(
                "OpenCode returned a different session or directory",
            ));
        }
        let path = format!("{base}/{id}");
        if self.v2
            && turn.resume.is_some()
            && let Some(model) = model
        {
            self.checked(
                Method::POST,
                &format!("{path}/model"),
                Some(&json!({"model":model})),
            )
            .await?;
        }
        Ok((id.into(), path))
    }
}

fn valid_id(id: &str) -> Result<&str> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(protocol("invalid OpenCode session ID"));
    }
    Ok(id)
}

pub(super) async fn probe(instance: &ProviderInstance, cwd: &Path) -> Result<ProviderReport> {
    let server = Server::connect(instance, cwd).await?;
    let mut report = ProviderReport::new(if server.v2 {
        "OpenCode v2 HTTP/SSE"
    } else {
        "OpenCode v1 HTTP"
    });
    report.resume = true;
    report.agent_tools = server.managed.is_some();
    let catalog = server
        .json(
            Method::GET,
            if server.v2 { "/api/model" } else { "/provider" },
            None,
        )
        .await?;
    if server.v2 {
        for m in catalog["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["enabled"] != false)
        {
            if let (Some(provider), Some(id)) = (m["providerID"].as_str(), m["id"].as_str()) {
                report.models.push(format!("{provider}/{id}"));
            }
        }
    } else {
        for p in catalog["all"].as_array().into_iter().flatten() {
            if let (Some(id), Some(models)) = (p["id"].as_str(), p["models"].as_object()) {
                report
                    .models
                    .extend(models.keys().map(|m| format!("{id}/{m}")));
            }
        }
    }
    report.models.sort();
    report.models.dedup();
    report.version = Some(server.version);
    Ok(report)
}

pub(crate) async fn execute(
    instance: &ProviderInstance,
    turn: Turn,
    mut stop: watch::Receiver<bool>,
    update: impl Fn(Update) -> Result<()>,
) -> Result<()> {
    let setup = async {
        let server = Server::connect(instance, &turn.cwd).await?;
        if server.managed.is_some()
            && let Some(tools) = &turn.agent_tools
        {
            // /mcp is process-wide. Only configure the process owned by this run.
            let config = json!({"type":"remote","url":tools.url,"headers":{"Authorization":format!("Bearer {}",tools.token)},"oauth":false});
            if server.v2 {
                server
                    .checked(
                        Method::PUT,
                        "/api/experimental/mcp/huggler",
                        Some(&json!({"config":config})),
                    )
                    .await?;
                // Older 2.x servers refresh their tool catalog asynchronously after connecting.
                tokio::time::sleep(Duration::from_millis(250)).await;
            } else {
                let config = json!({"name":"huggler","config":config});
                let status = server.json(Method::POST, "/mcp", Some(&config)).await?;
                if status["huggler"]["status"] != "connected" {
                    return Err(protocol(
                        "OpenCode could not connect to the Huggler MCP server",
                    ));
                }
            }
        }
        let (id, path) = server.session(&turn).await?;
        Ok((server, id, path))
    };
    let (server, id, path) = tokio::select! {biased; _=stop.wait_for(|s|*s)=>return Err(Error::Closed), r=timeout(Duration::from_secs(20),setup)=>r.map_err(|_|protocol("OpenCode startup timed out"))??};
    update(Update::Bound(NativeThreadId(id.clone())))?;
    let mut outstanding = Outstanding::None;
    let run = async {
        if server.v2 {
            return events::prompt(&server, &path, &id, &turn.text, &mut outstanding, update).await;
        }
        let mut body = json!({"parts":[{"type":"text","text":turn.text}]});
        if let Some((p, m)) = turn.model.as_deref().and_then(|s| s.split_once('/')) {
            body["model"] = json!({"providerID":p,"modelID":m});
        }
        update(Update::Started(None))?;
        // V1 holds this request open until the session's prompt settles.
        outstanding = Outstanding::Active;
        let reply = server
            .submit_prompt(&format!("{path}/message"), &body, &mut outstanding)
            .await?;
        outstanding = Outstanding::None;
        let info = &reply["info"];
        if info["role"] != "assistant" || info["sessionID"] != id || !info["error"].is_null() {
            return Err(protocol(
                "OpenCode returned no successful assistant response for the session",
            ));
        }
        let answer = reply["parts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["type"] == "text" && p["messageID"] == info["id"] && p["sessionID"] == id)
            .filter_map(|p| p["text"].as_str())
            .collect::<String>();
        if answer.is_empty() {
            return Err(protocol("OpenCode ended without assistant text"));
        }
        let tokens = &info["tokens"];
        if let (Some(input), Some(output)) = (tokens["input"].as_i64(), tokens["output"].as_i64())
            && input >= 0
            && output >= 0
        {
            update(Update::Usage { input, output })?;
        }
        update(Update::Finalizing(answer))
    };
    let result = tokio::select! {
        biased;
        _=stop.wait_for(|s|*s)=>Err(Error::Interrupted),
        result=run=>result,
    };
    if result.is_ok() {
        return result;
    }
    let (method, endpoint) = match outstanding {
        Outstanding::None => return result,
        Outstanding::Queued(inbox) => (Method::DELETE, format!("{path}/inbox/{inbox}")),
        Outstanding::Active if server.v2 => (Method::POST, format!("{path}/interrupt")),
        Outstanding::Active => (Method::POST, format!("{path}/abort")),
    };
    let _ = timeout(
        Duration::from_secs(2),
        server.checked(method, &endpoint, None),
    )
    .await;
    result
}
