use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::ffi::OsStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchCommand {
    pub program: PathBuf,
    pub prefix_args: Vec<String>,
}

impl LaunchCommand {
    fn direct(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            prefix_args: Vec::new(),
        }
    }
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
        let inherited = effective_inherited_path
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is unavailable"))?;
        return resolve_windows_provider_command(program.to_str().unwrap(), inherited);
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
fn first_file(path: &OsStr, relative: &Path) -> Option<PathBuf> {
    path_directories(path)
        .map(|directory| directory.join(relative))
        .find(|candidate| candidate.is_file())
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

    canonical_reviewed_file(
        &package_root,
        &package_root.join(entry_relative),
        entry_relative,
    )
}

#[cfg(windows)]
fn resolve_windows_provider_command(provider: &str, path: &OsStr) -> io::Result<LaunchCommand> {
    if let Some(executable) = first_file(path, Path::new(&format!("{provider}.exe"))) {
        return Ok(LaunchCommand::direct(executable));
    }

    if !matches!(provider, "codex" | "claude") {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "unsupported provider",
        ));
    }

    // The effective inherited child PATH is the explicit local trust boundary:
    // only absolute entries are searched. Package metadata and canonical
    // containment narrow npm fallback to the reviewed entry; this is not a
    // cryptographic provenance or signature check.
    let script =
        path_directories(path).find_map(|directory| reviewed_package_entry(&directory, provider));
    let node = first_file(path, Path::new("node.exe"));
    match (node, script) {
        (Some(node), Some(script)) => Ok(LaunchCommand {
            program: node,
            prefix_args: vec![script.to_string_lossy().into_owned()],
        }),
        _ => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "supported provider executable was not found",
        )),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
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

        let launch = resolve_windows_provider_command("codex", &path).unwrap();

        assert_eq!(launch.program, node_bin.join("node.exe"));
        assert_eq!(
            launch.prefix_args,
            [
                std::fs::canonicalize(npm.join("node_modules/@openai/codex/bin/codex.js"))
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            ]
        );
    }

    #[test]
    fn native_exe_wins_without_executing_any_shim() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("npm");
        let second = temp.path().join("native");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("codex"), "not executable").unwrap();
        std::fs::write(first.join("codex.cmd"), "@echo off").unwrap();
        std::fs::write(second.join("codex.exe"), b"MZ").unwrap();
        let path = std::env::join_paths([&first, &second]).unwrap();

        let launch = resolve_windows_provider_command("codex", &path).unwrap();

        assert_eq!(launch.program, second.join("codex.exe"));
        assert!(launch.prefix_args.is_empty());
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

        let launch = resolve_windows_provider_command("claude", &path).unwrap();

        assert_eq!(launch.program, node_bin.join("node.exe"));
        assert_eq!(launch.prefix_args.len(), 1);
        assert!(launch.prefix_args[0].ends_with("cli.js"));
    }

    #[test]
    fn unreviewed_or_incomplete_shims_are_not_executed() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("codex.cmd"), "@echo calc").unwrap();
        let path = std::env::join_paths([temp.path()]).unwrap();

        let error = resolve_windows_provider_command("codex", &path).unwrap_err();

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

            let error = resolve_windows_provider_command("codex", &path).unwrap_err();

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

        let relative_error = resolve_windows_provider_command("codex", &relative_path).unwrap_err();
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
