use ability_core::{
    Category, EnvironmentFingerprint, FailureKind, ModelSource, ModelVerification, PublicReport,
    ReportError, RunMode, RunRecord, RunStatus, ScoreSummary, TargetKind, TargetSelection,
    TaskOutcome, TaskResult, build_public_report, render_public_report_html,
    validate_public_report,
};
use std::collections::BTreeMap;

fn sample_evidence(model: &str) -> (RunRecord, Vec<TaskResult>) {
    let mut run = RunRecord::new(
        TargetSelection {
            kind: TargetKind::CodexCli,
            reported_model: model.into(),
            reasoning_effort: Some("high".into()),
            model_source: ModelSource::LegacyUnknown,
            model_verification: ModelVerification::LegacyUnknown,
        },
        RunMode::Quick,
        "cli-quick".into(),
        "1.0.0".into(),
        2,
        EnvironmentFingerprint {
            os_family: "Windows".into(),
            os_version: "11 Pro 22631".into(),
            app_version: "0.2.0".into(),
            cli_version: Some("codex-cli 1.2.3".into()),
            verifier_runtime_version: Some("v22.0.0".into()),
            suite_id: "cli-quick".into(),
            suite_version: "1.0.0".into(),
            suite_content_sha256: "f".repeat(64),
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
        category_scores: BTreeMap::from([(Category::CliCoding, 50.0)]),
    });
    let tasks = vec![
        TaskResult {
            run_id: run.id,
            task_id: "private-pass-id".into(),
            category: Category::CliCoding,
            outcome: TaskOutcome::Passed,
            score: Some(100.0),
            failure_kind: None,
            duration_ms: 1_000,
            answer_rel_path: Some("runs/private/answers/one.txt".into()),
            detail: "secret raw answer from C:\\Users\\Alice".into(),
        },
        TaskResult {
            run_id: run.id,
            task_id: "private-fail-id".into(),
            category: Category::CliCoding,
            outcome: TaskOutcome::Failed,
            score: Some(0.0),
            failure_kind: Some(FailureKind::WrongAnswer),
            duration_ms: 2_000,
            answer_rel_path: Some("runs/private/logs/secret.log".into()),
            detail: "sk-not-a-real-secret-in-private-detail".into(),
        },
    ];
    (run, tasks)
}

#[test]
fn public_report_is_a_structural_allowlist_not_a_redacted_domain_object() {
    let (run, tasks) = sample_evidence("  CLI current  ");

    let report = build_public_report(&run, &tasks).unwrap();
    let json = serde_json::to_string(&report).unwrap();

    assert_eq!(report.target.reported_model, "CLI current");
    assert_eq!(report.result.total_duration_ms, 3_000);
    assert!(!json.contains(&run.id.to_string()));
    assert!(!json.contains("private-pass-id"));
    assert!(!json.contains("secret raw answer"));
    assert!(!json.contains("runs/private"));
    assert!(!json.contains("Alice"));
    assert!(!json.contains("11 Pro 22631"));
    assert!(!json.contains("osVersion"));
    assert!(!json.contains("answerRelPath"));
    assert!(!json.contains("\"detail\""));
    assert!(json.contains("\"osFamily\":\"Windows\""));
    assert!(json.contains("\"interpretationStatus\":\"not_evaluated\""));
}

#[test]
fn public_report_v2_includes_persisted_model_provenance_and_html_labels() {
    let (mut run, tasks) = sample_evidence("GPT-5.6");
    run.target.kind = TargetKind::ChatGptClient;
    run.target.model_source = ModelSource::WindowsAccessibility;
    run.target.model_verification = ModelVerification::UserConfirmed;

    let report = build_public_report(&run, &tasks).unwrap();
    let json = serde_json::to_value(&report).unwrap();
    let html = render_public_report_html(&report).unwrap();

    assert_eq!(json["schemaVersion"], 2);
    assert_eq!(json["target"]["modelSource"], "windows_accessibility");
    assert_eq!(json["target"]["modelVerification"], "user_confirmed");
    assert!(html.contains("模型来源：Windows 客户端界面 · 用户已确认"));
}

