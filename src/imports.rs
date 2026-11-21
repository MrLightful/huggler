//! Decode visible text from a host-selected snapshot. Imported content never supplies executable settings.
use crate::{Error, ImportProvenance, ImportSource, NativeThreadId, Result, Role};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs::File, io::Read};

pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
pub(crate) type Messages = Vec<(Role, String)>;
pub(crate) struct Snapshot {
    pub provenance: ImportProvenance,
    pub messages: Messages,
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn text_blocks(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| {
            matches!(
                v["type"].as_str(),
                Some("text" | "input_text" | "output_text")
            )
        })
        .filter_map(|v| v["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn visible(role: &Value, content: &Value) -> Option<(Role, String)> {
    let role = match role.as_str()? {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => return None,
    };
    let text = text_blocks(content);
    (!text.trim().is_empty()).then_some((role, text))
}

fn pi_branch(records: &[Value]) -> Result<Vec<&Value>> {
    let entries: Vec<_> = records.iter().filter(|v| v["type"] != "session").collect();
    let Some(mut node) = entries.last().copied() else {
        return Ok(entries);
    };
    if entries.iter().all(|v| v.get("id").is_none()) {
        return Ok(entries);
    }
    let mut by_id = HashMap::new();
    for entry in &entries {
        let id = entry["id"]
            .as_str()
            .ok_or_else(|| Error::Invalid("missing Pi entry ID".into()))?;
        if by_id.insert(id, *entry).is_some() {
            return Err(Error::Invalid("duplicate Pi entry ID".into()));
        }
    }
    let mut branch = vec![];
    loop {
        by_id.remove(node["id"].as_str().unwrap());
        branch.push(node);
        if node["parentId"].is_null() {
            break;
        }
        node = node["parentId"]
            .as_str()
            .and_then(|id| by_id.get(id))
            .copied()
            .ok_or_else(|| Error::Invalid("incomplete or cyclic Pi branch".into()))?;
    }
    branch.reverse();
    Ok(branch)
}

fn codex(records: &[Value]) -> Result<Messages> {
    let mut messages = vec![];
    // A rollout may store the same user turn as a semantic event and a response item.
    // Match only within one exchange so identical later prompts remain visible.
    let mut pending = Vec::<(bool, String)>::new();
    let flush = |pending: &mut Vec<(bool, String)>, messages: &mut Messages| {
        let events: std::collections::HashSet<_> = pending
            .iter()
            .filter(|(event, _)| *event)
            .map(|(_, text)| text.trim().to_owned())
            .collect();
        messages.extend(
            pending
                .drain(..)
                .filter(|(event, text)| *event || !events.contains(text.trim()))
                .map(|(_, text)| (Role::User, text)),
        );
    };
    for record in records {
        let payload = &record["payload"];
        if record["type"] == "session_meta" && !payload["history_base"].is_null() {
            return Err(Error::Unsupported(
                "Codex rollouts with an external history prefix".into(),
            ));
        }
        if record["type"] == "event_msg" && payload["type"] == "user_message" {
            if let Some(text) = payload["message"].as_str().filter(|s| !s.trim().is_empty()) {
                pending.push((true, text.into()));
            }
        } else if record["type"] == "response_item"
            && payload["type"] == "message"
            && let Some((role, text)) = visible(&payload["role"], &payload["content"])
        {
            if role == Role::User {
                pending.push((false, text));
            } else {
                flush(&mut pending, &mut messages);
                messages.push((role, text));
            }
        }
    }
    flush(&mut pending, &mut messages);
    Ok(messages)
}

pub(crate) fn read_file(source: ImportSource) -> Result<Snapshot> {
    let path = match &source {
        ImportSource::Claude(p) | ImportSource::Codex(p) | ImportSource::Pi(p) => {
            p.canonicalize()?
        }
        _ => return Err(Error::Invalid("expected a transcript file".into())),
    };
    let mut open = File::options();
    open.read(true);
    // A FIFO selected directly or swapped in after canonicalization must not block intake.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NONBLOCK);
    }
    let file = open.open(&path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::Invalid("expected a regular transcript file".into()));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::Invalid("transcript exceeds 8 MiB".into()));
    }
    let mut rows = vec![];
    for line in bytes
        .split(|c| *c == b'\n')
        .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
    {
        if rows.len() == 32768 {
            return Err(Error::Invalid("too many transcript records".into()));
        }
        rows.push(serde_json::from_slice::<Value>(line)?);
    }
    let (source, session, messages) = match source {
        ImportSource::Claude(_) => {
            let rows = rows.iter().filter(|v| {
                !["isSidechain", "isMeta", "isCompactSummary"]
                    .iter()
                    .any(|k| v[k] == true)
            });
            let session = rows.clone().find_map(|v| v["sessionId"].as_str());
            let messages = rows
                .filter_map(|v| visible(&v["type"], &v["message"]["content"]))
                .collect();
            (ImportSource::Claude(path), session, messages)
        }
        ImportSource::Codex(_) => {
            let session = rows
                .iter()
                .find(|v| v["type"] == "session_meta")
                .and_then(|v| {
                    v["payload"]["id"]
                        .as_str()
                        .or_else(|| v["payload"]["session_id"].as_str())
                });
            (ImportSource::Codex(path), session, codex(&rows)?)
        }
        ImportSource::Pi(_) => {
            let session = rows
                .iter()
                .find(|v| v["type"] == "session")
                .and_then(|v| v["id"].as_str());
            let messages = pi_branch(&rows)?
                .into_iter()
                .filter(|v| v["type"] == "message")
                .filter_map(|v| visible(&v["message"]["role"], &v["message"]["content"]))
                .collect();
            (ImportSource::Pi(path), session, messages)
        }
        _ => unreachable!(),
    };
    Ok(Snapshot {
        provenance: ImportProvenance {
            source,
            native_session: session.map(|id| NativeThreadId(id.into())),
            digest: digest(&bytes),
        },
        messages,
    })
}
