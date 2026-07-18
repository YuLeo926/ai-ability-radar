#![cfg(windows)]

use ability_core::{ArtifactStore, ArtifactStoreError, TargetKind};
use std::fs;
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
