use ability_core::{
    AdapterLaunchKind, BatchCostPolicy, BatchExecutionSurface, BatchMode, EnvironmentFingerprint,
    ExecutionAdapterIdentity, LoadedPack, ModelSource, ModelVerification, PackLoader,
    ScanBatchPlan, ScanBatchTarget, TargetKind, TargetRouteIdentity, TargetSelection,
};
use chrono::{Duration, TimeZone, Utc};
use std::path::PathBuf;

fn issued_at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 23, 10, 0, 0)
        .single()
        .unwrap()
}

const CLIENT_PACK_HASH: &str = "cfd2b36af1688432626ee80e453d60cd1d8cb4f87371df5f53def6b551e06f8f";
const CLI_PACK_HASH: &str = "c52c76d1b562812909e88dd71a2f3c70ef874fd795f84c91017b94ad3bb01fea";

fn official_pack(directory: &str) -> LoadedPack {
    PackLoader::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../benchmark-packs")
            .join(directory),
    )
    .unwrap()
}

fn guided_pack() -> LoadedPack {
    official_pack("client-quick-v1")
}

fn cli_pack() -> LoadedPack {
    official_pack("cli-quick-v1")
}

fn target(
    kind: TargetKind,
    model: &str,
    effort: Option<&str>,
    source: ModelSource,
    verification: ModelVerification,
) -> TargetSelection {
    TargetSelection {
        kind,
        reported_model: model.into(),
        reasoning_effort: effort.map(str::to_owned),
        model_source: source,
        model_verification: verification,
    }
}

fn guided_adapter() -> ExecutionAdapterIdentity {
    ExecutionAdapterIdentity::new(
        BatchExecutionSurface::GuidedClient,
        "openai",
        AdapterLaunchKind::GuidedClient,
        Some("1.2.3"),
        "guided-client-v1",
    )
    .unwrap()
}

fn cli_adapter(version: &str) -> ExecutionAdapterIdentity {
    ExecutionAdapterIdentity::new(
        BatchExecutionSurface::AutomatedCli,
        "openai",
        AdapterLaunchKind::NativeExe,
        Some("codex 1.2.3"),
        version,
    )
    .unwrap()
}

fn guided_target(model: &str) -> ScanBatchTarget {
    ScanBatchTarget::new(
        target(
            TargetKind::ChatGptClient,
            model,
            Some("High"),
            ModelSource::Manual,
            ModelVerification::UserConfirmed,
        ),
        BatchExecutionSurface::GuidedClient,
        guided_adapter(),
    )
    .unwrap()
}

fn cli_target(model: &str) -> ScanBatchTarget {
    ScanBatchTarget::new(
        target(
            TargetKind::CodexCli,
            model,
            Some("HIGH"),
            ModelSource::CliRequested,
            ModelVerification::UserConfirmed,
        ),
        BatchExecutionSurface::AutomatedCli,
        cli_adapter("cli-adapter-v1"),
    )
    .unwrap()
}

#[test]
fn cost_policy_v1_exact_boundaries() {
    let policy = BatchCostPolicy::v1();
    let guided = policy
        .estimate(
            &guided_pack(),
            BatchExecutionSurface::GuidedClient,
            BatchMode::QuickComparison,
            4,
            issued_at(),
        )
        .unwrap();
    assert_eq!(guided.repetitions_per_target, 1);
    assert_eq!(guided.planned_member_runs, 4);
    assert_eq!(guided.task_launches, 32);
    assert_eq!(guided.guided_interactions, 32);
    assert_eq!(guided.max_provider_turns, 32);
    assert_eq!(guided.summed_task_budget_secs, 4_320);
    assert_eq!(guided.authorization_wall_clock_secs, 4 * 60 * 60);

    let rows = [
        (BatchMode::QuickComparison, 4, 4, 8, 160, 14_400, 8),
        (BatchMode::Standard, 4, 12, 24, 480, 43_200, 24),
        (BatchMode::Full, 5, 25, 50, 1_000, 90_000, 72),
    ];
    for (mode, targets, members, launches, turns, seconds, hours) in rows {
        let estimate = policy
            .estimate(
                &cli_pack(),
                BatchExecutionSurface::AutomatedCli,
                mode,
                targets,
                issued_at(),
            )
            .unwrap();
        assert_eq!(estimate.planned_member_runs, members);
        assert_eq!(estimate.task_launches, launches);
        assert_eq!(estimate.guided_interactions, 0);
        assert_eq!(estimate.max_provider_turns, turns);
        assert_eq!(estimate.summed_task_budget_secs, seconds);
        assert_eq!(estimate.authorization_wall_clock_secs, hours * 60 * 60);
    }
}

