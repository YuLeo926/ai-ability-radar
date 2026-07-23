use super::{
    extract_candidates, observe_window_controls, ClientSelectionDetection, ClientSelectionStatus,
    ControlKind, ObservedControl, ProviderFamily, RawControl, WindowIdentity,
};
use ability_core::TargetKind;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use windows::core::{BOOL, HRESULT, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, SetLastError, APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER,
    ERROR_SUCCESS, HANDLE, HWND, LPARAM, RECT, STILL_ACTIVE, WIN32_ERROR,
};
use windows::Win32::Storage::Packaging::Appx::{
    GetPackageFamilyName, PACKAGE_FAMILY_NAME_MAX_LENGTH,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
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
const MAX_LABEL_UTF16_UNITS: usize = MAX_LABEL_CHARS * 2;
const SCAN_BUDGET: Duration = Duration::from_millis(1_200);
const MAX_PROCESS_PATH_CHARS: usize = 32_768;
const MAX_TITLE_CHARS: usize = 512;
const TITLE_SENTINEL: u16 = 0xffff;
const MAX_PACKAGE_FAMILY_BUFFER_CHARS: usize = PACKAGE_FAMILY_NAME_MAX_LENGTH as usize + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScanFailure {
    TimedOut,
    Unavailable,
}

trait ScanClock {
    fn now(&self) -> Instant;
}

struct SystemClock;

impl ScanClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

struct DeadlineGate<'a, C> {
    deadline: Instant,
    clock: &'a C,
}

impl<'a, C: ScanClock> DeadlineGate<'a, C> {
    fn new(deadline: Instant, clock: &'a C) -> Self {
        Self { deadline, clock }
    }

    fn check(&self) -> Result<(), ScanFailure> {
        (self.clock.now() < self.deadline)
            .then_some(())
            .ok_or(ScanFailure::TimedOut)
    }

    fn call<T>(&self, native: impl FnOnce() -> Result<T, ScanFailure>) -> Result<T, ScanFailure> {
        self.check()?;
        let result = native();
        self.check()?;
        result
    }
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
    process_id: u32,
    process: ProcessHandle,
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
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    if Instant::now() >= context.deadline {
        context.stopped = Some(ScanFailure::TimedOut);
        return false.into();
    }
    if !visible {
        return true.into();
    }

    let mut process_id = 0;
    // SAFETY: `process_id` is writable for the duration of this call.
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if Instant::now() >= context.deadline {
        context.stopped = Some(ScanFailure::TimedOut);
        return false.into();
    }
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
    let clock = SystemClock;
    let gate = DeadlineGate::new(deadline, &clock);
    let enumeration = gate.call(|| enumerate_visible_windows(deadline))?;
    let handles = require_complete_enumeration(enumeration)?;
    let expected_provider = provider_for_target(target).ok_or(ScanFailure::Unavailable)?;
    let mut windows = Vec::new();

    for (hwnd, process_id) in handles {
        gate.check()?;
        let Some((identity, process)) = read_window_identity(hwnd, process_id, &gate)? else {
            continue;
        };
        if let Some(provider) = super::preliminary_provider(&identity) {
            if provider == expected_provider {
                windows.push(ProviderWindow {
                    hwnd,
                    process_id,
                    process,
                    provider,
                    identity,
                });
            }
        }
    }

    if windows.is_empty() {
        gate.check()?;
        return Ok(finish_scan(target, false, Vec::new()));
    }

    let _com = initialize_com(&gate)?;
    let automation: IUIAutomation = gate.call(|| {
        // SAFETY: COM is initialized above, no aggregation is requested, and
        // `IUIAutomation` owns the returned interface pointer.
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| ScanFailure::Unavailable)
    })?;
    let walker = gate.call(|| {
        // SAFETY: `automation` is a live UI Automation interface.
        unsafe { automation.RawViewWalker() }.map_err(|_| ScanFailure::Unavailable)
    })?;
    let mut budget = TraversalBudget::new(MAX_NODES, MAX_DEPTH, deadline);
    let mut observations = Vec::new();

    for window in windows {
        gate.check()?;
        let controls = collect_window_controls(&automation, &walker, &window, &mut budget, &gate)?;
        if let Some(checked) =
            observe_window_controls(window.provider, &controls, &window.identity.title_hint)
        {
            observations.extend(checked);
        }
    }

    gate.check()?;
    Ok(finish_scan(target, true, observations))
}

