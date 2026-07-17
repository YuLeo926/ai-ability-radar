use ability_core::{
    EnvironmentFingerprint, ManualRunService, PackLoader, RunMode, RunRepository, RunServiceError,
    RunStatus, TargetKind, TargetSelection,
};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

fn write_pack(root: &std::path::Path, target_kinds: &str, grader: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("one.txt"), "Only output the number 4").unwrap();
    fs::write(
        root.join("manifest.json"),
        format!(
            r#"{{
              "schema_version":1,
              "id":"manual-smoke",
              "version":"1.0.0",
              "title":"Manual Smoke",
              "target_kinds":{target_kinds},
              "tasks":[{{
                "id":"one",
                "category":"logic",
                "prompt_file":"one.txt",
                "starter_dir":null,
                "time_budget_secs":60,
                "max_turns":1,
                "grader":{grader}
              }}]
            }}"#
        ),
    )
    .unwrap();
}

fn environment(pack: &ability_core::LoadedPack) -> EnvironmentFingerprint {
    EnvironmentFingerprint {
        os_family: "windows".into(),
        os_version: "11".into(),
        app_version: "0.2.0".into(),
        cli_version: None,
        verifier_runtime_version: None,
        suite_id: pack.manifest.id.clone(),
        suite_version: pack.manifest.version.clone(),
        suite_content_sha256: pack.content_sha256.clone(),
        scoring_rule_version: "ability-v1".into(),
        resumed: false,
    }
}

fn chatgpt_target() -> TargetSelection {
    TargetSelection {
        kind: TargetKind::ChatGptClient,
        reported_model: "user-selected".into(),
        reasoning_effort: None,
    }
}

#[test]
fn manual_answers_checkpoint_and_complete_the_run() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_pack(
        &pack_dir,
        r#"["chat_gpt_client"]"#,
        r#"{"type":"exact_text","expected":"4"}"#,
    );
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));
    let run = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();

    let step = service.next_step(run.id).unwrap().unwrap();
    assert_eq!(step.task_id, "one");
    assert!(matches!(
        service.submit_answer(run.id, "one", &"x".repeat(256 * 1024 + 1)),
        Err(RunServiceError::AnswerTooLarge)
    ));
    assert!(repo.get_task_results(run.id).unwrap().is_empty());
    assert!(!dir.path().join("artifacts").join("runs").exists());

    service.submit_answer(run.id, "one", "4").unwrap();

    assert!(service.next_step(run.id).unwrap().is_none());
    let completed = repo.get_run(run.id).unwrap().unwrap();
    assert_eq!(completed.status, RunStatus::Completed);
    assert_eq!(completed.score.unwrap().ability_score, 100.0);
    let answer_path = dir
        .path()
        .join("artifacts")
        .join("runs")
        .join(run.id.to_string())
        .join("one.txt");
    assert_eq!(fs::read_to_string(answer_path).unwrap(), "4");
}

#[test]
fn start_rejects_a_client_not_supported_by_the_pack_before_persisting() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_pack(
        &pack_dir,
        r#"["claude_client"]"#,
        r#"{"type":"exact_text","expected":"4"}"#,
    );
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));

    assert!(matches!(
        service.start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack)
        ),
        Err(RunServiceError::UnsupportedTarget)
    ));
    assert!(repo.list_runs().unwrap().is_empty());
}

#[test]
fn start_rejects_a_mismatched_environment_or_external_verifier_before_persisting() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_pack(
        &pack_dir,
        r#"["chat_gpt_client"]"#,
        r#"{"type":"exact_text","expected":"4"}"#,
    );
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));
    let mut mismatched_environment = environment(&pack);
    mismatched_environment.suite_content_sha256 = "0".repeat(64);

    assert!(matches!(
        service.start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            mismatched_environment,
        ),
        Err(RunServiceError::EnvironmentMismatch)
    ));
    assert!(repo.list_runs().unwrap().is_empty());

    let external_pack_dir = dir.path().join("external-pack");
    write_pack(
        &external_pack_dir,
        r#"["chat_gpt_client"]"#,
        r#"{"type":"external_verifier","verifier_id":"approved-v1"}"#,
    );
    let external_pack = Arc::new(PackLoader::load(&external_pack_dir).unwrap());
    assert!(matches!(
        service.start(
            external_pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&external_pack),
        ),
        Err(RunServiceError::UnsupportedGrader { .. })
    ));
    assert!(repo.list_runs().unwrap().is_empty());
}

#[test]
fn submissions_must_follow_the_pack_order_and_cannot_follow_completion() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    fs::create_dir_all(&pack_dir).unwrap();
    fs::write(pack_dir.join("one.txt"), "one").unwrap();
    fs::write(pack_dir.join("two.txt"), "two").unwrap();
    fs::write(
        pack_dir.join("manifest.json"),
        r#"{
          "schema_version":1,"id":"two-step","version":"1.0.0","title":"Two Step",
          "target_kinds":["chat_gpt_client"],"tasks":[
            {"id":"one","category":"logic","prompt_file":"one.txt","starter_dir":null,"time_budget_secs":60,"max_turns":1,"grader":{"type":"exact_text","expected":"1"}},
            {"id":"two","category":"logic","prompt_file":"two.txt","starter_dir":null,"time_budget_secs":60,"max_turns":1,"grader":{"type":"exact_text","expected":"2"}}
          ]
        }"#,
    )
    .unwrap();
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));
    let run = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();

    assert!(matches!(
        service.submit_answer(run.id, "two", "2"),
        Err(RunServiceError::OutOfOrder)
    ));
    service.submit_answer(run.id, "one", "1").unwrap();
    service.submit_answer(run.id, "two", "2").unwrap();
    assert!(matches!(
        service.submit_answer(run.id, "two", "2"),
        Err(RunServiceError::RunNotFound(id)) if id == run.id
    ));
    assert_eq!(repo.get_task_results(run.id).unwrap().len(), 2);
}

#[cfg(windows)]
#[test]
fn artifact_root_cannot_be_reached_through_a_directory_junction() {
    use std::process::Command;

    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_pack(
        &pack_dir,
        r#"["chat_gpt_client"]"#,
        r#"{"type":"exact_text","expected":"4"}"#,
    );
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let outside = tempdir().unwrap();
    let junction = dir.path().join("artifact-junction");
    let status = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            junction.to_str().unwrap(),
            outside.path().to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let service = ManualRunService::new(repo.clone(), junction.join("artifacts"));
    let run = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();

    assert!(matches!(
        service.submit_answer(run.id, "one", "4"),
        Err(RunServiceError::UnsafeArtifactPath)
    ));
    assert!(repo.get_task_results(run.id).unwrap().is_empty());
    assert!(!outside.path().join("artifacts").exists());
}
