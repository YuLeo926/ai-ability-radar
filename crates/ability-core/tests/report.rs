use ability_core::{
    Category, EnvironmentFingerprint, FailureKind, ReportError, RunMode, RunRecord, RunStatus,
    ScoreSummary, TargetKind, TargetSelection, TaskOutcome, TaskResult, build_public_report,
    render_public_report_html,
};
use std::collections::BTreeMap;

fn sample_evidence(model: &str) -> (RunRecord, Vec<TaskResult>) {
    let mut run = RunRecord::new(
        TargetSelection {
            kind: TargetKind::CodexCli,
            reported_model: model.into(),
            reasoning_effort: Some("high".into()),
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
fn report_id_is_fresh_and_not_derived_from_the_local_run_id() {
    let (run, tasks) = sample_evidence("CLI current");

    let first = build_public_report(&run, &tasks).unwrap();
    let second = build_public_report(&run, &tasks).unwrap();

    assert_ne!(first.report_id, run.id);
    assert_ne!(second.report_id, run.id);
    assert_ne!(first.report_id, second.report_id);
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
