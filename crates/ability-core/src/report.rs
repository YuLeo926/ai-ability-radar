use crate::{
    Category, FailureKind, RunRecord, RunStatus, ScoreSummary, TargetKind, TaskOutcome, TaskResult,
    grading::has_coherent_task_evidence, is_valid_reported_model, summarize_scores,
};
use chrono::{DateTime, SecondsFormat, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use thiserror::Error;
use uuid::Uuid;

pub const PUBLIC_REPORT_SCHEMA_VERSION: u32 = 1;
pub const PUBLIC_INTERPRETATION_STATUS: &str = "not_evaluated";
pub const PUBLIC_METHODOLOGY_STATEMENT: &str =
    "v0.2 不生成降智结论；仅展示本题包的客观结果，不是 IQ，也不代表模型退化。";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicReport {
    pub schema_version: u32,
    pub report_id: Uuid,
    pub generated_at: DateTime<Utc>,
    pub target: PublicTarget,
    pub environment: PublicEnvironment,
    pub result: PublicResult,
    pub methodology: PublicMethodology,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicTarget {
    pub kind: TargetKind,
    pub reported_model: String,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicEnvironment {
    pub os_family: String,
    pub app_version: String,
    pub cli_version: Option<String>,
    pub verifier_runtime_version: Option<String>,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_content_sha256: String,
    pub scoring_rule_version: String,
    pub resumed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicResult {
    pub run_status: RunStatus,
    pub ability_score: Option<f64>,
    pub passed_tasks: u32,
    pub valid_tasks: u32,
    pub total_tasks: u32,
    pub category_scores: BTreeMap<Category, f64>,
    pub outcome_counts: BTreeMap<String, u32>,
    pub failure_counts: BTreeMap<FailureKind, u32>,
    pub total_duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicMethodology {
    pub interpretation_status: String,
    pub statement: String,
}

#[derive(Debug, Error)]
pub enum ReportError {
    #[error("report contains sensitive-looking text in {0}")]
    SensitiveText(&'static str),
    #[error("report data is inconsistent in {0}")]
    InvalidData(&'static str),
    #[error("only a coherent completed result can be exported")]
    NotCompleted,
    #[error("report duration is outside the supported range")]
    DurationOverflow,
    #[error("report serialization failed")]
    Json(#[from] serde_json::Error),
}

pub fn build_public_report(
    run: &RunRecord,
    tasks: &[TaskResult],
) -> Result<PublicReport, ReportError> {
    scan_source_strings(run)?;
    let total_duration_ms = validate_completed_evidence(run, tasks)?;
    let score = run.score.as_ref();
    let mut outcome_counts = BTreeMap::new();
    let mut failure_counts = BTreeMap::new();
    for task in tasks {
        *outcome_counts
            .entry(outcome_name(task.outcome).to_owned())
            .or_insert(0) += 1;
        if let Some(failure) = task.failure_kind {
            *failure_counts.entry(failure).or_insert(0) += 1;
        }
    }

    let report = PublicReport {
        schema_version: PUBLIC_REPORT_SCHEMA_VERSION,
        report_id: Uuid::new_v4(),
        generated_at: Utc::now(),
        target: PublicTarget {
            kind: run.target.kind,
            reported_model: required_reported_model(&run.target.reported_model)?,
            reasoning_effort: optional_text(
                run.target.reasoning_effort.as_deref(),
                "reasoningEffort",
                64,
            )?,
        },
        environment: PublicEnvironment {
            os_family: required_text(&run.environment.os_family, "osFamily", 120)?,
            app_version: required_text(&run.environment.app_version, "appVersion", 120)?,
            cli_version: optional_text(run.environment.cli_version.as_deref(), "cliVersion", 160)?,
            verifier_runtime_version: optional_text(
                run.environment.verifier_runtime_version.as_deref(),
                "verifierRuntimeVersion",
                160,
            )?,
            suite_id: required_text(&run.suite_id, "suiteId", 160)?,
            suite_version: required_text(&run.suite_version, "suiteVersion", 120)?,
            suite_content_sha256: required_text(
                &run.environment.suite_content_sha256,
                "suiteContentSha256",
                64,
            )?,
            scoring_rule_version: required_text(
                &run.environment.scoring_rule_version,
                "scoringRuleVersion",
                120,
            )?,
            resumed: run.environment.resumed,
        },
        result: PublicResult {
            run_status: run.status,
            ability_score: score.map(|value| value.ability_score),
            passed_tasks: score.map_or(0, |value| value.passed_tasks),
            valid_tasks: score.map_or(0, |value| value.valid_tasks),
            total_tasks: run.total_tasks,
            category_scores: score
                .map(|value| value.category_scores.clone())
                .unwrap_or_default(),
            outcome_counts,
            failure_counts,
            total_duration_ms,
        },
        methodology: PublicMethodology {
            interpretation_status: PUBLIC_INTERPRETATION_STATUS.into(),
            statement: PUBLIC_METHODOLOGY_STATEMENT.into(),
        },
    };
    validate_public_report(&report)?;
    Ok(report)
}

pub fn validate_public_report(report: &PublicReport) -> Result<(), ReportError> {
    if report.schema_version != PUBLIC_REPORT_SCHEMA_VERSION {
        return Err(ReportError::InvalidData("schemaVersion"));
    }
    if report.report_id.is_nil() {
        return Err(ReportError::InvalidData("reportId"));
    }

    validate_reported_model(&report.target.reported_model)?;
    validate_optional_text(
        report.target.reasoning_effort.as_deref(),
        "reasoningEffort",
        64,
    )?;
    validate_text(&report.environment.os_family, "osFamily", 120, true)?;
    validate_text(&report.environment.app_version, "appVersion", 120, true)?;
    validate_optional_text(report.environment.cli_version.as_deref(), "cliVersion", 160)?;
    validate_optional_text(
        report.environment.verifier_runtime_version.as_deref(),
        "verifierRuntimeVersion",
        160,
    )?;
    validate_text(&report.environment.suite_id, "suiteId", 160, true)?;
    validate_text(&report.environment.suite_version, "suiteVersion", 120, true)?;
    validate_text(
        &report.environment.suite_content_sha256,
        "suiteContentSha256",
        64,
        true,
    )?;
    validate_text(
        &report.environment.scoring_rule_version,
        "scoringRuleVersion",
        120,
        true,
    )?;
    if !is_lower_hex_sha256(&report.environment.suite_content_sha256) {
        return Err(ReportError::InvalidData("suiteContentSha256"));
    }

    let result = &report.result;
    if result.run_status != RunStatus::Completed {
        return Err(ReportError::NotCompleted);
    }
    if result.total_tasks == 0
        || result.passed_tasks > result.valid_tasks
        || result.valid_tasks > result.total_tasks
    {
        return Err(ReportError::InvalidData("resultCounts"));
    }
    match result.ability_score {
        Some(score) => {
            if result.valid_tasks == 0
                || result.category_scores.is_empty()
                || !valid_score(score)
                || result
                    .category_scores
                    .values()
                    .any(|value| !valid_score(*value))
            {
                return Err(ReportError::InvalidData("score"));
            }
            let category_mean = round_one(
                result.category_scores.values().sum::<f64>() / result.category_scores.len() as f64,
            );
            if score != category_mean {
                return Err(ReportError::InvalidData("score"));
            }
        }
        None => {
            if result.passed_tasks != 0
                || result.valid_tasks != 0
                || !result.category_scores.is_empty()
            {
                return Err(ReportError::InvalidData("score"));
            }
        }
    }

    let allowed_outcomes = ["passed", "failed", "invalid", "cancelled"];
    if result
        .outcome_counts
        .keys()
        .any(|key| !allowed_outcomes.contains(&key.as_str()))
        || result.outcome_counts.values().copied().sum::<u32>() != result.total_tasks
        || result.outcome_counts.get("passed").copied().unwrap_or(0) != result.passed_tasks
        || result.failure_counts.values().copied().sum::<u32>() > result.total_tasks
    {
        return Err(ReportError::InvalidData("resultCounts"));
    }

    if report.methodology.interpretation_status != PUBLIC_INTERPRETATION_STATUS
        || report.methodology.statement != PUBLIC_METHODOLOGY_STATEMENT
    {
        return Err(ReportError::InvalidData("methodology"));
    }
    validate_text(&report.methodology.statement, "statement", 240, true)?;
    serde_json::to_vec(report)?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReasoningEffortDisplayPolicy {
    chat_gpt_client: CanonicalEffortLabels,
    claude_client: CanonicalEffortLabels,
    codex_cli: CanonicalEffortLabels,
    claude_code: CanonicalEffortLabels,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalEffortLabels {
    none: String,
    minimal: String,
    low: String,
    medium: String,
    high: String,
    xhigh: String,
    max: String,
    ultra: String,
}

impl CanonicalEffortLabels {
    fn get(&self, value: &str) -> Option<&str> {
        match value {
            "none" => Some(&self.none),
            "minimal" => Some(&self.minimal),
            "low" => Some(&self.low),
            "medium" => Some(&self.medium),
            "high" => Some(&self.high),
            "xhigh" => Some(&self.xhigh),
            "max" => Some(&self.max),
            "ultra" => Some(&self.ultra),
            _ => None,
        }
    }
}

fn reasoning_effort_display_policy() -> &'static ReasoningEffortDisplayPolicy {
    static POLICY: OnceLock<ReasoningEffortDisplayPolicy> = OnceLock::new();
    POLICY.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../schemas/reasoning-effort-display.json"
        ))
        .expect("embedded reasoning effort display policy must be valid")
    })
}

fn reasoning_effort_display(kind: TargetKind, value: Option<&str>) -> &str {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return "\u{672a}\u{8bb0}\u{5f55}";
    };
    let policy = reasoning_effort_display_policy();
    let labels = match kind {
        TargetKind::ChatGptClient => &policy.chat_gpt_client,
        TargetKind::ClaudeClient => &policy.claude_client,
        TargetKind::CodexCli => &policy.codex_cli,
        TargetKind::ClaudeCode => &policy.claude_code,
    };
    labels.get(value).unwrap_or(value)
}

pub fn render_public_report_html(report: &PublicReport) -> Result<String, ReportError> {
    validate_public_report(report)?;
    let embedded_json = script_safe_json(&serde_json::to_string(report)?);
    let target = html_escape(target_kind_name(report.target.kind));
    let model = html_escape(&report.target.reported_model);
    let effort = html_escape(reasoning_effort_display(
        report.target.kind,
        report.target.reasoning_effort.as_deref(),
    ));
    let score = report
        .result
        .ability_score
        .map(|value| format!("{value:.1}"))
        .unwrap_or_else(|| "无有效分".into());
    let category_rows = report
        .result
        .category_scores
        .iter()
        .map(|(category, value)| {
            format!(
                "<li><span>{}</span><strong>{value:.1}</strong></li>",
                html_escape(&category.to_string())
            )
        })
        .collect::<String>();
    let outcome_rows = report
        .result
        .outcome_counts
        .iter()
        .map(|(outcome, count)| {
            format!(
                "<li><span>{}</span><strong>{count}</strong></li>",
                html_escape(outcome)
            )
        })
        .collect::<String>();
    let failure_rows = if report.result.failure_counts.is_empty() {
        "<li><span>无</span><strong>0</strong></li>".into()
    } else {
        report
            .result
            .failure_counts
            .iter()
            .map(|(failure, count)| {
                format!(
                    "<li><span>{}</span><strong>{count}</strong></li>",
                    html_escape(failure_name(*failure))
                )
            })
            .collect::<String>()
    };
    let category_section = if category_rows.is_empty() {
        "<p>本次没有形成可计分分类。</p>".into()
    } else {
        format!("<ul class=\"fact-list\">{category_rows}</ul>")
    };
    let cli_version = html_escape(
        report
            .environment
            .cli_version
            .as_deref()
            .unwrap_or("未记录"),
    );
    let verifier_version = html_escape(
        report
            .environment
            .verifier_runtime_version
            .as_deref()
            .unwrap_or("未记录"),
    );
    let generated_at = html_escape(
        &report
            .generated_at
            .to_rfc3339_opts(SecondsFormat::Secs, true),
    );
    let statement = html_escape(&report.methodology.statement);
    let suite_id = html_escape(&report.environment.suite_id);
    let suite_version = html_escape(&report.environment.suite_version);
    let suite_hash = html_escape(&report.environment.suite_content_sha256);
    let scoring_rule = html_escape(&report.environment.scoring_rule_version);
    let os_family = html_escape(&report.environment.os_family);
    let app_version = html_escape(&report.environment.app_version);
    let resumed = if report.environment.resumed {
        "恢复运行"
    } else {
        "完整运行"
    };
    let duration_seconds = report.result.total_duration_ms as f64 / 1_000.0;

    Ok(format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'">
<title>AI 能力雷达 · 匿名公开报告</title>
<style>
:root{{color-scheme:light;--ink:#17312c;--muted:#557069;--line:#c8d9d4;--paper:#f4faf8;--accent:#147d70;--warm:#f4aa49}}
*{{box-sizing:border-box}}body{{margin:0;background:#eef5f2;color:var(--ink);font:16px/1.65 "Microsoft YaHei UI","Segoe UI",sans-serif}}
main{{width:min(100% - 2rem,64rem);margin:2rem auto;padding:clamp(1.25rem,4vw,3rem);border:1px solid var(--line);border-radius:1.25rem;background:#fff;box-shadow:0 1.5rem 4rem rgba(23,49,44,.08)}}
h1,h2,p,dl,dd{{margin-block:0}}header{{display:grid;gap:.65rem;padding-bottom:1.5rem;border-bottom:1px solid var(--line)}}h1{{font-size:clamp(2rem,6vw,3.5rem);line-height:1.08;letter-spacing:-.045em}}
.eyebrow{{color:var(--muted);font-size:.78rem;font-weight:750;letter-spacing:.1em;text-transform:uppercase}}.score{{margin:.9rem 0;color:var(--accent);font-size:clamp(3rem,10vw,5.5rem);font-weight:850;line-height:1}}
.grid{{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:1rem;margin-top:1rem}}section{{padding:1.2rem;border:1px solid var(--line);border-radius:.9rem;background:var(--paper)}}section h2{{margin-bottom:.7rem;font-size:1.05rem}}
dl{{display:grid;gap:.45rem}}dl div,.fact-list li{{display:flex;justify-content:space-between;gap:1rem;border-bottom:1px solid #dce7e4;padding:.35rem 0}}dt,.fact-list span{{color:var(--muted)}}dd,.fact-list strong{{text-align:right;overflow-wrap:anywhere}}.fact-list{{display:grid;gap:.2rem;margin:0;padding:0;list-style:none}}
.boundary{{grid-column:1/-1;border-left:.35rem solid var(--accent);background:linear-gradient(120deg,rgba(20,125,112,.1),transparent 70%),var(--paper)}}footer{{margin-top:1.25rem;color:var(--muted);font-size:.85rem;overflow-wrap:anywhere}}
@media(max-width:42rem){{.grid{{grid-template-columns:1fr}}main{{width:100%;margin:0;border:0;border-radius:0}}}}
</style>
</head>
<body>
<main>
<header>
<p class="eyebrow">AI 能力雷达 · 匿名公开报告 · schema v{schema_version}</p>
<h1>{target} · {model}</h1>
<p>推理档位：{effort}</p>
<div class="score">{score}</div>
<p>{passed}/{valid} 题通过 · {valid}/{total} 题有效 · 总耗时 {duration_seconds:.1} 秒</p>
</header>
<div class="grid">
<section aria-labelledby="repro-title"><h2 id="repro-title">复现信息</h2><dl>
<div><dt>操作系统系列</dt><dd>{os_family}</dd></div>
<div><dt>应用版本</dt><dd>{app_version}</dd></div>
<div><dt>CLI 版本</dt><dd>{cli_version}</dd></div>
<div><dt>验证器运行时</dt><dd>{verifier_version}</dd></div>
<div><dt>题包</dt><dd>{suite_id} · {suite_version}</dd></div>
<div><dt>题包内容哈希</dt><dd>{suite_hash}</dd></div>
<div><dt>评分规则</dt><dd>{scoring_rule}</dd></div>
<div><dt>运行方式</dt><dd>{resumed}</dd></div>
</dl></section>
<section aria-labelledby="category-title"><h2 id="category-title">分类分数</h2>{category_section}</section>
<section aria-labelledby="outcome-title"><h2 id="outcome-title">客观结果计数</h2><ul class="fact-list">{outcome_rows}</ul></section>
<section aria-labelledby="failure-title"><h2 id="failure-title">失败分类计数</h2><ul class="fact-list">{failure_rows}</ul></section>
<section class="boundary" aria-labelledby="boundary-title"><h2 id="boundary-title">解释边界 · {interpretation_status}</h2><p>{statement}</p></section>
</div>
<footer>报告编号 {report_id} · 生成时间 {generated_at} · 状态 completed</footer>
<script type="application/json" id="ability-radar-report">{embedded_json}</script>
</main>
</body>
</html>"#,
        schema_version = report.schema_version,
        target = target,
        model = model,
        effort = effort,
        score = score,
        passed = report.result.passed_tasks,
        valid = report.result.valid_tasks,
        total = report.result.total_tasks,
        duration_seconds = duration_seconds,
        os_family = os_family,
        app_version = app_version,
        cli_version = cli_version,
        verifier_version = verifier_version,
        suite_id = suite_id,
        suite_version = suite_version,
        suite_hash = suite_hash,
        scoring_rule = scoring_rule,
        resumed = resumed,
        category_section = category_section,
        outcome_rows = outcome_rows,
        failure_rows = failure_rows,
        interpretation_status = PUBLIC_INTERPRETATION_STATUS,
        statement = statement,
        report_id = report.report_id,
        generated_at = generated_at,
        embedded_json = embedded_json,
    ))
}

pub fn public_report_sha256(contents: &str) -> String {
    format!("{:x}", Sha256::digest(contents.as_bytes()))
}

fn scan_source_strings(run: &RunRecord) -> Result<(), ReportError> {
    let candidates = [
        ("reportedModel", run.target.reported_model.as_str()),
        (
            "reasoningEffort",
            run.target.reasoning_effort.as_deref().unwrap_or(""),
        ),
        ("osFamily", run.environment.os_family.as_str()),
        ("appVersion", run.environment.app_version.as_str()),
        (
            "cliVersion",
            run.environment.cli_version.as_deref().unwrap_or(""),
        ),
        (
            "verifierRuntimeVersion",
            run.environment
                .verifier_runtime_version
                .as_deref()
                .unwrap_or(""),
        ),
        ("suiteId", run.suite_id.as_str()),
        ("suiteVersion", run.suite_version.as_str()),
        (
            "suiteContentSha256",
            run.environment.suite_content_sha256.as_str(),
        ),
        (
            "scoringRuleVersion",
            run.environment.scoring_rule_version.as_str(),
        ),
    ];
    for (field, value) in candidates {
        scan_text(field, value)?;
    }
    Ok(())
}

fn validate_completed_evidence(run: &RunRecord, tasks: &[TaskResult]) -> Result<u64, ReportError> {
    if run.status != RunStatus::Completed {
        return Err(ReportError::NotCompleted);
    }
    if run.total_tasks == 0
        || run.completed_tasks != run.total_tasks
        || run.finished_at.is_none()
        || run
            .finished_at
            .is_some_and(|finished| finished < run.started_at)
    {
        return Err(ReportError::InvalidData("completedTasks"));
    }
    if run.suite_id != run.environment.suite_id {
        return Err(ReportError::InvalidData("suiteId"));
    }
    if run.suite_version != run.environment.suite_version {
        return Err(ReportError::InvalidData("suiteVersion"));
    }
    if tasks.len() != usize::try_from(run.total_tasks).unwrap_or(usize::MAX) {
        return Err(ReportError::InvalidData("taskResults"));
    }

    let mut task_ids = BTreeSet::new();
    let mut total_duration_ms = 0_u64;
    for task in tasks {
        if task.run_id != run.id {
            return Err(ReportError::InvalidData("taskResults.runId"));
        }
        if task.task_id.is_empty()
            || task.task_id.len() > 128
            || task.task_id.chars().any(char::is_control)
            || !task_ids.insert(task.task_id.as_str())
        {
            return Err(ReportError::InvalidData("taskResults.taskId"));
        }
        if task.score.is_some_and(|score| !valid_score(score)) {
            return Err(ReportError::InvalidData("taskResults.score"));
        }
        if !has_coherent_task_evidence(task) {
            return Err(ReportError::InvalidData("taskResults.evidence"));
        }
        total_duration_ms = total_duration_ms
            .checked_add(task.duration_ms)
            .ok_or(ReportError::DurationOverflow)?;
    }

    let recomputed = summarize_scores(tasks, run.total_tasks);
    if !score_summaries_equal(run.score.as_ref(), recomputed.as_ref()) {
        return Err(ReportError::InvalidData("score"));
    }
    Ok(total_duration_ms)
}

fn score_summaries_equal(stored: Option<&ScoreSummary>, recomputed: Option<&ScoreSummary>) -> bool {
    match (stored, recomputed) {
        (None, None) => true,
        (Some(stored), Some(recomputed)) => {
            valid_score(stored.ability_score)
                && stored
                    .category_scores
                    .values()
                    .all(|score| valid_score(*score))
                && stored == recomputed
        }
        _ => false,
    }
}

fn validate_text(
    value: &str,
    field: &'static str,
    max_chars: usize,
    required: bool,
) -> Result<(), ReportError> {
    if value != value.trim()
        || (required && value.is_empty())
        || value.chars().count() > max_chars
        || value.chars().any(char::is_control)
    {
        return Err(ReportError::InvalidData(field));
    }
    scan_text(field, value)
}

fn validate_optional_text(
    value: Option<&str>,
    field: &'static str,
    max_chars: usize,
) -> Result<(), ReportError> {
    if let Some(value) = value {
        validate_text(value, field, max_chars, true)?;
    }
    Ok(())
}

fn validate_reported_model(value: &str) -> Result<(), ReportError> {
    if !is_valid_reported_model(value) {
        return Err(ReportError::InvalidData("reportedModel"));
    }
    scan_text("reportedModel", value)
}

fn required_reported_model(value: &str) -> Result<String, ReportError> {
    let trimmed = value.trim();
    validate_reported_model(trimmed)?;
    Ok(trimmed.to_owned())
}

fn required_text(
    value: &str,
    field: &'static str,
    max_chars: usize,
) -> Result<String, ReportError> {
    let trimmed = value.trim();
    validate_text(trimmed, field, max_chars, true)?;
    Ok(trimmed.to_owned())
}

fn optional_text(
    value: Option<&str>,
    field: &'static str,
    max_chars: usize,
) -> Result<Option<String>, ReportError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    validate_text(trimmed, field, max_chars, true)?;
    Ok(Some(trimmed.to_owned()))
}

fn scan_text(field: &'static str, value: &str) -> Result<(), ReportError> {
    if sensitive_text().is_match(value) || contains_sensitive_path(value) {
        return Err(ReportError::SensitiveText(field));
    }
    Ok(())
}

fn sensitive_text() -> &'static Regex {
    static SENSITIVE: OnceLock<Regex> = OnceLock::new();
    SENSITIVE.get_or_init(|| {
        Regex::new(
            r#"(?ix)
            (?:sk-(?:ant-|proj-)?[a-z0-9_-]{12,}) |
            (?:github_pat_[a-z0-9_]{12,}) |
            (?:gh[pousr]_[a-z0-9]{12,}) |
            (?:xox[baprs]-[a-z0-9-]{16,}) |
            (?:(?:hf|npm)_[a-z0-9_-]{12,}) |
            (?:aiza[a-z0-9_-]{20,}) |
             (?:bearer\s+[a-z0-9._~+/-]{12,}) |
             (?:akia[a-z0-9]{16}) |
             (?:-----begin\s+[a-z0-9 ]*private\s+key-----) |
             (?:\b(?:password|passwd|secret|token|api[\s_-]?key)\s*[:=]\s*[^\s,;]{8,}) |
             (?:[\p{L}\p{N}._%+-]+@[\p{L}\p{N}.-]+\.[\p{L}]{2,})
             "#,
        )
        .expect("static public-report sensitive-text regex")
    })
}

fn contains_sensitive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    for index in value.char_indices().map(|(index, _)| index) {
        let byte = bytes[index];
        let boundary = path_boundary(value[..index].chars().next_back());
        let path_start =
            boundary || (value[..index].ends_with(':') && !follows_web_url_scheme(value, index));

        if path_start
            && byte.is_ascii_alphabetic()
            && bytes.get(index + 1) == Some(&b':')
            && bytes
                .get(index + 2)
                .is_some_and(|next| is_path_separator(*next))
        {
            return true;
        }

        if path_start
            && byte == b'~'
            && bytes.get(index + 1) == Some(&b'/')
            && looks_like_posix_path(&value[index + 1..])
        {
            return true;
        }

        if !path_start || !is_path_separator(byte) {
            continue;
        }
        if bytes
            .get(index + 1)
            .is_some_and(|next| is_path_separator(*next))
            && looks_like_unc_path(&value[index..])
        {
            return true;
        }
        if byte == b'/' && looks_like_posix_path(&value[index..]) {
            return true;
        }
    }
    false
}

fn follows_web_url_scheme(value: &str, path_start: usize) -> bool {
    if value.as_bytes().get(path_start + 1) != Some(&b'/') {
        return false;
    }
    let Some(prefix) = value[..path_start].strip_suffix(':') else {
        return false;
    };
    let scheme_start = prefix
        .char_indices()
        .rev()
        .find(|(_, character)| {
            !character.is_ascii_alphanumeric() && !matches!(character, '+' | '-' | '.')
        })
        .map_or(0, |(index, character)| index + character.len_utf8());
    let scheme = &prefix[scheme_start..];
    scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
}

fn path_boundary(previous: Option<char>) -> bool {
    previous.is_none_or(|character| {
        !character.is_alphanumeric() && !matches!(character, '.' | '_' | '-' | '/' | '\\' | ':')
    })
}

fn is_path_separator(byte: u8) -> bool {
    matches!(byte, b'/' | b'\\')
}

fn path_token(value: &str) -> &str {
    let end = value
        .char_indices()
        .find_map(|(index, character)| {
            (index > 0
                && (character.is_whitespace()
                    || matches!(
                        character,
                        '"' | '\'' | '<' | '>' | '|' | ')' | ']' | '}' | ',' | ';'
                    )))
            .then_some(index)
        })
        .unwrap_or(value.len());
    &value[..end]
}

fn looks_like_unc_path(value: &str) -> bool {
    let token = path_token(value);
    let bytes = token.as_bytes();
    if bytes.len() < 5 || !is_path_separator(bytes[0]) || !is_path_separator(bytes[1]) {
        return false;
    }
    let components = token[2..]
        .split(['/', '\\'])
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    components.len() >= 2
        && components[0] != "."
        && components[0] != ".."
        && components[1] != "."
        && components[1] != ".."
}

fn looks_like_posix_path(value: &str) -> bool {
    let token = path_token(value);
    let components = token
        .strip_prefix('/')
        .unwrap_or(token)
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    if components.len() < 2
        || components
            .iter()
            .any(|component| matches!(*component, "." | ".."))
    {
        return false;
    }

    let numeric_version = components.iter().all(|component| {
        component
            .trim_start_matches(['v', 'V'])
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
    });
    !numeric_version
}

fn valid_score(value: f64) -> bool {
    value.is_finite() && (0.0..=100.0).contains(&value)
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn round_one(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn outcome_name(outcome: TaskOutcome) -> &'static str {
    match outcome {
        TaskOutcome::Passed => "passed",
        TaskOutcome::Failed => "failed",
        TaskOutcome::Invalid => "invalid",
        TaskOutcome::Cancelled => "cancelled",
    }
}

fn target_kind_name(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::ChatGptClient => "ChatGPT 客户端",
        TargetKind::ClaudeClient => "Claude 客户端",
        TargetKind::CodexCli => "Codex CLI",
        TargetKind::ClaudeCode => "Claude Code",
    }
}

fn failure_name(failure: FailureKind) -> &'static str {
    match failure {
        FailureKind::CliMissing => "cli_missing",
        FailureKind::RuntimeMissing => "runtime_missing",
        FailureKind::AuthExpired => "auth_expired",
        FailureKind::QuotaExhausted => "quota_exhausted",
        FailureKind::Network => "network",
        FailureKind::UserCancelled => "user_cancelled",
        FailureKind::AppInterrupted => "app_interrupted",
        FailureKind::InfrastructureTimeout => "infrastructure_timeout",
        FailureKind::AgentBudgetExceeded => "agent_budget_exceeded",
        FailureKind::VerifierError => "verifier_error",
        FailureKind::WrongAnswer => "wrong_answer",
    }
}

fn script_safe_json(value: &str) -> String {
    value
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
