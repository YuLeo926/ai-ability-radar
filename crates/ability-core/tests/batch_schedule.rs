use ability_core::{
    AdapterLaunchKind, BatchExecutionSurface, BatchMode, BatchScheduleError,
    ExecutionAdapterIdentity, ModelSource, ModelVerification, NextScheduledMember, PackLoader,
    ScanBatchPlan, ScanBatchTarget, ScheduledMemberLifecycle, ScheduledMemberState,
    SessionIsolationPolicy, TargetKind, TargetSelection, build_batch_schedule,
    select_next_scheduled_member,
};
use chrono::{TimeZone, Utc};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn cli_pack() -> ability_core::LoadedPack {
    PackLoader::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("benchmark-packs/cli-quick-v1"),
    )
    .unwrap()
}

fn cli_target(model: &str) -> ScanBatchTarget {
    ScanBatchTarget::new(
        TargetSelection {
            kind: TargetKind::CodexCli,
            reported_model: model.into(),
            reasoning_effort: Some("high".into()),
            model_source: ModelSource::CliRequested,
            model_verification: ModelVerification::UserConfirmed,
        },
        ability_core::BatchExecutionSurface::AutomatedCli,
        ExecutionAdapterIdentity::new(
            ability_core::BatchExecutionSurface::AutomatedCli,
            "openai",
            AdapterLaunchKind::NativeExe,
            Some("1.2.3"),
            "codex-cli-v1",
        )
        .unwrap(),
    )
    .unwrap()
}

fn client_pack() -> ability_core::LoadedPack {
    PackLoader::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("benchmark-packs/client-quick-v1"),
    )
    .unwrap()
}

fn client_target(kind: TargetKind, model: &str, provider: &str) -> ScanBatchTarget {
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
            AdapterLaunchKind::GuidedClient,
            None,
            "guided-client-v1",
        )
        .unwrap(),
    )
    .unwrap()
}

fn plan(mode: BatchMode, seed: u64, target_count: usize) -> ScanBatchPlan {
    let targets = (0..target_count)
        .map(|index| cli_target(&format!("model-{index}")))
        .collect();
    ScanBatchPlan::new(
        &cli_pack(),
        "ability-v1",
        mode,
        seed,
        targets,
        Utc.with_ymd_and_hms(2026, 7, 23, 9, 0, 0).single().unwrap(),
    )
    .unwrap()
}

fn target_rows(schedule: &ability_core::BatchSchedule) -> Vec<Vec<u32>> {
    let repetitions = schedule
        .members
        .iter()
        .map(|member| member.repetition_index)
        .max()
        .map_or(0, |value| value + 1);
    (0..repetitions)
        .map(|repetition| {
            schedule
                .members
                .iter()
                .filter(|member| member.repetition_index == repetition)
                .map(|member| member.target_position)
                .collect()
        })
        .collect()
}

#[test]
fn schedule_is_deterministic_and_balanced() {
    let plan = plan(BatchMode::Standard, 7, 4);
    let first = build_batch_schedule(&plan).unwrap();
    let second = build_batch_schedule(&plan).unwrap();
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(
        target_rows(&first),
        vec![vec![3, 0, 1, 2], vec![0, 3, 2, 1], vec![1, 2, 3, 0]]
    );
    assert_eq!(
        first
            .members
            .iter()
            .map(|member| member.ordinal)
            .collect::<Vec<_>>(),
        (0..12).collect::<Vec<_>>()
    );

    for row in target_rows(&first) {
        assert_eq!(row.iter().copied().collect::<BTreeSet<_>>().len(), 4);
    }
    let rows = target_rows(&first);
    assert_eq!(rows[0][0], 3);
    assert_eq!(rows[1][0], 0);
    assert_eq!(rows[2][0], 1);
    for (round, row) in rows.iter().enumerate() {
        for pair in row.windows(2) {
            let delta = (pair[1] + 4 - pair[0]) % 4;
            assert_eq!(delta, if round % 2 == 0 { 1 } else { 3 });
        }
    }
}

