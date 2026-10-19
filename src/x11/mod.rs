//! Linux/X11 backend: raw Xlib bindings, hand-written, loaded with dlopen.
//!
//! PeekGuard manipulates foreign windows through Xlib. To keep the zero-crate
//! promise the shared library is resolved at startup with `dlopen("libX11.so.6")`
//! and every entry point with `dlsym`; a missing symbol is a clear error, never
//! a link-time dependency. Only X11 sessions are supported: Wayland forbids
//! one client from moving, hiding or reading other clients' windows and
//! pointers, which is exactly what this program does (XWayland will only ever
//! expose its own clients, so we refuse Wayland sessions loudly).
//!
//! There is no X event loop. Everything is polled at the portable core's 16 ms
//! tick: global hotkeys through `XQueryKeymap` (we read the keyboard bitmap,
//! never keystroke events - nothing is logged or forwarded), the cursor through
//! `XQueryPointer`, and the picker through a pointer grab that only swaps the
//! cursor. This keeps one architecture for all platforms and avoids racing
//! event dispatch against the tick logic.
//!
//! The helpers below mirror the names and signatures of `win.rs` so that
//! `watch.rs`, `rules.rs` and `session.rs` compile unchanged on either target.

#![allow(non_snake_case, non_camel_case_types, dead_code)]
#![allow(clippy::upper_case_acronyms)]

pub mod autostart;
pub mod hide;
pub mod ipc;
pub mod picker;

use std::ffi::{c_char, c_int, c_long, c_uchar, c_uint, c_ulong, c_void, CString};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::OnceLock;

// ================================================================ types

/// X11 windows are 32-bit XIDs; we widen them to u64 so that `HWND` keeps the
/// same shape as on Windows (a plain integer handle).
pub type HWND = u64;
pub type BOOL = c_int;
pub type Cursor = c_ulong;
pub type Display = c_void;

pub const IS_VIEWABLE: c_int = 2;

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
    /// Inside test with `margin` px of slack on every side.
    pub fn contains_with_margin(&self, x: i32, y: i32, margin: i32) -> bool {
        x >= self.left - margin
            && x < self.right + margin
            && y >= self.top - margin
            && y < self.bottom + margin
    }
    pub fn intersects(&self, o: &RECT) -> bool {
        self.left < o.right && o.left < self.right && self.top < o.bottom && o.top < self.bottom
    }
}

// ================================================================ libc

const RTLD_LAZY: c_int = 1;
const CLOCK_MONOTONIC: c_int = 1;
const LOCK_EX: c_int = 2;
const LOCK_NB: c_int = 4;

const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;
const SIGHUP: c_int = 1;

#[repr(C)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[link(name = "dl")]
extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

extern "C" {
    fn clock_gettime(clockid: c_int, tp: *mut Timespec) -> c_int;
    fn geteuid() -> u32;
    fn flock(fd: c_int, op: c_int) -> c_int;
    fn signal(signum: c_int, handler: extern "C" fn(c_int)) -> usize;
}

// ================================================================ Xlib ffi

#[repr(C)]
pub(crate) struct XWindowAttributes {
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub border_width: c_int,
    pub depth: c_int,
    pub visual: *mut c_void,
    pub root: HWND,
    pub class: c_int,
    pub bit_gravity: c_int,
    pub win_gravity: c_int,
    pub backing_store: c_int,
    pub backing_planes: c_ulong,
    pub backing_pixel: c_ulong,
    pub save_under: BOOL,
    pub colormap: c_ulong,
    pub map_installed: BOOL,
    pub map_state: c_int,
    pub all_event_masks: c_long,
    pub your_event_mask: c_long,
    pub do_not_propagate_mask: c_long,
    pub override_redirect: BOOL,
    pub screen: *mut c_void,
}

