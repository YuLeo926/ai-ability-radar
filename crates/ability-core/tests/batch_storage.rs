use ability_core::{
    BatchExecutionSurface, BatchMemberSeed, BatchMemberStatus, BatchMode, BatchStatus, Category,
    EnvironmentFingerprint, ExecutionAdapterIdentity, FailureKind, IsolationAttestation,
    IsolationEnforcement, ModelSource, ModelVerification, PackLoader, RunMode, RunRecord,
    RunRepository, RunStatus, ScanBatchPlan, ScanBatchTarget, ScanExecutionAuthorization,
    TargetKind, TargetSelection, TaskOutcome, TaskResult, summarize_scores,
};
use chrono::{Duration, TimeZone, Utc};
use rusqlite::{Connection, params};
use std::path::PathBuf;
use tempfile::tempdir;
use uuid::Uuid;

fn pack_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("benchmark-packs")
        .join(name)
}

fn guided_target(kind: TargetKind, model: &str) -> ScanBatchTarget {
    let provider = match kind {
        TargetKind::ChatGptClient => "openai",
        TargetKind::ClaudeClient => "anthropic",
        _ => panic!("guided target kind required"),
    };
    ScanBatchTarget::new(
        TargetSelection {
            kind,
            reported_model: model.into(),
            reasoning_effort: Some("high".into()),
            model_source: ModelSource::Manual,
            model_verification: ModelVerification::UserConfirmed,
        },
        BatchExecutionSurface::GuidedClient,
        ExecutionAdapterIdentity::new(
            BatchExecutionSurface::GuidedClient,
            provider,
            ability_core::AdapterLaunchKind::GuidedClient,
            Some("desktop 1.0"),
            "guided-v1",
        )
        .unwrap(),
    )
    .unwrap()
}

fn sample_plan() -> (ability_core::LoadedPack, ScanBatchPlan) {
    sample_plan_with_seed(17)
}

fn sample_plan_with_seed(seed: u64) -> (ability_core::LoadedPack, ScanBatchPlan) {
    let pack = PackLoader::load(&pack_path("client-quick-v1")).unwrap();
    let issued_at = Utc.with_ymd_and_hms(2026, 7, 28, 1, 0, 0).unwrap();
    let plan = ScanBatchPlan::new(
        &pack,
        "ability-v1",
        BatchMode::QuickComparison,
        seed,
        vec![
            guided_target(TargetKind::ChatGptClient, "GPT-5.6"),
            guided_target(TargetKind::ClaudeClient, "Claude Sonnet 4.5"),
        ],
        issued_at,
    )
    .unwrap();
    (pack, plan)
}

fn schedule() -> Vec<BatchMemberSeed> {
    vec![
        BatchMemberSeed {
            ordinal: 0,
            target_position: 0,
            repetition_index: 0,
        },
        BatchMemberSeed {
            ordinal: 1,
            target_position: 1,
            repetition_index: 0,
        },
    ]
}

fn initial_authorization(batch_id: Uuid, plan: &ScanBatchPlan) -> ScanExecutionAuthorization {
    let created_at = plan.cost_estimate.issued_at;
    ScanExecutionAuthorization {
        batch_id,
        member_ordinal: None,
        attempt_number: 1,
        max_provider_turns: plan.cost_estimate.max_provider_turns,
        max_task_budget_secs: plan.cost_estimate.summed_task_budget_secs,
        acknowledgement_hash: plan.acknowledgement_hash.clone(),
        allowed_failure_kind: None,
        expires_at: created_at + Duration::hours(4),
        created_at,
    }
}

fn run_for(plan: &ScanBatchPlan, position: usize, id: Uuid) -> RunRecord {
    let target = &plan.targets[position];
    RunRecord {
        id,
        target: target.target.clone(),
        mode: RunMode::Quick,
        suite_id: plan.suite_id.clone(),
        suite_version: plan.suite_version.clone(),
        status: RunStatus::Created,
        started_at: plan.cost_estimate.issued_at,
        finished_at: None,
        total_tasks: u32::try_from(plan.sealed_task_budgets.len()).unwrap(),
        completed_tasks: 0,
        environment: EnvironmentFingerprint {
            os_family: "windows".into(),
            os_version: "11".into(),
            app_version: "0.2.2".into(),
            cli_version: None,
            verifier_runtime_version: None,
            suite_id: plan.suite_id.clone(),
            suite_version: plan.suite_version.clone(),
            suite_content_sha256: plan.suite_content_sha256.clone(),
            scoring_rule_version: plan.scoring_rule_version.clone(),
            execution_adapter_identity: Some(target.execution_adapter_identity.clone()),
            resumed: false,
        },
        score: None,
    }
}

