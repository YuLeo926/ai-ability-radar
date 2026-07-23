use super::{
    extract_candidates, observe_window_controls, ClientSelectionDetection, ClientSelectionStatus,
    ControlKind, ObservedControl, ProviderFamily, RawControl, WindowIdentity,
};
use ability_core::TargetKind;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use windows::core::{BOOL, HRESULT, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HANDLE, HWND,
    LPARAM, RECT,
};
use windows::Win32::Storage::Packaging::Appx::GetPackageFamilyName;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
    UIA_ButtonControlTypeId, UIA_ComboBoxControlTypeId, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_MenuItemControlTypeId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible,
};

const MAX_WINDOWS: usize = 24;
const MAX_NODES: usize = 512;
const MAX_DEPTH: usize = 24;
const MAX_LABEL_CHARS: usize = 120;
const SCAN_BUDGET: Duration = Duration::from_millis(1_200);
const MAX_PROCESS_PATH_CHARS: usize = 32_768;
const MAX_TITLE_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScanFailure {
    TimedOut,
    Unavailable,
}

#[derive(Debug)]
struct TraversalBudget {
    max_nodes: usize,
    max_depth: usize,
    visited_nodes: usize,
    deadline: Instant,
}

impl TraversalBudget {
    fn new(max_nodes: usize, max_depth: usize, deadline: Instant) -> Self {
        Self {
            max_nodes,
            max_depth,
            visited_nodes: 0,
            deadline,
        }
    }

    fn try_visit(&mut self, depth: usize, now: Instant) -> Result<(), ScanFailure> {
        self.check_deadline(now)?;
        if depth > self.max_depth || self.visited_nodes >= self.max_nodes {
            return Err(ScanFailure::TimedOut);
        }
        self.visited_nodes += 1;
        Ok(())
    }

    fn can_enqueue(&self, queued_nodes: usize, child_depth: usize) -> bool {
        child_depth <= self.max_depth
            && self.visited_nodes.saturating_add(queued_nodes) < self.max_nodes
    }

    fn check_deadline(&self, now: Instant) -> Result<(), ScanFailure> {
        (now < self.deadline)
            .then_some(())
            .ok_or(ScanFailure::TimedOut)
    }
}

struct ProcessHandle(HANDLE);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: `ProcessHandle` is created only from a successful `OpenProcess`
        // result and owns that single handle until drop.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: A `ComGuard` is constructed only after a successful
        // `CoInitializeEx` on this same blocking worker thread.
        unsafe { CoUninitialize() };
    }
}

struct ProviderWindow {
    hwnd: HWND,
    provider: ProviderFamily,
    identity: WindowIdentity,
}

struct EnumerationContext {
    handles: Vec<(HWND, u32)>,
    own_process_id: u32,
    deadline: Instant,
    stopped: Option<ScanFailure>,
}

struct EnumerationOutcome {
    handles: Vec<(HWND, u32)>,
    stopped: Option<ScanFailure>,
}

unsafe extern "system" fn enumerate_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `enumerate_visible_windows` passes a live, uniquely borrowed
    // `EnumerationContext` pointer for the synchronous duration of `EnumWindows`.
    let context = unsafe { &mut *(lparam.0 as *mut EnumerationContext) };
    if Instant::now() >= context.deadline {
        context.stopped = Some(ScanFailure::TimedOut);
        return false.into();
    }
    // SAFETY: `hwnd` is supplied by `EnumWindows` for this callback.
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return true.into();
    }

    let mut process_id = 0;
    // SAFETY: `process_id` is writable for the duration of this call.
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if thread_id == 0 || process_id == 0 || process_id == context.own_process_id {
        return true.into();
    }
    if let Err(failure) = reserve_window_slot(context.handles.len()) {
        context.stopped = Some(failure);
        return false.into();
    }

    context.handles.push((hwnd, process_id));
    true.into()
}