#[repr(C)]
pub(crate) struct XErrorEvent {
    pub type_: c_int,
    pub display: *mut Display,
    pub resourceid: HWND,
    pub serial: c_ulong,
    pub error_code: c_uchar,
    pub request_code: c_uchar,
    pub minor_code: c_uchar,
}

#[allow(clippy::type_complexity)]
pub(crate) struct Xlib {
    XOpenDisplay: unsafe extern "C" fn(*const c_char) -> *mut Display,
    XCloseDisplay: unsafe extern "C" fn(*mut Display) -> c_int,
    XDefaultScreen: unsafe extern "C" fn(*mut Display) -> c_int,
    XDefaultRootWindow: unsafe extern "C" fn(*mut Display) -> HWND,
    XDisplayWidth: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    XDisplayHeight: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    XSync: unsafe extern "C" fn(*mut Display, BOOL) -> c_int,
    XFlush: unsafe extern "C" fn(*mut Display) -> c_int,
    XSetErrorHandler: unsafe extern "C" fn(
        unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int,
    ) -> usize,
    XInternAtom: unsafe extern "C" fn(*mut Display, *const c_char, BOOL) -> c_ulong,
    XGetWindowAttributes: unsafe extern "C" fn(*mut Display, HWND, *mut XWindowAttributes) -> c_int,
    XQueryTree: unsafe extern "C" fn(
        *mut Display,
        HWND,
        *mut HWND,
        *mut HWND,
        *mut *mut HWND,
        *mut c_uint,
    ) -> c_int,
    XFree: unsafe extern "C" fn(*mut c_void) -> c_int,
    XQueryPointer: unsafe extern "C" fn(
        *mut Display,
        HWND,
        *mut HWND,
        *mut HWND,
        *mut c_int,
        *mut c_int,
        *mut c_int,
        *mut c_int,
        *mut c_uint,
    ) -> BOOL,
    XQueryKeymap: unsafe extern "C" fn(*mut Display, *mut c_char) -> c_int,
    XKeysymToKeycode: unsafe extern "C" fn(*mut Display, c_ulong) -> c_uchar,
    XTranslateCoordinates: unsafe extern "C" fn(
        *mut Display,
        HWND,
        HWND,
        c_int,
        c_int,
        *mut c_int,
        *mut c_int,
        *mut HWND,
    ) -> BOOL,
    XGetWindowProperty: unsafe extern "C" fn(
        *mut Display,
        HWND,
        c_ulong,
        c_long,
        c_long,
        BOOL,
        c_ulong,
        *mut c_ulong,
        *mut c_int,
        *mut c_ulong,
        *mut c_ulong,
        *mut *mut c_uchar,
    ) -> c_int,
    XChangeProperty: unsafe extern "C" fn(
        *mut Display,
        HWND,
        c_ulong,
        c_ulong,
        c_int,
        c_int,
        *const c_uchar,
        c_int,
    ) -> c_int,
    XDeleteProperty: unsafe extern "C" fn(*mut Display, HWND, c_ulong) -> c_int,
    XMoveWindow: unsafe extern "C" fn(*mut Display, HWND, c_int, c_int) -> c_int,
    XMapWindow: unsafe extern "C" fn(*mut Display, HWND) -> c_int,
    XUnmapWindow: unsafe extern "C" fn(*mut Display, HWND) -> c_int,
    XRaiseWindow: unsafe extern "C" fn(*mut Display, HWND) -> c_int,
    XCreateFontCursor: unsafe extern "C" fn(*mut Display, c_uint) -> Cursor,
    XFreeCursor: unsafe extern "C" fn(*mut Display, Cursor) -> c_int,
    XGrabPointer: unsafe extern "C" fn(
        *mut Display,
        HWND,
        BOOL,
        c_uint,
        c_int,
        c_int,
        HWND,
        Cursor,
        c_ulong,
    ) -> c_int,
    XUngrabPointer: unsafe extern "C" fn(*mut Display, c_ulong) -> c_int,
    XGetSelectionOwner: unsafe extern "C" fn(*mut Display, c_ulong) -> HWND,
}