fn create_batch(repository: &RunRepository) -> (Uuid, ScanBatchPlan) {
    let (pack, plan) = sample_plan();
    let batch_id = Uuid::new_v4();
    repository
        .insert_batch_plan(
            batch_id,
            &pack,
            &plan,
            &schedule(),
            plan.cost_estimate.issued_at,
        )
        .unwrap();
    repository
        .append_execution_authorization(&initial_authorization(batch_id, &plan))
        .unwrap();
    (batch_id, plan)
}

fn passing_result(run_id: Uuid, task_id: &str) -> TaskResult {
    let category = match task_id {
        "logic-schedule" | "logic-truth" | "logic-capacity" => Category::Logic,
        "review-python" | "review-typescript" => Category::CodeReview,
        "dedupe-events" | "retry-schedule" => Category::CliCoding,
        _ => Category::InstructionFollowing,
    };
    TaskResult {
        run_id,
        task_id: task_id.into(),
        category,
        outcome: TaskOutcome::Passed,
        score: Some(100.0),
        failure_kind: None,
        duration_ms: 250,
        answer_rel_path: Some("runs/member/answers/task.txt".into()),
        detail: "exact_json:pass".into(),
    }
}

fn attestation(plan: &ScanBatchPlan, minutes: i64) -> IsolationAttestation {
    IsolationAttestation {
        policy_version: 1,
        enforcement: IsolationEnforcement::UserAttested,
        user_attested: true,
        recorded_at: plan.cost_estimate.issued_at + Duration::minutes(minutes),
    }
}

fn complete_guided_member(
    repository: &RunRepository,
    batch_id: Uuid,
    plan: &ScanBatchPlan,
    ordinal: u32,
) -> RunRecord {
    let run = reserve_and_start(repository, batch_id, plan, ordinal);
    let task_ids = [
        "instruction-filter",
        "instruction-csv",
        "instruction-inventory",
        "logic-schedule",
        "logic-truth",
        "logic-capacity",
        "review-python",
        "review-typescript",
    ];
    for (index, task_id) in task_ids.iter().enumerate() {
        repository
            .save_guided_task_result_with_isolation(
                batch_id,
                ordinal,
                &passing_result(run.id, task_id),
                &attestation(plan, 2 + i64::try_from(index).unwrap()),
            )
            .unwrap();
    }
    let results = repository.get_task_results(run.id).unwrap();
    let score = summarize_scores(&results, 8).unwrap();
    repository.complete_run(run.id, Some(&score)).unwrap();
    repository
        .finish_batch_member(
            batch_id,
            ordinal,
            run.id,
            BatchMemberStatus::Completed,
            None,
            plan.cost_estimate.issued_at + Duration::minutes(12),
        )
        .unwrap();
    repository.get_run(run.id).unwrap().unwrap()
}

fn reserve_and_start(
    repository: &RunRepository,
    batch_id: Uuid,
    plan: &ScanBatchPlan,
    ordinal: u32,
) -> RunRecord {
    let run = run_for(plan, ordinal as usize, Uuid::new_v4());
    let reservation = repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(1),
            &run,
        )
        .unwrap()
        .unwrap();
    assert_eq!(reservation.member.ordinal, ordinal);
    repository
        .mark_member_launching(batch_id, ordinal, run.id, plan.cost_estimate.issued_at)
        .unwrap();
    repository
        .mark_member_running(batch_id, ordinal, run.id, plan.cost_estimate.issued_at)
        .unwrap();
    repository.get_run(run.id).unwrap().unwrap()
}

#[test]
fn migration_upgrades_v2_without_rewriting_old_run_json() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_init.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0002_settings.sql"))
        .unwrap();
    let legacy_target =
        r#"{"kind":"chat_gpt_client","reportedModel":"GPT-X","reasoningEffort":null}"#;
    let legacy_environment = r#"{"osFamily":"windows","osVersion":"11","appVersion":"0.2.1","cliVersion":null,"verifierRuntimeVersion":null,"suiteId":"client-quick","suiteVersion":"1.0.0","suiteContentSha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","scoringRuleVersion":"ability-v1","resumed":false}"#;
    connection
        .execute(
            "INSERT INTO targets(target_json) VALUES (?1)",
            [legacy_target],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO suite_versions(suite_id,suite_version,content_sha256,scoring_rule_version) VALUES ('client-quick','1.0.0',?1,'ability-v1')",
            ["b".repeat(64)],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO runs(id,target_json,mode_json,suite_id,suite_version,status_json,started_at,finished_at,total_tasks,completed_tasks,environment_json,score_json) VALUES (?1,?2,'\"quick\"','client-quick','1.0.0','\"created\"','2026-07-28T01:00:00Z',NULL,8,0,?3,NULL)",
            params![Uuid::new_v4().to_string(), legacy_target, legacy_environment],
        )
        .unwrap();
    drop(connection);

    let _repository = RunRepository::open(&database).unwrap();
    let connection = Connection::open(&database).unwrap();
    let stored: String = connection
        .query_row("SELECT environment_json FROM runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, legacy_environment);
    assert_eq!(
        connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        4
    );
}

