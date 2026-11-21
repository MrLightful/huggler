#![cfg(unix)]
use huggler::*;
#[path = "support/agents.rs"]
mod agents;
use agents::*;
#[path = "support/reviewer.rs"]
mod reviewer;
use reviewer::{Reviewer, allow_once, requested_huggler_tool};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use tokio::time::timeout;

async fn approve_peer_tools(
    rt: &Huggler,
    thread: &ThreadId,
    provider: Provider,
    mut events: Subscription,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> usize {
    let mut sends = 0;
    loop {
        let event = tokio::select! {
            _ = stop.wait_for(|v| *v) => return sends,
            event = events.next() => event.unwrap(),
        };
        if let Change::PermissionRequested { request } = event.change {
            match requested_huggler_tool(&request, provider) {
                "thread_list" => {}
                "thread_send" => sends += 1,
                other => panic!("unexpected tool approval {other:?}"),
            }
            rt.respond_to_permission(thread, &request.run_id, &request.id, allow_once(&request))
                .unwrap();
        }
    }
}

async fn exchange(author_provider: Provider, reviewer_harness: Reviewer) {
    let reviewer_provider = reviewer_harness.provider();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("checkout");
    checkout(&repo);
    let mut options = HugglerOptions::new(dir.path().join("state"));
    options.agent_tools = true;
    options.codex.request_approvals = true;
    reviewer_harness.configure(&mut options, &repo);
    let rt = Huggler::open(options.clone()).await.unwrap();
    let project = rt.open_project(&repo).unwrap();
    let author = launch(&rt, &project, "author", author_provider);
    let reviewer = launch(&rt, &project, "reviewer", reviewer_provider);
    let other = dir.path().join("other");
    checkout(&other);
    let outside = launch(
        &rt,
        &rt.open_project(other).unwrap(),
        "outside",
        author_provider,
    );
    let (_, mut events) = rt.subscribe(&author, None).unwrap();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let approvals =
        [(&author, author_provider), (&reviewer, reviewer_provider)].map(|(thread, provider)| {
            approve_peer_tools(
                &rt,
                thread,
                provider,
                rt.subscribe(thread, None).unwrap().1,
                stopped.clone(),
            )
        });
    let (author_idle, idle) = tokio::sync::oneshot::channel();
    let conversation = async {
        let review = match reviewer_harness {
            // Exercise the cross-harness delivery contract with explicit tool inputs.
            Reviewer::ClaudeAgent => {
                "Review 2+2. Call mcp__huggler__thread_send to reply. Set threadId to the thread_id in the sender header, message to the literal string '2+2=4', and clientRequestId to 'reply-1'. Call the same tool a second time with identical inputs. Copy the messageId from each result into your final answer, one per line."
            }
            // The Codex review also checks whether the reviewer chooses the peer tool.
            _ => {
                "Review 2+2. Reply to the independent peer who sent this request with exactly '2+2=4'. Use the request key reply-1 and retry the same delivery once with identical arguments. Include both returned message IDs in your final answer."
            }
        };
        let prompt = format!(
            "Use the huggler MCP thread_list to find the reviewer. Use huggler thread_send once with clientRequestId review-1. Set message to exactly the following quoted text: \"{review}\". After sending the review request, finish your own turn immediately. When the reply arrives simply acknowledge it."
        );
        let first = rt
            .send(CommandId("ask".into()), author.clone(), prompt)
            .unwrap()
            .run_id
            .unwrap();
        completed(&rt, &author, &first).await;
        author_idle.send(()).unwrap();
        let request = rt
            .peer_receipt(&author, "review-1")
            .unwrap()
            .unwrap_or_else(|| {
                panic!(
                    "Reviewer was never contacted. Author said {:?}",
                    rt.read(&author).unwrap().messages.last()
                )
            });
        completed(&rt, &reviewer, &request.run_id).await;
        let reply = rt
            .peer_receipt(&reviewer, "reply-1")
            .unwrap()
            .unwrap_or_else(|| {
                panic!(
                    "Author received no reply. Reviewer said {:?}",
                    rt.read(&reviewer).unwrap().messages.last()
                )
            });
        completed(&rt, &author, &reply.run_id).await;
        let answer = rt
            .read(&reviewer)
            .unwrap()
            .messages
            .last()
            .unwrap()
            .text
            .clone();
        assert!(
            answer.matches(&reply.message_id.0).count() >= 2,
            "Reviewer did not report both receipts: {answer}"
        );
        let mut delivered = vec![];
        while let Ok(Ok(event)) = timeout(Duration::from_millis(30), events.next()).await {
            if let Change::Message { message } = event.change
                && message.origin.is_some()
            {
                delivered.push(message.id);
            }
        }
        assert_eq!(delivered, vec![reply.message_id.clone()]);
        stop.send_replace(true);
        (first, request, reply)
    };
    let [a, b] = approvals;
    // Release the live reviewer's first send after the requesting turn has finished.
    let reviewer_approvals = async {
        idle.await.unwrap();
        b.await
    };
    let ((first, request, reply), author_sends, reviewer_sends) =
        tokio::join!(conversation, a, reviewer_approvals);
    assert_eq!(
        author_sends,
        usize::from(author_provider == Provider::Codex)
    );
    assert_eq!(reviewer_sends, 2);
    rt.close().await.unwrap();
    let rt = Huggler::open(options).await.unwrap();
    assert_eq!(request.delivery, PeerDelivery::Started);
    assert_eq!(reply.delivery, PeerDelivery::Started);
    assert!(rt.read(&author).unwrap().tasks.is_empty());
    assert!(rt.read(&reviewer).unwrap().tasks.is_empty());
    for (sender, source_run, key, receipt, recipient) in [
        (&author, &first, "review-1", &request, &reviewer),
        (&reviewer, &request.run_id, "reply-1", &reply, &author),
    ] {
        assert_eq!(
            rt.peer_receipt(sender, key).unwrap().as_ref(),
            Some(receipt)
        );
        assert!(receipt.task_id.is_none());
        assert_eq!(&receipt.thread_id, recipient);
        let thread = rt.read(recipient).unwrap();
        let received: Vec<_> = thread
            .messages
            .iter()
            .filter(|m| m.origin.is_some())
            .collect();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].id, receipt.message_id);
        assert_eq!(received[0].run_id, receipt.run_id);
        assert_eq!(
            received[0].origin.as_ref().unwrap(),
            &MessageOrigin {
                environment_id: None,
                thread_id: sender.clone(),
                run_id: source_run.clone(),
                request_key: key.into()
            }
        );
    }
    assert_eq!(rt.read(&author).unwrap().runs.len(), 2);
    assert_eq!(rt.read(&reviewer).unwrap().runs.len(), 1);
    assert!(rt.read(&outside).unwrap().runs.is_empty());
    assert_eq!(
        rt.read(&author)
            .unwrap()
            .messages
            .iter()
            .find(|m| m.id == reply.message_id)
            .unwrap()
            .text,
        "2+2=4"
    );
    rt.close().await.unwrap();
}

