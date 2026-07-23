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

mod checked_observation {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct ObservedControl {
        surface: ClientSurface,
        confidence: ClientSelectionConfidence,
        kind: ControlKind,
        role: SelectorRole,
        name: String,
    }

    impl ObservedControl {
        pub(super) fn surface(&self) -> ClientSurface {
            self.surface
        }

        pub(super) fn confidence(&self) -> ClientSelectionConfidence {
            self.confidence
        }

        pub(super) fn checked_value(&self, target: TargetKind) -> Option<(SelectorRole, String)> {
            if !allowed_selector(self.kind) {
                return None;
            }
            let (role, value) = selector_role(target, &self.name)?;
            (role == self.role).then_some((role, value))
        }
    }

    pub(crate) fn observe_window_controls(
        provider: ProviderFamily,
        controls: &[RawControl],
        title_hint: &str,
    ) -> Option<Vec<ObservedControl>> {
        let (surface, confidence) = classify_surface(provider, controls, title_hint)?;
        let target = match provider {
            ProviderFamily::OpenAi => TargetKind::ChatGptClient,
            ProviderFamily::Anthropic => TargetKind::ClaudeClient,
        };
        Some(
            controls
                .iter()
                .filter(|control| allowed_selector(control.kind))
                .filter_map(|control| {
                    let (role, name) = selector_role(target, &control.name)?;
                    Some(ObservedControl {
                        surface,
                        confidence,
                        kind: control.kind,
                        role,
                        name,
                    })
                })
                .collect(),
        )
    }

    #[cfg(test)]
    pub(super) fn test_fixture(
        surface: ClientSurface,
        confidence: ClientSelectionConfidence,
        kind: ControlKind,
        role: SelectorRole,
        name: &str,
    ) -> ObservedControl {
        ObservedControl {
            surface,
            confidence,
            kind,
            role,
            name: name.to_owned(),
        }
    }
}

pub(crate) use checked_observation::ObservedControl;

pub(crate) fn observe_window_controls(
    provider: ProviderFamily,
    controls: &[RawControl],
    title_hint: &str,
) -> Option<Vec<ObservedControl>> {
    checked_observation::observe_window_controls(provider, controls, title_hint)
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
    match identity.package_family.as_deref() {
        Some(package_family) => provider_from_package_family(package_family),
        None => claude_unpacked_identity(identity).then_some(ProviderFamily::Anthropic),
    }
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
    let Some(components) = validated_windows_file_components(value) else {
        return false;
    };
    components
        .last()
        .is_some_and(|name| name.eq_ignore_ascii_case("Claude.exe"))
        && !components
            .iter()
            .any(|component| matches!(component.to_ascii_lowercase().as_str(), "temp" | "tmp"))
}