#[test]
fn cost_policy_rejects_unsupported_modes_counts_caps_and_overflow() {
    let policy = BatchCostPolicy::v1();
    for mode in [BatchMode::Standard, BatchMode::Full] {
        assert!(
            policy
                .estimate(
                    &guided_pack(),
                    BatchExecutionSurface::GuidedClient,
                    mode,
                    2,
                    issued_at(),
                )
                .is_err()
        );
    }
    assert!(
        policy
            .estimate(
                &cli_pack(),
                BatchExecutionSurface::AutomatedCli,
                BatchMode::Full,
                6,
                issued_at(),
            )
            .is_err()
    );
    let mut forged = cli_pack();
    forged.tasks[0].definition.time_budget_secs = u64::MAX;
    assert!(
        policy
            .estimate(
                &forged,
                BatchExecutionSurface::AutomatedCli,
                BatchMode::QuickComparison,
                2,
                issued_at(),
            )
            .is_err()
    );
}

#[test]
fn estimates_use_exact_checked_formulas_and_unknown_quota() {
    let estimate = BatchCostPolicy::v1()
        .estimate(
            &cli_pack(),
            BatchExecutionSurface::AutomatedCli,
            BatchMode::Standard,
            2,
            issued_at(),
        )
        .unwrap();
    assert_eq!(estimate.repetitions_per_target, 3);
    assert_eq!(estimate.planned_member_runs, 6);
    assert_eq!(estimate.task_launches, 12);
    assert_eq!(estimate.max_provider_turns, 240);
    assert_eq!(estimate.summed_task_budget_secs, 21_600);
    assert_eq!(estimate.expected_elapsed_secs_min, 6 * 30 * 60);
    assert_eq!(estimate.expected_elapsed_secs_max, 6 * 60 * 60);
    assert_eq!(estimate.provider_execution_ceiling_secs, 21_600 + 6 * 300);
    assert_eq!(estimate.token_quota_amount, None);
    assert_eq!(estimate.automatic_retry_budget, 0);
}

#[test]
fn acknowledgement_and_execution_expiry_use_distinct_clocks() {
    let estimate = BatchCostPolicy::v1()
        .estimate(
            &cli_pack(),
            BatchExecutionSurface::AutomatedCli,
            BatchMode::QuickComparison,
            2,
            issued_at(),
        )
        .unwrap();
    assert_eq!(
        estimate.initial_acknowledgement_expires_at,
        issued_at() + Duration::minutes(15)
    );
    let started_at = issued_at() + Duration::minutes(10);
    assert_eq!(
        estimate
            .execution_authorization_expires_at(started_at)
            .unwrap(),
        started_at + Duration::hours(8),
        "pauses do not move the persisted execution deadline"
    );
}