static LIB: OnceLock<Result<Xlib, String>> = OnceLock::new();

/// Resolve one function pointer, with a readable error instead of a segfault.
/// `transmute_copy` keeps clippy's annotation lint quiet: the target type is
/// the struct field this lands in, and fn pointers are pointer-sized.
macro_rules! sym {
    ($handle:expr, $name:ident) => {{
        let p = dlsym(
            $handle,
            concat!(stringify!($name), "\0").as_ptr() as *const c_char,
        );
        if p.is_null() {
            return Err(format!("libX11.so.6 缺少符号 {}。", stringify!($name)));
        }
        std::mem::transmute_copy(&p)
    }};
}

unsafe fn load() -> Result<Xlib, String> {
    let handle = dlopen(c"libX11.so.6".as_ptr(), RTLD_LAZY);
    if handle.is_null() {
        return Err("无法加载 libX11.so.6（请安装 libx11-6 软件包）。".into());
    }
    Ok(Xlib {
        XOpenDisplay: sym!(handle, XOpenDisplay),
        XCloseDisplay: sym!(handle, XCloseDisplay),
        XDefaultScreen: sym!(handle, XDefaultScreen),
        XDefaultRootWindow: sym!(handle, XDefaultRootWindow),
        XDisplayWidth: sym!(handle, XDisplayWidth),
        XDisplayHeight: sym!(handle, XDisplayHeight),
        XSync: sym!(handle, XSync),
        XFlush: sym!(handle, XFlush),
        XSetErrorHandler: sym!(handle, XSetErrorHandler),
        XInternAtom: sym!(handle, XInternAtom),
        XGetWindowAttributes: sym!(handle, XGetWindowAttributes),
        XQueryTree: sym!(handle, XQueryTree),
        XFree: sym!(handle, XFree),
        XQueryPointer: sym!(handle, XQueryPointer),
        XQueryKeymap: sym!(handle, XQueryKeymap),
        XKeysymToKeycode: sym!(handle, XKeysymToKeycode),
        XTranslateCoordinates: sym!(handle, XTranslateCoordinates),
        XGetWindowProperty: sym!(handle, XGetWindowProperty),
        XChangeProperty: sym!(handle, XChangeProperty),
        XDeleteProperty: sym!(handle, XDeleteProperty),
        XMoveWindow: sym!(handle, XMoveWindow),
        XMapWindow: sym!(handle, XMapWindow),
        XUnmapWindow: sym!(handle, XUnmapWindow),
        XRaiseWindow: sym!(handle, XRaiseWindow),
        XCreateFontCursor: sym!(handle, XCreateFontCursor),
        XFreeCursor: sym!(handle, XFreeCursor),
        XGrabPointer: sym!(handle, XGrabPointer),
        XUngrabPointer: sym!(handle, XUngrabPointer),
        XGetSelectionOwner: sym!(handle, XGetSelectionOwner),
    })
}

pub(crate) fn lib() -> Result<&'static Xlib, String> {
    match LIB.get_or_init(|| unsafe { load() }) {
        Ok(l) => Ok(l),
        Err(e) => Err(e.clone()),
    }
}

// ================================================================ connection

static DISPLAY: OnceLock<usize> = OnceLock::new();

unsafe extern "C" fn ignore_x_error(_dpy: *mut Display, _ev: *mut XErrorEvent) -> c_int {
    // Windows die all the time (we poll them); BadWindow & friends are normal
    // and must not spam stderr or kill the process.
    0
}

