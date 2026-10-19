//! Platform dispatch layer.
//!
//! Everything that touches a foreign window goes through here. On Windows this
//! re-exports the hand-written Win32 bindings in `win.rs`; on Linux (X11) the
//! Xlib bindings in `x11/`. Both expose the same names for the shared
//! vocabulary — `HWND`, `RECT`, `POINT`, `is_valid`, `window_rect`, ... — so
//! the portable modules (`watch`, `rules`, `session`, `main`) never need to
//! know which one is active.
//!
//! Window operations are split out as submodule forwards (`plat::hide`,
//! `plat::picker`, `plat::autostart`) because their implementations diverge in
//! details the portable code does not need to see.

#[cfg(windows)]
pub use crate::win::*;

#[cfg(target_os = "linux")]
pub use crate::x11::*;

// Submodule forwards. The inner glob re-export is what makes this legal — the
// platform modules themselves are private to the crate.
#[cfg(windows)]
pub mod autostart {
    pub use crate::autostart::*;
}
#[cfg(target_os = "linux")]
pub mod autostart {
    pub use crate::x11::autostart::*;
}

#[cfg(windows)]
pub mod hide {
    pub use crate::hide::*;
}
#[cfg(target_os = "linux")]
pub mod hide {
    pub use crate::x11::hide::*;
}

#[cfg(windows)]
pub mod picker {
    pub use crate::picker::*;
}
#[cfg(target_os = "linux")]
pub mod picker {
    pub use crate::x11::picker::*;
}

// Mode display name ("透明" / "移出屏幕" / "隐藏窗口").
#[cfg(windows)]
pub use crate::tray::mode_name;
#[cfg(target_os = "linux")]
pub use crate::x11::mode_name;