#[test]
fn reserves_member_and_run_atomically() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = run_for(&plan, 0, Uuid::new_v4());

    let reservation = repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(1),
            &run,
        )
        .unwrap()
        .unwrap();

    assert_eq!(reservation.member.ordinal, 0);
    assert_eq!(reservation.member.status, BatchMemberStatus::Reserved);
    assert_eq!(reservation.member.run_id, Some(run.id));
    assert_eq!(repository.get_run(run.id).unwrap().unwrap(), run);
    assert!(repository.has_running_runs().unwrap());
    let stored = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(stored.members[0], reservation.member);
    assert_eq!(stored.members[1].status, BatchMemberStatus::Planned);

    let duplicate = run_for(&plan, 1, run.id);
    assert!(
        repository
            .reserve_next_runnable_member_and_run(
                batch_id,
                plan.cost_estimate.issued_at + Duration::minutes(2),
                &duplicate,
            )
            .is_err()
    );
    assert_eq!(
        repository.get_batch(batch_id).unwrap().unwrap().members[1].run_id,
        None
    );
}

#[test]
fn guided_result_and_attestation_are_atomic() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = reserve_and_start(&repository, batch_id, &plan, 0);
    let result = passing_result(run.id, "instruction-filter");
    let attestation = IsolationAttestation {
        policy_version: 1,
        enforcement: IsolationEnforcement::UserAttested,
        user_attested: true,
        recorded_at: plan.cost_estimate.issued_at + Duration::minutes(2),
    };

    repository
        .save_guided_task_result_with_isolation(batch_id, 0, &result, &attestation)
        .unwrap();
    assert_eq!(
        repository.get_task_results(run.id).unwrap(),
        vec![result.clone()]
    );

    let injector = Connection::open(&database).unwrap();
    injector
        .execute_batch(
            "CREATE TRIGGER fail_batch_task_result BEFORE INSERT ON task_results
             BEGIN SELECT RAISE(ABORT, 'injected result failure'); END;",
        )
        .unwrap();
    let second = passing_result(run.id, "instruction-csv");
    assert!(
        repository
            .save_guided_task_result_with_isolation(batch_id, 0, &second, &attestation)
            .is_err()
    );
    assert_eq!(repository.get_task_results(run.id).unwrap(), vec![result]);
    assert_eq!(
        injector
            .query_row(
                "SELECT COUNT(*) FROM scan_batch_task_isolation WHERE run_id=?1 AND task_id='instruction-csv'",
                [run.id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn reconciles_ambiguous_launch_without_replay() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = run_for(&plan, 0, Uuid::new_v4());
    repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(1),
            &run,
        )
        .unwrap();
    repository
        .mark_member_launching(batch_id, 0, run.id, plan.cost_estimate.issued_at)
        .unwrap();

    assert_eq!(
        repository
            .reconcile_batches_after_startup(plan.cost_estimate.issued_at + Duration::minutes(3))
            .unwrap(),
        1
    );
    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.members[0].status, BatchMemberStatus::Deferred);
    assert_eq!(batch.members[0].run_id, Some(run.id));
    assert_eq!(
        batch.members[0].failure_kind,
        Some(FailureKind::AppInterrupted)
    );
    assert_eq!(
        repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );

    let replacement = run_for(&plan, 1, Uuid::new_v4());
    let next = repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(4),
            &replacement,
        )
        .unwrap()
        .unwrap();
    assert_eq!(next.member.ordinal, 1);
    assert_eq!(batch.members[0].run_id, Some(run.id));
}

