//! Small, ownership-checked bridge to Apple's public Accessibility APIs.
//! CF objects are deliberately !Send/!Sync and used only on the AX worker.

use super::semantic::*;
use super::*;
use std::ffi::{c_char, c_void, CString};
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Instant;

type Ref = *const c_void;
type AxError = i32;
const UTF8: u32 = 0x08000100;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateSystemWide() -> Ref;
    fn AXUIElementCreateApplication(pid: i32) -> Ref;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementGetPid(element: Ref, pid: *mut i32) -> AxError;
    fn AXUIElementSetMessagingTimeout(element: Ref, seconds: f32) -> AxError;
    fn AXUIElementCopyAttributeValue(element: Ref, attribute: Ref, value: *mut Ref) -> AxError;
    fn AXUIElementIsAttributeSettable(element: Ref, attribute: Ref, settable: *mut u8) -> AxError;
    fn AXUIElementCopyActionNames(element: Ref, value: *mut Ref) -> AxError;
    fn AXUIElementSetAttributeValue(element: Ref, attribute: Ref, value: Ref) -> AxError;
    fn AXUIElementPerformAction(element: Ref, action: Ref) -> AxError;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: Ref);
    fn CFRetain(value: Ref) -> Ref;
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFEqual(first: Ref, second: Ref) -> u8;
    fn CFStringGetTypeID() -> usize;
    fn CFStringCreateWithBytes(
        allocator: Ref,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> Ref;
    fn CFStringGetLength(value: Ref) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(value: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(value: Ref) -> isize;
    fn CFArrayGetValueAtIndex(value: Ref, index: isize) -> Ref;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: Ref) -> u8;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(value: Ref, number_type: isize, result: *mut c_void) -> u8;
    static kCFBooleanTrue: Ref;
    static kCFBooleanFalse: Ref;
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}
#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

struct Pool(*mut c_void);
impl Pool {
    fn new() -> Self {
        Self(unsafe { objc_autoreleasePoolPush() })
    }
}
impl Drop for Pool {
    fn drop(&mut self) {
        unsafe {
            objc_autoreleasePoolPop(self.0);
        }
    }
}

struct Owned(Ref, PhantomData<Rc<()>>);
impl Owned {
    unsafe fn take(raw: Ref) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw, PhantomData))
    }
    fn string(value: &str) -> Self {
        // CF allocation failure is exceptional; null is safely rejected by
        // every attribute operation rather than passed into CoreFoundation.
        Self(
            unsafe {
                CFStringCreateWithBytes(
                    std::ptr::null(),
                    value.as_ptr(),
                    value.len() as isize,
                    UTF8,
                    0,
                )
            },
            PhantomData,
        )
    }
    fn text(&self) -> Option<String> {
        unsafe {
            if self.0.is_null() || CFGetTypeID(self.0) != CFStringGetTypeID() {
                return None;
            }
            let length = CFStringGetLength(self.0);
            // Accessibility values can be entire documents. Do not allocate
            // unbounded buffers, and never expose their text in observations.
            if length > 1_048_576 {
                return None;
            }
            let size = CFStringGetMaximumSizeForEncoding(length, UTF8).checked_add(1)?;
            if size <= 0 {
                return None;
            }
            let mut bytes = vec![0; size as usize];
            if CFStringGetCString(self.0, bytes.as_mut_ptr().cast(), size, UTF8) == 0 {
                return None;
            }
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            String::from_utf8(bytes[..end].to_vec()).ok()
        }
    }
    fn boolean(&self) -> Option<bool> {
        unsafe {
            if CFGetTypeID(self.0) == CFBooleanGetTypeID() {
                return Some(CFBooleanGetValue(self.0) != 0);
            }
            if CFGetTypeID(self.0) == CFNumberGetTypeID() {
                let mut number = 0_i32;
                if CFNumberGetValue(self.0, 3, (&mut number as *mut i32).cast()) != 0 {
                    // AXValue == 2 is the mixed checkbox state, not "true".
                    return match number {
                        0 => Some(false),
                        1 => Some(true),
                        _ => None,
                    };
                }
            }
            None
        }
    }
    fn array(&self, limit: usize) -> Vec<Self> {
        unsafe {
            if CFGetTypeID(self.0) != CFArrayGetTypeID() {
                return vec![];
            }
            (0..CFArrayGetCount(self.0).min(limit as isize))
                .filter_map(|index| {
                    let value = CFArrayGetValueAtIndex(self.0, index);
                    if value.is_null() {
                        None
                    } else {
                        Self::take(CFRetain(value))
                    }
                })
                .collect()
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CFRelease(self.0);
            }
        }
    }
}

