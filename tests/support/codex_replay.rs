use huggler::HugglerOptions;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub fn config(dir: &Path, records: &[Value], agent_tools: bool) -> HugglerOptions {
    let executable = dir.join("codex");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/replay-codex.py"),
        &executable,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let fixture = dir.join("replay.ndjson");
    fs::write(
        &fixture,
        records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let approvals = records[0]["scenario"] == "subagent_v2_nested_approval";
    fs::write(
        dir.join("case.json"),
        json!({"fixture":fixture,"version":env!("CARGO_PKG_VERSION"),"start":1,"approvals":approvals,"agentTools":agent_tools}).to_string(),
    )
    .unwrap();
    let mut options = HugglerOptions::new(dir.join("state"));
    options.agent_tools = agent_tools;
    options.codex.executable = executable;
    options.codex.request_approvals = approvals;
    options
}
