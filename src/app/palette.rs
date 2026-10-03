//! What the command palette's commands do on the layer. The palette itself is
//! in `bar` (the list, the keys) and `commands` (what exists, what matches);
//! here each command is mapped onto the functions the app already has.
//!
//! Autostart goes through COM and a backup copies the database: both run on
//! their own thread and report back through a channel, as a toast.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;

use egui::{Key, Modifiers, Ui, ViewportCommand, ViewportId};

use crate::autostart;
use crate::commands::{self, CommandId, Ctx};
use crate::library::{self, Request, Tab};
use crate::resurface;
use crate::search;
use crate::shell::Event;
use crate::store::{Store, backup_dir};
use crate::win;

use super::EbbApp;
use super::toast::Toast;

/// How a background command ended.
enum Finished {
    /// The logon task's state after the toggle.
    Autostart(Result<bool, String>),
    Backup(Result<(), String>),
}

pub(super) struct Palette {
    tx: mpsc::Sender<Finished>,
    rx: mpsc::Receiver<Finished>,
    autostart_running: Arc<AtomicBool>,
    backup_running: Arc<AtomicBool>,
}

impl Default for Palette {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, autostart_running: Arc::default(), backup_running: Arc::default() }
    }
}

/// A background command in progress: claimed once, released when dropped, so
/// a thread that failed to start doesn't leave the command blocked for good.
struct Running(Arc<AtomicBool>);

impl Running {
    /// None while the command is still running from before.
    fn claim(flag: &Arc<AtomicBool>) -> Option<Running> {
        (!flag.swap(true, Ordering::AcqRel)).then(|| Running(flag.clone()))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Flips the logon task; returns what it is now.
fn toggle_autostart() -> Result<bool, String> {
    let on = matches!(autostart::status().map_err(|e| e.message())?, autostart::Status::On { .. });
    let changed = if on { autostart::disable() } else { autostart::enable() };
    changed.map_err(|e| e.message())?;
    Ok(!on)
}

impl EbbApp {
    /// What the palette needs to know about the app: cheap fields only.
    pub(super) fn command_ctx(&self) -> Ctx {
        // A card command on a layer nobody sees would act blind: its toast and
        // the undo in it show only there.
        let card = self.active_card().filter(|_| self.layer_visible && !self.collapsed);
        Ctx {
            has_active_card: card.is_some(),
            active_pinned: card.is_some_and(|c| c.pinned),
            layer_visible: self.layer_visible,
            collapsed: self.collapsed,
            pin_bottom: win::PIN_BOTTOM.load(Ordering::Relaxed),
            // Reading it is COM, and the settings window and the tray change it
            // too: the title stays neutral rather than risk being wrong.
            autostart: None,
            monitors: win::monitor_count(),
            tint: self.tint,
        }
    }

    /// Ctrl+K on the layer opens the palette, unless something is being typed.
    pub(super) fn palette_key(&mut self, ui: &mut Ui) {
        if ui.ctx().egui_wants_keyboard_input() || self.editing.is_some() {
            return;
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::K)) {
            // Handled in logic() like the hotkey's, on the next pass.
            self.shell.events.lock().unwrap().push(Event::Palette(Instant::now()));
            ui.ctx().request_repaint();
        }
    }

    /// Runs a palette command; `arg` is the number of one that takes it.
    pub(super) fn run_command(&mut self, ctx: &egui::Context, id: CommandId, arg: Option<i64>) {
        match id {
            // The bar switches itself for these, except the capture, which needs the
            // window resized: the same pass as the hotkey.
            CommandId::NewNote => {
                self.shell.events.lock().unwrap().push(Event::Capture(Instant::now()));
            }
            CommandId::Search
            | CommandId::ShowIdeas
            | CommandId::ShowPrompts
            | CommandId::ShowGoals
            | CommandId::ShowReminders
            | CommandId::ShowLinks => {}
            CommandId::OpenReview => self.open_library(ctx, Tab::Review),
            CommandId::OpenArchive => self.open_library(ctx, Tab::Archive),
            CommandId::OpenTrash => self.open_library(ctx, Tab::Trash),
            CommandId::OpenSettings => self.open_settings(ctx, false),
            CommandId::ToggleLayer => self.toggle_layer(ctx),
            CommandId::TogglePinBottom => {
                let on = !win::PIN_BOTTOM.load(Ordering::Relaxed);
                self.apply_library_request(ctx, Request::SetPinBottom(on));
            }
            CommandId::ToggleCurtain => self.toggle_curtain(ctx),
            CommandId::MoveToMonitor => {
                // "Монитор N" as the settings number them: the order of `win::monitors`.
                let device = arg
                    .and_then(|n| usize::try_from(n - 1).ok())
                    .and_then(|i| win::monitors().into_iter().nth(i))
                    .and_then(|m| m.device_ids.first().cloned());
                if let Some(device) = device {
                    self.apply_library_request(ctx, Request::SetMonitor(device));
                }
            }
            CommandId::Dimming => {
                if let Some(percent) = arg {
                    self.apply_library_request(ctx, Request::SetTint(commands::tint_from_percent(percent)));
                }
            }
            CommandId::NextBackdrop => self.cycle_backdrop(),
            CommandId::ImportSticky => self.apply_library_request(ctx, Request::ImportSticky),
            CommandId::ToggleAutostart => self.toggle_autostart_in_background(ctx),
            CommandId::BackupNow => self.backup_in_background(ctx),
            CommandId::Quit => ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close),
            CommandId::ArchiveCard => self.archive_active(),
            CommandId::TogglePinCard => self.toggle_pin_active(),
        }
        if matches!(
            id,
            CommandId::TogglePinBottom | CommandId::MoveToMonitor | CommandId::Dimming | CommandId::NextBackdrop
        ) {
            self.sync_library_settings(ctx);
        }
        ctx.request_repaint_of(ViewportId::ROOT);
    }

