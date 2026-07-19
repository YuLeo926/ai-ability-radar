#![cfg(windows)]

use ability_core::{ArtifactStore, ArtifactStoreError, BackupRunBinding, TargetKind};
use std::fs;
use std::io::Read;
use std::process::Command;
use tempfile::tempdir;
use uuid::Uuid;

#[test]
fn handle_scoped_deletion_accepts_only_app_owned_manual_and_cli_layouts() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let manual_id = Uuid::new_v4();
    let manual = root.join("runs").join(manual_id.to_string());
    fs::create_dir_all(&manual).unwrap();
    fs::write(manual.join("answer.txt"), "raw manual answer").unwrap();
    let cli_id = Uuid::new_v4();
    let cli = root.join("runs").join(cli_id.to_string());
    fs::create_dir_all(cli.join("logs")).unwrap();
    fs::create_dir_all(cli.join("workspaces/task-one/src")).unwrap();
    fs::write(cli.join("logs/task-one.log"), "raw CLI log").unwrap();
    fs::write(
        cli.join("workspaces/task-one/src/index.mjs"),
        "workspace data",
    )
    .unwrap();
    let store = ArtifactStore::new(root);

    assert!(
        store
            .delete_run_artifacts(manual_id, TargetKind::ChatGptClient)
            .unwrap()
    );
    assert!(!manual.exists());
    assert!(
        store
            .delete_run_artifacts(cli_id, TargetKind::CodexCli)
            .unwrap()
    );
    assert!(!cli.exists());
    assert!(
        !store
            .delete_run_artifacts(cli_id, TargetKind::CodexCli)
            .unwrap()
    );
}

#[test]
fn deletion_rejects_unknown_top_level_entries_without_traversing_them() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let run_id = Uuid::new_v4();
    let run = root.join("runs").join(run_id.to_string());
    fs::create_dir_all(run.join("unknown-private-tree")).unwrap();
    fs::write(run.join("unknown-private-tree/owner.txt"), "preserve").unwrap();
    fs::write(run.join("answer.txt"), "raw").unwrap();
    let store = ArtifactStore::new(root);

    assert!(matches!(
        store.delete_run_artifacts(run_id, TargetKind::ChatGptClient),
        Err(ArtifactStoreError::UnexpectedEntry)
    ));
    assert_eq!(
        fs::read_to_string(run.join("unknown-private-tree/owner.txt")).unwrap(),
        "preserve"
    );
}

#[test]
fn deletion_rejects_run_and_descendant_junctions_without_touching_their_targets() {
    for descendant in [false, true] {
        let directory = tempdir().unwrap();
        let root = directory.path().join("artifacts");
        let run_id = Uuid::new_v4();
        let run = root.join("runs").join(run_id.to_string());
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("outside.txt"), "outside-owned").unwrap();
        fs::create_dir_all(root.join("runs")).unwrap();
        if descendant {
            fs::create_dir(&run).unwrap();
            let status = Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    run.join("logs").to_str().unwrap(),
                    outside.path().to_str().unwrap(),
                ])
                .status()
                .unwrap();
            assert!(status.success());
        } else {
            let status = Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    run.to_str().unwrap(),
                    outside.path().to_str().unwrap(),
                ])
                .status()
                .unwrap();
            assert!(status.success());
        }
        let store = ArtifactStore::new(root);

        assert!(matches!(
            store.delete_run_artifacts(
                run_id,
                if descendant {
                    TargetKind::CodexCli
                } else {
                    TargetKind::ChatGptClient
                }
            ),
            Err(ArtifactStoreError::UnsafeLayout)
        ));
        assert_eq!(
            fs::read_to_string(outside.path().join("outside.txt")).unwrap(),
            "outside-owned"
        );
    }
}

#[test]
fn direct_unknown_files_are_refused_instead_of_assumed_app_owned() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let run_id = Uuid::new_v4();
    let run = root.join("runs").join(run_id.to_string());
    fs::create_dir_all(&run).unwrap();
    fs::write(run.join("owner.bin"), "unknown").unwrap();
    let store = ArtifactStore::new(root);

    assert!(matches!(
        store.delete_run_artifacts(run_id, TargetKind::ChatGptClient),
        Err(ArtifactStoreError::UnexpectedEntry)
    ));
    assert_eq!(
        fs::read_to_string(run.join("owner.bin")).unwrap(),
        "unknown"
    );
}

#[test]
fn manual_layout_rejects_cli_subtrees_instead_of_using_a_union_policy() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let run_id = Uuid::new_v4();
    let run = root.join("runs").join(run_id.to_string());
    fs::create_dir_all(run.join("logs")).unwrap();
    fs::write(run.join("logs/task.log"), "not a manual artifact").unwrap();
    let store = ArtifactStore::new(root);

    assert!(matches!(
        store.delete_run_artifacts(run_id, TargetKind::ChatGptClient),
        Err(ArtifactStoreError::UnexpectedEntry)
    ));
    assert!(run.join("logs/task.log").exists());
}

