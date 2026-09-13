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
    FindWindowW, GWL_EXSTYLE, GWL_STYLE, GetCursorPos, GetWindowLongPtrW, HWND_BOTTOM, MONITORINFOF_PRIMARY,
    SET_WINDOW_POS_FLAGS, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SetWindowLongPtrW, SetWindowPos, WINDOWPOS, WM_STYLECHANGING, WM_WINDOWPOSCHANGING, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW, WS_SYSMENU,
};
use windows::core::{BOOL, PCWSTR, w};

pub fn hwnd_of(handle: &impl HasWindowHandle) -> Option<isize> {
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

pub fn find_window(title: PCWSTR) -> Option<isize> {
    let hwnd = unsafe { FindWindowW(PCWSTR::null(), title) }.ok()?;
    (!hwnd.is_invalid()).then_some(hwnd.0 as isize)
}

pub fn find_capture_window() -> Option<isize> {
    find_window(w!("Ambient Capture"))
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
    pub fn next(self) -> Self {
        match self {
            Self::DwmAcrylic => Self::AccentAcrylic,
            Self::AccentAcrylic => Self::Off,
            Self::Off => Self::DwmAcrylic,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::DwmAcrylic => "DWM acrylic (DWMSBT_TRANSIENTWINDOW)",
            Self::AccentAcrylic => "Accent acrylic (SetWindowCompositionAttribute)",
            Self::Off => "off",
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

        set_dwm_i32(h, DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
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
                set_accent(h, ACCENT_ENABLE_ACRYLICBLURBEHIND, 0x10_18_14_10);
            }
            Backdrop::Off => {
                set_dwm_i32(h, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE.0);
                set_accent(h, ACCENT_DISABLED, 0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Monitors
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub work: RECT,
    pub primary: bool,
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn cb(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let out = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(m, &mut info) }.as_bool() {
            out.push(Monitor {
                work: info.rcWork,
                primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
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

/// Cover the work area of a monitor (physical pixels, bypasses DPI conversions).
pub fn place_on(raw: isize, m: &Monitor) {
    let r = m.work;
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

pub fn dpi_scale(raw: isize) -> f32 {
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd(raw)) };
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

/// Center a window horizontally in the upper third of the monitor under the cursor.
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

/// Subclass flags (`dwRefData`).
pub const LAYER: usize = 1;

/// winit rewrites GWL_STYLE/GWL_EXSTYLE from its own flags whenever any of them
/// change (e.g. on show/hide), which would undo style tweaks made once. Enforcing
/// them in WM_STYLECHANGING keeps them regardless of who sets the style:
/// - every window: no WS_SYSMENU (otherwise DWM draws a caption "×" in the frame);
/// - the layer: WS_EX_TOOLWINDOW without WS_EX_APPWINDOW (no taskbar button, not in
///   Alt+Tab), and z-order pinned to HWND_BOTTOM in WM_WINDOWPOSCHANGING, so
///   activating the layer by clicking a card doesn't raise it over other windows.
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
                } else if wparam.0 as i32 == GWL_EXSTYLE.0 && flags & LAYER != 0 {
                    s.styleNew = (s.styleNew | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0;
                }
            }
            WM_WINDOWPOSCHANGING if flags & LAYER != 0 && PIN_BOTTOM.load(Ordering::Relaxed) => {
                let pos = &mut *(lparam.0 as *mut WINDOWPOS);
                if !pos.flags.contains(SWP_NOZORDER) {
                    pos.hwndInsertAfter = HWND_BOTTOM;
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
            if flags & LAYER != 0 && PIN_BOTTOM.load(Ordering::Relaxed) { Some(HWND_BOTTOM) } else { None },
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
    if on {
        unsafe {
            let _ = SetWindowPos(hwnd(raw), Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

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
