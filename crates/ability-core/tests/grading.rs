use ability_core::{
    Category, FailureKind, GraderSpec, TaskOutcome, TaskResult, grade_submission, summarize_scores,
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn exact_json_rejects_extra_markdown_but_ignores_key_order() {
    let grader = GraderSpec::ExactJson {
        expected: json!({"count": 2, "names": ["A", "B"]}),
    };

    assert!(grade_submission(&grader, r#"{"names":["A","B"],"count":2}"#).passed);
    assert!(
        !grade_submission(
            &grader,
            "answer:\n```json\n{\"count\":2,\"names\":[\"A\",\"B\"]}\n```"
        )
        .passed
    );
}

#[test]
fn exact_text_trims_submission_but_requires_the_expected_text() {
    let grader = GraderSpec::ExactText {
        expected: "done".into(),
    };

    assert!(grade_submission(&grader, "  done\n").passed);
    assert!(!grade_submission(&grader, "Done").passed);
}

#[test]
fn json_string_set_ignores_array_order() {
    let grader = GraderSpec::JsonStringSet {
        expected: vec!["A".into(), "B".into()],
    };

    assert!(grade_submission(&grader, r#"["B", "A"]"#).passed);
    assert!(!grade_submission(&grader, r#"["A"]"#).passed);
}

#[test]
fn category_scores_are_equal_weighted() {
    let run_id = Uuid::new_v4();
    let result = |task_id: &str, category: Category, score: f64| TaskResult {
        run_id,
        task_id: task_id.into(),
        category,
        outcome: if score == 100.0 {
            TaskOutcome::Passed
        } else {
            TaskOutcome::Failed
        },
        score: Some(score),
        failure_kind: if score == 100.0 {
            None
        } else {
            Some(FailureKind::WrongAnswer)
        },
        duration_ms: 1,
        answer_rel_path: None,
        detail: String::new(),
    };
    let results = vec![
        result("i1", Category::InstructionFollowing, 100.0),
        result("i2", Category::InstructionFollowing, 100.0),
        result("i3", Category::InstructionFollowing, 100.0),
        result("l1", Category::Logic, 0.0),
    ];

    let summary = summarize_scores(&results, 4).unwrap();
    assert_eq!(summary.ability_score, 50.0);
    assert_eq!(
        summary.category_scores[&Category::InstructionFollowing],
        100.0
    );
    assert_eq!(summary.category_scores[&Category::Logic], 0.0);
}

#[test]
fn invalid_tasks_do_not_enter_the_denominator() {
    let run_id = Uuid::new_v4();
    let results = vec![TaskResult {
        run_id,
        task_id: "network".into(),
        category: Category::Logic,
        outcome: TaskOutcome::Invalid,
        score: None,
        failure_kind: Some(FailureKind::Network),
        duration_ms: 1,
        answer_rel_path: None,
        detail: "network unavailable".into(),
    }];

    assert!(summarize_scores(&results, 1).is_none());
}

#[test]
fn invalid_and_cancelled_scores_do_not_create_ability_failures() {
    let run_id = Uuid::new_v4();
    let results = vec![
        task_result(
            run_id,
            "passed",
            Category::Logic,
            TaskOutcome::Passed,
            Some(100.0),
        ),
        task_result(
            run_id,
            "network",
            Category::Logic,
            TaskOutcome::Invalid,
            Some(0.0),
        ),
        task_result(
            run_id,
            "cancelled",
            Category::Logic,
            TaskOutcome::Cancelled,
            Some(0.0),
        ),
    ];

    let summary = summarize_scores(&results, 3).unwrap();
    assert_eq!(summary.valid_tasks, 1);
    assert_eq!(summary.passed_tasks, 1);
    assert_eq!(summary.ability_score, 100.0);
}

#[test]
fn infrastructure_failures_are_excluded_but_agent_budget_failures_are_scored() {
    let run_id = Uuid::new_v4();
    let mut infrastructure = task_result(
        run_id,
        "network",
        Category::Logic,
        TaskOutcome::Failed,
        Some(0.0),
    );
    infrastructure.failure_kind = Some(FailureKind::Network);
    let mut budget = task_result(
        run_id,
        "budget",
        Category::Logic,
        TaskOutcome::Failed,
        Some(0.0),
    );
    budget.failure_kind = Some(FailureKind::AgentBudgetExceeded);
    let results = vec![
        task_result(
            run_id,
            "passed",
            Category::Logic,
            TaskOutcome::Passed,
            Some(100.0),
        ),
        infrastructure,
        budget,
    ];

    let summary = summarize_scores(&results, 3).unwrap();
    assert_eq!(summary.valid_tasks, 2);
    assert_eq!(summary.ability_score, 50.0);
}

#[test]
fn malformed_scores_do_not_distort_the_summary() {
    let run_id = Uuid::new_v4();
    let results = vec![
        task_result(
            run_id,
            "passed",
            Category::Logic,
            TaskOutcome::Passed,
            Some(100.0),
        ),
        task_result(
            run_id,
            "failed",
            Category::Logic,
            TaskOutcome::Failed,
            Some(0.0),
        ),
        task_result(
            run_id,
            "nan",
            Category::Logic,
            TaskOutcome::Failed,
            Some(f64::NAN),
        ),
        task_result(
            run_id,
            "out_of_range",
            Category::Logic,
            TaskOutcome::Failed,
            Some(101.0),
        ),
        task_result(
            run_id,
            "status_mismatch",
            Category::Logic,
            TaskOutcome::Passed,
            Some(0.0),
        ),
    ];

    let summary = summarize_scores(&results, 5).unwrap();
    assert_eq!(summary.valid_tasks, 2);
    assert_eq!(summary.passed_tasks, 1);
    assert_eq!(summary.ability_score, 50.0);
}

#[test]
fn external_verifier_is_not_executed_by_deterministic_grading() {
    let grade = grade_submission(
        &GraderSpec::ExternalVerifier {
            verifier_id: "repo-tests".into(),
        },
        "ignored",
    );

    assert!(!grade.passed);
    assert_eq!(grade.score, 0.0);
    assert_eq!(grade.detail, "requires_external_verifier:repo-tests");
}

fn task_result(
    run_id: Uuid,
    task_id: &str,
    category: Category,
    outcome: TaskOutcome,
    score: Option<f64>,
) -> TaskResult {
    TaskResult {
        run_id,
        task_id: task_id.into(),
        category,
        outcome,
        score,
        failure_kind: None,
        duration_ms: 1,
        answer_rel_path: None,
        detail: String::new(),
    }
}