#[test]
fn rejects_mixed_surface_cohort() {
    let error = ScanBatchPlan::new(
        &cli_pack(),
        "ability-v1",
        BatchMode::QuickComparison,
        7,
        vec![guided_target("gpt-5"), cli_target("gpt-5")],
        issued_at(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("surface"));
}

#[test]
fn cost_policy_v1_is_bound_to_the_shipped_sealed_packs() {
    assert_eq!(cli_pack().content_sha256, CLI_PACK_HASH);
    assert!(
        ScanBatchPlan::new(
            &cli_pack(),
            "ability-v1",
            BatchMode::QuickComparison,
            7,
            vec![cli_target("gpt-5"), cli_target("gpt-5-mini")],
            issued_at(),
        )
        .is_ok()
    );
    let mut wrong_id = cli_pack();
    wrong_id.manifest.id = "other-pack".into();
    let mut wrong_version = cli_pack();
    wrong_version.manifest.version = "1.0.1".into();
    let mut wrong_hash = cli_pack();
    wrong_hash.content_sha256 = CLIENT_PACK_HASH.into();
    for pack in [wrong_id, wrong_version, wrong_hash] {
        assert!(
            ScanBatchPlan::new(
                &pack,
                "ability-v1",
                BatchMode::QuickComparison,
                7,
                vec![cli_target("gpt-5"), cli_target("gpt-5-mini")],
                issued_at(),
            )
            .is_err()
        );
    }
}

#[test]
fn plans_derive_budgets_from_verified_client_and_cli_packs() {
    let guided = ScanBatchPlan::new(
        &guided_pack(),
        "ability-v1",
        BatchMode::QuickComparison,
        7,
        vec![guided_target("gpt-5"), guided_target("gpt-5-mini")],
        issued_at(),
    )
    .unwrap();
    assert_eq!(guided.cost_estimate.task_launches, 16);
    assert_eq!(guided.cost_estimate.max_provider_turns, 16);
    assert_eq!(guided.cost_estimate.summed_task_budget_secs, 2_160);

    let cli = ScanBatchPlan::new(
        &cli_pack(),
        "ability-v1",
        BatchMode::QuickComparison,
        7,
        vec![cli_target("gpt-5"), cli_target("gpt-5-mini")],
        issued_at(),
    )
    .unwrap();
    assert_eq!(cli.cost_estimate.task_launches, 4);
    assert_eq!(cli.cost_estimate.max_provider_turns, 80);
    assert_eq!(cli.cost_estimate.summed_task_budget_secs, 7_200);
}

#[test]
fn rejects_understated_and_overstated_budgets_for_both_shipped_hashes() {
    for (task_index, time_budget, max_turns) in [(0, 119, 1), (0, 121, 1), (0, 120, 0), (0, 120, 2)]
    {
        let mut pack = guided_pack();
        assert_eq!(pack.content_sha256, CLIENT_PACK_HASH);
        pack.tasks[task_index].definition.time_budget_secs = time_budget;
        pack.tasks[task_index].definition.max_turns = max_turns;
        assert!(
            ScanBatchPlan::new(
                &pack,
                "ability-v1",
                BatchMode::QuickComparison,
                7,
                vec![guided_target("gpt-5"), guided_target("gpt-5-mini")],
                issued_at(),
            )
            .is_err()
        );
    }

    for (time_budget, max_turns) in [(1_799, 20), (1_801, 20), (1_800, 19), (1_800, 21)] {
        let mut pack = cli_pack();
        assert_eq!(pack.content_sha256, CLI_PACK_HASH);
        pack.tasks[0].definition.time_budget_secs = time_budget;
        pack.tasks[0].definition.max_turns = max_turns;
        assert!(
            ScanBatchPlan::new(
                &pack,
                "ability-v1",
                BatchMode::QuickComparison,
                7,
                vec![cli_target("gpt-5"), cli_target("gpt-5-mini")],
                issued_at(),
            )
            .is_err()
        );
    }
}

#[test]
fn forged_batch_target_fields_fail_reconstruction_validation() {
    let canonical = cli_target("gpt-5");
    let mut forgeries = Vec::new();

    let mut value = canonical.clone();
    value.route_identity.model_or_route = "gpt-5-mini".into();
    forgeries.push(value);

    let mut value = canonical.clone();
    value.route_identity.reasoning_effort = Some("low".into());
    forgeries.push(value);

    let mut value = canonical.clone();
    value.route_identity.is_default_route = true;
    forgeries.push(value);

    let mut value = canonical.clone();
    value.execution_adapter_identity.provider_family = "anthropic".into();
    forgeries.push(value);

    let mut value = canonical.clone();
    value.execution_adapter_identity.launch_kind = AdapterLaunchKind::GuidedClient;
    forgeries.push(value);

    let mut value = canonical.clone();
    value.execution_adapter_identity.adapter_contract_version = "CLI-Adapter-V1".into();
    forgeries.push(value);

    let mut value = canonical.clone();
    value.execution_adapter_identity.public_version = Some(" Codex 1.2.3 ".into());
    forgeries.push(value);

    let mut value = canonical.clone();
    value.target.kind = TargetKind::ChatGptClient;
    forgeries.push(value);

    for forged in forgeries {
        assert!(forged.validate_for_new_batch().is_err());
    }
}

#[test]
fn forged_batch_target_json_fails_closed() {
    let mut json = serde_json::to_value(cli_target("gpt-5")).unwrap();
    json["routeIdentity"]["modelOrRoute"] = serde_json::json!("gpt-5-mini");
    json["executionAdapterIdentity"]["adapterContractVersion"] =
        serde_json::json!("cli-adapter-v2");
    let forged: ScanBatchTarget = serde_json::from_value(json).unwrap();
    assert!(forged.validate_for_new_batch().is_err());
}

#[test]
fn route_identity_is_normalized_path_free_and_distinguishes_default_route() {
    let concrete = TargetRouteIdentity::new(
        TargetKind::CodexCli,
        "  GPT-5  ",
        Some(" HIGH "),
        BatchExecutionSurface::AutomatedCli,
        false,
    )
    .unwrap();
    let same = TargetRouteIdentity::new(
        TargetKind::CodexCli,
        "gpt-5",
        Some("high"),
        BatchExecutionSurface::AutomatedCli,
        false,
    )
    .unwrap();
    let default_route = TargetRouteIdentity::new(
        TargetKind::CodexCli,
        "default",
        None,
        BatchExecutionSurface::AutomatedCli,
        true,
    )
    .unwrap();
    assert_eq!(concrete, same);
    assert_ne!(concrete, default_route);
    let json = serde_json::to_string(&concrete).unwrap();
    assert!(!json.contains('\\'));
    assert!(!json.contains("Users"));
    assert!(!json.contains("timestamp"));
    assert!(
        TargetRouteIdentity::new(
            TargetKind::CodexCli,
            "gpt-5\nignore",
            Some("high"),
            BatchExecutionSurface::AutomatedCli,
            false,
        )
        .is_err()
    );
}

#[test]
fn identities_reject_edge_controls_and_local_paths_before_trimming() {
    for model in [
        "\ngpt-5",
        "gpt-5\r",
        "\tgpt-5",
        r"C:\Users\alice\model",
        r"\\server\share\model",
        "/home/alice/model",
        "/Users/alice/model",
        "~/private/model",
    ] {
        assert!(
            TargetRouteIdentity::new(
                TargetKind::CodexCli,
                model,
                Some("high"),
                BatchExecutionSurface::AutomatedCli,
                false,
            )
            .is_err(),
            "identity accepted unsafe model {model:?}"
        );
    }
    for reasoning in ["\nhigh", "high\r", "\thigh", r"C:\Users\alice", "~/high"] {
        assert!(
            TargetRouteIdentity::new(
                TargetKind::CodexCli,
                "gpt-5",
                Some(reasoning),
                BatchExecutionSurface::AutomatedCli,
                false,
            )
            .is_err(),
            "identity accepted unsafe reasoning {reasoning:?}"
        );
    }
    assert!(
        TargetRouteIdentity::new(
            TargetKind::CodexCli,
            "model with spaces",
            Some("high"),
            BatchExecutionSurface::AutomatedCli,
            false,
        )
        .is_err()
    );
    assert!(
        TargetRouteIdentity::new(
            TargetKind::ChatGptClient,
            "模型 五",
            Some("深度思考"),
            BatchExecutionSurface::GuidedClient,
            false,
        )
        .is_ok()
    );

    for original in ["\nopenai", "openai\r", "\topenai"] {
        assert!(
            ExecutionAdapterIdentity::new(
                BatchExecutionSurface::AutomatedCli,
                original,
                AdapterLaunchKind::NativeExe,
                Some("codex 1.2.3"),
                "cli-adapter-v1",
            )
            .is_err()
        );
    }
}

#[test]
fn route_identity_ignores_serialization_order_and_preserves_provenance() {
    let manual: TargetSelection = serde_json::from_str(
        r#"{"kind":"chat_gpt_client","reportedModel":"GPT-5","reasoningEffort":"High","modelSource":"manual","modelVerification":"user_confirmed"}"#,
    )
    .unwrap();
    let detected: TargetSelection = serde_json::from_str(
        r#"{"modelVerification":"user_confirmed","reasoningEffort":"high","reportedModel":"gpt-5","kind":"chat_gpt_client","modelSource":"windows_accessibility"}"#,
    )
    .unwrap();
    let first = ScanBatchTarget::new(
        manual,
        BatchExecutionSurface::GuidedClient,
        guided_adapter(),
    )
    .unwrap();
    let second = ScanBatchTarget::new(
        detected,
        BatchExecutionSurface::GuidedClient,
        guided_adapter(),
    )
    .unwrap();
    assert_eq!(first.route_identity, second.route_identity);
    assert_ne!(first.target.model_source, second.target.model_source);
}

#[test]
fn accepted_provenance_classes_are_surface_specific() {
    assert!(guided_target("gpt-5").validate_for_new_batch().is_ok());
    assert!(
        ScanBatchTarget::new(
            target(
                TargetKind::ClaudeClient,
                "claude-sonnet",
                None,
                ModelSource::WindowsAccessibility,
                ModelVerification::UserConfirmed,
            ),
            BatchExecutionSurface::GuidedClient,
            ExecutionAdapterIdentity::new(
                BatchExecutionSurface::GuidedClient,
                "anthropic",
                AdapterLaunchKind::GuidedClient,
                None,
                "guided-client-v1",
            )
            .unwrap(),
        )
        .is_ok()
    );
    assert!(cli_target("gpt-5").validate_for_new_batch().is_ok());
    assert!(
        ScanBatchTarget::new(
            target(
                TargetKind::CodexCli,
                "gpt-5",
                None,
                ModelSource::Manual,
                ModelVerification::UserConfirmed,
            ),
            BatchExecutionSurface::AutomatedCli,
            cli_adapter("cli-adapter-v1"),
        )
        .is_err()
    );
}

#[test]
fn adapter_identity_is_path_free() {
    let normalized = ExecutionAdapterIdentity::new(
        BatchExecutionSurface::AutomatedCli,
        " OpenAI ",
        AdapterLaunchKind::ReviewedNpm,
        Some(" Codex 1.2.3 "),
        " CLI-Adapter-V1 ",
    )
    .unwrap();
    assert_eq!(normalized.provider_family, "openai");
    assert_eq!(normalized.public_version.as_deref(), Some("codex 1.2.3"));
    assert_eq!(normalized.adapter_contract_version, "cli-adapter-v1");
    assert!(normalized.compatible_with(&normalized));

    for path in [
        r#"C:\Users\alice\AppData\npm\codex.cmd"#,
        "/home/alice/.local/bin/codex",
        "@openai/codex",
        "codex\n1.2.3",
    ] {
        assert!(
            ExecutionAdapterIdentity::new(
                BatchExecutionSurface::AutomatedCli,
                "openai",
                AdapterLaunchKind::NativeExe,
                Some(path),
                "cli-adapter-v1",
            )
            .is_err()
        );
    }
    let json = serde_json::to_string(&normalized).unwrap();
    for forbidden in ["path", "user", "package", "rawLabel", "timestamp"] {
        assert!(!json.contains(forbidden));
    }
}

#[test]
fn decoded_estimates_are_validated_and_authorization_expiry_is_checked() {
    let pack = cli_pack();
    let estimate = BatchCostPolicy::v1()
        .estimate(
            &pack,
            BatchExecutionSurface::AutomatedCli,
            BatchMode::QuickComparison,
            2,
            issued_at(),
        )
        .unwrap();
    assert!(estimate.validate_against_pack(&pack).is_ok());
    assert!(
        estimate
            .execution_authorization_expires_at(chrono::DateTime::<Utc>::MAX_UTC)
            .is_err()
    );

    let mut json = serde_json::to_value(&estimate).unwrap();
    json["taskLaunches"] = serde_json::json!(0);
    let understated: ability_core::BatchCostEstimate = serde_json::from_value(json).unwrap();
    assert!(understated.validate_against_pack(&pack).is_err());
    assert!(
        understated
            .execution_authorization_expires_at(issued_at())
            .is_err()
    );

    let mut json = serde_json::to_value(&estimate).unwrap();
    json["authorizationWallClockSecs"] = serde_json::json!(u64::MAX);
    let out_of_range: ability_core::BatchCostEstimate = serde_json::from_value(json).unwrap();
    assert!(out_of_range.validate_against_pack(&pack).is_err());
    assert!(
        out_of_range
            .execution_authorization_expires_at(issued_at())
            .is_err()
    );
}

#[test]
fn legacy_environment_defaults_adapter_to_absent_and_batch_requires_it() {
    let legacy = r#"{
        "osFamily":"Windows","osVersion":"11","appVersion":"0.2.2",
        "cliVersion":null,"verifierRuntimeVersion":null,"suiteId":"cli-quick",
        "suiteVersion":"1.0.0","suiteContentSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "scoringRuleVersion":"ability-v1","resumed":false
    }"#;
    let environment: EnvironmentFingerprint = serde_json::from_str(legacy).unwrap();
    assert_eq!(environment.execution_adapter_identity, None);
    assert!(
        environment
            .require_batch_adapter(&cli_adapter("cli-adapter-v1"))
            .is_err()
    );
    assert!(
        !serde_json::to_string(&environment)
            .unwrap()
            .contains("executionAdapterIdentity")
    );
}

