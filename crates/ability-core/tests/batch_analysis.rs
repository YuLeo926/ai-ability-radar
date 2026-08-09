use ability_core::{
    AcceptedProvenanceClass, AdapterLaunchKind, AnalysisError, BaselineEvidenceCandidate,
    BaselineExclusionReason, BaselineSnapshot, BatchExecutionSurface, BatchMemberSeed,
    BatchMemberStatus, BatchMode, BatchStatus, CalibrationPolicy, Category, CompletedBatchEvidence,
    ExecutionAdapterIdentity, MemberEvidence, ModelSource, ModelVerification, PackLoader,
    RegressionSignal, RunRepository, RunStatus, ScanBatchPlan, ScanBatchTarget, ScoreSummary,
    TargetKind, TargetSelection, TaskEvidence, TaskOutcome, analyze_matched_batch,
    build_batch_schedule, distribution,
};
use chrono::{Duration, TimeZone, Utc};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tempfile::tempdir;
use uuid::Uuid;

fn instant(day: u32, hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, day, hour, 0, 0)
        .single()
        .unwrap()
}

fn full_plan() -> ScanBatchPlan {
    let pack = PackLoader::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmark-packs/cli-quick-v1"),
    )
    .unwrap();
    let target = |kind: TargetKind, model: &str, provider: &str| {
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
                match kind {
                    TargetKind::CodexCli => "codex-cli-v1",
                    TargetKind::ClaudeCode => "claude-code-v1",
                    _ => unreachable!(),
                },
            )
            .unwrap(),
        )
        .unwrap()
    };
    ScanBatchPlan::new(
        &pack,
        "ability-v1",
        BatchMode::Full,
        17,
        vec![
            target(TargetKind::CodexCli, "gpt-5.6", "openai"),
            target(TargetKind::ClaudeCode, "claude-sonnet-4.6", "anthropic"),
        ],
        instant(20, 10),
    )
    .unwrap()
}

fn reviewed_policy() -> CalibrationPolicy {
    let mut policy = CalibrationPolicy::production_v1();
    policy.likely_regression_enabled = true;
    policy.bootstrap_resamples = 500;
    policy
}

fn baseline_candidate(
    plan: &ScanBatchPlan,
    batch_id: Uuid,
    finished_at: chrono::DateTime<Utc>,
) -> BaselineEvidenceCandidate {
    BaselineEvidenceCandidate {
        batch_id,
        mode: BatchMode::Full,
        status: BatchStatus::Completed,
        finished_at,
        identity: ability_core::BatchAnalysisIdentity::from_plan(plan).unwrap(),
        has_valid_snapshot: true,
    }
}

fn snapshot(
    plan: &ScanBatchPlan,
    candidate_id: Uuid,
    ids: &[(Uuid, chrono::DateTime<Utc>)],
    policy: &CalibrationPolicy,
) -> BaselineSnapshot {
    let evidence = ids
        .iter()
        .map(|(id, finished)| baseline_candidate(plan, *id, *finished))
        .collect::<Vec<_>>();
    BaselineSnapshot::freeze(candidate_id, plan, instant(20, 10), policy, &evidence).unwrap()
}

fn member(ordinal: u32, target_position: u32, ability: f64, task_count: usize) -> MemberEvidence {
    let mut category_scores = BTreeMap::new();
    category_scores.insert(Category::Logic, ability);
    MemberEvidence {
        member_ordinal: ordinal,
        target_position,
        status: BatchMemberStatus::Completed,
        run_status: Some(RunStatus::Completed),
        score: Some(ScoreSummary {
            ability_score: ability,
            passed_tasks: u32::try_from(task_count).unwrap(),
            valid_tasks: u32::try_from(task_count).unwrap(),
            total_tasks: u32::try_from(task_count).unwrap(),
            category_scores,
        }),
        task_results: (0..task_count)
            .map(|index| TaskEvidence {
                task_id: format!("logic-{index}"),
                category: Category::Logic,
                outcome: TaskOutcome::Passed,
                score: Some(ability),
                failure_kind: None,
            })
            .collect(),
        isolation_complete: true,
    }
}

