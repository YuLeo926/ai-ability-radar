use crate::batch_commands::{
    authorize_batch_execution_at, authorize_batch_retry_at, begin_guided_batch_member_at,
    cancel_batch_at, create_acknowledged_batch_at, decline_guided_batch_attestation_at,
    estimate_batch_at, estimate_batch_retry_at, get_batch_record, get_next_guided_member_record,
    list_batch_records, pause_batch_at, resume_batch_at, start_batch_at,
    submit_guided_batch_answer_at, BatchCommandContext, BATCH_CAPABILITIES,
};
use crate::dto::{
    AuthorizeBatchExecutionInput, AuthorizeBatchRetryInput, BatchIdInput, BatchPlanInput,
    BatchTargetInput, CreateAcknowledgedBatchInput, DeclineGuidedBatchAttestationInput,
    EstimateBatchRetryInput, SubmitGuidedBatchAnswerInput, TargetSelectionInput,
};
use ability_core::{
    build_batch_schedule, AdapterLaunchKind, BatchExecutionSurface, BatchFeatureLevel,
    BatchMemberSeed, BatchMemberStatus, BatchMode, BatchStatus, ExecutionAdapterIdentity,
    FailureKind, LoadedPack, ManualRunService, ModelSource, ModelVerification, PackLoader,
    RunRepository, ScanBatchPlan, ScanBatchTarget, TargetKind, TargetSelection,
};
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;
use uuid::Uuid;

fn pack(name: &str) -> LoadedPack {
    PackLoader::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("benchmark-packs")
            .join(name),
    )
    .unwrap()
}

fn issued_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 30, 2, 0, 0).single().unwrap()
}

fn guided_target(kind: TargetKind, model: &str) -> BatchTargetInput {
    let provider = match kind {
        TargetKind::ChatGptClient => "openai",
        TargetKind::ClaudeClient => "anthropic",
        _ => panic!("guided target required"),
    };
    BatchTargetInput {
        target: TargetSelectionInput {
            kind,
            reported_model: model.into(),
            reasoning_effort: Some("high".into()),
            model_source: ModelSource::Manual,
            model_verification: ModelVerification::UserConfirmed,
        },
        execution_surface: BatchExecutionSurface::GuidedClient,
        execution_adapter_identity: ExecutionAdapterIdentity::new(
            BatchExecutionSurface::GuidedClient,
            provider,
            AdapterLaunchKind::GuidedClient,
            None,
            "guided-client-v1",
        )
        .unwrap(),
    }
}

fn cli_target(kind: TargetKind, model: &str) -> BatchTargetInput {
    let (provider, contract) = match kind {
        TargetKind::CodexCli => ("openai", "codex-cli-v1"),
        TargetKind::ClaudeCode => ("anthropic", "claude-code-v1"),
        _ => panic!("CLI target required"),
    };
    BatchTargetInput {
        target: TargetSelectionInput {
            kind,
            reported_model: model.into(),
            reasoning_effort: Some("high".into()),
            model_source: ModelSource::CliRequested,
            model_verification: ModelVerification::UserConfirmed,
        },
        execution_surface: BatchExecutionSurface::AutomatedCli,
        execution_adapter_identity: ExecutionAdapterIdentity::new(
            BatchExecutionSurface::AutomatedCli,
            provider,
            AdapterLaunchKind::NativeExe,
            Some("1.2.3"),
            contract,
        )
        .unwrap(),
    }
}

fn guided_plan() -> BatchPlanInput {
    BatchPlanInput {
        mode: BatchMode::QuickComparison,
        seed: 17,
        targets: vec![
            guided_target(TargetKind::ChatGptClient, "GPT-5.6"),
            guided_target(TargetKind::ClaudeClient, "Claude Sonnet 4.5"),
        ],
    }
}

fn cli_plan(mode: BatchMode) -> BatchPlanInput {
    BatchPlanInput {
        mode,
        seed: 23,
        targets: vec![
            cli_target(TargetKind::CodexCli, "gpt-5.6-codex"),
            cli_target(TargetKind::ClaudeCode, "claude-opus-4.5"),
        ],
    }
}

