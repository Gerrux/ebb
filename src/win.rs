//! Thin Win32 layer: backdrop effects, monitor placement, hotkeys, metrics.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetCursorPos, GetWindowLongPtrW, HWND_BOTTOM, MONITORINFOF_PRIMARY,
    SET_WINDOW_POS_FLAGS, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SetWindowLongPtrW, SetWindowPos, WINDOWPOS, WM_ACTIVATEAPP, WM_STYLECHANGING, WM_WINDOWPOSCHANGING, WS_EX_APPWINDOW,
    WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_SYSMENU,
};
use windows::core::{BOOL, PCWSTR, w};

pub fn hwnd_of(handle: &impl HasWindowHandle) -> Option<isize> {
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

/// A top-level window of this process with the given title. Titles aren't unique
/// across processes (a second instance, EBB_INSTANCE), so FindWindow alone
/// could return another process's window.
pub fn find_own_window(title: PCWSTR) -> Option<isize> {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId};
    let me = unsafe { GetCurrentProcessId() };
    let mut after = None;
    loop {
        let hwnd = unsafe { FindWindowExW(None, after, PCWSTR::null(), title) }.ok()?;
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == me {
            return Some(hwnd.0 as isize);
        }
        after = Some(hwnd);
    }
}

pub fn find_capture_window() -> Option<isize> {
    find_own_window(w!("Ebb Capture"))
}

pub fn find_library_window() -> Option<isize> {
    find_own_window(w!("Ebb Library"))
}

fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut c_void)
}

// ---------------------------------------------------------------------------
// Backdrop
// ---------------------------------------------------------------------------

/// Two ways to get frosted glass behind a borderless transparent window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    /// Documented `DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_TRANSIENTWINDOW` (Win11 22H2+).
    DwmAcrylic,
    /// Undocumented `SetWindowCompositionAttribute(ACCENT_ENABLE_ACRYLICBLURBEHIND)`.
    /// Stays blurred when the window is inactive.
    AccentAcrylic,
    Off,
}

impl Backdrop {
    pub const ALL: [Backdrop; 3] = [Backdrop::AccentAcrylic, Backdrop::DwmAcrylic, Backdrop::Off];

    /// Stable name for settings.
    pub fn key(self) -> &'static str {
        match self {
            Self::DwmAcrylic => "dwm",
            Self::AccentAcrylic => "accent",
            Self::Off => "off",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.key() == key)
    }

    /// For the settings UI.
    pub fn title(self) -> &'static str {
        match self {
            Self::AccentAcrylic => "Размытие фона",
            Self::DwmAcrylic => "Размытие Windows (серое, когда слой неактивен)",
            Self::Off => "Без размытия",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::DwmAcrylic => Self::AccentAcrylic,
            Self::AccentAcrylic => Self::Off,
            Self::Off => Self::DwmAcrylic,
        }
    }
}

#[repr(C)]
struct AccentPolicy {
    accent_state: u32,
    accent_flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct WindowCompositionAttribData {
    attrib: u32,
    pv_data: *mut c_void,
    cb_data: usize,
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetWindowCompositionAttribute(hwnd: HWND, data: *mut WindowCompositionAttribData) -> BOOL;
}

const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

unsafe fn set_accent(hwnd: HWND, state: u32, abgr: u32) {
    let mut policy = AccentPolicy {
        accent_state: state,
        accent_flags: 0,
        gradient_color: abgr,
        animation_id: 0,
    };
    let mut data = WindowCompositionAttribData {
        attrib: WCA_ACCENT_POLICY,
        pv_data: &mut policy as *mut _ as *mut c_void,
        cb_data: size_of::<AccentPolicy>(),
    };
    unsafe {
        let _ = SetWindowCompositionAttribute(hwnd, &mut data);
    }
}

unsafe fn set_dwm_i32(hwnd: HWND, attr: windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE, v: i32) {
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, attr, &v as *const i32 as *const c_void, 4);
    }
}

