use ability_core::{PackError, PackLoader, PackRegistry};
use std::fs;
use std::fs::File;
use tempfile::tempdir;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

fn minimal_manifest(prompt_file: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "id": "security-pack",
        "version": "1.0.0",
        "title": "Security Pack",
        "target_kinds": ["chat_gpt_client"],
        "tasks": [{
            "id": "security-1",
            "category": "logic",
            "prompt_file": prompt_file,
            "starter_dir": null,
            "time_budget_secs": 60,
            "max_turns": 1,
            "grader": {"type":"exact_text","expected":"4"}
        }]
    })
}

fn write_minimal_pack(root: &std::path::Path, prompt_file: &str) {
    fs::write(root.join("prompt.txt"), "Only answer 4.").unwrap();
    let manifest = minimal_manifest(prompt_file);
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}

fn manifest_bytes_with_size(size: usize) -> Vec<u8> {
    let mut manifest = minimal_manifest("prompt.txt");
    manifest["title"] = "".into();
    let base_len = serde_json::to_vec(&manifest).unwrap().len();
    assert!(size > base_len);
    manifest["title"] = "x".repeat(size - base_len).into();
    let bytes = serde_json::to_vec(&manifest).unwrap();
    assert_eq!(bytes.len(), size);
    bytes
}

#[test]
fn loads_a_minimal_pack_and_computes_a_hash() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("prompt.txt"), "Only answer 4.").unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        r#"{
          "schema_version": 1,
          "id": "smoke-pack",
          "version": "1.0.0",
          "title": "Smoke Pack",
          "target_kinds": ["chat_gpt_client"],
          "tasks": [{
            "id": "smoke-1",
            "category": "logic",
            "prompt_file": "prompt.txt",
            "starter_dir": null,
            "time_budget_secs": 60,
            "max_turns": 1,
            "grader": {"type":"exact_text","expected":"4"}
          }]
        }"#,
    )
    .unwrap();

    let pack = PackLoader::load(dir.path()).unwrap();
    assert_eq!(pack.manifest.id, "smoke-pack");
    assert_eq!(pack.tasks[0].prompt, "Only answer 4.");
    assert_eq!(pack.content_sha256.len(), 64);
}

fn raw_runtime_manifest(schema: &str, time_budget: &str, max_turns: &str, grader: &str) -> String {
    format!(
        r#"{{"schema_version":{schema},"id":"security-pack","version":"1.0.0","title":"Security Pack","target_kinds":["chat_gpt_client"],"tasks":[{{"id":"security-1","category":"logic","prompt_file":"prompt.txt","starter_dir":null,"time_budget_secs":{time_budget},"max_turns":{max_turns},"grader":{grader}}}]}}"#
    )
}

