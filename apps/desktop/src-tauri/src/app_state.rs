use ability_adapters::{
    AgentAdapter, AuthState, ClaudeCodeAdapter, CliRunService, CodexAdapter, NodeVerifier,
    PrerequisiteStatus, ProcessEnvironment, ProcessRunner, ProcessSpec, TargetAvailability,
    TokioProcessRunner, WorkspaceVerifier,
};
use ability_core::{
    ArtifactStore, LoadedPack, ManualRunService, PackLoader, PackRegistry, RunRepository,
    TargetKind,
};
use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLayout {
    pub client_pack: PathBuf,
    pub cli_pack: PathBuf,
}

impl ResourceLayout {
    pub fn from_resource_dir(resource_dir: &Path) -> Self {
        let packs = resource_dir.join("benchmark-packs");
        Self {
            client_pack: packs.join("client-quick-v1"),
            cli_pack: packs.join("cli-quick-v1"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartupPaths {
    pub app_data: PathBuf,
    pub resource_dir: PathBuf,
}

#[derive(Clone, Default)]
pub(crate) struct CancellationRegistry {
    inner: Arc<Mutex<HashMap<Uuid, CancellationToken>>>,
}

impl CancellationRegistry {
    pub(crate) fn register(
        &self,
        run_id: Uuid,
        token: CancellationToken,
    ) -> Result<CancellationRegistration, String> {
        let mut entries = self.inner.lock();
        if entries.contains_key(&run_id) {
            return Err("run already has an active cancellation token".into());
        }
        entries.insert(run_id, token);
        Ok(CancellationRegistration {
            registry: self.clone(),
            run_id,
        })
    }

    pub(crate) fn cancel(&self, run_id: Uuid) -> bool {
        let token = self.inner.lock().get(&run_id).cloned();
        if let Some(token) = token {
            token.cancel();
            true
        } else {
            false
        }
    }

    fn remove(&self, run_id: Uuid) {
        self.inner.lock().remove(&run_id);
    }

    #[cfg(test)]
    fn contains(&self, run_id: Uuid) -> bool {
        self.inner.lock().contains_key(&run_id)
    }
}

pub(crate) struct CancellationRegistration {
    registry: CancellationRegistry,
    run_id: Uuid,
}

impl Drop for CancellationRegistration {
    fn drop(&mut self) {
        self.registry.remove(self.run_id);
    }
}

#[derive(Clone, Default)]
pub(crate) struct RunOperationRegistry {
    inner: Arc<Mutex<BTreeSet<Uuid>>>,
}

impl RunOperationRegistry {
    pub(crate) fn claim(
        &self,
        run_ids: impl IntoIterator<Item = Uuid>,
    ) -> Result<RunOperationClaim, String> {
        let mut run_ids = run_ids.into_iter().collect::<Vec<_>>();
        run_ids.sort_unstable();
        run_ids.dedup();

        let mut active = self.inner.lock();
        if run_ids.iter().any(|run_id| active.contains(run_id)) {
            return Err("run already has an active local-data operation".into());
        }
        active.extend(run_ids.iter().copied());
        Ok(RunOperationClaim {
            registry: self.clone(),
            run_ids,
        })
    }

    fn release(&self, run_ids: &[Uuid]) {
        let mut active = self.inner.lock();
        for run_id in run_ids {
            active.remove(run_id);
        }
    }

    pub(crate) fn any_active(&self) -> bool {
        !self.inner.lock().is_empty()
    }
}

pub(crate) struct RunOperationClaim {
    registry: RunOperationRegistry,
    run_ids: Vec<Uuid>,
}

impl Drop for RunOperationClaim {
    fn drop(&mut self) {
        self.registry.release(&self.run_ids);
    }
}

#[derive(Default)]
struct LocalDataGateState {
    mutation_claims: usize,
    exclusive: bool,
}

#[derive(Clone, Default)]
pub(crate) struct LocalDataGate {
    inner: Arc<Mutex<LocalDataGateState>>,
}

impl LocalDataGate {
    pub(crate) fn claim_mutating(&self) -> Result<LocalDataMutationClaim, String> {
        let mut state = self.inner.lock();
        if state.exclusive {
            return Err("an exclusive local-data snapshot is active".into());
        }
        state.mutation_claims = state
            .mutation_claims
            .checked_add(1)
            .ok_or_else(|| "too many local-data operations".to_string())?;
        Ok(LocalDataMutationClaim { gate: self.clone() })
    }

    pub(crate) fn claim_exclusive(&self) -> Result<LocalDataExclusiveClaim, String> {
        let mut state = self.inner.lock();
        if state.exclusive || state.mutation_claims != 0 {
            return Err("local data is busy".into());
        }
        state.exclusive = true;
        Ok(LocalDataExclusiveClaim { gate: self.clone() })
    }

    fn release_mutating(&self) {
        let mut state = self.inner.lock();
        state.mutation_claims = state
            .mutation_claims
            .checked_sub(1)
            .expect("a live mutation claim increments the count");
    }

    fn release_exclusive(&self) {
        let mut state = self.inner.lock();
        debug_assert!(state.exclusive);
        state.exclusive = false;
    }
}

pub(crate) struct LocalDataMutationClaim {
    gate: LocalDataGate,
}

impl Drop for LocalDataMutationClaim {
    fn drop(&mut self) {
        self.gate.release_mutating();
    }
}

pub(crate) struct LocalDataExclusiveClaim {
    gate: LocalDataGate,
}

impl Drop for LocalDataExclusiveClaim {
    fn drop(&mut self) {
        self.gate.release_exclusive();
    }
}

pub struct AppState {
    pub(crate) repository: Arc<RunRepository>,
    pub(crate) manual_runs: Arc<ManualRunService>,
    pub(crate) cli_runs: Arc<CliRunService>,
    pub(crate) client_pack: Arc<LoadedPack>,
    pub(crate) cli_pack: Arc<LoadedPack>,
    pub(crate) verifier: Arc<dyn WorkspaceVerifier>,
    pub(crate) runner: Arc<dyn ProcessRunner>,
    pub(crate) cancellations: CancellationRegistry,
    pub(crate) run_operations: RunOperationRegistry,
    pub(crate) local_data_gate: LocalDataGate,
    pub(crate) artifact_root: PathBuf,
    pub(crate) app_data: PathBuf,
    pub(crate) cleanup_pending: Arc<AtomicBool>,
}

impl AppState {
    pub fn build(app: &tauri::App) -> Result<Self, String> {
        let paths = StartupPaths {
            app_data: app
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?,
            resource_dir: app
                .path()
                .resource_dir()
                .map_err(|error| error.to_string())?,
        };
        Self::build_from_paths(paths, Arc::new(TokioProcessRunner))
    }

    pub(crate) fn build_from_paths(
        paths: StartupPaths,
        runner: Arc<dyn ProcessRunner>,
    ) -> Result<Self, String> {
        let layout = ResourceLayout::from_resource_dir(&paths.resource_dir);
        let (client_pack, cli_pack) = load_verified_packs(&layout)?;

        fs::create_dir_all(&paths.app_data).map_err(|error| error.to_string())?;
        let repository = Arc::new(
            RunRepository::open(&paths.app_data.join("ability-radar.db"))
                .map_err(|error| error.to_string())?,
        );
        repository
            .mark_running_as_interrupted()
            .map_err(|error| error.to_string())?;

        let artifact_root = paths.app_data.join("artifacts");
        fs::create_dir_all(&artifact_root).map_err(|error| error.to_string())?;
        let run_operations = RunOperationRegistry::default();
        let cleanup_pending = Arc::new(AtomicBool::new(
            crate::data_management::prune_expired_artifacts(
                &repository,
                &ArtifactStore::new(artifact_root.clone()),
                &run_operations,
                chrono::Utc::now(),
            )
            .is_err(),
        ));
        let verifier: Arc<dyn WorkspaceVerifier> =
            Arc::new(NodeVerifier::new(runner.clone(), layout.cli_pack));

        Ok(Self {
            manual_runs: Arc::new(ManualRunService::new(
                repository.clone(),
                artifact_root.clone(),
            )),
            cli_runs: Arc::new(CliRunService::new(
                repository.clone(),
                artifact_root.clone(),
            )),
            repository,
            client_pack,
            cli_pack,
            verifier,
            runner,
            cancellations: CancellationRegistry::default(),
            run_operations,
            local_data_gate: LocalDataGate::default(),
            artifact_root,
            app_data: paths.app_data,
            cleanup_pending,
        })
    }

    pub async fn target_availability(&self) -> Vec<TargetAvailability> {
        let node = probe_node(self.runner.clone()).await;
        let mut targets = vec![
            TargetAvailability {
                kind: TargetKind::ChatGptClient,
                installed: true,
                version: None,
                auth_state: AuthState::Unknown,
                prerequisites: Vec::new(),
            },
            TargetAvailability {
                kind: TargetKind::ClaudeClient,
                installed: true,
                version: None,
                auth_state: AuthState::Unknown,
                prerequisites: Vec::new(),
            },
        ];
        for adapter in fresh_provider_adapters(self.runner.clone()).values() {
            let mut availability = adapter.detect().await;
            availability.version = public_cli_version(availability.version);
            availability.prerequisites.push(node.clone());
            targets.push(availability);
        }
        targets
    }
}

pub(crate) fn fresh_provider_adapters(
    runner: Arc<dyn ProcessRunner>,
) -> BTreeMap<TargetKind, Arc<dyn AgentAdapter>> {
    let mut adapters: BTreeMap<TargetKind, Arc<dyn AgentAdapter>> = BTreeMap::new();
    adapters.insert(
        TargetKind::CodexCli,
        Arc::new(CodexAdapter::new(runner.clone())),
    );
    adapters.insert(
        TargetKind::ClaudeCode,
        Arc::new(ClaudeCodeAdapter::new(runner)),
    );
    adapters
}

fn load_verified_packs(
    layout: &ResourceLayout,
) -> Result<(Arc<LoadedPack>, Arc<LoadedPack>), String> {
    let client_pack =
        Arc::new(PackLoader::load(&layout.client_pack).map_err(|error| error.to_string())?);
    let cli_pack = Arc::new(PackLoader::load(&layout.cli_pack).map_err(|error| error.to_string())?);
    let trusted_registry =
        PackRegistry::parse(include_str!("../../../../benchmark-packs/registry.json"))
            .map_err(|error| error.to_string())?;
    trusted_registry
        .verify_bundled(&client_pack)
        .map_err(|error| error.to_string())?;
    trusted_registry
        .verify_bundled(&cli_pack)
        .map_err(|error| error.to_string())?;
    Ok((client_pack, cli_pack))
}

pub async fn probe_node(runner: Arc<dyn ProcessRunner>) -> PrerequisiteStatus {
    let output = runner
        .run(
            ProcessSpec {
                program: "node".into(),
                args: vec!["--version".into()],
                current_dir: std::env::temp_dir(),
                env: BTreeMap::new(),
                environment: ProcessEnvironment::Inherit,
                timeout: Duration::from_secs(10),
            },
            CancellationToken::new(),
        )
        .await;
    let version = match output {
        Ok(output)
            if output.exit_code == Some(0)
                && output
                    .stderr
                    .chars()
                    .all(|character| character.is_ascii_whitespace()) =>
        {
            let trimmed = output
                .stdout
                .trim_matches(|character: char| character.is_ascii_whitespace());
            parse_node_version(trimmed).map(|_| trimmed.to_owned())
        }
        _ => None,
    };
    PrerequisiteStatus {
        name: "Node.js 22/24 LTS".into(),
        available: version.as_deref().is_some_and(supported_node_lts),
        version,
    }
}

pub(crate) fn supported_node_lts(version: &str) -> bool {
    parse_node_version(version).is_some_and(|major| matches!(major, 22 | 24))
}

pub(crate) fn public_cli_version(version: Option<String>) -> Option<String> {
    let version = version?;
    if version.is_empty() || version != version.trim() || version.len() > 120 || !version.is_ascii()
    {
        return None;
    }
    let numeric = version
        .strip_prefix("codex-cli ")
        .or_else(|| version.strip_suffix(" (Claude Code)"))
        .unwrap_or(&version);
    is_version_triplet(numeric).then_some(version)
}

fn is_version_triplet(candidate: &str) -> bool {
    let mut components = candidate.split('.');
    let valid_component = |component: Option<&str>| {
        component.is_some_and(|value| {
            !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
        })
    };
    valid_component(components.next())
        && valid_component(components.next())
        && valid_component(components.next())
        && components.next().is_none()
}

fn parse_node_version(version: &str) -> Option<u32> {
    if version.len() > 64 || !version.is_ascii() {
        return None;
    }
    let bytes = version.as_bytes();
    if bytes.first() != Some(&b'v') {
        return None;
    }
    let components = version.get(1..)?.split('.').collect::<Vec<_>>();
    if components.len() != 3
        || components.iter().any(|component| {
            component.is_empty()
                || (component.len() > 1 && component.starts_with('0'))
                || !component.bytes().all(|byte| byte.is_ascii_digit())
                || component.parse::<u32>().is_err()
        })
    {
        return None;
    }
    components[0].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ability_adapters::{
        ProcessEnvironment, ProcessError, ProcessOutput, ProcessRunner, ProcessSpec,
    };
    use ability_core::{
        summarize_scores, Category, EnvironmentFingerprint, PackLoader, RunMode, RunRecord,
        RunRepository, RunStatus, TargetKind, TargetSelection, TaskOutcome, TaskResult,
    };
    use async_trait::async_trait;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    enum FakeResponse {
        Output(ProcessOutput),
        TimedOut,
        Cancelled,
    }

    struct RecordingRunner {
        response: StdMutex<Option<FakeResponse>>,
        specs: StdMutex<Vec<ProcessSpec>>,
    }

    impl RecordingRunner {
        fn output(exit_code: Option<i32>, stdout: &str, stderr: &str) -> Arc<Self> {
            Arc::new(Self {
                response: StdMutex::new(Some(FakeResponse::Output(ProcessOutput {
                    exit_code,
                    stdout: stdout.into(),
                    stderr: stderr.into(),
                    duration_ms: 1,
                }))),
                specs: StdMutex::new(Vec::new()),
            })
        }

        fn error(response: FakeResponse) -> Arc<Self> {
            Arc::new(Self {
                response: StdMutex::new(Some(response)),
                specs: StdMutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl ProcessRunner for RecordingRunner {
        async fn run(
            &self,
            spec: ProcessSpec,
            _cancellation: CancellationToken,
        ) -> Result<ProcessOutput, ProcessError> {
            self.specs.lock().unwrap().push(spec);
            match self.response.lock().unwrap().take().unwrap() {
                FakeResponse::Output(output) => Ok(output),
                FakeResponse::TimedOut => Err(ProcessError::TimedOut),
                FakeResponse::Cancelled => Err(ProcessError::Cancelled),
            }
        }
    }

    #[test]
    fn bundled_pack_paths_do_not_depend_on_the_source_checkout() {
        let layout = ResourceLayout::from_resource_dir(Path::new("D:/app/resources"));
        assert_eq!(
            layout.client_pack,
            PathBuf::from("D:/app/resources/benchmark-packs/client-quick-v1")
        );
        assert_eq!(
            layout.cli_pack,
            PathBuf::from("D:/app/resources/benchmark-packs/cli-quick-v1")
        );
    }

    #[test]
    fn only_the_v02_tested_node_lts_lines_are_accepted() {
        assert!(supported_node_lts("v22.23.1"));
        assert!(supported_node_lts("v24.18.0"));
        assert!(!supported_node_lts("v20.20.0"));
        assert!(!supported_node_lts("v26.5.0"));
        assert!(!supported_node_lts("not-node"));
    }

    #[test]
    fn public_cli_versions_are_bounded_metadata_not_raw_process_output() {
        assert_eq!(
            public_cli_version(Some("codex-cli 0.134.0".into())).as_deref(),
            Some("codex-cli 0.134.0")
        );
        assert_eq!(
            public_cli_version(Some("2.1.211 (Claude Code)".into())).as_deref(),
            Some("2.1.211 (Claude Code)")
        );
        for raw in [
            "AKIA_TASK13_BOOTSTRAP_SENTINEL_18QZ",
            r"C:\Users\Alice\.claude\credentials.json",
            r#"{"stdout":"provider payload","token":"secret"}"#,
            "codex-cli 0.134.0\nRAW_STDERR secret",
            "codex-cli 0.134.0 --flag=value",
            "codex-cli 0.134.0 AKIA_TASK13_SUFFIX_SENTINEL",
            "2.1.211 (Claude Code) RAW_STDERR secret",
        ] {
            assert_eq!(
                public_cli_version(Some(raw.into())),
                None,
                "{raw:?} must not cross the bootstrap boundary"
            );
        }
    }

    #[tokio::test]
    async fn node_probe_uses_a_direct_bounded_inherited_environment_spec() {
        let runner = RecordingRunner::output(Some(0), "v22.23.1\r\n", "");

        let result = probe_node(runner.clone()).await;

        assert!(result.available);
        assert_eq!(result.version.as_deref(), Some("v22.23.1"));
        assert_eq!(
            runner.specs.lock().unwrap().as_slice(),
            &[ProcessSpec {
                program: "node".into(),
                args: vec!["--version".into()],
                current_dir: std::env::temp_dir(),
                env: BTreeMap::new(),
                environment: ProcessEnvironment::Inherit,
                timeout: Duration::from_secs(10),
            }]
        );
    }

    #[tokio::test]
    async fn node_probe_accepts_only_exact_supported_output() {
        let accepted = [
            (Some(0), "v22.0.0", "", "v22.0.0"),
            (Some(0), "\r\nv24.18.0\r\n", " \r\n", "v24.18.0"),
        ];
        for (exit_code, stdout, stderr, expected) in accepted {
            let status = probe_node(RecordingRunner::output(exit_code, stdout, stderr)).await;
            assert!(status.available, "{stdout:?} should be accepted");
            assert_eq!(status.version.as_deref(), Some(expected));
        }

        let rejected = [
            (Some(0), "v22.1", ""),
            (Some(0), "v22.1.0-rc.1", ""),
            (Some(0), "node v22.1.0", ""),
            (Some(0), "v２２.1.0", ""),
            (Some(0), "v22.1.0\nv24.1.0", ""),
            (Some(0), "v22.1.0", "v24.1.0"),
            (Some(7), "v22.1.0", ""),
            (None, "v22.1.0", ""),
            (Some(0), "not-node", ""),
            (Some(0), "v20.20.0", ""),
            (Some(0), "v26.5.0", ""),
        ];
        for (exit_code, stdout, stderr) in rejected {
            let status = probe_node(RecordingRunner::output(exit_code, stdout, stderr)).await;
            assert!(!status.available, "{stdout:?} should be rejected");
        }

        for response in [FakeResponse::TimedOut, FakeResponse::Cancelled] {
            let status = probe_node(RecordingRunner::error(response)).await;
            assert!(!status.available);
            assert_eq!(status.version, None);
        }
    }

    #[test]
    fn cancellation_registration_is_idempotent_and_cleans_up_on_drop() {
        let registry = CancellationRegistry::default();
        let run_id = Uuid::new_v4();
        let token = CancellationToken::new();
        let registration = registry.register(run_id, token.clone()).unwrap();

        assert!(registry.cancel(run_id));
        assert!(registry.cancel(run_id));
        assert!(token.is_cancelled());
        assert!(registry.contains(run_id));

        drop(registration);
        assert!(!registry.contains(run_id));
        assert!(!registry.cancel(run_id));
    }

    #[test]
    fn bundled_resources_fail_closed_before_the_database_is_opened() {
        let resources = tempdir().unwrap();
        let app_data = tempdir().unwrap();
        let database = app_data.path().join("ability-radar.db");
        let paths = StartupPaths {
            app_data: app_data.path().to_path_buf(),
            resource_dir: resources.path().to_path_buf(),
        };

        let missing = AppState::build_from_paths(
            paths.clone(),
            RecordingRunner::output(Some(0), "v22.0.0", ""),
        );
        assert!(missing.is_err());
        assert!(!database.exists());

        copy_bundled_packs(resources.path());
        fs::write(
            resources
                .path()
                .join("benchmark-packs/client-quick-v1/prompts/logic-truth.txt"),
            "tampered",
        )
        .unwrap();
        let tampered =
            AppState::build_from_paths(paths, RecordingRunner::output(Some(0), "v22.0.0", ""));
        assert!(tampered.is_err());
        assert!(!database.exists());
    }

    #[test]
    fn startup_verifies_resources_then_recovers_running_rows() {
        let resources = tempdir().unwrap();
        let app_data = tempdir().unwrap();
        copy_bundled_packs(resources.path());
        let client_pack =
            PackLoader::load(&resources.path().join("benchmark-packs/client-quick-v1")).unwrap();
        let database = app_data.path().join("ability-radar.db");
        let repository = RunRepository::open(&database).unwrap();
        let mut run = RunRecord::new(
            TargetSelection {
                kind: TargetKind::ChatGptClient,
                reported_model: "test-model".into(),
                reasoning_effort: None,
            },
            RunMode::Quick,
            client_pack.manifest.id.clone(),
            client_pack.manifest.version.clone(),
            u32::try_from(client_pack.tasks.len()).unwrap(),
            EnvironmentFingerprint {
                os_family: "windows".into(),
                os_version: "test".into(),
                app_version: "0.2.0".into(),
                cli_version: None,
                verifier_runtime_version: None,
                suite_id: client_pack.manifest.id.clone(),
                suite_version: client_pack.manifest.version.clone(),
                suite_content_sha256: client_pack.content_sha256,
                scoring_rule_version: "ability-v1".into(),
                resumed: false,
            },
        );
        run.status = RunStatus::Running;
        repository.insert_run(&run).unwrap();
        drop(repository);

        let state = AppState::build_from_paths(
            StartupPaths {
                app_data: app_data.path().to_path_buf(),
                resource_dir: resources.path().to_path_buf(),
            },
            RecordingRunner::output(Some(0), "v22.0.0", ""),
        )
        .unwrap();

        assert_eq!(
            state.repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Interrupted
        );
    }

    #[test]
    fn hostile_startup_retention_sets_retryable_pending_state_without_bricking_app() {
        let resources = tempdir().unwrap();
        let app_data = tempdir().unwrap();
        copy_bundled_packs(resources.path());
        let database = app_data.path().join("ability-radar.db");
        let repository = RunRepository::open(&database).unwrap();
        repository.set_raw_retention_days(Some(7)).unwrap();
        let client_pack =
            PackLoader::load(&resources.path().join("benchmark-packs/client-quick-v1")).unwrap();
        let mut expired = RunRecord::new(
            TargetSelection {
                kind: TargetKind::ChatGptClient,
                reported_model: "fake-model".into(),
                reasoning_effort: None,
            },
            RunMode::Quick,
            client_pack.manifest.id.clone(),
            client_pack.manifest.version.clone(),
            1,
            EnvironmentFingerprint {
                os_family: "windows".into(),
                os_version: "test".into(),
                app_version: "0.2.0".into(),
                cli_version: None,
                verifier_runtime_version: None,
                suite_id: client_pack.manifest.id.clone(),
                suite_version: client_pack.manifest.version.clone(),
                suite_content_sha256: client_pack.content_sha256,
                scoring_rule_version: "ability-v1".into(),
                resumed: false,
            },
        );
        expired.status = RunStatus::Running;
        repository.insert_run(&expired).unwrap();
        let evidence = TaskResult {
            run_id: expired.id,
            task_id: "retention-fixture".into(),
            category: Category::Logic,
            outcome: TaskOutcome::Passed,
            score: Some(100.0),
            failure_kind: None,
            duration_ms: 1,
            answer_rel_path: None,
            detail: "coherent retention fixture".into(),
        };
        repository.save_task_result(&evidence).unwrap();
        let score = summarize_scores(&[evidence], 1).unwrap();
        repository.complete_run(expired.id, Some(&score)).unwrap();
        // Shift only the lifecycle-produced terminal timestamp to exercise the
        // deterministic startup-retention boundary.
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute(
                "UPDATE runs SET finished_at=?2 WHERE id=?1",
                rusqlite::params![
                    expired.id.to_string(),
                    (chrono::Utc::now() - chrono::Duration::days(8)).to_rfc3339()
                ],
            )
            .unwrap();
        let hostile = app_data
            .path()
            .join("artifacts/runs")
            .join(expired.id.to_string());
        fs::create_dir_all(&hostile).unwrap();
        fs::write(hostile.join("owner.bin"), "not app-owned layout").unwrap();
        drop(repository);
        let paths = StartupPaths {
            app_data: app_data.path().to_path_buf(),
            resource_dir: resources.path().to_path_buf(),
        };

        let pending = AppState::build_from_paths(
            paths.clone(),
            RecordingRunner::output(Some(0), "v22.0.0", ""),
        )
        .unwrap();
        assert!(pending
            .cleanup_pending
            .load(std::sync::atomic::Ordering::SeqCst));
        assert!(pending.repository.get_run(expired.id).unwrap().is_some());
        drop(pending);

        fs::remove_file(hostile.join("owner.bin")).unwrap();
        fs::write(hostile.join("answer.txt"), "app-owned raw answer").unwrap();
        let retried =
            AppState::build_from_paths(paths, RecordingRunner::output(Some(0), "v22.0.0", ""))
                .unwrap();
        assert!(!retried
            .cleanup_pending
            .load(std::sync::atomic::Ordering::SeqCst));
        assert!(!hostile.exists());
        assert!(retried.repository.get_run(expired.id).unwrap().is_some());
    }

    #[test]
    fn run_operation_claims_are_exclusive_and_batch_claims_are_atomic() {
        let registry = RunOperationRegistry::default();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let third = Uuid::new_v4();

        let first_claim = registry.claim([first]).unwrap();
        assert!(registry.claim([first]).is_err());
        assert!(
            registry.claim([second, first, third]).is_err(),
            "a conflicting batch may not partially claim its free IDs"
        );
        let free_batch = registry.claim([second, third]).unwrap();
        assert!(registry.claim([second]).is_err());

        drop(first_claim);
        assert!(registry.claim([first]).is_ok());
        drop(free_batch);
        assert!(registry.claim([second, third]).is_ok());
    }

    #[test]
    fn local_data_gate_excludes_backup_and_mutations_in_both_directions() {
        let gate = LocalDataGate::default();
        let first_mutation = gate.claim_mutating().unwrap();
        let second_mutation = gate.claim_mutating().unwrap();
        assert!(gate.claim_exclusive().is_err());
        drop(first_mutation);
        assert!(gate.claim_exclusive().is_err());
        drop(second_mutation);

        let backup = gate.claim_exclusive().unwrap();
        assert!(gate.claim_exclusive().is_err());
        assert!(gate.claim_mutating().is_err());
        drop(backup);
        assert!(gate.claim_mutating().is_ok());
    }

    #[test]
    fn local_data_gate_claims_release_on_every_error_return() {
        fn failing_mutation(gate: &LocalDataGate) -> Result<(), &'static str> {
            let _claim = gate.claim_mutating().map_err(|_| "busy")?;
            Err("injected mutation failure")
        }

        fn failing_backup(gate: &LocalDataGate) -> Result<(), &'static str> {
            let _claim = gate.claim_exclusive().map_err(|_| "busy")?;
            Err("injected backup failure")
        }

        let gate = LocalDataGate::default();
        assert_eq!(failing_mutation(&gate), Err("injected mutation failure"));
        assert!(gate.claim_exclusive().is_ok());
        assert_eq!(failing_backup(&gate), Err("injected backup failure"));
        assert!(gate.claim_mutating().is_ok());
    }

    #[test]
    fn run_operation_registry_reports_active_claims_for_backup_recheck() {
        let registry = RunOperationRegistry::default();
        let claim = registry.claim([Uuid::new_v4()]).unwrap();
        assert!(registry.any_active());
        drop(claim);
        assert!(!registry.any_active());
    }

    fn copy_bundled_packs(resource_dir: &Path) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../benchmark-packs");
        copy_tree(&source, &resource_dir.join("benchmark-packs"));
    }

    fn copy_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}