/// `rounded`: Windows 11 rounded corners, for floating windows; the full-screen
/// layer stays square.
pub fn apply_backdrop(raw: isize, mode: Backdrop, rounded: bool) {
    let h = hwnd(raw);
    unsafe {
        // Without WS_SYSMENU DWM stops drawing the (disabled) caption buttons
        // into the extended frame.
        let style = GetWindowLongPtrW(h, GWL_STYLE);
        if style & WS_SYSMENU.0 as isize != 0 {
            SetWindowLongPtrW(h, GWL_STYLE, style & !(WS_SYSMENU.0 as isize));
            let _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }

        set_dwm_i32(h, DWMWA_USE_IMMERSIVE_DARK_MODE, i32::from(!LIGHT_THEME.load(Ordering::Relaxed)));
        let corners = if rounded { DWMWCP_ROUND } else { DWMWCP_DONOTROUND };
        set_dwm_i32(h, DWMWA_WINDOW_CORNER_PREFERENCE, corners.0);
        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        let _ = DwmExtendFrameIntoClientArea(h, &margins);

        match mode {
            Backdrop::DwmAcrylic => {
                set_accent(h, ACCENT_DISABLED, 0);
                set_dwm_i32(h, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW.0);
            }
            Backdrop::AccentAcrylic => {
                set_dwm_i32(h, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE.0);
                // ABGR tint; low alpha, the egui layer adds its own tint on top.
                let tint = if LIGHT_THEME.load(Ordering::Relaxed) { 0x10_F4_F2_F0 } else { 0x10_18_14_10 };
                set_accent(h, ACCENT_ENABLE_ACRYLICBLURBEHIND, tint);
            }
            Backdrop::Off => {
                set_dwm_i32(h, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE.0);
                set_accent(h, ACCENT_DISABLED, 0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// System colors
// ---------------------------------------------------------------------------

/// The app's theme is light: backdrops set up after this use a light tint.
pub static LIGHT_THEME: AtomicBool = AtomicBool::new(false);

/// Windows' app theme and accent palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemColors {
    /// "Choose your default app mode": Light.
    pub light: bool,
    /// RGB: Light3, Light2, Light1, Accent, Dark1, Dark2, Dark3.
    pub palette: [[u8; 3]; 7],
    /// RGB of Start and the taskbar when "Show accent color on Start and taskbar"
    /// is on; `None` when they're the neutral system grey.
    pub start: Option<[u8; 3]>,
}

impl Default for SystemColors {
    /// Windows' default blue, dark mode.
    fn default() -> Self {
        Self {
            light: false,
            palette: [[153, 235, 255], [76, 194, 255], [0, 145, 248], [0, 120, 212], [0, 103, 192], [0, 62, 146], [0, 26, 104]],
            start: None,
        }
    }
}

fn reg_value(key: PCWSTR, name: PCWSTR, buf: &mut [u8]) -> Option<usize> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RegGetValueW};
    let mut len = buf.len() as u32;
    let flags = RRF_RT_REG_BINARY | RRF_RT_REG_DWORD;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, key, name, flags, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
    r.is_ok().then_some(len as usize)
}

/// Read from the registry: a few microseconds, no WinRT.
pub fn system_colors() -> SystemColors {
    let mut colors = SystemColors::default();
    let mut dword = [0u8; 4];
    if reg_value(w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), w!("AppsUseLightTheme"), &mut dword) == Some(4) {
        colors.light = u32::from_le_bytes(dword) != 0;
    }
    // 8 RGBA entries; the 8th isn't part of the ramp.
    let mut palette = [0u8; 32];
    if reg_value(w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent"), w!("AccentPalette"), &mut palette) == Some(32) {
        for (i, c) in colors.palette.iter_mut().enumerate() {
            *c = [palette[i * 4], palette[i * 4 + 1], palette[i * 4 + 2]];
        }
    }
    let mut prevalence = [0u8; 4];
    let accent_start = reg_value(w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), w!("ColorPrevalence"), &mut prevalence)
        == Some(4)
        && u32::from_le_bytes(prevalence) != 0;
    let mut start = [0u8; 4];
    if accent_start && reg_value(w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent"), w!("StartColorMenu"), &mut start) == Some(4) {
        // DWORD 0xAABBGGRR, little-endian: R, G, B, A.
        colors.start = Some([start[0], start[1], start[2]]);
    }
    colors
}