#[test]
fn report_builder_and_validator_accept_only_the_exact_provenance_wire_matrix() {
    let sources = [
        ModelSource::Manual,
        ModelSource::WindowsAccessibility,
        ModelSource::CliRequested,
        ModelSource::CliReported,
        ModelSource::DefaultRoute,
        ModelSource::LegacyUnknown,
    ];
    let verifications = [
        ModelVerification::UserConfirmed,
        ModelVerification::ProviderReported,
        ModelVerification::Unverified,
        ModelVerification::LegacyUnknown,
    ];
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
    let (mut run, tasks) = sample_evidence("GPT-5.6");

    for source in sources {
        for verification in verifications {
            run.target.model_source = source;
            run.target.model_verification = verification;
            let built = build_public_report(&run, &tasks);
            let expected = valid_pairs
                .iter()
                .find(|(valid_source, valid_verification, _, _)| {
                    source == *valid_source && verification == *valid_verification
                });
            assert_eq!(
                built.is_ok(),
                expected.is_some(),
                "builder matrix mismatch for {source:?}/{verification:?}"
            );
            if let Some((_, _, source_wire, verification_wire)) = expected {
                let value = serde_json::to_value(built.as_ref().unwrap()).unwrap();
                assert_eq!(
                    value["target"]["modelSource"], *source_wire,
                    "source wire value mismatch for {source:?}/{verification:?}"
                );
                assert_eq!(
                    value["target"]["modelVerification"], *verification_wire,
                    "verification wire value mismatch for {source:?}/{verification:?}"
                );
            }

            let mut value = serde_json::to_value(
                build_public_report(
                    &{
                        let mut valid_run = run.clone();
                        valid_run.target.model_source = ModelSource::LegacyUnknown;
                        valid_run.target.model_verification = ModelVerification::LegacyUnknown;
                        valid_run
                    },
                    &tasks,
                )
                .unwrap(),
            )
            .unwrap();
            value["target"]["modelSource"] = serde_json::to_value(source).unwrap();
            value["target"]["modelVerification"] = serde_json::to_value(verification).unwrap();
            let report: PublicReport = serde_json::from_value(value).unwrap();
            assert_eq!(
                validate_public_report(&report).is_ok(),
                expected.is_some(),
                "validator matrix mismatch for {source:?}/{verification:?}"
            );
        }
    }
}

#[test]
fn report_id_is_fresh_and_not_derived_from_the_local_run_id() {
    let (run, tasks) = sample_evidence("CLI current");

    let first = build_public_report(&run, &tasks).unwrap();
    let second = build_public_report(&run, &tasks).unwrap();

    assert_ne!(first.report_id, run.id);
    assert_ne!(second.report_id, run.id);
    assert_ne!(first.report_id, second.report_id);
}

#[test]
fn reported_model_requires_visible_unicode_in_built_and_validated_reports() {
    for model in [
        "\u{200b}",
        "\u{202e}",
        "\u{2060}",
        "\u{200b}\u{2060}",
        "GPT\u{200b}-5",
    ] {
        let (run, tasks) = sample_evidence(model);
        assert!(
            matches!(
                build_public_report(&run, &tasks),
                Err(ReportError::InvalidData("reportedModel"))
            ),
            "build accepted {model:?}"
        );

        let (valid_run, valid_tasks) = sample_evidence("模型-α");
        let mut report = build_public_report(&valid_run, &valid_tasks).unwrap();
        report.target.reported_model = model.into();
        assert!(
            matches!(
                validate_public_report(&report),
                Err(ReportError::InvalidData("reportedModel"))
            ),
            "validation accepted {model:?}"
        );
    }

    for model in ["模型-α".to_owned(), "模".repeat(120)] {
        let (run, tasks) = sample_evidence(&model);
        let report = build_public_report(&run, &tasks).unwrap();
        assert_eq!(report.target.reported_model, model);
        validate_public_report(&report).unwrap();
    }

    let too_long = "模".repeat(121);
    let (run, tasks) = sample_evidence(&too_long);
    assert!(matches!(
        build_public_report(&run, &tasks),
        Err(ReportError::InvalidData("reportedModel"))
    ));
}

#[test]
fn suspicious_text_is_rejected_with_the_stable_public_field_label() {
    for model in [
        "sk-ant-api03-not-a-real-token",
        "sk-proj-abcdefghijklmnopqrstuvwxyz",
        r#"C:\Users\爱丽丝\model"#,
        r#"D:/工作/private/model"#,
        r#"\\DESKTOP\共享\model"#,
        r#"\\服务器\共享\model"#,
        "/Users/alice/private/model",
        "/home/张三/private/model",
        "/tmp/private/model",
        "/workspace/张三/private/model",
        "~/private/model",
        "Bearer abcdefghijklmnopqrstuvwxyz",
        "github_pat_abcdefghijklmnopqrstuvwxyz",
        "AKIAABCDEFGHIJKLMNOP",
        "xoxb-123456789012-abcdefghijklmnop",
        "AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ123456",
        "hf_abcdefghijklmnopqrstuvwxyz",
        "npm_abcdefghijklmnopqrstuvwxyz",
        "-----BEGIN PRIVATE KEY-----",
        "alice@example.com",
        "爱丽丝@例子.公司",
        "password = super-secret-value",
    ] {
        let (run, tasks) = sample_evidence(model);
        assert!(
            matches!(
                build_public_report(&run, &tasks),
                Err(ReportError::SensitiveText("reportedModel"))
            ),
            "{model:?} should be rejected"
        );
    }
}

