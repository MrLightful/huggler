use super::{Outstanding, Server, http_error};
use crate::{
    Error, NativeTurnId, Result,
    providers::{Update, rpc::protocol},
};
use reqwest::{Method, Response};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

/// SSE frames can span TCP chunks. Retain incomplete UTF-8 and support CRLF and multiline data.
struct Events {
    response: Response,
    pending: Vec<u8>,
    offset: usize,
}

impl Events {
    async fn next(&mut self) -> Result<Value> {
        let mut data = Vec::new();
        loop {
            if let Some(end) = self.pending[self.offset..].iter().position(|b| *b == b'\n') {
                let line = &self.pending[self.offset..self.offset + end];
                self.offset += end + 1;
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                if line.is_empty() && !data.is_empty() {
                    return Ok(serde_json::from_slice(&data)?);
                }
                if let Some(value) = line.strip_prefix(b"data:") {
                    data.extend_from_slice(value.strip_prefix(b" ").unwrap_or(value));
                    data.push(b'\n');
                }
            } else {
                self.pending.drain(..self.offset);
                self.offset = 0;
                let bytes = self
                    .response
                    .chunk()
                    .await
                    .map_err(http_error)?
                    .ok_or_else(|| {
                        protocol("OpenCode event stream disconnected before settlement")
                    })?;
                self.pending.extend_from_slice(&bytes);
            }
            if self.pending.len() - self.offset + data.len() > 8 * 1024 * 1024 {
                return Err(protocol("OpenCode event exceeds 8 MiB"));
            }
        }
    }
}

pub(super) async fn prompt(
    server: &Server,
    path: &str,
    id: &str,
    text: &str,
    outstanding: &mut Outstanding,
    update: impl Fn(Update) -> Result<()>,
) -> Result<()> {
    let response = server.checked(Method::GET, "/api/event", None).await?;
    let mut events = Events {
        response,
        pending: vec![],
        offset: 0,
    };
    let inbox = format!("msg_{}", uuid::Uuid::new_v4().simple());
    *outstanding = Outstanding::Queued(inbox.clone());
    let reply = server
        .submit_prompt(
            &format!("{path}/prompt"),
            &json!({"id":inbox,"text":text,"delivery":"queue"}),
            outstanding,
        )
        .await?;
    let user = &reply["data"];
    if user["id"] != inbox || user["sessionID"] != id || user["type"] != "user" {
        return Err(protocol(
            "OpenCode prompt admission did not identify the submitted message and session",
        ));
    }
    update(Update::Started(Some(NativeTurnId(inbox.clone()))))?;
    let mut delivered = false;
    let mut message = None;
    let mut text_parts = BTreeMap::new();
    let mut seen = HashSet::new();
    let mut totals = (0i64, 0i64);
    loop {
        let e = events.next().await?;
        let d = &e["data"];
        if delivered && e["type"] == "form.created" && d["form"]["sessionID"] == id {
            return Err(protocol("OpenCode requested interactive form input"));
        }
        if d["sessionID"] != id {
            continue;
        }
        if let Some(event_id) = e["id"].as_str()
            && !seen.insert(event_id.to_owned())
        {
            continue;
        }
        let own_message =
            message.is_some() && d["assistantMessageID"].as_str() == message.as_deref();
        match e["type"].as_str() {
            Some("session.inbox.delivered") if d["inboxID"] == inbox => {
                delivered = true;
                *outstanding = Outstanding::Active;
            }
            Some("session.inbox.cancelled") if d["inboxID"] == inbox => {
                *outstanding = Outstanding::None;
                return Err(Error::Interrupted);
            }
            Some("session.inbox.delivered") if delivered => {
                *outstanding = Outstanding::None;
                return Err(protocol("another prompt joined this OpenCode execution"));
            }
            _ if !delivered => continue,
            Some("session.step.started") => {
                message = d["assistantMessageID"].as_str().map(str::to_owned);
                text_parts.clear();
            }
            Some("session.text.delta") if own_message => {
                if let Some(delta) = d["delta"].as_str() {
                    update(Update::Output(delta.into()))?;
                }
            }
            Some("session.text.ended") if own_message => {
                if let (Some(ordinal), Some(text)) = (d["ordinal"].as_u64(), d["text"].as_str()) {
                    text_parts.insert(ordinal, text.to_owned());
                }
            }
            Some("session.step.ended") if own_message => {
                let tokens = &d["tokens"];
                if let (Some(i), Some(o)) = (tokens["input"].as_i64(), tokens["output"].as_i64())
                    && i >= 0
                    && o >= 0
                {
                    totals.0 = totals.0.saturating_add(i);
                    totals.1 = totals.1.saturating_add(o);
                    update(Update::Usage {
                        input: totals.0,
                        output: totals.1,
                    })?;
                }
            }
            Some("session.execution.succeeded") => {
                *outstanding = Outstanding::None;
                let answer: String = text_parts.into_values().collect();
                if answer.is_empty() {
                    return Err(protocol("OpenCode ended without assistant text"));
                }
                return update(Update::Finalizing(answer));
            }
            Some("session.execution.interrupted" | "session.execution.cancelled") => {
                *outstanding = Outstanding::None;
                return Err(Error::Interrupted);
            }
            Some("session.execution.failed") => {
                *outstanding = Outstanding::None;
                return Err(protocol("OpenCode execution failed"));
            }
            Some("permission.asked") => {
                return Err(protocol(
                    "OpenCode execution failed or requires interactive input",
                ));
            }
            _ => {}
        }
    }
}