// ---------------------------------------------------------------------------
// Monitors
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Monitor {
    pub handle: isize,
    pub rect: RECT,
    pub work: RECT,
    pub primary: bool,
    /// Device interface paths of the display(s) behind this monitor, e.g.
    /// `\\?\DISPLAY#XMI27B1#5&c579a42&0&UID4353#{e6f07b5f-...}` — the form other
    /// apps (Sticky Notes) store window positions against.
    pub device_ids: Vec<String>,
}

impl Monitor {
    /// Whether `id` names this monitor; compared without the interface-class GUID.
    pub fn matches_device(&self, id: &str) -> bool {
        let key = |s: &str| s.split("#{").next().unwrap_or(s).to_ascii_lowercase();
        let id = key(id);
        self.device_ids.iter().any(|d| key(d) == id)
    }
}

fn wide_str(buf: &[u16]) -> String {
    String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())])
}

pub fn monitors() -> Vec<Monitor> {
    use windows::Win32::Graphics::Gdi::{DISPLAY_DEVICEW, EnumDisplayDevicesW, MONITORINFOEXW};

    unsafe extern "system" fn cb(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let out = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(m, &mut info as *mut _ as *mut MONITORINFO) }.as_bool() {
            let mut device_ids = Vec::new();
            for index in 0.. {
                let mut dd = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
                // EDD_GET_DEVICE_INTERFACE_NAME = 1
                if !unsafe { EnumDisplayDevicesW(PCWSTR(info.szDevice.as_ptr()), index, &mut dd, 1) }.as_bool() {
                    break;
                }
                device_ids.push(wide_str(&dd.DeviceID));
            }
            out.push(Monitor {
                handle: m.0 as isize,
                rect: info.monitorInfo.rcMonitor,
                work: info.monitorInfo.rcWork,
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                device_ids,
            });
        }
        true.into()
    }
    let mut out: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    // Secondary monitors first: the ambient layer prefers them.
    out.sort_by_key(|m| m.primary);
    out
}

/// The monitor a window is (mostly) on.
pub fn monitor_of(raw: isize) -> Option<Monitor> {
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
    let handle = unsafe { MonitorFromWindow(hwnd(raw), MONITOR_DEFAULTTONEAREST) }.0 as isize;
    monitors().into_iter().find(|m| m.handle == handle)
}