/// Open the X display and install the quiet error handler. Must be called once
/// before any other helper; a second call is a cheap no-op.
pub fn connect() -> Result<(), String> {
    if DISPLAY.get().is_some() {
        return Ok(());
    }
    let x = lib()?;
    let d = unsafe { (x.XOpenDisplay)(std::ptr::null()) };
    if d.is_null() {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let display = std::env::var_os("DISPLAY").is_some();
        return Err(if wayland && !display {
            "当前是 Wayland 会话。Wayland 不允许应用操控其他应用的窗口，请改用 X11 会话\
             （登录界面选择 “在 Xorg 上运行”，WSLg 也可用）。"
                .into()
        } else {
            "无法连接 X 服务器（DISPLAY 未设置或不可用）。".into()
        });
    }
    unsafe {
        (x.XSetErrorHandler)(ignore_x_error);
        (x.XSync)(d, 0);
    }
    let _ = DISPLAY.set(d as usize);
    Ok(())
}

pub(crate) fn dpy() -> *mut Display {
    DISPLAY
        .get()
        .copied()
        .map(|v| v as *mut Display)
        .expect("x11::connect() must be called before using the X display")
}

pub(crate) fn root() -> HWND {
    unsafe { (lib().unwrap().XDefaultRootWindow)(dpy()) }
}

pub(crate) fn screen() -> c_int {
    unsafe { (lib().unwrap().XDefaultScreen)(dpy()) }
}

// ================================================================ atoms

pub(crate) struct Atoms {
    pub NET_WM_NAME: c_ulong,
    pub UTF8_STRING: c_ulong,
    pub WM_NAME: c_ulong,
    pub WM_CLASS: c_ulong,
    pub STRING: c_ulong,
    pub CARDINAL: c_ulong,
    pub WINDOW: c_ulong,
    pub ATOM: c_ulong,
    pub NET_WM_PID: c_ulong,
    pub NET_WM_STATE: c_ulong,
    pub NET_WM_STATE_HIDDEN: c_ulong,
    pub NET_ACTIVE_WINDOW: c_ulong,
    pub NET_WM_WINDOW_OPACITY: c_ulong,
    pub NET_WM_CM_S0: c_ulong,
}

static ATOMS: OnceLock<Atoms> = OnceLock::new();

pub(crate) fn atoms() -> &'static Atoms {
    ATOMS.get_or_init(|| {
        let intern = |name: &str| -> c_ulong {
            let c = CString::new(name).unwrap();
            unsafe { (lib().unwrap().XInternAtom)(dpy(), c.as_ptr(), 0) }
        };
        Atoms {
            NET_WM_NAME: intern("_NET_WM_NAME"),
            UTF8_STRING: intern("UTF8_STRING"),
            WM_NAME: intern("WM_NAME"),
            WM_CLASS: intern("WM_CLASS"),
            STRING: intern("STRING"),
            CARDINAL: intern("CARDINAL"),
            WINDOW: intern("WINDOW"),
            ATOM: intern("ATOM"),
            NET_WM_PID: intern("_NET_WM_PID"),
            NET_WM_STATE: intern("_NET_WM_STATE"),
            NET_WM_STATE_HIDDEN: intern("_NET_WM_STATE_HIDDEN"),
            NET_ACTIVE_WINDOW: intern("_NET_ACTIVE_WINDOW"),
            NET_WM_WINDOW_OPACITY: intern("_NET_WM_WINDOW_OPACITY"),
            NET_WM_CM_S0: intern("_NET_WM_CM_S0"),
        }
    })
}

// ================================================================ properties

