/// Integration test that verifies the CliAgentRunner can actually spawn Claude
/// and get a structured result back.
///
/// This test requires `claude` to be on PATH and authenticated.
/// Run with: cargo test --test agent_integration_test -- --ignored
use std::time::Duration;

use tempfile::TempDir;

use phase_golem::agent::{AgentRunner, CliAgentRunner};
use phase_golem::types::ResultCode;

#[tokio::test]
#[ignore] // requires real claude CLI — run explicitly
async fn cli_agent_runner_can_spawn_and_get_result() {
    // Verify CLI exists first
    CliAgentRunner::verify_cli_available().expect("claude CLI not available");

    let tmp = TempDir::new().unwrap();
    let result_path = tmp.path().join("test_result.json");

    let prompt = format!(
        "You are a test agent. Write this exact JSON to the file below and do nothing else.\n\n\
         File: {}\n\n\
         ```json\n\
         {{\n\
         \x20 \"item_id\": \"TEST-001\",\n\
         \x20 \"phase\": \"smoke_test\",\n\
         \x20 \"result\": \"phase_complete\",\n\
         \x20 \"summary\": \"Smoke test passed\"\n\
         }}\n\
         ```\n\n\
         Write exactly that JSON. Do not add any extra fields or formatting.",
        result_path.display()
    );

    let runner = CliAgentRunner;
    let timeout = Duration::from_secs(120);

    let result = runner.run_agent(&prompt, &result_path, timeout).await;

    match result {
        Ok(phase_result) => {
            assert_eq!(phase_result.item_id, "TEST-001");
            assert_eq!(phase_result.phase, "smoke_test");
            assert_eq!(phase_result.result, ResultCode::PhaseComplete);
            eprintln!("PASS: Got expected result back from Claude CLI");
        }
        Err(e) => {
            panic!("Agent runner failed: {}\n\nThis means Claude CLI could not be spawned or did not produce a valid result file.", e);
        }
    }
}
