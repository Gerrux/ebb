//! Scripted measurement run, enabled by `AMBIENT_BENCH=<csv path>`.
//!
//! Timeline after the first frame: idle for `IDLE` (CPU time sampled across it),
//! optionally fire the capture hotkey path, sample memory with the capture window
//! up, then close. One CSV row per run.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use crate::win;

const SETTLE: Duration = Duration::from_millis(1000);
const IDLE: Duration = Duration::from_millis(3000);
const CAPTURE_UP: Duration = Duration::from_millis(1500);

pub const HEADER: &str = "label,proc_ms,main_ms,ws_first,priv_first,ws_idle,priv_idle,cpu_idle_ms,latency_ms,ws_capture,priv_capture";

/// `AMBIENT_BENCH=<csv>`, or `--bench=<csv>` for launchers that cannot set the
/// environment (Task Scheduler).
pub fn path() -> Option<PathBuf> {
    std::env::var_os("AMBIENT_BENCH").map(PathBuf::from).or_else(|| {
        std::env::args().find_map(|a| a.strip_prefix("--bench=").map(PathBuf::from))
    })
}

pub struct Hooks {
    /// Simulates a hotkey press; `None` for apps without a capture window.
    pub trigger: Option<Box<dyn Fn() + Send>>,
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

        let (mut latency, mut ws_cap, mut priv_cap) = (f64::NAN, f64::NAN, f64::NAN);
        if let Some(trigger) = &hooks.trigger {
            trigger();
            std::thread::sleep(CAPTURE_UP);
            latency = (hooks.latency_ms)().unwrap_or(f64::NAN);
            (ws_cap, priv_cap) = win::memory_mib();
        }

        let row = format!(
            "{label},{proc_ms:.0},{main_ms:.0},{ws_first:.1},{priv_first:.1},{ws_idle:.1},{priv_idle:.1},{cpu_idle:.0},{latency:.1},{ws_cap:.1},{priv_cap:.1}\n"
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