fn reserve_window_slot(current_windows: usize) -> Result<(), ScanFailure> {
    (current_windows < MAX_WINDOWS)
        .then_some(())
        .ok_or(ScanFailure::TimedOut)
}

pub(super) fn scan(target: TargetKind) -> Result<ClientSelectionDetection, ScanFailure> {
    let started = Instant::now();
    let deadline = started + SCAN_BUDGET;
    let enumeration = enumerate_visible_windows(deadline)?;
    let expected_provider = provider_for_target(target).ok_or(ScanFailure::Unavailable)?;
    let mut windows = Vec::new();

    for (hwnd, process_id) in enumeration.handles {
        if Instant::now() >= deadline {
            return Err(ScanFailure::TimedOut);
        }
        let Some(identity) = read_window_identity(hwnd, process_id) else {
            continue;
        };
        if let Some(provider) = super::preliminary_provider(&identity) {
            if provider == expected_provider {
                windows.push(ProviderWindow {
                    hwnd,
                    provider,
                    identity,
                });
            }
        }
    }

    if windows.is_empty() {
        return match enumeration.stopped {
            Some(failure) => Err(failure),
            None => Ok(finish_scan(target, false, Vec::new())),
        };
    }
    if let Some(failure) = enumeration.stopped {
        return Err(failure);
    }

    let _com = initialize_com()?;
    // SAFETY: COM is initialized on this worker thread, no aggregation is
    // requested, and `IUIAutomation` owns the returned interface pointer.
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| ScanFailure::Unavailable)?;
    // SAFETY: `automation` is a live UI Automation interface.
    let walker = unsafe { automation.RawViewWalker() }.map_err(|_| ScanFailure::Unavailable)?;
    let mut budget = TraversalBudget::new(MAX_NODES, MAX_DEPTH, deadline);
    let mut observations = Vec::new();

    for window in windows {
        budget.check_deadline(Instant::now())?;
        let controls = collect_window_controls(&automation, &walker, &window, &mut budget)?;
        if let Some(checked) =
            observe_window_controls(window.provider, &controls, &window.identity.title_hint)
        {
            observations.extend(checked);
        }
    }

    Ok(finish_scan(target, true, observations))
}

fn provider_for_target(target: TargetKind) -> Option<ProviderFamily> {
    match target {
        TargetKind::ChatGptClient => Some(ProviderFamily::OpenAi),
        TargetKind::ClaudeClient => Some(ProviderFamily::Anthropic),
        _ => None,
    }
}

fn enumerate_visible_windows(deadline: Instant) -> Result<EnumerationOutcome, ScanFailure> {
    let mut context = EnumerationContext {
        handles: Vec::with_capacity(MAX_WINDOWS),
        own_process_id: std::process::id(),
        deadline,
        stopped: None,
    };
    // SAFETY: The callback and LPARAM point to `context`, which remains alive
    // and uniquely borrowed until this synchronous enumeration returns.
    let result = unsafe {
        EnumWindows(
            Some(enumerate_window),
            LPARAM((&mut context as *mut EnumerationContext) as isize),
        )
    };
    if result.is_err() && context.stopped.is_none() {
        return Err(ScanFailure::Unavailable);
    }
    Ok(EnumerationOutcome {
        handles: context.handles,
        stopped: context.stopped,
    })
}

fn read_window_identity(hwnd: HWND, process_id: u32) -> Option<WindowIdentity> {
    // SAFETY: The process ID comes from `GetWindowThreadProcessId`; the handle
    // is read-only and is wrapped immediately for balanced release.
    let process = ProcessHandle(
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?,
    );
    let executable_path = process_image_path(process.0)?;
    let process_name = executable_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_owned();
    if process_name.is_empty() {
        return None;
    }
    let package_family = process_package_family(process.0)?;
    let title_hint = window_title(hwnd)?;

    Some(WindowIdentity {
        process_name,
        package_family,
        title_hint,
        executable_path: Some(executable_path),
    })
}

