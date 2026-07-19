# v0.2 Windows Acceptance Matrix

This matrix separates required coverage from evidence. “Yes” means the release
requires that cell; it does not mean the cell passed. Manual evidence is valid
only from a clean VM and must record tester, date, app commit, exact OS build,
and a pass/fail evidence link. Code inspection is never a manual pass.

| Area | Windows 10 x64 | Windows 11 x64 | Automated | Release blocker |
|---|---:|---:|---:|---:|
| Install / uninstall NSIS | Yes | Yes | No | Yes |
| MSI installation | Yes | Yes | No | Yes |
| Client run without Node.js | Yes | Yes | Partial | Yes |
| Codex fake CLI 2/2 | Yes | Yes | Yes | Yes |
| Claude fake CLI 2/2 | Yes | Yes | Yes | Yes |
| Missing Node blocks before CLI call | Yes | Yes | Yes | Yes |
| Unsupported Node 20/26 blocks before CLI call | Yes | Yes | Yes | Yes |
| Cancel kills child process tree | Yes | Yes | Yes | Yes |
| Crash/restart resumes checkpoint | Yes | Yes | Yes | Yes |
| Clean and resumed history stay separate | Yes | Yes | Yes | Yes |
| Public report redaction | Yes | Yes | Yes | Yes |
| Full backup and retention | Yes | Yes | Yes | Yes |
| Light/dark, keyboard, 200% scale | Yes | Yes | Partial | Yes |
| GitHub Pages no tracker | N/A | N/A | Yes | Yes |
| macOS/Linux runtime | Deferred | Deferred | Compile optional | No for v0.2 |

## Automated evidence

The Task 23 implementation report records exact commands, exit codes, and test
counts for repository contracts, the locked Rust workspace, frontend tests and
build, the real coordinator with both fake adapters, cancellation supervision,
adversarial scans, and NSIS/MSI bundle attempts. Automated evidence does not
replace either clean-VM column.

## Clean-VM evidence ledger

No clean Windows 10 or Windows 11 VM was available during Task 23. Every
applicable manual cell therefore remains Pending.

| Area | Windows 10 x64 evidence | Windows 11 x64 evidence |
|---|---|---|
| Install / uninstall NSIS | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| MSI installation | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Client run without Node.js | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Codex fake CLI 2/2 | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Claude fake CLI 2/2 | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Missing Node blocks before CLI call | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Unsupported Node 20/26 blocks before CLI call | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Cancel kills child process tree | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Crash/restart resumes checkpoint | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Clean and resumed history stay separate | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Public report redaction | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Full backup and retention | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| Light/dark, keyboard, 200% scale | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable | Pending — tester/date/app commit/OS build/pass-fail link: not recorded; clean VM unavailable |
| GitHub Pages no tracker | N/A — platform-independent automated contract | N/A — platform-independent automated contract |
| macOS/Linux runtime | Deferred — outside v0.2 release scope | Deferred — outside v0.2 release scope |

## Local fake-CLI acceptance

These commands build the first-party fixture once, make two disposable copies,
prepend only that directory for the child test process, and then delete it.
They must be run in the Visual Studio Rust shell. Never substitute installed
provider CLIs.

```powershell
cargo build -p ability-radar-fake-cli --locked
$fakeBin = Join-Path $env:TEMP ("ability-radar-fake-bin-" + [guid]::NewGuid().ToString("N"))
$oldPath = $env:PATH
New-Item -ItemType Directory -Path $fakeBin | Out-Null
try {
  Copy-Item target/debug/ability-radar-fake-cli.exe (Join-Path $fakeBin "codex.exe")
  Copy-Item target/debug/ability-radar-fake-cli.exe (Join-Path $fakeBin "claude.exe")
  $env:PATH = "$fakeBin;$oldPath"
  $env:ABILITY_RADAR_FAKE_CLI_E2E = "1"
  cargo test -p ability-radar --test fake_cli_e2e --locked -- --ignored
} finally {
  Remove-Item Env:\ABILITY_RADAR_FAKE_CLI_E2E -ErrorAction SilentlyContinue
  $env:PATH = $oldPath
  Remove-Item -LiteralPath $fakeBin -Recurse -Force
}
```
