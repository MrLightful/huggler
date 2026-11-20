use crate::{NativeChild, NativeChildStatus as Status, NativeThreadId, RunId};
use serde_json::Value;
use std::collections::HashMap;

struct Child {
    view: NativeChild,
    // Keep the advertised capability across idle periods. An existing session
    // can resume with a state-only notification.
    cancel: bool,
    observed: bool,
    message_id: Option<String>,
}

pub(super) struct Children<'a> {
    owner: &'a RunId,
    sessions: HashMap<String, Child>,
}

pub(super) fn text_chunk(update: &Value) -> Option<&str> {
    (update["sessionUpdate"] == "agent_message_chunk" && update["content"]["type"] == "text")
        .then(|| update["content"]["text"].as_str())?
}

impl<'a> Children<'a> {
    pub fn new(owner: &'a RunId) -> Self {
        Self {
            owner,
            sessions: HashMap::new(),
        }
    }

    pub fn running(&self, session: &str) -> bool {
        self.sessions
            .get(session)
            .is_some_and(|child| child.view.status.working())
    }

    pub fn active(&self) -> bool {
        self.sessions
            .values()
            .any(|child| child.view.status.working())
    }

    pub fn can_interrupt(&self, session: &str) -> bool {
        self.sessions
            .get(session)
            .is_some_and(|child| child.view.can_interrupt)
    }

    /// A load replays complete transcripts, followed by a current-state update
    /// even for long-idle children. Remember only lineage until work resumes.
    pub fn announcement(
        &mut self,
        root: &str,
        params: &Value,
        history: bool,
    ) -> Option<NativeChild> {
        let parent = params["sessionId"].as_str()?;
        let update = &params["update"];
        if update["sessionUpdate"] != "subagent_update" {
            return None;
        }
        let id = update["sessionId"].as_str()?;
        if id.is_empty()
            || id == root
            || id == parent
            || (parent != root && !self.sessions.contains_key(parent))
        {
            return None;
        }
        let child = self.sessions.entry(id.to_owned()).or_insert_with(|| Child {
            view: NativeChild {
                run_id: self.owner.clone(),
                parent_thread_id: NativeThreadId(parent.into()),
                parent_turn_id: None,
                thread_id: NativeThreadId(id.into()),
                turn_id: None,
                path: id.into(),
                can_interrupt: false,
                status: Status::Unknown,
                result: None,
            },
            cancel: false,
            observed: !history,
            message_id: None,
        });
        if child.view.parent_thread_id.0 != parent {
            return None;
        }
        if let Some(title) = update.get("title") {
            child.view.path = title.as_str().unwrap_or(id).to_owned();
        }
        if history {
            return None;
        }
        let state = &update["state"];
        let status = match state["state"].as_str() {
            Some("running") => Status::Running,
            Some("requires_action") => Status::RequiresAction,
            Some("idle") => match state["stopReason"].as_str() {
                Some("end_turn") => Status::Completed,
                Some("cancelled") => Status::Interrupted,
                Some("error") => Status::Failed,
                _ => Status::Idle,
            },
            Some(_) => Status::Unknown,
            None if update.get("state").is_some() => Status::Unknown,
            None => child.view.status,
        };
        if let Some(capabilities) = update.get("capabilities") {
            child.cancel = capabilities["cancel"].is_object();
        }
        if !child.observed && !status.working() {
            return None;
        }
        if status.working() && !child.view.status.working() {
            child.view.result = None;
            child.message_id = None;
        }
        child.observed = true;
        child.view.status = status;
        child.view.can_interrupt = child.cancel && status.working();
        if status == Status::Failed
            && let Some(error) = state["error"]["message"].as_str()
        {
            child.view.result = Some(error.into());
        }
        Some(child.view.clone())
    }

    pub fn message(&mut self, session: &str, update: &Value) -> Option<NativeChild> {
        if let Some(text) = text_chunk(update) {
            let child = self.sessions.get_mut(session)?;
            if child.view.status.working() {
                let id = update["messageId"].as_str();
                if id != child.message_id.as_deref() {
                    child.message_id = id.map(str::to_owned);
                    child.view.result = None;
                }
                // Publish the assembled message on the next state update.
                child
                    .view
                    .result
                    .get_or_insert_with(String::new)
                    .push_str(text);
            }
        } else if update["sessionUpdate"] == "session_message" {
            let sender = update["senderSessionId"].as_str()?;
            let child = self.sessions.get_mut(sender)?;
            // Read the parent's incoming copy of a child's handback. A launch
            // prompt and a mirrored outgoing message do not contribute text.
            if !child.observed
                || session != child.view.parent_thread_id.0
                || update["recipientSessionId"] != session
            {
                return None;
            }
            let report: String = update["content"]
                .as_array()?
                .iter()
                .filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str())
                .collect();
            if !report.is_empty() && child.view.result.as_deref() != Some(&report) {
                child.view.result = Some(report);
                return Some(child.view.clone());
            }
        }
        None
    }
}
