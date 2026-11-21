use crate::{NativeChild, NativeChildStatus, NativeThreadId, NativeTurnId, RunId};
use serde_json::Value;
use std::collections::{HashMap, hash_map::Entry};

/// One app-server process and one owning Huggler run. Accept a child only
/// when its spawn is reported on the root turn or on a known descendant turn.
#[derive(Default)]
pub(super) struct Children(HashMap<String, NativeChild>);

impl Children {
    pub fn active(&self) -> bool {
        self.0.values().any(|c| {
            matches!(
                c.status,
                NativeChildStatus::Starting | NativeChildStatus::Running
            )
        })
    }

    pub fn running(&self, thread: &str, turn: &str) -> bool {
        self.0.get(thread).is_some_and(|c| {
            c.status == NativeChildStatus::Running
                && c.turn_id.as_ref().is_some_and(|t| t.0 == turn)
        })
    }

    pub fn observe(
        &mut self,
        root: &str,
        root_turn: &str,
        owner: &RunId,
        frame: &Value,
    ) -> Option<NativeChild> {
        let p = &frame["params"];
        let thread = p["threadId"].as_str()?;
        let method = frame["method"].as_str()?;
        let item = &p["item"];
        if matches!(method, "item/started" | "item/completed") && item["type"] == "subAgentActivity"
        {
            let parent_turn = p["turnId"].as_str()?;
            let target = item["agentThreadId"].as_str()?;
            if target == root || target == thread {
                return None;
            }
            if item["kind"] == "started" {
                let from_root = thread == root && parent_turn == root_turn;
                if !(from_root || self.running(thread, parent_turn)) {
                    return None;
                }
                let Entry::Vacant(entry) = self.0.entry(target.to_owned()) else {
                    return None;
                };
                let child = NativeChild {
                    run_id: owner.clone(),
                    parent_thread_id: NativeThreadId(thread.into()),
                    parent_turn_id: Some(NativeTurnId(parent_turn.into())),
                    can_interrupt: false,
                    thread_id: NativeThreadId(target.into()),
                    turn_id: None,
                    path: item["agentPath"].as_str()?.into(),
                    status: NativeChildStatus::Starting,
                    result: None,
                };
                entry.insert(child.clone());
                return Some(child);
            }
            // When Codex streams the child's own turn, its turn/completed frame
            // carries the outcome. Otherwise the parent activity is all we have.
            // Such a notification can arrive after the parent's turn has ended.
            let child = self.0.get_mut(target)?;
            if child.parent_thread_id.0 != thread
                || child.parent_turn_id.as_ref()?.0 != parent_turn
                || child.status != NativeChildStatus::Starting
            {
                return None;
            }
            child.status = match item["kind"].as_str()? {
                "completed" => NativeChildStatus::Completed,
                "interrupted" => NativeChildStatus::Interrupted,
                _ => return None,
            };
            return Some(child.clone());
        }
        let child = self.0.get_mut(thread)?;
        match method {
            "turn/started" => {
                let turn = NativeTurnId(p["turn"]["id"].as_str()?.into());
                if child.turn_id.as_ref() == Some(&turn) {
                    return None;
                }
                child.turn_id = Some(turn);
                child.result = None;
                child.status = NativeChildStatus::Running;
                child.can_interrupt = true;
            }
            "item/completed" if item["type"] == "agentMessage" && item["phase"] != "commentary" => {
                let text = item["text"].as_str()?;
                if child.turn_id.as_ref()?.0 != p["turnId"].as_str()?
                    || text.is_empty()
                    || child.status != NativeChildStatus::Running
                    || child.result.as_deref() == Some(text)
                {
                    return None;
                }
                child.result = Some(text.into());
            }
            "turn/completed" => {
                if child.turn_id.as_ref()?.0 != p["turn"]["id"].as_str()?
                    || child.status != NativeChildStatus::Running
                {
                    return None;
                }
                child.status = match p["turn"]["status"].as_str()? {
                    "completed" => NativeChildStatus::Completed,
                    "interrupted" => NativeChildStatus::Interrupted,
                    "failed" => NativeChildStatus::Failed,
                    _ => return None,
                };
                child.can_interrupt = false;
            }
            _ => return None,
        }
        Some(child.clone())
    }
}