#[test]
fn every_projected_local_string_is_scanned() {
    type Mutator = fn(&mut RunRecord, String);
    let cases: [(&str, &str, Mutator); 9] = [
        ("reasoningEffort", r"C:\Users\Alice\effort", |run, value| {
            run.target.reasoning_effort = Some(value);
        }),
        ("osFamily", "alice@example.com", |run, value| {
            run.environment.os_family = value;
        }),
        (
            "appVersion",
            "Bearer abcdefghijklmnopqrstuvwxyz",
            |run, value| {
                run.environment.app_version = value;
            },
        ),
        ("cliVersion", r"\\HOST\share\cli", |run, value| {
            run.environment.cli_version = Some(value);
        }),
        (
            "verifierRuntimeVersion",
            "/home/alice/node",
            |run, value| {
                run.environment.verifier_runtime_version = Some(value);
            },
        ),
        (
            "suiteId",
            "sk-proj-abcdefghijklmnopqrstuvwxyz",
            |run, value| {
                run.suite_id = value.clone();
                run.environment.suite_id = value;
            },
        ),
        ("suiteVersion", r"D:\private\suite", |run, value| {
            run.suite_version = value.clone();
            run.environment.suite_version = value;
        }),
        (
            "suiteContentSha256",
            "password=super-secret-value",
            |run, value| {
                run.environment.suite_content_sha256 = value;
            },
        ),
        (
            "scoringRuleVersion",
            "github_pat_abcdefghijklmnopqrstuvwxyz",
            |run, value| {
                run.environment.scoring_rule_version = value;
            },
        ),
    ];

    for (field, secret, mutate) in cases {
        let (mut run, tasks) = sample_evidence("CLI current");
        mutate(&mut run, secret.into());
        assert!(
            matches!(
                build_public_report(&run, &tasks),
                Err(ReportError::SensitiveText(actual)) if actual == field
            ),
            "{field} was not scanned"
        );
    }
}

#[test]
fn embedded_absolute_and_network_paths_are_rejected_in_every_projected_field() {
    type Mutator = fn(&mut RunRecord, String);
    let cases: [(&str, &str, Mutator); 10] = [
        (
            "reportedModel",
            "模型位置：/srv/tenant/model.bin",
            |run, value| run.target.reported_model = value,
        ),
        (
            "reasoningEffort",
            "effort cache at /data/ability-radar/state.json",
            |run, value| run.target.reasoning_effort = Some(value),
        ),
        (
            "osFamily",
            "Windows image from //server/share/images/os.txt",
            |run, value| run.environment.os_family = value,
        ),
        (
            "appVersion",
            r"0.2.0 from \\server/share\releases/app.exe",
            |run, value| run.environment.app_version = value,
        ),
        (
            "cliVersion",
            r"cli at //server\share/tools/cli.exe",
            |run, value| run.environment.cli_version = Some(value),
        ),
        (
            "verifierRuntimeVersion",
            "runtime at /srv/verifier/bin/node.exe",
            |run, value| run.environment.verifier_runtime_version = Some(value),
        ),
        (
            "suiteId",
            "suite copied from /data/packs/current/suite",
            |run, value| {
                run.suite_id = value.clone();
                run.environment.suite_id = value;
            },
        ),
        (
            "suiteVersion",
            r"version from \\server/share\releases/version.txt",
            |run, value| {
                run.suite_version = value.clone();
                run.environment.suite_version = value;
            },
        ),
        (
            "suiteContentSha256",
            "hash read from //server/share/hashes/current.txt",
            |run, value| run.environment.suite_content_sha256 = value,
        ),
        (
            "scoringRuleVersion",
            "rule loaded from /srv/scoring/rules/current.json",
            |run, value| run.environment.scoring_rule_version = value,
        ),
    ];

    for (field, private_path, mutate) in cases {
        let (mut run, tasks) = sample_evidence("CLI current");
        mutate(&mut run, private_path.into());

        let error = build_public_report(&run, &tasks).unwrap_err();

        assert!(
            matches!(error, ReportError::SensitiveText(actual) if actual == field),
            "{field} did not reject {private_path:?}"
        );
        assert!(
            !error.to_string().contains(private_path),
            "{field} echoed the rejected source text"
        );
    }
}

