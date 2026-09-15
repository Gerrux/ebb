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
mod resurface;
mod rich_text;
mod search;
mod shell;
mod sticky;
mod store;
mod theme;
mod vault;
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

/// Drops what a panic message quotes: `…` and '…' spans can hold note text
/// (a string slice panic quotes the string), which must not reach a log.
fn without_quoted(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut open: Option<char> = None;
    for c in message.chars() {
        match open {
            Some(q) if c == q => {
                out.push('…');
                out.push(c);
                open = None;
            }
            Some(_) => {}
            None if c == '`' || c == '\'' => {
                out.push(c);
                open = Some(c);
            }
            None => out.push(c),
        }
    }
    if open.is_some() {
        out.push('…');
    }
    out
}

/// Appends a panic's place and message to `%LOCALAPPDATA%\Ebb\crash.log`
/// (release builds have no console, and `panic = "abort"` leaves nothing else).
/// Written before the abort; kept small, and without note text.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let at = resurface::unix_now();
        let (y, m, d) = search::civil_from_days(at.div_euclid(86_400));
        let secs = at.rem_euclid(86_400);
        let line = format!(
            "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC  v{}  thread {}  at {}: {}\n",
            secs / 3600,
            secs / 60 % 60,
            secs % 60,
            env!("CARGO_PKG_VERSION"),
            std::thread::current().name().unwrap_or("?"),
            info.location().map_or_else(|| "?".into(), |l| format!("{}:{}", l.file(), l.line())),
            without_quoted(&message),
        );
        let base = std::env::var_os("LOCALAPPDATA").map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
        let path = base.join("Ebb").join("crash.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024) {
            let _ = std::fs::remove_file(&path);
        }
        let _ = std::fs::create_dir_all(base.join("Ebb"));
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = std::io::Write::write_all(&mut f, line.as_bytes());
        }
        default(info);
    }));
}

#[cfg(test)]
#[test]
fn crash_log_drops_quoted_text() {
    assert_eq!(
        without_quoted("byte index 3 is not a char boundary; it is inside 'п' (bytes 2..4) of `пароль hunter2`"),
        "byte index 3 is not a char boundary; it is inside '…' (bytes 2..4) of `…`"
    );
    assert_eq!(without_quoted("called `Option::unwrap()` on a `None` value"), "called `…` on a `…` value");
    assert_eq!(without_quoted("unterminated `secret"), "unterminated `…");
}

fn main() -> eframe::Result {
    let main_started = Instant::now();
    install_crash_log();
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
    let fail = |what: &str, e: rusqlite::Error| -> ! {
        win::error_box(&format!(
            "Не удалось {what}:\n{e}\n\n{}\n\nЗаметки не тронуты. Проверь, что диск доступен и на нём есть место, и запусти Ebb снова.",
            store::db_path().display()
        ));
        std::process::exit(1)
    };
    let store = Store::open().unwrap_or_else(|e| fail("открыть базу заметок", e));
    // The day's Rediscover pick isn't made here: on 10k notes it takes ~40 ms.
    // The app runs it on a background thread shortly after the first frame.
    let cards = store.load().unwrap_or_else(|e| fail("прочитать заметки", e));

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
