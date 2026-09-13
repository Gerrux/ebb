#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bar;
mod autostart;
mod bench;
mod card;
mod import_ui;
mod renderer;
mod search;
mod shell;
mod sticky;
mod store;
mod theme;
mod win;

use std::time::Instant;

use card::{Card, parse_capture};
use store::Store;

fn seed(store: &Store) -> Vec<Card> {
    const SAMPLES: &[&str] = &[
        "идея: Попробовать onboarding без обязательной регистрации #product #onboarding",
        "промпт: Product Critic\nТы — строгий продуктовый критик. Найди 5 самых слабых мест в идее и предложи, как их проверить за неделю. #ai",
        "vpn staging vpn.staging.internal #infra",
        "через две недели проверить новую pricing модель #pricing",
        "цель: Запустить MVP Ambient Notes\nПрототип → сплит на модули → импорт Sticky Notes #q4",
        "секрет: Wi-Fi офис\nguest / s3cret-pass",
        "https://www.egui.rs — демо виджетов #egui",
    ];
    let area = egui::vec2(1600.0, 900.0);
    let mut cards = Vec::new();
    for s in SAMPLES {
        let pos = card::free_slot(&cards, area);
        if let Ok(c) = store.insert(&parse_capture(s), pos) {
            cards.push(c);
        }
    }
    cards
}

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

    let store = Store::open().expect("open database");
    let mut cards = store.load().expect("load cards");
    if cards.is_empty() {
        cards = seed(&store);
    }

    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ambient")
            .with_inner_size([1280.0, 800.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_close_button(false)
            .with_minimize_button(false)
            .with_maximize_button(false),
        ..Default::default()
    };
    renderer::configure(&mut options);

    eframe::run_native(
        "Ambient",
        options,
        Box::new(move |cc| Ok(Box::new(app::AmbientApp::new(cc, store, cards, main_started, autostarted)))),
    )
}