#[test]
fn runtime_parser_rejects_nonportable_json_and_numeric_forms() {
    let cases = [
        (
            "UTF-8 BOM",
            format!(
                "\u{feff}{}",
                raw_runtime_manifest("1", "30", "1", r#"{"type":"exact_text","expected":"4"}"#)
            ),
        ),
        (
            "duplicate object keys",
            raw_runtime_manifest(
                r#"2,"schema_version":1"#,
                "30",
                "1",
                r#"{"type":"exact_text","expected":"4"}"#,
            ),
        ),
        (
            "fractional integer lexeme",
            raw_runtime_manifest("1", "30", "1.0", r#"{"type":"exact_text","expected":"4"}"#),
        ),
        (
            "exponent integer lexeme",
            raw_runtime_manifest("1", "3e1", "1", r#"{"type":"exact_text","expected":"4"}"#),
        ),
        (
            "u64 overflow",
            raw_runtime_manifest(
                "1",
                "18446744073709551616",
                "1",
                r#"{"type":"exact_text","expected":"4"}"#,
            ),
        ),
        (
            "non-finite exact JSON number",
            raw_runtime_manifest("1", "30", "1", r#"{"type":"exact_json","expected":1e400}"#),
        ),
        (
            "time budget below range",
            raw_runtime_manifest("1", "0", "1", r#"{"type":"exact_text","expected":"4"}"#),
        ),
        (
            "time budget above range",
            raw_runtime_manifest("1", "7201", "1", r#"{"type":"exact_text","expected":"4"}"#),
        ),
        (
            "turn count below range",
            raw_runtime_manifest("1", "30", "0", r#"{"type":"exact_text","expected":"4"}"#),
        ),
        (
            "turn count above range",
            raw_runtime_manifest("1", "30", "101", r#"{"type":"exact_text","expected":"4"}"#),
        ),
    ];

    for (name, manifest) in cases {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("prompt.txt"), "Only answer 4.").unwrap();
        fs::write(dir.path().join("manifest.json"), manifest).unwrap();
        assert!(PackLoader::load(dir.path()).is_err(), "accepted {name}");
    }
}

#[test]
fn runtime_parser_accepts_inclusive_task_range_endpoints() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("prompt.txt"), "Only answer 4.").unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        raw_runtime_manifest(
            "1",
            "7200",
            "100",
            r#"{"type":"exact_text","expected":"4"}"#,
        ),
    )
    .unwrap();

    assert!(PackLoader::load(dir.path()).is_ok());
}

#[test]
fn hash_covers_starter_and_verifier_files_not_only_prompts() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("starter/src")).unwrap();
    fs::write(dir.path().join("prompt.txt"), "Fix the function.").unwrap();
    fs::write(
        dir.path().join("starter/src/index.mjs"),
        "export const value = 1;",
    )
    .unwrap();
    fs::write(dir.path().join("verify.mjs"), "console.log('TASK_PASSED');").unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        r#"{
          "schema_version": 1,
          "id": "hash-pack",
          "version": "1.0.0",
          "title": "Hash Pack",
          "target_kinds": ["codex_cli"],
          "tasks": [{
            "id": "hash-1",
            "category": "cli_coding",
            "prompt_file": "prompt.txt",
            "starter_dir": "starter",
            "time_budget_secs": 60,
            "max_turns": 1,
            "grader": {"type":"external_verifier","verifier_id":"hash-v1"}
          }]
        }"#,
    )
    .unwrap();

    let before = PackLoader::load(dir.path()).unwrap().content_sha256;
    fs::write(
        dir.path().join("starter/src/index.mjs"),
        "export const value = 2;",
    )
    .unwrap();
    let after_starter = PackLoader::load(dir.path()).unwrap().content_sha256;
    assert_ne!(before, after_starter);

    fs::write(dir.path().join("verify.mjs"), "console.log('TASK_FAILED');").unwrap();
    let after_verifier = PackLoader::load(dir.path()).unwrap().content_sha256;
    assert_ne!(after_starter, after_verifier);
}

#[test]
fn embedded_registry_rejects_a_modified_bundled_pack() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("prompt.txt"), "Only answer 4.").unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        r#"{
          "schema_version":1,"id":"sealed-pack","version":"1.0.0",
          "title":"Sealed","target_kinds":["chat_gpt_client"],
          "tasks":[{"id":"one","category":"logic","prompt_file":"prompt.txt",
            "starter_dir":null,"time_budget_secs":60,"max_turns":1,
            "grader":{"type":"exact_text","expected":"4"}}]
        }"#,
    )
    .unwrap();
    let pack = PackLoader::load(dir.path()).unwrap();
    let registry = PackRegistry::parse(
        r#"{"schema_version":1,"packs":[{
          "id":"sealed-pack","version":"1.0.0","path":"sealed-pack",
          "license":"Apache-2.0","bundled":true,
          "content_sha256":"0000000000000000000000000000000000000000000000000000000000000000"
        }]}"#,
    )
    .unwrap();
    assert!(matches!(
        registry.verify_bundled(&pack),
        Err(PackError::HashMismatch { .. })
    ));
}

#[test]
fn rejects_prompt_path_traversal() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        r#"{
          "schema_version": 1,
          "id": "bad-pack",
          "version": "1.0.0",
          "title": "Bad Pack",
          "target_kinds": ["chat_gpt_client"],
          "tasks": [{
            "id": "bad-1",
            "category": "logic",
            "prompt_file": "../secret.txt",
            "starter_dir": null,
            "time_budget_secs": 60,
            "max_turns": 1,
            "grader": {"type":"exact_text","expected":"4"}
          }]
        }"#,
    )
    .unwrap();

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::UnsafePath(_))
    ));
}