fn validated_windows_file_components(value: &str) -> Option<Vec<&str>> {
    let bytes = value.as_bytes();
    let tail = if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        &value[3..]
    } else if bytes.len() >= 2
        && matches!(bytes[0], b'\\' | b'/')
        && matches!(bytes[1], b'\\' | b'/')
    {
        &value[2..]
    } else {
        return None;
    };

    let components = tail.split(['\\', '/']).collect::<Vec<_>>();
    let is_unc = matches!(bytes.first(), Some(b'\\' | b'/'));
    if (is_unc && components.len() < 3)
        || (is_unc && matches!(components.first(), Some(&"." | &"?")))
        || components.is_empty()
        || components
            .iter()
            .any(|component| component.is_empty() || matches!(*component, "." | ".."))
    {
        return None;
    }
    Some(components)
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
        return confirm_provider(provider, controls).then_some((
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectorRole {
    Model,
    Effort,
}

fn selector_role(target: TargetKind, value: &str) -> Option<(SelectorRole, String)> {
    let trimmed = value.trim();
    if is_reserved_anchor(trimmed) {
        return None;
    }
    if looks_like_model(target, trimmed) && is_valid_reported_model(trimmed) {
        return Some((SelectorRole::Model, trimmed.to_owned()));
    }
    normalized_effort(trimmed).map(|effort| (SelectorRole::Effort, effort))
}

fn is_reserved_anchor(value: &str) -> bool {
    ["claude", "chatgpt", "chat", "work", "codex"]
        .iter()
        .any(|anchor| value.eq_ignore_ascii_case(anchor))
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedValue {
    value: String,
    confidence: ClientSelectionConfidence,
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

        let mut models: Vec<ObservedValue> = Vec::new();
        let mut efforts: Vec<ObservedValue> = Vec::new();
        for control in controls
            .iter()
            .filter(|control| control.surface() == surface)
        {
            match control.checked_value(target) {
                Some((SelectorRole::Model, model)) => {
                    insert_observed_value(&mut models, model, control.confidence());
                }
                Some((SelectorRole::Effort, effort)) => {
                    insert_observed_value(&mut efforts, effort, control.confidence());
                }
                _ => {}
            }
        }

        match (models.is_empty(), efforts.is_empty()) {
            (true, true) => {}
            (false, true) => {
                for model in models {
                    push_unique_candidate(
                        &mut candidates,
                        surface,
                        Some(model.value),
                        None,
                        model.confidence,
                    );
                }
            }
            (true, false) => {
                for effort in efforts {
                    push_unique_candidate(
                        &mut candidates,
                        surface,
                        None,
                        Some(effort.value),
                        effort.confidence,
                    );
                }
            }
            (false, false) => {
                for model in &models {
                    for effort in &efforts {
                        push_unique_candidate(
                            &mut candidates,
                            surface,
                            Some(model.value.clone()),
                            Some(effort.value.clone()),
                            weakest_confidence(model.confidence, effort.confidence),
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

fn insert_observed_value(
    values: &mut Vec<ObservedValue>,
    value: String,
    confidence: ClientSelectionConfidence,
) {
    if let Some(existing) = values.iter_mut().find(|existing| existing.value == value) {
        existing.confidence = weakest_confidence(existing.confidence, confidence);
    } else {
        values.push(ObservedValue { value, confidence });
    }
}

fn weakest_confidence(
    left: ClientSelectionConfidence,
    right: ClientSelectionConfidence,
) -> ClientSelectionConfidence {
    if matches!(left, ClientSelectionConfidence::BestEffort)
        || matches!(right, ClientSelectionConfidence::BestEffort)
    {
        ClientSelectionConfidence::BestEffort
    } else {
        ClientSelectionConfidence::VisibleSelector
    }
}

fn push_unique_candidate(
    candidates: &mut Vec<ClientSelectionCandidate>,
    surface: ClientSurface,
    model: Option<String>,
    reasoning_effort: Option<String>,
    confidence: ClientSelectionConfidence,
) {
    if let Some(existing) = candidates.iter_mut().find(|candidate| {
        candidate.surface == surface
            && candidate.model == model
            && candidate.reasoning_effort == reasoning_effort
    }) {
        existing.confidence = weakest_confidence(existing.confidence, confidence);
        return;
    }
    candidates.push(ClientSelectionCandidate {
        model,
        reasoning_effort,
        surface,
        source: ModelSource::WindowsAccessibility,
        confidence,
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
        let target = match surface {
            ClientSurface::ChatGpt | ClientSurface::CodexDesktop => TargetKind::ChatGptClient,
            ClientSurface::Claude => TargetKind::ClaudeClient,
        };
        let role = selector_role(target, name)
            .map(|(role, _)| role)
            .unwrap_or(SelectorRole::Model);
        checked_observation::test_fixture(
            surface,
            ClientSelectionConfidence::VisibleSelector,
            kind,
            role,
            name,
        )
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
    fn exact_claude_header_anchor_is_not_a_model_candidate() {
        let controls = vec![control_for(
            ClientSurface::Claude,
            ControlKind::Button,
            "Claude",
        )];

        let result = extract_candidates(TargetKind::ClaudeClient, &controls);

        assert_eq!(result.status, ClientSelectionStatus::NotExposed);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn exact_openai_surface_anchors_are_not_model_candidates() {
        for anchor in ["ChatGPT", "Chat", "Work", "Codex"] {
            let result = extract_candidates(
                TargetKind::ChatGptClient,
                &[control(ControlKind::Button, anchor)],
            );

            assert_eq!(result.status, ClientSelectionStatus::NotExposed, "{anchor}");
            assert!(result.candidates.is_empty(), "{anchor}");
        }
    }

    #[test]
    fn surface_anchor_plus_real_model_returns_only_the_real_model() {
        let controls = vec![
            control(ControlKind::Button, "ChatGPT"),
            control(ControlKind::Button, "GPT-5.6"),
        ];

        let result = extract_candidates(TargetKind::ChatGptClient, &controls);

        assert_eq!(result.status, ClientSelectionStatus::Detected);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].model.as_deref(), Some("GPT-5.6"));
    }

    #[test]
    fn model_like_thinking_label_is_not_reused_as_effort() {
        let model_only = extract_candidates(
            TargetKind::ChatGptClient,
            &[control(ControlKind::Button, "GPT-5 Thinking")],
        );
        assert_eq!(model_only.status, ClientSelectionStatus::Detected);
        assert_eq!(model_only.candidates.len(), 1);
        assert_eq!(
            model_only.candidates[0].model.as_deref(),
            Some("GPT-5 Thinking")
        );
        assert_eq!(model_only.candidates[0].reasoning_effort, None);

        let with_effort = extract_candidates(
            TargetKind::ChatGptClient,
            &[
                control(ControlKind::Button, "GPT-5 Thinking"),
                control(ControlKind::ComboBox, "high"),
            ],
        );
        assert_eq!(with_effort.status, ClientSelectionStatus::Detected);
        assert_eq!(with_effort.candidates.len(), 1);
        assert_eq!(
            with_effort.candidates[0].reasoning_effort.as_deref(),
            Some("high")
        );
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
    fn title_fallback_confidence_reaches_the_candidate() {
        let observed = observe_window_controls(
            ProviderFamily::OpenAi,
            &[raw_control(ControlKind::Button, "GPT-5.6")],
            "Codex",
        )
        .unwrap();

        let result = extract_candidates(TargetKind::ChatGptClient, &observed);

        assert_eq!(result.status, ClientSelectionStatus::Detected);
        assert_eq!(
            result.candidates[0].confidence,
            ClientSelectionConfidence::BestEffort
        );
        assert_eq!(result.candidates[0].surface, ClientSurface::CodexDesktop);
    }

    #[test]
    fn visible_surface_anchor_confidence_reaches_the_candidate() {
        let observed = observe_window_controls(
            ProviderFamily::OpenAi,
            &[
                raw_control(ControlKind::Button, "Codex"),
                raw_control(ControlKind::Button, "GPT-5.6"),
            ],
            "ChatGPT",
        )
        .unwrap();

        let result = extract_candidates(TargetKind::ChatGptClient, &observed);

        assert_eq!(result.status, ClientSelectionStatus::Detected);
        assert_eq!(
            result.candidates[0].confidence,
            ClientSelectionConfidence::VisibleSelector
        );
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].model.as_deref(), Some("GPT-5.6"));
    }

    #[test]
    fn conflicting_title_surface_produces_no_observations_or_candidates() {
        let observed = observe_window_controls(
            ProviderFamily::OpenAi,
            &[raw_control(ControlKind::Button, "GPT-5.6")],
            "ChatGPT Codex",
        );

        assert_eq!(observed, None);
        let result = extract_candidates(TargetKind::ChatGptClient, &[]);
        assert_eq!(result.status, ClientSelectionStatus::NotExposed);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn model_effort_pair_uses_the_weaker_observation_confidence() {
        let mut observed = observe_window_controls(
            ProviderFamily::OpenAi,
            &[raw_control(ControlKind::Button, "GPT-5.6")],
            "Codex",
        )
        .unwrap();
        observed.extend(
            observe_window_controls(
                ProviderFamily::OpenAi,
                &[
                    raw_control(ControlKind::Button, "Codex"),
                    raw_control(ControlKind::ComboBox, "high"),
                ],
                "ChatGPT",
            )
            .unwrap(),
        );

        let result = extract_candidates(TargetKind::ChatGptClient, &observed);

        assert_eq!(result.status, ClientSelectionStatus::Detected);
        assert_eq!(
            result.candidates[0].reasoning_effort.as_deref(),
            Some("high")
        );
        assert_eq!(
            result.candidates[0].confidence,
            ClientSelectionConfidence::BestEffort
        );
    }

    #[test]
    fn claude_observation_factory_enforces_confirmation_and_removes_anchor_role() {
        for controls in [
            vec![],
            vec![raw_control(ControlKind::Document, "Claude")],
            vec![raw_control(ControlKind::Button, "Claude Sonnet 4")],
        ] {
            assert_eq!(
                observe_window_controls(ProviderFamily::Anthropic, &controls, "Claude"),
                None
            );
        }

        let observed = observe_window_controls(
            ProviderFamily::Anthropic,
            &[
                raw_control(ControlKind::Button, "Claude"),
                raw_control(ControlKind::Button, "Claude Sonnet 4"),
            ],
            "Claude",
        )
        .unwrap();
        let result = extract_candidates(TargetKind::ClaudeClient, &observed);
        assert_eq!(result.status, ClientSelectionStatus::Detected);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(
            result.candidates[0].model.as_deref(),
            Some("Claude Sonnet 4")
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
    fn unknown_package_family_never_falls_back_to_unpackaged_claude_signals() {
        let mut identity = identity("Claude.exe", Some("Other.App_publisher"), "Claude");
        identity.executable_path = Some(r"C:\Program Files\Claude\Claude.exe".to_owned());

        assert_eq!(preliminary_provider(&identity), None);
    }

    #[test]
    fn unpackaged_claude_rejects_non_install_path_syntax() {
        for path in [
            r"Claude.exe",
            r"C:\Program Files\Claude\NotClaude.exe",
            r"\\.\Claude.exe",
            r"\\?\C:\Program Files\Claude\Claude.exe",
            r"\\server\Claude.exe",
            r"\\server\\Claude.exe",
            r"C:\Program Files\.\Claude.exe",
            r"C:\Program Files\Claude\..\Claude.exe",
        ] {
            let mut identity = identity("Claude.exe", None, "Claude");
            identity.executable_path = Some(path.to_owned());

            assert_eq!(preliminary_provider(&identity), None, "{path}");
        }
    }

    #[test]
    fn unpackaged_claude_rejects_temp_components_but_not_template() {
        for path in [
            r"C:\Temp\Claude.exe",
            r"C:\TMP\Claude.exe",
            r"C:\Program Files/TeMp\Claude.exe",
        ] {
            let mut identity = identity("Claude.exe", None, "Claude");
            identity.executable_path = Some(path.to_owned());

            assert_eq!(preliminary_provider(&identity), None, "{path}");
        }

        let mut template = identity("Claude.exe", None, "Claude");
        template.executable_path = Some(r"C:\Program Files\Template\Claude\Claude.exe".to_owned());
        assert_eq!(
            preliminary_provider(&template),
            Some(ProviderFamily::Anthropic)
        );
    }

    #[test]
    fn unpackaged_claude_accepts_drive_rooted_and_complete_unc_file_paths() {
        for path in [
            r"C:\Program Files\Claude\Claude.exe",
            r"\\server\share\Claude\Claude.exe",
        ] {
            let mut identity = identity("Claude.exe", None, "Claude");
            identity.executable_path = Some(path.to_owned());

            assert_eq!(
                preliminary_provider(&identity),
                Some(ProviderFamily::Anthropic),
                "{path}"
            );
        }
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
        for controls in [
            vec![],
            vec![raw_control(ControlKind::Document, "Claude")],
            vec![raw_control(ControlKind::Button, "Claude Sonnet 4")],
        ] {
            assert_eq!(
                classify_surface(ProviderFamily::Anthropic, &controls, "Claude"),
                None
            );
        }
        for kind in [
            ControlKind::Button,
            ControlKind::ComboBox,
            ControlKind::MenuItem,
        ] {
            assert_eq!(
                classify_surface(
                    ProviderFamily::Anthropic,
                    &[raw_control(kind, "Claude")],
                    "anything",
                ),
                Some((
                    ClientSurface::Claude,
                    ClientSelectionConfidence::VisibleSelector,
                ))
            );
        }
    }
}