#[test]
fn standard_and_full_never_pin_one_target_to_an_edge() {
    for (mode, target_count, repetitions) in [
        (BatchMode::Standard, 4, 3_usize),
        (BatchMode::Full, 5, 5_usize),
    ] {
        let schedule = build_batch_schedule(&plan(mode, 17, target_count)).unwrap();
        let rows = target_rows(&schedule);
        assert_eq!(rows.len(), repetitions);
        let firsts = rows.iter().map(|row| row[0]).collect::<BTreeSet<_>>();
        let lasts = rows
            .iter()
            .map(|row| *row.last().unwrap())
            .collect::<BTreeSet<_>>();
        assert!(firsts.len() > 1);
        assert!(lasts.len() > 1);
        for target in 0..u32::try_from(target_count).unwrap() {
            assert!(rows.iter().filter(|row| row[0] == target).count() < repetitions);
            assert!(
                rows.iter()
                    .filter(|row| *row.last().unwrap() == target)
                    .count()
                    < repetitions
            );
        }
    }
}

#[test]
fn target_membership_and_order_are_bound_to_the_plan_hash() {
    let original = plan(BatchMode::Standard, 5, 3);
    let reordered = ScanBatchPlan::new(
        &cli_pack(),
        "ability-v1",
        BatchMode::Standard,
        5,
        original.targets.iter().cloned().rev().collect(),
        original.cost_estimate.issued_at,
    )
    .unwrap();
    let reduced = ScanBatchPlan::new(
        &cli_pack(),
        "ability-v1",
        BatchMode::Standard,
        5,
        original.targets[..2].to_vec(),
        original.cost_estimate.issued_at,
    )
    .unwrap();
    assert_ne!(
        original.acknowledgement_hash,
        reordered.acknowledgement_hash
    );
    assert_ne!(original.acknowledgement_hash, reduced.acknowledgement_hash);
    assert_ne!(
        serde_json::to_vec(&build_batch_schedule(&original).unwrap()).unwrap(),
        serde_json::to_vec(&build_batch_schedule(&reordered).unwrap()).unwrap()
    );
}

#[test]
fn earliest_runnable_skips_deferred_not_active() {
    let schedule = build_batch_schedule(&plan(BatchMode::QuickComparison, 0, 3)).unwrap();
    let states = vec![
        ScheduledMemberState {
            ordinal: 0,
            lifecycle: ScheduledMemberLifecycle::Deferred,
        },
        ScheduledMemberState {
            ordinal: 1,
            lifecycle: ScheduledMemberLifecycle::Runnable,
        },
        ScheduledMemberState {
            ordinal: 2,
            lifecycle: ScheduledMemberLifecycle::Terminal,
        },
    ];
    assert_eq!(
        select_next_scheduled_member(&schedule, &states).unwrap(),
        NextScheduledMember::Runnable(schedule.members[1].clone())
    );

    let mut blocked = states;
    blocked[2].lifecycle = ScheduledMemberLifecycle::Launching;
    assert_eq!(
        select_next_scheduled_member(&schedule, &blocked).unwrap(),
        NextScheduledMember::BlockedByActive { ordinal: 2 }
    );
}

#[test]
fn reauthorized_member_keeps_ordinal() {
    let schedule = build_batch_schedule(&plan(BatchMode::Standard, 9, 2)).unwrap();
    let schedule_bytes = serde_json::to_vec(&schedule).unwrap();
    let mut states = schedule
        .members
        .iter()
        .map(|member| ScheduledMemberState {
            ordinal: member.ordinal,
            lifecycle: ScheduledMemberLifecycle::Terminal,
        })
        .collect::<Vec<_>>();
    states[1].lifecycle = ScheduledMemberLifecycle::Deferred;
    states[3].lifecycle = ScheduledMemberLifecycle::Runnable;
    assert_eq!(
        select_next_scheduled_member(&schedule, &states).unwrap(),
        NextScheduledMember::Runnable(schedule.members[3].clone())
    );

    states[1].lifecycle = ScheduledMemberLifecycle::Runnable;
    assert_eq!(
        select_next_scheduled_member(&schedule, &states).unwrap(),
        NextScheduledMember::Runnable(schedule.members[1].clone())
    );
    assert_eq!(serde_json::to_vec(&schedule).unwrap(), schedule_bytes);
}

