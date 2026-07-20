use ability_core::{PackLoader, PackRegistry};
use std::env;
use std::fs;
use std::path::PathBuf;

fn validate() -> Result<(), String> {
    let mut arguments = env::args_os().skip(1);
    let packs_root = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "expected one benchmark-packs directory".to_owned())?;
    if arguments.next().is_some() {
        return Err("expected one benchmark-packs directory".into());
    }
    let packs_root = packs_root
        .canonicalize()
        .map_err(|_| "benchmark-packs directory is unavailable".to_owned())?;
    if !packs_root.is_dir() {
        return Err("benchmark-packs path is not a directory".into());
    }

    let registry_text = fs::read_to_string(packs_root.join("registry.json"))
        .map_err(|_| "registry is unavailable or not UTF-8".to_owned())?;
    let registry = PackRegistry::parse(&registry_text)
        .map_err(|_| "runtime registry parse failed".to_owned())?;
    for entry in &registry.packs {
        let pack_root = packs_root.join(&entry.path);
        let canonical_pack = pack_root
            .canonicalize()
            .map_err(|_| "registered pack directory is unavailable".to_owned())?;
        if !canonical_pack.starts_with(&packs_root) {
            return Err("registered pack path escapes benchmark-packs".into());
        }
        let pack =
            PackLoader::load(&pack_root).map_err(|_| "runtime pack load failed".to_owned())?;
        if pack.manifest.id != entry.id || pack.manifest.version != entry.version {
            return Err("runtime pack identity differs from registry".into());
        }
        registry
            .verify_bundled(&pack)
            .map_err(|_| "runtime pack seal verification failed".to_owned())?;
    }
    Ok(())
}

fn main() {
    if let Err(message) = validate() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