#[test]
fn accepts_4096_pack_entries_and_rejects_the_4097th() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("prompt.txt"), "Only answer 4.").unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        r#"{
          "schema_version": 1,
          "id": "entry-limit-pack",
          "version": "1.0.0",
          "title": "Entry Limit Pack",
          "target_kinds": ["chat_gpt_client"],
          "tasks": [{
            "id": "entry-limit",
            "category": "logic",
            "prompt_file": "prompt.txt",
            "starter_dir": null,
            "time_budget_secs": 60,
            "max_turns": 1,
            "grader": {"type":"exact_text","expected":"4"}
          }]
        }"#,
    )
    .unwrap();

    // manifest.json and prompt.txt are the first two entries.
    for index in 0..4_094 {
        fs::create_dir(dir.path().join(format!("empty-{index:04}"))).unwrap();
    }
    assert!(PackLoader::load(dir.path()).is_ok());

    fs::create_dir(dir.path().join("empty-4094")).unwrap();

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::TooLarge(message)) if message == "entire pack entry count"
    ));
}

#[test]
fn hash_includes_the_manifest() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    let before = PackLoader::load(dir.path()).unwrap().content_sha256;

    let manifest_path = dir.path().join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["title"] = "Changed Security Pack".into();
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let after = PackLoader::load(dir.path()).unwrap().content_sha256;
    assert_ne!(before, after);
}

#[test]
fn hash_matches_the_stable_golden_vector_regardless_of_creation_order() {
    const MANIFEST: &str = r#"{"schema_version":1,"id":"golden-pack","version":"1.0.0","title":"Golden","target_kinds":["chat_gpt_client"],"tasks":[{"id":"golden-1","category":"logic","prompt_file":"prompt.txt","starter_dir":null,"time_budget_secs":60,"max_turns":1,"grader":{"type":"exact_text","expected":"4"}}]}"#;
    const EXPECTED: &str = "e316461c10ee9711875fadff3c6e2d0ca5af2aa3787dd8618494227752bfd6a5";

    let manifest_first = tempdir().unwrap();
    fs::write(manifest_first.path().join("manifest.json"), MANIFEST).unwrap();
    fs::write(manifest_first.path().join("prompt.txt"), "Only answer 4.").unwrap();

    let prompt_first = tempdir().unwrap();
    fs::write(prompt_first.path().join("prompt.txt"), "Only answer 4.").unwrap();
    fs::write(prompt_first.path().join("manifest.json"), MANIFEST).unwrap();

    let first_hash = PackLoader::load(manifest_first.path())
        .unwrap()
        .content_sha256;
    let second_hash = PackLoader::load(prompt_first.path())
        .unwrap()
        .content_sha256;
    assert_eq!(first_hash, EXPECTED);
    assert_eq!(second_hash, EXPECTED);
}

#[test]
fn rejects_windows_separator_path_traversal() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), r"..\secret.txt");

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::UnsafePath(_))
    ));
}

#[test]
fn rejects_unix_windows_and_unc_absolute_paths() {
    for unsafe_path in [
        "/tmp/secret.txt",
        r"C:\secret.txt",
        r"\\server\share\secret.txt",
    ] {
        let dir = tempdir().unwrap();
        write_minimal_pack(dir.path(), unsafe_path);
        assert!(
            matches!(PackLoader::load(dir.path()), Err(PackError::UnsafePath(_))),
            "accepted unsafe path {unsafe_path}"
        );
    }
}

#[test]
fn rejects_colon_paths_that_can_address_ntfs_alternate_data_streams() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt:hidden");
    fs::write(dir.path().join("prompt.txt:hidden"), "unhashed prompt").unwrap();

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::UnsafePath(_))
    ));
}

#[cfg(unix)]
#[test]
fn rejects_backslash_filenames_that_alias_portable_hash_paths() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    fs::write(dir.path().join(r"foo\bar"), "ambiguous").unwrap();

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::UnsafePath(_))
    ));
}

#[test]
fn rejects_unknown_manifest_task_and_grader_fields() {
    for pointer in ["", "/tasks/0", "/tasks/0/grader"] {
        let dir = tempdir().unwrap();
        write_minimal_pack(dir.path(), "prompt.txt");
        let manifest_path = dir.path().join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), true.into());
        fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        assert!(
            matches!(PackLoader::load(dir.path()), Err(PackError::InvalidJson(_))),
            "accepted unknown field at {pointer}"
        );
    }
}