#[test]
fn startup_with_only_expired_and_ambiguous_work_persists_interrupted() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = run_for(&plan, 0, Uuid::new_v4());
    repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(1),
            &run,
        )
        .unwrap();
    repository
        .mark_member_launching(batch_id, 0, run.id, plan.cost_estimate.issued_at)
        .unwrap();
    repository
        .reconcile_batches_after_startup(
            plan.cost_estimate.issued_at + Duration::hours(4) + Duration::seconds(1),
        )
        .unwrap();

    assert_eq!(
        repository.get_batch(batch_id).unwrap().unwrap().status,
        BatchStatus::Interrupted
    );
    assert!(
        repository
            .reserve_next_runnable_member_and_run(
                batch_id,
                plan.cost_estimate.issued_at + Duration::hours(4) + Duration::seconds(2),
                &run_for(&plan, 1, Uuid::new_v4()),
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn reserved_crash_reuses_preallocated_run_and_never_inserts_a_replacement() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = run_for(&plan, 0, Uuid::new_v4());
    repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(1),
            &run,
        )
        .unwrap();
    repository
        .reconcile_batches_after_startup(plan.cost_estimate.issued_at + Duration::minutes(2))
        .unwrap();
    let reconciled = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(reconciled.members[0].status, BatchMemberStatus::Planned);
    assert_eq!(reconciled.members[0].run_id, Some(run.id));

    let same = run_for(&plan, 0, run.id);
    let reservation = repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(3),
            &same,
        )
        .unwrap()
        .unwrap();
    assert_eq!(reservation.member.run_id, Some(run.id));
    assert_eq!(repository.list_runs().unwrap().len(), 1);
}

#[test]
fn running_crash_is_deferred_on_the_same_run_without_completion() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = reserve_and_start(&repository, batch_id, &plan, 0);

    repository
        .reconcile_batches_after_startup(plan.cost_estimate.issued_at + Duration::minutes(3))
        .unwrap();
    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.members[0].status, BatchMemberStatus::Deferred);
    assert_eq!(batch.members[0].run_id, Some(run.id));
    assert_eq!(
        repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
    assert!(repository.get_task_results(run.id).unwrap().is_empty());
}

#[test]
fn startup_reconciliation_preserves_the_exact_durable_retry_failure() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = reserve_and_start(&repository, batch_id, &plan, 0);
    let mut marker = passing_result(run.id, "instruction-filter");
    marker.outcome = TaskOutcome::Invalid;
    marker.score = None;
    marker.failure_kind = Some(FailureKind::Network);
    marker.detail = "synthetic network failure".into();
    repository
        .save_guided_task_result_with_isolation(batch_id, 0, &marker, &attestation(&plan, 2))
        .unwrap();

    repository
        .reconcile_batches_after_startup(plan.cost_estimate.issued_at + Duration::minutes(3))
        .unwrap();

    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.members[0].status, BatchMemberStatus::Deferred);
    assert_eq!(batch.members[0].run_id, Some(run.id));
    assert_eq!(batch.members[0].failure_kind, Some(FailureKind::Network));
    assert_eq!(repository.get_task_results(run.id).unwrap(), vec![marker]);
}

#[test]
fn batch_status_is_derived_from_members() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let first = reserve_and_start(&repository, batch_id, &plan, 0);
    repository
        .defer_batch_member(
            batch_id,
            0,
            Some(first.id),
            FailureKind::Network,
            plan.cost_estimate.issued_at + Duration::minutes(2),
        )
        .unwrap();
    assert_eq!(
        repository
            .derive_batch_status(
                batch_id,
                plan.cost_estimate.issued_at + Duration::minutes(2)
            )
            .unwrap(),
        BatchStatus::Running
    );

    let second = reserve_and_start(&repository, batch_id, &plan, 1);
    repository
        .defer_batch_member(
            batch_id,
            1,
            Some(second.id),
            FailureKind::QuotaExhausted,
            plan.cost_estimate.issued_at + Duration::minutes(3),
        )
        .unwrap();
    assert_eq!(
        repository
            .derive_batch_status(
                batch_id,
                plan.cost_estimate.issued_at + Duration::minutes(3)
            )
            .unwrap(),
        BatchStatus::Paused
    );
}

