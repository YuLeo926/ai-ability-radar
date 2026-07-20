use crate::command_locator::{LaunchCommand, LaunchDiscovery, discover_provider_commands};
use crate::{AvailabilityStatus, ProcessEnvironment, ProcessError, ProcessRunner, ProcessSpec};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub(crate) struct WorkingLaunch {
    pub launch: LaunchCommand,
    pub version: String,
}

const SINGLE_PROBE_BUDGET: Duration = Duration::from_secs(8);
const TOTAL_PROBE_BUDGET: Duration = Duration::from_secs(25);

pub(crate) async fn probe_provider_launches(
    provider: &str,
    runner: Arc<dyn ProcessRunner>,
) -> Result<WorkingLaunch, AvailabilityStatus> {
    let discovery = discover_provider_commands(provider, std::env::var_os("PATH").as_deref())
        .map_err(|_| AvailabilityStatus::NotFound)?;
    probe_launch_candidates(discovery, runner).await
}

pub(crate) async fn probe_launch_candidates(
    discovery: LaunchDiscovery,
    runner: Arc<dyn ProcessRunner>,
) -> Result<WorkingLaunch, AvailabilityStatus> {
    probe_launch_candidates_with_budget(discovery, runner, TOTAL_PROBE_BUDGET).await
}

async fn probe_launch_candidates_with_budget(
    discovery: LaunchDiscovery,
    runner: Arc<dyn ProcessRunner>,
    total_budget: Duration,
) -> Result<WorkingLaunch, AvailabilityStatus> {
    match tokio::time::timeout(
        total_budget,
        probe_launch_candidates_inner(discovery, runner),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(AvailabilityStatus::VersionProbeFailed),
    }
}