#[test]
fn obvious_urls_model_names_and_version_fractions_are_not_paths() {
    let (mut run, tasks) = sample_evidence("openai/gpt-5.1-codex");
    run.environment.app_version =
        "0.2.0 docs https://example.com/releases/ability-radar/v1.2".into();
    run.environment.cli_version = Some("codex-cli v1/2 compatibility".into());
    run.environment.verifier_runtime_version = Some("Node.js 22/24 LTS".into());

    let report = build_public_report(&run, &tasks).unwrap();

    assert_eq!(report.target.reported_model, "openai/gpt-5.1-codex");
    assert_eq!(
        report.environment.cli_version.as_deref(),
        Some("codex-cli v1/2 compatibility")
    );
}

#[test]
fn absolute_paths_after_colon_are_rejected_without_blocking_web_urls() {
    for private_path in [
        "metadata:path:/srv/tenant/model.bin",
        r"metadata:path:C:\Users\Alice\model.bin",
        "metadata:file:///home/alice/model.bin",
        "metadata:path:http-like:/srv/tenant/model.bin",
    ] {
        let (run, tasks) = sample_evidence(private_path);

        let error = build_public_report(&run, &tasks).unwrap_err();

        assert!(matches!(error, ReportError::SensitiveText("reportedModel")));
        assert!(!error.to_string().contains(private_path));
    }

    for web_url in [
        "docs https://example.com/models/current",
        "docs HTTPS://example.com/models/current",
    ] {
        let (run, tasks) = sample_evidence(web_url);
        assert!(build_public_report(&run, &tasks).is_ok());
    }
}