fn core_target(input: &BatchTargetInput) -> ScanBatchTarget {
    ScanBatchTarget::new(
        TargetSelection {
            kind: input.target.kind,
            reported_model: input.target.reported_model.clone(),
            reasoning_effort: input.target.reasoning_effort.clone(),
            model_source: input.target.model_source,
            model_verification: input.target.model_verification,
        },
        input.execution_surface,
        input.execution_adapter_identity.clone(),
    )
    .unwrap()
}

fn context<'a>(
    repository: &'a RunRepository,
    client_pack: &'a LoadedPack,
    cli_pack: &'a LoadedPack,
) -> BatchCommandContext<'a> {
    BatchCommandContext {
        repository,
        client_pack,
        cli_pack,
    }
}

fn create_batch(
    context: &BatchCommandContext<'_>,
    plan_input: BatchPlanInput,
) -> ability_core::ScanBatchRecord {
    let issued = issued_at();
    let estimate = estimate_batch_at(context, plan_input.clone(), issued).unwrap();
    create_acknowledged_batch_at(
        context,
        CreateAcknowledgedBatchInput {
            plan: plan_input,
            estimate_issued_at: issued,
            acknowledgement_hash: estimate.plan.acknowledgement_hash,
        },
        issued + Duration::seconds(1),
        Uuid::new_v4(),
    )
    .unwrap()
}

#[test]
fn batch_dtos_use_exact_camel_case_and_reject_unknown_fields() {
    let wire = json!({
        "mode": "quick_comparison",
        "seed": 17,
        "targets": [{
            "target": {
                "kind": "chat_gpt_client",
                "reportedModel": "GPT-5.6",
                "reasoningEffort": "high",
                "modelSource": "manual",
                "modelVerification": "user_confirmed"
            },
            "executionSurface": "guided_client",
            "executionAdapterIdentity": {
                "executionSurface": "guided_client",
                "providerFamily": "openai",
                "launchKind": "guided_client",
                "publicVersion": null,
                "adapterContractVersion": "guided-client-v1"
            }
        }, {
            "target": {
                "kind": "claude_client",
                "reportedModel": "Claude Sonnet 4.5",
                "reasoningEffort": "high",
                "modelSource": "manual",
                "modelVerification": "user_confirmed"
            },
            "executionSurface": "guided_client",
            "executionAdapterIdentity": {
                "executionSurface": "guided_client",
                "providerFamily": "anthropic",
                "launchKind": "guided_client",
                "publicVersion": null,
                "adapterContractVersion": "guided-client-v1"
            }
        }]
    });
    let parsed: BatchPlanInput = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(parsed, guided_plan());
    assert_eq!(serde_json::to_value(parsed).unwrap(), wire);

    let mut unknown_outer = wire.clone();
    unknown_outer["providerRequestCount"] = json!(1);
    assert!(serde_json::from_value::<BatchPlanInput>(unknown_outer).is_err());
    let mut unknown_nested = wire;
    unknown_nested["targets"][0]["executionAdapterIdentity"]["program"] =
        json!("C:/private/codex.exe");
    assert!(serde_json::from_value::<BatchPlanInput>(unknown_nested).is_err());
    for (path, unknown_value) in [
        ("mode", json!("turbo")),
        ("targets.0.executionSurface", json!("browser")),
        (
            "targets.0.executionAdapterIdentity.launchKind",
            json!("shell"),
        ),
    ] {
        let mut unknown_enum = serde_json::to_value(guided_plan()).unwrap();
        match path {
            "mode" => unknown_enum["mode"] = unknown_value,
            "targets.0.executionSurface" => {
                unknown_enum["targets"][0]["executionSurface"] = unknown_value
            }
            _ => {
                unknown_enum["targets"][0]["executionAdapterIdentity"]["launchKind"] = unknown_value
            }
        }
        assert!(serde_json::from_value::<BatchPlanInput>(unknown_enum).is_err());
    }
    let canonical_id = Uuid::new_v4();
    assert_eq!(
        serde_json::from_value::<BatchIdInput>(json!({
            "batchId": canonical_id
        }))
        .unwrap()
        .batch_id,
        canonical_id
    );
    for invalid in [
        json!({"batchId": Uuid::nil()}),
        json!({"batchId": canonical_id.simple().to_string()}),
        json!({"batchId": canonical_id, "force": true}),
    ] {
        assert!(serde_json::from_value::<BatchIdInput>(invalid).is_err());
    }

    let run_id = Uuid::new_v4();
    let guided_answer = json!({
        "batchId": canonical_id,
        "memberOrdinal": 0,
        "runId": run_id,
        "taskId": "logic-grid",
        "answer": "answer",
        "userAttestedNewConversation": true
    });
    assert!(serde_json::from_value::<SubmitGuidedBatchAnswerInput>(guided_answer.clone()).is_ok());
    for invalid in [
        {
            let mut value = guided_answer.clone();
            value["userAttestedNewConversation"] = json!(false);
            value
        },
        {
            let mut value = guided_answer.clone();
            value["runId"] = json!(run_id.simple().to_string());
            value
        },
        {
            let mut value = guided_answer;
            value["copiedAutomatically"] = json!(true);
            value
        },
    ] {
        assert!(serde_json::from_value::<SubmitGuidedBatchAnswerInput>(invalid).is_err());
    }
}

