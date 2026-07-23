use crate::app_state::RunOperationRegistry;
use crate::data_management::{create_full_backup, prune_expired_artifacts};
use ability_core::{
    summarize_scores, ArtifactStore, Category, EnvironmentFingerprint, ModelSource,
    ModelVerification, RunMode, RunRecord, RunRepository, RunStatus, TargetKind, TargetSelection,
    TaskOutcome, TaskResult,
};
use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use tempfile::tempdir;
use uuid::Uuid;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-19T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn run(target: TargetKind) -> RunRecord {
    let mut run = RunRecord::new(
        TargetSelection {
            kind: target,
            reported_model: "fake-model".into(),
            reasoning_effort: None,
            model_source: ModelSource::LegacyUnknown,
            model_verification: ModelVerification::LegacyUnknown,
        },
        RunMode::Quick,
        "test-suite".into(),
        "1.0.0".into(),
        1,
        EnvironmentFingerprint {
            os_family: "windows".into(),
            os_version: "test".into(),
            app_version: "0.2.0".into(),
            cli_version: None,
            verifier_runtime_version: None,
            suite_id: "test-suite".into(),
            suite_version: "1.0.0".into(),
            suite_content_sha256: "a".repeat(64),
            scoring_rule_version: "ability-v1".into(),
            resumed: false,
        },
    );
    run.status = RunStatus::Running;
    run
}

fn result(run_id: Uuid) -> TaskResult {
    TaskResult {
        run_id,
        task_id: "answer".into(),
        category: Category::Logic,
        outcome: TaskOutcome::Passed,
        score: Some(100.0),
        failure_kind: None,
        duration_ms: 1,
        answer_rel_path: Some(format!("runs/{run_id}/answer.txt")),
        detail: "evidence preserved".into(),
    }
}

fn persist_terminal(repository: &RunRepository, run: RunRecord, status: RunStatus) -> RunRecord {
    repository.insert_run(&run).unwrap();
    let evidence = result(run.id);
    repository.save_task_result(&evidence).unwrap();
    match status {
        RunStatus::Completed => {
            let score = summarize_scores(&[evidence], 1).unwrap();
            repository.complete_run(run.id, Some(&score)).unwrap();
        }
        RunStatus::Interrupted | RunStatus::Cancelled => {
            repository.finish_without_score(run.id, status).unwrap();
        }
        _ => panic!("terminal fixture requires a terminal status"),
    }
    repository.get_run(run.id).unwrap().unwrap()
}

fn set_fixture_finished_at(database: &std::path::Path, run: &RunRecord, value: DateTime<Utc>) {
    // Retention tests need a deterministic clock; the run reached its terminal
    // state through the production lifecycle before only its timestamp is shifted.
    Connection::open(database)
        .unwrap()
        .execute(
            "UPDATE runs SET finished_at=?2 WHERE id=?1",
            rusqlite::params![run.id.to_string(), value.to_rfc3339()],
        )
        .unwrap();
}

#[test]
fn pruning_deletes_only_expired_terminal_raw_trees_and_preserves_evidence() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    repository.set_raw_retention_days(Some(7)).unwrap();
    let expired = persist_terminal(
        &repository,
        run(TargetKind::ChatGptClient),
        RunStatus::Completed,
    );
    let interrupted = persist_terminal(
        &repository,
        run(TargetKind::ChatGptClient),
        RunStatus::Interrupted,
    );
    set_fixture_finished_at(&database, &expired, now() - Duration::days(7));
    set_fixture_finished_at(&database, &interrupted, now() - Duration::days(30));
    let artifact_root = directory.path().join("artifacts");
    for id in [expired.id, interrupted.id] {
        let tree = artifact_root.join("runs").join(id.to_string());
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("answer.txt"), id.to_string()).unwrap();
    }
    let store = ArtifactStore::new(artifact_root.clone());
    let operations = RunOperationRegistry::default();

    assert_eq!(
        prune_expired_artifacts(&repository, &store, &operations, now()).unwrap(),
        1
    );
    assert!(!artifact_root
        .join("runs")
        .join(expired.id.to_string())
        .exists());
    assert!(artifact_root
        .join("runs")
        .join(interrupted.id.to_string())
        .exists());
    let retained = repository.get_task_results(expired.id).unwrap().remove(0);
    assert_eq!(retained.answer_rel_path, None);
    assert_eq!(retained.score, Some(100.0));
    assert_eq!(retained.detail, "evidence preserved");
    assert!(repository.get_run(expired.id).unwrap().is_some());
}

#[test]
fn pruning_claims_the_complete_candidate_set_before_artifact_access() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    repository.set_raw_retention_days(Some(7)).unwrap();
    let first = persist_terminal(
        &repository,
        run(TargetKind::ChatGptClient),
        RunStatus::Completed,
    );
    let second = persist_terminal(
        &repository,
        run(TargetKind::ChatGptClient),
        RunStatus::Completed,
    );
    set_fixture_finished_at(&database, &first, now() - Duration::days(8));
    set_fixture_finished_at(&database, &second, now() - Duration::days(9));
    let artifact_root = directory.path().join("artifacts");
    for id in [first.id, second.id] {
        let tree = artifact_root.join("runs").join(id.to_string());
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("answer.txt"), "raw").unwrap();
    }
    let store = ArtifactStore::new(artifact_root.clone());
    let operations = RunOperationRegistry::default();
    let conflict = operations.claim([second.id]).unwrap();

    assert!(prune_expired_artifacts(&repository, &store, &operations, now()).is_err());
    assert!(artifact_root
        .join("runs")
        .join(first.id.to_string())
        .exists());
    assert!(artifact_root
        .join("runs")
        .join(second.id.to_string())
        .exists());
    drop(conflict);
}