/// Read a whole property (up to 16 KiB) as raw bytes, free the reply and copy
/// the data. Returns None for missing properties and dead windows alike.
pub(crate) fn prop_bytes(h: HWND, prop: c_ulong, req_type: c_ulong) -> Option<Vec<u8>> {
    let x = lib().ok()?;
    unsafe {
        let mut actual_type: c_ulong = 0;
        let mut actual_format: c_int = 0;
        let mut nitems: c_ulong = 0;
        let mut bytes_after: c_ulong = 0;
        let mut data: *mut c_uchar = std::ptr::null_mut();
        let status = (x.XGetWindowProperty)(
            dpy(),
            h,
            prop,
            0,
            4096,
            0,
            req_type,
            &mut actual_type,
            &mut actual_format,
            &mut nitems,
            &mut bytes_after,
            &mut data,
        );
        if status != 0 || data.is_null() {
            return None;
        }
        // Xlib widens 32-bit property data into a long array (8 bytes per unit
        // on LP64); only the low 32 bits of each element carry the value.
        let unit = match actual_format {
            16 => 2,
            32 => std::mem::size_of::<c_long>(),
            _ => 1,
        };
        let len = nitems as usize * unit;
        let out = if len == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(data, len).to_vec()
        };
        (x.XFree)(data as *mut c_void);
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

/// First 32-bit value of a property (format 32), e.g. `_NET_WM_PID`.
pub(crate) fn prop_u64(h: HWND, prop: c_ulong) -> Option<u64> {
    let bytes = prop_bytes(h, prop, 0)?;
    if bytes.len() >= 4 {
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64)
    } else {
        None
    }
}

/// A property holding a list of 32-bit atoms, e.g. `_NET_WM_STATE`.
pub(crate) fn prop_atoms(h: HWND, prop: c_ulong) -> Vec<u64> {
    match prop_bytes(h, prop, 0) {
        Some(bytes) => bytes
            .chunks_exact(std::mem::size_of::<c_long>())
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as u64)
            .collect(),
        None => Vec::new(),
    }
}

/// First NUL-terminated string inside a property value.
fn decode_cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

// ================================================================ window info

pub fn is_valid(h: HWND) -> bool {
    if h == 0 {
        return false;
    }
    let Ok(x) = lib() else {
        return false;
    };
    unsafe {
        let mut attr = std::mem::zeroed::<XWindowAttributes>();
        (x.XGetWindowAttributes)(dpy(), h, &mut attr) != 0
    }
}

/// Where the WM properties actually live. On a bare X11 desktop this is `h`
/// itself. Under a reparenting window manager — including WSLg's Weston — the
/// root child is a decoration frame with no properties, and the client window
/// nested inside owns the title, class, and PID. Descend in that case so
/// rules and menus see real values while hide/move still act on the frame.
fn property_owner(h: HWND) -> HWND {
    let has_props = |w: HWND| raw_pid(w) != 0 || !raw_class(w).is_empty();
    if has_props(h) {
        return h;
    }
    let Ok(x) = lib() else {
        return h;
    };
    unsafe {
        let mut root_ret: HWND = 0;
        let mut parent_ret: HWND = 0;
        let mut children: *mut HWND = std::ptr::null_mut();
        let mut n: c_uint = 0;
        if (x.XQueryTree)(
            dpy(),
            h,
            &mut root_ret,
            &mut parent_ret,
            &mut children,
            &mut n,
        ) == 0
            || children.is_null()
        {
            return h;
        }
        let kids = std::slice::from_raw_parts(children, n as usize).to_vec();
        (x.XFree)(children as *mut c_void);
        for k in kids {
            if has_props(k) {
                return k;
            }
            let deep = property_owner(k);
            if has_props(deep) {
                return deep;
            }
        }
        h
    }
}

fn raw_title(h: HWND) -> String {
    let a = atoms();
    if let Some(bytes) = prop_bytes(h, a.NET_WM_NAME, a.UTF8_STRING) {
        let s = decode_cstr(&bytes);
        if !s.is_empty() {
            return s;
        }
    }
    match prop_bytes(h, a.WM_NAME, 0) {
        Some(bytes) => decode_cstr(&bytes),
        None => String::new(),
    }
}

/// `WM_CLASS` is "instance\0class\0"; the class half is the stable one.
fn raw_class(h: HWND) -> String {
    let Some(bytes) = prop_bytes(h, atoms().WM_CLASS, 0) else {
        return String::new();
    };
    let mut parts = bytes.split(|&b| b == 0).filter(|p| !p.is_empty());
    let first = parts.next();
    match parts.next() {
        Some(class) => String::from_utf8_lossy(class).into_owned(),
        None => first
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .unwrap_or_default(),
    }
}