#[test]
fn invalid_schedules_transitions_and_ownership_leave_rows_unchanged() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (pack, plan) = sample_plan();
    for invalid in [
        vec![schedule()[0].clone(), schedule()[0].clone()],
        vec![
            BatchMemberSeed {
                ordinal: 1,
                ..schedule()[0].clone()
            },
            schedule()[1].clone(),
        ],
        vec![
            schedule()[0].clone(),
            BatchMemberSeed {
                repetition_index: 1,
                ..schedule()[1].clone()
            },
        ],
    ] {
        assert!(
            repository
                .insert_batch_plan(
                    Uuid::new_v4(),
                    &pack,
                    &plan,
                    &invalid,
                    plan.cost_estimate.issued_at
                )
                .is_err()
        );
    }
    assert!(repository.list_batches().unwrap().is_empty());

    let (batch_id, plan) = create_batch(&repository);
    let run = run_for(&plan, 0, Uuid::new_v4());
    assert!(
        repository
            .mark_member_launching(batch_id, 0, run.id, plan.cost_estimate.issued_at)
            .is_err()
    );
    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.members[0].status, BatchMemberStatus::Planned);
    assert_eq!(batch.members[0].run_id, None);
}

#[test]
fn reservation_rejects_target_suite_provenance_and_adapter_mismatches_atomically() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let mut cases = Vec::new();

    let mut wrong_target = run_for(&plan, 0, Uuid::new_v4());
    wrong_target.target.reported_model = "forged-model".into();
    cases.push(wrong_target);
    let mut wrong_hash = run_for(&plan, 0, Uuid::new_v4());
    wrong_hash.environment.suite_content_sha256 = "0".repeat(64);
    cases.push(wrong_hash);
    let mut missing_adapter = run_for(&plan, 0, Uuid::new_v4());
    missing_adapter.environment.execution_adapter_identity = None;
    cases.push(missing_adapter);
    let mut wrong_provenance = run_for(&plan, 0, Uuid::new_v4());
    wrong_provenance.target.model_verification = ModelVerification::Unverified;
    cases.push(wrong_provenance);

    for run in cases {
        assert!(
            repository
                .reserve_next_runnable_member_and_run(
                    batch_id,
                    plan.cost_estimate.issued_at + Duration::minutes(1),
                    &run,
                )
                .is_err()
        );
        assert!(repository.get_run(run.id).unwrap().is_none());
        assert_eq!(
            repository.get_batch(batch_id).unwrap().unwrap().members[0].run_id,
            None
        );
    }
}

#[test]
fn cancellation_and_foreign_keys_do_not_orphan_owned_runs() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = run_for(&plan, 0, Uuid::new_v4());
    repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(1),
            &run,
        )
        .unwrap();
    assert!(repository.delete_batch(batch_id).is_err());

    repository
        .cancel_batch(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(2),
        )
        .unwrap();
    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.status, BatchStatus::Cancelled);
    assert_eq!(batch.members[0].status, BatchMemberStatus::Cancelled);
    assert_eq!(batch.members[1].status, BatchMemberStatus::Cancelled);

    let connection = Connection::open(&database).unwrap();
    assert!(
        connection
            .execute("DELETE FROM runs WHERE id=?1", [run.id.to_string()])
            .is_err()
    );
    assert!(repository.delete_batch(batch_id).is_err());
}

#[test]
fn created_batch_keeps_target_and_suite_identities_alive() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    let (pack, plan) = sample_plan();
    let batch_id = Uuid::new_v4();
    repository
        .insert_batch_plan(
            batch_id,
            &pack,
            &plan,
            &schedule(),
            plan.cost_estimate.issued_at,
        )
        .unwrap();

    let unrelated = run_for(&plan, 0, Uuid::new_v4());
    repository.insert_run(&unrelated).unwrap();
    assert!(repository.delete_run(unrelated.id).unwrap());
    let connection = Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM suite_versions WHERE suite_id=?1 AND suite_version=?2",
                params![plan.suite_id, plan.suite_version],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(repository.get_batch(batch_id).unwrap().unwrap().plan, plan);
}

#[test]
fn initial_authorization_is_exact_fresh_and_expiring() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (pack, plan) = sample_plan();
    let batch_id = Uuid::new_v4();
    repository
        .insert_batch_plan(
            batch_id,
            &pack,
            &plan,
            &schedule(),
            plan.cost_estimate.issued_at,
        )
        .unwrap();

    let mut understated = initial_authorization(batch_id, &plan);
    understated.max_provider_turns -= 1;
    assert!(
        repository
            .append_execution_authorization(&understated)
            .is_err()
    );

    let mut stale = initial_authorization(batch_id, &plan);
    stale.created_at = plan.cost_estimate.initial_acknowledgement_expires_at + Duration::seconds(1);
    stale.expires_at = stale.created_at + Duration::hours(4);
    assert!(repository.append_execution_authorization(&stale).is_err());

    repository
        .append_execution_authorization(&initial_authorization(batch_id, &plan))
        .unwrap();
    let run = run_for(&plan, 0, Uuid::new_v4());
    assert!(
        repository
            .reserve_next_runnable_member_and_run(
                batch_id,
                plan.cost_estimate.issued_at + Duration::hours(4) + Duration::seconds(1),
                &run,
            )
            .unwrap()
            .is_none()
    );
    assert!(repository.get_run(run.id).unwrap().is_none());
}

