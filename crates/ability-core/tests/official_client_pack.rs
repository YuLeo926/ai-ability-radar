use ability_core::{Category, PackError, PackLoader, PackRegistry, TargetKind, grade_submission};
use std::fs;
use std::path::PathBuf;

#[test]
fn client_quick_pack_has_the_approved_shape_and_gold_answers() {
    let packs_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmark-packs");
    let root = packs_root.join("client-quick-v1");
    let registry =
        PackRegistry::parse(&fs::read_to_string(packs_root.join("registry.json")).unwrap())
            .unwrap();
    let pack = PackLoader::load(&root).unwrap();
    registry.verify_bundled(&pack).unwrap();
    let client_entry = registry
        .packs
        .iter()
        .find(|entry| entry.id == "client-quick")
        .unwrap();
    assert_eq!(client_entry.path, "client-quick-v1");
    assert_eq!(
        client_entry.content_sha256,
        "cfd2b36af1688432626ee80e453d60cd1d8cb4f87371df5f53def6b551e06f8f"
    );
    let cli_entry = registry
        .packs
        .iter()
        .find(|entry| entry.id == "cli-quick")
        .unwrap();
    assert_eq!(cli_entry.path, "cli-quick-v1");
    assert_eq!(cli_entry.content_sha256, "0".repeat(64));
    let mut unsealed_client_registry = registry.clone();
    unsealed_client_registry
        .packs
        .iter_mut()
        .find(|entry| entry.id == "client-quick")
        .unwrap()
        .content_sha256 = "0".repeat(64);
    assert!(matches!(
        unsealed_client_registry.verify_bundled(&pack),
        Err(PackError::HashMismatch { .. })
    ));
    assert_eq!(pack.manifest.id, "client-quick");
    assert_eq!(pack.manifest.version, "1.0.0");
    assert_eq!(
        pack.manifest.target_kinds,
        vec![TargetKind::ChatGptClient, TargetKind::ClaudeClient]
    );
    assert_eq!(
        pack.tasks
            .iter()
            .map(|task| task.definition.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "instruction-filter",
            "instruction-csv",
            "instruction-inventory",
            "logic-schedule",
            "logic-truth",
            "logic-capacity",
            "review-python",
            "review-typescript",
        ]
    );
    assert_eq!(
        pack.tasks
            .iter()
            .map(|task| task.definition.time_budget_secs)
            .collect::<Vec<_>>(),
        vec![120, 120, 120, 120, 120, 120, 180, 180]
    );
    assert!(pack.tasks.iter().all(|task| task.definition.max_turns == 1));
    assert!(pack.tasks.iter().all(|task| !task.prompt.is_empty()));
    assert_eq!(pack.tasks.len(), 8);
    assert_eq!(
        pack.tasks
            .iter()
            .filter(|task| task.definition.category == Category::InstructionFollowing)
            .count(),
        3
    );
    assert_eq!(
        pack.tasks
            .iter()
            .filter(|task| task.definition.category == Category::Logic)
            .count(),
        3
    );
    assert_eq!(
        pack.tasks
            .iter()
            .filter(|task| task.definition.category == Category::CodeReview)
            .count(),
        2
    );

    let gold = [
        r#"{"count":3,"names":["Mira","An","Bo"]}"#,
        "sku,total\nB2,42\nC3,35",
        r#"[{"sku":"C","net":90},{"sku":"A","net":72}]"#,
        r#"{"09:00":"D","10:00":"B","11:00":"A","12:00":"C"}"#,
        r#"{"liar":"B","box":3}"#,
        r#"{"trips":4,"unused":6}"#,
        r#"["A","D"]"#,
        r#"["A","C"]"#,
    ];
    for (task, answer) in pack.tasks.iter().zip(gold) {
        assert!(
            grade_submission(&task.definition.grader, answer).passed,
            "{}",
            task.definition.id
        );
    }
}