#[test]
fn backup_enumeration_returns_sorted_uuid_scoped_names_and_retained_file_handles() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let manual_id = Uuid::new_v4();
    let manual = root.join("runs").join(manual_id.to_string());
    fs::create_dir_all(&manual).unwrap();
    fs::write(manual.join("z.txt"), "manual-z").unwrap();
    fs::write(manual.join("a.txt"), "manual-a").unwrap();
    let cli_id = Uuid::new_v4();
    let cli = root.join("runs").join(cli_id.to_string());
    fs::create_dir_all(cli.join("logs")).unwrap();
    fs::create_dir_all(cli.join("workspaces/task-one/src")).unwrap();
    fs::write(cli.join("logs/task-one.log"), "cli-log").unwrap();
    fs::write(cli.join("workspaces/task-one/src/index.mjs"), "workspace").unwrap();
    let store = ArtifactStore::new(root);

    let mut files = store
        .open_backup_files(&[
            BackupRunBinding {
                id: manual_id,
                target: TargetKind::ChatGptClient,
            },
            BackupRunBinding {
                id: cli_id,
                target: TargetKind::CodexCli,
            },
        ])
        .unwrap();
    let names = files
        .iter()
        .map(|file| file.zip_name().to_owned())
        .collect::<Vec<_>>();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert_eq!(names.len(), 4);
    assert!(names.iter().all(|name| {
        name.starts_with("artifacts/runs/") && !name.contains('\\') && !name.contains("/../")
    }));

    let locked_source = manual.join("a.txt");
    assert!(
        fs::rename(&locked_source, manual.join("attacker.txt")).is_err(),
        "the returned read handle must deny replacement/deletion sharing"
    );
    let file = files
        .iter_mut()
        .find(|file| file.zip_name().ends_with("/a.txt"))
        .unwrap();
    let mut bytes = String::new();
    file.read_to_string(&mut bytes).unwrap();
    assert_eq!(bytes, "manual-a");
}

#[test]
fn backup_enumeration_rejects_unknown_snapshot_uuid_and_target_layout_mismatch() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let known = Uuid::new_v4();
    let unknown = Uuid::new_v4();
    fs::create_dir_all(root.join("runs").join(unknown.to_string())).unwrap();
    fs::write(
        root.join("runs")
            .join(unknown.to_string())
            .join("answer.txt"),
        "unknown",
    )
    .unwrap();
    let store = ArtifactStore::new(root.clone());
    assert!(matches!(
        store.open_backup_files(&[BackupRunBinding {
            id: known,
            target: TargetKind::ChatGptClient,
        }]),
        Err(ArtifactStoreError::UnexpectedEntry)
    ));

    fs::remove_dir_all(root.join("runs").join(unknown.to_string())).unwrap();
    let known_dir = root.join("runs").join(known.to_string());
    fs::create_dir_all(known_dir.join("logs")).unwrap();
    fs::write(known_dir.join("logs/task.log"), "wrong target layout").unwrap();
    assert!(matches!(
        store.open_backup_files(&[BackupRunBinding {
            id: known,
            target: TargetKind::ChatGptClient,
        }]),
        Err(ArtifactStoreError::UnexpectedEntry)
    ));
}

#[test]
fn backup_enumeration_rejects_duplicate_file_identity_from_hard_links() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let run_id = Uuid::new_v4();
    let run = root.join("runs").join(run_id.to_string());
    fs::create_dir_all(&run).unwrap();
    let first = run.join("first.txt");
    fs::write(&first, "same private bytes").unwrap();
    fs::hard_link(&first, run.join("second.txt")).unwrap();

    let store = ArtifactStore::new(root);
    assert!(matches!(
        store.open_backup_files(&[BackupRunBinding {
            id: run_id,
            target: TargetKind::ChatGptClient,
        }]),
        Err(ArtifactStoreError::UnexpectedEntry)
    ));
}

#[test]
fn backup_enumeration_rejects_unknown_artifact_root_entries() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("owner.bin"), "not an app-owned backup entry").unwrap();

    let store = ArtifactStore::new(root);
    assert!(matches!(
        store.open_backup_files(&[]),
        Err(ArtifactStoreError::UnexpectedEntry)
    ));
}

#[test]
fn backup_enumeration_rejects_root_run_and_descendant_junctions() {
    for location in ["root", "run", "descendant"] {
        let directory = tempdir().unwrap();
        let artifact_root = directory.path().join("artifacts");
        let run_id = Uuid::new_v4();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("outside.txt"), "outside-owned").unwrap();

        match location {
            "root" => {
                let status = Command::new("cmd")
                    .args([
                        "/C",
                        "mklink",
                        "/J",
                        artifact_root.to_str().unwrap(),
                        outside.path().to_str().unwrap(),
                    ])
                    .status()
                    .unwrap();
                assert!(status.success());
            }
            "run" => {
                fs::create_dir_all(artifact_root.join("runs")).unwrap();
                let status = Command::new("cmd")
                    .args([
                        "/C",
                        "mklink",
                        "/J",
                        artifact_root
                            .join("runs")
                            .join(run_id.to_string())
                            .to_str()
                            .unwrap(),
                        outside.path().to_str().unwrap(),
                    ])
                    .status()
                    .unwrap();
                assert!(status.success());
            }
            "descendant" => {
                let run = artifact_root.join("runs").join(run_id.to_string());
                fs::create_dir_all(&run).unwrap();
                let status = Command::new("cmd")
                    .args([
                        "/C",
                        "mklink",
                        "/J",
                        run.join("logs").to_str().unwrap(),
                        outside.path().to_str().unwrap(),
                    ])
                    .status()
                    .unwrap();
                assert!(status.success());
            }
            _ => unreachable!(),
        }

        let store = ArtifactStore::new(artifact_root);
        assert!(
            store
                .open_backup_files(&[BackupRunBinding {
                    id: run_id,
                    target: TargetKind::CodexCli,
                }])
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(outside.path().join("outside.txt")).unwrap(),
            "outside-owned"
        );
    }
}