#[test]
fn launch_rechecks_authorization_before_provider_boundary() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = run_for(&plan, 0, Uuid::new_v4());
    repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            plan.cost_estimate.issued_at + Duration::seconds(1),
            &run,
        )
        .unwrap()
        .unwrap();

    let after_expiry = plan.cost_estimate.issued_at + Duration::hours(4) + Duration::seconds(1);
    assert!(
        repository
            .mark_member_launching(batch_id, 0, run.id, after_expiry)
            .is_err()
    );
    assert_eq!(
        repository.get_batch(batch_id).unwrap().unwrap().members[0].status,
        BatchMemberStatus::Reserved
    );
    assert_eq!(
        repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Created
    );

    repository
        .reconcile_batches_after_startup(after_expiry)
        .unwrap();
    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.members[0].status, BatchMemberStatus::Deferred);
    assert_eq!(
        batch.members[0].failure_kind,
        Some(FailureKind::AuthExpired)
    );
    assert_eq!(batch.status, BatchStatus::Paused);
}

#[test]
fn expired_retry_does_not_fall_back_to_initial_batch_authorization() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let first = reserve_and_start(&repository, batch_id, &plan, 0);
    repository
        .defer_batch_member(
            batch_id,
            0,
            Some(first.id),
            FailureKind::Network,
            plan.cost_estimate.issued_at + Duration::minutes(2),
        )
        .unwrap();
    complete_guided_member(&repository, batch_id, &plan, 1);

    let member_runs = plan.cost_estimate.planned_member_runs;
    let mut retry = ScanExecutionAuthorization {
        batch_id,
        member_ordinal: Some(0),
        attempt_number: 2,
        max_provider_turns: plan.cost_estimate.max_provider_turns / member_runs,
        max_task_budget_secs: plan.cost_estimate.summed_task_budget_secs / member_runs,
        acknowledgement_hash: String::new(),
        allowed_failure_kind: Some(FailureKind::Network),
        created_at: plan.cost_estimate.issued_at + Duration::minutes(20),
        expires_at: plan.cost_estimate.issued_at + Duration::minutes(21),
    };
    retry.acknowledgement_hash = retry.expected_retry_acknowledgement_hash(&plan).unwrap();
    repository.append_execution_authorization(&retry).unwrap();

    let same_run = run_for(&plan, 0, first.id);
    assert!(
        repository
            .reserve_next_runnable_member_and_run(
                batch_id,
                retry.expires_at + Duration::seconds(1),
                &same_run,
            )
            .unwrap()
            .is_none()
    );
    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.status, BatchStatus::Paused);
    assert_eq!(batch.members[0].status, BatchMemberStatus::Deferred);
    assert_eq!(
        batch.members[0].failure_kind,
        Some(FailureKind::AuthExpired)
    );
    assert_eq!(repository.list_runs().unwrap().len(), 2);
}

#[test]
fn one_acknowledgement_cannot_create_two_batches() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (pack, plan) = sample_plan();
    repository
        .insert_batch_plan(
            Uuid::new_v4(),
            &pack,
            &plan,
            &schedule(),
            plan.cost_estimate.issued_at,
        )
        .unwrap();
    assert!(
        repository
            .insert_batch_plan(
                Uuid::new_v4(),
                &pack,
                &plan,
                &schedule(),
                plan.cost_estimate.issued_at,
            )
            .is_err()
    );
    assert_eq!(repository.list_batches().unwrap().len(), 1);
}

