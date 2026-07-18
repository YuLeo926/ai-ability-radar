use ability_core::{PackLoader, PackRegistry};
use std::path::PathBuf;

#[test]
fn every_bundled_pack_matches_the_committed_registry_hash() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmark-packs");
    let registry =
        PackRegistry::parse(&std::fs::read_to_string(root.join("registry.json")).unwrap()).unwrap();
    assert!(
        registry
            .packs
            .iter()
            .all(|entry| entry.content_sha256 != "0".repeat(64))
    );
    for directory in ["client-quick-v1", "cli-quick-v1"] {
        let pack = PackLoader::load(&root.join(directory)).unwrap();
        registry.verify_bundled(&pack).unwrap();
    }
}
