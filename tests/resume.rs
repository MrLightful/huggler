#![cfg(unix)]
#[path = "support/resume.rs"]
mod resume;
#[tokio::test]
async fn resumes_the_recorded_conversation_across_two_runtimes() {
    resume::scenario(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).await;
}
