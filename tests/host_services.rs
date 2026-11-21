#![cfg(all(unix, feature = "application"))]
use huggler::{
    services::{self, Action, Kind, Open, Output, Resource, ServiceHost},
    *,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[path = "support/agents.rs"]
mod agents;

fn options(dir: &Path) -> HugglerOptions {
    let mut config = HugglerOptions::new(dir);
    config.codex.executable =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/context-provider.py");
    config
}
async fn output(
    rt: &Huggler,
    t: &ThreadId,
    id: &ServiceHandleId,
    after: Option<u64>,
) -> (u64, Vec<u8>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Output::Terminal { next, data, eof } =
                rt.service_call(t, id, Action::Output { after }).unwrap()
                && (!data.is_empty() || eof)
            {
                return (next, data);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn terminal_reconnect_close_and_host_loss() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let rt = Huggler::open(options(&dir.path().join("state"))).await?;
    let repo = dir.path().join("repo");
    agents::checkout(&repo);
    let project = rt.open_project(&repo)?;
    let a = agents::launch(&rt, &project, "a", Provider::Codex);
    let b = agents::launch(&rt, &project, "b", Provider::Codex);
    let open = || {
        let script = "printf first; while [ ! -e resume ]; do sleep .02; done; printf second >&2; exec sleep 30";
        rt.open_service(
            &a,
            Open::Terminal {
                executable: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
            },
        )
    };
    assert!(matches!(open(), Err(Error::Unsupported(_))));
    rt.set_service_host(
        Kind::Terminal,
        Some(Box::new(services::terminal::TerminalHost)),
    )?;
    let id = open()?;
    let (cursor, first) = output(&rt, &a, &id, None).await;
    assert_eq!(first, b"first");
    assert!(rt.service_handles(&b, Kind::Terminal)?.is_empty());
    assert!(matches!(rt.close_service(&b, &id), Err(Error::Forbidden)));
    std::fs::write(repo.join("resume"), "")?;
    let handle = rt.service_handles(&a, Kind::Terminal)?.pop().unwrap();
    let (_, second) = output(&rt, &a, &handle, Some(cursor)).await;
    assert_eq!(second, b"second");
    rt.close_service(&a, &id)?;
    assert!(matches!(
        rt.service_call(&a, &id, Action::Output { after: None }),
        Err(Error::Forbidden)
    ));
    let flood = rt.open_service(
        &a,
        Open::Terminal {
            executable: "python3".into(),
            args: vec![
                "-c".into(),
                "import sys; sys.stdout.buffer.write(b'x' * 70000)".into(),
            ],
        },
    )?;
    let retained = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Output::Terminal {
                next,
                data,
                eof: true,
            } = rt
                .service_call(&a, &flood, Action::Output { after: None })
                .unwrap()
            {
                break (next, data);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(retained, (70000, vec![b'x'; 65536]));
    assert!(matches!(
        rt.service_call(&a, &flood, Action::Output { after: Some(0) }),
        Err(Error::CursorExpired)
    ));
    rt.close_service(&a, &flood)?;
    let id = open()?;
    rt.set_service_host(Kind::Terminal, None)?;
    assert!(matches!(
        rt.service_call(&a, &id, Action::Output { after: None }),
        Err(Error::Forbidden)
    ));
    assert!(matches!(open(), Err(Error::Unsupported(_))));
    rt.close().await
}

const HOME: &str = "<h1>Home</h1>";
const REPORT: &str = "<h1>Review results</h1>";
const URI: &str = "ui://review/panel";
fn page(url: &str) -> Result<&'static str> {
    match url {
        "https://fixture.invalid/home" => Ok(HOME),
        "https://fixture.invalid/report" => Ok(REPORT),
        _ => Err(Error::Forbidden),
    }
}

// This host accepts two fixture pages, one inline document and one virtual device.
// The explicit Chromium qualification renders those documents in an isolated profile.
struct DisplayHost {
    drops: Arc<AtomicUsize>,
    chrome: Option<PathBuf>,
}
struct Display {
    content: &'static str,
    drops: Arc<AtomicUsize>,
    chrome: Option<PathBuf>,
}
impl Drop for Display {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl ServiceHost for DisplayHost {
    fn devices(&mut self) -> Result<Vec<String>> {
        Ok(vec!["virtual-display".into()])
    }
    fn open(&mut self, _: &ThreadId, _: &Path, request: Open) -> Result<Box<dyn Resource>> {
        let content = match request {
            Open::Preview { url } => page(&url)?,
            Open::Html { document } if document == REPORT => REPORT,
            Open::Device { id } if id == "virtual-display" => HOME,
            _ => return Err(Error::Forbidden),
        };
        Ok(Box::new(Display {
            content,
            drops: self.drops.clone(),
            chrome: self.chrome.clone(),
        }))
    }
}
impl Resource for Display {
    fn call(&mut self, action: Action) -> Result<Output> {
        match action {
            Action::Navigate { url } => {
                self.content = page(&url)?;
                Ok(Output::Done)
            }
            Action::Capture => {
                if let Some(chrome) = &self.chrome {
                    let dir = tempfile::tempdir()?;
                    let input = dir.path().join("input.html");
                    let png = dir.path().join("view.png");
                    std::fs::write(&input, self.content)?;
                    let output = std::process::Command::new(chrome)
                        .args([
                            "--headless",
                            "--no-first-run",
                            "--disable-background-networking",
                            "--disable-component-update",
                            "--window-size=400,240",
                        ])
                        .arg(format!(
                            "--user-data-dir={}",
                            dir.path().join("profile").display()
                        ))
                        .arg(format!("--screenshot={}", png.display()))
                        .arg(input)
                        .output()?;
                    assert!(
                        output.status.success(),
                        "Chromium capture exited with {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr)
                    );
                    Ok(Output::Bytes {
                        mime: "image/png".into(),
                        data: std::fs::read(png)?,
                    })
                } else {
                    Ok(Output::Bytes {
                        mime: "text/html".into(),
                        data: self.content.as_bytes().to_vec(),
                    })
                }
            }
            _ => Err(Error::Forbidden),
        }
    }
}
async fn display_lifecycle(chrome: Option<PathBuf>) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let rt = Huggler::open(options(&dir.path().join("state"))).await?;
    let repo = dir.path().join("repo");
    agents::checkout(&repo);
    let project = rt.open_project(&repo)?;
    let owner = agents::launch(&rt, &project, "owner", Provider::Codex);
    let other = agents::launch(&rt, &project, "other", Provider::Codex);
    let drops = Arc::new(AtomicUsize::new(0));
    let host = || {
        Some(Box::new(DisplayHost {
            drops: drops.clone(),
            chrome: chrome.clone(),
        }) as Box<dyn ServiceHost>)
    };
    let capture = |id: &ServiceHandleId| -> Result<Vec<u8>> {
        let Output::Bytes { mime, data } = rt.service_call(&owner, id, Action::Capture)? else {
            panic!("missing capture")
        };
        if chrome.is_some() {
            assert_eq!(mime, "image/png");
            assert!(data.starts_with(b"\x89PNG\r\n\x1a\n"));
        } else {
            assert_eq!(mime, "text/html");
        }
        Ok(data)
    };
    for kind in [Kind::Preview, Kind::Device, Kind::Html] {
        let open = || match kind {
            Kind::Preview => Open::Preview {
                url: "https://fixture.invalid/home".into(),
            },
            Kind::Device => Open::Device {
                id: "virtual-display".into(),
            },
            Kind::Html => Open::Html {
                document: REPORT.into(),
            },
            _ => unreachable!(),
        };
        assert!(matches!(
            rt.open_service(&owner, open()),
            Err(Error::Unsupported(_))
        ));
        rt.set_service_host(kind, host())?;
        if kind == Kind::Device {
            assert_eq!(rt.devices(&owner)?, ["virtual-display"]);
            assert!(matches!(
                rt.open_service(
                    &owner,
                    Open::Device {
                        id: "physical".into()
                    }
                ),
                Err(Error::Forbidden)
            ));
        }
        if kind == Kind::Html {
            assert!(matches!(
                rt.open_service(
                    &owner,
                    Open::Html {
                        document: "<script>bad()</script>".into()
                    }
                ),
                Err(Error::Forbidden)
            ));
        }
        let id = rt.open_service(&owner, open())?;
        assert!(matches!(
            rt.service_call(&other, &id, Action::Capture),
            Err(Error::Forbidden)
        ));
        let before = capture(&id)?;
        if kind == Kind::Preview {
            assert!(matches!(
                rt.service_call(
                    &owner,
                    &id,
                    Action::Navigate {
                        url: "file:///".into()
                    }
                ),
                Err(Error::Forbidden)
            ));
            rt.service_call(
                &owner,
                &id,
                Action::Navigate {
                    url: "https://fixture.invalid/report".into(),
                },
            )?;
            assert_ne!(capture(&id)?, before);
        }
        rt.close_service(&owner, &id)?;
        let lost = rt.open_service(&owner, open())?;
        rt.set_service_host(kind, None)?;
        assert!(matches!(capture(&lost), Err(Error::Forbidden)));
        rt.set_service_host(kind, host())?;
        rt.open_service(&owner, open())?;
        rt.open_service(&other, open())?;
    }
    rt.delete_thread(&owner)?;
    assert_eq!(drops.load(Ordering::SeqCst), 9);
    rt.close().await?;
    assert_eq!(drops.load(Ordering::SeqCst), 12);
    Ok(())
}
#[tokio::test]
async fn preview_device_and_html_lifecycle() -> Result<()> {
    display_lifecycle(None).await
}
#[tokio::test]
#[ignore = "set HUGGLER_TEST_CHROME to a local Chromium executable to qualify offscreen rendering"]
async fn chromium_capture() -> Result<()> {
    display_lifecycle(Some(
        std::env::var_os("HUGGLER_TEST_CHROME")
            .expect("Chromium executable")
            .into(),
    ))
    .await
}

struct AppServer;
impl services::mcp_app::Connection for AppServer {
    fn request(&self, thread: &ThreadId, method: &str, params: Value) -> Result<Value> {
        assert_eq!(thread.0.as_str(), "owner");
        match method {
            "resources/read" if params == json!({"uri":URI}) => Ok(
                json!({"contents":[{"uri":URI,"mimeType":"text/html;profile=mcp-app","text":REPORT}]}),
            ),
            "tools/call" if params == json!({"name":"show","arguments":{"id":7}}) => Ok(
                json!({"content":[{"type":"text","text":"private tool result"}],"structuredContent":{"selected":7}}),
            ),
            _ => Err(Error::Forbidden),
        }
    }
}
#[tokio::test]
async fn app_resources_tools_and_approved_next_turn_context() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(&dir.path().join("state"));
    let rt = Huggler::open(config.clone()).await?;
    let repo = dir.path().join("repo");
    agents::checkout(&repo);
    let project = rt.open_project(&repo)?;
    let owner = agents::launch(&rt, &project, "owner", Provider::Codex);
    let other = agents::launch(&rt, &project, "other", Provider::Codex);
    let open = || Open::McpApp {
        resource: URI.into(),
    };
    assert!(matches!(
        rt.open_service(&owner, open()),
        Err(Error::Unsupported(_))
    ));
    rt.set_service_host(
        Kind::McpApp,
        Some(Box::new(services::mcp_app::AppHost {
            connection: Arc::new(AppServer),
            resources: [URI.into()].into(),
            tools: ["show".into()].into(),
        })),
    )?;
    let id = rt.open_service(&owner, open())?;
    let Output::Json(doc) =
        rt.service_call(&owner, &id, Action::ReadResource { uri: URI.into() })?
    else {
        panic!()
    };
    assert_eq!(doc["contents"][0]["text"], REPORT);
    let tool = |name: &str| Action::CallTool {
        name: name.into(),
        arguments: json!({"id":7}),
    };
    assert!(matches!(
        rt.service_call(&other, &id, tool("show")),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        rt.service_call(&owner, &id, tool("delete")),
        Err(Error::Forbidden)
    ));
    let Output::Json(response) = rt.service_call(&owner, &id, tool("show"))? else {
        panic!()
    };
    assert!(matches!(
        rt.approve_app_context(&other, &id, &response),
        Err(Error::Forbidden)
    ));
    rt.approve_app_context(&owner, &id, &response["structuredContent"])?;
    rt.set_service_host(Kind::McpApp, None)?;
    assert!(matches!(
        rt.service_call(&owner, &id, tool("show")),
        Err(Error::Forbidden)
    ));
    rt.close().await?;
    let rt = Huggler::open(config.clone()).await?;
    let marker = repo.join("fail-before-prompt");
    std::fs::write(&marker, "")?;
    let failed = rt.send(
        CommandId("unavailable".into()),
        owner.clone(),
        "unavailable".into(),
    )?;
    assert_eq!(
        rt.wait(&owner, &failed.run_id.unwrap()).await?.status,
        RunStatus::Failed
    );
    rt.close().await?;
    std::fs::remove_file(marker)?;
    let rt = Huggler::open(config).await?;
    for (thread, text) in [
        (&other, "unrelated"),
        (&owner, "review"),
        (&owner, "continue"),
    ] {
        let receipt = rt.send(CommandId(text.into()), thread.clone(), text.into())?;
        agents::completed(&rt, thread, &receipt.run_id.unwrap()).await;
    }
    let frames = std::fs::read_to_string(repo.join("frames.ndjson"))?;
    let prompts: Vec<_> = frames
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .filter(|f| f["method"] == "turn/start")
        .map(|f| {
            f["params"]["input"][0]["text"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        prompts,
        [
            "unrelated",
            "MCP App context (host-approved data):\n{\"selected\":7}\n\nreview",
            "continue"
        ]
    );
    rt.close().await
}
