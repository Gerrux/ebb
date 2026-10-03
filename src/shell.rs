//! Process-level shell integration: single instance, tray icon, global hotkeys.
//!
//! One background thread owns a hidden top-level window that receives tray
//! callbacks, `WM_HOTKEY`, `TaskbarCreated` (Explorer restarts) and the "show
//! the layer" request from a second instance. It blocks in `GetMessageW`, so it
//! costs nothing while idle. Results reach the UI as [`Event`]s plus a repaint
//! request, which runs `App::logic` even while the layer is hidden.
//!
//! Hotkeys are registered to that window only, so the UI asks for changes
//! ([`set_hotkey`], [`suspend_hotkeys`]) through a command queue in [`Shared`]
//! and a posted message, and hears back with [`Event::HotkeySet`]. Which
//! combinations to bind comes from [`crate::hotkey`].

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_LWIN, VK_RWIN,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NIN_SELECT, NINF_KEY,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, GetWindowThreadProcessId, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    FindWindowW, GetMessageW, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW,
    WM_DISPLAYCHANGE,
    RegisterClassExW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateMessage, WM_APP, WM_CONTEXTMENU, WM_HOTKEY, WM_NULL, WM_POWERBROADCAST, WM_SETTINGCHANGE,
    WM_TIMECHANGE,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::autostart;
use crate::hotkey::{self, Action, Combo, Config, Wanted};

/// Instance identity. `EBB_INSTANCE=<name>` runs a separate instance (own mutex
/// and shell window) next to the normal one, e.g. for testing against another
/// LOCALAPPDATA while the real app keeps running.
fn instance_suffix() -> String {
    std::env::var("EBB_INSTANCE").map(|s| format!(".{s}")).unwrap_or_default()
}

fn class_name() -> &'static [u16] {
    static NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    NAME.get_or_init(|| wide(&format!("Ebb.Shell{}", instance_suffix())))
}
/// `WM_SETTINGCHANGE` wparam: the work area changed (taskbar moved or resized).
const SPI_SETWORKAREA: usize = 0x002F;
/// Not in the windows crate's Dwm module.
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
const WM_TRAY: u32 = WM_APP + 1;
const WM_SHOW_LAYER: u32 = WM_APP + 2;
const WM_QUIT_APP: u32 = WM_APP + 3;
const WM_RETRY_HOTKEYS: u32 = WM_APP + 4;
const WM_HOTKEY_COMMANDS: u32 = WM_APP + 5;
const TRAY_ID: u32 = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
/// `WM_POWERBROADCAST`: resumed from sleep or hibernation.
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;

pub enum Event {
    /// Capture hotkey (or tray/menu equivalent) pressed at this instant.
    Capture(Instant),
    Search(Instant),
    /// Command palette hotkey: the search bar with `>` typed.
    Palette(Instant),
    ToggleLayer,
    /// Left click on the tray icon: summon the layer over the windows, or dismiss it.
    TrayClick,
    /// Ebb was started again (a shortcut) while running.
    Launched,
    TogglePinBottom,
    ImportSticky,
    /// Open the library window; `true` = on the settings tab.
    OpenLibrary(bool),
    /// Open the weekly review in the library window.
    OpenReview,
    /// Windows' light/dark mode or accent color changed.
    SystemColors,
    /// The bindings changed on their own (a taken hotkey was registered later, or
    /// they came back after a recording); see [`Shared::hotkeys`].
    HotkeysChanged,
    /// The answer to [`set_hotkey`]; `ok` is false when a custom combination was
    /// held by another program (the action then keeps what it had).
    HotkeySet { action: Action, wanted: Wanted, ok: bool },
    /// Back from sleep, or the clock or time zone changed: timers set for a
    /// wall-clock moment (the morning Rediscover) should look at the time again.
    ClockChanged,
    /// A monitor was plugged or unplugged, or its resolution, scale or work area
    /// changed (taskbar moved). Bursts are coalesced into one pending event.
    DisplayChanged,
    Exit,
}