fn history(
    ids: &[(Uuid, chrono::DateTime<Utc>)],
    score: f64,
    task_count: usize,
) -> Vec<CompletedBatchEvidence> {
    ids.iter()
        .map(|(batch_id, finished_at)| CompletedBatchEvidence {
            batch_id: *batch_id,
            finished_at: *finished_at,
            members: (0..5)
                .map(|ordinal| member(ordinal, 0, score, task_count))
                .collect(),
        })
        .collect()
}

#[test]
fn category_medians_and_median_absolute_deviation_are_deterministic() {
    let summary = distribution(&[1.0, 3.0, 5.0, 100.0]).unwrap().unwrap();
    assert_eq!(summary.count, 4);
    assert_eq!(summary.median, 4.0);
    assert_eq!(summary.median_absolute_deviation, 2.0);
}

#[test]
fn candidate_and_later_evidence_are_excluded() {
    let plan = full_plan();
    let policy = reviewed_policy();
    let candidate = Uuid::new_v4();
    let later = Uuid::new_v4();
    let duplicate = Uuid::new_v4();
    let eligible = Uuid::new_v4();
    let evidence = vec![
        baseline_candidate(&plan, candidate, instant(19, 10)),
        baseline_candidate(&plan, later, instant(20, 10)),
        baseline_candidate(&plan, duplicate, instant(18, 10)),
        baseline_candidate(&plan, duplicate, instant(17, 10)),
        baseline_candidate(&plan, eligible, instant(16, 10)),
    ];
    let frozen =
        BaselineSnapshot::freeze(candidate, &plan, instant(20, 10), &policy, &evidence).unwrap();
    assert_eq!(frozen.selected_batch_ids, vec![eligible]);
    assert!(frozen.exclusions.iter().any(|entry| {
        entry.batch_id == candidate && entry.reason == BaselineExclusionReason::CandidateBatch
    }));
    assert!(frozen.exclusions.iter().any(|entry| {
        entry.batch_id == later && entry.reason == BaselineExclusionReason::NotStrictlyBeforeCutoff
    }));
    assert!(frozen.exclusions.iter().any(|entry| {
        entry.batch_id == duplicate && entry.reason == BaselineExclusionReason::DuplicateEvidenceId
    }));
}

#[test]
fn snapshot_selects_latest_per_day_then_twelve_most_recent_days() {
    let plan = full_plan();
    let policy = reviewed_policy();
    let candidate = Uuid::new_v4();
    let same_day_old = Uuid::new_v4();
    let same_day_new = Uuid::new_v4();
    let mut evidence = vec![
        baseline_candidate(&plan, same_day_old, instant(19, 1)),
        baseline_candidate(&plan, same_day_new, instant(19, 2)),
    ];
    for offset in 2..=14 {
        evidence.push(baseline_candidate(
            &plan,
            Uuid::from_u128(u128::from(u32::try_from(offset).unwrap())),
            instant(20, 10) - Duration::days(i64::from(offset)),
        ));
    }
    let first =
        BaselineSnapshot::freeze(candidate, &plan, instant(20, 10), &policy, &evidence).unwrap();
    evidence.reverse();
    let second =
        BaselineSnapshot::freeze(candidate, &plan, instant(20, 10), &policy, &evidence).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.selected_batch_ids.len(), 12);
    assert!(first.selected_batch_ids.contains(&same_day_new));
    assert!(!first.selected_batch_ids.contains(&same_day_old));
}

