#![cfg(windows)]

use ability_adapters::{
    AgentAdapter, AuthState, ClaudeCodeAdapter, CliRunService, CodexAdapter, NodeVerifier,
    ProcessRunner, TokioProcessRunner, WorkspaceVerifier,
};
use ability_core::{
    EnvironmentFingerprint, LoadedPack, ModelSource, ModelVerification, PackLoader, RunMode,
    RunRepository, RunStatus, TargetKind, TargetSelection, TaskOutcome,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn require_opt_in() -> bool {
    if std::env::var("ABILITY_RADAR_FAKE_CLI_E2E").as_deref() == Ok("1") {
        true
    } else {
        eprintln!("set ABILITY_RADAR_FAKE_CLI_E2E=1 to run this ignored test");
        false
    }
}

fn assert_program(program: &str) {
    let output = Command::new(program)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("{program} is missing from PATH: {error}"));
    assert!(
        output.status.success(),
        "{program} --version failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    if program != "node" {
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "ability-radar-fake-cli 0.1.0\n",
            "{program} must be the first-party fake executable"
        );
    }
}

fn source_pack_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../benchmark-packs/cli-quick-v1")
        .canonicalize()
        .expect("bundled CLI quick pack")
}

fn environment(pack: &LoadedPack, cli_version: String) -> EnvironmentFingerprint {
    EnvironmentFingerprint {
        os_family: "Windows".into(),
        os_version: "fake-cli-e2e".into(),
        app_version: "0.2.0-test".into(),
        cli_version: Some(cli_version),
        verifier_runtime_version: Some("node from PATH".into()),
        suite_id: pack.manifest.id.clone(),
        suite_version: pack.manifest.version.clone(),
        suite_content_sha256: pack.content_sha256.clone(),
        scoring_rule_version: "ability-v1".into(),
        execution_adapter_identity: None,
        resumed: false,
    }
}

async fn execute_complete_run(
    service: &CliRunService,
    repository: &RunRepository,
    pack: Arc<LoadedPack>,
    adapter: Arc<dyn AgentAdapter>,
    verifier: Arc<dyn WorkspaceVerifier>,
) {
    let availability = adapter.detect().await;
    assert!(availability.installed);
    assert_eq!(availability.auth_state, AuthState::Ready);
    let version = availability.version.expect("fake CLI version");
    let run = service
        .prepare(
            pack.clone(),
            TargetSelection {
                kind: adapter.kind(),
                reported_model: "deterministic fake".into(),
                reasoning_effort: None,
                model_source: ModelSource::LegacyUnknown,
                model_verification: ModelVerification::LegacyUnknown,
            },
            RunMode::Quick,
            environment(&pack, version),
        )
        .unwrap();
    let (events, _receiver) = mpsc::unbounded_channel();
    service
        .execute(
            run.id,
            pack,
            adapter,
            verifier,
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap();

    let stored = repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Completed);
    let score = stored.score.expect("completed score");
    assert_eq!(score.passed_tasks, 2);
    assert_eq!(score.valid_tasks, 2);
    assert_eq!(score.total_tasks, 2);
    assert_eq!(score.ability_score, 100.0);
}

fn process_is_running(pid: u32) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .expect("query Windows process table");
    tasklist_reports_running(
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        pid,
    )
    .expect("interpret Windows process query")
}

fn tasklist_reports_running(query_succeeded: bool, stdout: &str, pid: u32) -> Result<bool, String> {
    if !query_succeeded {
        return Err("tasklist process query failed".into());
    }
    Ok(stdout.contains(&format!("\"{pid}\"")))
}

#[test]
fn failed_tasklist_query_cannot_prove_process_termination() {
    assert!(tasklist_reports_running(false, "", 4242).is_err());
}

#[test]
fn tasklist_query_detects_a_present_pid() {
    let stdout = "\"codex.exe\",\"4242\",\"Console\",\"1\",\"1,024 K\"\r\n";
    assert_eq!(tasklist_reports_running(true, stdout, 4242), Ok(true));
}

#[test]
fn successful_tasklist_query_accepts_an_absent_pid() {
    let stdout = "INFO: No tasks are running which match the specified criteria.\r\n";
    assert_eq!(tasklist_reports_running(true, stdout, 4242), Ok(false));
}

