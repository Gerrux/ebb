//! Process-level shell integration: single instance, tray icon, global hotkey.
//!
//! One background thread owns a hidden top-level window that receives tray
//! callbacks, `WM_HOTKEY`, `TaskbarCreated` (Explorer restarts) and the "show
//! the layer" request from a second instance. It blocks in `GetMessageW`, so it
//! costs nothing while idle. Results reach the UI as [`Event`]s plus a repaint
//! request, which runs `App::logic` even while the layer is hidden.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_WIN, RegisterHotKey, VK_SPACE};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NIN_SELECT, NINF_KEY,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, GetWindowThreadProcessId, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    FindWindowW, GetMessageW, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW,
    RegisterClassExW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateMessage, WM_APP, WM_CONTEXTMENU, WM_HOTKEY, WM_NULL, WM_SETTINGCHANGE,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::autostart;

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
/// Not in the windows crate's Dwm module.
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
const WM_TRAY: u32 = WM_APP + 1;
const WM_SHOW_LAYER: u32 = WM_APP + 2;
const WM_QUIT_APP: u32 = WM_APP + 3;
const WM_RETRY_HOTKEYS: u32 = WM_APP + 4;
const TRAY_ID: u32 = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

pub enum Event {
    /// Capture hotkey (or tray/menu equivalent) pressed at this instant.
    Capture(Instant),
    Search(Instant),
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
    /// A hotkey taken at startup was registered later; see [`Shared::hotkeys`].
    HotkeysChanged,
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
    hwnd: AtomicIsize,
}

/// Which combination each hotkey got; `None` = every candidate was taken.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Hotkeys {
    pub capture: Option<&'static str>,
    pub search: Option<&'static str>,
    pub library: Option<&'static str>,
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

/// Candidate capture hotkeys, first free one wins. Ctrl+Alt+Space is often taken
/// (PowerToys), Ctrl+Space and Ctrl+Shift+Space collide with IDE completion.
const CAPTURE_HOTKEYS: &[(&str, HOT_KEY_MODIFIERS, u32)] = &[
    ("Win+Alt+N", HOT_KEY_MODIFIERS(MOD_WIN.0 | MOD_ALT.0), b'N' as u32),
    ("Ctrl+Alt+N", HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0), b'N' as u32),
    ("Ctrl+Alt+Space", HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0), VK_SPACE.0 as u32),
];

/// Search: the spec's Ctrl+Space collides with IDE completion, like capture's.
const SEARCH_HOTKEYS: &[(&str, HOT_KEY_MODIFIERS, u32)] = &[
    ("Win+Alt+F", HOT_KEY_MODIFIERS(MOD_WIN.0 | MOD_ALT.0), b'F' as u32),
    ("Ctrl+Alt+F", HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0), b'F' as u32),
];
/// Library (archive, trash, settings).
const LIBRARY_HOTKEYS: &[(&str, HOT_KEY_MODIFIERS, u32)] = &[("Win+Alt+L", HOT_KEY_MODIFIERS(MOD_WIN.0 | MOD_ALT.0), b'L' as u32)];
/// `WM_HOTKEY` ids: capture candidates use 1.., search 101.., library 201...
const SEARCH_ID_BASE: i32 = 101;
const LIBRARY_ID_BASE: i32 = 201;

/// Registers the first free combination; ids are `base + index`.
unsafe fn register_first(hwnd: HWND, base: i32, candidates: &[(&'static str, HOT_KEY_MODIFIERS, u32)]) -> Option<&'static str> {
    candidates
        .iter()
        .enumerate()
        .find(|(i, (_, mods, vk))| unsafe { RegisterHotKey(Some(hwnd), base + *i as i32, *mods | MOD_NOREPEAT, *vk) }.is_ok())
        .map(|(_, (name, ..))| *name)
}

/// Registers whichever of the three hotkeys is still missing. Runs on the shell
/// thread, which owns the window the hotkeys are registered to.
unsafe fn register_missing(hwnd: HWND, mut keys: Hotkeys) -> Hotkeys {
    unsafe {
        keys.capture = keys.capture.or_else(|| register_first(hwnd, 1, CAPTURE_HOTKEYS));
        keys.search = keys.search.or_else(|| register_first(hwnd, SEARCH_ID_BASE, SEARCH_HOTKEYS));
        keys.library = keys.library.or_else(|| register_first(hwnd, LIBRARY_ID_BASE, LIBRARY_HOTKEYS));
    }
    keys
}

/// Retries the missing hotkeys and tells the UI when something changed.
unsafe fn retry_missing(hwnd: HWND) {
    let Some((shared, before)) = STATE.with_borrow(|s| s.as_ref().map(|s| (s.shared.clone(), s.shared.hotkeys()))) else { return };
    let Some(before) = before else { return };
    if before.capture.is_some() && before.search.is_some() && before.library.is_some() {
        return;
    }
    let after = unsafe { register_missing(hwnd, before) };
    if after != before {
        *shared.hotkeys.lock().unwrap() = Some(after);
        push(Event::HotkeysChanged);
    }
}

struct ThreadState {
    ctx: egui::Context,
    shared: Arc<Shared>,
    taskbar_created: u32,
}

thread_local! {
    static STATE: RefCell<Option<ThreadState>> = const { RefCell::new(None) };
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
pub fn spawn(ctx: egui::Context, shared: Arc<Shared>) {
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
                *shared.hotkeys.lock().unwrap() = Some(Hotkeys::default());
                return;
            };
            shared.hwnd.store(hwnd.0 as isize, Ordering::Relaxed);

            *shared.hotkeys.lock().unwrap() = Some(register_missing(hwnd, Hotkeys::default()));
            ctx.request_repaint();

            STATE.set(Some(ThreadState { ctx, shared, taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")) }));
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
        WM_HOTKEY if wparam.0 as i32 >= LIBRARY_ID_BASE => push(Event::OpenLibrary(false)),
        WM_HOTKEY if wparam.0 as i32 >= SEARCH_ID_BASE => push(Event::Search(Instant::now())),
        WM_HOTKEY => push(Event::Capture(Instant::now())),
        WM_SHOW_LAYER => push(Event::Launched),
        // Broadcast to top-level windows: "ImmersiveColorSet" when the app mode or
        // the accent changes.
        WM_SETTINGCHANGE if lparam.0 != 0 && unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.is_ok_and(|s| s == "ImmersiveColorSet") => {
            push(Event::SystemColors)
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => push(Event::SystemColors),
        WM_QUIT_APP => push(Event::Exit),
        WM_RETRY_HOTKEYS => unsafe { retry_missing(hwnd) },
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
    let (hotkey, search_hotkey) = (keys.capture, keys.search);
    let with_hotkey = |label: &str, key: Option<&str>| match key {
        Some(k) => format!("{label}\t{k}"),
        None => label.to_owned(),
    };
    // Tens of milliseconds of COM before the menu shows; acceptable on a right click.
    let autostart_on = matches!(autostart::status(), Ok(autostart::Status::On { .. }));
    let check = |on: bool| if on { MF_STRING | MF_CHECKED } else { MF_STRING };

    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let items: [(_, usize, Option<String>); 11] = [
            (MF_STRING, LAYER, Some(if visible { "Скрыть слой" } else { "Показать слой" }.into())),
            (MF_STRING, CAPTURE, Some(with_hotkey("Записать мысль", hotkey))),
            (MF_STRING, SEARCH, Some(with_hotkey("Найти", search_hotkey))),
            (MF_STRING, LIBRARY, Some(with_hotkey("Архив и корзина…", keys.library))),
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
