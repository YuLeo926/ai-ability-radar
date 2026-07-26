use ability_core::{
    Category, EnvironmentFingerprint, FailureKind, ModelSource, ModelVerification, RunMode,
    RunRecord, RunStatus, ScoreSummary, TargetKind, TargetSelection, TaskOutcome, TaskResult,
    build_public_report,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn fixture_report() -> Value {
    fixture_report_with_provenance(ModelSource::LegacyUnknown, ModelVerification::LegacyUnknown)
}

fn fixture_report_with_provenance(
    model_source: ModelSource,
    model_verification: ModelVerification,
) -> Value {
    let mut run = RunRecord::new(
        TargetSelection {
            kind: TargetKind::ChatGptClient,
            reported_model: "GPT-X".into(),
            reasoning_effort: None,
            model_source,
            model_verification,
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
            execution_adapter_identity: None,
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
fn public_report_schema_is_v2_and_requires_provenance() {
    let schema = schema();
    let target_required = schema["properties"]["target"]["required"]
        .as_array()
        .unwrap();
    let report = fixture_report();

    assert_eq!(schema["title"], "AI Ability Radar public report v2");
    assert_eq!(schema["properties"]["schemaVersion"]["const"], 2);
    assert!(target_required.contains(&json!("modelSource")));
    assert!(target_required.contains(&json!("modelVerification")));
    assert_eq!(report["schemaVersion"], 2);
    assert_eq!(report["target"]["modelSource"], "legacy_unknown");
    assert_eq!(report["target"]["modelVerification"], "legacy_unknown");
}

#[test]
fn schema_accepts_only_the_provenance_matrix() {
    let schema = schema();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();
    let sources = [
        "manual",
        "windows_accessibility",
        "cli_requested",
        "cli_reported",
        "default_route",
        "legacy_unknown",
    ];
    let verifications = [
        "user_confirmed",
        "provider_reported",
        "unverified",
        "legacy_unknown",
    ];
    let valid_pairs = [
        ("manual", "user_confirmed"),
        ("windows_accessibility", "user_confirmed"),
        ("cli_requested", "user_confirmed"),
        ("cli_reported", "provider_reported"),
        ("default_route", "unverified"),
        ("legacy_unknown", "legacy_unknown"),
    ];

    for source in sources {
        for verification in verifications {
            let mut report = fixture_report();
            report["target"]["modelSource"] = json!(source);
            report["target"]["modelVerification"] = json!(verification);
            assert_eq!(
                validator.is_valid(&report),
                valid_pairs.contains(&(source, verification)),
                "schema matrix mismatch for {source}/{verification}"
            );
        }
    }

    let mut missing_source = fixture_report();
    missing_source["target"]
        .as_object_mut()
        .unwrap()
        .remove("modelSource");
    assert!(!validator.is_valid(&missing_source));

    let mut missing_verification = fixture_report();
    missing_verification["target"]
        .as_object_mut()
        .unwrap()
        .remove("modelVerification");
    assert!(!validator.is_valid(&missing_verification));
}

#[test]
fn every_rust_provenance_pair_uses_exact_wire_values_and_matches_the_schema() {
    let schema = schema();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();
    let valid_pairs = [
        (
            ModelSource::Manual,
            ModelVerification::UserConfirmed,
            "manual",
            "user_confirmed",
        ),
        (
            ModelSource::WindowsAccessibility,
            ModelVerification::UserConfirmed,
            "windows_accessibility",
            "user_confirmed",
        ),
        (
            ModelSource::CliRequested,
            ModelVerification::UserConfirmed,
            "cli_requested",
            "user_confirmed",
        ),
        (
            ModelSource::CliReported,
            ModelVerification::ProviderReported,
            "cli_reported",
            "provider_reported",
        ),
        (
            ModelSource::DefaultRoute,
            ModelVerification::Unverified,
            "default_route",
            "unverified",
        ),
        (
            ModelSource::LegacyUnknown,
            ModelVerification::LegacyUnknown,
            "legacy_unknown",
            "legacy_unknown",
        ),
    ];

    for (source, verification, source_wire, verification_wire) in valid_pairs {
        let report = fixture_report_with_provenance(source, verification);
        assert_eq!(
            report["target"]["modelSource"], source_wire,
            "Rust source wire value mismatch for {source:?}/{verification:?}"
        );
        assert_eq!(
            report["target"]["modelVerification"], verification_wire,
            "Rust verification wire value mismatch for {source:?}/{verification:?}"
        );
        assert!(
            validator.is_valid(&report),
            "committed schema rejected {source_wire}/{verification_wire}"
        );
    }
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

    let mut with_unknown_target_field = fixture_report();
    with_unknown_target_field["target"]["providerModel"] = json!("private provider detail");
    assert!(!validator.is_valid(&with_unknown_target_field));

    let mut with_unknown_source = fixture_report();
    with_unknown_source["target"]["modelSource"] = json!("future_source");
    assert!(!validator.is_valid(&with_unknown_source));

    let mut with_unknown_verification = fixture_report();
    with_unknown_verification["target"]["modelVerification"] = json!("future_verification");
    assert!(!validator.is_valid(&with_unknown_verification));

    let mut with_illegal_pair = fixture_report();
    with_illegal_pair["target"]["modelSource"] = json!("manual");
    with_illegal_pair["target"]["modelVerification"] = json!("provider_reported");
    assert!(!validator.is_valid(&with_illegal_pair));

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