#[test]
fn terminal_and_ambiguous_active_members_are_never_resumed() {
    let schedule = build_batch_schedule(&plan(BatchMode::QuickComparison, 4, 2)).unwrap();
    let terminal = schedule
        .members
        .iter()
        .map(|member| ScheduledMemberState {
            ordinal: member.ordinal,
            lifecycle: ScheduledMemberLifecycle::Terminal,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        select_next_scheduled_member(&schedule, &terminal).unwrap(),
        NextScheduledMember::Exhausted
    );
    for lifecycle in [
        ScheduledMemberLifecycle::Reserved,
        ScheduledMemberLifecycle::Launching,
        ScheduledMemberLifecycle::Running,
    ] {
        let mut states = terminal.clone();
        states[0].lifecycle = lifecycle;
        states[1].lifecycle = ScheduledMemberLifecycle::Runnable;
        assert_eq!(
            select_next_scheduled_member(&schedule, &states).unwrap(),
            NextScheduledMember::BlockedByActive { ordinal: 0 }
        );
    }

    let mut invalid = terminal;
    let mut misaligned = invalid.clone();
    misaligned.swap(0, 1);
    assert_eq!(
        select_next_scheduled_member(&schedule, &misaligned),
        Err(BatchScheduleError::InvalidMemberStateVector)
    );
    invalid[0].lifecycle = ScheduledMemberLifecycle::Reserved;
    invalid[1].lifecycle = ScheduledMemberLifecycle::Running;
    assert_eq!(
        select_next_scheduled_member(&schedule, &invalid),
        Err(BatchScheduleError::MultipleActiveMembers)
    );
}

#[test]
fn task_session_policy_is_bound_to_the_sealed_pack() {
    let plan = plan(BatchMode::Standard, 3, 3);
    let schedule = build_batch_schedule(&plan).unwrap();
    assert_eq!(
        schedule.plan_acknowledgement_hash,
        plan.acknowledgement_hash
    );
    assert_eq!(
        schedule.task_session_binding.suite_content_sha256,
        plan.suite_content_sha256
    );
    assert_eq!(
        schedule.task_session_binding.task_count,
        u32::try_from(plan.sealed_task_budgets.len()).unwrap()
    );
    assert_eq!(
        schedule.task_session_binding.isolation_policy,
        SessionIsolationPolicy::MachineEnforcedFreshSessionAndWorkspacePerTask
    );

    let guided_plan = ScanBatchPlan::new(
        &client_pack(),
        "ability-v1",
        BatchMode::QuickComparison,
        3,
        vec![
            client_target(TargetKind::ChatGptClient, "gpt-5", "openai"),
            client_target(TargetKind::ClaudeClient, "claude-sonnet", "anthropic"),
        ],
        plan.cost_estimate.issued_at,
    )
    .unwrap();
    assert_eq!(
        build_batch_schedule(&guided_plan)
            .unwrap()
            .task_session_binding
            .isolation_policy,
        SessionIsolationPolicy::UserAttestedFreshConversationPerTask
    );

    let mut forged = plan.clone();
    forged.suite_content_sha256 = "0".repeat(64);
    assert!(build_batch_schedule(&forged).is_err());

    let mut forged_policy = plan.clone();
    forged_policy.task_session_policy_version += 1;
    assert!(build_batch_schedule(&forged_policy).is_err());

    let mut forged_schedule_policy = plan;
    forged_schedule_policy.schedule_policy_version += 1;
    assert!(build_batch_schedule(&forged_schedule_policy).is_err());
}