#[test]
fn guided_command_path_reuses_the_reserved_run_and_advances_the_schedule() {
    let directory = tempdir().unwrap();
    let repository = Arc::new(RunRepository::open(&directory.path().join("ability.db")).unwrap());
    let artifact_root = directory.path().join("artifacts");
    std::fs::create_dir_all(&artifact_root).unwrap();
    let service = ManualRunService::new(repository.clone(), artifact_root);
    let client_pack = Arc::new(pack("client-quick-v1"));
    let cli_pack = pack("cli-quick-v1");
    let context = context(&repository, &client_pack, &cli_pack);
    let created = create_batch(&context, guided_plan());
    authorize_batch_execution_at(
        &context,
        AuthorizeBatchExecutionInput {
            batch_id: created.id,
            acknowledgement_hash: created.plan.acknowledgement_hash.clone(),
        },
        issued_at() + Duration::seconds(2),
    )
    .unwrap();
    start_batch_at(
        &context,
        BatchIdInput {
            batch_id: created.id,
        },
        issued_at() + Duration::seconds(3),
    )
    .unwrap();

    let run_id = Uuid::new_v4();
    let run = begin_guided_batch_member_at(
        &context,
        &service,
        client_pack.clone(),
        BatchIdInput {
            batch_id: created.id,
        },
        issued_at() + Duration::seconds(4),
        run_id,
    )
    .unwrap();
    assert_eq!(run.id, run_id);
    assert_eq!(repository.list_runs().unwrap().len(), 1);
    let active = repository.get_batch(created.id).unwrap().unwrap();
    let first = &active.members[0];
    assert_eq!(first.run_id, Some(run_id));
    assert_eq!(first.status, BatchMemberStatus::Running);
    assert!(begin_guided_batch_member_at(
        &context,
        &service,
        client_pack.clone(),
        BatchIdInput {
            batch_id: created.id,
        },
        issued_at() + Duration::seconds(5),
        Uuid::new_v4(),
    )
    .is_err());
    assert_eq!(repository.list_runs().unwrap().len(), 1);

    let mut submitted = 0_i64;
    while let Some(step) = service.next_step(run_id).unwrap() {
        submit_guided_batch_answer_at(
            &service,
            SubmitGuidedBatchAnswerInput {
                batch_id: created.id,
                member_ordinal: first.ordinal,
                run_id,
                task_id: step.task_id,
                answer: "deterministic local answer".into(),
                user_attested_new_conversation: true,
            },
            issued_at() + Duration::seconds(6 + submitted),
        )
        .unwrap();
        submitted += 1;
    }
    assert_eq!(usize::try_from(submitted).unwrap(), client_pack.tasks.len());
    let advanced = repository.get_batch(created.id).unwrap().unwrap();
    assert_eq!(advanced.members[0].status, BatchMemberStatus::Completed);
    assert_eq!(advanced.members[1].status, BatchMemberStatus::Planned);
    assert_eq!(
        repository.get_task_results(run_id).unwrap().len(),
        client_pack.tasks.len()
    );
    assert_eq!(
        get_next_guided_member_record(
            &context,
            BatchIdInput {
                batch_id: created.id,
            }
        )
        .unwrap()
        .member
        .unwrap()
        .ordinal,
        1
    );
}

