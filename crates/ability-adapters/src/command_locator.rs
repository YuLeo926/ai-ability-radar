use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

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

pub(crate) fn resolve_launch_command(program: &str) -> io::Result<LaunchCommand> {
    let path = Path::new(program);
    if path.is_absolute() || path.components().count() > 1 {
        return Ok(LaunchCommand::direct(path));
    }

    #[cfg(windows)]
    if matches!(program, "codex" | "claude") {
        let inherited = std::env::var_os("PATH")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is unavailable"))?;
        return resolve_windows_provider_command(program, &inherited);
    }

    Ok(LaunchCommand::direct(program))
}

#[cfg(windows)]
fn path_directories(path: &OsStr) -> impl Iterator<Item = PathBuf> + '_ {
    std::env::split_paths(path)
}

#[cfg(windows)]
fn first_file(path: &OsStr, relative: &Path) -> Option<PathBuf> {
    path_directories(path)
        .map(|directory| directory.join(relative))
        .find(|candidate| candidate.is_file())
}

#[cfg(windows)]
fn resolve_windows_provider_command(provider: &str, path: &OsStr) -> io::Result<LaunchCommand> {
    if let Some(executable) = first_file(path, Path::new(&format!("{provider}.exe"))) {
        return Ok(LaunchCommand::direct(executable));
    }

    let (package_entry, shim_name) = match provider {
        "codex" => (
            Path::new("node_modules/@openai/codex/bin/codex.js"),
            "codex.cmd",
        ),
        "claude" => (
            Path::new("node_modules/@anthropic-ai/claude-code/cli.js"),
            "claude.cmd",
        ),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "unsupported provider",
            ));
        }
    };

    let script = path_directories(path).find_map(|directory| {
        let shim = directory.join(shim_name);
        let entry = directory.join(package_entry);
        (shim.is_file() && entry.is_file()).then_some(entry)
    });
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
        std::fs::write(node_bin.join("node.exe"), b"MZ").unwrap();
        let path = std::env::join_paths([&npm, &node_bin]).unwrap();

        let launch = resolve_windows_provider_command("codex", &path).unwrap();

        assert_eq!(launch.program, node_bin.join("node.exe"));
        assert_eq!(
            launch.prefix_args,
            [npm.join("node_modules/@openai/codex/bin/codex.js")
                .to_string_lossy()
                .into_owned()]
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
}