fn require_complete_enumeration(
    outcome: EnumerationOutcome,
) -> Result<Vec<(HWND, u32)>, ScanFailure> {
    match outcome.stopped {
        Some(failure) => Err(failure),
        None => Ok(outcome.handles),
    }
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

fn read_window_identity<C: ScanClock>(
    hwnd: HWND,
    process_id: u32,
    gate: &DeadlineGate<'_, C>,
) -> Result<Option<(WindowIdentity, ProcessHandle)>, ScanFailure> {
    match read_window_identity_inner(hwnd, process_id, gate) {
        Ok(identity) => Ok(Some(identity)),
        Err(ScanFailure::Unavailable) => Ok(None),
        Err(ScanFailure::TimedOut) => Err(ScanFailure::TimedOut),
    }
}

fn read_window_identity_inner<C: ScanClock>(
    hwnd: HWND,
    process_id: u32,
    gate: &DeadlineGate<'_, C>,
) -> Result<(WindowIdentity, ProcessHandle), ScanFailure> {
    if process_id == 0 {
        return Err(ScanFailure::Unavailable);
    }
    let process = gate.call(|| {
        // SAFETY: `process_id` came from `GetWindowThreadProcessId`. The
        // successful result is immediately placed in its owning RAII guard.
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
            .map(ProcessHandle)
            .map_err(|_| ScanFailure::Unavailable)
    })?;
    verify_window_process_binding(gate, hwnd, process_id, &process)?;
    let executable_path = process_image_path(process.0, gate)?;
    let process_name = executable_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_owned();
    if process_name.is_empty() {
        return Err(ScanFailure::Unavailable);
    }
    let package_family = process_package_family(process.0, gate)?;

    verify_window_process_binding(gate, hwnd, process_id, &process)?;
    let first_title = window_title(hwnd, gate)?;
    verify_window_process_binding(gate, hwnd, process_id, &process)?;
    let second_title = window_title(hwnd, gate)?;
    verify_window_process_binding(gate, hwnd, process_id, &process)?;
    let title_hint = ensure_consistent_title(first_title, second_title)?;

    Ok((
        WindowIdentity {
            process_name,
            package_family,
            title_hint,
            executable_path: Some(executable_path),
        },
        process,
    ))
}

fn process_image_path<C: ScanClock>(
    handle: HANDLE,
    gate: &DeadlineGate<'_, C>,
) -> Result<String, ScanFailure> {
    let mut buffer = vec![0_u16; MAX_PROCESS_PATH_CHARS];
    let mut length = u32::try_from(buffer.len()).map_err(|_| ScanFailure::Unavailable)?;
    gate.call(|| {
        // SAFETY: The buffer is writable for `length` UTF-16 code units and
        // the process handle remains live for the call.
        unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        }
        .map_err(|_| ScanFailure::Unavailable)
    })?;
    let used = usize::try_from(length).map_err(|_| ScanFailure::Unavailable)?;
    let content = buffer.get(..used).ok_or(ScanFailure::Unavailable)?;
    if content.is_empty() || content.contains(&0) {
        return Err(ScanFailure::Unavailable);
    }
    String::from_utf16(content).map_err(|_| ScanFailure::Unavailable)
}