#[test]
fn guided_decline_command_invalidates_only_the_exact_active_member() {
    let directory = tempdir().unwrap();
    let repository = Arc::new(RunRepository::open(&directory.path().join("ability.db")).unwrap());
    let artifact_root = directory.path().join("artifacts");
    std::fs::create_dir_all(&artifact_root).unwrap();
    let service = ManualRunService::new(repository.clone(), artifact_root);
    let client_pack = Arc::new(pack("client-quick-v1"));
    let cli_pack = pack("cli-quick-v1");
    let context = context(&repository, &client_pack, &cli_pack);
    let created = create_batch(&context, guided_plan());
    authorize_batch_execution_at(
        &context,
        AuthorizeBatchExecutionInput {
            batch_id: created.id,
            acknowledgement_hash: created.plan.acknowledgement_hash.clone(),
        },
        issued_at() + Duration::seconds(2),
    )
    .unwrap();
    start_batch_at(
        &context,
        BatchIdInput {
            batch_id: created.id,
        },
        issued_at() + Duration::seconds(3),
    )
    .unwrap();
    let run_id = Uuid::new_v4();
    begin_guided_batch_member_at(
        &context,
        &service,
        client_pack.clone(),
        BatchIdInput {
            batch_id: created.id,
        },
        issued_at() + Duration::seconds(4),
        run_id,
    )
    .unwrap();
    let step = service.next_step(run_id).unwrap().unwrap();

    let declined = decline_guided_batch_attestation_at(
        &context,
        &service,
        DeclineGuidedBatchAttestationInput {
            batch_id: created.id,
            member_ordinal: 0,
            run_id,
            task_id: step.task_id,
        },
        issued_at() + Duration::seconds(5),
    )
    .unwrap();
    assert_eq!(declined.members[0].status, BatchMemberStatus::Invalid);
    assert_eq!(
        declined.members[0].failure_kind,
        Some(FailureKind::UserCancelled)
    );
    assert_eq!(declined.members[1].status, BatchMemberStatus::Planned);
    assert!(repository.get_task_results(run_id).unwrap().is_empty());
}

#[test]
fn full_mode_is_gated_before_reliable_analysis() {
    assert_eq!(
        BATCH_CAPABILITIES,
        [
            BatchFeatureLevel::GuidedQuickV1,
            BatchFeatureLevel::CliStandardV1
        ]
    );
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let client_pack = pack("client-quick-v1");
    let cli_pack = pack("cli-quick-v1");
    let context = context(&repository, &client_pack, &cli_pack);

    let full_input = cli_plan(BatchMode::Full);
    assert!(estimate_batch_at(&context, full_input.clone(), issued_at()).is_err());
    assert!(create_acknowledged_batch_at(
        &context,
        CreateAcknowledgedBatchInput {
            plan: full_input.clone(),
            estimate_issued_at: issued_at(),
            acknowledgement_hash: "0".repeat(64),
        },
        issued_at(),
        Uuid::new_v4(),
    )
    .is_err());
    assert!(repository.list_batches().unwrap().is_empty());
    assert!(repository.list_runs().unwrap().is_empty());

    let full_plan = ScanBatchPlan::new(
        &cli_pack,
        "ability-v1",
        BatchMode::Full,
        full_input.seed,
        full_input.targets.iter().map(core_target).collect(),
        issued_at(),
    )
    .unwrap();
    let schedule = build_batch_schedule(&full_plan).unwrap();
    let members = schedule
        .members
        .iter()
        .map(|member| BatchMemberSeed {
            ordinal: member.ordinal,
            target_position: member.target_position,
            repetition_index: member.repetition_index,
        })
        .collect::<Vec<_>>();
    let batch_id = Uuid::new_v4();
    repository
        .insert_batch_plan(batch_id, &cli_pack, &full_plan, &members, issued_at())
        .unwrap();
    let before = repository.get_batch(batch_id).unwrap().unwrap();
    assert!(start_batch_at(&context, BatchIdInput { batch_id }, issued_at()).is_err());
    assert_eq!(repository.get_batch(batch_id).unwrap().unwrap(), before);
    assert!(repository.list_runs().unwrap().is_empty());
}

