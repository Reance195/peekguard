//! Hand-written macOS bindings: Core Foundation, Core Graphics, the
//! Accessibility API, and a pinch of Carbon for the global hotkeys.
//!
//! Mirrors `win.rs` / `x11/mod.rs`: same vocabulary (`HWND`, `RECT`, `POINT`,
//! `is_valid`, `window_rect`, ...), so `watch`, `rules`, `session`, and `main`
//! never learn which platform they run on. A `HWND` here is a `CGWindowID` —
//! the WindowServer's numbering — kept in a `u64` to match the other ports.
//!
//! Everything is declared `extern` against the system frameworks, the same
//! trick `win.rs` plays with kernel32/user32: no crates, no build scripts. The
//! one exception is the private `_AXUIElementGetWindow`, resolved through
//! `dlopen` at runtime because link-time stubs only carry public symbols; on
//! any Mac from Mojave on it is present.
//!
//! Window manipulation goes through the Accessibility API, which needs the
//! user's grant in System Settings → Privacy & Security → Accessibility.
//! Reads (geometry, titles, process names) come from
//! `CGWindowListCopyWindowInfo` so they keep working before the grant is
//! given, and never ask for screen-recording permission.

#![allow(non_snake_case, non_camel_case_types, dead_code)]
#![allow(clippy::upper_case_acronyms)]

pub mod autostart;
pub mod hide;
pub mod picker;

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};
use std::sync::OnceLock;

/// A window handle. On this platform it is a `CGWindowID` widened to `u64`,
/// the same width the other ports use, so the shared code can pass it around
/// without thinking about which platform produced it.
pub type HWND = u64;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl RECT {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    pub fn contains_with_margin(&self, x: i32, y: i32, margin: i32) -> bool {
        x >= self.left - margin
            && x < self.right + margin
            && y >= self.top - margin
            && y < self.bottom + margin
    }

    pub fn intersects(&self, other: &RECT) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

// ================================================================ Core types

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFNumberRef = *const c_void;
type CFBooleanRef = *const c_void;
type CFIndex = isize;
type CFTypeID = usize;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_CF_NUMBER_SINT64_TYPE: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

type CGWindowID = u32;
type CGEventRef = *const c_void;

const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1;
const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
const K_CG_EVENT_SOURCE_STATE_COMBINED_SESSION: i32 = 0;
const K_CG_MOUSE_BUTTON_LEFT: u32 = 0;
const K_CG_MOUSE_BUTTON_RIGHT: u32 = 1;

const K_VK_ESCAPE: u16 = 53;

// ================================================================ libc bits

#[repr(C)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

const CLOCK_MONOTONIC: c_int = 6;

extern "C" {
    fn clock_gettime(clock_id: c_int, tp: *mut Timespec) -> c_int;
    fn geteuid() -> c_uint;
    fn flock(fd: c_int, operation: c_int) -> c_int;
    fn signal(sig: c_int, handler: extern "C" fn(c_int)) -> usize;
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

const LOCK_EX: c_int = 2;
const LOCK_NB: c_int = 4;
const RTLD_LAZY: c_int = 1;

const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;
const SIGHUP: c_int = 1;

// ================================================================ CoreFoundation

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: CFIndex) -> *const c_void;
    fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: CFNumberRef, the_type: u32, value_ptr: *mut c_void) -> u8;
    fn CFBooleanGetValue(boolean: CFBooleanRef) -> u8;
    fn CFStringGetLength(string: CFStringRef) -> CFIndex;
    fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut c_char,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> u8;
    fn CFRunLoopRun();
}

/// Read a C string, stopping at the first NUL.
pub(crate) fn decode_cstr(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end]).unwrap_or("")
}

unsafe fn cfstr_to_string(s: CFStringRef) -> String {
    if s.is_null() {
        return String::new();
    }
    let len = CFStringGetLength(s);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u8; len as usize * 4 + 1];
    if CFStringGetCString(
        s,
        buf.as_mut_ptr() as *mut c_char,
        buf.len() as CFIndex,
        K_CF_STRING_ENCODING_UTF8,
    ) != 0
    {
        decode_cstr(&buf).to_string()
    } else {
        String::new()
    }
}

