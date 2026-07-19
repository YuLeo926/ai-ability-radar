use ability_adapters::{
    MAX_CAPTURE_BYTES_PER_STREAM, OutputStream, ProcessEnvironment, ProcessError, ProcessRunner,
    ProcessSpec, TokioProcessRunner,
};
use std::collections::BTreeMap;
#[cfg(windows)]
use std::sync::OnceLock;
use std::time::Duration;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

fn spec(script: impl Into<String>) -> ProcessSpec {
    ProcessSpec {
        program: "powershell".into(),
        args: vec!["-NoProfile".into(), "-Command".into(), script.into()],
        current_dir: tempdir().unwrap().keep(),
        env: BTreeMap::new(),
        environment: ProcessEnvironment::Inherit,
        timeout: Duration::from_secs(5),
    }
}

#[test]
fn provider_resolution_never_routes_user_arguments_through_a_shell() {
    let locator = include_str!("../src/command_locator.rs");
    let runner = include_str!("../src/process.rs");
    for source in [locator, runner] {
        assert!(!source.contains("cmd.exe"));
        assert!(!source.contains("powershell"));
        assert!(!source.contains(".bat"));
        assert!(!source.contains(".ps1"));
    }
    assert!(!runner.contains(".cmd"));
    assert!(runner.contains("resolve_launch_command"));
    assert_eq!(runner.matches("Command::new(").count(), 1);
    assert!(runner.contains("Command::new(&launch.program)"));
    assert!(runner.contains(".args(&launch.prefix_args)\n            .args(&spec.args)"));
    assert!(locator.contains("@openai/codex"));
    assert!(locator.contains("@anthropic-ai/claude-code"));
}

#[cfg(windows)]
const PROVIDER_PATH_CHILD_CASE: &str = "ABILITY_RADAR_PROVIDER_PATH_CHILD_CASE";
#[cfg(windows)]
const PROVIDER_FAKE_PATH: &str = "ABILITY_RADAR_PROVIDER_FAKE_PATH";
#[cfg(windows)]
const PROVIDER_FIXTURE_ROOT: &str = "ABILITY_RADAR_PROVIDER_FIXTURE_ROOT";

#[cfg(windows)]
#[test]
fn provider_resolution_uses_the_child_scoped_path_and_preserves_hostile_argv() {
    let fixture = FakeProviderFixture::new();
    let empty_parent_path = fixture.root.path().join("empty-parent-path");
    std::fs::create_dir_all(&empty_parent_path).unwrap();

    run_provider_path_child(
        "override",
        empty_parent_path.as_os_str(),
        &fixture.path,
        fixture.root.path(),
    );
}

#[cfg(windows)]
#[test]
fn explicitly_empty_child_path_hides_a_parent_provider() {
    let fixture = FakeProviderFixture::new();

    run_provider_path_child("empty", &fixture.path, &fixture.path, fixture.root.path());
}

#[cfg(windows)]
#[tokio::test]
async fn provider_path_child_case() {
    let Ok(case) = std::env::var(PROVIDER_PATH_CHILD_CASE) else {
        return;
    };
    let fake_path = std::env::var_os(PROVIDER_FAKE_PATH).unwrap();
    let fixture_root = std::path::PathBuf::from(std::env::var_os(PROVIDER_FIXTURE_ROOT).unwrap());
    let hostile_args = vec![
        "prompt=&;|<>$() \"quoted value\"".to_owned(),
        "--model".to_owned(),
        "model $(not-a-command)".to_owned(),
        "--workspace".to_owned(),
        fixture_root
            .join("workspace with spaces & metacharacters")
            .to_string_lossy()
            .into_owned(),
    ];
    let mut env = BTreeMap::new();
    env.insert(
        "pAtH".to_owned(),
        if case == "empty" {
            String::new()
        } else {
            fake_path.to_string_lossy().into_owned()
        },
    );
    let process = ProcessSpec {
        program: "codex".to_owned(),
        args: hostile_args.clone(),
        current_dir: fixture_root.clone(),
        env,
        environment: ProcessEnvironment::Inherit,
        timeout: Duration::from_secs(5),
    };

    if case == "empty" {
        let error = TokioProcessRunner
            .run(process, CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ProcessError::Spawn(ref source) if source.kind() == std::io::ErrorKind::NotFound
        ));
        return;
    }

    let output = TokioProcessRunner
        .run(process, CancellationToken::new())
        .await
        .unwrap();
    let reviewed_entry =
        std::fs::canonicalize(fixture_root.join("npm/node_modules/@openai/codex/bin/codex.js"))
            .unwrap()
            .to_string_lossy()
            .into_owned();
    let mut expected = vec![reviewed_entry];
    expected.extend(hostile_args);

    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.stdout.lines().collect::<Vec<_>>(), expected);
}