#[test]
fn stale_acknowledgement_is_rejected() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let client_pack = pack("client-quick-v1");
    let cli_pack = pack("cli-quick-v1");
    let context = context(&repository, &client_pack, &cli_pack);
    let input = guided_plan();
    let estimate = estimate_batch_at(&context, input.clone(), issued_at()).unwrap();

    for (plan, estimate_issued_at, acknowledgement_hash, created_at) in [
        (
            input.clone(),
            issued_at(),
            "0".repeat(64),
            issued_at() + Duration::seconds(1),
        ),
        (
            BatchPlanInput {
                seed: input.seed + 1,
                ..input.clone()
            },
            issued_at(),
            estimate.plan.acknowledgement_hash.clone(),
            issued_at() + Duration::seconds(1),
        ),
        (
            input.clone(),
            issued_at(),
            estimate.plan.acknowledgement_hash.clone(),
            issued_at() + Duration::minutes(16),
        ),
    ] {
        assert!(create_acknowledged_batch_at(
            &context,
            CreateAcknowledgedBatchInput {
                plan,
                estimate_issued_at,
                acknowledgement_hash,
            },
            created_at,
            Uuid::new_v4(),
        )
        .is_err());
    }
    assert!(repository.list_batches().unwrap().is_empty());
}

#[test]
fn mixed_surface_cohort_is_rejected() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let client_pack = pack("client-quick-v1");
    let cli_pack = pack("cli-quick-v1");
    let context = context(&repository, &client_pack, &cli_pack);
    let mixed = BatchPlanInput {
        mode: BatchMode::QuickComparison,
        seed: 1,
        targets: vec![
            guided_target(TargetKind::ChatGptClient, "GPT-5.6"),
            cli_target(TargetKind::CodexCli, "gpt-5.6-codex"),
        ],
    };
    assert!(estimate_batch_at(&context, mixed, issued_at()).is_err());
    assert!(repository.list_batches().unwrap().is_empty());
}

#[test]
fn duplicate_incoherent_and_unknown_targets_fail_before_creation() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let client_pack = pack("client-quick-v1");
    let cli_pack = pack("cli-quick-v1");
    let context = context(&repository, &client_pack, &cli_pack);

    let duplicate = BatchPlanInput {
        targets: vec![
            guided_target(TargetKind::ChatGptClient, "GPT-5.6"),
            guided_target(TargetKind::ChatGptClient, "GPT-5.6"),
        ],
        ..guided_plan()
    };
    assert!(estimate_batch_at(&context, duplicate, issued_at()).is_err());

    let mut incoherent = guided_plan();
    incoherent.targets[0].target.model_source = ModelSource::CliReported;
    incoherent.targets[0].target.model_verification = ModelVerification::ProviderReported;
    assert!(estimate_batch_at(&context, incoherent, issued_at()).is_err());

    let mut wrong_surface = cli_plan(BatchMode::Standard);
    wrong_surface.targets[0].execution_surface = BatchExecutionSurface::GuidedClient;
    assert!(estimate_batch_at(&context, wrong_surface, issued_at()).is_err());

    let mut path_adapter = guided_plan();
    path_adapter.targets[0]
        .execution_adapter_identity
        .adapter_contract_version = "C:/private/adapter.exe".into();
    assert!(estimate_batch_at(&context, path_adapter, issued_at()).is_err());

    let mut unknown_adapter = guided_plan();
    unknown_adapter.targets[0]
        .execution_adapter_identity
        .adapter_contract_version = "unknown-adapter-v9".into();
    assert!(estimate_batch_at(&context, unknown_adapter, issued_at()).is_err());

    let too_many = BatchPlanInput {
        targets: (0..6)
            .map(|index| cli_target(TargetKind::CodexCli, &format!("model-{index}")))
            .collect(),
        ..cli_plan(BatchMode::Standard)
    };
    assert!(estimate_batch_at(&context, too_many, issued_at()).is_err());
    assert!(repository.list_batches().unwrap().is_empty());
}

