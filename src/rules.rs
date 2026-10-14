//! Rule scan: keep rule-matched windows protected without a manual bind.
//!
//! `Config::rules` used to be write-only — the tray could save a rule and the
//! README advertised it, but nothing ever read the list back. This module is the
//! missing half: it periodically looks for a window the rules want protected and
//! hands it to the watcher.
//!
//! The watcher guards exactly one window at a time, so the scanner picks a single
//! best match and re-picks when that window goes away. Selection prefers the
//! foreground window, then the topmost match in z-order, so the choice is stable
//! rather than flickering between two candidates.
//!
//! Minimized windows are skipped entirely. `hide()` cannot act on one, so a
//! minimized window is useless as a guard target — and worse than useless if it is
//! picked: the watcher would be parked on a window that can never hide while a
//! perfectly hideable match (say the visible window the user is actually working
//! in) waits behind it. That is not hypothetical; it is exactly how a minimized
//! sibling window starved the rule during testing. The scan is cheap and repeats
//! every second, so a window minimized at scan time is simply picked up once it is
//! restored.

use crate::config::Config;
use crate::win::*;

/// How often to look for new rule matches.
///
/// Measured on a 10-window desktop (`which_part_costs_what` in the tests below),
/// one full scan costs roughly:
///
/// | part                              | cost   |
/// |-----------------------------------|--------|
/// | `EnumWindows`                     | 740 us |
/// | `process_name` (OpenProcess+query) | 570 us |
/// | class name                        | 37 us  |
/// | caption                           | 4 us   |
///
/// ~1.3 ms total, dominated by the per-window process lookup. At a 1 Hz interval
/// that is 0.13% of one tick, and the scan never touches the guarded window
/// itself. `SCAN_MS` is kept at 1000 rather than raised: latency to notice a newly
/// opened window matters more than this much CPU.
///
/// (Methodology note: an earlier measurement was ~3x this because the first
/// `EnumWindows` after process start pays for lazy API-set resolution. Always
/// warm up before timing — see `scan_costs_far_less_than_its_interval`.)
const SCAN_MS: u64 = 1000;

pub struct Scanner {
    /// Windows the user explicitly released. Suppressed per-handle, not globally:
    /// the rule "protect QQ" should still protect the *next* QQ window, but the
    /// one the user just pushed away must not snap back into a guard loop.
    suppressed: Vec<HWND>,
    /// Global pause, only ever set by the explicit "规则自动保护" menu toggle.
    muted: bool,
    next_scan_ms: u64,
}

