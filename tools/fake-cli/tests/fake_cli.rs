#![cfg(windows)]

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ability-radar-fake-cli-test-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create unique temporary directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove temporary directory");
    }
}

fn fake_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ability-radar-fake-cli"))
}

fn install_as(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(format!("{name}.exe"));
    fs::copy(fake_binary(), &path).expect("copy fake executable");
    path
}

fn run(program: &Path, args: &[&str], current_dir: &Path) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(current_dir)
        .output()
        .expect("run fake executable")
}

fn assert_jsonl(output: &Output, expected_type: &str) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout.clone()).expect("UTF-8 stdout");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let value: Value = serde_json::from_str(lines[0]).expect("valid terminal JSONL");
    assert_eq!(value["type"], expected_type);
}

#[test]
fn supports_only_exact_version_and_auth_probes() {
    let temporary = TempDirectory::new();
    let codex = install_as(temporary.path(), "codex");
    let claude = install_as(temporary.path(), "claude");

    let version = run(&codex, &["--version"], temporary.path());
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        "ability-radar-fake-cli 0.1.0\n"
    );

    let codex_auth = run(&codex, &["login", "status"], temporary.path());
    assert!(codex_auth.status.success());
    assert_eq!(
        String::from_utf8_lossy(&codex_auth.stdout),
        "Logged in using ChatGPT\n"
    );

    let claude_auth = run(&claude, &["auth", "status"], temporary.path());
    assert!(claude_auth.status.success());
    let auth: Value = serde_json::from_slice(&claude_auth.stdout).unwrap();
    assert_eq!(auth, serde_json::json!({"loggedIn": true}));

    for args in [
        vec!["--help"],
        vec!["login"],
        vec!["auth", "status", "--verbose"],
    ] {
        assert!(!run(&codex, &args, temporary.path()).status.success());
    }
}

#[test]
fn writes_verified_fixture_solutions_for_both_execution_shapes() {
    let temporary = TempDirectory::new();
    let codex = install_as(temporary.path(), "codex");
    let claude = install_as(temporary.path(), "claude");
    let dedupe = temporary.path().join("dedupe-events");
    let retry = temporary.path().join("retry-schedule");
    fs::create_dir_all(dedupe.join("src")).unwrap();
    fs::create_dir_all(retry.join("src")).unwrap();

    let codex_output = run(
        &codex,
        &[
            "exec",
            "--ephemeral",
            "--json",
            "--sandbox",
            "workspace-write",
            "--ignore-user-config",
            "--ignore-rules",
            "--model",
            "deterministic fake",
            "solve the fixture",
        ],
        &dedupe,
    );
    assert_jsonl(&codex_output, "turn.completed");
    assert!(dedupe.join("src/dedupeEvents.mjs").is_file());

    let claude_output = run(
        &claude,
        &[
            "-p",
            "solve the fixture",
            "--bare",
            "--no-session-persistence",
            "--output-format",
            "stream-json",
            "--max-turns",
            "20",
            "--tools",
            "Read,Edit,Write",
            "--allowedTools",
            "Read",
            "Edit",
            "Write",
            "--permission-mode",
            "dontAsk",
            "--model",
            "deterministic fake",
        ],
        &retry,
    );
    assert_jsonl(&claude_output, "result");
    assert!(retry.join("src/retrySchedule.mjs").is_file());
}

#[test]
fn rejects_unknown_tasks_and_unconstrained_execution_shapes() {
    let temporary = TempDirectory::new();
    let codex = install_as(temporary.path(), "codex");
    let unknown = temporary.path().join("unknown-task");
    fs::create_dir_all(unknown.join("src")).unwrap();

    let unknown_task = run(
        &codex,
        &[
            "exec",
            "--ephemeral",
            "--json",
            "--sandbox",
            "workspace-write",
            "--ignore-user-config",
            "--ignore-rules",
            "solve",
        ],
        &unknown,
    );
    assert!(!unknown_task.status.success());

    let dangerous_shape = run(
        &codex,
        &[
            "exec",
            "--ephemeral",
            "--json",
            "--sandbox",
            "danger-full-access",
            "--ignore-user-config",
            "--ignore-rules",
            "solve",
        ],
        &unknown,
    );
    assert!(!dangerous_shape.status.success());
    assert!(!unknown.join("src/dedupeEvents.mjs").exists());
    assert!(!unknown.join("src/retrySchedule.mjs").exists());
}