fn process_image_path(handle: HANDLE) -> Option<String> {
    let mut buffer = vec![0_u16; MAX_PROCESS_PATH_CHARS];
    let mut length = u32::try_from(buffer.len()).ok()?;
    // SAFETY: The buffer is writable for `length` UTF-16 code units and the
    // process handle remains live for the call.
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .ok()?;
    let used = usize::try_from(length).ok()?;
    String::from_utf16(buffer.get(..used)?).ok()
}

fn process_package_family(handle: HANDLE) -> Option<Option<String>> {
    let mut length = 0_u32;
    // SAFETY: This first call intentionally supplies no output buffer so the
    // API returns either the required size or NO_PACKAGE.
    let status = unsafe { GetPackageFamilyName(handle, &mut length, None) };
    if status == APPMODEL_ERROR_NO_PACKAGE {
        return Some(None);
    }
    if status != ERROR_INSUFFICIENT_BUFFER || length == 0 {
        return None;
    }

    let mut buffer = vec![0_u16; usize::try_from(length).ok()?];
    // SAFETY: The buffer has the exact size reported by the first call and the
    // process handle remains live.
    let status =
        unsafe { GetPackageFamilyName(handle, &mut length, Some(PWSTR(buffer.as_mut_ptr()))) };
    if status != ERROR_SUCCESS {
        return None;
    }
    let used = usize::try_from(length).ok()?.min(buffer.len());
    let content = buffer
        .get(..used)?
        .strip_suffix(&[0])
        .unwrap_or(&buffer[..used]);
    String::from_utf16(content).ok().map(Some)
}

fn window_title(hwnd: HWND) -> Option<String> {
    // SAFETY: `hwnd` came from the current synchronous EnumWindows snapshot.
    let reported = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let capacity = reported.min(MAX_TITLE_CHARS).saturating_add(1);
    let mut buffer = vec![0_u16; capacity.max(1)];
    // SAFETY: `buffer` is writable, and the generated binding passes its
    // length to `GetWindowTextW`.
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if copied < 0 {
        return None;
    }
    String::from_utf16(buffer.get(..usize::try_from(copied).ok()?)?).ok()
}

fn initialize_com() -> Result<ComGuard, ScanFailure> {
    // SAFETY: This runs on the dedicated blocking worker, and successful calls
    // are balanced by the returned guard.
    let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    com_initialization_result(result)?;
    Ok(ComGuard)
}

fn com_initialization_result(result: HRESULT) -> Result<(), ScanFailure> {
    result.ok().map_err(|_| ScanFailure::Unavailable)
}

fn collect_window_controls(
    automation: &IUIAutomation,
    walker: &IUIAutomationTreeWalker,
    window: &ProviderWindow,
    budget: &mut TraversalBudget,
) -> Result<Vec<RawControl>, ScanFailure> {
    let mut window_bounds = RECT::default();
    // SAFETY: The HWND was returned by EnumWindows and the RECT is writable.
    unsafe { GetWindowRect(window.hwnd, &mut window_bounds) }
        .map_err(|_| ScanFailure::Unavailable)?;
    if window_bounds.right <= window_bounds.left || window_bounds.bottom <= window_bounds.top {
        return Err(ScanFailure::Unavailable);
    }
    // SAFETY: `automation` is live and the HWND was selected before UIA was
    // constructed.
    let root = unsafe { automation.ElementFromHandle(window.hwnd) }
        .map_err(|_| ScanFailure::Unavailable)?;
    let mut queue = VecDeque::from([(root, 0_usize)]);
    let mut controls = Vec::new();
    let mut traversal_truncated = false;

    while let Some((element, depth)) = queue.pop_front() {
        budget.try_visit(depth, Instant::now())?;
        collect_element_name(&element, window_bounds, &mut controls)?;
        budget.check_deadline(Instant::now())?;

        // SAFETY: `walker` and `element` are live COM interfaces on the
        // initialized worker thread.
        let mut child = optional_element(unsafe { walker.GetFirstChildElement(&element) })?;
        while let Some(current) = child {
            budget.check_deadline(Instant::now())?;
            let child_depth = depth.saturating_add(1);
            if !budget.can_enqueue(queue.len(), child_depth) {
                traversal_truncated = true;
                break;
            }
            queue.push_back((current.clone(), child_depth));
            // SAFETY: Both interfaces are live and stay on this COM worker.
            child = optional_element(unsafe { walker.GetNextSiblingElement(&current) })?;
        }
    }

    if traversal_truncated {
        Err(ScanFailure::TimedOut)
    } else {
        Ok(controls)
    }
}

