//! Window picker: the user clicks the window to protect.
//!
//! Unlike the X11 port this needs no pointer grab — the WindowServer hands
//! out a front-to-back window list, so picking is a plain hit test polled
//! each tick: the first left click after the grace period reports the
//! window under the pointer; a right click or Escape cancels.

use super::*;

pub enum Outcome {
    Continue,
    Pick(HWND),
    Cancel,
}

/// Clicks inside this window are treated as leftovers from whatever the user
/// was doing before the picker opened.
const GRACE_MS: u64 = 150;

pub struct Picker {
    started: u64,
}

impl Picker {
    /// macOS always succeeds: there is no grab to win.
    pub fn open() -> Option<Picker> {
        Some(Picker { started: tick_ms() })
    }

    pub fn tick(&mut self, now: u64) -> Outcome {
        if escape_down() {
            return Outcome::Cancel;
        }
        let elapsed = now.saturating_sub(self.started);
        if mouse_button_down(true) {
            return Outcome::Cancel;
        }
        if elapsed < GRACE_MS {
            return Outcome::Continue;
        }
        if mouse_button_down(false) {
            if let Some(h) = hit_test(cursor_pos()) {
                return Outcome::Pick(h);
            }
        }
        Outcome::Continue
    }
}

/// The topmost standard window under `pt`, or none.
fn hit_test(pt: POINT) -> Option<HWND> {
    let windows: Vec<(HWND, RECT)> = on_screen_list()
        .into_iter()
        .filter(|w| w.layer == 0)
        .map(|w| (w.id, w.rect))
        .collect();
    hit_test_in(pt, &windows)
}

/// Pure hit test over a front-to-back list; pickable in unit tests.
fn hit_test_in(pt: POINT, windows: &[(HWND, RECT)]) -> Option<HWND> {
    windows
        .iter()
        .find(|(_, r)| r.contains_with_margin(pt.x, pt.y, 0))
        .map(|(h, _)| *h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        }
    }

    #[test]
    fn picks_the_frontmost_window_under_the_pointer() {
        let windows = [
            (0x11 as HWND, rect(100, 100, 300, 300)),
            (0x22 as HWND, rect(200, 200, 400, 400)),
        ];
        assert_eq!(hit_test_in(POINT { x: 250, y: 250 }, &windows), Some(0x11));
        assert_eq!(hit_test_in(POINT { x: 350, y: 350 }, &windows), Some(0x22));
    }

    #[test]
    fn misses_outside_every_window() {
        let windows = [(0x11 as HWND, rect(100, 100, 300, 300))];
        assert_eq!(hit_test_in(POINT { x: 50, y: 50 }, &windows), None);
        assert_eq!(hit_test_in(POINT { x: 300, y: 300 }, &windows), None);
    }
}
