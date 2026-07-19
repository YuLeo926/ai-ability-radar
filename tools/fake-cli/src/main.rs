use serde_json::json;
use std::env;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

const CODEX_PREFIX: &[&str] = &[
    "exec",
    "--ephemeral",
    "--json",
    "--sandbox",
    "workspace-write",
    "--ignore-user-config",
    "--ignore-rules",
];

const CLAUDE_SUFFIX: &[&str] = &[
    "--bare",
    "--no-session-persistence",
    "--output-format",
    "stream-json",
    "--max-turns",
    "20",
    "--tools",
    "Read,Edit,Write",
    "--allowedTools",
    "Read",
    "Edit",
    "Write",
    "--permission-mode",
    "dontAsk",
];

fn main() {
    if let Err(detail) = run() {
        eprintln!("{detail}");
        std::process::exit(64);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let invoked_as = env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|value| value.to_string_lossy().to_lowercase())
        })
        .unwrap_or_default();

    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--version"] => {
            println!("ability-radar-fake-cli 0.1.0");
            return Ok(());
        }
        ["login", "status"] if invoked_as == "codex" => {
            println!("Logged in using ChatGPT");
            return Ok(());
        }
        ["auth", "status"] if invoked_as == "claude" => {
            println!("{}", json!({"loggedIn": true}));
            return Ok(());
        }
        _ => {}
    }

    let invocation = if invoked_as == "codex" && valid_codex_execution(&args) {
        Invocation::Codex
    } else if invoked_as == "claude" && valid_claude_execution(&args) {
        Invocation::Claude
    } else {
        return Err("unsupported fake CLI invocation".into());
    };

    let workspace = env::current_dir().map_err(|error| format!("current directory: {error}"))?;
    let task = workspace
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "workspace has no UTF-8 task ID".to_owned())?;
    if !matches!(task, "dedupe-events" | "retry-schedule") {
        return Err(format!("unsupported fake CLI workspace task: {task}"));
    }

    if let Ok(pid_file) = env::var("ABILITY_RADAR_FAKE_PID_FILE") {
        fs::write(pid_file, std::process::id().to_string())
            .map_err(|error| format!("write fake PID marker: {error}"))?;
    }
    let delay = env::var("ABILITY_RADAR_FAKE_DELAY_MS").ok().or_else(|| {
        workspace.ancestors().find_map(|directory| {
            fs::read_to_string(directory.join(".ability-radar-fake-delay-ms")).ok()
        })
    });
    if let Some(milliseconds) = delay {
        let milliseconds = milliseconds
            .trim()
            .parse::<u64>()
            .map_err(|_| "fake delay must be an integer".to_owned())?;
        thread::sleep(Duration::from_millis(milliseconds));
    }

    match task {
        "dedupe-events" => write(
            &workspace.join("src/dedupeEvents.mjs"),
            r#"export function dedupeEvents(events) {
  const latest = new Map();
  events.forEach((event, index) => {
    if (!event || typeof event !== "object" || typeof event.id !== "string" ||
        event.id.length === 0 || Number.isNaN(Date.parse(event.occurredAt))) return;
    const previous = latest.get(event.id);
    const time = Date.parse(event.occurredAt);
    if (!previous || time >= previous.time) {
      latest.set(event.id, { time, index, event: structuredClone(event) });
    }
  });
  return [...latest.values()]
    .sort((a, b) => a.time - b.time || a.event.id.localeCompare(b.event.id))
    .map(({ event }) => event);
}"#,
        )?,
        "retry-schedule" => write(
            &workspace.join("src/retrySchedule.mjs"),
            r#"export function buildRetrySchedule({
  maxAttempts, baseDelayMs, maxDelayMs, retryAfterMs = [],
}) {
  const values = [maxAttempts, baseDelayMs, maxDelayMs, ...retryAfterMs];
  if (!values.every(Number.isInteger) || values.some((value) => value < 0) ||
      maxAttempts < 1 || baseDelayMs < 1 || maxDelayMs < baseDelayMs) {
    throw new TypeError("invalid options");
  }
  const result = [0];
  for (let retry = 1; retry < maxAttempts; retry += 1) {
    const base = Math.min(baseDelayMs * 2 ** (retry - 1), maxDelayMs);
    const delay = Math.max(base, retryAfterMs[retry - 1] ?? 0);
    result.push(result.at(-1) + delay);
  }
  return result;
}"#,
        )?,
        _ => unreachable!("task ID validated above"),
    }

    match invocation {
        Invocation::Claude => println!("{}", json!({"type": "result", "subtype": "success"})),
        Invocation::Codex => println!(
            "{}",
            json!({
                "type": "turn.completed",
                "usage": {"input_tokens": 0, "output_tokens": 0}
            })
        ),
    }
    Ok(())
}

enum Invocation {
    Codex,
    Claude,
}

fn valid_codex_execution(args: &[String]) -> bool {
    if args.len() <= CODEX_PREFIX.len()
        || !args
            .iter()
            .take(CODEX_PREFIX.len())
            .map(String::as_str)
            .eq(CODEX_PREFIX.iter().copied())
    {
        return false;
    }
    let mut tail = &args[CODEX_PREFIX.len()..];
    if tail.first().map(String::as_str) == Some("--model") {
        if tail.get(1).is_none_or(String::is_empty) {
            return false;
        }
        tail = &tail[2..];
    }
    if tail.first().map(String::as_str) == Some("--config") {
        let Some(config) = tail.get(1) else {
            return false;
        };
        if ![
            r#"model_reasoning_effort="low""#,
            r#"model_reasoning_effort="medium""#,
            r#"model_reasoning_effort="high""#,
        ]
        .contains(&config.as_str())
        {
            return false;
        }
        tail = &tail[2..];
    }
    matches!(tail, [prompt] if !prompt.is_empty())
}

fn valid_claude_execution(args: &[String]) -> bool {
    if args.len() < 2 + CLAUDE_SUFFIX.len()
        || args.first().map(String::as_str) != Some("-p")
        || args.get(1).is_none_or(String::is_empty)
        || !args[2..]
            .iter()
            .take(CLAUDE_SUFFIX.len())
            .map(String::as_str)
            .eq(CLAUDE_SUFFIX.iter().copied())
    {
        return false;
    }
    let mut tail = &args[2 + CLAUDE_SUFFIX.len()..];
    if tail.first().map(String::as_str) == Some("--model") {
        if tail.get(1).is_none_or(String::is_empty) {
            return false;
        }
        tail = &tail[2..];
    }
    if tail.first().map(String::as_str) == Some("--effort") {
        if !matches!(
            tail.get(1).map(String::as_str),
            Some("low" | "medium" | "high")
        ) {
            return false;
        }
        tail = &tail[2..];
    }
    tail.is_empty()
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|error| format!("write fixture solution: {error}"))
}