#[tokio::test]
#[ignore = "uses two live Codex sessions with an authenticated local CLI"]
async fn live_codex_peer_exchange() {
    timeout(
        Duration::from_secs(240),
        exchange(Provider::Codex, Reviewer::Codex),
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "uses a live Codex author with the scripted ACP reviewer"]
async fn live_codex_to_acp_peer_exchange() {
    timeout(
        Duration::from_secs(240),
        exchange(Provider::Codex, Reviewer::ScriptedAcp),
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "uses a live Codex author and an authenticated standalone Claude Agent ACP reviewer"]
async fn live_codex_to_claude_peer_exchange() {
    timeout(
        Duration::from_secs(240),
        exchange(Provider::Codex, Reviewer::ClaudeAgent),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn acp_peer_exchange() {
    timeout(
        Duration::from_secs(15),
        exchange(Provider::ClaudeAgent, Reviewer::ScriptedAcp),
    )
    .await
    .unwrap();
}

#[derive(serde::Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct CreatedPeer {
    thread_id: ThreadId,
    run_id: Option<RunId>,
}

async fn batch(
    rt: &Huggler,
    caller: &ThreadId,
    command: &str,
    entries: &Value,
) -> (RunId, Vec<CreatedPeer>) {
    let input = json!({"threads": entries, "clientRequestId": "peer-batch"});
    let run = rt
        .send(
            CommandId(command.into()),
            caller.clone(),
            format!("create peers {input}"),
        )
        .unwrap()
        .run_id
        .unwrap();
    completed(rt, caller, &run).await;
    let snapshot = rt.read(caller).unwrap();
    let reply: Value = serde_json::from_str(&snapshot.messages.last().unwrap().text).unwrap();
    (
        run,
        serde_json::from_value(reply["threads"].clone()).unwrap(),
    )
}

#[tokio::test]
async fn batch_creation_retries_after_reopen_and_accepts_a_late_prompt() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        checkout(dir.path());
        std::fs::write(dir.path().join("batch-marker"), "shared checkout").unwrap();
        let mut options = HugglerOptions::new(dir.path().join("state"));
        options.agent_tools = true;
        options.claude_agent.executable =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/peer-acp.py");
        options.instances.push(ProviderInstance::new(
            "writer-profile",
            Provider::ClaudeAgent,
            &options.claude_agent.executable,
        ));
        let rt = Huggler::open(options.clone()).await.unwrap();
        let project = rt.open_project(dir.path()).unwrap();
        let author = launch(&rt, &project, "author", Provider::ClaudeAgent);
        let mut entries = json!([
            {"title":"Reviewer","prompt":"read batch marker"},
            {"title":"Writer","target":{"providerInstanceId":"writer-profile"}}
        ]);
        let (creator_run, peers) = batch(&rt, &author, "create", &entries).await;
        let [reviewer, writer] = peers.as_slice() else {
            panic!("expected two peers")
        };
        let review_run = reviewer.run_id.as_ref().unwrap();
        completed(&rt, &reviewer.thread_id, review_run).await;
        assert_ne!(reviewer.thread_id, writer.thread_id);
        assert!(writer.run_id.is_none());
        rt.close().await.unwrap();

        let rt = Huggler::open(options).await.unwrap();
        let (_, replay) = batch(&rt, &author, "replay", &entries).await;
        assert_eq!(replay, peers);
        let reviewed = rt.read(&reviewer.thread_id).unwrap();
        let idle = rt.read(&writer.thread_id).unwrap();
        assert_eq!(rt.threads().unwrap().len(), 3);
        assert_eq!(idle.provider_instance.as_str(), "writer-profile");
        assert_eq!(reviewed.provider_instance.as_str(), "claudeAgent");
        for thread in [&reviewed, &idle] {
            assert_eq!(thread.project, project.id);
            assert_eq!(thread.provider, Provider::ClaudeAgent);
            assert!(thread.tasks.is_empty());
        }
        let parent = rt.read(&author).unwrap();
        assert!(parent.tasks.is_empty());
        assert_eq!(parent.runs.len(), 2);
        let (_, mut events) = rt.subscribe(&author, Some(0)).unwrap();
        let mut created = Vec::new();
        loop {
            let event = events.next().await.unwrap();
            if let Change::PeerCreated {
                caller_run_id,
                peer_thread_id,
            } = event.change
            {
                assert_eq!(caller_run_id, creator_run);
                created.push(peer_thread_id);
            }
            if event.sequence == parent.cursor {
                break;
            }
        }
        assert_eq!(
            created,
            [reviewer.thread_id.clone(), writer.thread_id.clone()]
        );
        assert_eq!(reviewed.runs.len(), 1);
        assert_eq!(reviewed.messages.len(), 2);
        assert_eq!(reviewed.messages[0].run_id, *review_run);
        assert_eq!(
            reviewed.messages[0].origin.as_ref().unwrap(),
            &MessageOrigin {
                environment_id: None,
                thread_id: author.clone(),
                run_id: creator_run,
                request_key: "peer-batch".into(),
            }
        );
        assert_eq!(reviewed.messages[1].text, "shared checkout");
        assert!(idle.runs.is_empty() && idle.messages.is_empty());
        assert_eq!(
            (reviewed.name.as_str(), idle.name.as_str()),
            ("Reviewer", "Writer")
        );

        entries[1]["prompt"] = json!("read batch marker");
        let (later, prompted) = batch(&rt, &author, "late-prompt", &entries).await;
        assert_eq!(prompted[0], *reviewer);
        assert_eq!(prompted[1].thread_id, writer.thread_id);
        completed(&rt, &writer.thread_id, prompted[1].run_id.as_ref().unwrap()).await;
        let started = rt.read(&writer.thread_id).unwrap();
        assert_eq!(started.runs.len(), 1);
        assert_eq!(started.messages[0].origin.as_ref().unwrap().run_id, later);
        assert_eq!(started.messages[1].text, "shared checkout");

        // A different caller can use the same key for a fresh, fully prompted batch.
        let (_, other) = batch(&rt, &writer.thread_id, "both-prompted", &entries).await;
        for peer in other {
            completed(&rt, &peer.thread_id, peer.run_id.as_ref().unwrap()).await;
            let snapshot = rt.read(&peer.thread_id).unwrap();
            assert_eq!(snapshot.provider_instance.as_str(), "writer-profile");
            assert_eq!(snapshot.runs.len(), 1);
        }
        assert_eq!(rt.threads().unwrap().len(), 5);
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

async fn held(rt: &Huggler, thread: &ThreadId, prompt: &str) -> PermissionRequest {
    let (_, mut events) = rt.subscribe(thread, None).unwrap();
    rt.send(CommandId(prompt.into()), thread.clone(), prompt.into())
        .unwrap();
    loop {
        if let Change::PermissionRequested { request } = events.next().await.unwrap().change {
            return request;
        }
    }
}

#[tokio::test]
async fn revoked_sender_and_restart_with_two_queued_deliveries() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        checkout(dir.path());
        let mut options = HugglerOptions::new(dir.path().join("state"));
        options.agent_tools = true;
        options.claude_agent.executable =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/peer-acp.py");
        let rt = Huggler::open(options.clone()).await.unwrap();
        let project = rt.open_project(dir.path()).unwrap();
        let author = launch(&rt, &project, "author", Provider::ClaudeAgent);
        let reviewer = launch(&rt, &project, "reviewer", Provider::ClaudeAgent);
        held(&rt, &author, "hold").await;
        let request = held(&rt, &reviewer, "queue-two").await;
        let receipts =
            ["first", "second"].map(|key| rt.peer_receipt(&reviewer, key).unwrap().unwrap());
        let snapshot = rt.read(&author).unwrap();
        for (run, receipt) in snapshot.runs[1..].iter().zip(&receipts) {
            assert_eq!(run.status, RunStatus::Queued);
            assert_eq!(run.id, receipt.run_id);
            assert_eq!(receipt.delivery, PeerDelivery::Queued);
        }
        assert!(!rt.revoke_agent_tools(&author, &request.run_id).unwrap());
        assert!(rt.revoke_agent_tools(&reviewer, &request.run_id).unwrap());
        rt.respond_to_permission(&reviewer, &request.run_id, &request.id, "done".into())
            .unwrap();
        completed(&rt, &reviewer, &request.run_id).await;
        assert!(rt.peer_receipt(&reviewer, "revoked").unwrap().is_none());
        rt.close().await.unwrap();

        let rt = Huggler::open(options).await.unwrap();
        completed(&rt, &author, &receipts[1].run_id).await;
        let snapshot = rt.read(&author).unwrap();
        assert_eq!(snapshot.runs.len(), 3);
        assert_eq!(snapshot.runs[0].status, RunStatus::Failed);
        assert_eq!(snapshot.runs[1].status, RunStatus::Completed);
        let answers: Vec<_> = snapshot
            .messages
            .iter()
            .filter(|m| m.role == Role::Assistant)
            .map(|m| m.text.as_str())
            .collect();
        assert_eq!(answers, ["Received first", "Received second"]);
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}