#[test]
fn every_matched_identity_dimension_is_fail_closed() {
    let plan = full_plan();
    let policy = reviewed_policy();
    let candidate = Uuid::new_v4();
    let base = ability_core::BatchAnalysisIdentity::from_plan(&plan).unwrap();
    let mut variants = Vec::new();

    let mut content = base.clone();
    content.suite_content_sha256 = "e".repeat(64);
    variants.push(content);
    let mut scoring = base.clone();
    scoring.scoring_rule_version = "ability-v2".into();
    variants.push(scoring);
    let mut analysis = base.clone();
    analysis.analysis_version += 1;
    variants.push(analysis);
    let mut surface = base.clone();
    surface.execution_surface = BatchExecutionSurface::GuidedClient;
    variants.push(surface);
    let mut route = base.clone();
    route.targets[0].route_identity.model_or_route = "gpt-5.6-other".into();
    variants.push(route);
    let mut provenance = base.clone();
    provenance.targets[0].provenance_class = AcceptedProvenanceClass::CliDefaultUnverified;
    variants.push(provenance);
    let mut adapter = base;
    adapter.targets[0].adapter_contract_version = "codex-cli-v2".into();
    variants.push(adapter);

    let evidence = variants
        .into_iter()
        .enumerate()
        .map(|(index, identity)| BaselineEvidenceCandidate {
            batch_id: Uuid::from_u128(100 + u128::try_from(index).unwrap()),
            mode: BatchMode::Full,
            status: BatchStatus::Completed,
            finished_at: instant(19, 9) - Duration::days(i64::try_from(index).unwrap()),
            identity,
            has_valid_snapshot: true,
        })
        .collect::<Vec<_>>();
    let frozen =
        BaselineSnapshot::freeze(candidate, &plan, instant(20, 10), &policy, &evidence).unwrap();
    assert!(frozen.selected_batch_ids.is_empty());
    assert_eq!(frozen.exclusions.len(), 7);
    assert!(
        frozen
            .exclusions
            .iter()
            .all(|entry| { entry.reason == BaselineExclusionReason::IncompatibleIdentity })
    );
}

#[test]
fn snapshot_digest_freezes_ids_exclusions_cutoff_and_policy() {
    let plan = full_plan();
    let policy = reviewed_policy();
    let candidate = Uuid::new_v4();
    let evidence_id = Uuid::new_v4();
    let frozen = snapshot(&plan, candidate, &[(evidence_id, instant(19, 9))], &policy);
    frozen.validate().unwrap();
    for mutation in 0..4 {
        let mut changed = frozen.clone();
        match mutation {
            0 => changed.baseline_as_of += Duration::seconds(1),
            1 => changed.selected_batch_ids.clear(),
            2 => changed.bootstrap_resamples += 1,
            3 => changed.calibration_policy_version += 1,
            _ => unreachable!(),
        }
        assert_eq!(
            changed.validate().unwrap_err(),
            AnalysisError::InvalidSnapshot
        );
    }
}

#[test]
fn bootstrap_resamples_runs_and_batches_not_tasks() {
    let plan = full_plan();
    let policy = reviewed_policy();
    let candidate_id = Uuid::new_v4();
    let ids = (0..5)
        .map(|offset| {
            (
                Uuid::new_v4(),
                instant(19 - u32::try_from(offset).unwrap(), 9),
            )
        })
        .collect::<Vec<_>>();
    let frozen = snapshot(&plan, candidate_id, &ids, &policy);
    let candidate_one_task = (0..5)
        .map(|ordinal| member(ordinal, 0, 80.0, 1))
        .collect::<Vec<_>>();
    let candidate_twenty_tasks = (0..5)
        .map(|ordinal| member(ordinal, 0, 80.0, 20))
        .collect::<Vec<_>>();
    let one = analyze_matched_batch(
        BatchMode::Full,
        candidate_id,
        &candidate_one_task,
        Some(&frozen),
        &history(&ids, 90.0, 1),
        &policy,
    )
    .unwrap();
    let twenty = analyze_matched_batch(
        BatchMode::Full,
        candidate_id,
        &candidate_twenty_tasks,
        Some(&frozen),
        &history(&ids, 90.0, 20),
        &policy,
    )
    .unwrap();
    assert_eq!(one.targets[0].candidate_member_count, 5);
    assert_eq!(twenty.targets[0].candidate_member_count, 5);
    assert_eq!(one.targets[0].baseline_batch_count, 5);
    assert_eq!(twenty.targets[0].baseline_batch_count, 5);
    assert_eq!(
        one.targets[0].delta_confidence_interval,
        twenty.targets[0].delta_confidence_interval
    );
}