    /// An open settings window shows what a command just changed: its copy of
    /// the settings is taken when it opens.
    fn sync_library_settings(&self, ctx: &egui::Context) {
        if !self.library.lock().unwrap().open {
            return;
        }
        let settings = self.layer_settings();
        self.library.lock().unwrap().settings = settings;
        ctx.request_repaint_of(library::viewport_id());
    }

    /// One toggle at a time: two at once would both read "off" and both turn it on.
    fn toggle_autostart_in_background(&self, ctx: &egui::Context) {
        let Some(running) = Running::claim(&self.palette.autostart_running) else { return };
        let (tx, ctx) = (self.palette.tx.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("autostart".into()).spawn(move || {
            let result = toggle_autostart();
            drop(running);
            let _ = tx.send(Finished::Autostart(result));
            ctx.request_repaint_of(ViewportId::ROOT);
        });
        if let Err(e) = spawned {
            eprintln!("autostart: {e}");
        }
    }

    /// A copy of the database on its own thread and connection: `VACUUM INTO`
    /// can take longer than a frame may.
    fn backup_in_background(&self, ctx: &egui::Context) {
        let Some(running) = Running::claim(&self.palette.backup_running) else { return };
        let (tx, ctx) = (self.palette.tx.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("backup".into()).spawn(move || {
            let (now, offset) = (resurface::unix_now(), search::local_offset_secs());
            let result = Store::open()
                .and_then(|store| store.backup_into(&backup_dir(), now, offset))
                .map(|_| ())
                .map_err(|e| e.to_string());
            drop(running);
            let _ = tx.send(Finished::Backup(result));
            ctx.request_repaint_of(ViewportId::ROOT);
        });
        if let Err(e) = spawned {
            eprintln!("backup: {e}");
        }
    }

    /// Reports the background commands that have finished.
    pub(super) fn command_results(&mut self, ctx: &egui::Context) {
        while let Ok(finished) = self.palette.rx.try_recv() {
            let text = match finished {
                Finished::Autostart(Ok(true)) => "Запускать при входе: включено",
                Finished::Autostart(Ok(false)) => "Запускать при входе: выключено",
                Finished::Autostart(Err(e)) => {
                    eprintln!("autostart: {e}");
                    "Не удалось изменить автозапуск"
                }
                Finished::Backup(Ok(())) => "Резервная копия сохранена",
                Finished::Backup(Err(e)) => {
                    eprintln!("backup failed: {e}");
                    "Не удалось сделать резервную копию"
                }
            };
            self.toast = Some(Toast::new(text, None));
            // An open settings tab shows autostart and the last backup as they are now.
            self.library.lock().unwrap().reread_system_state();
            ctx.request_repaint_of(library::viewport_id());
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_background_command_runs_once_at_a_time_and_is_released() {
        let flag = Arc::new(AtomicBool::new(false));
        let first = Running::claim(&flag);
        assert!(first.is_some());
        assert!(Running::claim(&flag).is_none());
        drop(first);
        // Moved into a thread that never started (the closure is dropped): released.
        let claimed = Running::claim(&flag).unwrap();
        drop(move || drop(claimed));
        assert!(Running::claim(&flag).is_some());
    }
}
