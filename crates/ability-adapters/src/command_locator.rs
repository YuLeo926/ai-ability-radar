use crate::LaunchSource;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::ffi::OsStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchCommand {
    pub program: PathBuf,
    pub prefix_args: Vec<String>,
    pub source: LaunchSource,
}

impl LaunchCommand {
    fn direct(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            prefix_args: Vec::new(),
            source: LaunchSource::NativeExe,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchDiscovery {
    pub candidates: Vec<LaunchCommand>,
    pub reviewed_npm_without_node: bool,
}

pub(crate) fn resolve_launch_command(
    program: &Path,
    effective_inherited_path: Option<&std::ffi::OsStr>,
) -> io::Result<LaunchCommand> {
    if program.is_absolute() || program.components().count() > 1 {
        return Ok(LaunchCommand::direct(program));
    }

    #[cfg(windows)]
    if matches!(program.to_str(), Some("codex" | "claude")) {
        return discover_provider_commands(program.to_str().unwrap(), effective_inherited_path)
            .and_then(|discovery| {
                discovery.candidates.into_iter().next().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "supported provider executable was not found",
                    )
                })
            });
    }

    #[cfg(not(windows))]
    let _ = effective_inherited_path;

    Ok(LaunchCommand::direct(program))
}

#[cfg(windows)]
fn path_directories(path: &OsStr) -> impl Iterator<Item = PathBuf> + '_ {
    std::env::split_paths(path)
        .filter(|directory| !directory.as_os_str().is_empty() && directory.is_absolute())
}

#[cfg(windows)]
fn canonical_reviewed_file(
    package_root: &Path,
    candidate: &Path,
    expected_relative: &Path,
) -> Option<PathBuf> {
    let canonical_root = std::fs::canonicalize(package_root).ok()?;
    let canonical_candidate = std::fs::canonicalize(candidate).ok()?;
    let actual_relative = canonical_candidate.strip_prefix(&canonical_root).ok()?;
    (actual_relative == expected_relative && canonical_candidate.is_file())
        .then_some(canonical_candidate)
}

#[cfg(windows)]
fn reviewed_package_entry(directory: &Path, provider: &str) -> Option<PathBuf> {
    let (package_relative, package_name, bin_name, entry_relative, shim_name) = match provider {
        "codex" => (
            Path::new("node_modules/@openai/codex"),
            "@openai/codex",
            "codex",
            Path::new("bin/codex.js"),
            "codex.cmd",
        ),
        "claude" => (
            Path::new("node_modules/@anthropic-ai/claude-code"),
            "@anthropic-ai/claude-code",
            "claude",
            Path::new("cli.js"),
            "claude.cmd",
        ),
        _ => return None,
    };

    if !directory.join(shim_name).is_file() {
        return None;
    }

    let package_root = directory.join(package_relative);
    let metadata_path = canonical_reviewed_file(
        &package_root,
        &package_root.join("package.json"),
        Path::new("package.json"),
    )?;
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(metadata_path).ok()?).ok()?;
    if metadata.get("name").and_then(serde_json::Value::as_str) != Some(package_name) {
        return None;
    }
    let bin = metadata.get("bin").and_then(serde_json::Value::as_object)?;
    if bin.len() != 1
        || bin.get(bin_name).and_then(serde_json::Value::as_str) != Some(entry_relative.to_str()?)
    {
        return None;
    }

    let entry = package_root.join(entry_relative);
    canonical_reviewed_file(&package_root, &entry, entry_relative)?;
    Some(entry)
}

