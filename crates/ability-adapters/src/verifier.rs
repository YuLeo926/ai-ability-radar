use crate::{OutputStream, ProcessEnvironment, ProcessError, ProcessRunner, ProcessSpec};
use ability_core::{FailureKind, TaskOutcome};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq)]
pub struct VerificationGrade {
    pub outcome: TaskOutcome,
    pub score: Option<f64>,
    pub failure_kind: Option<FailureKind>,
    pub detail: String,
    pub duration_ms: u64,
}

#[async_trait]
pub trait WorkspaceVerifier: Send + Sync {
    async fn verify(
        &self,
        verifier_id: &str,
        workspace: &Path,
        cancellation: CancellationToken,
    ) -> VerificationGrade;
}

pub struct NodeVerifier {
    runner: Arc<dyn ProcessRunner>,
    pack_root: PathBuf,
}

impl NodeVerifier {
    pub fn new(runner: Arc<dyn ProcessRunner>, pack_root: PathBuf) -> Self {
        Self { runner, pack_root }
    }

    pub async fn verify(
        &self,
        verifier_id: &str,
        workspace: &Path,
        cancellation: CancellationToken,
    ) -> VerificationGrade {
        let script = match verifier_script(&self.pack_root, verifier_id) {
            Some(path) => path,
            None => {
                return invalid(format!("unknown_verifier:{verifier_id}"), 0);
            }
        };
        let (script, workspace) = match prepare_paths(&script, workspace) {
            Ok(paths) => paths,
            Err(detail) => return invalid(detail, 0),
        };
        let mut env = BTreeMap::new();
        #[cfg(windows)]
        if let Ok(system_root) = std::env::var("SystemRoot") {
            env.insert("SystemRoot".into(), system_root);
        }
        let spec = ProcessSpec {
            program: "node".into(),
            args: vec![
                "--no-warnings".into(),
                script.to_string_lossy().into_owned(),
                workspace.to_string_lossy().into_owned(),
            ],
            current_dir: workspace,
            env,
            environment: ProcessEnvironment::Clear,
            stdin: None,
            timeout: Duration::from_secs(120),
        };
        match self.runner.run(spec, cancellation).await {
            Ok(output)
                if output.exit_code == Some(0)
                    && terminal_marker(&output.stdout, "TASK_PASSED")
                    && output.stderr.is_empty() =>
            {
                VerificationGrade {
                    outcome: TaskOutcome::Passed,
                    score: Some(100.0),
                    failure_kind: None,
                    detail: "hidden_tests:pass".into(),
                    duration_ms: output.duration_ms,
                }
            }
            Ok(output)
                if output.exit_code == Some(1)
                    && output.stdout.is_empty()
                    && terminal_marker(&output.stderr, "TASK_FAILED") =>
            {
                VerificationGrade {
                    outcome: TaskOutcome::Failed,
                    score: Some(0.0),
                    failure_kind: Some(FailureKind::WrongAnswer),
                    detail: "hidden_tests:fail".into(),
                    duration_ms: output.duration_ms,
                }
            }
            Ok(output) => invalid(
                format!(
                    "invalid_verifier_protocol:exit={:?}:stdout={:?}:stderr={:?}",
                    output.exit_code, output.stdout, output.stderr
                ),
                output.duration_ms,
            ),
            Err(ProcessError::Spawn(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                VerificationGrade {
                    outcome: TaskOutcome::Invalid,
                    score: None,
                    failure_kind: Some(FailureKind::RuntimeMissing),
                    detail: "node_runtime_missing".into(),
                    duration_ms: 0,
                }
            }
            Err(ProcessError::Cancelled) => VerificationGrade {
                outcome: TaskOutcome::Cancelled,
                score: None,
                failure_kind: Some(FailureKind::UserCancelled),
                detail: "verifier_cancelled".into(),
                duration_ms: 0,
            },
            Err(ProcessError::TimedOut) => invalid("verifier_timeout".into(), 120_000),
            Err(ProcessError::Spawn(error)) => invalid(error.to_string(), 0),
            Err(ProcessError::Supervision(error)) => invalid(error.to_string(), 0),
            Err(ProcessError::Wait(error)) => invalid(error.to_string(), 0),
            Err(ProcessError::CaptureFailed) => invalid("verifier_output_capture_failed".into(), 0),
            Err(ProcessError::StdinFailed) => invalid("verifier_input_write_failed".into(), 0),
            Err(ProcessError::StdinLimit) => invalid("verifier_input_limit".into(), 0),
            Err(ProcessError::OutputLimit {
                stream: OutputStream::Stdout,
            }) => invalid("verifier_stdout_limit".into(), 0),
            Err(ProcessError::OutputLimit {
                stream: OutputStream::Stderr,
            }) => invalid("verifier_stderr_limit".into(), 0),
            Err(ProcessError::TerminationFailed) => {
                invalid("verifier_termination_failed".into(), 0)
            }
            Err(ProcessError::DurationOverflow) => invalid("verifier_duration_overflow".into(), 0),
        }
    }
}

#[async_trait]
impl WorkspaceVerifier for NodeVerifier {
    async fn verify(
        &self,
        verifier_id: &str,
        workspace: &Path,
        cancellation: CancellationToken,
    ) -> VerificationGrade {
        NodeVerifier::verify(self, verifier_id, workspace, cancellation).await
    }
}

fn verifier_script(pack_root: &Path, verifier_id: &str) -> Option<PathBuf> {
    let root = pack_root.join("tasks");
    match verifier_id {
        "dedupe-events-v1" => Some(root.join("dedupe-events/verify.mjs")),
        "retry-schedule-v1" => Some(root.join("retry-schedule/verify.mjs")),
        _ => None,
    }
}

fn prepare_paths(script: &Path, workspace: &Path) -> Result<(PathBuf, PathBuf), String> {
    reject_link_or_reparse_point(workspace)?;
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("invalid_workspace:{error}"))?;
    if !workspace.is_dir() {
        return Err("invalid_workspace:not_a_directory".into());
    }
    reject_workspace_links(&workspace)?;

