# v0.2 Windows Preview Release Checklist

Unchecked items are release gates, not implementation claims. Clean Windows 10
and Windows 11 VM evidence is still Pending.

## Scope and truthfulness

- [ ] Exactly 8 client tasks and 2 CLI tasks are bundled.
- [ ] ChatGPT, Claude, Codex CLI, and Claude Code never share a score.
- [ ] No screen says IQ or claims certainty from insufficient evidence.
- [ ] Time and subscription-cost estimates appear before starting.
- [ ] Infrastructure invalidity and agent-budget failure remain distinct.

## Reproducibility

- [ ] All pack hashes match the release resources.
- [ ] Same fixture history produces byte-equivalent analysis JSON.
- [ ] Clean and resumed runs never share one history series.
- [ ] v0.2 never emits a degradation verdict from historical scores.

## Privacy and security

- [ ] No API key, login token, CLI auth file, or environment dump is collected.
- [ ] Tauri capability file has no shell, HTTP, filesystem, or SQL permission.
- [ ] Public report contains no raw answer, log, username, hostname, or path.
- [ ] Full backup is explicitly labeled unencrypted/private.
- [ ] Cancellation kills the Windows child process tree.
- [ ] Only signed bundled verifier IDs can execute.

## Quality

- [ ] cargo fmt, clippy, tests, npm tests, build, and axe pass.
- [ ] Windows 10 and 11 acceptance matrix is complete.
- [ ] NSIS and MSI install, launch, and uninstall.
- [ ] 100–200% scaling and keyboard-only operation pass.
- [ ] Offline client-only use works without Node.js.

## GitHub release

- [ ] Version matches tag and documentation.
- [ ] THIRD_PARTY_NOTICES is current.
- [ ] SHA256SUMS.txt matches every installer.
- [ ] Draft notes state unsigned preview / SmartScreen behavior.
- [ ] Updater remains disabled.
- [ ] Pages links point to the correct repository and release.