#[cfg(windows)]
pub(crate) fn discover_provider_commands(
    provider: &str,
    inherited_path: Option<&OsStr>,
) -> io::Result<LaunchDiscovery> {
    let inherited = inherited_path
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is unavailable"))?;
    let directories = path_directories(inherited).collect::<Vec<_>>();
    let node = directories
        .iter()
        .map(|directory| directory.join("node.exe"))
        .find(|candidate| candidate.is_file());
    let mut candidates = Vec::new();
    let mut reviewed_npm_without_node = false;

    // The effective inherited child PATH is the explicit local trust boundary:
    // only absolute entries are searched. Package metadata and canonical
    // containment narrow npm fallback to the reviewed entry; this is not a
    // cryptographic provenance or signature check.
    for directory in directories {
        let native = directory.join(format!("{provider}.exe"));
        if native.is_file() {
            candidates.push(LaunchCommand {
                program: native,
                prefix_args: Vec::new(),
                source: LaunchSource::NativeExe,
            });
        }

        if let Some(script) = reviewed_package_entry(&directory, provider) {
            if let Some(node) = node.as_ref() {
                candidates.push(LaunchCommand {
                    program: node.clone(),
                    prefix_args: vec![script.to_string_lossy().into_owned()],
                    source: LaunchSource::ReviewedNpm,
                });
            } else {
                reviewed_npm_without_node = true;
            }
        }
    }

    let mut seen = std::collections::HashSet::new();
    candidates.retain(|candidate| {
        seen.insert((candidate.program.clone(), candidate.prefix_args.clone()))
    });
    Ok(LaunchDiscovery {
        candidates,
        reviewed_npm_without_node,
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::LaunchSource;
    use std::io;

    fn write_package_metadata(package_root: &Path, name: &str, bin_name: &str, bin_entry: &str) {
        std::fs::write(
            package_root.join("package.json"),
            serde_json::json!({
                "name": name,
                "bin": {
                    (bin_name): bin_entry,
                },
            })
            .to_string(),
        )
        .unwrap();
    }

    fn write_executable(path: &Path) {
        std::fs::write(path, b"MZ").unwrap();
    }

    #[test]
    fn npm_extensionless_shim_does_not_hide_the_reviewed_codex_package() {
        let temp = tempfile::tempdir().unwrap();
        let npm = temp.path().join("npm");
        let node_bin = temp.path().join("node-bin");
        std::fs::create_dir_all(npm.join("node_modules/@openai/codex/bin")).unwrap();
        std::fs::create_dir_all(&node_bin).unwrap();
        std::fs::write(npm.join("codex"), "#!/bin/sh").unwrap();
        std::fs::write(npm.join("codex.cmd"), "@echo off").unwrap();
        std::fs::write(
            npm.join("node_modules/@openai/codex/bin/codex.js"),
            "console.log('fake')",
        )
        .unwrap();
        write_package_metadata(
            &npm.join("node_modules/@openai/codex"),
            "@openai/codex",
            "codex",
            "bin/codex.js",
        );
        std::fs::write(node_bin.join("node.exe"), b"MZ").unwrap();
        let path = std::env::join_paths([&npm, &node_bin]).unwrap();

        let launch = resolve_launch_command(Path::new("codex"), Some(&path)).unwrap();

        assert_eq!(launch.program, node_bin.join("node.exe"));
        assert_eq!(
            launch.prefix_args,
            [npm.join("node_modules/@openai/codex")
                .join("bin/codex.js")
                .to_string_lossy()
                .into_owned()]
        );
        assert!(!launch.prefix_args[0].starts_with(r"\\?\"));
    }

    #[test]
    fn earlier_reviewed_npm_precedes_later_native_exe() {
        let temp = tempfile::tempdir().unwrap();
        let npm = temp.path().join("npm");
        let node_bin = temp.path().join("node");
        let later_native = temp.path().join("windows-app");
        std::fs::create_dir_all(npm.join("node_modules/@openai/codex/bin")).unwrap();
        std::fs::create_dir_all(&node_bin).unwrap();
        std::fs::create_dir_all(&later_native).unwrap();
        std::fs::write(npm.join("codex.cmd"), "@echo off").unwrap();
        std::fs::write(
            npm.join("node_modules/@openai/codex/bin/codex.js"),
            "console.log('fake')",
        )
        .unwrap();
        write_package_metadata(
            &npm.join("node_modules/@openai/codex"),
            "@openai/codex",
            "codex",
            "bin/codex.js",
        );
        write_executable(&node_bin.join("node.exe"));
        write_executable(&later_native.join("codex.exe"));
        let path = std::env::join_paths([&npm, &node_bin, &later_native]).unwrap();

        let discovery = discover_provider_commands("codex", Some(&path)).unwrap();

        assert_eq!(discovery.candidates.len(), 2);
        assert_eq!(discovery.candidates[0].source, LaunchSource::ReviewedNpm);
        assert_eq!(discovery.candidates[1].source, LaunchSource::NativeExe);
    }

    #[test]
    fn duplicate_path_directory_keeps_only_first_native_and_reviewed_npm_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("provider-bin");
        std::fs::create_dir_all(directory.join("node_modules/@openai/codex/bin")).unwrap();
        std::fs::write(directory.join("codex.cmd"), "@echo off").unwrap();
        std::fs::write(
            directory.join("node_modules/@openai/codex/bin/codex.js"),
            "console.log('fake')",
        )
        .unwrap();
        write_package_metadata(
            &directory.join("node_modules/@openai/codex"),
            "@openai/codex",
            "codex",
            "bin/codex.js",
        );
        write_executable(&directory.join("codex.exe"));
        write_executable(&directory.join("node.exe"));
        let path = std::env::join_paths([&directory, &directory]).unwrap();

        let discovery = discover_provider_commands("codex", Some(&path)).unwrap();

        assert_eq!(discovery.candidates.len(), 2);
        assert_eq!(discovery.candidates[0].source, LaunchSource::NativeExe);
        assert_eq!(discovery.candidates[1].source, LaunchSource::ReviewedNpm);
    }

    #[test]
    fn reviewed_npm_without_node_is_reported_without_a_launch_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let npm = temp.path().join("npm");
        std::fs::create_dir_all(npm.join("node_modules/@openai/codex/bin")).unwrap();
        std::fs::write(npm.join("codex.cmd"), "@echo off").unwrap();
        std::fs::write(
            npm.join("node_modules/@openai/codex/bin/codex.js"),
            "console.log('fake')",
        )
        .unwrap();
        write_package_metadata(
            &npm.join("node_modules/@openai/codex"),
            "@openai/codex",
            "codex",
            "bin/codex.js",
        );
        let path = std::env::join_paths([&npm]).unwrap();

        let discovery = discover_provider_commands("codex", Some(&path)).unwrap();

        assert!(discovery.candidates.is_empty());
        assert!(discovery.reviewed_npm_without_node);
    }

    #[test]
    fn claude_uses_only_the_reviewed_npm_entry() {
        let temp = tempfile::tempdir().unwrap();
        let npm = temp.path().join("npm");
        let node_bin = temp.path().join("node");
        std::fs::create_dir_all(npm.join("node_modules/@anthropic-ai/claude-code")).unwrap();
        std::fs::create_dir_all(&node_bin).unwrap();
        std::fs::write(npm.join("claude.cmd"), "@echo off").unwrap();
        std::fs::write(
            npm.join("node_modules/@anthropic-ai/claude-code/cli.js"),
            "console.log('fake')",
        )
        .unwrap();
        write_package_metadata(
            &npm.join("node_modules/@anthropic-ai/claude-code"),
            "@anthropic-ai/claude-code",
            "claude",
            "cli.js",
        );
        std::fs::write(node_bin.join("node.exe"), b"MZ").unwrap();
        let path = std::env::join_paths([&npm, &node_bin]).unwrap();

        let launch = resolve_launch_command(Path::new("claude"), Some(&path)).unwrap();

        assert_eq!(launch.program, node_bin.join("node.exe"));
        assert_eq!(launch.prefix_args.len(), 1);
        assert!(launch.prefix_args[0].ends_with("cli.js"));
    }

    #[test]
    fn unreviewed_or_incomplete_shims_are_not_executed() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("codex.cmd"), "@echo calc").unwrap();
        let path = std::env::join_paths([temp.path()]).unwrap();

        let error = resolve_launch_command(Path::new("codex"), Some(&path)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn provider_package_metadata_must_match_the_reviewed_identity_and_bin() {
        let invalid_metadata = [
            None,
            Some("{"),
            Some(r#"{"name":"not-openai/codex","bin":{"codex":"bin/codex.js"}}"#),
            Some(r#"{"name":"@openai/codex","bin":{"codex":"other.js"}}"#),
            Some(r#"{"name":"@openai/codex","bin":{"other":"bin/codex.js"}}"#),
        ];

        for metadata in invalid_metadata {
            let temp = tempfile::tempdir().unwrap();
            let npm = temp.path().join("npm");
            let node_bin = temp.path().join("node");
            let package_root = npm.join("node_modules/@openai/codex");
            std::fs::create_dir_all(package_root.join("bin")).unwrap();
            std::fs::create_dir_all(&node_bin).unwrap();
            std::fs::write(npm.join("codex.cmd"), "@echo off").unwrap();
            std::fs::write(package_root.join("bin/codex.js"), "fake").unwrap();
            if let Some(metadata) = metadata {
                std::fs::write(package_root.join("package.json"), metadata).unwrap();
            }
            std::fs::write(node_bin.join("node.exe"), b"MZ").unwrap();
            let path = std::env::join_paths([&npm, &node_bin]).unwrap();

            let error = resolve_launch_command(Path::new("codex"), Some(&path)).unwrap_err();

            assert_eq!(
                error.kind(),
                io::ErrorKind::NotFound,
                "metadata unexpectedly accepted: {metadata:?}"
            );
        }
    }

    #[test]
    fn provider_search_rejects_relative_and_empty_path_entries() {
        let current = std::env::current_dir().unwrap();
        let relative_directory = tempfile::Builder::new()
            .prefix("relative-provider-")
            .tempdir_in(&current)
            .unwrap();
        std::fs::write(relative_directory.path().join("codex.exe"), b"MZ").unwrap();
        let relative = relative_directory.path().strip_prefix(&current).unwrap();
        let relative_path = std::env::join_paths([relative]).unwrap();

        let relative_error =
            resolve_launch_command(Path::new("codex"), Some(&relative_path)).unwrap_err();
        assert_eq!(relative_error.kind(), io::ErrorKind::NotFound);

        let directories = path_directories(OsStr::new("")).collect::<Vec<_>>();
        assert!(directories.is_empty());
    }

    #[test]
    fn provider_entry_must_remain_inside_the_canonical_package_root() {
        let temp = tempfile::tempdir().unwrap();
        let package_root = temp.path().join("node_modules/@openai/codex");
        let external_entry = temp.path().join("external-codex.js");
        std::fs::create_dir_all(&package_root).unwrap();
        std::fs::write(&external_entry, "fake").unwrap();
        let accepted =
            canonical_reviewed_file(&package_root, &external_entry, Path::new("bin/codex.js"));

        assert!(accepted.is_none());
    }
}