#[test]
fn creation_authorization_and_lifecycle_do_not_reserve_or_launch_a_run() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let client_pack = pack("client-quick-v1");
    let cli_pack = pack("cli-quick-v1");
    let context = context(&repository, &client_pack, &cli_pack);
    let created = create_batch(&context, guided_plan());
    assert_eq!(created.status, BatchStatus::Created);
    assert_eq!(list_batch_records(&context).unwrap(), vec![created.clone()]);
    assert_eq!(
        get_batch_record(
            &context,
            BatchIdInput {
                batch_id: created.id
            }
        )
        .unwrap(),
        Some(created.clone())
    );

    assert!(authorize_batch_execution_at(
        &context,
        AuthorizeBatchExecutionInput {
            batch_id: created.id,
            acknowledgement_hash: "0".repeat(64),
        },
        issued_at() + Duration::seconds(2),
    )
    .is_err());
    assert_eq!(
        repository.get_batch(created.id).unwrap().unwrap().status,
        BatchStatus::Created
    );

    let authorization = authorize_batch_execution_at(
        &context,
        AuthorizeBatchExecutionInput {
            batch_id: created.id,
            acknowledgement_hash: created.plan.acknowledgement_hash.clone(),
        },
        issued_at() + Duration::seconds(2),
    )
    .unwrap();
    assert_eq!(authorization.member_ordinal, None);
    assert_eq!(authorization.attempt_number, 1);
    assert_eq!(
        authorization.max_task_launches,
        created.plan.cost_estimate.task_launches
    );
    assert_eq!(
        authorization.max_provider_turns,
        created.plan.cost_estimate.max_provider_turns
    );
    assert_eq!(
        authorization.max_guided_interactions,
        created.plan.cost_estimate.guided_interactions
    );

    let running = start_batch_at(
        &context,
        BatchIdInput {
            batch_id: created.id,
        },
        issued_at() + Duration::seconds(3),
    )
    .unwrap();
    assert_eq!(running.status, BatchStatus::Running);
    let next = get_next_guided_member_record(
        &context,
        BatchIdInput {
            batch_id: created.id,
        },
    )
    .unwrap();
    assert_eq!(next.decision, crate::dto::GuidedMemberDecision::Runnable);
    assert_eq!(next.member.as_ref().unwrap().ordinal, 0);
    let target_position = usize::try_from(next.member.as_ref().unwrap().target_position).unwrap();
    assert_eq!(
        next.target,
        Some(created.plan.targets[target_position].clone())
    );
    assert!(pause_batch_at(
        &context,
        BatchIdInput {
            batch_id: created.id
        },
        issued_at() + Duration::seconds(4),
    )
    .is_err());
    assert!(repository.list_runs().unwrap().is_empty());

    let cancelled = cancel_batch_at(
        &context,
        BatchIdInput {
            batch_id: created.id,
        },
        issued_at() + Duration::seconds(5),
    )
    .unwrap();
    assert_eq!(cancelled.status, BatchStatus::Cancelled);
    assert!(cancelled
        .members
        .iter()
        .all(|member| member.status == BatchMemberStatus::Cancelled));
    assert!(repository.list_runs().unwrap().is_empty());
}

#[test]
fn expired_initial_authorization_cannot_start_or_reserve_work() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let client_pack = pack("client-quick-v1");
    let cli_pack = pack("cli-quick-v1");
    let context = context(&repository, &client_pack, &cli_pack);
    let created = create_batch(&context, guided_plan());
    let authorization = authorize_batch_execution_at(
        &context,
        AuthorizeBatchExecutionInput {
            batch_id: created.id,
            acknowledgement_hash: created.plan.acknowledgement_hash.clone(),
        },
        issued_at() + Duration::seconds(2),
    )
    .unwrap();

    assert!(start_batch_at(
        &context,
        BatchIdInput {
            batch_id: created.id
        },
        authorization.expires_at + Duration::seconds(1),
    )
    .is_err());
    let stored = repository.get_batch(created.id).unwrap().unwrap();
    assert_eq!(stored.status, BatchStatus::Paused);
    assert!(stored.members.iter().all(|member| {
        member.status == BatchMemberStatus::Deferred
            && member.failure_kind == Some(FailureKind::AuthExpired)
            && member.run_id.is_none()
    }));
    assert!(repository.list_runs().unwrap().is_empty());
}