#[test]
fn full_backup_has_exact_unique_safe_entries_manifest_and_readable_snapshot() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let manual = persist_terminal(
        &repository,
        run(TargetKind::ChatGptClient),
        RunStatus::Completed,
    );
    let artifact_root = directory.path().join("artifacts");
    let tree = artifact_root.join("runs").join(manual.id.to_string());
    fs::create_dir_all(&tree).unwrap();
    fs::write(tree.join("answer.txt"), "raw answer sentinel").unwrap();
    let store = ArtifactStore::new(artifact_root);
    let snapshot_path = directory.path().join("private-snapshot.sqlite");
    let mut snapshot = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&snapshot_path)
        .unwrap();
    let mut archive = Vec::new();

    create_full_backup(
        &repository,
        &store,
        &snapshot_path,
        &mut snapshot,
        &mut archive,
        now(),
        "0.2.0",
    )
    .unwrap();
    if let Some(path) = std::env::var_os("ABILITY_RADAR_TEST_ZIP_OUT") {
        fs::write(path, &archive).unwrap();
    }
    let entries = read_stored_zip(&archive);
    let names = entries.keys().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        names,
        BTreeSet::from([
            "ability-radar.sqlite".to_owned(),
            "backup-manifest.json".to_owned(),
            format!("artifacts/runs/{}/answer.txt", manual.id),
        ])
    );
    assert_eq!(entries.len(), names.len());
    assert!(names.iter().all(|name| {
        !name.starts_with(['/', '\\'])
            && !name.contains('\\')
            && !name
                .split('/')
                .any(|component| matches!(component, "" | "." | ".."))
    }));

    let manifest: Value = serde_json::from_slice(&entries["backup-manifest.json"]).unwrap();
    assert_eq!(
        manifest,
        serde_json::json!({
            "schemaVersion": 1,
            "createdAt": "2026-07-19T12:00:00Z",
            "appVersion": "0.2.0",
            "containsRawAnswersAndLogs": true,
            "encrypted": false
        })
    );
    assert!(!manifest
        .to_string()
        .contains(directory.path().to_string_lossy().as_ref()));
    assert_eq!(
        entries[&format!("artifacts/runs/{}/answer.txt", manual.id)],
        b"raw answer sentinel"
    );

    let snapshot_path = directory.path().join("inspected.sqlite");
    fs::write(&snapshot_path, &entries["ability-radar.sqlite"]).unwrap();
    let snapshot = Connection::open(snapshot_path).unwrap();
    let count: i64 = snapshot
        .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn full_backup_propagates_stream_failure_without_creating_internal_archive_files() {
    struct FailAfter {
        remaining: usize,
    }
    impl Write for FailAfter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::other("injected ZIP failure"));
            }
            let written = bytes.len().min(self.remaining);
            self.remaining -= written;
            Ok(written)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let store = ArtifactStore::new(directory.path().join("artifacts"));
    let snapshot_path = directory.path().join("private-snapshot.sqlite");
    let mut snapshot = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&snapshot_path)
        .unwrap();
    let mut writer = FailAfter { remaining: 20 };
    assert!(create_full_backup(
        &repository,
        &store,
        &snapshot_path,
        &mut snapshot,
        &mut writer,
        now(),
        "0.2.0",
    )
    .is_err());
    drop(snapshot);
    fs::remove_file(snapshot_path).unwrap();
    assert_eq!(
        fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            std::ffi::OsString::from("ability.db"),
            std::ffi::OsString::from("ability.db-shm"),
            std::ffi::OsString::from("ability.db-wal"),
        ])
    );
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_stored_zip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let eocd = bytes
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .unwrap();
    let entries = usize::from(read_u16(bytes, eocd + 10));
    let mut central = read_u32(bytes, eocd + 16) as usize;
    let mut output = BTreeMap::new();
    for _ in 0..entries {
        assert_eq!(&bytes[central..central + 4], b"PK\x01\x02");
        assert_eq!(read_u16(bytes, central + 10), 0);
        let size = read_u32(bytes, central + 24) as usize;
        let name_len = usize::from(read_u16(bytes, central + 28));
        let extra_len = usize::from(read_u16(bytes, central + 30));
        let comment_len = usize::from(read_u16(bytes, central + 32));
        let local = read_u32(bytes, central + 42) as usize;
        let name = std::str::from_utf8(&bytes[central + 46..central + 46 + name_len])
            .unwrap()
            .to_owned();
        assert_eq!(&bytes[local..local + 4], b"PK\x03\x04");
        let local_name_len = usize::from(read_u16(bytes, local + 26));
        let local_extra_len = usize::from(read_u16(bytes, local + 28));
        let data = local + 30 + local_name_len + local_extra_len;
        assert!(output
            .insert(name, bytes[data..data + size].to_vec())
            .is_none());
        central += 46 + name_len + extra_len + comment_len;
    }
    output
}