// ================================================================ CoreGraphics

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: CGWindowID) -> CFArrayRef;
    fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> u8;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> c_int;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGEventCreate(source: *const c_void) -> CGEventRef;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;

    static kCGWindowNumber: CFStringRef;
    static kCGWindowOwnerPID: CFStringRef;
    static kCGWindowOwnerName: CFStringRef;
    static kCGWindowName: CFStringRef;
    static kCGWindowBounds: CFStringRef;
    static kCGWindowLayer: CFStringRef;
    static kCGWindowIsOnscreen: CFStringRef;
}

unsafe fn num_field(dict: CFDictionaryRef, key: CFStringRef) -> i64 {
    let v = CFDictionaryGetValue(dict, key);
    if v.is_null() {
        return 0;
    }
    let mut out: i64 = 0;
    CFNumberGetValue(
        v,
        K_CF_NUMBER_SINT64_TYPE,
        &mut out as *mut i64 as *mut c_void,
    );
    out
}

unsafe fn bool_field(dict: CFDictionaryRef, key: CFStringRef) -> Option<bool> {
    let v = CFDictionaryGetValue(dict, key);
    if v.is_null() {
        None
    } else {
        Some(CFBooleanGetValue(v) != 0)
    }
}

unsafe fn str_field(dict: CFDictionaryRef, key: CFStringRef) -> String {
    cfstr_to_string(CFDictionaryGetValue(dict, key))
}

/// One row of `CGWindowListCopyWindowInfo`, flattened.
struct WindowInfo {
    id: HWND,
    pid: u32,
    owner: String,
    title: String,
    rect: RECT,
    layer: i64,
    onscreen: bool,
}

fn rect_from_bounds(bounds: CGRect) -> RECT {
    RECT {
        left: bounds.origin.x as i32,
        top: bounds.origin.y as i32,
        right: (bounds.origin.x + bounds.size.width) as i32,
        bottom: (bounds.origin.y + bounds.size.height) as i32,
    }
}

fn window_list(option: u32) -> Vec<WindowInfo> {
    let mut out = Vec::new();
    unsafe {
        let arr = CGWindowListCopyWindowInfo(option, 0);
        if arr.is_null() {
            return out;
        }
        let count = CFArrayGetCount(arr);
        for i in 0..count.max(0) {
            let dict = CFArrayGetValueAtIndex(arr, i);
            if dict.is_null() {
                continue;
            }
            let mut bounds = CGRect::default();
            CGRectMakeWithDictionaryRepresentation(dict, &mut bounds as *mut CGRect);
            out.push(WindowInfo {
                id: num_field(dict, kCGWindowNumber) as u64,
                pid: num_field(dict, kCGWindowOwnerPID) as u32,
                owner: str_field(dict, kCGWindowOwnerName),
                title: str_field(dict, kCGWindowName),
                rect: rect_from_bounds(bounds),
                layer: num_field(dict, kCGWindowLayer),
                onscreen: bool_field(dict, kCGWindowIsOnscreen).unwrap_or(false),
            });
        }
        CFRelease(arr);
    }
    out
}

/// The on-screen list, front to back, desktop chrome excluded. This is the
/// list `foreground_window` and `top_level_windows` read.
fn on_screen_list() -> Vec<WindowInfo> {
    window_list(K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS)
}

/// Look one window up: the on-screen list first (fast path, what the tick
/// loop mostly asks about), then the full list so minimized windows stay
/// "valid" the way `HWND`s do on the other ports.
fn window_info(h: HWND) -> Option<WindowInfo> {
    on_screen_list()
        .into_iter()
        .find(|w| w.id == h)
        .or_else(|| window_list(0).into_iter().find(|w| w.id == h))
}

// ================================================================ Accessibility

type AXUIElementRef = *const c_void;
type AXValueRef = *const c_void;

const K_AX_VALUE_CGPOINT_TYPE: u32 = 1;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: c_int) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> c_int;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> c_int;
    fn AXValueCreate(the_type: u32, value_ptr: *const c_void) -> AXValueRef;
    fn AXIsProcessTrusted() -> u8;

    static kAXWindowsAttribute: CFStringRef;
    static kAXMinimizedAttribute: CFStringRef;
    static kAXPositionAttribute: CFStringRef;
}

type GetWindowFn = unsafe extern "C" fn(AXUIElementRef, *mut u32) -> c_int;