#[test]
fn rejects_a_file_larger_than_two_mib() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    File::create(dir.path().join("oversized.bin"))
        .unwrap()
        .set_len(2 * 1024 * 1024 + 1)
        .unwrap();

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::TooLarge(path)) if path == "oversized.bin"
    ));
}

#[test]
fn manifest_size_limit_is_inclusive() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("prompt.txt"), "Only answer 4.").unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        manifest_bytes_with_size((256 * KIB) as usize),
    )
    .unwrap();
    assert!(PackLoader::load(dir.path()).is_ok());

    fs::write(
        dir.path().join("manifest.json"),
        manifest_bytes_with_size((256 * KIB + 1) as usize),
    )
    .unwrap();
    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::TooLarge(path)) if path == "manifest.json"
    ));
}

#[test]
fn prompt_size_limit_is_inclusive() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    fs::write(
        dir.path().join("prompt.txt"),
        vec![b'x'; (256 * KIB) as usize],
    )
    .unwrap();
    assert!(PackLoader::load(dir.path()).is_ok());

    fs::write(
        dir.path().join("prompt.txt"),
        vec![b'x'; (256 * KIB + 1) as usize],
    )
    .unwrap();
    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::TooLarge(path)) if path == "prompt.txt"
    ));
}

#[test]
fn ordinary_pack_file_size_limit_is_inclusive() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    let boundary_file = dir.path().join("boundary.bin");
    File::create(&boundary_file)
        .unwrap()
        .set_len(2 * MIB)
        .unwrap();
    assert!(PackLoader::load(dir.path()).is_ok());

    File::create(&boundary_file)
        .unwrap()
        .set_len(2 * MIB + 1)
        .unwrap();
    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::TooLarge(path)) if path == "boundary.bin"
    ));
}

#[test]
fn total_pack_size_limit_is_inclusive() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    let base_bytes = fs::metadata(dir.path().join("manifest.json"))
        .unwrap()
        .len()
        + fs::metadata(dir.path().join("prompt.txt")).unwrap().len();
    let mut remaining = 32 * MIB - base_bytes;
    let mut index = 0;
    while remaining > 0 {
        let chunk = remaining.min(2 * MIB);
        File::create(dir.path().join(format!("padding-{index:02}.bin")))
            .unwrap()
            .set_len(chunk)
            .unwrap();
        remaining -= chunk;
        index += 1;
    }
    assert!(PackLoader::load(dir.path()).is_ok());

    File::create(dir.path().join("one-byte-over.bin"))
        .unwrap()
        .set_len(1)
        .unwrap();
    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::TooLarge(message)) if message == "entire pack"
    ));
}

#[test]
fn rejects_more_than_thirty_two_mib_in_total() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    for index in 0..16 {
        File::create(dir.path().join(format!("padding-{index:02}.bin")))
            .unwrap()
            .set_len(2 * 1024 * 1024)
            .unwrap();
    }

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::TooLarge(message)) if message == "entire pack"
    ));
}

#[test]
fn bundled_registry_requires_matching_id_version_and_hash() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    let pack = PackLoader::load(dir.path()).unwrap();

    let registry_json = |id: &str, version: &str, hash: &str| {
        serde_json::json!({
            "schema_version": 1,
            "packs": [{
                "id": id,
                "version": version,
                "path": "security-pack",
                "license": "Apache-2.0",
                "bundled": true,
                "content_sha256": hash
            }]
        })
        .to_string()
    };

    let matching = PackRegistry::parse(&registry_json(
        "security-pack",
        "1.0.0",
        &pack.content_sha256,
    ))
    .unwrap();
    assert!(matching.verify_bundled(&pack).is_ok());

    for (id, version) in [("other-pack", "1.0.0"), ("security-pack", "2.0.0")] {
        let registry =
            PackRegistry::parse(&registry_json(id, version, &pack.content_sha256)).unwrap();
        assert!(matches!(
            registry.verify_bundled(&pack),
            Err(PackError::InvalidManifest(_))
        ));
    }
}

#[test]
fn rejects_runtime_version_with_unicode_decimal_digits() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    let manifest_path = dir.path().join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["version"] = "١.٢.٣".into();
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::InvalidManifest(_))
    ));
}