fn process_package_family<C: ScanClock>(
    handle: HANDLE,
    gate: &DeadlineGate<'_, C>,
) -> Result<Option<String>, ScanFailure> {
    let mut length = 0_u32;
    let status = gate.call(|| {
        // SAFETY: A null output buffer intentionally requests the required
        // size, and `length` is writable for the duration of the call.
        Ok(unsafe { GetPackageFamilyName(handle, &mut length, None) })
    })?;
    if status == APPMODEL_ERROR_NO_PACKAGE {
        return Ok(None);
    }
    if status != ERROR_INSUFFICIENT_BUFFER || length == 0 {
        return Err(ScanFailure::Unavailable);
    }

    let allocated = usize::try_from(length).map_err(|_| ScanFailure::Unavailable)?;
    validate_package_family_length(allocated)?;
    let mut buffer = vec![0_u16; allocated];
    let status = gate.call(|| {
        // SAFETY: The buffer has the exact size reported by the first call and
        // the process handle remains live.
        Ok(unsafe { GetPackageFamilyName(handle, &mut length, Some(PWSTR(buffer.as_mut_ptr()))) })
    })?;
    if status != ERROR_SUCCESS {
        return Err(ScanFailure::Unavailable);
    }
    let returned = usize::try_from(length).map_err(|_| ScanFailure::Unavailable)?;
    decode_package_family(allocated, returned, &buffer).map(Some)
}

fn window_title<C: ScanClock>(
    hwnd: HWND,
    gate: &DeadlineGate<'_, C>,
) -> Result<String, ScanFailure> {
    let reported = gate.call(|| {
        // SAFETY: `hwnd` is bound to the retained process immediately before
        // this title snapshot.
        Ok(unsafe { GetWindowTextLengthW(hwnd) })
    })?;
    let reported = usize::try_from(reported).map_err(|_| ScanFailure::Unavailable)?;
    if reported > MAX_TITLE_CHARS {
        return Err(ScanFailure::Unavailable);
    }
    let capacity = reported.checked_add(2).ok_or(ScanFailure::Unavailable)?;
    let mut buffer = vec![TITLE_SENTINEL; capacity];
    gate.check()?;
    // SAFETY: Clearing last error, reading the title, and capturing last error
    // form one atomic Win32 error-observation sequence. The deadline is checked
    // immediately before it and after the potentially blocking title read.
    let (copied, last_error) = unsafe {
        SetLastError(ERROR_SUCCESS);
        let copied = GetWindowTextW(hwnd, &mut buffer);
        (copied, GetLastError())
    };
    gate.check()?;
    let after_reported = gate.call(|| {
        // SAFETY: `hwnd` is still the bound candidate window handle.
        Ok(unsafe { GetWindowTextLengthW(hwnd) })
    })?;
    let after_reported = usize::try_from(after_reported).map_err(|_| ScanFailure::Unavailable)?;
    decode_window_title(reported, copied, last_error, after_reported, &buffer)
}

fn ensure_consistent_title(first: String, second: String) -> Result<String, ScanFailure> {
    (first == second)
        .then_some(first)
        .ok_or(ScanFailure::Unavailable)
}

fn verify_window_process_binding<C: ScanClock>(
    gate: &DeadlineGate<'_, C>,
    hwnd: HWND,
    expected_process_id: u32,
    process: &ProcessHandle,
) -> Result<(), ScanFailure> {
    if expected_process_id == 0 || hwnd.0.is_null() || process.0.is_invalid() {
        return Err(ScanFailure::Unavailable);
    }

    let (thread_id, window_process_id) = gate.call(|| {
        let mut process_id = 0;
        // SAFETY: `process_id` is writable and `hwnd` is checked again at this
        // native boundary rather than trusted from enumeration.
        let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
        Ok((thread_id, process_id))
    })?;
    if thread_id == 0 || window_process_id != expected_process_id {
        return Err(ScanFailure::Unavailable);
    }

    let handle_process_id = gate.call(|| {
        // SAFETY: `process` owns a live read-only process handle.
        Ok(unsafe { GetProcessId(process.0) })
    })?;
    if handle_process_id == 0 || handle_process_id != expected_process_id {
        return Err(ScanFailure::Unavailable);
    }

    let mut exit_code = 0_u32;
    gate.call(|| {
        // SAFETY: `exit_code` is writable and `process` owns the handle.
        unsafe { GetExitCodeProcess(process.0, &mut exit_code) }
            .map_err(|_| ScanFailure::Unavailable)
    })?;
    validate_process_binding(
        expected_process_id,
        thread_id,
        window_process_id,
        handle_process_id,
        exit_code,
    )
}