#[test]
fn next_guided_member_rejects_cli_batches() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let client_pack = pack("client-quick-v1");
    let cli_pack = pack("cli-quick-v1");
    let context = context(&repository, &client_pack, &cli_pack);
    let created = create_batch(&context, cli_plan(BatchMode::Standard));
    let authorization = authorize_batch_execution_at(
        &context,
        AuthorizeBatchExecutionInput {
            batch_id: created.id,
            acknowledgement_hash: created.plan.acknowledgement_hash.clone(),
        },
        issued_at() + Duration::seconds(2),
    )
    .unwrap();
    assert_eq!(authorization.max_guided_interactions, 0);
    assert_eq!(
        authorization.max_task_launches,
        created.plan.cost_estimate.task_launches
    );

    assert!(get_next_guided_member_record(
        &context,
        BatchIdInput {
            batch_id: created.id
        }
    )
    .is_err());
    assert!(repository.list_runs().unwrap().is_empty());
}

#[test]
fn retry_authorization_binds_failure_budget_attempt_and_expiry() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let client_pack = pack("client-quick-v1");
    let cli_pack = pack("cli-quick-v1");
    let context = context(&repository, &client_pack, &cli_pack);
    let created = create_batch(&context, guided_plan());
    authorize_batch_execution_at(
        &context,
        AuthorizeBatchExecutionInput {
            batch_id: created.id,
            acknowledgement_hash: created.plan.acknowledgement_hash.clone(),
        },
        issued_at() + Duration::seconds(2),
    )
    .unwrap();
    repository
        .defer_batch_member(
            created.id,
            0,
            None,
            FailureKind::Network,
            issued_at() + Duration::seconds(3),
        )
        .unwrap();

    for failure in [FailureKind::WrongAnswer, FailureKind::AuthExpired] {
        assert!(estimate_batch_retry_at(
            &context,
            EstimateBatchRetryInput {
                batch_id: created.id,
                member_ordinal: 0,
                expected_failure_kind: failure,
            },
            issued_at() + Duration::seconds(4),
        )
        .is_err());
    }

    let estimate = estimate_batch_retry_at(
        &context,
        EstimateBatchRetryInput {
            batch_id: created.id,
            member_ordinal: 0,
            expected_failure_kind: FailureKind::Network,
        },
        issued_at() + Duration::seconds(4),
    )
    .unwrap();
    assert_eq!(estimate.authorization.member_ordinal, Some(0));
    assert_eq!(
        estimate.authorization.max_task_launches,
        created.plan.cost_estimate.tasks_per_member_run
    );
    assert_eq!(
        estimate.authorization.max_provider_turns,
        created.plan.cost_estimate.max_provider_turns
            / created.plan.cost_estimate.planned_member_runs
    );
    assert_eq!(
        estimate.authorization.max_guided_interactions,
        created.plan.cost_estimate.tasks_per_member_run
    );

    let wrong_failure = AuthorizeBatchRetryInput {
        batch_id: created.id,
        member_ordinal: 0,
        allowed_failure_kind: FailureKind::WrongAnswer,
        estimate_created_at: estimate.authorization.created_at,
        acknowledgement_hash: estimate.authorization.acknowledgement_hash.clone(),
    };
    assert!(
        authorize_batch_retry_at(&context, wrong_failure, issued_at() + Duration::seconds(5))
            .is_err()
    );

    let correct = AuthorizeBatchRetryInput {
        batch_id: created.id,
        member_ordinal: 0,
        allowed_failure_kind: FailureKind::Network,
        estimate_created_at: estimate.authorization.created_at,
        acknowledgement_hash: estimate.authorization.acknowledgement_hash.clone(),
    };
    assert!(authorize_batch_retry_at(
        &context,
        correct.clone(),
        estimate.authorization.expires_at + Duration::seconds(1)
    )
    .is_err());
    assert_eq!(
        repository.get_batch(created.id).unwrap().unwrap().members[0].status,
        BatchMemberStatus::Deferred
    );

    let authorized =
        authorize_batch_retry_at(&context, correct, issued_at() + Duration::seconds(5)).unwrap();
    assert_eq!(
        authorized.acknowledgement_hash,
        estimate.authorization.acknowledgement_hash
    );
    let resumed = resume_batch_at(
        &context,
        BatchIdInput {
            batch_id: created.id,
        },
        issued_at() + Duration::seconds(6),
    )
    .unwrap();
    assert_eq!(resumed.status, BatchStatus::Running);
    assert_eq!(resumed.members[0].ordinal, 0);
    assert_eq!(resumed.members[0].attempt_number, 1);
}
