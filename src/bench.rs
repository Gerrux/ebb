//! Scripted measurement run, enabled by `EBB_BENCH=<csv path>`.
//!
//! Timeline after the first frame: idle for `IDLE` (CPU time sampled across it),
//! optionally fire the capture hotkey path and sample memory with the bar up, hide
//! it, open search the same way and sample again, then close. One CSV row per run.
//! Alongside `latency_ms`/`search_latency_ms` (hotkey to first frame), `shown_ms`/
//! `search_shown_ms` poll for the bar window actually becoming visible on screen,
//! catching regressions (e.g. a stray `DwmFlush`) that delay the visible show
//! without showing up in the frame latency.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::win;

const SETTLE: Duration = Duration::from_millis(1000);
const IDLE: Duration = Duration::from_millis(3000);
const CAPTURE_UP: Duration = Duration::from_millis(1500);

/// Polls (1ms, bench-only) from `start` until the bar window is visible on
/// screen, up to `CAPTURE_UP`; NAN on timeout. Re-finds the window each call
/// since the bar can be hidden-but-existing (pre-created, just shown/hidden).
fn wait_shown(start: Instant) -> f64 {
    loop {
        if let Some(h) = win::find_capture_window() {
            if win::is_window_visible(h) {
                return start.elapsed().as_secs_f64() * 1000.0;
            }
        }
        if start.elapsed() >= CAPTURE_UP {
            return f64::NAN;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

// New columns go at the end: older builds write fewer columns into the same CSV
// (see scripts/bench.ps1), and their rows must still line up under the header.
pub const HEADER: &str = "label,proc_ms,main_ms,ws_first,priv_first,ws_idle,priv_idle,cpu_idle_ms,latency_ms,ws_capture,priv_capture,search_latency_ms,ws_search,priv_search,shown_ms,search_shown_ms";

/// `EBB_BENCH=<csv>`, or `--bench=<csv>` for launchers that cannot set the
/// environment (Task Scheduler).
pub fn path() -> Option<PathBuf> {
    std::env::var_os("EBB_BENCH").map(PathBuf::from).or_else(|| {
        std::env::args().find_map(|a| a.strip_prefix("--bench=").map(PathBuf::from))
    })
}

pub struct Hooks {
    /// Simulates a hotkey press (`true` = search, `false` = capture); `None` for
    /// apps without the bar.
    pub trigger: Option<Box<dyn Fn(bool) + Send>>,
    pub latency_ms: Box<dyn Fn() -> Option<f64> + Send>,
}

/// Call once, from the first frame.
pub fn start(ctx: egui::Context, out: PathBuf, label: String, proc_ms: f64, main_ms: f64, hooks: Hooks) {
    let (ws_first, priv_first) = win::memory_mib();
    std::thread::spawn(move || {
        std::thread::sleep(SETTLE);
        let cpu0 = win::cpu_ms();
        std::thread::sleep(IDLE);
        let cpu_idle = win::cpu_ms() - cpu0;
        let (ws_idle, priv_idle) = win::memory_mib();

        let (mut latency, mut shown, mut ws_cap, mut priv_cap) = (f64::NAN, f64::NAN, f64::NAN, f64::NAN);
        let (mut search_latency, mut search_shown, mut ws_search, mut priv_search) =
            (f64::NAN, f64::NAN, f64::NAN, f64::NAN);
        if let Some(trigger) = &hooks.trigger {
            let t0 = Instant::now();
            trigger(false);
            shown = wait_shown(t0);
            std::thread::sleep(CAPTURE_UP.saturating_sub(t0.elapsed()));
            latency = (hooks.latency_ms)().unwrap_or(f64::NAN);
            (ws_cap, priv_cap) = win::memory_mib();

            trigger(false); // hide
            std::thread::sleep(Duration::from_millis(400));
            let t1 = Instant::now();
            trigger(true);
            search_shown = wait_shown(t1);
            std::thread::sleep(CAPTURE_UP.saturating_sub(t1.elapsed()));
            search_latency = (hooks.latency_ms)().unwrap_or(f64::NAN);
            (ws_search, priv_search) = win::memory_mib();
        }

        let row = format!(
            "{label},{proc_ms:.0},{main_ms:.0},{ws_first:.1},{priv_first:.1},{ws_idle:.1},{priv_idle:.1},{cpu_idle:.0},{latency:.1},{ws_cap:.1},{priv_cap:.1},{search_latency:.1},{ws_search:.1},{priv_search:.1},{shown:.1},{search_shown:.1}\n"
        );
        let new = !out.exists();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&out) {
            if new {
                let _ = writeln!(f, "{HEADER}");
            }
            let _ = f.write_all(row.as_bytes());
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    });
}