impl Scanner {
    pub fn new() -> Scanner {
        Scanner {
            suppressed: Vec::new(),
            muted: false,
            next_scan_ms: 0,
        }
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn set_muted(&mut self, m: bool) {
        self.muted = m;
        if !m {
            // Resuming is also the user saying "forget the past": they may have
            // restarted the target, or replaced it with a new window.
            self.suppressed.clear();
        }
    }

    /// Stop guarding this particular window (explicit unbind, or a window that
    /// turned out to refuse being hidden).
    pub fn suppress(&mut self, hwnd: HWND) {
        if !self.suppressed.contains(&hwnd) {
            self.suppressed.push(hwnd);
        }
    }

    /// Drop suppression for a window the user has just bound by hand.
    fn unsuppress(&mut self, hwnd: HWND) {
        self.suppressed.retain(|h| *h != hwnd);
    }

    /// Forget dead handles. Windows handles get recycled, so dropping them as soon
    /// as they stop being valid keeps the list from suppressing an unrelated
    /// window that later reuses the same value.
    fn purge(&mut self) {
        self.suppressed.retain(|h| is_valid(*h));
    }

    /// Rate limit so the scan runs at most once per `SCAN_MS`.
    pub fn due(&mut self, now_ms: u64) -> bool {
        if now_ms < self.next_scan_ms {
            return false;
        }
        self.next_scan_ms = now_ms + SCAN_MS;
        true
    }

    /// The window the rules want protected, if any is eligible right now.
    pub fn find(&mut self, cfg: &Config) -> Option<HWND> {
        if self.muted || cfg.rules.is_empty() {
            return None;
        }
        self.purge();
        let fg = unsafe { GetForegroundWindow() };
        let own_pid = std::process::id();
        let mut best: Option<HWND> = None;
        for hwnd in top_level_windows() {
            if self.suppressed.contains(&hwnd) || process_id(hwnd) == own_pid {
                continue;
            }
            // Hidden covers both "minimized" and "not visible at all", and either
            // way the window cannot be hidden usefully: picking it would park the
            // guard on something unbeatable and starve the other matches.
            if !is_visible(hwnd) || is_minimized(hwnd) {
                continue;
            }
            let proc = process_name(process_id(hwnd));
            let cls = class_name(hwnd);
            let title = window_title(hwnd);
            if !cfg.rules.iter().any(|r| r.matches(&proc, &cls, &title)) {
                continue;
            }
            if hwnd == fg {
                return Some(hwnd); // the one the user is looking at wins
            }
            if best.is_none() {
                best = Some(hwnd); // EnumWindows walks top-to-bottom in z-order
            }
        }
        best
    }

    /// Called when the watcher takes a window: it is wanted again.
    pub fn note_bound(&mut self, hwnd: HWND) {
        self.unsuppress(hwnd);
    }
}

impl Default for Scanner {
    fn default() -> Self {
        Scanner::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muted_scanner_finds_nothing() {
        let mut s = Scanner::new();
        s.set_muted(true);
        let mut cfg = Config::default();
        cfg.add_rule(crate::config::Rule::parse("process:whatever.exe").unwrap());
        assert!(s.find(&cfg).is_none());
    }

    #[test]
    fn empty_rules_find_nothing() {
        let mut s = Scanner::new();
        assert!(s.find(&Config::default()).is_none());
    }

    #[test]
    fn scan_is_rate_limited_but_repeats() {
        let mut s = Scanner::new();
        assert!(s.due(0), "first scan should run immediately");
        assert!(!s.due(1), "second call within the window is suppressed");
        assert!(!s.due(SCAN_MS - 1));
        assert!(s.due(SCAN_MS), "and it opens up again after the interval");
    }

    #[test]
    fn suppression_is_per_window_and_reversible() {
        let mut s = Scanner::new();
        let a = 0x1000 as HWND;
        let b = 0x2000 as HWND;
        s.suppress(a);
        assert!(s.suppressed.contains(&a));
        s.suppress(a);
        assert_eq!(
            s.suppressed.len(),
            1,
            "suppressing twice must not duplicate"
        );
        s.suppress(b);
        assert_eq!(s.suppressed.len(), 2);
        s.note_bound(a);
        assert!(!s.suppressed.contains(&a));
        assert!(s.suppressed.contains(&b), "other windows stay suppressed");
    }

    #[test]
    fn unmuting_forgets_suppression() {
        let mut s = Scanner::new();
        s.set_muted(true);
        s.suppress(0x1234 as HWND);
        s.set_muted(false);
        assert!(s.suppressed.is_empty());
    }

    #[test]
    fn purge_drops_dead_handles() {
        let mut s = Scanner::new();
        s.suppress(0xdead as HWND); // not a real window
        s.purge();
        assert!(s.suppressed.is_empty());
    }

    /// Perf guard rather than a micro-benchmark. The point is to catch a gross
    /// regression — a blocking call entering the per-window loop, or
    /// `window_title` moved in front of the cheap process-name filter — not to
    /// measure the machine.
    ///
    /// The budget therefore scales with the desktop, because the scan's cost
    /// does: one `OpenProcess` plus one `SendMessageTimeout` per top-level
    /// window is the floor, so a flat millisecond cap only passes on a quiet
    /// machine with few windows (it used to read 25 ms and failed at 27 ms
    /// with 23 windows open). The reported time is the fastest pass, not the
    /// average, so a single pass that lands on CPU contention cannot fail an
    /// otherwise healthy run.
    #[test]
    fn scan_costs_far_less_than_its_interval() {
        let mut cfg = Config::default();
        // Matches nothing, so this measures the worst case: every window is
        // examined and rejected.
        cfg.add_rule(crate::config::Rule::parse("process:definitely-not-running-xyz.exe").unwrap());
        let mut s = Scanner::new();
        // Warm up: the first scan pays lazy API-set resolution and can be several
        // times slower than the steady state, which would make this test flaky
        // rather than informative. Also covers the empty-desktop case.
        for _ in 0..3 {
            let _ = s.find(&cfg);
        }

        let runs = 20;
        let mut per_scan: Option<std::time::Duration> = None;
        for _ in 0..runs {
            let t0 = std::time::Instant::now();
            assert!(s.find(&cfg).is_none());
            let d = t0.elapsed();
            per_scan = Some(per_scan.map_or(d, |p| p.min(d)));
        }
        let per_scan = per_scan.unwrap();
        let n = top_level_windows().len();
        println!("scan: {per_scan:?} over {n} windows (fastest of {runs})");
        let budget = std::time::Duration::from_millis(n as u64 * 3 + 10);
        assert!(
            per_scan <= budget,
            "one scan took {per_scan:?} over {n} windows against a {budget:?} budget"
        );
    }
}

#[cfg(test)]
mod bench_parts {
    use crate::win::*;
    use std::time::Instant;

    /// Diagnostic, not a test: measures where a scan spends its time and prints the
    /// breakdown. Ignored by default because it asserts nothing.
    ///
    ///     cargo test --release -- --ignored --nocapture which_part
    ///
    /// The slowest step is the per-window `OpenProcess`; `EnumWindows` is second.
    /// If scanning ever needs to get cheaper, cache `process_name` by pid.
    #[test]
    #[ignore]
    fn which_part_costs_what() {
        let ws = top_level_windows();
        let n = ws.len().max(1);

        let t = Instant::now();
        for _ in 0..10 {
            let _ = top_level_windows();
        }
        println!("enum      : {:?}/scan ({} windows)", t.elapsed() / 10, n);

        let t = Instant::now();
        for _ in 0..10 {
            for w in &ws {
                let _ = process_id(*w);
            }
        }
        println!("pid       : {:?}/scan", t.elapsed() / 10);

        let t = Instant::now();
        for _ in 0..10 {
            for w in &ws {
                let _ = window_title(*w);
            }
        }
        println!("title(smto): {:?}/scan", t.elapsed() / 10);

        let t = Instant::now();
        for _ in 0..10 {
            for w in &ws {
                let _ = process_name(process_id(*w));
            }
        }
        println!("procname  : {:?}/scan", t.elapsed() / 10);

        let t = Instant::now();
        for _ in 0..10 {
            for w in &ws {
                let _ = class_name(*w);
            }
        }
        println!("class     : {:?}/scan", t.elapsed() / 10);
    }
}
