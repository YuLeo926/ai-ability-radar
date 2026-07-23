use ability_core::{
    contains_forbidden_display_character, is_valid_reported_model, ModelSource, TargetKind,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // Terminal scan outcomes are constructed by the bounded scanner task.
pub(crate) enum ClientSelectionStatus {
    Detected,
    Multiple,
    NotRunning,
    NotExposed,
    Unsupported,
    TimedOut,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClientSurface {
    #[serde(rename = "chatgpt")]
    ChatGpt,
    CodexDesktop,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClientSelectionConfidence {
    VisibleSelector,
    BestEffort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientSelectionCandidate {
    pub(crate) model: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) surface: ClientSurface,
    pub(crate) source: ModelSource,
    pub(crate) confidence: ClientSelectionConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientSelectionDetection {
    pub(crate) status: ClientSelectionStatus,
    pub(crate) candidates: Vec<ClientSelectionCandidate>,
}

impl ClientSelectionDetection {
    pub(crate) fn failed(status: ClientSelectionStatus) -> Self {
        Self {
            status,
            candidates: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderFamily {
    OpenAi,
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlKind {
    Button,
    ComboBox,
    MenuItem,
    Document,
    Edit,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawControl {
    pub(crate) kind: ControlKind,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedControl {
    pub(crate) surface: ClientSurface,
    pub(crate) kind: ControlKind,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowIdentity {
    pub(crate) process_name: String,
    pub(crate) package_family: Option<String>,
    pub(crate) title_hint: String,
    pub(crate) executable_path: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct ProviderFingerprint {
    provider: ProviderFamily,
    package_name: &'static str,
    package_name_match: PackageNameMatch,
    publisher_id: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
enum PackageNameMatch {
    Exact,
    Prefix,
}

const PROVIDER_FINGERPRINTS_V1: &[ProviderFingerprint] = &[
    ProviderFingerprint {
        provider: ProviderFamily::OpenAi,
        package_name: "OpenAI.",
        package_name_match: PackageNameMatch::Prefix,
        publisher_id: Some("2p2nqsd0c76g0"),
    },
    ProviderFingerprint {
        provider: ProviderFamily::Anthropic,
        package_name: "Anthropic.Claude",
        package_name_match: PackageNameMatch::Exact,
        publisher_id: None,
    },
];

pub(crate) fn preliminary_provider(identity: &WindowIdentity) -> Option<ProviderFamily> {
    if let Some(package_family) = identity.package_family.as_deref() {
        if let Some(provider) = provider_from_package_family(package_family) {
            return Some(provider);
        }
    }

    claude_unpacked_identity(identity).then_some(ProviderFamily::Anthropic)
}

fn provider_from_package_family(package_family: &str) -> Option<ProviderFamily> {
    let (package_name, publisher_id) = package_family.rsplit_once('_')?;
    PROVIDER_FINGERPRINTS_V1
        .iter()
        .find(|fingerprint| {
            package_name_matches(
                package_name,
                fingerprint.package_name,
                fingerprint.package_name_match,
            ) && fingerprint
                .publisher_id
                .is_none_or(|expected| publisher_id == expected)
        })
        .map(|fingerprint| fingerprint.provider)
}

fn package_name_matches(value: &str, expected: &str, match_kind: PackageNameMatch) -> bool {
    match match_kind {
        PackageNameMatch::Exact => value == expected,
        PackageNameMatch::Prefix => value.starts_with(expected),
    }
}

fn claude_unpacked_identity(identity: &WindowIdentity) -> bool {
    let process_is_claude =
        windows_basename(&identity.process_name).eq_ignore_ascii_case("Claude.exe");
    let title_is_claude = title_contains_token(&identity.title_hint, "claude");
    let path_is_safe_install = identity
        .executable_path
        .as_deref()
        .is_some_and(is_absolute_non_temporary_claude_path);

    process_is_claude && title_is_claude && path_is_safe_install
}

fn is_absolute_non_temporary_claude_path(value: &str) -> bool {
    windows_path_is_absolute(value)
        && windows_basename(value).eq_ignore_ascii_case("Claude.exe")
        && !value
            .split(['\\', '/'])
            .any(|component| matches!(component.to_ascii_lowercase().as_str(), "temp" | "tmp"))
}

fn windows_path_is_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    (bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/'))
        || value.starts_with(r"\\")
}

fn windows_basename(value: &str) -> &str {
    value.rsplit(['\\', '/']).next().unwrap_or(value)
}

pub(crate) fn confirm_provider(provider: ProviderFamily, controls: &[RawControl]) -> bool {
    match provider {
        ProviderFamily::OpenAi => true,
        ProviderFamily::Anthropic => controls.iter().any(|control| {
            allowed_selector(control.kind) && control.name.trim().eq_ignore_ascii_case("Claude")
        }),
    }
}

pub(crate) fn classify_surface(
    provider: ProviderFamily,
    controls: &[RawControl],
    title_hint: &str,
) -> Option<(ClientSurface, ClientSelectionConfidence)> {
    if provider == ProviderFamily::Anthropic {
        return Some((
            ClientSurface::Claude,
            ClientSelectionConfidence::VisibleSelector,
        ));
    }

    let mut chatgpt_anchor = false;
    let mut codex_anchor = false;
    for control in controls
        .iter()
        .filter(|control| allowed_selector(control.kind))
    {
        match control.name.trim().to_ascii_lowercase().as_str() {
            "chatgpt" | "chat" | "work" => chatgpt_anchor = true,
            "codex" => codex_anchor = true,
            _ => {}
        }
    }

    match (chatgpt_anchor, codex_anchor) {
        (true, true) => None,
        (true, false) => Some((
            ClientSurface::ChatGpt,
            ClientSelectionConfidence::VisibleSelector,
        )),
        (false, true) => Some((
            ClientSurface::CodexDesktop,
            ClientSelectionConfidence::VisibleSelector,
        )),
        (false, false) => surface_from_title_hint(title_hint),
    }
}

fn surface_from_title_hint(title_hint: &str) -> Option<(ClientSurface, ClientSelectionConfidence)> {
    let chatgpt = ["chatgpt", "chat", "work"]
        .iter()
        .any(|anchor| title_contains_token(title_hint, anchor));
    let codex = title_contains_token(title_hint, "codex");

    match (chatgpt, codex) {
        (true, false) => Some((
            ClientSurface::ChatGpt,
            ClientSelectionConfidence::BestEffort,
        )),
        (false, true) => Some((
            ClientSurface::CodexDesktop,
            ClientSelectionConfidence::BestEffort,
        )),
        _ => None,
    }
}

fn title_contains_token(value: &str, expected: &str) -> bool {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|token| token.eq_ignore_ascii_case(expected))
}

fn allowed_selector(kind: ControlKind) -> bool {
    matches!(
        kind,
        ControlKind::Button | ControlKind::ComboBox | ControlKind::MenuItem
    )
}

fn normalized_effort(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if !safe_display_text(trimmed) || trimmed.chars().count() > 40 {
        return None;
    }

    let canonical = match trimmed.to_ascii_lowercase().as_str() {
        "无" | "none" => Some("none"),
        "最少" | "minimal" => Some("minimal"),
        "轻度" | "低" | "low" | "light" => Some("low"),
        "中" | "medium" => Some("medium"),
        "高" | "high" => Some("high"),
        "极高" | "extra high" | "xhigh" => Some("xhigh"),
        "最高" | "max" => Some("max"),
        "ultra" => Some("ultra"),
        _ => None,
    };
    if let Some(canonical) = canonical {
        return Some(canonical.to_owned());
    }

    let lower = trimmed.to_ascii_lowercase();
    ["reason", "thinking", "effort", "推理", "思考"]
        .iter()
        .any(|marker| lower.contains(marker))
        .then(|| trimmed.to_owned())
}

fn safe_display_text(value: &str) -> bool {
    !value.is_empty() && !contains_forbidden_display_character(value)
}

fn looks_like_model(target: TargetKind, value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    match target {
        TargetKind::ChatGptClient => {
            lower.contains("gpt")
                || lower.contains("codex")
                || (lower.starts_with('o')
                    && lower[1..]
                        .chars()
                        .next()
                        .is_some_and(|value| value.is_ascii_digit()))
        }
        TargetKind::ClaudeClient => {
            lower.contains("claude")
                || lower.contains("sonnet")
                || lower.contains("opus")
                || lower.contains("haiku")
        }
        _ => false,
    }
}

fn surface_matches_target(target: TargetKind, surface: ClientSurface) -> bool {
    match target {
        TargetKind::ChatGptClient => {
            matches!(
                surface,
                ClientSurface::ChatGpt | ClientSurface::CodexDesktop
            )
        }
        TargetKind::ClaudeClient => surface == ClientSurface::Claude,
        _ => false,
    }
}

pub(crate) fn extract_candidates(
    target: TargetKind,
    controls: &[ObservedControl],
) -> ClientSelectionDetection {
    if !matches!(target, TargetKind::ChatGptClient | TargetKind::ClaudeClient) {
        return ClientSelectionDetection::failed(ClientSelectionStatus::Unsupported);
    }

    let mut candidates = Vec::new();
    for surface in [
        ClientSurface::ChatGpt,
        ClientSurface::CodexDesktop,
        ClientSurface::Claude,
    ] {
        if !surface_matches_target(target, surface) {
            continue;
        }

        let mut models = Vec::new();
        let mut efforts = Vec::new();
        for control in controls
            .iter()
            .filter(|control| control.surface == surface && allowed_selector(control.kind))
        {
            let trimmed = control.name.trim();
            if looks_like_model(target, trimmed)
                && is_valid_reported_model(trimmed)
                && !models.iter().any(|model| model == trimmed)
            {
                models.push(trimmed.to_owned());
            }
            if let Some(effort) = normalized_effort(trimmed) {
                if !efforts.contains(&effort) {
                    efforts.push(effort);
                }
            }
        }

        match (models.is_empty(), efforts.is_empty()) {
            (true, true) => {}
            (false, true) => {
                for model in models {
                    push_unique_candidate(&mut candidates, surface, Some(model), None);
                }
            }
            (true, false) => {
                for effort in efforts {
                    push_unique_candidate(&mut candidates, surface, None, Some(effort));
                }
            }
            (false, false) => {
                for model in &models {
                    for effort in &efforts {
                        push_unique_candidate(
                            &mut candidates,
                            surface,
                            Some(model.clone()),
                            Some(effort.clone()),
                        );
                    }
                }
            }
        }
    }

    let status = match candidates.len() {
        0 => ClientSelectionStatus::NotExposed,
        1 => ClientSelectionStatus::Detected,
        _ => ClientSelectionStatus::Multiple,
    };
    ClientSelectionDetection { status, candidates }
}

fn push_unique_candidate(
    candidates: &mut Vec<ClientSelectionCandidate>,
    surface: ClientSurface,
    model: Option<String>,
    reasoning_effort: Option<String>,
) {
    if candidates.iter().any(|candidate| {
        candidate.surface == surface
            && candidate.model == model
            && candidate.reasoning_effort == reasoning_effort
    }) {
        return;
    }
    candidates.push(ClientSelectionCandidate {
        model,
        reasoning_effort,
        surface,
        source: ModelSource::WindowsAccessibility,
        confidence: ClientSelectionConfidence::VisibleSelector,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw_control(kind: ControlKind, name: &str) -> RawControl {
        RawControl {
            kind,
            name: name.to_owned(),
        }
    }

    fn control_for(surface: ClientSurface, kind: ControlKind, name: &str) -> ObservedControl {
        ObservedControl {
            surface,
            kind,
            name: name.to_owned(),
        }
    }

    fn control(kind: ControlKind, name: &str) -> ObservedControl {
        control_for(ClientSurface::ChatGpt, kind, name)
    }

    fn identity(
        process_name: &str,
        package_family: Option<&str>,
        title_hint: &str,
    ) -> WindowIdentity {
        WindowIdentity {
            process_name: process_name.to_owned(),
            package_family: package_family.map(str::to_owned),
            title_hint: title_hint.to_owned(),
            executable_path: None,
        }
    }

    #[test]
    fn openai_selector_extracts_model_and_effort_without_document_text() {
        let controls = vec![
            control(ControlKind::Document, "private conversation GPT-Fake"),
            control(ControlKind::Button, "GPT-5.6"),
            control(ControlKind::ComboBox, "最高"),
        ];

        let result = extract_candidates(TargetKind::ChatGptClient, &controls);

        assert_eq!(result.status, ClientSelectionStatus::Detected);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].model.as_deref(), Some("GPT-5.6"));
        assert_eq!(
            result.candidates[0].reasoning_effort.as_deref(),
            Some("max")
        );
        assert_eq!(
            result.candidates[0].source,
            ModelSource::WindowsAccessibility
        );
    }

    #[test]
    fn detection_serializes_to_the_reviewed_wire_shape() {
        let result = ClientSelectionDetection {
            status: ClientSelectionStatus::Detected,
            candidates: vec![ClientSelectionCandidate {
                model: Some("GPT-5.6".to_owned()),
                reasoning_effort: Some("max".to_owned()),
                surface: ClientSurface::ChatGpt,
                source: ModelSource::WindowsAccessibility,
                confidence: ClientSelectionConfidence::VisibleSelector,
            }],
        };

        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({
                "status": "detected",
                "candidates": [{
                    "model": "GPT-5.6",
                    "reasoningEffort": "max",
                    "surface": "chatgpt",
                    "source": "windows_accessibility",
                    "confidence": "visible_selector"
                }]
            })
        );
    }

    #[test]
    fn multiple_visible_models_are_returned_without_guessing() {
        let controls = vec![
            control(ControlKind::Button, "GPT-5.6"),
            control(ControlKind::Button, "GPT-5.6 Codex"),
        ];

        let result = extract_candidates(TargetKind::ChatGptClient, &controls);

        assert_eq!(result.status, ClientSelectionStatus::Multiple);
        assert_eq!(result.candidates.len(), 2);
    }

    #[test]
    fn claude_parser_rejects_openai_labels_and_unsafe_text() {
        let controls = vec![
            control_for(ClientSurface::Claude, ControlKind::Button, "GPT-5.6"),
            control_for(
                ClientSurface::Claude,
                ControlKind::Button,
                "Claude Sonnet\u{202e}",
            ),
        ];

        let result = extract_candidates(TargetKind::ClaudeClient, &controls);

        assert_eq!(result.status, ClientSelectionStatus::NotExposed);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn safe_unknown_effort_survives_as_an_exact_custom_value() {
        let controls = vec![
            control(ControlKind::Button, "GPT-5.6"),
            control(ControlKind::MenuItem, "扩展思考"),
        ];

        let result = extract_candidates(TargetKind::ChatGptClient, &controls);

        assert_eq!(result.status, ClientSelectionStatus::Detected);
        assert_eq!(
            result.candidates[0].reasoning_effort.as_deref(),
            Some("扩展思考")
        );
    }

    #[test]
    fn overlong_effort_edit_text_and_overlong_model_are_discarded() {
        let controls = vec![
            control(ControlKind::Button, "GPT-5.6"),
            control(ControlKind::ComboBox, &format!("思{}", "考".repeat(40))),
            control(ControlKind::Edit, "GPT-Edit"),
            control(ControlKind::Other, "GPT-Other"),
            control(ControlKind::Button, &format!("GPT-{}", "x".repeat(117))),
        ];

        let result = extract_candidates(TargetKind::ChatGptClient, &controls);

        assert_eq!(result.status, ClientSelectionStatus::Detected);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].model.as_deref(), Some("GPT-5.6"));
        assert_eq!(result.candidates[0].reasoning_effort, None);
    }

    #[test]
    fn distinct_efforts_form_distinct_deduplicated_candidates() {
        let controls = vec![
            control(ControlKind::Button, "GPT-5.6"),
            control(ControlKind::Button, "GPT-5.6"),
            control(ControlKind::ComboBox, "high"),
            control(ControlKind::ComboBox, "max"),
            control(ControlKind::ComboBox, "max"),
        ];

        let result = extract_candidates(TargetKind::ChatGptClient, &controls);

        assert_eq!(result.status, ClientSelectionStatus::Multiple);
        assert_eq!(result.candidates.len(), 2);
    }

    #[test]
    fn process_name_title_and_model_label_never_establish_provider_identity() {
        assert_eq!(
            preliminary_provider(&identity("ChatGPT.exe", None, "ChatGPT")),
            None
        );
        assert_eq!(
            preliminary_provider(&identity("Claude.exe", None, "Claude")),
            None
        );
        assert_eq!(
            preliminary_provider(&identity("other.exe", None, "GPT-5.6")),
            None
        );
    }

    #[test]
    fn openai_package_establishes_provider_but_header_establishes_surface() {
        assert_eq!(
            preliminary_provider(&identity(
                "ChatGPT.exe",
                Some("OpenAI.Codex_2p2nqsd0c76g0"),
                "ChatGPT"
            )),
            Some(ProviderFamily::OpenAi)
        );
        assert_eq!(
            classify_surface(
                ProviderFamily::OpenAi,
                &[raw_control(ControlKind::Button, "Codex")],
                "ChatGPT",
            ),
            Some((
                ClientSurface::CodexDesktop,
                ClientSelectionConfidence::VisibleSelector,
            ))
        );
        assert_eq!(
            preliminary_provider(&identity(
                "ChatGPT.exe",
                Some("Other.Codex_2p2nqsd0c76g0"),
                "ChatGPT"
            )),
            None
        );
        assert_eq!(
            preliminary_provider(&identity(
                "ChatGPT.exe",
                Some("OpenAI.Codex_wrongpublisher"),
                "ChatGPT"
            )),
            None
        );
    }

    #[test]
    fn openai_surface_conflict_is_ambiguous_and_non_header_controls_are_ignored() {
        assert_eq!(
            classify_surface(
                ProviderFamily::OpenAi,
                &[
                    raw_control(ControlKind::Button, "ChatGPT"),
                    raw_control(ControlKind::MenuItem, "Codex"),
                ],
                "ChatGPT",
            ),
            None
        );
        assert_eq!(
            classify_surface(
                ProviderFamily::OpenAi,
                &[raw_control(ControlKind::Document, "Codex")],
                "ChatGPT",
            ),
            Some((
                ClientSurface::ChatGpt,
                ClientSelectionConfidence::BestEffort,
            ))
        );
        assert_eq!(
            classify_surface(ProviderFamily::OpenAi, &[], "Homework"),
            None
        );
    }

    #[test]
    fn claude_unpackaged_identity_requires_path_process_and_title_then_header_confirmation() {
        let mut valid = identity("Claude.exe", None, "Claude");
        valid.executable_path = Some(r"C:\Program Files\Claude\Claude.exe".to_owned());
        assert_eq!(
            preliminary_provider(&valid),
            Some(ProviderFamily::Anthropic)
        );
        assert!(confirm_provider(
            ProviderFamily::Anthropic,
            &[raw_control(ControlKind::Button, "Claude")]
        ));
        assert!(!confirm_provider(
            ProviderFamily::Anthropic,
            &[raw_control(ControlKind::Button, "Claude Sonnet 4")]
        ));

        let mut temporary = identity("Claude.exe", None, "Claude");
        temporary.executable_path = Some(r"C:\Users\me\AppData\Local\Temp\Claude.exe".to_owned());
        assert_eq!(preliminary_provider(&temporary), None);

        let mut wrong_title = identity("Claude.exe", None, "Messages");
        wrong_title.executable_path = Some(r"C:\Program Files\Claude\Claude.exe".to_owned());
        assert_eq!(preliminary_provider(&wrong_title), None);

        let mut embedded_title = identity("Claude.exe", None, "NotClaude");
        embedded_title.executable_path = Some(r"C:\Program Files\Claude\Claude.exe".to_owned());
        assert_eq!(preliminary_provider(&embedded_title), None);
    }

    #[test]
    fn claude_package_identity_still_requires_visible_header_confirmation() {
        let packaged = identity("host.exe", Some("Anthropic.Claude_publisher"), "Messages");
        assert_eq!(
            preliminary_provider(&packaged),
            Some(ProviderFamily::Anthropic)
        );
        assert_eq!(
            preliminary_provider(&identity(
                "host.exe",
                Some("Anthropic.ClaudeImpostor_publisher"),
                "Messages",
            )),
            None
        );
        assert!(!confirm_provider(
            ProviderFamily::Anthropic,
            &[raw_control(ControlKind::Document, "Claude")]
        ));
        assert_eq!(
            classify_surface(ProviderFamily::Anthropic, &[], "anything"),
            Some((
                ClientSurface::Claude,
                ClientSelectionConfidence::VisibleSelector,
            ))
        );
    }
}
