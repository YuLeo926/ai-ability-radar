#![cfg(windows)]

use ability_adapters::{
    AdapterCompletion, AgentAdapter, AuthState, ClaudeCodeAdapter, CodexAdapter, ExecutionRequest,
    ProcessError, ProcessOutput, ProcessRunner, ProcessSpec, TokioProcessRunner,
};
use async_trait::async_trait;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

const CHILD_PROVIDER: &str = "ABILITY_RADAR_PROVIDER_SESSION_CHILD";
const CHILD_LAYOUT: &str = "ABILITY_RADAR_PROVIDER_SESSION_LAYOUT";
const FAKE_LOG: &str = "ABILITY_RADAR_PROVIDER_SESSION_LOG";

struct PathMutatingRunner {
    calls: AtomicUsize,
    replacement_path: OsString,
}

#[async_trait]
impl ProcessRunner for PathMutatingRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        let result = TokioProcessRunner.run(spec, cancellation).await;
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            // SAFETY: this test runs in its own child process and no other test
            // shares or reads its environment.
            unsafe {
                std::env::set_var("PATH", &self.replacement_path);
            }
        }
        result
    }
}

#[test]
fn provider_sessions_retain_one_command_after_path_mutation() {
    for provider in ["codex", "claude"] {
        for layout in ["native", "npm"] {
            let output = Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("provider_session_path_mutation_child")
                .arg("--nocapture")
                .env(CHILD_PROVIDER, provider)
                .env(CHILD_LAYOUT, layout)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{provider} {layout} retained-command child failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
    }
}

#[tokio::test]
async fn provider_session_path_mutation_child() {
    let Ok(provider) = std::env::var(CHILD_PROVIDER) else {
        return;
    };
    let layout = std::env::var(CHILD_LAYOUT).unwrap();
    let fixture = tempdir().unwrap();
    let first = fixture.path().join("first");
    let second = fixture.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let helper = compile_fake_provider(fixture.path());
    let (initial_path, replacement_path) = match layout.as_str() {
        "native" => {
            fs::copy(&helper, first.join(format!("{provider}.exe"))).unwrap();
            fs::copy(&helper, second.join(format!("{provider}.exe"))).unwrap();
            fs::write(first.join("identity.txt"), "first").unwrap();
            fs::write(second.join("identity.txt"), "replacement").unwrap();
            (
                std::env::join_paths([&first]).unwrap(),
                std::env::join_paths([&second]).unwrap(),
            )
        }
        "npm" => (
            install_npm_provider(&first, &provider, &helper, "first"),
            install_npm_provider(&second, &provider, &helper, "replacement"),
        ),
        other => panic!("unsupported provider layout: {other}"),
    };
    let log = fixture.path().join("invocations.log");

    // SAFETY: this test runs in its own child process and no other test shares
    // or reads its environment.
    unsafe {
        std::env::set_var("PATH", &initial_path);
        std::env::set_var(FAKE_LOG, &log);
    }
    let runner: Arc<dyn ProcessRunner> = Arc::new(PathMutatingRunner {
        calls: AtomicUsize::new(0),
        replacement_path,
    });
    let adapter: Arc<dyn AgentAdapter> = match provider.as_str() {
        "codex" => Arc::new(CodexAdapter::new(runner)),
        "claude" => Arc::new(ClaudeCodeAdapter::new(runner)),
        other => panic!("unsupported provider child: {other}"),
    };

    let availability = adapter.detect().await;
    assert!(availability.installed);
    assert_eq!(availability.auth_state, AuthState::Ready);
    let completion = adapter
        .execute(
            ExecutionRequest {
                prompt: "local fake prompt".into(),
                workspace: fixture.path().to_path_buf(),
                time_budget_secs: 10,
                max_turns: 20,
                model: None,
                reasoning_effort: None,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(completion, AdapterCompletion::Completed { .. }));

    let invocations = fs::read_to_string(log).unwrap();
    let lines = invocations.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert!(
        lines.iter().all(|line| line.starts_with("first|")),
        "PATH replacement was invoked:\n{invocations}",
    );
}

fn install_npm_provider(
    root: &std::path::Path,
    provider: &str,
    helper: &std::path::Path,
    identity: &str,
) -> OsString {
    let npm = root.join("npm");
    let node = root.join("node");
    let (package_root, entry, metadata) = match provider {
        "codex" => (
            npm.join("node_modules/@openai/codex"),
            "bin/codex.js",
            r#"{"name":"@openai/codex","bin":{"codex":"bin/codex.js"}}"#,
        ),
        "claude" => (
            npm.join("node_modules/@anthropic-ai/claude-code"),
            "cli.js",
            r#"{"name":"@anthropic-ai/claude-code","bin":{"claude":"cli.js"}}"#,
        ),
        other => panic!("unsupported npm provider: {other}"),
    };
    fs::create_dir_all(package_root.join(std::path::Path::new(entry).parent().unwrap())).unwrap();
    fs::create_dir_all(&node).unwrap();
    fs::write(npm.join(format!("{provider}.cmd")), "@echo off").unwrap();
    fs::write(package_root.join("package.json"), metadata).unwrap();
    fs::write(package_root.join(entry), "reviewed fake entry").unwrap();
    fs::copy(helper, node.join("node.exe")).unwrap();
    fs::write(node.join("identity.txt"), identity).unwrap();
    std::env::join_paths([&npm, &node]).unwrap()
}

fn compile_fake_provider(root: &std::path::Path) -> PathBuf {
    let source = root.join("fake-provider.rs");
    fs::write(
        &source,
        r###"
use std::fs::{self, OpenOptions};
use std::io::Write;

fn main() {
    let executable = std::env::current_exe().unwrap();
    let directory = executable.parent().unwrap();
    let identity = fs::read_to_string(directory.join("identity.txt")).unwrap();
    let invoked_as = executable.file_stem().unwrap().to_string_lossy().to_lowercase();
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let provider = if invoked_as == "node" {
        let entry = args.remove(0).replace('\\', "/");
        if entry.contains("/@openai/codex/") {
            "codex".to_owned()
        } else if entry.contains("/@anthropic-ai/claude-code/") {
            "claude".to_owned()
        } else {
            panic!("unexpected reviewed entry: {entry}");
        }
    } else {
        invoked_as
    };
    let log = std::env::var_os("ABILITY_RADAR_PROVIDER_SESSION_LOG").unwrap();
    let mut output = OpenOptions::new().create(true).append(true).open(log).unwrap();
    writeln!(output, "{}|{}|{}", identity, provider, args.join("\u{1f}")).unwrap();

    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--version"] if provider == "codex" => println!("codex-cli 1.2.3"),
        ["--version"] => println!("2.1.3"),
        ["login", "status"] if provider == "codex" && identity == "first" => {
            println!("Logged in using ChatGPT");
        }
        ["login", "status"] if provider == "codex" => println!("Not logged in"),
        ["auth", "status"] if provider == "claude" && identity == "first" => {
            println!(r#"{{"loggedIn":true}}"#);
        }
        ["auth", "status"] if provider == "claude" => {
            println!(r#"{{"loggedIn":false}}"#);
            std::process::exit(1);
        }
        _ if identity == "first" && provider == "codex" => {
            println!(r#"{{"type":"turn.completed"}}"#);
        }
        _ if identity == "first" && provider == "claude" => {
            println!(r#"{{"type":"result","subtype":"success"}}"#);
        }
        _ => {
            eprintln!("replacement executable must not run");
            std::process::exit(64);
        }
    }
}
"###,
    )
    .unwrap();
    let executable = root.join("fake-provider.exe");
    let status = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile local fake provider");
    executable
}