#[tokio::test]
async fn captures_stdout_stderr_exit_code_and_duration() {
    let output = TokioProcessRunner
        .run(
            spec("[Console]::Out.Write('ready'); [Console]::Error.Write('warning'); exit 7"),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(output.exit_code, Some(7));
    assert_eq!(output.stdout, "ready");
    assert_eq!(output.stderr, "warning");
    assert!(output.duration_ms < 5_000);
}

#[cfg(windows)]
struct FakeProviderFixture {
    root: tempfile::TempDir,
    path: std::ffi::OsString,
}

#[cfg(windows)]
impl FakeProviderFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let npm = root.path().join("npm");
        let node_bin = root.path().join("node-bin");
        let package_root = npm.join("node_modules/@openai/codex");
        std::fs::create_dir_all(package_root.join("bin")).unwrap();
        std::fs::create_dir_all(&node_bin).unwrap();
        std::fs::write(npm.join("codex.cmd"), "@echo off").unwrap();
        std::fs::write(package_root.join("bin/codex.js"), "fake provider entry").unwrap();
        std::fs::write(
            package_root.join("package.json"),
            r#"{"name":"@openai/codex","bin":{"codex":"bin/codex.js"}}"#,
        )
        .unwrap();

        let helper_source = root.path().join("fake-node.rs");
        std::fs::write(
            &helper_source,
            r#"
fn main() {
    for argument in std::env::args_os().skip(1) {
        println!("{}", argument.to_string_lossy());
    }
}
"#,
        )
        .unwrap();
        let helper = node_bin.join("node.exe");
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let compiled = std::process::Command::new(rustc)
            .arg(&helper_source)
            .arg("-o")
            .arg(&helper)
            .status()
            .unwrap();
        assert!(compiled.success(), "failed to compile fake node helper");

        Self {
            root,
            path: std::env::join_paths([&npm, &node_bin]).unwrap(),
        }
    }
}

#[cfg(windows)]
fn run_provider_path_child(
    case: &str,
    parent_path: &std::ffi::OsStr,
    fake_path: &std::ffi::OsStr,
    fixture_root: &std::path::Path,
) {
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("provider_path_child_case")
        .arg("--nocapture")
        .env("PATH", parent_path)
        .env(PROVIDER_PATH_CHILD_CASE, case)
        .env(PROVIDER_FAKE_PATH, fake_path)
        .env(PROVIDER_FIXTURE_ROOT, fixture_root)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "provider child case {case} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr),
    );
}

#[tokio::test]
async fn pre_cancelled_run_does_not_spawn_the_program() {
    let token = CancellationToken::new();
    token.cancel();
    let mut cancelled = spec("exit 0");
    cancelled.program = "definitely-not-a-program".into();

    assert!(matches!(
        TokioProcessRunner.run(cancelled, token).await,
        Err(ProcessError::Cancelled)
    ));
}

#[tokio::test]
async fn timeout_is_distinct_from_cancellation() {
    let mut command = spec("Start-Sleep -Seconds 30");
    command.timeout = Duration::from_millis(50);

    assert!(matches!(
        TokioProcessRunner
            .run(command, CancellationToken::new())
            .await,
        Err(ProcessError::TimedOut)
    ));
}

