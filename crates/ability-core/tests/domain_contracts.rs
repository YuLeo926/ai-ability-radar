use ability_core::{
    Category, EnvironmentFingerprint, RunMode, RunRecord, RunStatus, TargetKind, TargetSelection,
};

#[test]
fn target_kind_serializes_as_stable_snake_case() {
    let json = serde_json::to_string(&TargetKind::ClaudeCode).unwrap();
    assert_eq!(json, "\"claude_code\"");
}

#[test]
fn a_new_run_starts_created_with_zero_progress() {
    let run = RunRecord::new(
        TargetSelection {
            kind: TargetKind::CodexCli,
            reported_model: "current CLI selection".into(),
            reasoning_effort: Some("high".into()),
        },
        RunMode::Quick,
        "cli-quick".into(),
        "1.0.0".into(),
        2,
        EnvironmentFingerprint {
            os_family: "windows".into(),
            os_version: "11".into(),
            app_version: "0.2.0".into(),
            cli_version: Some("codex 1.2.3".into()),
            verifier_runtime_version: Some("node v22.0.0".into()),
            suite_id: "cli-quick".into(),
            suite_version: "1.0.0".into(),
            suite_content_sha256: "a".repeat(64),
            scoring_rule_version: "ability-v1".into(),
            resumed: false,
        },
    );

    assert_eq!(run.status, RunStatus::Created);
    assert_eq!(run.completed_tasks, 0);
    assert_eq!(run.total_tasks, 2);
    assert!(run.score.is_none());
    assert_eq!(Category::CliCoding.to_string(), "cli_coding");
}