/// Cover the work area of a monitor (physical pixels, bypasses DPI conversions).
pub fn place_on(raw: isize, m: &Monitor) {
    let r = m.work;
    let _placing = Placing::begin();
    unsafe {
        let _ = SetWindowPos(
            hwnd(raw),
            None,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Set while Ebb itself moves or sizes the layer (see [`place_on`]); any other
/// move or resize of the layer — dragging its edge, Aero Snap, Win+arrows — is
/// dropped in the layer's WM_WINDOWPOSCHANGING.
static PLACING: AtomicBool = AtomicBool::new(false);

struct Placing;

impl Placing {
    fn begin() -> Self {
        PLACING.store(true, Ordering::Relaxed);
        Placing
    }
}

impl Drop for Placing {
    fn drop(&mut self) {
        PLACING.store(false, Ordering::Relaxed);
    }
}

/// Effective DPI scale of a monitor (1.0 at 96 DPI).
pub fn monitor_scale(m: &Monitor) -> f32 {
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    let (mut x, mut y) = (0, 0);
    match unsafe { GetDpiForMonitor(HMONITOR(m.handle as *mut c_void), MDT_EFFECTIVE_DPI, &mut x, &mut y) } {
        Ok(()) if x > 0 => x as f32 / 96.0,
        _ => 1.0,
    }
}

/// A window of `size` points, centered at the top or bottom edge of a monitor's
/// work area, `gap` points away from it (the collapsed layer).
pub fn place_edge_center(raw: isize, m: &Monitor, size: (f32, f32), gap: f32, top: bool) {
    let scale = monitor_scale(m);
    let (w, h) = ((size.0 * scale).round() as i32, (size.1 * scale).round() as i32);
    let r = m.work;
    let gap = (gap * scale).round() as i32;
    let x = r.left + ((r.right - r.left) - w) / 2;
    let y = if top { r.top + gap } else { r.bottom - h - gap };
    let _placing = Placing::begin();
    unsafe {
        let _ = SetWindowPos(hwnd(raw), None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

pub fn dpi_scale(raw: isize) -> f32 {
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd(raw)) };
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

/// Center a window horizontally in the upper third of the monitor under the cursor.
/// Center a window of the given size on the work area of the monitor under the cursor.
pub fn center_near_cursor(raw: isize, width_px: i32, height_px: i32) {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            return;
        }
        let r = info.rcWork;
        let x = r.left + ((r.right - r.left) - width_px) / 2;
        let y = r.top + ((r.bottom - r.top) - height_px) / 2;
        let _ = SetWindowPos(hwnd(raw), None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

pub fn move_near_cursor(raw: isize, width_px: i32) {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            return;
        }
        let r = info.rcWork;
        let x = r.left + ((r.right - r.left) - width_px) / 2;
        let y = r.top + (r.bottom - r.top) / 4;
        let _ = SetWindowPos(
            hwnd(raw),
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

// ---------------------------------------------------------------------------
// Window behaviour
// ---------------------------------------------------------------------------

/// Whether the layer is kept at the bottom of the z-order (under other windows).
pub static PIN_BOTTOM: AtomicBool = AtomicBool::new(true);
/// The layer was summoned over other windows (tray click, launch from a shortcut);
/// [`PIN_BOTTOM`] is suspended until it's dismissed or another app is activated.
pub static RAISED: AtomicBool = AtomicBool::new(false);
/// `GetTickCount64` when the layer last went back to the bottom because another
/// app was activated. Clicking the tray icon activates the taskbar first, so the
/// click that follows must not raise the layer again.
static LOWERED_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn keep_bottom() -> bool {
    PIN_BOTTOM.load(Ordering::Relaxed) && !RAISED.load(Ordering::Relaxed)
}

/// Milliseconds since the layer was lowered by activating another app.
pub fn ms_since_auto_lowered() -> u64 {
    let at = LOWERED_AT.load(Ordering::Relaxed);
    if at == 0 { u64::MAX } else { unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }.saturating_sub(at) }
}

/// Shows the window if hidden, puts it above other windows and activates it.
/// Shown directly rather than waiting for eframe's command, which lands ~100 ms
/// later for a hidden root window (winit reads visibility back from the window).
pub fn raise(raw: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{HWND_TOP, IsWindowVisible, SW_SHOWNA, SetForegroundWindow, ShowWindow};
    RAISED.store(true, Ordering::Relaxed);
    let h = hwnd(raw);
    unsafe {
        if !IsWindowVisible(h).as_bool() {
            let _ = ShowWindow(h, SW_SHOWNA);
        }
        let _ = SetWindowPos(h, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        let _ = SetForegroundWindow(h);
    }
}

/// Sends the window to the bottom of the z-order and, if it had the focus, hands
/// it to the topmost window of another app, so typing goes where the user looks.
pub fn lower(raw: isize) {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        GW_HWNDNEXT, GetForegroundWindow, GetTopWindow, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        SetForegroundWindow,
    };
    RAISED.store(false, Ordering::Relaxed);
    let h = hwnd(raw);
    unsafe {
        let _ = SetWindowPos(h, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        let fg = GetForegroundWindow();
        let mut pid = 0;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid != GetCurrentProcessId() {
            return;
        }
        let me = pid;
        let mut next = GetTopWindow(None).ok();
        while let Some(w) = next {
            let mut owner = 0;
            GetWindowThreadProcessId(w, Some(&mut owner));
            let ex = GetWindowLongPtrW(w, GWL_EXSTYLE);
            if owner != me
                && IsWindowVisible(w).as_bool()
                && !IsIconic(w).as_bool()
                && ex & WS_EX_TOOLWINDOW.0 as isize == 0
                && ex & windows::Win32::UI::WindowsAndMessaging::WS_EX_NOACTIVATE.0 as isize == 0
            {
                let _ = SetForegroundWindow(w);
                return;
            }
            next = GetWindow(w, GW_HWNDNEXT).ok();
        }
    }
}

/// Subclass flags (`dwRefData`).
pub const LAYER: usize = 1;
/// Keeps WS_EX_LAYERED so the whole window (backdrop included) can fade; see [`fade`].
pub const FADE: usize = 2;

/// Whole-window opacity through the layered-window alpha, which DWM applies to the
/// composed window including its acrylic backdrop (egui can only fade its own
/// drawing). Needs the FADE window rule.
pub fn set_window_alpha(raw: isize, alpha: u8) {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::UI::WindowsAndMessaging::{LWA_ALPHA, SetLayeredWindowAttributes};
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd(raw), COLORREF(0), alpha, LWA_ALPHA);
    }
}

/// Latest fade per window; an older fade thread stops when it sees a newer one.
static FADES: std::sync::Mutex<Vec<(isize, u64)>> = std::sync::Mutex::new(Vec::new());
static FADE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn current_fade(raw: isize) -> Option<u64> {
    FADES.lock().unwrap().iter().find(|(h, _)| *h == raw).map(|(_, g)| *g)
}

/// Stops any fade on the window and sets its alpha right away (no thread).
pub fn set_alpha_now(raw: isize, alpha: u8) {
    FADES.lock().unwrap().retain(|(h, _)| *h != raw);
    set_window_alpha(raw, alpha);
}

/// Animates the window alpha from `from` to `to` over `duration` (ease-out) on a
/// short-lived thread, stepping every ~8 ms; a newer fade on the same window
/// cancels this one. `done` runs at the end unless cancelled.
///
/// Steps use a high-resolution waitable timer, not DwmFlush: DwmFlush blocks
/// until the next present, and running it while a window presents its first
/// frame delayed that window's appearance by ~170 ms.
pub fn fade(raw: isize, from: u8, to: u8, duration: std::time::Duration, done: impl FnOnce() + Send + 'static) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE, SetWaitableTimer, TIMER_ALL_ACCESS,
        WaitForSingleObject,
    };
    let generation = FADE_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    {
        let mut fades = FADES.lock().unwrap();
        fades.retain(|(h, _)| *h != raw);
        fades.push((raw, generation));
    }
    set_window_alpha(raw, from);
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let timer = unsafe {
            CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)
        }
        .ok();
        let step = || match timer {
            Some(t) => unsafe {
                // Relative due time in 100 ns units: 8 ms.
                let due = -80_000i64;
                if SetWaitableTimer(t, &due, 0, None, None, false).is_ok() {
                    WaitForSingleObject(t, INFINITE);
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(8));
                }
            },
            None => std::thread::sleep(std::time::Duration::from_millis(8)),
        };
        let completed = loop {
            step();
            if current_fade(raw) != Some(generation) {
                break false;
            }
            let t = (start.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            let alpha = from as f32 + (to as f32 - from as f32) * eased;
            set_window_alpha(raw, alpha.round() as u8);
            if t >= 1.0 {
                break true;
            }
        };
        if let Some(t) = timer {
            unsafe {
                let _ = CloseHandle(t);
            }
        }
        if completed {
            done();
        }
    });
}

/// winit rewrites GWL_STYLE/GWL_EXSTYLE from its own flags whenever any of them
/// change (e.g. on show/hide), which would undo style tweaks made once. Enforcing
/// them in WM_STYLECHANGING keeps them regardless of who sets the style:
/// - every window: no WS_SYSMENU (otherwise DWM draws a caption "×" in the frame);
/// - the layer: WS_EX_TOOLWINDOW without WS_EX_APPWINDOW (no taskbar button, not in
///   Alt+Tab), and z-order pinned to HWND_BOTTOM in WM_WINDOWPOSCHANGING, so
///   activating the layer by clicking a card doesn't raise it over other windows;
///   there too, its position and size only change through [`place_on`] and
///   [`place_edge_center`].
unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    flags: usize,
) -> LRESULT {
    unsafe {
        match msg {
            WM_STYLECHANGING => {
                let s = &mut *(lparam.0 as *mut STYLESTRUCT);
                if wparam.0 as i32 == GWL_STYLE.0 {
                    s.styleNew &= !WS_SYSMENU.0;
                } else if wparam.0 as i32 == GWL_EXSTYLE.0 {
                    if flags & LAYER != 0 {
                        s.styleNew = (s.styleNew | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0;
                    }
                    if flags & FADE != 0 {
                        s.styleNew |= WS_EX_LAYERED.0;
                    }
                }
            }
            WM_WINDOWPOSCHANGING if flags & LAYER != 0 => {
                let pos = &mut *(lparam.0 as *mut WINDOWPOS);
                if keep_bottom() && !pos.flags.contains(SWP_NOZORDER) {
                    pos.hwndInsertAfter = HWND_BOTTOM;
                }
                // The layer stays where Ebb puts it: no dragging by the frame's edge,
                // no Aero Snap, no Win+arrows.
                if !PLACING.load(Ordering::Relaxed) {
                    pos.flags |= SWP_NOMOVE | SWP_NOSIZE;
                }
            }
            // Another app got activated: a summoned layer goes back under the windows.
            WM_ACTIVATEAPP if flags & LAYER != 0 && wparam.0 == 0 && RAISED.swap(false, Ordering::Relaxed) => {
                LOWERED_AT.store(windows::Win32::System::SystemInformation::GetTickCount64(), Ordering::Relaxed);
                if PIN_BOTTOM.load(Ordering::Relaxed) {
                    let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// Installs [`subclass_proc`] (must run on the window's thread) and re-applies the
/// styles through it. `flags` is 0 or [`LAYER`].
pub fn install_window_rules(raw: isize, flags: usize) {
    let h = hwnd(raw);
    unsafe {
        let _ = SetWindowSubclass(h, Some(subclass_proc), 1, flags);
        for index in [GWL_STYLE, GWL_EXSTYLE] {
            SetWindowLongPtrW(h, index, GetWindowLongPtrW(h, index));
        }
        let _ = SetWindowPos(
            h,
            if flags & LAYER != 0 && keep_bottom() { Some(HWND_BOTTOM) } else { None },
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE
                | if flags & LAYER != 0 { SET_WINDOW_POS_FLAGS(0) } else { SWP_NOZORDER },
        );
    }
}

/// Re-applies the z-order after toggling [`PIN_BOTTOM`].
pub fn set_pin_bottom(raw: isize, on: bool) {
    PIN_BOTTOM.store(on, Ordering::Relaxed);
    if keep_bottom() {
        unsafe {
            let _ = SetWindowPos(hwnd(raw), Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// Opens an http(s) address in the default browser.
pub fn open_url(url: &str) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    let wide: Vec<u16> = url.encode_utf16().chain([0]).collect();
    unsafe { ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), None, None, SW_SHOWNORMAL) };
}

/// Maps a file read-only for the rest of the process lifetime. The pages are
/// file-backed and shared with every other process that maps the same file
/// (system fonts are mapped by most GUI apps), so they are not private bytes.
pub fn map_file_static(path: &str) -> Option<&'static [u8]> {
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, GetFileSizeEx, OPEN_EXISTING,
    };
    use windows::Win32::System::Memory::{CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY};

    let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
    unsafe {
        let file = CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .ok()?;
        let mut size = 0i64;
        let mapping = GetFileSizeEx(file, &mut size)
            .ok()
            .filter(|_| size > 0)
            .and_then(|_| CreateFileMappingW(file, None, PAGE_READONLY, 0, 0, None).ok());
        let _ = CloseHandle(file);
        let mapping = mapping?;
        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
        // The view keeps the section alive; it is never unmapped.
        let _ = CloseHandle(mapping);
        (!view.Value.is_null()).then(|| std::slice::from_raw_parts(view.Value as *const u8, size as usize))
    }
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Moves all pages out of the working set (they stay on the standby list and
/// fault back in cheaply). Changes working set only, not private bytes.
pub fn trim_working_set() {
    unsafe {
        let _ = windows::Win32::System::ProcessStatus::EmptyWorkingSet(GetCurrentProcess());
    }
}

fn filetime_u64(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

/// Current UTC time in 100 ns FILETIME units.
pub fn now_filetime() -> u64 {
    filetime_u64(unsafe { GetSystemTimePreciseAsFileTime() })
}

fn process_creation(process: windows::Win32::Foundation::HANDLE) -> Option<u64> {
    let (mut creation, mut exit, mut kernel, mut user) = Default::default();
    unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }.ok()?;
    Some(filetime_u64(creation))
}

pub fn ms_between(from: u64, to: u64) -> f64 {
    (to as i64 - from as i64) as f64 / 10_000.0
}

/// Milliseconds since the OS created this process (includes loader time).
pub fn ms_since_process_start() -> f64 {
    let now = now_filetime();
    process_creation(unsafe { GetCurrentProcess() }).map_or(f64::NAN, |c| ms_between(c, now))
}

pub fn process_start_filetime() -> Option<u64> {
    process_creation(unsafe { GetCurrentProcess() })
}

/// When the logon session of this process was created (credentials accepted).
/// Own session only, so no privileges are needed.
pub fn logon_filetime() -> Option<u64> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::Authentication::Identity::{LsaFreeReturnBuffer, LsaGetLogonSessionData};
    use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_STATISTICS, TokenStatistics};
    use windows::Win32::System::Threading::OpenProcessToken;

    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut stats = TOKEN_STATISTICS::default();
        let mut len = 0;
        let got = GetTokenInformation(
            token,
            TokenStatistics,
            Some(&mut stats as *mut _ as *mut c_void),
            size_of::<TOKEN_STATISTICS>() as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        got.ok()?;
        let mut data = std::ptr::null_mut();
        if LsaGetLogonSessionData(&stats.AuthenticationId, &mut data).is_err() || data.is_null() {
            return None;
        }
        let logon = (*data).LogonTime;
        let _ = LsaFreeReturnBuffer(data as *const c_void);
        Some(logon as u64)
    }
}

/// Creation time of the oldest explorer.exe in this session (the shell).
pub fn explorer_start_filetime() -> Option<u64> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        let mut my_session = 0;
        ProcessIdToSessionId(GetCurrentProcessId(), &mut my_session).ok()?;
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut oldest: Option<u64> = None;
        let mut more = Process32FirstW(snap, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            let mut session = 0;
            if name.eq_ignore_ascii_case("explorer.exe")
                && ProcessIdToSessionId(entry.th32ProcessID, &mut session).is_ok()
                && session == my_session
            {
                if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, entry.th32ProcessID) {
                    if let Some(t) = process_creation(h) {
                        oldest = Some(oldest.map_or(t, |o| o.min(t)));
                    }
                    let _ = CloseHandle(h);
                }
            }
            more = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        oldest
    }
}

/// User + kernel CPU time consumed by this process, in milliseconds.
pub fn cpu_ms() -> f64 {
    unsafe {
        let (mut creation, mut exit, mut kernel, mut user) = Default::default();
        if GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user).is_err() {
            return f64::NAN;
        }
        (filetime_u64(kernel) + filetime_u64(user)) as f64 / 10_000.0
    }
}

/// (working set, private bytes) in MiB.
pub fn memory_mib() -> (f64, f64) {
    unsafe {
        let mut c = PROCESS_MEMORY_COUNTERS_EX {
            cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        let ok = GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut c as *mut _ as *mut _,
            c.cb,
        );
        if ok.is_err() {
            return (f64::NAN, f64::NAN);
        }
        let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
        (mib(c.WorkingSetSize), mib(c.PrivateUsage))
    }
}
