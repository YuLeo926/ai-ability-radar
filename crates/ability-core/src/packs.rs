use crate::{Category, TargetKind};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_PROMPT_BYTES: u64 = 256 * 1024;
const MAX_PACK_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PACK_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PACK_ENTRIES: usize = 4_096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub title: String,
    pub target_kinds: Vec<TargetKind>,
    pub tasks: Vec<TaskDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDefinition {
    pub id: String,
    pub category: Category,
    pub prompt_file: String,
    pub starter_dir: Option<String>,
    pub time_budget_secs: u64,
    pub max_turns: u32,
    pub grader: GraderSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GraderSpec {
    ExactText { expected: String },
    ExactJson { expected: Value },
    JsonStringSet { expected: Vec<String> },
    ExternalVerifier { verifier_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackRegistry {
    pub schema_version: u32,
    pub packs: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    pub id: String,
    pub version: String,
    pub path: String,
    pub license: String,
    pub bundled: bool,
    pub content_sha256: String,
}

impl PackRegistry {
    pub fn parse(json: &str) -> Result<Self, PackError> {
        let registry: Self = serde_json::from_str(json)?;
        if registry.schema_version != 1 {
            return Err(PackError::InvalidManifest(
                "unsupported registry schema".into(),
            ));
        }
        Ok(registry)
    }

    pub fn verify_bundled(&self, pack: &LoadedPack) -> Result<(), PackError> {
        let entry = self
            .packs
            .iter()
            .find(|entry| {
                entry.id == pack.manifest.id
                    && entry.version == pack.manifest.version
                    && entry.bundled
            })
            .ok_or_else(|| {
                PackError::InvalidManifest(format!(
                    "untrusted bundled pack {} {}",
                    pack.manifest.id, pack.manifest.version
                ))
            })?;
        if entry.content_sha256 != pack.content_sha256 {
            return Err(PackError::HashMismatch {
                expected: entry.content_sha256.clone(),
                actual: pack.content_sha256.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LoadedTask {
    pub definition: TaskDefinition,
    pub prompt: String,
    pub pack_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct LoadedPack {
    pub manifest: PackManifest,
    pub tasks: Vec<LoadedTask>,
    pub content_sha256: String,
}

#[derive(Debug, Error)]
pub enum PackError {
    #[error("pack file is missing: {0}")]
    Missing(String),
    #[error("pack path is unsafe: {0}")]
    UnsafePath(String),
    #[error("pack file exceeds size limit: {0}")]
    TooLarge(String),
    #[error("pack id is invalid: {0}")]
    InvalidId(String),
    #[error("pack manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("pack hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("pack text is not UTF-8: {0}")]
    InvalidUtf8(String),
    #[error("pack JSON is invalid: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("pack I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub struct PackLoader;

impl PackLoader {
    pub fn load(root: &Path) -> Result<LoadedPack, PackError> {
        reject_link_or_reparse_point(root, root)?;
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(PackError::Missing("pack root".into()));
        }

        let manifest_path = root.join("manifest.json");
        reject_link_or_reparse_point(&root, &manifest_path)?;
        let metadata =
            fs::metadata(&manifest_path).map_err(|_| PackError::Missing("manifest.json".into()))?;
        if !metadata.is_file() {
            return Err(PackError::Missing("manifest.json".into()));
        }
        if metadata.len() > MAX_MANIFEST_BYTES {
            return Err(PackError::TooLarge("manifest.json".into()));
        }

        let manifest_bytes = fs::read(&manifest_path)?;
        let manifest: PackManifest = serde_json::from_slice(&manifest_bytes)?;
        let id_re = Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$").unwrap();
        let version_re = Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+$").unwrap();
        let verifier_id_re = Regex::new(r"^[a-z0-9-]+$").unwrap();
        if !id_re.is_match(&manifest.id) {
            return Err(PackError::InvalidId(manifest.id));
        }
        if manifest.schema_version != 1
            || !version_re.is_match(&manifest.version)
            || manifest.title.trim().is_empty()
            || manifest.target_kinds.is_empty()
            || manifest.tasks.is_empty()
        {
            return Err(PackError::InvalidManifest(manifest.id));
        }

        let content_sha256 = hash_directory(&root)?;
        let mut tasks = Vec::with_capacity(manifest.tasks.len());
        let mut task_ids = BTreeSet::new();

        for definition in &manifest.tasks {
            if !id_re.is_match(&definition.id) || !task_ids.insert(definition.id.clone()) {
                return Err(PackError::InvalidId(definition.id.clone()));
            }
            if definition.time_budget_secs == 0
                || definition.time_budget_secs > 7_200
                || definition.max_turns == 0
                || definition.max_turns > 100
            {
                return Err(PackError::InvalidManifest(definition.id.clone()));
            }
            match &definition.grader {
                GraderSpec::JsonStringSet { expected }
                    if expected.iter().collect::<BTreeSet<_>>().len() != expected.len() =>
                {
                    return Err(PackError::InvalidManifest(definition.id.clone()));
                }
                GraderSpec::ExternalVerifier { verifier_id }
                    if !verifier_id_re.is_match(verifier_id) =>
                {
                    return Err(PackError::InvalidManifest(definition.id.clone()));
                }
                _ => {}
            }

            let prompt_path = safe_child(&root, &definition.prompt_file)?;
            let prompt_metadata = fs::metadata(&prompt_path)
                .map_err(|_| PackError::Missing(definition.prompt_file.clone()))?;
            if !prompt_metadata.is_file() {
                return Err(PackError::Missing(definition.prompt_file.clone()));
            }
            if prompt_metadata.len() > MAX_PROMPT_BYTES {
                return Err(PackError::TooLarge(definition.prompt_file.clone()));
            }
            let prompt_bytes = fs::read(&prompt_path)?;
            let prompt = String::from_utf8(prompt_bytes)
                .map_err(|_| PackError::InvalidUtf8(definition.prompt_file.clone()))?;

            if let Some(starter_dir) = &definition.starter_dir {
                let starter_path = safe_child(&root, starter_dir)?;
                if !starter_path.is_dir() {
                    return Err(PackError::Missing(starter_dir.clone()));
                }
            }

            tasks.push(LoadedTask {
                definition: definition.clone(),
                prompt,
                pack_root: root.clone(),
            });
        }

        Ok(LoadedPack {
            manifest,
            tasks,
            content_sha256,
        })
    }
}

fn hash_directory(root: &Path) -> Result<String, PackError> {
    let mut files = Vec::<(String, PathBuf, u64)>::new();
    collect_pack_files(root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let total_bytes = files
        .iter()
        .try_fold(0_u64, |total, (_, _, size)| total.checked_add(*size))
        .ok_or_else(|| PackError::TooLarge("entire pack".into()))?;
    if total_bytes > MAX_PACK_BYTES {
        return Err(PackError::TooLarge("entire pack".into()));
    }

    let mut digest = Sha256::new();
    for (relative, path, size) in files {
        let relative = relative.as_bytes();
        digest.update((relative.len() as u64).to_le_bytes());
        digest.update(relative);
        digest.update(size.to_le_bytes());
        digest.update(fs::read(path)?);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn collect_pack_files(
    root: &Path,
    files: &mut Vec<(String, PathBuf, u64)>,
) -> Result<(), PackError> {
    let mut pending_directories = vec![root.to_path_buf()];
    let mut entry_count = 0_usize;

    while let Some(current) = pending_directories.pop() {
        for entry in fs::read_dir(current)? {
            let path = entry?.path();
            let relative = portable_relative_path(root, &path)?;
            let metadata = fs::symlink_metadata(&path)?;
            if is_link_or_reparse_point(&metadata) {
                return Err(PackError::UnsafePath(relative));
            }

            entry_count = entry_count
                .checked_add(1)
                .ok_or_else(|| PackError::TooLarge("entire pack entry count".into()))?;
            if entry_count > MAX_PACK_ENTRIES {
                return Err(PackError::TooLarge("entire pack entry count".into()));
            }

            if metadata.is_dir() {
                pending_directories.push(path);
            } else if metadata.is_file() {
                if metadata.len() > MAX_PACK_FILE_BYTES {
                    return Err(PackError::TooLarge(relative));
                }
                files.push((relative, path, metadata.len()));
            } else {
                return Err(PackError::UnsafePath(relative));
            }
        }
    }

    Ok(())
}

fn safe_child(root: &Path, relative: &str) -> Result<PathBuf, PackError> {
    if !is_safe_relative_path(relative) {
        return Err(PackError::UnsafePath(relative.into()));
    }

    let joined = root.join(relative);
    let canonical = joined
        .canonicalize()
        .map_err(|_| PackError::Missing(relative.into()))?;
    if !canonical.starts_with(root) {
        return Err(PackError::UnsafePath(relative.into()));
    }

    reject_path_chain(root, &joined, relative)?;
    Ok(canonical)
}

fn is_safe_relative_path(relative: &str) -> bool {
    if relative.is_empty()
        || relative.starts_with(['/', '\\'])
        || relative.contains(':')
        || relative.split(['/', '\\']).any(|part| part == "..")
    {
        return false;
    }

    let path = Path::new(relative);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn reject_path_chain(root: &Path, joined: &Path, display: &str) -> Result<(), PackError> {
    let relative = joined
        .strip_prefix(root)
        .map_err(|_| PackError::UnsafePath(display.into()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        reject_link_or_reparse_point(root, &current)?;
    }
    Ok(())
}

fn reject_link_or_reparse_point(root: &Path, path: &Path) -> Result<(), PackError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PackError::Missing(path.display().to_string())
        } else {
            PackError::Io(error)
        }
    })?;
    if is_link_or_reparse_point(&metadata) {
        let display = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        return Err(PackError::UnsafePath(display));
    }
    Ok(())
}

#[cfg(windows)]
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn portable_relative_path(root: &Path, path: &Path) -> Result<String, PackError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| PackError::UnsafePath(path.display().to_string()))?
        .to_str()
        .ok_or_else(|| PackError::UnsafePath(path.display().to_string()))?;

    #[cfg(not(windows))]
    if relative.contains(['\\', ':']) {
        return Err(PackError::UnsafePath(relative.into()));
    }

    Ok(relative.replace('\\', "/"))
}