/// Public within the crate for the legacy window service. Keep instances on
/// the owning thread; re-acquire by application/window identity for each call.
pub(crate) struct Element(Owned);
impl Element {
    fn from_owned(value: Owned) -> Option<Self> {
        if unsafe { CFGetTypeID(value.0) } != unsafe { AXUIElementGetTypeID() } {
            return None;
        }
        unsafe {
            AXUIElementSetMessagingTimeout(value.0, 0.15);
        }
        Some(Self(value))
    }
    pub(crate) fn application(pid: i32) -> Option<Self> {
        Self::from_owned(unsafe { Owned::take(AXUIElementCreateApplication(pid)) }?)
    }
    pub(crate) fn system() -> Option<Self> {
        Self::from_owned(unsafe { Owned::take(AXUIElementCreateSystemWide()) }?)
    }
    fn attribute(&self, name: &str, deadline: Instant) -> Option<Owned> {
        if Instant::now() >= deadline {
            return None;
        }
        let key = Owned::string(name);
        if key.0.is_null() {
            return None;
        }
        let mut result = std::ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(self.0 .0, key.0, &mut result) };
        let owned = unsafe { Owned::take(result) };
        if status == 0 {
            owned
        } else {
            None
        }
    }
    pub(crate) fn text(&self, name: &str, deadline: Instant) -> Option<String> {
        self.attribute(name, deadline)?
            .text()
            .filter(|s| !s.is_empty())
    }
    pub(crate) fn boolean(&self, name: &str, deadline: Instant) -> Option<bool> {
        self.attribute(name, deadline)?.boolean()
    }
    pub(crate) fn child(&self, name: &str, deadline: Instant) -> Option<Self> {
        Self::from_owned(self.attribute(name, deadline)?)
    }
    pub(crate) fn children(&self, name: &str, limit: usize, deadline: Instant) -> Vec<Self> {
        self.attribute(name, deadline)
            .map(|a| {
                a.array(limit)
                    .into_iter()
                    .filter_map(Self::from_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(crate) fn pid(&self) -> Option<i32> {
        let mut pid = 0;
        (unsafe { AXUIElementGetPid(self.0 .0, &mut pid) } == 0).then_some(pid)
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        unsafe { CFEqual(self.0 .0, other.0 .0) != 0 }
    }
    fn settable(&self, name: &str, deadline: Instant) -> bool {
        if Instant::now() >= deadline {
            return false;
        }
        let key = Owned::string(name);
        let mut result = 0;
        !key.0.is_null()
            && unsafe { AXUIElementIsAttributeSettable(self.0 .0, key.0, &mut result) } == 0
            && result != 0
    }
    fn action_names(&self, deadline: Instant) -> Vec<String> {
        if Instant::now() >= deadline {
            return vec![];
        }
        let mut result = std::ptr::null();
        let status = unsafe { AXUIElementCopyActionNames(self.0 .0, &mut result) };
        let owned = unsafe { Owned::take(result) };
        if status != 0 {
            return vec![];
        }
        owned
            .map(|v| v.array(32).iter().filter_map(Owned::text).collect())
            .unwrap_or_default()
    }
    pub(crate) fn perform(&self, name: &str) -> Result<(), AutomationError> {
        let name = Owned::string(name);
        if name.0.is_null() {
            return Err(AutomationError::Execution(
                "AX action allocation failed".into(),
            ));
        }
        check(unsafe { AXUIElementPerformAction(self.0 .0, name.0) })
    }
    pub(crate) fn set_bool(&self, name: &str, value: bool) -> Result<(), AutomationError> {
        let key = Owned::string(name);
        if key.0.is_null() {
            return Err(AutomationError::Execution(
                "AX attribute allocation failed".into(),
            ));
        }
        check(unsafe {
            AXUIElementSetAttributeValue(
                self.0 .0,
                key.0,
                if value {
                    kCFBooleanTrue
                } else {
                    kCFBooleanFalse
                },
            )
        })
    }
    fn set_text(&self, name: &str, value: &str) -> Result<(), AutomationError> {
        let key = Owned::string(name);
        let value = Owned::string(value);
        if key.0.is_null() || value.0.is_null() {
            return Err(AutomationError::Execution(
                "AX value allocation failed".into(),
            ));
        }
        check(unsafe { AXUIElementSetAttributeValue(self.0 .0, key.0, value.0) })
    }
}

fn check(status: AxError) -> Result<(), AutomationError> {
    if status == 0 {
        Ok(())
    } else {
        Err(AutomationError::Execution(format!("Accessibility operation failed (AXError {status}); refresh the interface before retrying")))
    }
}

pub(crate) fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// NSRunningApplication is safe to query off the main thread. NSString values
/// are copied before draining this thread's autorelease pool.
pub(crate) fn application_identity(pid: i32) -> Option<AppIdentity> {
    let _pool = Pool::new();
    unsafe {
        let class_name = CString::new("NSRunningApplication").ok()?;
        let class = objc_getClass(class_name.as_ptr());
        if class.is_null() {
            return None;
        }
        let selector = CString::new("runningApplicationWithProcessIdentifier:").ok()?;
        let send_pid: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const ());
        let app = send_pid(class, sel_registerName(selector.as_ptr()), pid);
        if app.is_null() {
            return None;
        }
        let send: unsafe extern "C" fn(*mut c_void, *mut c_void) -> Ref =
            std::mem::transmute(objc_msgSend as *const ());
        let string = |selector: &str| -> Option<String> {
            let selector = CString::new(selector).ok()?;
            let raw = send(app, sel_registerName(selector.as_ptr()));
            if raw.is_null() {
                return None;
            }
            Owned::take(CFRetain(raw))?.text()
        };
        let id = string("bundleIdentifier")?;
        Some(AppIdentity {
            display_name: redact(&string("localizedName").unwrap_or_else(|| id.clone())),
            id,
        })
    }
}

pub(super) struct Snapshot {
    pub app: AppIdentity,
    pub window: WindowIdentity,
    pub metadata: Vec<Metadata>,
    pub fingerprint: String,
}
struct NativeSnapshot {
    snapshot: Snapshot,
    elements: Vec<Element>,
    app: Element,
}

pub(super) fn capture(
    limit: usize,
    scope: &ObservationScope,
    deadline: Instant,
) -> Result<Snapshot, AutomationError> {
    Ok(capture_native(limit, scope, deadline)?.snapshot)
}

fn capture_native(
    limit: usize,
    scope: &ObservationScope,
    deadline: Instant,
) -> Result<NativeSnapshot, AutomationError> {
    if !trusted() {
        return Err(AutomationError::Observation("macOS Accessibility permission is required; enable Nuphus in System Settings > Privacy & Security > Accessibility".into()));
    }
    if scope.subtree_id.is_some() {
        return Err(AutomationError::Observation(
            "subtree observation is not supported; observe the target window".into(),
        ));
    }
    let system = Element::system()
        .ok_or_else(|| AutomationError::Observation("could not create AX system element".into()))?;
    let app = system
        .child("AXFocusedApplication", deadline)
        .ok_or_else(|| {
            AutomationError::Observation("no accessible foreground application".into())
        })?;
    let pid = app.pid().ok_or_else(|| {
        AutomationError::Observation("foreground application has no process identity".into())
    })?;
    let app = Element::application(pid).ok_or_else(|| {
        AutomationError::Observation("foreground application is no longer available".into())
    })?;
    let identity = application_identity(pid).ok_or_else(|| {
        AutomationError::Observation(
            "foreground application has no stable bundle identifier".into(),
        )
    })?;
    let window = app
        .child("AXFocusedWindow", deadline)
        .or_else(|| app.child("AXMainWindow", deadline))
        .ok_or_else(|| {
            AutomationError::Observation("foreground application has no accessible window".into())
        })?;
    let title = window.text("AXTitle", deadline).unwrap_or_default();
    let identifier = window.text("AXIdentifier", deadline);
    // Window titles are the fallback only when the provider has no stable AX
    // identifier. Never persist a pid, CGWindowID or AX object address.
    let window_id = format!(
        "axw:{}",
        hash(&format!(
            "{}|{}",
            identity.id,
            identifier.as_deref().unwrap_or(&title)
        ))
    );
    if scope.app_id.as_ref().is_some_and(|id| id != &identity.id)
        || scope.window_id.as_ref().is_some_and(|id| id != &window_id)
    {
        return Err(AutomationError::Observation(
            "foreground application/window is outside the requested scope".into(),
        ));
    }
    let window_identity = WindowIdentity {
        id: window_id,
        title: redact(&title),
    };
    let mut pending =
        std::collections::VecDeque::from([(window, Vec::<SemanticContext>::new(), false, 0_usize)]);
    // The menu bar is outside AXWindow. Include it so native application menus
    // are available without resorting to blind keyboard/coordinate sequences.
    if let Some(menu) = app.child("AXMenuBar", deadline) {
        pending.push_back((menu, vec![], false, 0));
    }
    let mut metadata = Vec::new();
    let mut elements: Vec<Element> = Vec::new();
    while let Some((element, ancestors, secure_parent, depth)) = pending.pop_front() {
        if metadata.len() >= limit || Instant::now() >= deadline {
            break;
        }
        if elements.iter().any(|old| old.same(&element)) {
            continue;
        }
        let raw_role = element.text("AXRole", deadline).unwrap_or_default();
        let role = role(&raw_role);
        let secure = secure_parent
            || element.text("AXSubrole", deadline).as_deref() == Some("AXSecureTextField")
            || element
                .boolean("AXProtectedContent", deadline)
                .unwrap_or(false);
        let identifier = element.text("AXIdentifier", deadline);
        let raw_name = if secure {
            None
        } else {
            element
                .text("AXTitle", deadline)
                .or_else(|| element.text("AXDescription", deadline))
                .or_else(|| element.text("AXHelp", deadline))
                .map(|s| s.chars().take(256).collect::<String>())
        };
        let value = if secure {
            None
        } else {
            element.attribute("AXValue", deadline)
        };
        let expanded = element.boolean("AXExpanded", deadline);
        let toggled = if role == UiRole::CheckBox {
            value.as_ref().and_then(Owned::boolean)
        } else {
            None
        };
        let selected = element.boolean("AXSelected", deadline).or_else(|| {
            (role == UiRole::RadioButton)
                .then(|| value.as_ref().and_then(Owned::boolean))
                .flatten()
        });
        let names = element.action_names(deadline);
        let supported_actions = actions(
            &role,
            secure,
            &names,
            element.settable("AXFocused", deadline),
            element.settable("AXValue", deadline),
            element.settable("AXSelected", deadline),
            element.settable("AXExpanded", deadline),
            expanded,
        );
        let key = hash(&format!(
            "{}|{}|{:?}|{:?}|{:?}|{:?}",
            identity.id, window_identity.id, role, identifier, raw_name, ancestors
        ));
        let node = UiNode {
            opaque_id: format!("axe:{key}"),
            semantic_key: Some(format!("ax:{key}")),
            role: role.clone(),
            name: raw_name.as_deref().map(redact),
            short_value: None,
            enabled: element.boolean("AXEnabled", deadline).unwrap_or(true),
            visible: !element.boolean("AXHidden", deadline).unwrap_or(false),
            focused: element.boolean("AXFocused", deadline).unwrap_or(false),
            secure,
            toggled,
            selected,
            expanded,
            value_fingerprint: value
                .as_ref()
                .and_then(Owned::text)
                .map(|s| format!("value:{}", hash(&s))),
            supported_actions,
        };
        let mut child_ancestors = ancestors.clone();
        // Root window identity already lives in the locator. Keeping window
        // titles out of ancestry allows providers' stable AXIdentifier to work.
        if depth > 0 && (identifier.is_some() || raw_name.is_some()) {
            child_ancestors.push(SemanticContext {
                role: Some(role),
                automation_id: identifier.clone(),
                accessible_name: raw_name.clone(),
            });
            if child_ancestors.len() > 8 {
                child_ancestors.remove(0);
            }
        }
        if depth < 24 && !secure {
            let remaining = limit
                .saturating_sub(metadata.len() + pending.len())
                .min(200);
            for child in element.children("AXChildren", remaining, deadline) {
                pending.push_back((child, child_ancestors.clone(), secure, depth + 1));
            }
        }
        metadata.push(Metadata {
            node,
            identifier,
            raw_name,
            ancestors,
        });
        elements.push(element);
    }
    if metadata.is_empty() {
        return Err(AutomationError::Observation(
            "Accessibility tree was empty or timed out".into(),
        ));
    }
    let fingerprint = hash(&format!(
        "{}|{}|{:?}",
        identity.id, window_identity.id, metadata
    ));
    Ok(NativeSnapshot {
        snapshot: Snapshot {
            app: identity,
            window: window_identity,
            metadata,
            fingerprint,
        },
        elements,
        app,
    })
}

pub(super) fn validate_locator_window(
    locator: &SemanticLocator,
    observation: &Observation,
) -> Result<(), AutomationError> {
    if locator.app_id != observation.app.id
        || locator
            .window_id
            .as_ref()
            .map(|id| id != &observation.window.id)
            .unwrap_or_else(|| {
                locator
                    .window_title
                    .as_ref()
                    .is_some_and(|title| title != &observation.window.title)
            })
    {
        return Err(AutomationError::Candidates(
            "foreground application/window does not match saved AX locator".into(),
        ));
    }
    Ok(())
}

pub(super) fn execute(
    limit: usize,
    locator: &SemanticLocator,
    action: &NativeAction,
    input: &ExecutionInput,
    deadline: Instant,
    cancelled: impl Fn() -> bool,
) -> Result<(), AutomationError> {
    let snapshot = capture_native(
        limit,
        &ObservationScope {
            app_id: Some(locator.app_id.clone()),
            window_id: locator.window_id.clone(),
            subtree_id: None,
        },
        deadline,
    )?;
    let found: Vec<_> = snapshot
        .snapshot
        .metadata
        .iter()
        .enumerate()
        .filter(|(_, m)| matches(m, locator, action))
        .collect();
    let [(index, _)] = found.as_slice() else {
        return Err(AutomationError::Execution(
            "AX target is missing or ambiguous after refresh".into(),
        ));
    };
    if Instant::now() >= deadline || cancelled() {
        return Err(AutomationError::Execution(
            "AX request expired before dispatch".into(),
        ));
    }
    let element = &snapshot.elements[*index];
    // A user can switch applications while a large tree is being read. Check
    // the concrete foreground window again immediately before dispatch.
    let foreground = Element::system()
        .and_then(|system| system.child("AXFocusedApplication", deadline))
        .ok_or_else(|| {
            AutomationError::Execution("foreground application changed before AX dispatch".into())
        })?;
    let focused_window = foreground
        .child("AXFocusedWindow", deadline)
        .or_else(|| foreground.child("AXMainWindow", deadline));
    if !foreground.same(&snapshot.app)
        || focused_window
            .as_ref()
            .is_none_or(|window| !window.same(&snapshot.elements[0]))
        || Instant::now() >= deadline
        || cancelled()
    {
        return Err(AutomationError::Execution(
            "foreground application/window changed before AX dispatch".into(),
        ));
    }
    match action {
        NativeAction::Invoke | NativeAction::Toggle => element.perform("AXPress"),
        NativeAction::Select => {
            if element.settable("AXSelected", deadline) {
                element.set_bool("AXSelected", true)
            } else {
                element.perform("AXPress")
            }
        }
        NativeAction::Expand => element.set_bool("AXExpanded", true),
        NativeAction::Collapse => element.set_bool("AXExpanded", false),
        NativeAction::Focus => element.set_bool("AXFocused", true),
        NativeAction::SetValue => {
            let value = input.value.as_deref().ok_or_else(|| {
                AutomationError::Execution("AX SetValue requires a local value slot".into())
            })?;
            element.set_text("AXValue", value)
        }
        _ => Err(AutomationError::Execution("unsupported AX action".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_foundation_string_roundtrips_unicode_and_preserves_owned_lifetime() {
        let value = Owned::string("中文 RPA 🖥️");
        let retained = unsafe { Owned::take(CFRetain(value.0)) }.unwrap();
        drop(value);
        assert_eq!(retained.text().as_deref(), Some("中文 RPA 🖥️"));
        assert!(retained.boolean().is_none());
        assert!(retained.array(10).is_empty());
    }

    #[test]
    fn actual_ax_permission_absence_is_a_recoverable_observation_error() {
        if trusted() {
            return;
        }
        let result = capture(
            10,
            &ObservationScope::default(),
            Instant::now() + std::time::Duration::from_secs(1),
        );
        assert!(
            matches!(result, Err(AutomationError::Observation(message)) if message.contains("permission"))
        );
    }

    #[test]
    #[ignore = "requires a logged-in macOS desktop, Accessibility permission, and a foreground test application"]
    fn live_accessibility_observation_reports_semantics_without_document_values() {
        let snapshot = capture(
            200,
            &ObservationScope::default(),
            Instant::now() + std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert!(!snapshot.app.id.is_empty());
        assert!(!snapshot.metadata.is_empty());
        assert!(snapshot
            .metadata
            .iter()
            .all(|meta| meta.node.short_value.is_none()));
        assert!(snapshot
            .metadata
            .iter()
            .filter(|meta| meta.node.secure)
            .all(|meta| meta.node.name.is_none() && meta.node.value_fingerprint.is_none()));
    }
}