/// State shared between the shell thread and the UI.
#[derive(Default)]
pub struct Shared {
    pub events: Mutex<Vec<Event>>,
    /// Mirrors of UI state, read when the tray menu opens.
    pub layer_visible: AtomicBool,
    pub pin_bottom: AtomicBool,
    /// Hotkeys registered so far; `None` until the shell thread is up.
    hotkeys: Mutex<Option<Hotkeys>>,
    /// Changes asked of the shell thread, in order; see [`WM_HOTKEY_COMMANDS`].
    commands: Mutex<Vec<Command>>,
    hwnd: AtomicIsize,
}

enum Command {
    Set(Action, Wanted),
    Suspend(bool),
}

/// How one action's hotkey stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Binding {
    #[default]
    Off,
    Bound(Combo),
    /// Held by another program: the wanted custom combination, or `None` when
    /// every default candidate was taken.
    Taken(Option<Combo>),
}

impl Binding {
    /// For the layer's header: the combination or why there is none.
    pub fn short(self) -> String {
        match self {
            Binding::Bound(c) => c.to_string(),
            Binding::Off => "хоткей выключен".to_owned(),
            Binding::Taken(_) => "хоткей занят".to_owned(),
        }
    }
}

/// Every action's [`Binding`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hotkeys {
    slots: [Binding; hotkey::ACTION_COUNT],
}

impl Hotkeys {
    pub fn get(&self, action: Action) -> Binding {
        self.slots[action.index()]
    }

    /// The combination the action holds now, if any.
    pub fn bound(&self, action: Action) -> Option<Combo> {
        match self.get(action) {
            Binding::Bound(c) => Some(c),
            _ => None,
        }
    }

    fn set(&mut self, action: Action, binding: Binding) {
        self.slots[action.index()] = binding;
    }

    /// Nothing registered (no shell window): what each wish comes to.
    fn unbound(config: Config) -> Self {
        let mut keys = Self::default();
        for a in Action::ALL {
            keys.set(
                a,
                match config[a.index()] {
                    Wanted::Default if a.defaults().is_empty() => Binding::Off,
                    Wanted::Default => Binding::Taken(None),
                    Wanted::Off => Binding::Off,
                    Wanted::Custom(c) => Binding::Taken(Some(c)),
                },
            );
        }
        keys
    }
}

impl Shared {
    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }

    pub fn hotkeys(&self) -> Option<Hotkeys> {
        *self.hotkeys.lock().unwrap()
    }
}

/// Asks the shell thread to try again for hotkeys that were taken when Ebb
/// started (the app holding them may have quit). Answered with
/// [`Event::HotkeysChanged`] if one could be registered now.
pub fn retry_hotkeys(shared: &Shared) {
    let raw = shared.hwnd.load(Ordering::Relaxed);
    if raw != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(raw as *mut c_void)), WM_RETRY_HOTKEYS, WPARAM(0), LPARAM(0));
        }
    }
}

/// Queues `command` for the shell thread. False if it has no window to post to.
fn post_command(shared: &Shared, command: Command) -> bool {
    let raw = shared.hwnd.load(Ordering::Relaxed);
    if raw == 0 {
        return false;
    }
    shared.commands.lock().unwrap().push(command);
    // Once queued the command runs, if not on this post then with the next
    // drain; reporting a failed post would answer it twice.
    let _ = unsafe { PostMessageW(Some(HWND(raw as *mut c_void)), WM_HOTKEY_COMMANDS, WPARAM(0), LPARAM(0)) };
    true
}

/// Asks the shell thread to bind `action` as `wanted`; never blocks. Answered
/// with [`Event::HotkeySet`].
pub fn set_hotkey(shared: &Shared, action: Action, wanted: Wanted) {
    if !post_command(shared, Command::Set(action, wanted)) {
        // No shell window (it failed to start): nothing can be bound.
        shared.events.lock().unwrap().push(Event::HotkeySet { action, wanted, ok: false });
    }
}

/// Unregisters all of Ebb's hotkeys (`true`) or binds them again as configured
/// (`false`), so a combination Ebb itself holds can be pressed in the settings
/// window while one is being recorded.
pub fn suspend_hotkeys(shared: &Shared, suspend: bool) {
    post_command(shared, Command::Suspend(suspend));
}

