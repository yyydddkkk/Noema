#![cfg(feature = "live-eval")]

use std::process::Command;

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_noema-live-eval"));
    command
        .env("NOEMA_OPENAI_MODEL", "gpt-test-snapshot")
        .env("NOEMA_OPENAI_INPUT_USD_PER_MILLION", "2")
        .env("NOEMA_OPENAI_OUTPUT_USD_PER_MILLION", "8")
        .env("NOEMA_LIVE_EVAL_MAX_USD", "1")
        .env_remove("OPENAI_API_KEY")
        .env_remove("NOEMA_LIVE_EVAL");
    command
}

#[test]
fn dry_run_computes_the_budget_without_credentials_or_network() {
    let output = command().arg("--dry-run").output().expect("run dry mode");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 output");
    assert!(stdout.contains("LIVE_EVAL_DRY_RUN"));
    assert!(stdout.contains("requests=1"));
    assert!(stdout.contains("estimated_maximum_input_tokens="));
    assert!(stdout.contains("maximum_output_tokens=2048"));
}

#[test]
fn live_mode_refuses_before_reading_a_key_or_using_network() {
    let output = command().arg("--live").output().expect("run live mode");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 error");
    assert!(stderr.contains("set NOEMA_LIVE_EVAL=ONE_REQUEST_TO_OPENAI"));
    assert!(!stderr.contains("test-key"));
}

#[test]
fn dry_run_rejects_an_insufficient_budget_without_network() {
    let output = command()
        .env("NOEMA_LIVE_EVAL_MAX_USD", "0.000001")
        .arg("--dry-run")
        .output()
        .expect("run dry mode");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 error");
    assert!(stderr.contains("exceeds budget"));
}