#[test]
fn deferred_member_reauthorization_reuses_the_same_run_after_other_target_completes() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let first = reserve_and_start(&repository, batch_id, &plan, 0);
    repository
        .defer_batch_member(
            batch_id,
            0,
            Some(first.id),
            FailureKind::Network,
            plan.cost_estimate.issued_at + Duration::minutes(2),
        )
        .unwrap();
    assert!(
        repository
            .resume_run(first.id, &first.target, |_run, _results| Ok(()))
            .is_err()
    );
    let second = complete_guided_member(&repository, batch_id, &plan, 1);
    assert_eq!(second.status, RunStatus::Completed);

    let member_runs = plan.cost_estimate.planned_member_runs;
    let mut authorization = ScanExecutionAuthorization {
        batch_id,
        member_ordinal: Some(0),
        attempt_number: 2,
        max_provider_turns: plan.cost_estimate.max_provider_turns / member_runs,
        max_task_budget_secs: plan.cost_estimate.summed_task_budget_secs / member_runs,
        acknowledgement_hash: String::new(),
        allowed_failure_kind: Some(FailureKind::Network),
        expires_at: plan.cost_estimate.issued_at + Duration::hours(3),
        created_at: plan.cost_estimate.issued_at + Duration::minutes(20),
    };
    authorization.acknowledgement_hash = authorization
        .expected_retry_acknowledgement_hash(&plan)
        .unwrap();

    let mut forged = authorization.clone();
    forged.acknowledgement_hash = "0".repeat(64);
    assert!(repository.append_execution_authorization(&forged).is_err());

    let mut predates_failure = authorization.clone();
    predates_failure.created_at = plan.cost_estimate.issued_at + Duration::minutes(1);
    predates_failure.expires_at = predates_failure.created_at + Duration::hours(1);
    predates_failure.acknowledgement_hash = predates_failure
        .expected_retry_acknowledgement_hash(&plan)
        .unwrap();
    assert!(
        repository
            .append_execution_authorization(&predates_failure)
            .is_err()
    );

    let mut tampered_budget = authorization.clone();
    tampered_budget.max_provider_turns -= 1;
    assert!(
        repository
            .append_execution_authorization(&tampered_budget)
            .is_err()
    );

    repository
        .append_execution_authorization(&authorization)
        .unwrap();
    repository
        .resume_batch(batch_id, authorization.created_at)
        .unwrap();

    let same_id = run_for(&plan, 0, first.id);
    let reservation = repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            authorization.created_at + Duration::seconds(1),
            &same_id,
        )
        .unwrap()
        .unwrap();
    assert_eq!(reservation.member.ordinal, 0);
    assert_eq!(reservation.member.run_id, Some(first.id));
    assert_eq!(reservation.run.status, RunStatus::Interrupted);
    assert_eq!(repository.list_runs().unwrap().len(), 2);
}

#[test]
fn startup_reconciliation_repairs_stale_terminal_direction_without_replay() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let completed = complete_guided_member(&repository, batch_id, &plan, 0);

    let connection = Connection::open(&database).unwrap();
    connection
        .execute(
            "UPDATE scan_batch_members SET status_json='\"running\"' WHERE batch_id=?1 AND ordinal=0",
            [batch_id.to_string()],
        )
        .unwrap();
    assert_eq!(
        repository
            .reconcile_batches_after_startup(plan.cost_estimate.issued_at + Duration::minutes(30))
            .unwrap(),
        1
    );
    assert_eq!(
        repository.get_batch(batch_id).unwrap().unwrap().members[0].status,
        BatchMemberStatus::Completed
    );

    connection
        .execute(
            "UPDATE runs SET status_json='\"running\"',finished_at=NULL,score_json=NULL WHERE id=?1",
            [completed.id.to_string()],
        )
        .unwrap();
    assert_eq!(
        repository
            .reconcile_batches_after_startup(plan.cost_estimate.issued_at + Duration::minutes(31))
            .unwrap(),
        1
    );
    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.members[0].status, BatchMemberStatus::Invalid);
    assert_eq!(batch.members[0].run_id, Some(completed.id));
    assert_eq!(
        repository.get_run(completed.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
}

#[test]
fn guided_checkpoint_rejects_wrong_task_authorization_ownership_and_attestation() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = reserve_and_start(&repository, batch_id, &plan, 0);
    let good = attestation(&plan, 2);

    assert!(
        repository
            .finish_without_score(run.id, RunStatus::Interrupted)
            .is_err()
    );
    assert!(
        repository
            .save_task_result(&passing_result(run.id, "instruction-filter"))
            .is_err()
    );

    let mut declined = good.clone();
    declined.user_attested = false;
    assert!(
        repository
            .save_guided_task_result_with_isolation(
                batch_id,
                0,
                &passing_result(run.id, "instruction-filter"),
                &declined,
            )
            .is_err()
    );
    let mut wrong_category = passing_result(run.id, "instruction-filter");
    wrong_category.category = Category::Logic;
    assert!(
        repository
            .save_guided_task_result_with_isolation(batch_id, 0, &wrong_category, &good,)
            .is_err()
    );
    assert!(
        repository
            .save_guided_task_result_with_isolation(
                batch_id,
                0,
                &passing_result(run.id, "not-in-sealed-pack"),
                &good,
            )
            .is_err()
    );
    assert!(
        repository
            .save_guided_task_result_with_isolation(
                batch_id,
                1,
                &passing_result(run.id, "instruction-filter"),
                &good,
            )
            .is_err()
    );
    let mut expired = good;
    expired.recorded_at = plan.cost_estimate.issued_at + Duration::hours(5);
    assert!(
        repository
            .save_guided_task_result_with_isolation(
                batch_id,
                0,
                &passing_result(run.id, "instruction-filter"),
                &expired,
            )
            .is_err()
    );
    assert!(repository.get_task_results(run.id).unwrap().is_empty());
}

