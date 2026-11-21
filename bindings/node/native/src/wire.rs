// The SDK owns these field names. Keep opaque provider JSON (including question
// extensions and tool input) intact and format SQLite integers before V8 sees them.
use huggler::*;
use serde_json::{Value, json};

pub fn run(r: Run) -> Value {
    json!({
        "id": r.id, "ordinal": r.ordinal.to_string(), "status": r.status, "startedAtMs": r.started_at_ms,
        "queueHeld": r.queue_held, "deliverySlot": r.delivery_slot.map(|n| n.to_string()),
        "nativeTurn": r.native_turn, "usageLimitResetAt": r.usage_limit_reset_at.map(|n| n.to_string()),
        "promptSuggestion": r.prompt_suggestion,
        // Existing Cargo hosts may have persisted raw provider diagnostics.
        "error": r.error.as_deref().map(run_diagnostic),
    })
}

pub fn message(m: Message) -> Value {
    json!({
        "id": m.id, "runId": m.run_id, "role": m.role, "text": m.text,
        "origin": m.origin.map(|o| json!({"environmentId": o.environment_id,
            "threadId": o.thread_id, "runId": o.run_id, "requestKey": o.request_key})),
    })
}

fn question(q: UserInputRequest) -> Value {
    json!({"id": q.id, "runId": q.run_id, "questions": q.questions})
}

fn permission(p: PermissionRequest) -> Value {
    json!({"id": p.id, "runId": p.run_id, "toolCall": p.tool_call, "options": p.options,
        "nativeChild": p.native_child.map(|c| json!({"threadId": c.thread_id, "turnId": c.turn_id}))})
}

fn child(c: NativeChild) -> Value {
    json!({"runId": c.run_id, "threadId": c.thread_id, "turnId": c.turn_id, "parentThreadId": c.parent_thread_id,
        "parentTurnId": c.parent_turn_id, "path": c.path, "canInterrupt": c.can_interrupt, "status": c.status,
        "result": result_text(c.status == NativeChildStatus::Failed, c.result.as_deref())})
}

fn task(t: DelegatedTask) -> Value {
    json!({"taskId": t.task_id, "parentThreadId": t.parent_thread_id, "parentRunId": t.parent_run_id,
        "childThreadId": t.child_thread_id, "childRunId": t.child_run_id, "resultRunId": t.result_run_id,
        "delivery": t.delivery, "wakeRunId": t.wake_run_id, "status": t.status,
        "result": result_text(t.status == RunStatus::Failed, t.result.as_deref())})
}

fn result_text(failed: bool, text: Option<&str>) -> Option<&str> {
    if failed {
        text.map(run_diagnostic)
    } else {
        text
    }
}

pub fn thread(t: Thread) -> Value {
    json!({
        "id": t.id, "projectId": t.project, "name": t.name, "settled": t.settled, "model": t.model, "provider": t.provider,
        "providerInstanceId": t.provider_instance, "worktreePath": t.worktree_path, "nativeThread": t.native_thread,
        "messages": t.messages.into_iter().map(message).collect::<Vec<_>>(),
        "runs": t.runs.into_iter().map(run).collect::<Vec<_>>(), "cursor": t.cursor.to_string(),
        "questions": t.questions.into_iter().map(question).collect::<Vec<_>>(),
        "permissions": t.permissions.into_iter().map(permission).collect::<Vec<_>>(),
        "nativeChildren": t.native_children.into_iter().map(child).collect::<Vec<_>>(),
        "tasks": t.tasks.into_iter().map(task).collect::<Vec<_>>(),
    })
}

pub fn receipt(r: Receipt) -> Value {
    json!({"commandId": r.command_id, "threadId": r.thread_id, "runId": r.run_id, "messageId": r.message_id,
        "eventSequences": r.event_sequences.iter().map(ToString::to_string).collect::<Vec<_>>()})
}

pub fn detail(d: ConversationDetail) -> Result<Value> {
    // Only Rust decodes this token. Its embedded i64 never passes through a JS number.
    let older = d
        .history
        .older
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let messages: Vec<_> = d
        .history
        .messages
        .into_iter()
        .map(|i| {
            json!({"message": message(i.message), "truncated": i.truncated,
            "timestampMs": i.timestamp_ms, "turnDurationMs": i.turn_duration_ms})
        })
        .collect();
    Ok(json!({
        "id": d.id, "name": d.name, "settled": d.settled, "providerInstanceId": d.provider_instance, "model": d.model,
        "latestRun": d.latest_run.map(run), "foregroundRun": d.foreground_run.map(run),
        "queuedCount": d.queued_count.to_string(), "queueHeld": d.queue_held,
        "streamingText": d.streaming_text, "commandItems": d.command_items,
        "activity": d.activity.into_iter().map(|a| json!({
            "runId": a.run_id, "status": a.status, "truncated": a.truncated,
            "items": a.items.into_iter().map(|item| json!({
                "id": item.id, "kind": item.kind, "title": item.title, "status": item.status,
                "timestampMs": item.timestamp_ms
            })).collect::<Vec<_>>(),
            "turnDurationMs": a.turn_duration_ms, "counts": a.counts
        })).collect::<Vec<_>>(),
        "questions": d.questions.into_iter().map(question).collect::<Vec<_>>(),
        "permissions": d.permissions.into_iter().map(permission).collect::<Vec<_>>(),
        "nativeChildren": d.native_children.into_iter().map(child).collect::<Vec<_>>(),
        "history": {"sequence": d.history.sequence.to_string(), "older": older, "messages": messages}
    }))
}