fn validate_process_binding(
    expected_process_id: u32,
    thread_id: u32,
    window_process_id: u32,
    handle_process_id: u32,
    exit_code: u32,
) -> Result<(), ScanFailure> {
    (expected_process_id != 0
        && thread_id != 0
        && window_process_id == expected_process_id
        && handle_process_id == expected_process_id
        && exit_code == STILL_ACTIVE.0 as u32)
        .then_some(())
        .ok_or(ScanFailure::Unavailable)
}

fn decode_window_title(
    reported: usize,
    copied: i32,
    last_error: WIN32_ERROR,
    after_reported: usize,
    buffer: &[u16],
) -> Result<String, ScanFailure> {
    if reported > MAX_TITLE_CHARS || copied < 0 || buffer.len() != reported.saturating_add(2) {
        return Err(ScanFailure::Unavailable);
    }
    let copied = usize::try_from(copied).map_err(|_| ScanFailure::Unavailable)?;
    if (copied == 0 && last_error != ERROR_SUCCESS)
        || copied > reported
        || copied >= buffer.len().saturating_sub(1)
        || after_reported != copied
        || buffer.get(copied) != Some(&0)
        || buffer.get(copied + 1) != Some(&TITLE_SENTINEL)
    {
        return Err(ScanFailure::Unavailable);
    }
    let content = buffer.get(..copied).ok_or(ScanFailure::Unavailable)?;
    if content.contains(&0) {
        return Err(ScanFailure::Unavailable);
    }
    String::from_utf16(content).map_err(|_| ScanFailure::Unavailable)
}

fn validate_package_family_length(length: usize) -> Result<(), ScanFailure> {
    (1..=MAX_PACKAGE_FAMILY_BUFFER_CHARS)
        .contains(&length)
        .then_some(())
        .ok_or(ScanFailure::Unavailable)
}

fn decode_package_family(
    allocated: usize,
    returned: usize,
    buffer: &[u16],
) -> Result<String, ScanFailure> {
    validate_package_family_length(allocated)?;
    if buffer.len() != allocated || returned == 0 || returned > allocated {
        return Err(ScanFailure::Unavailable);
    }
    let returned = buffer.get(..returned).ok_or(ScanFailure::Unavailable)?;
    let content = returned
        .strip_suffix(&[0])
        .ok_or(ScanFailure::Unavailable)?;
    if content.contains(&0) {
        return Err(ScanFailure::Unavailable);
    }
    String::from_utf16(content).map_err(|_| ScanFailure::Unavailable)
}

fn initialize_com<C: ScanClock>(gate: &DeadlineGate<'_, C>) -> Result<ComGuard, ScanFailure> {
    gate.check()?;
    // SAFETY: This runs on the dedicated blocking worker. A successful call is
    // placed in `guard` before the post-call deadline check, so timeout cleanup
    // still balances COM initialization.
    let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let guard = result.is_ok().then_some(ComGuard);
    gate.check()?;
    com_initialization_result(result)?;
    guard.ok_or(ScanFailure::Unavailable)
}

fn com_initialization_result(result: HRESULT) -> Result<(), ScanFailure> {
    result.ok().map_err(|_| ScanFailure::Unavailable)
}

