#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bench;
mod card;
mod renderer;
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

fn main() -> eframe::Result {
    let main_started = Instant::now();

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
            .with_close_button(false)
            .with_minimize_button(false)
            .with_maximize_button(false),
        ..Default::default()
    };
    renderer::configure(&mut options);

    eframe::run_native(
        "Ambient",
        options,
        Box::new(move |cc| Ok(Box::new(app::AmbientApp::new(cc, store, cards, main_started)))),
    )
}