#[test]
fn partial_member_is_not_regression_evidence() {
    let plan = full_plan();
    let policy = reviewed_policy();
    let candidate_id = Uuid::new_v4();
    let ids = (0..5)
        .map(|offset| (Uuid::new_v4(), instant(19 - offset, 9)))
        .collect::<Vec<_>>();
    let frozen = snapshot(&plan, candidate_id, &ids, &policy);
    let mut members = (0..5)
        .map(|ordinal| member(ordinal, 0, 80.0, 2))
        .collect::<Vec<_>>();
    members[4].task_results.pop();
    let analysis = analyze_matched_batch(
        BatchMode::Full,
        candidate_id,
        &members,
        Some(&frozen),
        &history(&ids, 90.0, 2),
        &policy,
    )
    .unwrap();
    assert_eq!(analysis.targets[0].candidate_member_count, 4);
    assert_eq!(
        analysis.targets[0].excluded_candidate_member_ordinals,
        vec![4]
    );
    assert_eq!(analysis.targets[0].signal, RegressionSignal::Watch);
}

#[test]
fn full_sufficient_evidence_requires_absolute_relative_and_ci_gates() {
    let plan = full_plan();
    let policy = reviewed_policy();
    let candidate_id = Uuid::new_v4();
    let ids = (0..5)
        .map(|offset| (Uuid::new_v4(), instant(19 - offset, 9)))
        .collect::<Vec<_>>();
    let frozen = snapshot(&plan, candidate_id, &ids, &policy);
    let candidate = (0..5)
        .map(|ordinal| member(ordinal, 0, 80.0, 2))
        .collect::<Vec<_>>();
    let analysis = analyze_matched_batch(
        BatchMode::Full,
        candidate_id,
        &candidate,
        Some(&frozen),
        &history(&ids, 90.0, 2),
        &policy,
    )
    .unwrap();
    assert_eq!(
        analysis.targets[0].signal,
        RegressionSignal::LikelyRegression
    );
    assert_eq!(analysis.targets[0].delta, Some(-10.0));
    assert_eq!(analysis.targets[0].absolute_drop, Some(10.0));
    assert!(analysis.targets[0].relative_drop.unwrap() > 0.11);

    let mut absolute_only = policy.clone();
    absolute_only.tolerated_relative_drop = 0.2;
    let analysis = analyze_matched_batch(
        BatchMode::Full,
        candidate_id,
        &candidate,
        Some(&frozen),
        &history(&ids, 90.0, 2),
        &absolute_only,
    )
    .unwrap();
    assert_eq!(analysis.targets[0].signal, RegressionSignal::Watch);
}

#[test]
fn non_positive_baseline_cannot_pass_the_relative_gate() {
    let plan = full_plan();
    let policy = reviewed_policy();
    let candidate_id = Uuid::new_v4();
    let ids = (0..5)
        .map(|offset| (Uuid::new_v4(), instant(19 - offset, 9)))
        .collect::<Vec<_>>();
    let frozen = snapshot(&plan, candidate_id, &ids, &policy);
    let candidate = (0..5)
        .map(|ordinal| member(ordinal, 0, 0.0, 2))
        .collect::<Vec<_>>();
    let analysis = analyze_matched_batch(
        BatchMode::Full,
        candidate_id,
        &candidate,
        Some(&frozen),
        &history(&ids, 0.0, 2),
        &policy,
    )
    .unwrap();
    assert_eq!(analysis.targets[0].relative_drop, None);
    assert_ne!(
        analysis.targets[0].signal,
        RegressionSignal::LikelyRegression
    );
}