    reject_link_or_reparse_point(script)?;
    let script = script
        .canonicalize()
        .map_err(|error| format!("invalid_verifier_script:{error}"))?;
    if !script.is_file() {
        return Err("invalid_verifier_script:not_a_file".into());
    }
    Ok((
        node_compatible_absolute_path(script),
        node_compatible_absolute_path(workspace),
    ))
}

#[cfg(windows)]
fn node_compatible_absolute_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(path) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{path}"));
    }
    if let Some(path) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(path);
    }
    path
}

#[cfg(not(windows))]
fn node_compatible_absolute_path(path: PathBuf) -> PathBuf {
    path
}

fn reject_workspace_links(root: &Path) -> Result<(), String> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in
            fs::read_dir(&directory).map_err(|error| format!("invalid_workspace:{error}"))?
        {
            let path = entry
                .map_err(|error| format!("invalid_workspace:{error}"))?
                .path();
            reject_link_or_reparse_point(&path)?;
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(())
}

fn reject_link_or_reparse_point(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| format!("invalid_path:{error}"))?;
    if is_link_or_reparse_point(&metadata) {
        return Err(format!("unsafe_workspace_path:{}", path.display()));
    }
    Ok(())
}

#[cfg(windows)]
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn terminal_marker(output: &str, marker: &str) -> bool {
    output == marker || output == format!("{marker}\n") || output == format!("{marker}\r\n")
}

fn invalid(detail: String, duration_ms: u64) -> VerificationGrade {
    VerificationGrade {
        outcome: TaskOutcome::Invalid,
        score: None,
        failure_kind: Some(FailureKind::VerifierError),
        detail,
        duration_ms,
    }
}