fn collect_window_controls<C: ScanClock>(
    automation: &IUIAutomation,
    walker: &IUIAutomationTreeWalker,
    window: &ProviderWindow,
    budget: &mut TraversalBudget,
    gate: &DeadlineGate<'_, C>,
) -> Result<Vec<RawControl>, ScanFailure> {
    let (window_bounds, root) = acquire_verified_root(automation, window, gate)?;
    let mut queue = VecDeque::from([(root, 0_usize)]);
    let mut controls = Vec::new();
    let mut traversal_truncated = false;

    while let Some((element, depth)) = queue.pop_front() {
        budget.try_visit(depth, gate.clock.now())?;
        collect_element_name(&element, window_bounds, &mut controls, gate)?;
        gate.check()?;

        let mut child = gate.call(|| {
            // SAFETY: `walker` and `element` are live COM interfaces on the
            // initialized worker thread.
            optional_element(unsafe { walker.GetFirstChildElement(&element) })
        })?;
        while let Some(current) = child {
            gate.check()?;
            let child_depth = depth.saturating_add(1);
            if !budget.can_enqueue(queue.len(), child_depth) {
                traversal_truncated = true;
                break;
            }
            queue.push_back((current.clone(), child_depth));
            child = gate.call(|| {
                // SAFETY: Both interfaces are live and stay on this COM worker.
                optional_element(unsafe { walker.GetNextSiblingElement(&current) })
            })?;
        }
    }

    if traversal_truncated {
        Err(ScanFailure::TimedOut)
    } else {
        Ok(controls)
    }
}

fn acquire_verified_root<C: ScanClock>(
    automation: &IUIAutomation,
    window: &ProviderWindow,
    gate: &DeadlineGate<'_, C>,
) -> Result<(RECT, IUIAutomationElement), ScanFailure> {
    acquire_verified_root_with(
        gate,
        window.process_id,
        || verify_window_process_binding(gate, window.hwnd, window.process_id, &window.process),
        || {
            let mut bounds = RECT::default();
            // SAFETY: The HWND has just been rebound to the retained process
            // and `bounds` is writable for this call.
            unsafe { GetWindowRect(window.hwnd, &mut bounds) }
                .map_err(|_| ScanFailure::Unavailable)?;
            Ok(bounds)
        },
        || {
            // SAFETY: `automation` is live and the HWND is rebound immediately
            // before this UIA root creation call.
            unsafe { automation.ElementFromHandle(window.hwnd) }
                .map_err(|_| ScanFailure::Unavailable)
        },
        |root| {
            // SAFETY: `root` is the live UIA element returned above.
            unsafe { root.CurrentProcessId() }.map_err(|_| ScanFailure::Unavailable)
        },
    )
}

fn acquire_verified_root_with<C, Root, Verify, ReadRect, CreateRoot, ReadRootProcess>(
    gate: &DeadlineGate<'_, C>,
    expected_process_id: u32,
    mut verify_window_binding: Verify,
    read_window_rect: ReadRect,
    create_root: CreateRoot,
    read_root_process_id: ReadRootProcess,
) -> Result<(RECT, Root), ScanFailure>
where
    C: ScanClock,
    Verify: FnMut() -> Result<(), ScanFailure>,
    ReadRect: FnOnce() -> Result<RECT, ScanFailure>,
    CreateRoot: FnOnce() -> Result<Root, ScanFailure>,
    ReadRootProcess: FnOnce(&Root) -> Result<i32, ScanFailure>,
{
    if expected_process_id == 0 {
        return Err(ScanFailure::Unavailable);
    }
    verify_window_binding()?;
    let bounds = gate.call(read_window_rect)?;
    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return Err(ScanFailure::Unavailable);
    }

    verify_window_binding()?;
    let root = gate.call(create_root)?;
    let root_process_id = gate.call(|| read_root_process_id(&root))?;
    let root_process_id = u32::try_from(root_process_id).map_err(|_| ScanFailure::Unavailable)?;
    if root_process_id == 0 || root_process_id != expected_process_id {
        return Err(ScanFailure::Unavailable);
    }
    verify_window_binding()?;
    Ok((bounds, root))
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

