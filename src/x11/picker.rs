//! Crosshair window picker for X11.
//!
//! The Windows picker paints a full-screen colour-key window; X11 plays by
//! different rules, so this one grabs the pointer with a crosshair cursor
//! and polls `XQueryPointer` — the same polling philosophy as the rest of
//! the Linux backend.  While the grab is active, clicks are delivered to us
//! (we never read them; only the button mask is inspected) so they cannot
//! reach the window underneath, and the target application is reported
//! through `XQueryPointer`'s child window.

use std::os::raw::{c_int, c_uint};

use super::*;

/// XC_crosshair – font-cursor id used while picking.
const CROSSHAIR_CURSOR: c_int = 34;
/// Ignore clicks arriving right after opening (e.g. the button that opened
/// the picker is still held).
const GRACE_MS: u64 = 150;

const BUTTON1_MASK: u32 = 1 << 8;
const BUTTON3_MASK: u32 = 1 << 10;

pub enum Outcome {
    Continue,
    Pick(HWND),
    Cancel,
}

pub struct Picker {
    started: u64,
    cursor: Cursor,
}

impl Picker {
    /// Open the picker.  Returns `None` when the pointer grab fails.
    pub fn open() -> Option<Picker> {
        let x = lib().ok()?;
        let cursor = unsafe { (x.XCreateFontCursor)(dpy(), CROSSHAIR_CURSOR as c_uint) };
        if cursor == 0 {
            return None;
        }
        let grabbed = unsafe {
            (x.XGrabPointer)(
                dpy(),
                root(),
                0,      // owner_events: we only poll the button mask
                0,      // event_mask: no events needed
                1,      // pointer_mode: GrabModeAsync
                1,      // keyboard_mode: GrabModeAsync
                0,      // confine_to: none
                cursor, // shown while the grab is active
                0,      // CurrentTime
            )
        };
        if grabbed != 0 {
            // GrabSuccess == 0; anything else (AlreadyGrabbed, Frozen, ...)
            unsafe { (x.XFreeCursor)(dpy(), cursor) };
            return None;
        }
        Some(Picker {
            started: tick_ms(),
            cursor,
        })
    }

    /// One polling step; call every tick while the picker is open.
    pub fn tick(&mut self, now: u64) -> Outcome {
        if key_down(XK_ESCAPE) {
            return Outcome::Cancel;
        }
        let Some(p) = query_pointer(root()) else {
            return Outcome::Continue;
        };
        match classify(&p, now.saturating_sub(self.started)) {
            // A stale child (window closed mid-click) simply keeps the picker open.
            Outcome::Pick(h) if !is_valid(h) => Outcome::Continue,
            outcome => outcome,
        }
    }

    fn close(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if let Ok(x) = lib() {
            unsafe {
                (x.XUngrabPointer)(dpy(), 0);
                (x.XFreeCursor)(dpy(), self.cursor);
            }
            flush();
        }
        self.cursor = 0;
    }
}

/// What one polled pointer state means for the picker. Kept pure — no Xlib
/// calls — so the click rules stay unit-testable on a headless machine.
fn classify(p: &Pointer, elapsed_ms: u64) -> Outcome {
    if p.mask & BUTTON3_MASK != 0 {
        return Outcome::Cancel;
    }
    // The click that opened the picker may still be held; ignore it.
    if elapsed_ms < GRACE_MS {
        return Outcome::Continue;
    }
    if p.mask & BUTTON1_MASK != 0 && p.child != 0 {
        return Outcome::Pick(p.child);
    }
    Outcome::Continue
}

impl Drop for Picker {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer(mask: u32, child: HWND) -> Pointer {
        Pointer {
            x: 0,
            y: 0,
            child,
            mask,
        }
    }

    #[test]
    fn grace_period_swallows_the_opening_click() {
        assert!(matches!(
            classify(&pointer(BUTTON1_MASK, 0x11), 0),
            Outcome::Continue
        ));
        assert!(matches!(
            classify(&pointer(BUTTON1_MASK, 0x11), GRACE_MS - 1),
            Outcome::Continue
        ));
    }

    #[test]
    fn a_click_after_grace_picks_the_child() {
        match classify(&pointer(BUTTON1_MASK, 0x11), GRACE_MS) {
            Outcome::Pick(h) => assert_eq!(h, 0x11),
            _ => panic!("expected Pick"),
        }
    }

    #[test]
    fn right_button_cancels_at_any_time() {
        assert!(matches!(
            classify(&pointer(BUTTON3_MASK, 0x11), GRACE_MS + 1000),
            Outcome::Cancel
        ));
        assert!(matches!(
            classify(&pointer(BUTTON3_MASK, 0), 0),
            Outcome::Cancel
        ));
    }

    #[test]
    fn clicks_without_a_child_window_are_ignored() {
        assert!(matches!(
            classify(&pointer(BUTTON1_MASK, 0), GRACE_MS + 1000),
            Outcome::Continue
        ));
        assert!(matches!(
            classify(&pointer(0, 0x11), GRACE_MS + 1000),
            Outcome::Continue
        ));
    }
}