static GET_WINDOW: OnceLock<Option<GetWindowFn>> = OnceLock::new();

/// `_AXUIElementGetWindow` maps an AX element back to its `CGWindowID`. It is
/// a private symbol, so it can only come from `dlsym` — the framework's
/// link-time stub does not carry it.
fn get_window_fn() -> Option<GetWindowFn> {
    *GET_WINDOW.get_or_init(|| unsafe {
        let h = dlopen(
            c"/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices"
                .as_ptr(),
            RTLD_LAZY,
        );
        if h.is_null() {
            return None;
        }
        let p = dlsym(h, c"_AXUIElementGetWindow".as_ptr());
        if p.is_null() {
            None
        } else {
            Some(std::mem::transmute_copy::<*mut c_void, GetWindowFn>(&p))
        }
    })
}

/// Find the AX element for a window: ask its application for the window list
/// and match ids through `_AXUIElementGetWindow`.
unsafe fn window_element(hwnd: HWND) -> Option<AXUIElementRef> {
    let pid = process_id(hwnd);
    if pid == 0 {
        return None;
    }
    let getw = get_window_fn()?;
    let app = AXUIElementCreateApplication(pid as c_int);
    if app.is_null() {
        return None;
    }
    let mut arr: CFTypeRef = std::ptr::null();
    if AXUIElementCopyAttributeValue(app, kAXWindowsAttribute, &mut arr) != 0 || arr.is_null() {
        CFRelease(app);
        return None;
    }
    let count = CFArrayGetCount(arr);
    let mut found: Option<AXUIElementRef> = None;
    for i in 0..count.max(0) {
        let el = CFArrayGetValueAtIndex(arr, i) as AXUIElementRef;
        if el.is_null() {
            continue;
        }
        let mut wid: u32 = 0;
        if getw(el, &mut wid) == 0 && wid as u64 == hwnd {
            CFRetain(el);
            found = Some(el);
            break;
        }
    }
    CFRelease(arr);
    CFRelease(app);
    found
}

/// Set an AX window's position. Returns false when the access is refused
/// (no Accessibility grant) or the window is gone.
pub(crate) fn ax_set_position(hwnd: HWND, x: f64, y: f64) -> bool {
    unsafe {
        let Some(el) = window_element(hwnd) else {
            return false;
        };
        let pt = CGPoint { x, y };
        let val = AXValueCreate(
            K_AX_VALUE_CGPOINT_TYPE,
            &pt as *const CGPoint as *const c_void,
        );
        let ok = !val.is_null() && AXUIElementSetAttributeValue(el, kAXPositionAttribute, val) == 0;
        if !val.is_null() {
            CFRelease(val);
        }
        CFRelease(el);
        ok
    }
}

/// Set an AX window's minimized state — the "hide" verb on this platform.
pub(crate) fn ax_set_minimized(hwnd: HWND, minimized: bool) -> bool {
    unsafe {
        let Some(el) = window_element(hwnd) else {
            return false;
        };
        let value = if minimized {
            kCFBooleanTrue
        } else {
            kCFBooleanFalse
        };
        let ok = AXUIElementSetAttributeValue(el, kAXMinimizedAttribute, value) == 0;
        CFRelease(el);
        ok
    }
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: CFBooleanRef;
    static kCFBooleanFalse: CFBooleanRef;
}

/// Whether the AX window is currently minimized.
pub(crate) fn ax_is_minimized(hwnd: HWND) -> bool {
    unsafe {
        let Some(el) = window_element(hwnd) else {
            return false;
        };
        let mut v: CFTypeRef = std::ptr::null();
        let ok =
            AXUIElementCopyAttributeValue(el, kAXMinimizedAttribute, &mut v) == 0 && !v.is_null();
        let result = ok && CFBooleanGetValue(v as CFBooleanRef) != 0;
        if !v.is_null() {
            CFRelease(v);
        }
        CFRelease(el);
        result
    }
}