/// Whether the Win key is down right now; egui's modifiers don't have it.
pub fn win_key_down() -> bool {
    [VK_LWIN, VK_RWIN].into_iter().any(|vk| key_down(u32::from(vk.0)))
}

/// Whether the key with virtual-key code `vk` is down right now.
pub fn key_down(vk: u32) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

// ---------------------------------------------------------------------------
// Single instance
// ---------------------------------------------------------------------------

/// Returns false if another instance already runs in this session. The mutex is
/// held for the whole process lifetime and released by the OS on exit.
pub fn claim_single_instance() -> bool {
    unsafe {
        let mutex = wide(&format!("Local\\Ebb.Instance{}", instance_suffix()));
        match CreateMutexW(None, false, PCWSTR(mutex.as_ptr())) {
            Ok(handle) if GetLastError() == ERROR_ALREADY_EXISTS => {
                let _ = CloseHandle(handle);
                false
            }
            // Leaked on purpose: owning the handle is what marks the instance.
            Ok(_) => true,
            // Can't tell; running a second copy beats not starting at all.
            Err(_) => true,
        }
    }
}

pub enum Request {
    ShowLayer,
    Quit,
}

/// Sends a request to the running instance. It may still be starting up and not
/// have its shell window yet, so retry for a couple of seconds.
pub fn send_to_existing(request: Request) -> bool {
    let msg = match request {
        Request::ShowLayer => WM_SHOW_LAYER,
        Request::Quit => WM_QUIT_APP,
    };
    for _ in 0..40 {
        if let Ok(hwnd) = unsafe { FindWindowW(PCWSTR(class_name().as_ptr()), PCWSTR::null()) } {
            unsafe {
                // A process the user just started may take the foreground; pass that
                // right on, or the running instance can't bring its windows up.
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
            }
            return unsafe { PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0)) }.is_ok();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

// ---------------------------------------------------------------------------
// Shell thread
// ---------------------------------------------------------------------------

unsafe fn try_register(hwnd: HWND, id: i32, combo: &Combo) -> bool {
    unsafe { RegisterHotKey(Some(hwnd), id, HOT_KEY_MODIFIERS(combo.modifiers()) | MOD_NOREPEAT, combo.vk) }.is_ok()
}

/// Binds `action` as `wanted`: a default is the first free candidate, a custom
/// combination is all or nothing. Runs on the shell thread, which owns the
/// window the hotkeys are registered to.
unsafe fn register_action(hwnd: HWND, action: Action, wanted: Wanted) -> Binding {
    match wanted {
        Wanted::Off => Binding::Off,
        Wanted::Default if action.defaults().is_empty() => Binding::Off,
        Wanted::Default => action
            .defaults()
            .iter()
            .enumerate()
            .find(|(i, c)| unsafe { try_register(hwnd, hotkey::hotkey_id(action, *i), c) })
            .map_or(Binding::Taken(None), |(_, c)| Binding::Bound(*c)),
        Wanted::Custom(c) if unsafe { try_register(hwnd, hotkey::hotkey_id(action, 0), &c) } => Binding::Bound(c),
        Wanted::Custom(c) => Binding::Taken(Some(c)),
    }
}

/// Drops whatever `action` has registered (any candidate, or the custom one).
unsafe fn unregister_action(hwnd: HWND, action: Action) {
    for i in 0..action.defaults().len().max(1) {
        let _ = unsafe { UnregisterHotKey(Some(hwnd), hotkey::hotkey_id(action, i)) };
    }
}

unsafe fn register_all(hwnd: HWND, config: Config) -> Hotkeys {
    let mut keys = Hotkeys::default();
    for a in hotkey::registration_order(&config) {
        keys.set(a, unsafe { register_action(hwnd, a, config[a.index()]) });
    }
    keys
}

/// Runs `f` on the shell thread's state (never across a call that pushes events).
fn with_state<R>(f: impl FnOnce(&mut ThreadState) -> R) -> Option<R> {
    STATE.with_borrow_mut(|s| s.as_mut().map(f))
}

/// Publishes new bindings to the UI, with a notice if they differ.
fn publish_hotkeys(shared: &Shared, before: Hotkeys, after: Hotkeys) {
    if after != before {
        *shared.hotkeys.lock().unwrap() = Some(after);
        push(Event::HotkeysChanged);
    }
}

/// Retries the hotkeys that were taken and tells the UI when something changed.
unsafe fn retry_missing(hwnd: HWND) {
    let Some((shared, config, suspended)) = with_state(|s| (s.shared.clone(), s.config, s.suspended)) else { return };
    let Some(before) = shared.hotkeys() else { return };
    if suspended || !Action::ALL.iter().any(|a| matches!(before.get(*a), Binding::Taken(_))) {
        return;
    }
    let mut after = before;
    for a in hotkey::registration_order(&config) {
        if matches!(before.get(a), Binding::Taken(_)) {
            after.set(a, unsafe { register_action(hwnd, a, config[a.index()]) });
        }
    }
    publish_hotkeys(&shared, before, after);
}

/// Drains the queue of [`Command`]s in the order they were sent.
unsafe fn run_commands(hwnd: HWND) {
    let Some(shared) = with_state(|s| s.shared.clone()) else { return };
    let commands = std::mem::take(&mut *shared.commands.lock().unwrap());
    for command in commands {
        match command {
            Command::Set(action, wanted) => unsafe { set_one(hwnd, action, wanted) },
            Command::Suspend(on) => unsafe { suspend(hwnd, on) },
        }
    }
}

unsafe fn suspend(hwnd: HWND, on: bool) {
    let Some((shared, config, was)) = with_state(|s| (s.shared.clone(), s.config, std::mem::replace(&mut s.suspended, on))) else { return };
    if on == was {
        return;
    }
    if on {
        for a in Action::ALL {
            unsafe { unregister_action(hwnd, a) };
        }
    } else if let Some(before) = shared.hotkeys() {
        publish_hotkeys(&shared, before, unsafe { register_all(hwnd, config) });
    }
}

/// Moves one action to `wanted`; if a custom combination can't be had, the
/// action goes back to what it had.
unsafe fn set_one(hwnd: HWND, action: Action, wanted: Wanted) {
    // Recording ended without its resume reaching us (or never began): bind
    // everything again first, so the old state is the one that gets restored.
    unsafe { suspend(hwnd, false) };
    let Some((shared, old)) = with_state(|s| (s.shared.clone(), s.config[action.index()])) else { return };
    let Some(mut keys) = shared.hotkeys() else { return };
    unsafe { unregister_action(hwnd, action) };
    let mut binding = unsafe { register_action(hwnd, action, wanted) };
    let ok = !(matches!(wanted, Wanted::Custom(_)) && matches!(binding, Binding::Taken(_)));
    if ok {
        with_state(|s| s.config[action.index()] = wanted);
    } else {
        binding = unsafe { register_action(hwnd, action, old) };
    }
    keys.set(action, binding);
    *shared.hotkeys.lock().unwrap() = Some(keys);
    push(Event::HotkeySet { action, wanted, ok });
}

struct ThreadState {
    ctx: egui::Context,
    shared: Arc<Shared>,
    taskbar_created: u32,
    /// What each action's hotkey should be; the registrations follow it.
    config: Config,
    /// All hotkeys are unregistered while a combination is recorded.
    suspended: bool,
}

thread_local! {
    static STATE: RefCell<Option<ThreadState>> = const { RefCell::new(None) };
}

/// Queues [`Event::DisplayChanged`] unless one is already waiting: plugging a
/// monitor sends a burst of broadcasts, and the UI re-reads the whole layout.
fn push_display_changed() {
    STATE.with_borrow(|s| {
        if let Some(s) = s {
            let mut events = s.shared.events.lock().unwrap();
            if !events.iter().any(|e| matches!(e, Event::DisplayChanged)) {
                events.push(Event::DisplayChanged);
            }
            drop(events);
            s.ctx.request_repaint();
        }
    });
}

fn push(event: Event) {
    STATE.with_borrow(|s| {
        if let Some(s) = s {
            s.shared.events.lock().unwrap().push(event);
            s.ctx.request_repaint();
        }
    });
}

/// Starts the shell thread without waiting for it: window class, tray icon and
/// hotkey registration cost ~20 ms that would otherwise delay the first frame.
pub fn spawn(ctx: egui::Context, shared: Arc<Shared>, config: Config) {
    std::thread::Builder::new()
        .name("shell".into())
        .spawn(move || unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(class_name().as_ptr()),
                ..Default::default()
            };
            RegisterClassExW(&class);
            // A hidden top-level window rather than HWND_MESSAGE: message-only
            // windows don't receive the TaskbarCreated broadcast.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                PCWSTR(class_name().as_ptr()),
                w!("Ebb"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            );
            let Ok(hwnd) = hwnd else {
                *shared.hotkeys.lock().unwrap() = Some(Hotkeys::unbound(config));
                return;
            };
            shared.hwnd.store(hwnd.0 as isize, Ordering::Relaxed);

            *shared.hotkeys.lock().unwrap() = Some(register_all(hwnd, config));
            ctx.request_repaint();

            STATE.set(Some(ThreadState {
                ctx,
                shared,
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                config,
                suspended: false,
            }));
            add_tray_icon(hwnd);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .expect("spawn shell thread");
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY => match hotkey::action_of_id(wparam.0 as i32) {
            Some(Action::Capture) => push(Event::Capture(Instant::now())),
            Some(Action::Search) => push(Event::Search(Instant::now())),
            Some(Action::Library) => push(Event::OpenLibrary(false)),
            Some(Action::Layer) => push(Event::ToggleLayer),
            Some(Action::Palette) => push(Event::Palette(Instant::now())),
            None => {}
        },
        WM_SHOW_LAYER => push(Event::Launched),
        // Broadcast to top-level windows: "ImmersiveColorSet" when the app mode or
        // the accent changes.
        WM_SETTINGCHANGE if lparam.0 != 0 && unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.is_ok_and(|s| s == "ImmersiveColorSet") => {
            push(Event::SystemColors)
        }
        WM_SETTINGCHANGE if wparam.0 == SPI_SETWORKAREA => push_display_changed(),
        WM_DISPLAYCHANGE => push_display_changed(),
        WM_DWMCOLORIZATIONCOLORCHANGED => push(Event::SystemColors),
        WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC => push(Event::ClockChanged),
        WM_TIMECHANGE => push(Event::ClockChanged),
        WM_QUIT_APP => push(Event::Exit),
        WM_RETRY_HOTKEYS => unsafe { retry_missing(hwnd) },
        WM_HOTKEY_COMMANDS => unsafe { run_commands(hwnd) },
        WM_TRAY => match (lparam.0 & 0xFFFF) as u32 {
            NIN_SELECT | NIN_KEYSELECT => push(Event::TrayClick),
            WM_CONTEXTMENU => {
                // Version 4: the anchor point comes in wparam (screen coordinates).
                let pt = POINT { x: (wparam.0 & 0xFFFF) as i16 as i32, y: ((wparam.0 >> 16) & 0xFFFF) as i16 as i32 };
                unsafe { tray_menu(hwnd, pt) };
            }
            _ => {}
        },
        m if STATE.with_borrow(|s| s.as_ref().is_some_and(|s| s.taskbar_created == m)) => unsafe {
            add_tray_icon(hwnd)
        },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

unsafe fn tray_menu(hwnd: HWND, pt: POINT) {
    const LAYER: usize = 1;
    const CAPTURE: usize = 2;
    const BOTTOM: usize = 3;
    const AUTOSTART: usize = 4;
    const IMPORT: usize = 5;
    const SEARCH: usize = 6;
    const LIBRARY: usize = 7;
    const SETTINGS: usize = 8;
    const EXIT: usize = 9;

    // Opening the menu is a natural moment to pick up a hotkey freed since startup.
    unsafe { retry_missing(hwnd) };
    let (visible, bottom, keys) = STATE.with_borrow(|s| {
        let s = s.as_ref().unwrap();
        (s.shared.layer_visible.load(Ordering::Relaxed), s.shared.pin_bottom.load(Ordering::Relaxed), s.shared.hotkeys().unwrap_or_default())
    });
    let with_hotkey = |label: &str, key: Option<Combo>| match key {
        Some(k) => format!("{label}\t{k}"),
        None => label.to_owned(),
    };
    // Tens of milliseconds of COM before the menu shows; acceptable on a right click.
    let autostart_on = matches!(autostart::status(), Ok(autostart::Status::On { .. }));
    let check = |on: bool| if on { MF_STRING | MF_CHECKED } else { MF_STRING };

    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let items: [(_, usize, Option<String>); 11] = [
            (MF_STRING, LAYER, Some(with_hotkey(if visible { "Скрыть слой" } else { "Показать слой" }, keys.bound(Action::Layer)))),
            (MF_STRING, CAPTURE, Some(with_hotkey("Записать мысль", keys.bound(Action::Capture)))),
            (MF_STRING, SEARCH, Some(with_hotkey("Найти", keys.bound(Action::Search)))),
            (MF_STRING, LIBRARY, Some(with_hotkey("Архив и корзина…", keys.bound(Action::Library)))),
            (MF_SEPARATOR, 0, None),
            (check(bottom), BOTTOM, Some("Слой под окнами".into())),
            (check(autostart_on), AUTOSTART, Some("Запускать при входе в Windows".into())),
            (MF_STRING, IMPORT, Some("Импорт из Sticky Notes…".into())),
            (MF_STRING, SETTINGS, Some("Настройки…".into())),
            (MF_SEPARATOR, 0, None),
            (MF_STRING, EXIT, Some("Выход".into())),
        ];
        for (flags, id, text) in items {
            let text = text.map(|t| wide(&t));
            let ptr = text.as_ref().map_or(PCWSTR::null(), |t| PCWSTR(t.as_ptr()));
            let _ = AppendMenuW(menu, flags, id, ptr);
        }
        // Without foreground the menu doesn't close when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0,
            pt.x,
            pt.y,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        match cmd.0 as usize {
            LAYER => push(Event::ToggleLayer),
            CAPTURE => push(Event::Capture(Instant::now())),
            SEARCH => push(Event::Search(Instant::now())),
            BOTTOM => push(Event::TogglePinBottom),
            AUTOSTART => {
                let result = if autostart_on { autostart::disable() } else { autostart::enable() };
                if let Err(e) = result {
                    eprintln!("autostart: {e}");
                }
            }
            IMPORT => push(Event::ImportSticky),
            LIBRARY => push(Event::OpenLibrary(false)),
            SETTINGS => push(Event::OpenLibrary(true)),
            EXIT => push(Event::Exit),
            _ => {}
        }
    }
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

unsafe fn add_tray_icon(hwnd: HWND) {
    let mut data = tray_data(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = tray_icon();
    for (dst, src) in data.szTip.iter_mut().zip("Ebb".encode_utf16()) {
        *dst = src;
    }
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
    }
}

/// Removes the tray icon immediately (otherwise it lingers until hovered).
pub fn remove_tray_icon(shared: &Shared) {
    let raw = shared.hwnd.load(Ordering::Relaxed);
    if raw != 0 {
        let data = tray_data(HWND(raw as *mut c_void));
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
}

// ---------------------------------------------------------------------------
// Icon
// ---------------------------------------------------------------------------

/// The exe's icon resource (id 1, see build.rs) at the tray's small-icon size,
/// so Windows picks the hand-tuned 16/20/24 px frame instead of scaling one.
fn tray_icon() -> HICON {
    unsafe {
        let size = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()).max(16);
        let instance = GetModuleHandleW(None).unwrap_or_default();
        LoadImageW(Some(instance.into()), PCWSTR(std::ptr::without_provenance(1)), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
            .map_or_else(|_| HICON::default(), |h| HICON(h.0))
    }
}