#[test]
fn public_schema_version_pattern_is_ascii() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../schemas/pack.schema.json")).unwrap();
    assert_eq!(
        schema.pointer("/properties/version/pattern").unwrap(),
        "^[0-9]+\\.[0-9]+\\.[0-9]+$"
    );
}

#[test]
fn rejects_runtime_duplicate_json_string_set_values() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    let manifest_path = dir.path().join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["tasks"][0]["grader"] = serde_json::json!({
        "type": "json_string_set",
        "expected": ["same", "same"]
    });
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::InvalidManifest(_))
    ));
}

#[test]
fn rejects_runtime_invalid_external_verifier_id() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    let manifest_path = dir.path().join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["tasks"][0]["grader"] = serde_json::json!({
        "type": "external_verifier",
        "verifier_id": "Invalid_ID"
    });
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::InvalidManifest(_))
    ));
}

#[test]
fn rejects_empty_prompt_and_starter_paths() {
    for (pointer, value) in [
        ("/tasks/0/prompt_file", serde_json::Value::String("".into())),
        ("/tasks/0/starter_dir", serde_json::Value::String("".into())),
    ] {
        let dir = tempdir().unwrap();
        write_minimal_pack(dir.path(), "prompt.txt");
        let manifest_path = dir.path().join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        *manifest.pointer_mut(pointer).unwrap() = value;
        fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(
            matches!(PackLoader::load(dir.path()), Err(PackError::UnsafePath(_))),
            "accepted empty path at {pointer}"
        );
    }
}

#[test]
fn rejects_a_matching_registry_entry_when_not_bundled() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    let pack = PackLoader::load(dir.path()).unwrap();
    let registry = PackRegistry::parse(
        &serde_json::json!({
            "schema_version": 1,
            "packs": [{
                "id": pack.manifest.id,
                "version": pack.manifest.version,
                "path": "security-pack",
                "license": "Apache-2.0",
                "bundled": false,
                "content_sha256": pack.content_sha256
            }]
        })
        .to_string(),
    )
    .unwrap();

    assert!(matches!(
        registry.verify_bundled(&pack),
        Err(PackError::InvalidManifest(_))
    ));
}

#[test]
fn loads_a_deep_pack_directory_without_recursive_traversal() {
    let dir = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    let mut current = dir.path().to_path_buf();
    for _ in 0..96 {
        current.push("d");
        fs::create_dir(&current).unwrap();
    }
    fs::write(current.join("leaf.txt"), "deep").unwrap();

    assert!(PackLoader::load(dir.path()).is_ok());
}

#[cfg(windows)]
#[test]
fn rejects_a_directory_junction() {
    use std::process::Command;

    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    write_minimal_pack(dir.path(), "prompt.txt");
    fs::write(outside.path().join("secret.txt"), "outside").unwrap();
    let junction = dir.path().join("linked");
    let status = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            junction.to_str().unwrap(),
            outside.path().to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::UnsafePath(path)) if path == "linked"
    ));
}

#[cfg(windows)]
#[test]
fn rejects_a_root_directory_junction() {
    use std::process::Command;

    let container = tempdir().unwrap();
    let target = container.path().join("target");
    fs::create_dir(&target).unwrap();
    write_minimal_pack(&target, "prompt.txt");
    let junction = container.path().join("root-junction");
    let status = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            junction.to_str().unwrap(),
            target.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    assert!(matches!(
        PackLoader::load(&junction),
        Err(PackError::UnsafePath(_))
    ));
}

#[cfg(unix)]
#[test]
fn rejects_a_root_directory_symlink() {
    use std::os::unix::fs::symlink;

    let container = tempdir().unwrap();
    let target = container.path().join("target");
    fs::create_dir(&target).unwrap();
    write_minimal_pack(&target, "prompt.txt");
    let root_link = container.path().join("root-link");
    symlink(&target, &root_link).unwrap();

    assert!(matches!(
        PackLoader::load(&root_link),
        Err(PackError::UnsafePath(_))
    ));
}

#[cfg(unix)]
#[test]
fn rejects_a_prompt_file_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    write_minimal_pack(dir.path(), "linked-prompt.txt");
    fs::write(outside.path().join("prompt.txt"), "outside").unwrap();
    symlink(
        outside.path().join("prompt.txt"),
        dir.path().join("linked-prompt.txt"),
    )
    .unwrap();

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::UnsafePath(_))
    ));
}