#[test]
fn batch_resume_rejects_incompatible_adapter_contract() {
    let expected = cli_adapter("cli-adapter-v1");
    let mut environment: EnvironmentFingerprint = serde_json::from_str(
        r#"{"osFamily":"Windows","osVersion":"11","appVersion":"0.2.2","cliVersion":"codex 1.2.3","verifierRuntimeVersion":null,"suiteId":"cli-quick","suiteVersion":"1.0.0","suiteContentSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","scoringRuleVersion":"ability-v1","resumed":false}"#,
    )
    .unwrap();
    environment.execution_adapter_identity = Some(expected.clone());
    assert!(environment.require_batch_adapter(&expected).is_ok());
    assert!(
        environment
            .require_batch_adapter(&cli_adapter("cli-adapter-v2"))
            .is_err()
    );
}

#[test]
fn duplicate_route_identities_are_rejected() {
    let error = ScanBatchPlan::new(
        &cli_pack(),
        "ability-v1",
        BatchMode::QuickComparison,
        1,
        vec![cli_target("GPT-5"), cli_target(" gpt-5 ")],
        issued_at(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate"));
}

#[test]
fn every_plan_mutation_changes_the_acknowledgement_hash() {
    let plan = ScanBatchPlan::new(
        &cli_pack(),
        "ability-v1",
        BatchMode::QuickComparison,
        42,
        vec![cli_target("gpt-5"), cli_target("gpt-5-mini")],
        issued_at(),
    )
    .unwrap();
    let mutations = [
        ScanBatchPlan::new(
            &cli_pack(),
            "ability-v2",
            BatchMode::QuickComparison,
            42,
            plan.targets.clone(),
            issued_at(),
        )
        .unwrap(),
        ScanBatchPlan::new(
            &cli_pack(),
            "ability-v1",
            BatchMode::QuickComparison,
            43,
            plan.targets.clone(),
            issued_at(),
        )
        .unwrap(),
        ScanBatchPlan::new(
            &cli_pack(),
            "ability-v1",
            BatchMode::QuickComparison,
            42,
            plan.targets.iter().cloned().rev().collect(),
            issued_at(),
        )
        .unwrap(),
        ScanBatchPlan::new(
            &cli_pack(),
            "ability-v1",
            BatchMode::Standard,
            42,
            plan.targets.clone(),
            issued_at(),
        )
        .unwrap(),
    ];
    for mutation in mutations {
        assert_ne!(plan.acknowledgement_hash, mutation.acknowledgement_hash);
    }
}