#[test]
fn html_is_fully_offline_and_escapes_visible_and_embedded_json_text() {
    let (run, tasks) = sample_evidence(r#"<Model & "Test"> </script><img src=x>"#);

    let report = build_public_report(&run, &tasks).unwrap();
    let html = render_public_report_html(&report).unwrap();

    assert!(html.contains("&lt;Model &amp; &quot;Test&quot;&gt;"));
    assert!(!html.contains(r#"<img src=x>"#));
    assert!(!html.contains(r#"</script><img"#));
    assert!(html.contains(r#"\u003c/script\u003e\u003cimg src=x\u003e"#));
    assert!(html.contains("v0.2 不生成降智结论"));
    assert!(html.contains("不是 IQ"));
    assert!(html.contains("不代表模型退化"));
    assert!(!html.contains("src=\"http"));
    assert!(!html.contains("href=\"http"));
    assert!(!html.contains("https://"));
    assert!(!html.contains("http://"));
}

#[test]
fn html_report_translates_known_efforts_but_json_stays_canonical() {
    let (mut run, tasks) = sample_evidence("GPT-5.6");
    run.target.kind = TargetKind::ChatGptClient;
    run.target.reasoning_effort = Some("xhigh".into());

    let report = build_public_report(&run, &tasks).unwrap();
    assert_eq!(report.target.reasoning_effort.as_deref(), Some("xhigh"));
    let html = render_public_report_html(&report).unwrap();
    assert!(html.contains("\u{63a8}\u{7406}\u{6863}\u{4f4d}\u{ff1a}\u{6781}\u{9ad8}"));

    run.target.kind = TargetKind::ChatGptClient;
    run.target.reasoning_effort = Some("low".into());
    let report = build_public_report(&run, &tasks).unwrap();
    assert!(
        render_public_report_html(&report)
            .unwrap()
            .contains("\u{63a8}\u{7406}\u{6863}\u{4f4d}\u{ff1a}\u{8f7b}\u{5ea6}")
    );
}

#[test]
fn html_report_uses_the_complete_target_specific_effort_display_table() {
    let cases = [
        (TargetKind::ChatGptClient, "none", "无"),
        (TargetKind::ChatGptClient, "minimal", "最小"),
        (TargetKind::ChatGptClient, "low", "轻度"),
        (TargetKind::ChatGptClient, "medium", "中"),
        (TargetKind::ChatGptClient, "high", "高"),
        (TargetKind::ChatGptClient, "xhigh", "极高"),
        (TargetKind::ChatGptClient, "max", "最高"),
        (TargetKind::ChatGptClient, "ultra", "Ultra"),
        (TargetKind::ClaudeClient, "none", "无"),
        (TargetKind::ClaudeClient, "minimal", "最小"),
        (TargetKind::ClaudeClient, "low", "低"),
        (TargetKind::ClaudeClient, "medium", "中"),
        (TargetKind::ClaudeClient, "high", "高"),
        (TargetKind::ClaudeClient, "xhigh", "极高"),
        (TargetKind::ClaudeClient, "max", "最高"),
        (TargetKind::ClaudeClient, "ultra", "Ultra"),
        (TargetKind::CodexCli, "none", "无"),
        (TargetKind::CodexCli, "minimal", "最小"),
        (TargetKind::CodexCli, "low", "低"),
        (TargetKind::CodexCli, "medium", "中"),
        (TargetKind::CodexCli, "high", "高"),
        (TargetKind::CodexCli, "xhigh", "极高"),
        (TargetKind::CodexCli, "max", "最高"),
        (TargetKind::CodexCli, "ultra", "Ultra"),
        (TargetKind::ClaudeCode, "none", "无"),
        (TargetKind::ClaudeCode, "minimal", "最小"),
        (TargetKind::ClaudeCode, "low", "低"),
        (TargetKind::ClaudeCode, "medium", "中"),
        (TargetKind::ClaudeCode, "high", "高"),
        (TargetKind::ClaudeCode, "xhigh", "极高"),
        (TargetKind::ClaudeCode, "max", "最高"),
        (TargetKind::ClaudeCode, "ultra", "Ultra"),
    ];

    for (kind, canonical, expected_label) in cases {
        let (mut run, tasks) = sample_evidence("GPT-5.6");
        run.target.kind = kind;
        run.target.reasoning_effort = Some(canonical.into());

        let report = build_public_report(&run, &tasks).unwrap();
        assert_eq!(
            report.target.reasoning_effort.as_deref(),
            Some(canonical),
            "{kind:?}/{canonical}"
        );
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            json.contains(&format!(r#""reasoningEffort":"{canonical}""#)),
            "{kind:?}/{canonical} JSON was not canonical"
        );
        let html = render_public_report_html(&report).unwrap();
        assert!(
            html.contains(&format!("<p>推理档位：{expected_label}</p>")),
            "{kind:?}/{canonical} did not render {expected_label:?}"
        );
    }
}

#[test]
fn legacy_custom_effort_with_forbidden_display_text_fails_public_reports_closed() {
    for effort in [
        "\u{200b}",
        "\u{202e}",
        "\u{2060}",
        "\u{200b}\u{2060}",
        "\u{6269}\u{200b}\u{5c55}",
    ] {
        let (mut run, tasks) = sample_evidence("GPT-5.6");
        run.target.reasoning_effort = Some(effort.into());
        assert!(
            matches!(
                build_public_report(&run, &tasks),
                Err(ReportError::InvalidData("reasoningEffort"))
            ),
            "build accepted {effort:?}"
        );

        let (valid_run, valid_tasks) = sample_evidence("GPT-5.6");
        let mut report = build_public_report(&valid_run, &valid_tasks).unwrap();
        report.target.reasoning_effort = Some(effort.into());
        assert!(
            matches!(
                validate_public_report(&report),
                Err(ReportError::InvalidData("reasoningEffort"))
            ),
            "validation accepted {effort:?}"
        );
        assert!(
            matches!(
                render_public_report_html(&report),
                Err(ReportError::InvalidData("reasoningEffort"))
            ),
            "HTML accepted {effort:?}"
        );
    }
}

#[test]
fn html_report_uses_target_kind_for_the_default_model_sentinel() {
    let cases = [
        (TargetKind::ChatGptClient, "ChatGPT 客户端", "default"),
        (TargetKind::ClaudeClient, "Claude 客户端", "default"),
        (
            TargetKind::CodexCli,
            "Codex CLI",
            "\u{9ed8}\u{8ba4}\u{8def}\u{7531}\u{ff08}\u{672a}\u{56fa}\u{5b9a}\u{ff09}",
        ),
        (
            TargetKind::ClaudeCode,
            "Claude Code",
            "\u{9ed8}\u{8ba4}\u{8def}\u{7531}\u{ff08}\u{672a}\u{56fa}\u{5b9a}\u{ff09}",
        ),
    ];

    for (kind, target_label, model_label) in cases {
        let (mut run, tasks) = sample_evidence("default");
        run.target.kind = kind;

        let report = build_public_report(&run, &tasks).unwrap();
        assert_eq!(report.target.reported_model, "default");
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""reportedModel":"default""#));
        let html = render_public_report_html(&report).unwrap();
        assert!(
            html.contains(&format!("<h1>{target_label} · {model_label}</h1>")),
            "{kind:?} rendered the wrong model label"
        );
    }
}

#[test]
fn html_report_preserves_and_escapes_custom_effort_labels() {
    let (mut run, tasks) = sample_evidence("Claude");
    run.target.kind = TargetKind::ClaudeClient;
    run.target.reasoning_effort = Some("<\u{6269}\u{5c55}\u{601d}\u{8003}>".into());

    let report = build_public_report(&run, &tasks).unwrap();
    let html = render_public_report_html(&report).unwrap();
    assert!(html.contains(
        "\u{63a8}\u{7406}\u{6863}\u{4f4d}\u{ff1a}&lt;\u{6269}\u{5c55}\u{601d}\u{8003}&gt;"
    ));
    assert!(
        !html
            .contains("\u{63a8}\u{7406}\u{6863}\u{4f4d}\u{ff1a}<\u{6269}\u{5c55}\u{601d}\u{8003}>")
    );
}

#[test]
fn report_builder_rejects_incoherent_completed_evidence() {
    let (run, tasks) = sample_evidence("CLI current");

    let mut partial = run.clone();
    partial.completed_tasks = 1;
    assert!(matches!(
        build_public_report(&partial, &tasks),
        Err(ReportError::InvalidData("completedTasks"))
    ));

    assert!(matches!(
        build_public_report(&run, &tasks[..1]),
        Err(ReportError::InvalidData("taskResults"))
    ));

    let mut wrong_run = tasks.clone();
    wrong_run[0].run_id = uuid::Uuid::new_v4();
    assert!(matches!(
        build_public_report(&run, &wrong_run),
        Err(ReportError::InvalidData("taskResults.runId"))
    ));

    let mut duplicate = tasks.clone();
    duplicate[1].task_id = duplicate[0].task_id.clone();
    assert!(matches!(
        build_public_report(&run, &duplicate),
        Err(ReportError::InvalidData("taskResults.taskId"))
    ));

    let mut mismatched_suite = run.clone();
    mismatched_suite.environment.suite_version = "2.0.0".into();
    assert!(matches!(
        build_public_report(&mismatched_suite, &tasks),
        Err(ReportError::InvalidData("suiteVersion"))
    ));

    let mut wrong_summary = run.clone();
    wrong_summary.score.as_mut().unwrap().ability_score = 99.0;
    assert!(matches!(
        build_public_report(&wrong_summary, &tasks),
        Err(ReportError::InvalidData("score"))
    ));
}

#[test]
fn report_builder_rejects_invalid_numbers_malformed_semantics_and_duration_overflow() {
    let (run, tasks) = sample_evidence("CLI current");

    let mut nan_task = tasks.clone();
    nan_task[0].score = Some(f64::NAN);
    assert!(matches!(
        build_public_report(&run, &nan_task),
        Err(ReportError::InvalidData("taskResults.score"))
    ));

    let mut malformed_failure = tasks.clone();
    malformed_failure[1].score = None;
    assert!(matches!(
        build_public_report(&run, &malformed_failure),
        Err(ReportError::InvalidData("taskResults.evidence"))
    ));

    let mut overflowing = tasks.clone();
    overflowing[0].duration_ms = u64::MAX;
    overflowing[1].duration_ms = 1;
    assert!(matches!(
        build_public_report(&run, &overflowing),
        Err(ReportError::DurationOverflow)
    ));
}

#[test]
fn a_complete_all_infrastructure_result_is_exportable_without_inventing_a_score() {
    let (mut run, mut tasks) = sample_evidence("CLI current");
    for (index, task) in tasks.iter_mut().enumerate() {
        task.outcome = TaskOutcome::Invalid;
        task.score = None;
        task.failure_kind = Some(if index == 0 {
            FailureKind::Network
        } else {
            FailureKind::VerifierError
        });
    }
    run.score = None;

    let report = build_public_report(&run, &tasks).unwrap();

    assert_eq!(report.result.ability_score, None);
    assert_eq!(report.result.valid_tasks, 0);
    assert_eq!(report.result.failure_counts[&FailureKind::Network], 1);
    assert_eq!(report.result.failure_counts[&FailureKind::VerifierError], 1);
}