#[tokio::test]
async fn preserves_argument_boundaries_and_overlays_environment() {
    let directory = tempdir().unwrap();
    let script = directory.path().join("arguments.ps1");
    std::fs::write(
        &script,
        "[Console]::Out.Write(($args -join '|') + '|' + $env:ABILITY_RADAR_TEST_VALUE)",
    )
    .unwrap();
    let mut env = BTreeMap::new();
    env.insert("ABILITY_RADAR_TEST_VALUE".into(), "overlay value".into());
    let process = ProcessSpec {
        program: "powershell".into(),
        args: vec![
            "-NoProfile".into(),
            "-File".into(),
            script.into_os_string().into_string().unwrap(),
            "space value".into(),
            "&;|<>$()".into(),
        ],
        current_dir: directory.path().to_path_buf(),
        env,
        environment: ProcessEnvironment::Inherit,
        timeout: Duration::from_secs(5),
    };

    let output = TokioProcessRunner
        .run(process, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(output.stdout, "space value|&;|<>$()|overlay value");
}

#[cfg(windows)]
#[tokio::test]
async fn environment_policy_distinguishes_inherit_from_clear() {
    let environment_probe = || {
        let mut command = spec("exit 0");
        command.program = std::env::var("ComSpec").unwrap();
        command.args = vec![
            "/d".into(),
            "/c".into(),
            "if defined PATH (echo False) else (echo True)".into(),
        ];
        command
    };
    let inherited = TokioProcessRunner
        .run(environment_probe(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(inherited.stdout.trim(), "False");

    let mut cleared = environment_probe();
    cleared.environment = ProcessEnvironment::Clear;
    let cleared = TokioProcessRunner
        .run(cleared, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(cleared.stdout.trim(), "True");
}

#[test]
fn process_spec_debug_redacts_environment_values() {
    let secret = "ability-radar-secret-marker-9b6e";
    let mut process = spec("exit 0");
    process
        .env
        .insert("ABILITY_RADAR_SECRET".into(), secret.into());

    assert!(!format!("{process:?}").contains(secret));
}

#[tokio::test]
async fn captures_exactly_the_per_stream_limit() {
    let script = format!("[Console]::Out.Write('x' * {MAX_CAPTURE_BYTES_PER_STREAM})");
    let output = TokioProcessRunner
        .run(spec(script), CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(output.stdout.len(), MAX_CAPTURE_BYTES_PER_STREAM);
}

#[tokio::test]
async fn output_over_the_limit_terminates_with_a_truthful_error() {
    let script = format!(
        "[Console]::Error.Write('Z' * {})",
        MAX_CAPTURE_BYTES_PER_STREAM + 1
    );
    let error = TokioProcessRunner
        .run(spec(script), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ProcessError::OutputLimit {
            stream: OutputStream::Stderr
        }
    ));
    assert!(!error.to_string().contains('Z'));
}

#[tokio::test]
async fn drains_both_streams_without_pipe_deadlock() {
    let script = format!(
        "[Console]::Out.Write('o' * {}); [Console]::Error.Write('e' * {})",
        MAX_CAPTURE_BYTES_PER_STREAM / 2,
        MAX_CAPTURE_BYTES_PER_STREAM / 2,
    );
    let output = TokioProcessRunner
        .run(spec(script), CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(output.stdout.len(), MAX_CAPTURE_BYTES_PER_STREAM / 2);
    assert_eq!(output.stderr.len(), MAX_CAPTURE_BYTES_PER_STREAM / 2);
}

#[cfg(windows)]
#[tokio::test]
async fn unmanaged_positive_control_allows_the_grandchild_sentinel() {
    let _guard = process_tree_test_guard().await;
    let directory = tempdir().unwrap();
    let ready = directory.path().join("ready.txt");
    let sentinel = directory.path().join("orphan-sentinel.txt");
    let helper_pid = directory.path().join("helper.pid");
    let helper = write_delayed_sentinel_helper(
        directory.path(),
        &ready,
        &helper_pid,
        &sentinel,
        None,
        false,
        700,
    );
    let launcher = write_start_helper_launcher(directory.path(), &helper);
    let parent_pid = directory.path().join("parent.pid");
    let parent_exit_gate = directory.path().join("parent-exit.gate");
    let process = parent_exits_after_starting(&launcher, &ready, &parent_pid, &parent_exit_gate);
    let mut command = tokio::process::Command::new(&process.program);
    command.args(&process.args);
    let mut parent = command.spawn().unwrap();
    wait_for_path(&ready).await;
    wait_for_path(&parent_pid).await;
    let parent_handle = open_published_process(&parent_pid);
    let helper_handle = open_published_process(&helper_pid);
    std::fs::write(&parent_exit_gate, "release").unwrap();
    wait_for_process_signal(parent_handle).await;
    parent.wait().await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(
        sentinel.exists(),
        "the positive-control helper did not write"
    );
    wait_for_process_signal(helper_handle).await;
}

#[cfg(windows)]
#[tokio::test]
async fn cancellation_kills_a_ready_grandchild_after_the_parent_exits() {
    let _guard = process_tree_test_guard().await;
    let directory = tempdir().unwrap();
    let ready = directory.path().join("ready.txt");
    let sentinel = directory.path().join("orphan-sentinel.txt");
    let helper_pid = directory.path().join("helper.pid");
    let helper = write_delayed_sentinel_helper(
        directory.path(),
        &ready,
        &helper_pid,
        &sentinel,
        None,
        false,
        700,
    );
    let launcher = write_start_helper_launcher(directory.path(), &helper);
    let parent_pid = directory.path().join("parent.pid");
    let parent_exit_gate = directory.path().join("parent-exit.gate");
    let token = CancellationToken::new();
    let run = tokio::spawn({
        let token = token.clone();
        let ready_for_run = ready.clone();
        let parent_pid_for_run = parent_pid.clone();
        let parent_exit_gate_for_run = parent_exit_gate.clone();
        async move {
            TokioProcessRunner
                .run(
                    parent_exits_after_starting(
                        &launcher,
                        &ready_for_run,
                        &parent_pid_for_run,
                        &parent_exit_gate_for_run,
                    ),
                    token,
                )
                .await
        }
    });
    wait_for_path(&ready).await;
    wait_for_path(&parent_pid).await;
    let parent_handle = open_published_process(&parent_pid);
    let helper_handle = open_published_process(&helper_pid);
    std::fs::write(&parent_exit_gate, "release").unwrap();
    wait_for_process_signal(parent_handle).await;
    assert!(
        !run.is_finished(),
        "runner returned before supervised helper ended"
    );
    token.cancel();

    assert!(matches!(run.await.unwrap(), Err(ProcessError::Cancelled)));
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(!sentinel.exists(), "a descendant survived cancellation");
    wait_for_process_signal(helper_handle).await;
}

#[cfg(windows)]
#[tokio::test]
async fn timeout_kills_a_ready_grandchild_after_the_parent_exits() {
    let _guard = process_tree_test_guard().await;
    let directory = tempdir().unwrap();
    let ready = directory.path().join("ready.txt");
    let sentinel = directory.path().join("orphan-sentinel.txt");
    let helper_pid = directory.path().join("helper.pid");
    let helper = write_delayed_sentinel_helper(
        directory.path(),
        &ready,
        &helper_pid,
        &sentinel,
        None,
        false,
        8_000,
    );
    let launcher = write_start_helper_launcher(directory.path(), &helper);
    let parent_pid = directory.path().join("parent.pid");
    let parent_exit_gate = directory.path().join("parent-exit.gate");
    let mut process =
        parent_exits_after_starting(&launcher, &ready, &parent_pid, &parent_exit_gate);
    // `wait_for_path` twice plus `wait_for_process_signal` are each bounded at
    // two seconds. Keep the runner timeout above their six-second worst case.
    process.timeout = Duration::from_secs(7);
    let helper_ready_deadline = Duration::from_millis(8_200);
    let run = tokio::spawn(async move {
        TokioProcessRunner
            .run(process, CancellationToken::new())
            .await
    });
    wait_for_path(&ready).await;
    let helper_ready_at = tokio::time::Instant::now();
    wait_for_path(&parent_pid).await;
    let parent_handle = open_published_process(&parent_pid);
    let helper_handle = open_published_process(&helper_pid);
    std::fs::write(&parent_exit_gate, "release").unwrap();
    wait_for_process_signal(parent_handle).await;
    assert!(
        !run.is_finished(),
        "runner timed out before the verified direct-parent exit"
    );
    assert_eq!(
        unsafe { WaitForSingleObject(helper_handle, 0) },
        WAIT_TIMEOUT,
        "helper exited before the runner's natural timeout"
    );
    assert!(matches!(run.await.unwrap(), Err(ProcessError::TimedOut)));
    tokio::time::sleep(helper_ready_deadline.saturating_sub(helper_ready_at.elapsed())).await;
    assert!(!sentinel.exists(), "a descendant survived timeout");
    wait_for_process_signal(helper_handle).await;
}

#[cfg(windows)]
#[tokio::test]
async fn output_limit_kills_a_ready_grandchild_after_the_parent_exits() {
    let _guard = process_tree_test_guard().await;
    let directory = tempdir().unwrap();
    let ready = directory.path().join("ready.txt");
    let sentinel = directory.path().join("orphan-sentinel.txt");
    let helper_pid = directory.path().join("helper.pid");
    let output_gate = directory.path().join("output.gate");
    let helper = write_delayed_sentinel_helper(
        directory.path(),
        &ready,
        &helper_pid,
        &sentinel,
        Some(&output_gate),
        true,
        700,
    );
    let launcher = write_start_helper_launcher(directory.path(), &helper);
    let parent_pid = directory.path().join("parent.pid");
    let parent_exit_gate = directory.path().join("parent-exit.gate");
    let script = parent_exits_after_starting(&launcher, &ready, &parent_pid, &parent_exit_gate);
    let run = tokio::spawn(async move {
        TokioProcessRunner
            .run(script, CancellationToken::new())
            .await
    });
    wait_for_path(&ready).await;
    wait_for_path(&parent_pid).await;
    let parent_handle = open_published_process(&parent_pid);
    let helper_handle = open_published_process(&helper_pid);
    std::fs::write(&parent_exit_gate, "release").unwrap();
    wait_for_process_signal(parent_handle).await;
    std::fs::write(&output_gate, "release").unwrap();
    assert!(matches!(
        run.await.unwrap(),
        Err(ProcessError::OutputLimit {
            stream: OutputStream::Stderr
        })
    ));
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(!sentinel.exists(), "a descendant survived output cleanup");
    wait_for_process_signal(helper_handle).await;
}

#[cfg(windows)]
fn write_delayed_sentinel_helper(
    directory: &std::path::Path,
    ready: &std::path::Path,
    helper_pid: &std::path::Path,
    sentinel: &std::path::Path,
    output_gate: Option<&std::path::Path>,
    writes_overflow: bool,
    delay_ms: u64,
) -> std::path::PathBuf {
    let helper = directory.join("delayed-sentinel.ps1");
    let ready_text = ready.to_string_lossy().replace('\'', "''");
    let helper_pid_text = helper_pid.to_string_lossy().replace('\'', "''");
    let sentinel_text = sentinel.to_string_lossy().replace('\'', "''");
    let gate_wait = output_gate.map_or_else(String::new, |gate| {
        format!(
            "; while (!(Test-Path -LiteralPath '{}')) {{ Start-Sleep -Milliseconds 10 }}",
            gate.to_string_lossy().replace('\'', "''")
        )
    });
    let overflow = if writes_overflow {
        format!(
            "; [Console]::Error.Write('Z' * {})",
            MAX_CAPTURE_BYTES_PER_STREAM + 1
        )
    } else {
        String::new()
    };
    std::fs::write(
        &helper,
        format!(
            "Set-Content -LiteralPath '{helper_pid_text}' -Value $PID; Set-Content -LiteralPath '{ready_text}' -Value ready{gate_wait}{overflow}; Start-Sleep -Milliseconds {delay_ms}; Set-Content -LiteralPath '{sentinel_text}' -Value orphan"
        ),
    )
    .unwrap();
    helper
}

#[cfg(windows)]
fn parent_exits_after_starting(
    launcher: &std::path::Path,
    ready: &std::path::Path,
    parent_pid: &std::path::Path,
    parent_exit_gate: &std::path::Path,
) -> ProcessSpec {
    let launcher_text = launcher.to_string_lossy().replace('\'', "''");
    let ready_text = ready.to_string_lossy().replace('\'', "''");
    let parent_pid_text = parent_pid.to_string_lossy().replace('\'', "''");
    let gate_text = parent_exit_gate.to_string_lossy().replace('\'', "''");
    spec(format!(
        "& cmd.exe /d /c '{launcher_text}'; while (!(Test-Path -LiteralPath '{ready_text}')) {{ Start-Sleep -Milliseconds 10 }}; Set-Content -LiteralPath '{parent_pid_text}' -Value $PID; while (!(Test-Path -LiteralPath '{gate_text}')) {{ Start-Sleep -Milliseconds 10 }}; exit 0"
    ))
}

#[cfg(windows)]
fn write_start_helper_launcher(
    directory: &std::path::Path,
    helper: &std::path::Path,
) -> std::path::PathBuf {
    let launcher = directory.join("start-helper.cmd");
    std::fs::write(
        &launcher,
        format!(
            "@echo off\r\nstart \"\" /b powershell.exe -NoProfile -File \"{}\"\r\n",
            helper.display()
        ),
    )
    .unwrap();
    launcher
}

#[cfg(windows)]
async fn wait_for_path(path: &std::path::Path) {
    for _ in 0..80 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for helper readiness: {}", path.display());
}

#[cfg(windows)]
fn open_published_process(pid_path: &std::path::Path) -> HANDLE {
    let pid = std::fs::read_to_string(pid_path)
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!handle.is_null(), "could not open published process {pid}");
    handle
}

#[cfg(windows)]
async fn wait_for_process_signal(handle: HANDLE) {
    for _ in 0..80 {
        if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
            unsafe { CloseHandle(handle) };
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    unsafe { CloseHandle(handle) };
    panic!("timed out waiting for published process to exit");
}

#[cfg(windows)]
async fn process_tree_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    static GUARD: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    GUARD
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}