pub fn event(e: Event) -> Value {
    use Change::*;
    let change = match e.change {
        ActivityChanged { run_id } => json!({"type": "activityChanged", "runId": run_id}),
        Launched => json!({"type": "launched"}),
        SessionReset => json!({"type": "sessionReset"}),
        Run { run: r } => json!({"type": "run", "run": run(r)}),
        Message { message: m } => json!({"type": "message", "message": message(m)}),
        Bound { native_thread } => json!({"type": "bound", "nativeThread": native_thread}),
        SettlementChanged { settled } => json!({"type":"settlementChanged","settled":settled}),
        ModelSelected { model } => json!({"type": "modelSelected", "model": model}),
        QuestionRequested { request } => {
            json!({"type": "questionRequested", "request": question(request)})
        }
        PermissionRequested { request } => {
            json!({"type": "permissionRequested", "request": permission(request)})
        }
        QuestionClosed {
            id,
            answered,
            responder_run,
        } => {
            json!({"type": "questionClosed", "id": id, "answered": answered, "responderRun": responder_run})
        }
        PermissionClosed { id, option } => {
            json!({"type": "permissionClosed", "id": id, "option": option})
        }
        NativeChild { child: c } => json!({"type": "nativeChild", "child": child(c)}),
        Task { task: t } => json!({"type": "task", "task": task(t)}),
        CommandOutputAvailable { run_id, item } => {
            json!({"type": "commandOutputAvailable", "runId": run_id, "item": item})
        }
        UsageLimitContinuation { failed, run_id } => {
            json!({"type": "usageLimitContinuation", "failed": failed, "runId": run_id})
        }
        Steer {
            run_id,
            command_id,
            outcome,
        } => {
            let outcome = match outcome {
                SteerOutcome::Rejected(_) => {
                    json!({"rejected": "The provider rejected the steer request"})
                }
                other => json!(other),
            };
            json!({"type": "steer", "runId": run_id, "commandId": command_id, "outcome": outcome})
        }
        RunStopRequested { run_id } => json!({"type": "runStopRequested", "runId": run_id}),
        OutputDelta { run_id, text } => {
            json!({"type": "outputDelta", "runId": run_id, "text": text})
        }
        PeerCreated {
            caller_run_id,
            peer_thread_id,
        } => {
            json!({"type": "peerCreated", "callerRunId": caller_run_id, "peerThreadId": peer_thread_id})
        }
        RunInterruptRequested {
            run_id,
            caller_thread_id,
            caller_run_id,
            reason,
        } => {
            json!({"type": "runInterruptRequested", "runId": run_id, "callerThreadId": caller_thread_id, "callerRunId": caller_run_id, "reason": reason})
        }
        UserNotice { run_id, text } => json!({"type": "userNotice", "runId": run_id, "text": text}),
    };
    json!({"threadId": e.thread_id, "sequence": e.sequence.to_string(), "change": change})
}

// Stored run failures can contain provider replies, command lines, URLs or paths.
// Keep actionable, known categories and keep free-form diagnostics within the host.
fn run_diagnostic(message: &str) -> &'static str {
    if message.starts_with("sign in with the provider CLI") {
        "Sign in with the provider CLI for this instance"
    } else if message.starts_with("install the provider executable") {
        "Install the provider executable or check its configured path"
    } else if message.starts_with("provider usage limit exceeded") {
        "Provider usage limit exceeded"
    } else {
        "The run failed. Check the provider and host configuration"
    }
}

pub fn error(e: Error) -> napi::Error {
    use Error::*;
    // Free-form adapter and system messages are unsuitable for caller logs.
    // Preserve identifiers supplied by the caller and structured retry metadata.
    let plain = |message: &str| (message.to_owned(), json!(message));
    let (message, detail) = match &e {
        Conflict {
            command,
            original_thread,
            requested_thread,
        } => (
            e.to_string(),
            json!({"command": command, "originalThread": original_thread, "requestedThread": requested_thread}),
        ),
        SchemaVersion(version) => (e.to_string(), json!(version.to_string())),
        UsageLimit { reset_at, .. } => (
            "Provider usage limit exceeded".into(),
            json!({"message": "Provider usage limit exceeded", "resetAt": reset_at.map(|n| n.to_string())}),
        ),
        LoginRequired | ServerAccessDenied | AlreadyAnswered | AlreadyOpen | Forbidden
        | Interrupted | Closed | CursorExpired | HistoryChanged | Busy => {
            (e.to_string(), Value::Null)
        }
        Provider(_) => plain("The provider could not complete the request"),
        CheckoutUnavailable(_) => plain("Checkout directory is unavailable"),
        IncompatibleProvider(_) => plain("Check the provider version and protocol"),
        MissingExecutable(_) => {
            plain("Install the provider executable or check its configured path")
        }
        Storage(_) => plain("Database operation failed"),
        Data(_) => plain("Invalid stored data"),
        Io(err) => plain(&format!("I/O error: {}", err.kind())),
        Worker(_) => plain("Runtime worker failed"),
        Integration(_) => plain("Host integration failed"),
        Invalid(_) => plain("Check the supplied arguments and configuration"),
        Unsupported(_) => plain("Unsupported operation"),
        NotFound(_) => plain("The requested resource was not found"),
    };
    napi::Error::from_reason(
        json!({"code": json!(e)["code"], "message": message, "detail": detail}).to_string(),
    )
}

pub fn parse<T: serde::de::DeserializeOwned>(input: &str) -> Result<T> {
    serde_json::from_str(input).map_err(|e| Error::Invalid(e.to_string()))
}
