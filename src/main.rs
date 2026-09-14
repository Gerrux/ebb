#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bar;
mod autostart;
mod bench;
mod card;
mod emoji;
mod import_ui;
mod library;
mod renderer;
mod search;
mod shell;
mod sticky;
mod store;
mod theme;
mod win;

use std::time::Instant;

use store::Store;

/// `--autostart-on|off|run|status`: manage the logon task without the UI.
/// Exit code 0 on success (for `status`: 0 = on, 1 = off), 2 on error.
fn autostart_cli(arg: &str) -> Option<i32> {
    let result = match arg {
        "--autostart-on" => autostart::enable().map(|_| 0),
        "--autostart-off" => autostart::disable().map(|_| 0),
        "--autostart-run" => autostart::run_now().map(|_| 0),
        "--autostart-status" => autostart::status().map(|s| {
            println!("{s:?}");
            if matches!(s, autostart::Status::Off) { 1 } else { 0 }
        }),
        _ => return None,
    };
    Some(result.unwrap_or_else(|e| {
        eprintln!("{arg}: {e}");
        2
    }))
}

fn main() -> eframe::Result {
    let main_started = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = args.first().and_then(|a| autostart_cli(a)) {
        std::process::exit(code);
    }
    let autostarted = args.iter().any(|a| a == autostart::FLAG);
    // Before opening the database or creating any window: a second launch only
    // asks the running instance to show its layer (or, with --quit, to exit).
    let quit = args.iter().any(|a| a == "--quit");
    if !shell::claim_single_instance() {
        shell::send_to_existing(if quit { shell::Request::Quit } else { shell::Request::ShowLayer });
        return Ok(());
    }
    if quit {
        return Ok(()); // nothing running
    }

    store::migrate_legacy_data();
    let store = Store::open().expect("open database");
    let cards = store.load().expect("load cards");

    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ebb")
            .with_inner_size([1280.0, 800.0])
            .with_decorations(false)
            .with_transparent(true)
            // The layer always covers a monitor's work area: no edge resizing.
            .with_resizable(false)
            .with_taskbar(false)
            .with_close_button(false)
            .with_minimize_button(false)
            .with_maximize_button(false),
        ..Default::default()
    };
    renderer::configure(&mut options);

    eframe::run_native(
        "Ebb",
        options,
        Box::new(move |cc| Ok(Box::new(app::EbbApp::new(cc, store, cards, main_started, autostarted)))),
    )
}
