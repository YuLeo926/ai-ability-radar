use crate::{Category, FailureKind, GraderSpec, ScoreSummary, TaskOutcome, TaskResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskGrade {
    pub score: f64,
    pub passed: bool,
    pub detail: String,
}

pub fn grade_submission(grader: &GraderSpec, submission: &str) -> TaskGrade {
    match grader {
        GraderSpec::ExactText { expected } => {
            binary_grade(submission.trim() == expected, "exact_text")
        }
        GraderSpec::ExactJson { expected } => match serde_json::from_str::<Value>(submission) {
            Ok(actual) => binary_grade(actual == *expected, "exact_json"),
            Err(error) => TaskGrade {
                score: 0.0,
                passed: false,
                detail: format!("invalid_json:{error}"),
            },
        },
        GraderSpec::JsonStringSet { expected } => {
            match serde_json::from_str::<Vec<String>>(submission) {
                Ok(actual) => {
                    let actual: BTreeSet<String> = actual.into_iter().collect();
                    let expected: BTreeSet<String> = expected.iter().cloned().collect();
                    binary_grade(actual == expected, "json_string_set")
                }
                Err(error) => TaskGrade {
                    score: 0.0,
                    passed: false,
                    detail: format!("invalid_string_array:{error}"),
                },
            }
        }
        GraderSpec::ExternalVerifier { verifier_id } => TaskGrade {
            score: 0.0,
            passed: false,
            detail: format!("requires_external_verifier:{verifier_id}"),
        },
    }
}

pub fn summarize_scores(results: &[TaskResult], total_tasks: u32) -> Option<ScoreSummary> {
    let mut grouped: BTreeMap<Category, Vec<f64>> = BTreeMap::new();
    let mut passed_tasks = 0_u32;
    let mut valid_tasks = 0_u32;

    for result in results {
        let Some(score) = scoreable_result_score(result) else {
            continue;
        };

        valid_tasks += 1;
        if result.outcome == TaskOutcome::Passed {
            passed_tasks += 1;
        }
        grouped.entry(result.category).or_default().push(score);
    }

    if grouped.is_empty() {
        return None;
    }

    let category_scores: BTreeMap<Category, f64> = grouped
        .into_iter()
        .map(|(category, scores)| {
            let average = scores.iter().sum::<f64>() / scores.len() as f64;
            (category, round_one(average))
        })
        .collect();
    let ability_score =
        round_one(category_scores.values().sum::<f64>() / category_scores.len() as f64);

    Some(ScoreSummary {
        ability_score,
        passed_tasks,
        valid_tasks,
        total_tasks,
        category_scores,
    })
}

fn scoreable_result_score(result: &TaskResult) -> Option<f64> {
    let score = result.score?;
    if !score.is_finite() || !(0.0..=100.0).contains(&score) {
        return None;
    }

    match result.outcome {
        TaskOutcome::Passed if result.failure_kind.is_none() && score == 100.0 => Some(score),
        TaskOutcome::Failed if score < 100.0 && !is_infrastructure_failure(result.failure_kind) => {
            Some(score)
        }
        TaskOutcome::Invalid
        | TaskOutcome::Cancelled
        | TaskOutcome::Passed
        | TaskOutcome::Failed => None,
    }
}

fn is_infrastructure_failure(failure_kind: Option<FailureKind>) -> bool {
    matches!(
        failure_kind,
        Some(
            FailureKind::CliMissing
                | FailureKind::RuntimeMissing
                | FailureKind::AuthExpired
                | FailureKind::QuotaExhausted
                | FailureKind::Network
                | FailureKind::UserCancelled
                | FailureKind::AppInterrupted
                | FailureKind::InfrastructureTimeout
                | FailureKind::VerifierError
        )
    )
}

fn binary_grade(passed: bool, label: &str) -> TaskGrade {
    TaskGrade {
        score: if passed { 100.0 } else { 0.0 },
        passed,
        detail: if passed {
            format!("{label}:pass")
        } else {
            format!("{label}:mismatch")
        },
    }
}

fn round_one(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
