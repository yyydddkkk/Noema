use std::{env, path::Path, process::Command, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    let command = arguments.next();
    if command.as_deref() != Some("check") || arguments.next().is_some() {
        eprintln!("usage: cargo xtask check");
        return ExitCode::FAILURE;
    }

    match check() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn check() -> Result<(), String> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be a direct child of the workspace root");

    run_cargo(workspace, &["fmt", "--all", "--check"])?;
    run_cargo(
        workspace,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run_cargo(workspace, &["test", "--workspace", "--all-features"])
}

fn run_cargo(workspace: &Path, arguments: &[&str]) -> Result<(), String> {
    eprintln!("+ cargo {}", arguments.join(" "));
    let status = Command::new("cargo")
        .args(arguments)
        .current_dir(workspace)
        .status()
        .map_err(|error| format!("failed to start cargo: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "cargo {} exited with {status}",
            arguments.join(" ")
        ))
    }
}
