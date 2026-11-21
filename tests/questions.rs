#![cfg(unix)]
#[path = "support/agents.rs"]
mod agents;
#[path = "support/codex_replay.rs"]
mod replay;

use agents::{checkout, completed, launch};
use huggler::*;
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};
use tokio::time::timeout;

fn recording() -> Vec<Value> {
    include_str!("fixtures/codex-plan-questions/codex_transcript.ndjson")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn index(rows: &[Value], direction: &str, label: &str) -> usize {
    rows.iter()
        .position(|r| r["type"] == direction && r["label"] == label)
        .unwrap()
}

async fn plan(
    dir: &Path,
    rows: &[Value],
    agent_tools: bool,
) -> (
    HugglerOptions,
    Huggler,
    Project,
    ThreadId,
    RunId,
    Subscription,
) {
    let mut options = replay::config(dir, rows, agent_tools);
    options.claude_agent.executable =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/peer-acp.py");
    let rt = Huggler::open(options.clone()).await.unwrap();
    let repo = dir.join("repo");
    checkout(&repo);
    let project = rt.open_project(&repo).unwrap();
    let thread = ThreadId("planner".into());
    rt.launch(
        CommandId("launch-plan".into()),
        thread.clone(),
        project.id.clone(),
        "Replay fixture: plan_questions".into(),
        Some("gpt-6-luna".into()),
    )
    .unwrap();
    let (_, events) = rt.subscribe(&thread, None).unwrap();
    let start = &rows[index(rows, "expect_outbound", "turn/start")]["frame"]["params"];
    let receipt = rt
        .dispatch(Command {
            id: CommandId("plan".into()),
            thread_id: thread.clone(),
            action: Action::SendPlan {
                text: start["input"][0]["text"].as_str().unwrap().into(),
            },
        })
        .unwrap();
    (
        options,
        rt,
        project,
        thread,
        receipt.run_id.unwrap(),
        events,
    )
}

async fn requested(events: &mut Subscription) -> UserInputRequest {
    loop {
        if let Change::QuestionRequested { request } = events.next().await.unwrap().change {
            return request;
        }
    }
}

fn send(rt: &Huggler, thread: &ThreadId, text: String) -> RunId {
    rt.send(CommandId(format!("send-{thread}")), thread.clone(), text)
        .unwrap()
        .run_id
        .unwrap()
}

#[tokio::test]
async fn peer_answers_recorded_question_once_and_only_the_host_resolves_permissions() {
    timeout(Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        let rows = recording();
        let (options, rt, project, planner, run, mut events) = plan(dir.path(), &rows, true).await;
        let request = requested(&mut events).await;
        let question_frame =
            &rows[index(&rows, "emit_inbound", "item/tool/requestUserInput")]["frame"];
        assert_eq!(
            json!(request.questions),
            question_frame["params"]["questions"]
        );
        assert_eq!(request.run_id, run);
        let held = launch(&rt, &project, "held", Provider::ClaudeAgent);
        let (_, mut held_events) = rt.subscribe(&held, None).unwrap();
        let held_run = send(&rt, &held, "hold".into());
        let permission = loop {
            if let Change::PermissionRequested { request } =
                held_events.next().await.unwrap().change
            {
                break request;
            }
        };
        let other = dir.path().join("other");
        checkout(&other);
        let outside = launch(
            &rt,
            &rt.open_project(other).unwrap(),
            "outside",
            Provider::ClaudeAgent,
        );
        let peer = launch(&rt, &project, "reviewer", Provider::ClaudeAgent);
        let check = |outside| {
            let case = json!({
                "outside": outside,
                "id": request.id,
                "permission": permission.id,
                "questions": request.questions,
            });
            format!("check question {case}")
        };
        completed(&rt, &outside, &send(&rt, &outside, check(true))).await;
        assert_eq!(
            rt.read(&planner).unwrap().questions,
            std::slice::from_ref(&request)
        );
        let peer_run = send(&rt, &peer, check(false));
        completed(&rt, &peer, &peer_run).await;
        completed(&rt, &planner, &run).await;
        assert_eq!(
            rt.read(&planner).unwrap().messages.last().unwrap().text,
            "plan questions fixture complete"
        );
        assert_eq!(rt.read(&held).unwrap().permissions[0].id, permission.id);
        rt.respond_to_permission(&held, &held_run, &permission.id, "done".into())
            .unwrap();
        completed(&rt, &held, &held_run).await;
        rt.close().await.unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("cursor")).unwrap(),
            (rows.len() - 1).to_string()
        );
        let rt = Huggler::open(options).await.unwrap();
        let (snapshot, mut history) = rt.subscribe(&planner, Some(0)).unwrap();
        assert!(snapshot.questions.is_empty());
        let mut answered = vec![];
        loop {
            let event = history.next().await.unwrap();
            if let Change::QuestionClosed {
                id,
                answered: true,
                responder_run,
            } = event.change
            {
                answered.push((id, responder_run));
            }
            if event.sequence == snapshot.cursor {
                break;
            }
        }
        assert_eq!(answered, [(request.id.clone(), Some(peer_run))]);
        let retry = rt
            .respond_to_question(&planner, &run, &request.id, UserAnswers::new())
            .await;
        assert!(matches!(retry, Err(Error::AlreadyAnswered)));
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn early_question_preserves_previews_and_expires_on_provider_loss() {
    timeout(Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        let mut rows = recording();
        let request_at = index(&rows, "emit_inbound", "item/tool/requestUserInput");
        let mut early = rows.remove(request_at);
        early["frame"]["params"]["questions"][0]["options"][0]["preview"] =
            json!("Proposed schema");
        let start = index(&rows, "emit_inbound", "turn/start");
        rows.insert(start, early);
        let response_at = index(&rows, "expect_outbound", "item/tool/requestUserInput");
        rows.truncate(response_at);
        rows.push(json!({"type":"await_release"}));
        rows.push(json!({"type":"runtime_exit"}));
        let (options, rt, _, thread, run, mut events) = plan(dir.path(), &rows, false).await;
        let request = requested(&mut events).await;
        assert_eq!(
            request.questions[0]["options"][0]["preview"],
            "Proposed schema"
        );
        fs::write(dir.path().join("release"), "").unwrap();
        assert_eq!(
            rt.wait(&thread, &run).await.unwrap().status,
            RunStatus::Failed
        );
        let event = events.next().await.unwrap();
        assert!(matches!(event.change, Change::QuestionClosed {
            id, answered: false, responder_run: None
        } if id == request.id));
        rt.close().await.unwrap();
        let rt = Huggler::open(options).await.unwrap();
        assert!(rt.read(&thread).unwrap().questions.is_empty());
        let late = rt
            .respond_to_question(&thread, &run, &request.id, UserAnswers::new())
            .await;
        assert!(matches!(late, Err(Error::Invalid(reason)) if reason.contains("expired")));
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn host_answers_on_the_owning_run_without_agent_tools() {
    timeout(Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        let rows = recording();
        let (_, rt, _, thread, run, mut events) = plan(dir.path(), &rows, false).await;
        let request = requested(&mut events).await;
        let other = RunId("another-run".into());
        let result = rt
            .respond_to_question(&thread, &other, &request.id, UserAnswers::new())
            .await;
        assert!(matches!(result, Err(Error::NotFound(_))));
        let answers = UserAnswers::from([(
            "schema_preference".into(),
            UserAnswer::Choices(vec!["Strict schemas (Recommended)".into()]),
        )]);
        rt.respond_to_question(&thread, &run, &request.id, answers)
            .await
            .unwrap();
        let event = events.next().await.unwrap();
        assert!(matches!(event.change, Change::QuestionClosed {
            id, answered: true, responder_run: None
        } if id == request.id));
        completed(&rt, &thread, &run).await;
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn provider_resolves_an_unanswered_question_before_completing() {
    timeout(Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        let mut rows = recording();
        let answer = index(&rows, "expect_outbound", "item/tool/requestUserInput");
        rows[answer] = json!({"type": "await_release"});
        let (_, rt, _, thread, run, mut events) = plan(dir.path(), &rows, false).await;
        let request = requested(&mut events).await;
        fs::write(dir.path().join("release"), "").unwrap();
        let event = events.next().await.unwrap();
        assert!(matches!(event.change, Change::QuestionClosed {
            id, answered: false, responder_run: None
        } if id == request.id));
        completed(&rt, &thread, &run).await;
        assert!(rt.read(&thread).unwrap().questions.is_empty());
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}
