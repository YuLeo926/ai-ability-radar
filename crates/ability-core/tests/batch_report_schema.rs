use ability_core::{
    AdapterLaunchKind, BatchAnalysis, BatchExecutionSurface, BatchMemberStatus, BatchMode,
    BatchStatus, Category, DistributionSummary, ExecutionAdapterIdentity, ModelSource,
    ModelVerification, PackLoader, PublicBatchReport, RegressionSignal, ScanBatchMemberRecord,
    ScanBatchPlan, ScanBatchRecord, ScanBatchTarget, TargetBatchAnalysis, TargetKind,
    TargetSelection, build_batch_schedule, build_public_batch_report, validate_public_batch_report,
};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

fn instant() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 9, 12, 0, 0).single().unwrap()
}

fn fixture() -> (ScanBatchRecord, BatchAnalysis) {
    let pack = PackLoader::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmark-packs/cli-quick-v1"),
    )
    .unwrap();
    let make_target = |kind, model: &str, provider: &str, contract: &str| {
        ScanBatchTarget::new(
            TargetSelection {
                kind,
                reported_model: model.into(),
                reasoning_effort: Some("high".into()),
                model_source: ModelSource::CliRequested,
                model_verification: ModelVerification::UserConfirmed,
            },
            BatchExecutionSurface::AutomatedCli,
            ExecutionAdapterIdentity::new(
                BatchExecutionSurface::AutomatedCli,
                provider,
                AdapterLaunchKind::NativeExe,
                Some("1.2.3"),
                contract,
            )
            .unwrap(),
        )
        .unwrap()
    };
    let plan = ScanBatchPlan::new(
        &pack,
        "ability-v1",
        BatchMode::Full,
        42,
        vec![
            make_target(TargetKind::CodexCli, "gpt-5.6", "openai", "codex-cli-v1"),
            make_target(
                TargetKind::ClaudeCode,
                "claude-sonnet-4.6",
                "anthropic",
                "claude-code-v1",
            ),
        ],
        instant(),
    )
    .unwrap();
    let schedule = build_batch_schedule(&plan).unwrap();
    let members = schedule
        .members
        .iter()
        .map(|seed| ScanBatchMemberRecord {
            ordinal: seed.ordinal,
            target_position: seed.target_position,
            repetition_index: seed.repetition_index,
            run_id: Some(Uuid::new_v4()),
            status: BatchMemberStatus::Completed,
            failure_kind: None,
            attempt_number: 1,
            updated_at: instant(),
        })
        .collect::<Vec<_>>();
    let batch_id = Uuid::new_v4();
    let target_analysis = |target_position| TargetBatchAnalysis {
        target_position,
        signal: RegressionSignal::InsufficientData,
        candidate: Some(DistributionSummary {
            count: 5,
            median: 84.0,
            median_absolute_deviation: 2.0,
        }),
        baseline: None,
        baseline_batch_count: 0,
        baseline_utc_day_count: 0,
        candidate_member_count: 5,
        delta: None,
        absolute_drop: None,
        relative_drop: None,
        delta_confidence_interval: None,
        category_candidate: BTreeMap::from([(
            Category::Logic,
            DistributionSummary {
                count: 5,
                median: 84.0,
                median_absolute_deviation: 2.0,
            },
        )]),
        category_baseline: BTreeMap::new(),
        matched_task_deltas: Vec::new(),
        excluded_candidate_member_ordinals: Vec::new(),
    };
    (
        ScanBatchRecord {
            id: batch_id,
            plan,
            baseline_snapshot: None,
            status: BatchStatus::Completed,
            cancel_requested: false,
            planned_member_count: u32::try_from(members.len()).unwrap(),
            terminal_member_count: u32::try_from(members.len()).unwrap(),
            created_at: instant(),
            updated_at: instant(),
            members,
        },
        BatchAnalysis {
            candidate_batch_id: batch_id,
            analysis_version: 1,
            calibration_policy_version: 1,
            baseline_snapshot_sha256: None,
            signal: RegressionSignal::InsufficientData,
            targets: vec![target_analysis(0), target_analysis(1)],
        },
    )
}

fn schema() -> Value {
    serde_json::from_str(include_str!(
        "../../../schemas/public-batch-report.schema.json"
    ))
    .unwrap()
}

#[test]
fn exact_aggregate_export_round_trips_and_matches_schema() {
    let (batch, analysis) = fixture();
    let report = build_public_batch_report(&batch, &analysis).unwrap();
    let value = serde_json::to_value(&report).unwrap();
    let decoded: PublicBatchReport = serde_json::from_value(value.clone()).unwrap();
    validate_public_batch_report(&decoded).unwrap();
    assert_eq!(decoded, report);

    let schema = schema();
    jsonschema::meta::validate(&schema).unwrap();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();
    assert!(validator.is_valid(&value));
    assert_eq!(
        value["cohort"]["suiteContentSha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(value["analysis"]["targets"][0]["candidateMemberCount"], 5);
    assert!(value.to_string().find("rawAnswer").is_none());
    assert!(value.to_string().find("runId").is_none());
    assert!(value.to_string().find("batchId").is_none());
}

#[test]
fn schema_rejects_unknown_sensitive_and_malformed_fields() {
    let (batch, analysis) = fixture();
    let report = build_public_batch_report(&batch, &analysis).unwrap();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema())
        .unwrap();

    let mut raw = serde_json::to_value(&report).unwrap();
    raw["analysis"]["rawAnswer"] = json!("private answer");
    assert!(!validator.is_valid(&raw));

    let mut local = serde_json::to_value(&report).unwrap();
    local["targets"][0]["localExecutablePath"] = json!("C:\\Users\\private\\codex.exe");
    assert!(!validator.is_valid(&local));

    let mut bad_hash = serde_json::to_value(&report).unwrap();
    bad_hash["cohort"]["suiteContentSha256"] = json!("not-a-hash");
    assert!(!validator.is_valid(&bad_hash));
}

#[test]
fn aggregate_export_refuses_active_or_mismatched_batches() {
    let (mut batch, analysis) = fixture();
    batch.status = BatchStatus::Running;
    assert!(build_public_batch_report(&batch, &analysis).is_err());

    let (batch, mut analysis) = fixture();
    analysis.candidate_batch_id = Uuid::new_v4();
    assert!(build_public_batch_report(&batch, &analysis).is_err());
}