#[test]
fn isolation_insert_failure_rolls_back_the_task_result_too() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = reserve_and_start(&repository, batch_id, &plan, 0);
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_isolation BEFORE INSERT ON scan_batch_task_isolation
             BEGIN SELECT RAISE(ABORT, 'injected isolation failure'); END;",
        )
        .unwrap();

    assert!(
        repository
            .save_guided_task_result_with_isolation(
                batch_id,
                0,
                &passing_result(run.id, "instruction-filter"),
                &attestation(&plan, 2),
            )
            .is_err()
    );
    assert!(repository.get_task_results(run.id).unwrap().is_empty());
}

#[test]
fn cancellation_leaves_crossed_provider_boundary_active_until_terminalized() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = run_for(&plan, 0, Uuid::new_v4());
    repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(1),
            &run,
        )
        .unwrap();
    repository
        .mark_member_launching(batch_id, 0, run.id, plan.cost_estimate.issued_at)
        .unwrap();
    repository
        .cancel_batch(
            batch_id,
            plan.cost_estimate.issued_at + Duration::minutes(2),
        )
        .unwrap();

    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert!(batch.cancel_requested);
    assert_eq!(batch.status, BatchStatus::Running);
    assert_eq!(batch.members[0].status, BatchMemberStatus::Launching);
    assert_eq!(batch.members[1].status, BatchMemberStatus::Cancelled);
}

#[test]
fn startup_reconciliation_finishes_pending_cancellation() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let (batch_id, plan) = create_batch(&repository);
    let run = reserve_and_start(&repository, batch_id, &plan, 0);
    let cancelled_at = plan.cost_estimate.issued_at + Duration::minutes(2);
    repository.cancel_batch(batch_id, cancelled_at).unwrap();
    assert!(
        repository
            .defer_batch_member(
                batch_id,
                0,
                Some(run.id),
                FailureKind::Network,
                cancelled_at + Duration::seconds(1),
            )
            .is_err()
    );

    assert_eq!(
        repository
            .reconcile_batches_after_startup(cancelled_at + Duration::seconds(2))
            .unwrap(),
        1
    );
    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.status, BatchStatus::Cancelled);
    assert!(batch.members.iter().all(|member| matches!(
        member.status,
        BatchMemberStatus::Completed
            | BatchMemberStatus::Invalid
            | BatchMemberStatus::Unavailable
            | BatchMemberStatus::Cancelled
    )));
    assert_eq!(batch.members[0].status, BatchMemberStatus::Cancelled);
    assert_eq!(
        repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Cancelled
    );
}

#[test]
fn indexed_identity_corruption_is_rejected_on_read() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    let (batch_id, _) = create_batch(&repository);
    let (second_pack, second_plan) = sample_plan_with_seed(18);
    let mutated_plan_batch = Uuid::new_v4();
    repository
        .insert_batch_plan(
            mutated_plan_batch,
            &second_pack,
            &second_plan,
            &schedule(),
            second_plan.cost_estimate.issued_at,
        )
        .unwrap();
    let connection = Connection::open(&database).unwrap();
    connection
        .execute(
            "UPDATE scan_batches SET content_sha256=?2 WHERE id=?1",
            params![batch_id.to_string(), "0".repeat(64)],
        )
        .unwrap();
    assert!(repository.get_batch(batch_id).is_err());
    assert!(repository.list_batches().is_err());

    let plan_json: String = connection
        .query_row(
            "SELECT plan_json FROM scan_batches WHERE id=?1",
            [mutated_plan_batch.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let mut plan_value: serde_json::Value = serde_json::from_str(&plan_json).unwrap();
    plan_value["seed"] = serde_json::json!(999);
    connection
        .execute(
            "UPDATE scan_batches SET plan_json=?2,seed=999 WHERE id=?1",
            params![
                mutated_plan_batch.to_string(),
                serde_json::to_string(&plan_value).unwrap()
            ],
        )
        .unwrap();
    assert!(repository.get_batch(mutated_plan_batch).is_err());
}
