//! Empty eframe window, to separate framework/driver cost from app cost.
//! Uses the same renderer configuration and bench harness as the app.
#![allow(dead_code)]

#[path = "../src/bench.rs"]
mod bench;
#[path = "../src/renderer.rs"]
mod renderer;
#[path = "../src/win.rs"]
mod win;

fn main() -> eframe::Result {
    let t0 = std::time::Instant::now();
    let mut options = eframe::NativeOptions::default();
    renderer::configure(&mut options);
    let mut started = false;
    eframe::run_ui_native("baseline", options, move |ui, _| {
        ui.label(format!("hello {:?}", t0.elapsed()));
        if !started {
            started = true;
            let (proc_ms, main_ms) = (win::ms_since_process_start(), t0.elapsed().as_secs_f64() * 1000.0);
            if let Some(out) = bench::path() {
                let hooks = bench::Hooks { trigger: None, latency_ms: Box::new(|| None) };
                let label = format!("baseline-{}", renderer::NAME);
                bench::start(ui.ctx().clone(), out, label, proc_ms, main_ms, hooks);
            }
        }
    })
}