fn collect_element_name<C: ScanClock>(
    element: &IUIAutomationElement,
    window_bounds: RECT,
    controls: &mut Vec<RawControl>,
    gate: &DeadlineGate<'_, C>,
) -> Result<(), ScanFailure> {
    let control_type = gate.call(|| {
        // SAFETY: `element` is a live UI Automation element on the COM worker.
        unsafe { element.CurrentControlType() }.map_err(|_| ScanFailure::Unavailable)
    })?;
    let kind = control_kind(control_type);
    if !matches!(
        kind,
        ControlKind::Button | ControlKind::ComboBox | ControlKind::MenuItem
    ) {
        return Ok(());
    }
    let bounds = gate.call(|| {
        // SAFETY: Bounding rectangles are read only after the element has been
        // identified as one of the three allowlisted selector control types.
        unsafe { element.CurrentBoundingRectangle() }.map_err(|_| ScanFailure::Unavailable)
    })?;
    if !eligible_selector_bounds(kind, bounds, window_bounds) {
        return Ok(());
    }
    let name = gate.call(|| {
        // SAFETY: CurrentName is the sole label read and is called only for an
        // allowlisted selector fully inside the upper provider-window band.
        unsafe { element.CurrentName() }.map_err(|_| ScanFailure::Unavailable)
    })?;
    gate.check()?;
    if let Some(control) = decode_bounded_label(kind, &name) {
        controls.push(control);
    }
    Ok(())
}