#[test]
fn quick_and_standard_never_emit_regression_signals() {
    for mode in [BatchMode::QuickComparison, BatchMode::Standard] {
        let analysis =
            analyze_matched_batch(mode, Uuid::new_v4(), &[], None, &[], &reviewed_policy())
                .unwrap();
        assert_eq!(analysis.signal, RegressionSignal::InsufficientData);
        assert!(analysis.baseline_snapshot_sha256.is_none());
    }
}

#[test]
fn malformed_non_finite_evidence_fails_closed() {
    assert_eq!(
        distribution(&[1.0, f64::NAN]).unwrap_err(),
        AnalysisError::MalformedEvidence
    );
    assert_eq!(
        distribution(&[f64::INFINITY]).unwrap_err(),
        AnalysisError::MalformedEvidence
    );
}

#[test]
fn legacy_unknown_has_no_accepted_provenance_class() {
    assert_eq!(
        AcceptedProvenanceClass::from_plan_target(
            ModelSource::LegacyUnknown,
            ModelVerification::LegacyUnknown,
            BatchExecutionSurface::AutomatedCli,
        ),
        None
    );
}

#[test]
fn full_creation_freezes_baseline_atomically() {
    let plan = full_plan();
    let policy = CalibrationPolicy::production_v1();
    let candidate_id = Uuid::new_v4();
    let directory = tempdir().unwrap();
    let database_path = directory.path().join("analysis.sqlite3");
    let repository = RunRepository::open(&database_path).unwrap();
    let pack = PackLoader::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmark-packs/cli-quick-v1"),
    )
    .unwrap();
    let members = build_batch_schedule(&plan)
        .unwrap()
        .members
        .into_iter()
        .map(|member| BatchMemberSeed {
            ordinal: member.ordinal,
            target_position: member.target_position,
            repetition_index: member.repetition_index,
        })
        .collect::<Vec<_>>();
    let frozen = repository
        .create_full_batch_with_baseline_snapshot(
            candidate_id,
            &pack,
            &plan,
            &members,
            instant(20, 10),
            &policy,
        )
        .unwrap();
    frozen.validate().unwrap();
    assert_eq!(frozen.candidate_batch_id, candidate_id);
    assert_eq!(frozen.baseline_as_of, instant(20, 10));
    assert_eq!(frozen.content_sha256.len(), 64);
    assert_eq!(
        repository.get_baseline_snapshot(candidate_id).unwrap(),
        Some(frozen)
    );
    assert!(repository.get_batch(candidate_id).unwrap().is_some());

    let rejected_id = Uuid::new_v4();
    assert!(
        repository
            .create_full_batch_with_baseline_snapshot(
                rejected_id,
                &pack,
                &plan,
                &members,
                instant(20, 10),
                &policy,
            )
            .is_err()
    );
    assert!(repository.get_batch(rejected_id).unwrap().is_none());
    assert!(
        repository
            .get_baseline_snapshot(rejected_id)
            .unwrap()
            .is_none()
    );

    let forged_id = Uuid::new_v4();
    assert!(
        repository
            .insert_batch_plan(forged_id, &pack, &plan, &members, instant(20, 10))
            .is_err()
    );
    assert!(repository.get_batch(forged_id).unwrap().is_none());

    Connection::open(&database_path)
        .unwrap()
        .execute(
            "UPDATE baseline_snapshots SET content_sha256=?2 WHERE candidate_batch_id=?1",
            [candidate_id.to_string(), "d".repeat(64)],
        )
        .unwrap();
    assert!(repository.get_batch(candidate_id).is_err());
}
