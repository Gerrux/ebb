//! Empty eframe window, to separate framework/driver cost from app cost.
fn main() -> eframe::Result {
    let t0 = std::time::Instant::now();
    eframe::run_ui_native("baseline", Default::default(), move |ui, _| {
        ui.label(format!("hello {:?}", t0.elapsed()));
    })
}