fn decode_bounded_label(kind: ControlKind, utf16: &[u16]) -> Option<RawControl> {
    if utf16.len() > MAX_LABEL_UTF16_UNITS {
        return None;
    }
    let name = String::from_utf16(utf16).ok()?;
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
    use std::cell::Cell;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{
        ERROR_INVALID_WINDOW_HANDLE, ERROR_SUCCESS, RECT, RPC_E_CHANGED_MODE, S_FALSE, S_OK,
    };

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
        let accepted = "x".repeat(120).encode_utf16().collect::<Vec<_>>();
        let rejected = "x".repeat(121).encode_utf16().collect::<Vec<_>>();
        assert!(decode_bounded_label(ControlKind::Button, &accepted).is_some());
        assert!(decode_bounded_label(ControlKind::Button, &rejected).is_none());
    }

    #[test]
    fn utf16_label_boundary_accepts_120_ascii_and_120_surrogate_pairs() {
        let ascii = "x".repeat(120).encode_utf16().collect::<Vec<_>>();
        let astral = "😀".repeat(120).encode_utf16().collect::<Vec<_>>();

        assert_eq!(
            decode_bounded_label(ControlKind::Button, &ascii)
                .unwrap()
                .name,
            "x".repeat(120)
        );
        assert_eq!(astral.len(), 240);
        assert_eq!(
            decode_bounded_label(ControlKind::Button, &astral)
                .unwrap()
                .name
                .chars()
                .count(),
            120
        );
    }

    #[test]
    fn utf16_label_boundary_rejects_121_scalars_over_240_units_and_lone_surrogates() {
        let scalars_121 = "x".repeat(121).encode_utf16().collect::<Vec<_>>();
        let units_242 = "😀".repeat(121).encode_utf16().collect::<Vec<_>>();

        assert!(decode_bounded_label(ControlKind::Button, &scalars_121).is_none());
        assert!(decode_bounded_label(ControlKind::Button, &units_242).is_none());
        assert!(decode_bounded_label(ControlKind::Button, &[0xd800]).is_none());
    }

    fn title_buffer(value: &[u16], capacity: usize) -> Vec<u16> {
        let mut buffer = vec![TITLE_SENTINEL; capacity];
        buffer[..value.len()].copy_from_slice(value);
        buffer[value.len()] = 0;
        buffer
    }

    #[test]
    fn title_growth_or_a_full_sentinel_buffer_is_rejected() {
        let grown = "ClaudeX".encode_utf16().collect::<Vec<_>>();
        let grown_buffer = title_buffer(&grown, 8);
        assert_eq!(
            decode_window_title(6, 7, ERROR_SUCCESS, 7, &grown_buffer),
            Err(ScanFailure::Unavailable)
        );

        let invalid_utf16 = title_buffer(&[0xd800], 3);
        assert_eq!(
            decode_window_title(1, 1, ERROR_SUCCESS, 1, &invalid_utf16),
            Err(ScanFailure::Unavailable)
        );

        let full = "ChatGPT".encode_utf16().collect::<Vec<_>>();
        let full_buffer = title_buffer(&full, 8);
        assert_eq!(
            decode_window_title(6, 7, ERROR_SUCCESS, 7, &full_buffer),
            Err(ScanFailure::Unavailable)
        );
    }

    #[test]
    fn zero_length_title_distinguishes_empty_from_native_failure() {
        let empty = title_buffer(&[], 2);
        assert_eq!(
            decode_window_title(0, 0, ERROR_SUCCESS, 0, &empty),
            Ok(String::new())
        );
        assert_eq!(
            decode_window_title(0, 0, ERROR_INVALID_WINDOW_HANDLE, 0, &empty),
            Err(ScanFailure::Unavailable)
        );
    }

    #[test]
    fn overlong_reported_title_is_rejected_without_using_a_prefix() {
        let buffer = title_buffer(&[], 2);
        assert_eq!(
            decode_window_title(MAX_TITLE_CHARS + 1, 0, ERROR_SUCCESS, 0, &buffer),
            Err(ScanFailure::Unavailable)
        );
    }

    #[test]
    fn package_family_rejects_oversize_overflow_and_missing_terminator() {
        assert_eq!(
            validate_package_family_length(MAX_PACKAGE_FAMILY_BUFFER_CHARS + 1),
            Err(ScanFailure::Unavailable)
        );

        let valid = "OpenAI.Codex_2p2nqsd0c76g0"
            .encode_utf16()
            .chain([0])
            .collect::<Vec<_>>();
        assert_eq!(
            decode_package_family(valid.len(), valid.len() + 1, &valid),
            Err(ScanFailure::Unavailable)
        );

        let mut missing_terminator = valid.clone();
        *missing_terminator.last_mut().unwrap() = u16::from(b'x');
        assert_eq!(
            decode_package_family(
                missing_terminator.len(),
                missing_terminator.len(),
                &missing_terminator,
            ),
            Err(ScanFailure::Unavailable)
        );

        let interior_nul = [u16::from(b'a'), 0, u16::from(b'b'), 0];
        assert_eq!(
            decode_package_family(interior_nul.len(), interior_nul.len(), &interior_nul),
            Err(ScanFailure::Unavailable)
        );
        let invalid_utf16 = [0xd800, 0];
        assert_eq!(
            decode_package_family(invalid_utf16.len(), invalid_utf16.len(), &invalid_utf16),
            Err(ScanFailure::Unavailable)
        );
    }

    struct SyntheticClock {
        before: Instant,
        deadline: Instant,
        valid_checks: usize,
        checks: Cell<usize>,
    }

    impl ScanClock for SyntheticClock {
        fn now(&self) -> Instant {
            let check = self.checks.get();
            self.checks.set(check + 1);
            if check < self.valid_checks {
                self.before
            } else {
                self.deadline
            }
        }
    }

    #[test]
    fn every_native_stage_crossing_the_deadline_blocks_all_later_calls() {
        const STAGES: usize = 18;
        for crossing_stage in 0..STAGES {
            let before = Instant::now();
            let clock = SyntheticClock {
                before,
                deadline: before + Duration::from_millis(1),
                valid_checks: crossing_stage * 2 + 1,
                checks: Cell::new(0),
            };
            let gate = DeadlineGate::new(clock.deadline, &clock);
            let calls = (0..STAGES).map(|_| Cell::new(0)).collect::<Vec<_>>();

            let result = (0..STAGES).try_for_each(|stage| {
                gate.call(|| {
                    calls[stage].set(calls[stage].get() + 1);
                    Ok(())
                })
            });

            assert_eq!(result, Err(ScanFailure::TimedOut), "{crossing_stage}");
            for (stage, count) in calls.iter().enumerate() {
                assert_eq!(
                    count.get(),
                    usize::from(stage <= crossing_stage),
                    "crossing={crossing_stage}, stage={stage}"
                );
            }
        }
    }

    #[test]
    fn stopped_enumeration_never_starts_identity_calls() {
        let identity_calls = Cell::new(0);
        let outcome = EnumerationOutcome {
            handles: Vec::new(),
            stopped: Some(ScanFailure::TimedOut),
        };

        let result = require_complete_enumeration(outcome).map(|_| {
            identity_calls.set(identity_calls.get() + 1);
        });

        assert_eq!(result, Err(ScanFailure::TimedOut));
        assert_eq!(identity_calls.get(), 0);
    }

    #[test]
    fn pid_change_between_identity_and_uia_prevents_every_property_call() {
        let before = Instant::now();
        let clock = SyntheticClock {
            before,
            deadline: before + Duration::from_secs(1),
            valid_checks: usize::MAX,
            checks: Cell::new(0),
        };
        let gate = DeadlineGate::new(clock.deadline, &clock);
        let binding_checks = Cell::new(0);
        let replacement_window_process_id = Cell::new(99_u32);
        let rect_calls = Cell::new(0);
        let root_calls = Cell::new(0);
        let root_pid_calls = Cell::new(0);
        let selector_property_calls = Cell::new(0);

        let result = acquire_verified_root_with(
            &gate,
            41,
            || {
                binding_checks.set(binding_checks.get() + 1);
                (replacement_window_process_id.get() == 41)
                    .then_some(())
                    .ok_or(ScanFailure::Unavailable)
            },
            || {
                rect_calls.set(rect_calls.get() + 1);
                Ok(RECT {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 100,
                })
            },
            || {
                root_calls.set(root_calls.get() + 1);
                Ok(())
            },
            |_| {
                root_pid_calls.set(root_pid_calls.get() + 1);
                Ok(41)
            },
        )
        .map(|_| {
            selector_property_calls.set(selector_property_calls.get() + 1);
        });

        assert_eq!(result, Err(ScanFailure::Unavailable));
        assert_eq!(binding_checks.get(), 1);
        assert_eq!(rect_calls.get(), 0);
        assert_eq!(root_calls.get(), 0);
        assert_eq!(root_pid_calls.get(), 0);
        assert_eq!(selector_property_calls.get(), 0);
    }

    #[test]
    fn mismatched_uia_root_owner_prevents_selector_property_calls() {
        let before = Instant::now();
        let clock = SyntheticClock {
            before,
            deadline: before + Duration::from_secs(1),
            valid_checks: usize::MAX,
            checks: Cell::new(0),
        };
        let gate = DeadlineGate::new(clock.deadline, &clock);
        let binding_checks = Cell::new(0);
        let selector_property_calls = Cell::new(0);

        let result = acquire_verified_root_with(
            &gate,
            41,
            || {
                binding_checks.set(binding_checks.get() + 1);
                Ok(())
            },
            || {
                Ok(RECT {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 100,
                })
            },
            || Ok(()),
            |_| Ok(99),
        )
        .map(|_| {
            selector_property_calls.set(selector_property_calls.get() + 1);
        });

        assert_eq!(result, Err(ScanFailure::Unavailable));
        assert_eq!(binding_checks.get(), 2);
        assert_eq!(selector_property_calls.get(), 0);
    }

    #[test]
    fn zero_reused_or_exited_process_bindings_fail_closed() {
        let active = STILL_ACTIVE.0 as u32;
        assert_eq!(validate_process_binding(41, 7, 41, 41, active), Ok(()));

        for binding in [
            (0, 7, 41, 41, active),
            (41, 0, 41, 41, active),
            (41, 7, 0, 41, active),
            (41, 7, 99, 41, active),
            (41, 7, 41, 0, active),
            (41, 7, 41, 99, active),
            (41, 7, 41, 41, 0),
        ] {
            assert_eq!(
                validate_process_binding(binding.0, binding.1, binding.2, binding.3, binding.4),
                Err(ScanFailure::Unavailable),
                "{binding:?}"
            );
        }
    }

    #[test]
    fn same_length_title_changes_are_rejected() {
        assert_eq!(
            ensure_consistent_title("ChatGPT".to_owned(), "ClaudeX".to_owned()),
            Err(ScanFailure::Unavailable)
        );
    }
}