async fn probe_launch_candidates_inner(
    discovery: LaunchDiscovery,
    runner: Arc<dyn ProcessRunner>,
) -> Result<WorkingLaunch, AvailabilityStatus> {
    if discovery.candidates.is_empty() {
        return Err(if discovery.reviewed_npm_without_node {
            AvailabilityStatus::RuntimeMissing
        } else {
            AvailabilityStatus::NotFound
        });
    }

    let reviewed_npm_without_node = discovery.reviewed_npm_without_node;
    let mut inaccessible = false;
    let mut probe_failed = false;
    for launch in discovery.candidates {
        let mut args = launch.prefix_args.clone();
        args.push("--version".into());
        let output = runner
            .run(
                ProcessSpec {
                    program: launch.program.clone(),
                    args,
                    current_dir: std::env::temp_dir(),
                    env: BTreeMap::new(),
                    environment: ProcessEnvironment::Inherit,
                    timeout: SINGLE_PROBE_BUDGET,
                },
                CancellationToken::new(),
            )
            .await;
        match output {
            Ok(output)
                if output.exit_code == Some(0) && valid_version_text(output.stdout.trim()) =>
            {
                return Ok(WorkingLaunch {
                    launch,
                    version: output.stdout.trim().to_owned(),
                });
            }
            Err(ProcessError::Spawn(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
                ) =>
            {
                inaccessible = true;
            }
            Err(ProcessError::Supervision(_)) => inaccessible = true,
            _ => probe_failed = true,
        }
    }

    Err(if probe_failed {
        AvailabilityStatus::VersionProbeFailed
    } else if reviewed_npm_without_node {
        AvailabilityStatus::RuntimeMissing
    } else if inaccessible {
        AvailabilityStatus::EntryInaccessible
    } else {
        AvailabilityStatus::NotFound
    })
}

fn valid_version_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .chars()
            .all(|character| character.is_ascii() && !character.is_ascii_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_locator::{LaunchCommand, LaunchDiscovery};
    use crate::{
        AvailabilityStatus, LaunchSource, ProcessError, ProcessOutput, ProcessRunner, ProcessSpec,
    };
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::io;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    struct QueueRunner {
        results: Mutex<VecDeque<Result<ProcessOutput, ProcessError>>>,
        seen: Mutex<Vec<ProcessSpec>>,
    }

    impl QueueRunner {
        fn new(
            results: impl IntoIterator<Item = Result<ProcessOutput, ProcessError>>,
        ) -> Arc<Self> {
            Arc::new(Self {
                results: Mutex::new(results.into_iter().collect()),
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl ProcessRunner for QueueRunner {
        async fn run(
            &self,
            spec: ProcessSpec,
            _cancellation: CancellationToken,
        ) -> Result<ProcessOutput, ProcessError> {
            self.seen.lock().unwrap().push(spec);
            self.results.lock().unwrap().pop_front().unwrap()
        }
    }

    struct NeverCompletes;

    #[async_trait]
    impl ProcessRunner for NeverCompletes {
        async fn run(
            &self,
            _spec: ProcessSpec,
            _cancellation: CancellationToken,
        ) -> Result<ProcessOutput, ProcessError> {
            std::future::pending().await
        }
    }

    fn launch(program: &str, source: LaunchSource) -> LaunchCommand {
        LaunchCommand {
            program: PathBuf::from(program),
            prefix_args: Vec::new(),
            source,
        }
    }

    fn discovery(
        candidates: Vec<LaunchCommand>,
        reviewed_npm_without_node: bool,
    ) -> LaunchDiscovery {
        LaunchDiscovery {
            candidates,
            reviewed_npm_without_node,
        }
    }

    fn output(exit_code: Option<i32>, stdout: impl Into<String>) -> ProcessOutput {
        ProcessOutput {
            exit_code,
            stdout: stdout.into(),
            stderr: String::new(),
            duration_ms: 1,
        }
    }

    #[tokio::test]
    async fn missing_reviewed_npm_runtime_is_reported() {
        let result = probe_launch_candidates(
            discovery(Vec::new(), true),
            QueueRunner::new(Vec::<Result<ProcessOutput, ProcessError>>::new()),
        )
        .await;

        assert!(matches!(result, Err(AvailabilityStatus::RuntimeMissing)));
    }

    #[tokio::test]
    async fn inaccessible_entries_are_reported() {
        let result = probe_launch_candidates(
            discovery(vec![launch("blocked.exe", LaunchSource::NativeExe)], false),
            QueueRunner::new([Err(ProcessError::Spawn(io::Error::from(
                io::ErrorKind::PermissionDenied,
            )))]),
        )
        .await;

        assert!(matches!(result, Err(AvailabilityStatus::EntryInaccessible)));
    }

    #[tokio::test]
    async fn spawn_not_found_and_supervision_are_entry_inaccessible() {
        let errors = [
            ProcessError::Spawn(io::Error::from(io::ErrorKind::NotFound)),
            ProcessError::Supervision(io::Error::from(io::ErrorKind::Other)),
        ];

        for error in errors {
            let result = probe_launch_candidates(
                discovery(vec![launch("blocked.exe", LaunchSource::NativeExe)], false),
                QueueRunner::new([Err(error)]),
            )
            .await;

            assert!(matches!(result, Err(AvailabilityStatus::EntryInaccessible)));
        }
    }

    #[tokio::test]
    async fn ordinary_process_errors_are_version_probe_failed_before_runtime_missing() {
        let errors = [
            ProcessError::TimedOut,
            ProcessError::Cancelled,
            ProcessError::Wait(io::Error::from(io::ErrorKind::Other)),
            ProcessError::CaptureFailed,
        ];

        for error in errors {
            let result = probe_launch_candidates(
                discovery(vec![launch("provider.exe", LaunchSource::NativeExe)], true),
                QueueRunner::new([Err(error)]),
            )
            .await;

            assert!(matches!(
                result,
                Err(AvailabilityStatus::VersionProbeFailed)
            ));
        }
    }

    #[tokio::test]
    async fn version_probe_failure_has_highest_aggregation_precedence() {
        let result = probe_launch_candidates(
            discovery(
                vec![
                    launch("blocked.exe", LaunchSource::NativeExe),
                    launch("bad-version.exe", LaunchSource::NativeExe),
                ],
                true,
            ),
            QueueRunner::new([
                Err(ProcessError::Spawn(io::Error::from(
                    io::ErrorKind::PermissionDenied,
                ))),
                Ok(output(Some(1), "provider 1.0")),
            ]),
        )
        .await;

        assert!(matches!(
            result,
            Err(AvailabilityStatus::VersionProbeFailed)
        ));
    }

    #[tokio::test]
    async fn no_candidates_and_no_missing_runtime_is_not_found() {
        let result = probe_launch_candidates(
            discovery(Vec::new(), false),
            QueueRunner::new(Vec::<Result<ProcessOutput, ProcessError>>::new()),
        )
        .await;

        assert!(matches!(result, Err(AvailabilityStatus::NotFound)));
    }

    #[tokio::test]
    async fn version_output_must_be_bounded_printable_ascii() {
        for invalid in [
            String::new(),
            "x".repeat(161),
            "provider\n1.0".into(),
            "provider\u{7f}1.0".into(),
            "provider \u{e9}".into(),
        ] {
            let result = probe_launch_candidates(
                discovery(vec![launch("provider.exe", LaunchSource::NativeExe)], false),
                QueueRunner::new([Ok(output(Some(0), invalid))]),
            )
            .await;

            assert!(matches!(
                result,
                Err(AvailabilityStatus::VersionProbeFailed)
            ));
        }
    }

    #[tokio::test]
    async fn first_working_launch_is_returned_with_an_eight_second_probe_timeout() {
        let runner = QueueRunner::new([
            Err(ProcessError::Spawn(io::Error::from(
                io::ErrorKind::NotFound,
            ))),
            Ok(output(Some(0), "provider 1.2.3")),
        ]);
        let result = probe_launch_candidates(
            discovery(
                vec![
                    launch("missing.exe", LaunchSource::NativeExe),
                    launch("node.exe", LaunchSource::ReviewedNpm),
                ],
                false,
            ),
            runner.clone(),
        )
        .await
        .unwrap();

        assert_eq!(result.launch.program, PathBuf::from("node.exe"));
        assert_eq!(result.version, "provider 1.2.3");
        assert!(
            runner
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|spec| spec.timeout == Duration::from_secs(8))
        );
    }

    #[tokio::test]
    async fn total_probe_timeout_is_version_probe_failed() {
        let result = probe_launch_candidates_with_budget(
            discovery(vec![launch("never.exe", LaunchSource::NativeExe)], false),
            Arc::new(NeverCompletes),
            Duration::from_millis(10),
        )
        .await;

        assert!(matches!(
            result,
            Err(AvailabilityStatus::VersionProbeFailed)
        ));
    }
}
