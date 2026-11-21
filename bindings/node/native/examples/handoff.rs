//! Rust host in the packed SDK's cross-process continuation test.
use huggler::{CommandId, Huggler, HugglerOptions, Provider, ProviderInstance, ThreadId};
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args_os().nth(1).expect("consumer directory"));
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("saved.json"))?)?;
    let mut options = HugglerOptions::new(dir.join("state"));
    options.storage.database_path = dir.join("custom.sqlite");
    options.storage.worktree_root = huggler::WorktreeRoot::Shared(dir.join("trees"));
    options.output.version = "fixture-sdk".into();
    let mut instance = ProviderInstance::new("recorded", Provider::Codex, dir.join("codex"));
    instance.home = Some(dir.join("codex-home"));
    options.instances.push(instance);
    options.instances.push(ProviderInstance::new(
        "approval",
        Provider::ClaudeAgent,
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/permission-acp.py")
            .canonicalize()?,
    ));
    let rt = Huggler::open(options).await?;
    let thread = ThreadId("conversation".into());
    let snapshot = rt.read(&thread)?;
    assert_eq!(snapshot.cursor.to_string(), saved["cursor"]);
    let message = &snapshot.messages[0];
    let receipt = rt.send(CommandId("send-first".into()), thread, message.text.clone())?;
    assert_eq!(
        serde_json::to_value(receipt.run_id)?,
        saved["sent"]["runId"]
    );
    rt.close().await?;
    Ok(())
}
