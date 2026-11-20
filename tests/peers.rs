#![cfg(unix)]
use huggler::*;
#[path = "support/agents.rs"]
mod agents;
use agents::*;
use std::{path::Path, time::Duration};
use tokio::time::timeout;

async fn approve_peer_tools(
    rt: &Huggler,
    thread: &ThreadId,
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
            let params = &request.tool_call["rawInput"];
            assert_eq!(params["serverName"], "huggler");
            match params["_meta"]["tool_name"].as_str() {
                Some("thread_list") => {}
                Some("thread_send") => sends += 1,
                other => panic!("unexpected tool approval {other:?}"),
            }
            rt.respond_to_permission(thread, &request.run_id, &request.id, "accept".into())
                .unwrap();
        }
    }
}

async fn exchange(provider: Provider, live: bool) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("checkout");
    checkout(&repo);
    let mut options = HugglerOptions::new(dir.path().join("state"));
    options.agent_tools = true;
    options.codex.request_approvals = live;
    if !live {
        options.claude_agent.executable =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/peer-acp.py");
    }
    let rt = Huggler::open(options.clone()).await.unwrap();
    let project = rt.open_project(&repo).unwrap();
    let author = launch(&rt, &project, "author", provider);
    let reviewer = launch(&rt, &project, "reviewer", provider);
    let other = dir.path().join("other");
    checkout(&other);
    let outside = launch(&rt, &rt.open_project(other).unwrap(), "outside", provider);
    let (_, mut events) = rt.subscribe(&author, None).unwrap();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let approvals = [&author, &reviewer].map(|thread| {
        approve_peer_tools(
            &rt,
            thread,
            rt.subscribe(thread, None).unwrap().1,
            stopped.clone(),
        )
    });
    let conversation = async {
        let first = rt.send(CommandId("ask".into()),author.clone(),
        "Use the huggler MCP thread_list to find the reviewer. Use huggler thread_send with clientRequestId review-1 and this exact message: Review 2+2. Send your finding '2+2=4' to the sender thread from the origin, using huggler thread_send with clientRequestId reply-1. Call the same tool a second time with identical arguments, then put both returned messageIds in your final answer. After sending the review request, finish your own turn immediately. When the reply arrives simply acknowledge it.".into()).unwrap().run_id.unwrap();
        completed(&rt, &author, &first).await;
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
            if let Change::Message { message } = event.change {
                if message.origin.is_some() {
                    delivered.push(message.id);
                }
            }
        }
        assert_eq!(delivered, vec![reply.message_id.clone()]);
        stop.send_replace(true);
        (first, request, reply)
    };
    let [a, b] = approvals;
    let ((first, request, reply), author_sends, reviewer_sends) = tokio::join!(conversation, a, b);
    if live {
        assert_eq!((author_sends, reviewer_sends), (1, 2));
    }
    rt.close().await.unwrap();
    let rt = Huggler::open(options).await.unwrap();
    for (sender, source_run, key, receipt, recipient) in [
        (&author, &first, "review-1", &request, &reviewer),
        (&reviewer, &request.run_id, "reply-1", &reply, &author),
    ] {
        assert_eq!(
            rt.peer_receipt(sender, key).unwrap().as_ref(),
            Some(receipt)
        );
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
    timeout(Duration::from_secs(240), exchange(Provider::Codex, true))
        .await
        .unwrap();
}

#[tokio::test]
async fn acp_peer_exchange() {
    timeout(
        Duration::from_secs(15),
        exchange(Provider::ClaudeAgent, false),
    )
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