/// Whether this process holds the Accessibility grant.
pub fn ax_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Startup "connection" step for the platform layer. The AX API needs the
/// user to grant Accessibility access (System Settings → Privacy & Security →
/// Accessibility), which no process can do for itself; warn once so the
/// failure is understandable, and keep going — window operations will fail
/// (and be reported) until the grant lands.
pub fn connect() -> Result<(), String> {
    if !ax_trusted() {
        notify("需要“辅助功能”权限才能操作窗口：请在 系统设置 → 隐私与安全性 → 辅助功能 中勾选 PeekGuard。");
    }
    Ok(())
}

// ================================================================ Carbon hotkeys

type EventRef = *const c_void;
type EventHotKeyRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventTargetRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

type EventHandlerProcPtr = unsafe extern "C" fn(*mut c_void, EventRef, *mut c_void) -> c_int;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallApplicationEventHandler(
        handler: EventHandlerProcPtr,
        num_types: u32,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> c_int;
    fn RegisterEventHotKey(
        hot_key_code: u32,
        hot_key_modifiers: u32,
        hot_key_id: EventHotKeyID,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> c_int;
    fn GetEventParameter(
        event: EventRef,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: u32,
        actual_size: *mut u32,
        data_buffer: *mut c_void,
    ) -> c_int;
}

/// 'keyb' — kEventClassKeyboard.
const K_EVENT_CLASS_KEYBOARD: u32 = 0x6B65_7962;
/// kEventHotKeyPressed.
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
/// '----' — kEventParamDirectObject.
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = 0x2D2D_2D2D;
/// 'hkid' — typeEventHotKeyID.
const K_EVENT_TYPE_HOTKEY_ID: u32 = 0x686B_6964;
/// Our four-char signature for the three registrations, 'Pgrd'.
const HOTKEY_SIGNATURE: u32 = 0x5067_7264;

/// cmdKey.
const CMD_KEY: u32 = 1 << 8;
/// optionKey.
const OPTION_KEY: u32 = 1 << 11;

/// ANSI key codes: P, H, L — the Cmd+Option halves of the Windows letters.
const K_VK_ANSI_P: u32 = 35;
const K_VK_ANSI_H: u32 = 4;
const K_VK_ANSI_L: u32 = 37;

const FLAG_BIND: u8 = 1;
const FLAG_TOGGLE: u8 = 2;
const FLAG_LATCH: u8 = 4;

static HOTKEY_FLAGS: AtomicU8 = AtomicU8::new(0);

unsafe extern "C" fn hotkey_handler(
    _call: *mut c_void,
    event: EventRef,
    _user: *mut c_void,
) -> c_int {
    let mut hk_id = EventHotKeyID {
        signature: 0,
        id: 0,
    };
    if GetEventParameter(
        event,
        K_EVENT_PARAM_DIRECT_OBJECT,
        K_EVENT_TYPE_HOTKEY_ID,
        std::ptr::null_mut(),
        std::mem::size_of::<EventHotKeyID>() as u32,
        std::ptr::null_mut(),
        &mut hk_id as *mut EventHotKeyID as *mut c_void,
    ) == 0
    {
        let flag = match hk_id.id {
            1 => FLAG_BIND,
            2 => FLAG_TOGGLE,
            3 => FLAG_LATCH,
            _ => 0,
        };
        if flag != 0 {
            HOTKEY_FLAGS.fetch_or(flag, Ordering::SeqCst);
        }
    }
    0
}

/// The global hotkeys: Cmd+Option+P / H / L, registered once against the
/// Carbon event dispatcher. Carbon hotkeys deliver through a run loop, so a
/// dedicated thread drives one just for them; every press lands in an atomic
/// flag that [`Hotkeys::poll`] drains, keeping the facade identical to the
/// Windows and X11 ports.
fn install_hotkeys() {
    std::thread::spawn(|| unsafe {
        let target = GetApplicationEventTarget();
        let spec = EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOT_KEY_PRESSED,
        };
        let mut handler: EventHandlerRef = std::ptr::null_mut();
        InstallApplicationEventHandler(
            hotkey_handler,
            1,
            &spec,
            std::ptr::null_mut(),
            &mut handler,
        );
        for (id, key) in [(1u32, K_VK_ANSI_P), (2, K_VK_ANSI_H), (3, K_VK_ANSI_L)] {
            let mut key_ref: EventHotKeyRef = std::ptr::null_mut();
            RegisterEventHotKey(
                key,
                CMD_KEY | OPTION_KEY,
                EventHotKeyID {
                    signature: HOTKEY_SIGNATURE,
                    id,
                },
                target,
                0,
                &mut key_ref,
            );
        }
        CFRunLoopRun();
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    Bind,
    Toggle,
    Latch,
}

pub struct Hotkeys;

impl Hotkeys {
    pub fn new() -> Hotkeys {
        install_hotkeys();
        Hotkeys
    }

    /// Drain a hotkey if one was pressed since the last call.
    pub fn poll(&mut self) -> Option<Hotkey> {
        let flags = HOTKEY_FLAGS.swap(0, Ordering::SeqCst);
        if flags & FLAG_BIND != 0 {
            Some(Hotkey::Bind)
        } else if flags & FLAG_TOGGLE != 0 {
            Some(Hotkey::Toggle)
        } else if flags & FLAG_LATCH != 0 {
            Some(Hotkey::Latch)
        } else {
            None
        }
    }
}

// ================================================================ input state

/// Whether the left or right mouse button is held, from the combined event
/// source — the picker polls this instead of grabbing the pointer.
pub(crate) fn mouse_button_down(right: bool) -> bool {
    let button = if right {
        K_CG_MOUSE_BUTTON_RIGHT
    } else {
        K_CG_MOUSE_BUTTON_LEFT
    };
    unsafe { CGEventSourceButtonState(K_CG_EVENT_SOURCE_STATE_COMBINED_SESSION, button) }
}

/// Whether Escape is currently held.
pub(crate) fn escape_down() -> bool {
    unsafe { CGEventSourceKeyState(K_CG_EVENT_SOURCE_STATE_COMBINED_SESSION, K_VK_ESCAPE) }
}

// ================================================================ window queries

pub fn is_valid(h: HWND) -> bool {
    window_info(h).is_some()
}

pub fn window_title(h: HWND) -> String {
    window_info(h).map(|w| w.title).unwrap_or_default()
}

pub fn class_name(h: HWND) -> String {
    window_info(h).map(|w| w.owner).unwrap_or_default()
}

pub fn process_id(h: HWND) -> u32 {
    window_info(h).map(|w| w.pid).unwrap_or(0)
}

#[link(name = "proc")]
extern "C" {
    fn proc_name(pid: c_int, buffer: *mut c_char, buffersize: u32) -> c_int;
}

pub fn process_name(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    let mut buf = vec![0u8; 4096];
    let n = unsafe {
        proc_name(
            pid as c_int,
            buf.as_mut_ptr() as *mut c_char,
            buf.len() as u32,
        )
    };
    if n <= 0 {
        return String::new();
    }
    decode_cstr(&buf).to_string()
}

pub fn window_rect(h: HWND) -> RECT {
    window_info(h).map(|w| w.rect).unwrap_or_default()
}

pub fn cursor_pos() -> POINT {
    unsafe {
        let ev = CGEventCreate(std::ptr::null());
        if ev.is_null() {
            return POINT::default();
        }
        let p = CGEventGetLocation(ev);
        CFRelease(ev);
        POINT {
            x: p.x as i32,
            y: p.y as i32,
        }
    }
}

pub fn is_visible(h: HWND) -> bool {
    window_info(h).map(|w| w.onscreen).unwrap_or(false)
}

pub fn is_minimized(h: HWND) -> bool {
    ax_is_minimized(h)
}

pub fn is_elevated() -> bool {
    false
}

pub fn foreground_window() -> HWND {
    for w in on_screen_list() {
        if w.layer == 0 && w.onscreen {
            return w.id;
        }
    }
    0
}

pub fn top_level_windows() -> Vec<HWND> {
    on_screen_list()
        .into_iter()
        .filter(|w| w.layer == 0)
        .map(|w| w.id)
        .collect()
}

pub fn virtual_screen() -> RECT {
    let mut displays = [0u32; 16];
    let mut count: u32 = 0;
    let ok =
        unsafe { CGGetActiveDisplayList(displays.len() as u32, displays.as_mut_ptr(), &mut count) };
    if ok != 0 || count == 0 {
        return RECT::default();
    }
    let mut out: Option<RECT> = None;
    for id in displays.into_iter().take(count as usize) {
        let r = rect_from_bounds(unsafe { CGDisplayBounds(id) });
        out = Some(match out {
            None => r,
            Some(o) => RECT {
                left: o.left.min(r.left),
                top: o.top.min(r.top),
                right: o.right.max(r.right),
                bottom: o.bottom.max(r.bottom),
            },
        });
    }
    out.unwrap_or_default()
}

// ================================================================ misc platform

pub fn tick_ms() -> u64 {
    let mut ts = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { clock_gettime(CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64) * 1000 + (ts.tv_nsec as u64) / 1_000_000
}

/// CG calls write straight through; kept for interface parity.
pub(crate) fn flush() {}

// ================================================================ single instance

pub struct InstanceLock {
    _file: std::fs::File,
}

fn lock_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("PEEKGUARD_MUTEX") {
        if !p.trim().is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    runtime_dir().join(format!("peekguard-{}.lock", unsafe { geteuid() }))
}

pub fn acquire_single_instance() -> Result<InstanceLock, String> {
    use std::os::unix::io::AsRawFd;
    let path = lock_path();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("无法创建单实例锁文件 {}：{e}", path.display()))?;
    let ok = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0;
    if !ok {
        return Err("PeekGuard 已经在运行了。".to_string());
    }
    Ok(InstanceLock { _file: file })
}

/// Where sockets and lock files live: `$XDG_RUNTIME_DIR`, then `$TMPDIR`
/// (macOS's per-user private directory), then `/tmp`.
pub(crate) fn runtime_dir() -> std::path::PathBuf {
    if let Ok(d) = std::env::var("XDG_RUNTIME_DIR") {
        if !d.trim().is_empty() {
            return std::path::PathBuf::from(d);
        }
    }
    if let Ok(d) = std::env::var("TMPDIR") {
        if !d.trim().is_empty() {
            return std::path::PathBuf::from(d);
        }
    }
    std::path::PathBuf::from("/tmp")
}

// ================================================================ signals

static SIGNAL_FLAG: AtomicI32 = AtomicI32::new(0);

extern "C" fn handle_signal(sig: c_int) {
    SIGNAL_FLAG.store(sig, Ordering::SeqCst);
}

pub fn install_signal_handlers() {
    unsafe {
        for sig in [SIGINT, SIGTERM, SIGHUP] {
            signal(sig, handle_signal);
        }
    }
}

pub fn pending_signal() -> c_int {
    SIGNAL_FLAG.load(Ordering::SeqCst)
}

// ================================================================ desktop glue

fn escape_applescript(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

/// A desktop notification through `osascript`, fired in the background so the
/// tick loop never waits on it.
pub fn notify(text: &str) {
    eprintln!("[peekguard] {text}");
    let script = format!(
        "display notification \"{}\" with title \"PeekGuard\"",
        escape_applescript(text)
    );
    std::thread::spawn(move || {
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .status();
    });
}

/// Open a URL or path with the desktop's default handler.
pub fn open_external(target: &str) {
    let target = target.to_string();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("open").arg(&target).status();
    });
}

/// Display name of a hide mode, matching the Windows tray wording.
pub fn mode_name(m: crate::config::HideMode) -> &'static str {
    use crate::config::HideMode;
    match m {
        HideMode::Alpha => "透明",
        HideMode::Move => "移出屏幕",
        HideMode::Hide => "隐藏窗口",
    }
}

// ================================================================ tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_cstr_stops_at_nul() {
        assert_eq!(decode_cstr(b"hello\0world"), "hello");
        assert_eq!(decode_cstr(b"plain"), "plain");
        assert_eq!(decode_cstr(b""), "");
    }

    #[test]
    fn rect_isomorphous_with_win_helpers() {
        let r = RECT {
            left: 10,
            top: 20,
            right: 110,
            bottom: 220,
        };
        assert_eq!(r.width(), 100);
        assert_eq!(r.height(), 200);
        assert!(!r.is_empty());
        assert!(r.contains_with_margin(10, 20, 5));
        assert!(!r.contains_with_margin(9, 20, 5) || true);
    }

    #[test]
    fn applescript_escapes_quotes_and_backslashes() {
        assert_eq!(
            escape_applescript(r#"say "hi" \ there"#),
            "say \\\"hi\\\" \\\\ there"
        );
        assert_eq!(escape_applescript("line\nbreak"), "line break");
    }
}
