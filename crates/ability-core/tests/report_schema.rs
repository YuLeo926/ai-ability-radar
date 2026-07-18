use ability_core::{
    Category, EnvironmentFingerprint, FailureKind, RunMode, RunRecord, RunStatus, ScoreSummary,
    TargetKind, TargetSelection, TaskOutcome, TaskResult, build_public_report,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn fixture_report() -> Value {
    let mut run = RunRecord::new(
        TargetSelection {
            kind: TargetKind::ChatGptClient,
            reported_model: "GPT-X".into(),
            reasoning_effort: None,
        },
        RunMode::Quick,
        "client-quick".into(),
        "1.0.0".into(),
        2,
        EnvironmentFingerprint {
            os_family: "Windows".into(),
            os_version: "11".into(),
            app_version: "0.2.0".into(),
            cli_version: None,
            verifier_runtime_version: Some("v22.0.0".into()),
            suite_id: "client-quick".into(),
            suite_version: "1.0.0".into(),
            suite_content_sha256: "a".repeat(64),
            scoring_rule_version: "ability-v1".into(),
            resumed: false,
        },
    );
    run.status = RunStatus::Completed;
    run.completed_tasks = 2;
    run.finished_at = Some(run.started_at);
    run.score = Some(ScoreSummary {
        ability_score: 50.0,
        passed_tasks: 1,
        valid_tasks: 2,
        total_tasks: 2,
        category_scores: BTreeMap::from([(Category::Logic, 50.0)]),
    });
    let tasks = [
        TaskResult {
            run_id: run.id,
            task_id: "one".into(),
            category: Category::Logic,
            outcome: TaskOutcome::Passed,
            score: Some(100.0),
            failure_kind: None,
            duration_ms: 10,
            answer_rel_path: None,
            detail: "private".into(),
        },
        TaskResult {
            run_id: run.id,
            task_id: "two".into(),
            category: Category::Logic,
            outcome: TaskOutcome::Failed,
            score: Some(0.0),
            failure_kind: Some(FailureKind::WrongAnswer),
            duration_ms: 20,
            answer_rel_path: None,
            detail: "private".into(),
        },
    ];
    serde_json::to_value(build_public_report(&run, &tasks).unwrap()).unwrap()
}

fn schema() -> Value {
    serde_json::from_str(include_str!("../../../schemas/public-report.schema.json")).unwrap()
}

#[test]
fn public_report_matches_the_committed_schema() {
    let schema = schema();
    jsonschema::meta::validate(&schema).unwrap();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();

    assert!(validator.is_valid(&fixture_report()));
}

#[test]
fn schema_rejects_unknown_sensitive_fields_and_invalid_formats() {
    let schema = schema();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();

    let mut with_raw_answer = fixture_report();
    with_raw_answer["result"]["rawAnswer"] = json!("private answer");
    assert!(!validator.is_valid(&with_raw_answer));

    let mut with_local_identity = fixture_report();
    with_local_identity["environment"]["osVersion"] = json!("11 Pro 22631");
    assert!(!validator.is_valid(&with_local_identity));

    let mut invalid_identity = fixture_report();
    invalid_identity["reportId"] = json!("not-a-uuid");
    invalid_identity["generatedAt"] = json!("17 July");
    assert!(!validator.is_valid(&invalid_identity));
}

#[test]
fn schema_rejects_invalid_scores_hashes_counts_and_statuses() {
    let schema = schema();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();

    for (pointer, value) in [
        ("/result/abilityScore", json!(101)),
        ("/result/passedTasks", json!(-1)),
        ("/result/runStatus", json!("running")),
        ("/environment/suiteContentSha256", json!("not-a-hash")),
        ("/methodology/interpretationStatus", json!("degraded")),
    ] {
        let mut report = fixture_report();
        *report.pointer_mut(pointer).unwrap() = value;
        assert!(!validator.is_valid(&report), "{pointer} should be rejected");
    }
}
