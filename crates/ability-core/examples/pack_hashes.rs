use ability_core::PackLoader;
use serde_json::Value;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmark-packs");
    let registry_path = root.join("registry.json");
    let mut registry: Value =
        serde_json::from_str(&std::fs::read_to_string(&registry_path).unwrap()).unwrap();
    for directory in ["client-quick-v1", "cli-quick-v1"] {
        let pack = PackLoader::load(&root.join(directory)).unwrap();
        let entry = registry["packs"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["id"] == pack.manifest.id)
            .unwrap();
        entry["content_sha256"] = Value::String(pack.content_sha256.clone());
        println!(
            "{} {} {}",
            pack.manifest.id, pack.manifest.version, pack.content_sha256
        );
    }
    std::fs::write(
        registry_path,
        format!("{}\n", serde_json::to_string_pretty(&registry).unwrap()),
    )
    .unwrap();
}