fn optional_element(
    result: windows::core::Result<IUIAutomationElement>,
) -> Result<Option<IUIAutomationElement>, ScanFailure> {
    match result {
        Ok(element) => Ok(Some(element)),
        Err(error) if error.code().is_ok() => Ok(None),
        Err(_) => Err(ScanFailure::Unavailable),
    }
}

fn collect_element_name(
    element: &IUIAutomationElement,
    window_bounds: RECT,
    controls: &mut Vec<RawControl>,
) -> Result<(), ScanFailure> {
    // SAFETY: `element` is a live UI Automation element on the COM worker.
    let control_type =
        unsafe { element.CurrentControlType() }.map_err(|_| ScanFailure::Unavailable)?;
    let kind = control_kind(control_type);
    if !matches!(
        kind,
        ControlKind::Button | ControlKind::ComboBox | ControlKind::MenuItem
    ) {
        return Ok(());
    }
    // SAFETY: Bounding rectangles are read only after the element has been
    // identified as one of the three allowlisted selector control types.
    let bounds =
        unsafe { element.CurrentBoundingRectangle() }.map_err(|_| ScanFailure::Unavailable)?;
    if !eligible_selector_bounds(kind, bounds, window_bounds) {
        return Ok(());
    }
    // SAFETY: CurrentName is the sole label read and is called only for an
    // allowlisted selector fully inside the upper provider-window band.
    let name = unsafe { element.CurrentName() }.map_err(|_| ScanFailure::Unavailable)?;
    if let Some(control) = bounded_raw_control(kind, name.to_string()) {
        controls.push(control);
    }
    Ok(())
}

fn bounded_raw_control(kind: ControlKind, name: String) -> Option<RawControl> {
    (name.chars().count() <= MAX_LABEL_CHARS).then_some(RawControl { kind, name })
}

fn control_kind(
    control_type: windows::Win32::UI::Accessibility::UIA_CONTROLTYPE_ID,
) -> ControlKind {
    match control_type {
        value if value == UIA_ButtonControlTypeId => ControlKind::Button,
        value if value == UIA_ComboBoxControlTypeId => ControlKind::ComboBox,
        value if value == UIA_MenuItemControlTypeId => ControlKind::MenuItem,
        value if value == UIA_DocumentControlTypeId => ControlKind::Document,
        value if value == UIA_EditControlTypeId => ControlKind::Edit,
        _ => ControlKind::Other,
    }
}

fn eligible_selector_bounds(kind: ControlKind, control: RECT, window: RECT) -> bool {
    if !matches!(
        kind,
        ControlKind::Button | ControlKind::ComboBox | ControlKind::MenuItem
    ) || window.right <= window.left
        || window.bottom <= window.top
        || control.right <= control.left
        || control.bottom <= control.top
    {
        return false;
    }

    let height = i64::from(window.bottom) - i64::from(window.top);
    let upper_band_bottom = i64::from(window.top) + height * 28 / 100;
    control.left >= window.left
        && control.right <= window.right
        && control.top >= window.top
        && i64::from(control.bottom) <= upper_band_bottom
}

fn finish_scan(
    target: TargetKind,
    provider_window_found: bool,
    observations: Vec<ObservedControl>,
) -> ClientSelectionDetection {
    if provider_window_found {
        extract_candidates(target, &observations)
    } else {
        ClientSelectionDetection::failed(ClientSelectionStatus::NotRunning)
    }
}

