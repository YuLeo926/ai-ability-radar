use ability_core::FailureKind;

pub fn classify_cli_failure(text: &str) -> FailureKind {
    let lower = text.to_lowercase();
    if lower.contains("not logged in")
        || lower.contains("authentication")
        || lower.contains("unauthorized")
    {
        FailureKind::AuthExpired
    } else if lower.contains("quota")
        || lower.contains("usage limit")
        || lower.contains("rate limit")
    {
        FailureKind::QuotaExhausted
    } else if lower.contains("network") || lower.contains("connection") || lower.contains("dns") {
        FailureKind::Network
    } else {
        FailureKind::AppInterrupted
    }
}

pub fn is_agent_budget_exhaustion(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "max_turns",
        "max turns",
        "maximum number of turns",
        "turn limit reached",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}