async fn execute_cancelled_run(
    service: &CliRunService,
    repository: &RunRepository,
    artifact_root: &Path,
    pack: Arc<LoadedPack>,
    adapter: Arc<dyn AgentAdapter>,
    verifier: Arc<dyn WorkspaceVerifier>,
    pid_file: &Path,
) {
    let availability = adapter.detect().await;
    assert!(availability.installed);
    assert_eq!(availability.auth_state, AuthState::Ready);
    let run = service
        .prepare(
            pack.clone(),
            TargetSelection {
                kind: TargetKind::CodexCli,
                reported_model: "delayed deterministic fake".into(),
                reasoning_effort: None,
                model_source: ModelSource::LegacyUnknown,
                model_verification: ModelVerification::LegacyUnknown,
            },
            RunMode::Quick,
            environment(&pack, "fake delayed".into()),
        )
        .unwrap();
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        trigger.cancel();
    });
    let (events, _receiver) = mpsc::unbounded_channel();
    let delay_marker = artifact_root.join(".ability-radar-fake-delay-ms");
    fs::write(&delay_marker, "10000").unwrap();
    unsafe {
        std::env::set_var("ABILITY_RADAR_FAKE_PID_FILE", pid_file);
    }
    let result = service
        .execute(run.id, pack, adapter, verifier, cancellation, events)
        .await;
    unsafe {
        std::env::remove_var("ABILITY_RADAR_FAKE_PID_FILE");
    }
    fs::remove_file(&delay_marker).unwrap();
    cancel_task.await.unwrap();
    result.unwrap();

    let pid = fs::read_to_string(pid_file)
        .expect("delayed fake process wrote its PID")
        .trim()
        .parse::<u32>()
        .expect("fake process PID");
    assert!(!process_is_running(pid), "fake CLI process {pid} leaked");

    let stored = repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Cancelled);
    assert!(stored.score.is_none());
    let results = repository.get_task_results(run.id).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].task_id, "dedupe-events");
    assert_eq!(results[0].outcome, TaskOutcome::Cancelled);
    assert!(results[0].score.is_none());
    let workspaces = artifact_root
        .join("runs")
        .join(run.id.to_string())
        .join("workspaces");
    assert!(workspaces.join("dedupe-events").is_dir());
    assert!(!workspaces.join("retry-schedule").exists());
    assert!(!delay_marker.exists());
}

fn assert_tree_is_contained(root: &Path) {
    let canonical_root = root.canonicalize().expect("artifact root");
    let mut pending = vec![canonical_root.clone()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let metadata = fs::symlink_metadata(entry.path()).unwrap();
            assert!(!metadata.file_type().is_symlink());
            let canonical = entry.path().canonicalize().unwrap();
            assert!(
                canonical.starts_with(&canonical_root),
                "{} escaped {}",
                canonical.display(),
                canonical_root.display(),
            );
            if metadata.is_dir() {
                pending.push(canonical);
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires fake codex.exe/claude.exe plus real node.exe on PATH"]
async fn bundled_pack_passes_both_adapters_and_cancellation_is_safe() {
    if !require_opt_in() {
        return;
    }
    assert_program("codex");
    assert_program("claude");
    assert_program("node");

    let temporary = tempdir().unwrap();
    let artifact_root = temporary.path().join("artifacts");
    fs::create_dir_all(&artifact_root).unwrap();
    let repository = Arc::new(RunRepository::open(&temporary.path().join("runs.sqlite")).unwrap());
    let pack = Arc::new(PackLoader::load(&source_pack_root()).unwrap());
    assert_eq!(pack.tasks.len(), 2);
    let runner: Arc<dyn ProcessRunner> = Arc::new(TokioProcessRunner);
    let verifier: Arc<dyn WorkspaceVerifier> =
        Arc::new(NodeVerifier::new(runner.clone(), source_pack_root()));
    let service = CliRunService::new(repository.clone(), artifact_root.clone());

    execute_complete_run(
        &service,
        &repository,
        pack.clone(),
        Arc::new(CodexAdapter::new(runner.clone())),
        verifier.clone(),
    )
    .await;
    execute_complete_run(
        &service,
        &repository,
        pack.clone(),
        Arc::new(ClaudeCodeAdapter::new(runner.clone())),
        verifier.clone(),
    )
    .await;
    let pid_file = temporary.path().join("delayed-fake.pid");
    execute_cancelled_run(
        &service,
        &repository,
        &artifact_root,
        pack,
        Arc::new(CodexAdapter::new(runner)),
        verifier,
        &pid_file,
    )
    .await;
    assert_tree_is_contained(&artifact_root);
}
