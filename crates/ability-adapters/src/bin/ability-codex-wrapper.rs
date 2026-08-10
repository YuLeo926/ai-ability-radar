use std::ffi::OsString;
use std::process::{Command, ExitCode, Stdio};

const NODE_PROGRAM: &str = "ABILITY_RADAR_NODE_PROGRAM";
const CODEX_ENTRY: &str = "ABILITY_RADAR_CODEX_ENTRY";

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(()) => ExitCode::FAILURE,
    }
}

fn run() -> Result<u8, ()> {
    let node = std::env::var_os(NODE_PROGRAM).ok_or(())?;
    let entry = std::env::var_os(CODEX_ENTRY).ok_or(())?;
    let args = isolated_args(std::env::args_os().skip(1).collect())?;
    let status = Command::new(node)
        .arg(entry)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|_| ())?;
    Ok(status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1))
}

fn isolated_args(mut args: Vec<OsString>) -> Result<Vec<OsString>, ()> {
    if args.first().and_then(|value| value.to_str()) != Some("exec") {
        return Err(());
    }
    for required in ["--ephemeral", "--ignore-user-config", "--ignore-rules"] {
        if !args.iter().any(|value| value == required) {
            args.insert(1, required.into());
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapper_accepts_only_exec_and_inserts_every_isolation_flag_once() {
        let args = isolated_args(vec!["exec".into(), "--experimental-json".into()]).unwrap();
        for required in ["--ephemeral", "--ignore-user-config", "--ignore-rules"] {
            assert_eq!(args.iter().filter(|value| *value == required).count(), 1);
        }
        assert!(isolated_args(vec!["login".into(), "status".into()]).is_err());
    }
}