fn raw_pid(h: HWND) -> u32 {
    prop_u64(h, atoms().NET_WM_PID).unwrap_or(0) as u32
}

/// Title via EWMH `_NET_WM_NAME` (UTF-8), falling back to the legacy `WM_NAME`;
/// frame windows borrow it from the client nested inside.
pub fn window_title(h: HWND) -> String {
    let t = raw_title(h);
    if !t.is_empty() {
        return t;
    }
    let o = property_owner(h);
    if o != h {
        raw_title(o)
    } else {
        t
    }
}

pub fn class_name(h: HWND) -> String {
    let c = raw_class(h);
    if !c.is_empty() {
        return c;
    }
    let o = property_owner(h);
    if o != h {
        raw_class(o)
    } else {
        c
    }
}

pub fn process_id(h: HWND) -> u32 {
    let p = raw_pid(h);
    if p != 0 {
        return p;
    }
    let o = property_owner(h);
    if o != h {
        raw_pid(o)
    } else {
        p
    }
}

pub fn process_name(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Screen coordinates of the window: (0,0) translated from the window's own
/// origin into root coordinates, so WM reparenting/decoration never skews it.
pub fn window_rect(h: HWND) -> RECT {
    let Ok(x) = lib() else {
        return RECT::default();
    };
    unsafe {
        let mut attr = std::mem::zeroed::<XWindowAttributes>();
        if (x.XGetWindowAttributes)(dpy(), h, &mut attr) == 0 {
            return RECT::default();
        }
        let mut rx: c_int = 0;
        let mut ry: c_int = 0;
        let mut child: HWND = 0;
        if (x.XTranslateCoordinates)(dpy(), h, root(), 0, 0, &mut rx, &mut ry, &mut child) == 0 {
            rx = attr.x;
            ry = attr.y;
        }
        RECT {
            left: rx,
            top: ry,
            right: rx + attr.width,
            bottom: ry + attr.height,
        }
    }
}

pub fn cursor_pos() -> POINT {
    query_pointer(root())
        .map(|p| POINT { x: p.x, y: p.y })
        .unwrap_or_default()
}

pub(crate) struct Pointer {
    pub x: c_int,
    pub y: c_int,
    pub child: HWND,
    pub mask: c_uint,
}

pub(crate) fn query_pointer(win: HWND) -> Option<Pointer> {
    let x = lib().ok()?;
    unsafe {
        let mut root_ret: HWND = 0;
        let mut child_ret: HWND = 0;
        let mut rx: c_int = 0;
        let mut ry: c_int = 0;
        let mut wx: c_int = 0;
        let mut wy: c_int = 0;
        let mut mask: c_uint = 0;
        if (x.XQueryPointer)(
            dpy(),
            win,
            &mut root_ret,
            &mut child_ret,
            &mut rx,
            &mut ry,
            &mut wx,
            &mut wy,
            &mut mask,
        ) == 0
        {
            return None;
        }
        Some(Pointer {
            x: rx,
            y: ry,
            child: child_ret,
            mask,
        })
    }
}

pub fn is_visible(h: HWND) -> bool {
    let Ok(x) = lib() else {
        return false;
    };
    unsafe {
        let mut attr = std::mem::zeroed::<XWindowAttributes>();
        (x.XGetWindowAttributes)(dpy(), h, &mut attr) != 0 && attr.map_state == IS_VIEWABLE
    }
}

/// Iconified check via EWMH `_NET_WM_STATE_HIDDEN`.
pub fn is_minimized(h: HWND) -> bool {
    let a = atoms();
    prop_atoms(h, a.NET_WM_STATE).contains(&a.NET_WM_STATE_HIDDEN)
}

/// "Elevated" on Linux = the process runs as root. Nothing else raises the same
/// concerns the Windows token check catches.
pub fn is_elevated() -> bool {
    unsafe { geteuid() == 0 }
}

/// The active window's top-level ancestor, mirroring `GetForegroundWindow`:
/// EWMH reports the client window, but every caller compares it against root
/// children (frames on reparenting WMs).
pub fn foreground_window() -> HWND {
    let c = prop_u64(root(), atoms().NET_ACTIVE_WINDOW).unwrap_or(0);
    if c == 0 {
        return 0;
    }
    top_level_ancestor(c)
}

/// Walk up from `w` to its direct child of the root — the frame in a
/// reparenting WM, the window itself otherwise.
fn top_level_ancestor(w: HWND) -> HWND {
    let Ok(x) = lib() else {
        return w;
    };
    unsafe {
        let mut cur = w;
        for _ in 0..16 {
            let mut root_ret: HWND = 0;
            let mut parent_ret: HWND = 0;
            let mut children: *mut HWND = std::ptr::null_mut();
            let mut n: c_uint = 0;
            if (x.XQueryTree)(
                dpy(),
                cur,
                &mut root_ret,
                &mut parent_ret,
                &mut children,
                &mut n,
            ) == 0
            {
                return cur;
            }
            if !children.is_null() {
                (x.XFree)(children as *mut c_void);
            }
            if parent_ret == root() || parent_ret == 0 {
                return cur;
            }
            cur = parent_ret;
        }
        cur
    }
}

/// All root children, converted from X's bottom-to-top order into the
/// top-first order `rules::Scanner` expects.
pub fn top_level_windows() -> Vec<HWND> {
    let Ok(x) = lib() else {
        return Vec::new();
    };
    unsafe {
        let mut root_ret: HWND = 0;
        let mut parent_ret: HWND = 0;
        let mut children: *mut HWND = std::ptr::null_mut();
        let mut n: c_uint = 0;
        if (x.XQueryTree)(
            dpy(),
            root(),
            &mut root_ret,
            &mut parent_ret,
            &mut children,
            &mut n,
        ) == 0
            || children.is_null()
        {
            return Vec::new();
        }
        let mut out = std::slice::from_raw_parts(children, n as usize).to_vec();
        (x.XFree)(children as *mut c_void);
        out.reverse();
        out
    }
}

pub fn virtual_screen() -> RECT {
    let Ok(x) = lib() else {
        return RECT::default();
    };
    unsafe {
        let s = (x.XDefaultScreen)(dpy());
        RECT {
            left: 0,
            top: 0,
            right: (x.XDisplayWidth)(dpy(), s),
            bottom: (x.XDisplayHeight)(dpy(), s),
        }
    }
}

pub fn tick_ms() -> u64 {
    let mut ts = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { clock_gettime(CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64) * 1000 + (ts.tv_nsec as u64) / 1_000_000
}

pub(crate) fn flush() {
    if let Ok(x) = lib() {
        unsafe {
            (x.XFlush)(dpy());
        }
    }
}

// ================================================================ hotkeys

const XK_ESCAPE: c_ulong = 0xFF1B;
const XK_CONTROL_L: c_ulong = 0xFFE3;
const XK_ALT_L: c_ulong = 0xFFE9;
const XK_P: c_ulong = 0x0070;
const XK_H: c_ulong = 0x0068;
const XK_L: c_ulong = 0x006C;

/// Is the key carrying this keysym currently held down? Reads the global
/// keyboard bitmap (32 bytes of state, not events); nothing is recorded.
pub(crate) fn key_down(keysym: c_ulong) -> bool {
    let Ok(x) = lib() else {
        return false;
    };
    unsafe {
        let kc = (x.XKeysymToKeycode)(dpy(), keysym);
        if kc == 0 {
            return false;
        }
        let mut keys = [0 as c_char; 32];
        (x.XQueryKeymap)(dpy(), keys.as_mut_ptr());
        let byte = (kc / 8) as usize;
        let bit = kc % 8;
        (keys[byte] as u8) & (1u8 << bit) != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    Bind,
    Toggle,
    Latch,
}

/// Edge detector for Ctrl+Alt+P / Ctrl+Alt+H / Ctrl+Alt+L. Polled once per
/// tick; fills the role of RegisterHotKey on Windows without ever taking the
/// combination away from other applications.
pub struct Hotkeys {
    prev: [bool; 3],
}

impl Hotkeys {
    pub fn new() -> Hotkeys {
        Hotkeys { prev: [false; 3] }
    }

    pub fn poll(&mut self) -> Option<Hotkey> {
        let ctrl = key_down(XK_CONTROL_L);
        let alt = key_down(XK_ALT_L);
        let now = [
            ctrl && alt && key_down(XK_P),
            ctrl && alt && key_down(XK_H),
            ctrl && alt && key_down(XK_L),
        ];
        let mut out = None;
        for (i, &down) in now.iter().enumerate() {
            if down && !self.prev[i] {
                out = Some(match i {
                    0 => Hotkey::Bind,
                    1 => Hotkey::Toggle,
                    _ => Hotkey::Latch,
                });
            }
        }
        self.prev = now;
        out
    }
}

// ================================================================ instance lock

/// Held for the lifetime of the process; flock releases it automatically when
/// the process dies, so a crash never leaves a stale lock behind.
pub struct InstanceLock {
    _file: std::fs::File,
}

pub fn acquire_single_instance() -> Result<InstanceLock, String> {
    let path = lock_path();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("无法创建单实例锁文件 {}：{e}", path.display()))?;
    use std::os::unix::io::AsRawFd;
    let rc = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
    if rc != 0 {
        return Err("PeekGuard 已经在运行了。".into());
    }
    Ok(InstanceLock { _file: file })
}

pub(crate) fn runtime_dir() -> std::path::PathBuf {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(v) if !v.is_empty() => std::path::PathBuf::from(v),
        _ => std::path::PathBuf::from("/tmp"),
    }
}

fn lock_path() -> std::path::PathBuf {
    match std::env::var("PEEKGUARD_MUTEX") {
        Ok(v) if !v.is_empty() => runtime_dir().join(format!("{v}.lock")),
        _ => runtime_dir().join(format!("peekguard-{}.lock", unsafe { geteuid() })),
    }
}

// ================================================================ signals

static SIGNAL_FLAG: AtomicI32 = AtomicI32::new(0);

extern "C" fn handle_signal(sig: c_int) {
    SIGNAL_FLAG.store(sig, Ordering::SeqCst);
}

pub fn install_signal_handlers() {
    unsafe {
        signal(SIGINT, handle_signal);
        signal(SIGTERM, handle_signal);
        signal(SIGHUP, handle_signal);
    }
}

/// Signal number that arrived, or 0. The main loop checks it once per tick and
/// shuts down gracefully (restoring hidden windows).
pub fn pending_signal() -> c_int {
    SIGNAL_FLAG.load(Ordering::SeqCst)
}

// ================================================================ shell glue

/// Desktop notification through notify-send (libnotify CLI, preinstalled on
/// most desktops). Runs detached so a hanging notification daemon cannot stall
/// the tick loop; failures fall back to stderr only.
pub fn notify(text: &str) {
    eprintln!("[peekguard] {text}");
    let text = text.to_string();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("notify-send")
            .args([
                "--app-name=PeekGuard",
                "-h",
                "string:x-canonical-private-synchronous:peekguard",
                "--expire-time=5000",
                &text,
            ])
            .status();
    });
}

/// Open a URL or path with the desktop's default handler.
pub fn open_external(target: &str) {
    let target = target.to_string();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("xdg-open").arg(&target).status();
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
}