pub(super) fn detection_from_scan(
    result: Result<ClientSelectionDetection, ScanFailure>,
) -> ClientSelectionDetection {
    match result {
        Ok(detection) => detection,
        Err(ScanFailure::TimedOut) => {
            ClientSelectionDetection::failed(ClientSelectionStatus::TimedOut)
        }
        Err(ScanFailure::Unavailable) => {
            ClientSelectionDetection::failed(ClientSelectionStatus::Failed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{RECT, RPC_E_CHANGED_MODE, S_FALSE, S_OK};

    #[test]
    fn node_513_is_never_visited() {
        let started = Instant::now();
        let mut budget = TraversalBudget::new(512, 24, started + Duration::from_secs(1));
        let mut visited = 0;

        for _ in 0..513 {
            if budget.try_visit(0, started).is_err() {
                break;
            }
            visited += 1;
        }

        assert_eq!(visited, 512);
        assert_eq!(budget.try_visit(0, started), Err(ScanFailure::TimedOut));
    }

    #[test]
    fn twenty_fifth_visible_external_window_is_rejected() {
        assert_eq!(reserve_window_slot(0), Ok(()));
        assert_eq!(reserve_window_slot(23), Ok(()));
        assert_eq!(reserve_window_slot(24), Err(ScanFailure::TimedOut));
    }

    #[test]
    fn depth_25_is_never_descended_into() {
        let started = Instant::now();
        let mut budget = TraversalBudget::new(512, 24, started + Duration::from_secs(1));
        let mut deepest_visited = None;

        for depth in 0..=25 {
            if budget.try_visit(depth, started).is_err() {
                break;
            }
            deepest_visited = Some(depth);
        }

        assert_eq!(deepest_visited, Some(24));
        assert_eq!(budget.try_visit(25, started), Err(ScanFailure::TimedOut));
    }

    #[test]
    fn exhausted_deadline_maps_to_timed_out() {
        let started = Instant::now();
        let mut budget = TraversalBudget::new(512, 24, started);

        assert_eq!(budget.try_visit(0, started), Err(ScanFailure::TimedOut));
        assert_eq!(
            detection_from_scan(Err(ScanFailure::TimedOut)).status,
            ClientSelectionStatus::TimedOut
        );
    }

    #[test]
    fn empty_allowlisted_window_set_maps_to_not_running() {
        let detection = finish_scan(TargetKind::ChatGptClient, false, Vec::new());

        assert_eq!(detection.status, ClientSelectionStatus::NotRunning);
        assert!(detection.candidates.is_empty());
    }

    #[test]
    fn com_success_codes_are_accepted_and_failures_are_unavailable() {
        assert_eq!(com_initialization_result(S_OK), Ok(()));
        assert_eq!(com_initialization_result(S_FALSE), Ok(()));
        assert_eq!(
            com_initialization_result(RPC_E_CHANGED_MODE),
            Err(ScanFailure::Unavailable)
        );
    }

    #[test]
    fn only_selector_controls_fully_inside_the_upper_band_are_eligible() {
        let window = RECT {
            left: 0,
            top: 100,
            right: 1000,
            bottom: 1100,
        };
        let safe = RECT {
            left: 10,
            top: 110,
            right: 200,
            bottom: 379,
        };
        let too_low = RECT {
            bottom: 381,
            ..safe
        };

        assert!(eligible_selector_bounds(ControlKind::Button, safe, window));
        assert!(!eligible_selector_bounds(
            ControlKind::Document,
            safe,
            window
        ));
        assert!(!eligible_selector_bounds(
            ControlKind::Button,
            too_low,
            window
        ));
    }

    #[test]
    fn labels_longer_than_120_characters_are_discarded() {
        assert!(bounded_raw_control(ControlKind::Button, "x".repeat(120)).is_some());
        assert!(bounded_raw_control(ControlKind::Button, "x".repeat(121)).is_none());
    }
}
