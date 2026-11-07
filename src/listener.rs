use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostQuitMessage,
    RegisterClassExW, SetTimer, TranslateMessage, MSG, WM_DESTROY, WM_DPICHANGED,
    WNDCLASSEXW,
};

use crate::color::{encode_wide, get_current_dark_system_accent, get_current_system_accent, Rgb};
use crate::config::AppConfig;
use crate::e11_svg::{apply_e11_colorization, find_e11_packages};
use crate::system::{detect_windows_version, is_elevated, run_elevated_self, WindowsVersion};
use crate::win32_theme::{modify_styles, modify_sys_colors};

const WM_SETTINGCHANGE: u32 = 0x001A;
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
const WM_THEMECHANGED: u32 = 0x031A;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WM_TIMER: u32 = 0x0113;

const WTS_SESSION_UNLOCK: usize = 0x8;
const NOTIFY_FOR_THIS_SESSION: u32 = 0;
const TIMER_COLOR_UPDATE: usize = 1001;

static RUNNING: AtomicBool = AtomicBool::new(true);

type FnWTSRegisterSessionNotification = unsafe extern "system" fn(HWND, u32) -> i32;
type FnWTSUnRegisterSessionNotification = unsafe extern "system" fn(HWND) -> i32;

struct ListenerState {
    config: AppConfig,
    win_ver: WindowsVersion,
    last_accent: Option<Rgb>,
}

static STATE: Mutex<Option<ListenerState>> = Mutex::new(None);

pub fn run_listener(config: AppConfig) {
    let win_ver = detect_windows_version();

    // Initial apply
    let initial_accent = match &config.custom_accent_hex {
        Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_system_accent),
        None => get_current_system_accent(),
    };

    apply_all_in_memory(&config, win_ver, initial_accent);

    // Initial check for E11 SVG if enabled
    if config.colorize_e11_svg && win_ver >= WindowsVersion::Win11 {
        apply_e11_if_needed(&config, initial_accent);
    }

    {
        let mut st = STATE.lock().unwrap();
        *st = Some(ListenerState {
            config,
            win_ver,
            last_accent: Some(initial_accent),
        });
    }

    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = encode_wide("ACCENTCOLORIZER_RUST_LISTENER");

        let mut wcex: WNDCLASSEXW = std::mem::zeroed();
        wcex.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        wcex.lpfnWndProc = Some(wndproc);
        wcex.hInstance = instance;
        wcex.lpszClassName = class_name.as_ptr();

        if RegisterClassExW(&wcex) == 0 {
            eprintln!("Failed to register window class.");
            return;
        }

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );

        if hwnd.is_null() {
            eprintln!("Failed to create listener window.");
            return;
        }

        // Register session notification
        let wts_mod = LoadLibraryW(encode_wide("wtsapi32.dll").as_ptr());
        if !wts_mod.is_null() {
            if let Some(proc) = GetProcAddress(wts_mod, b"WTSRegisterSessionNotification\0".as_ptr()) {
                let register: FnWTSRegisterSessionNotification = std::mem::transmute(proc);
                register(hwnd, NOTIFY_FOR_THIS_SESSION);
            }
        }

        let mut msg: MSG = std::mem::zeroed();
        while RUNNING.load(Ordering::SeqCst) && GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Cleanup WTS registration
        if !wts_mod.is_null() {
            if let Some(proc) = GetProcAddress(wts_mod, b"WTSUnRegisterSessionNotification\0".as_ptr()) {
                let unregister: FnWTSUnRegisterSessionNotification = std::mem::transmute(proc);
                unregister(hwnd);
            }
        }
    }
}

pub fn apply_all_in_memory(config: &AppConfig, win_ver: WindowsVersion, accent: Rgb) {
    let accent_h = accent.to_hsv().h;
    modify_sys_colors(accent_h);
    modify_styles(
        accent_h,
        win_ver,
        config.colorize_menus,
        config.colorize_progress_bar,
    );
}

fn apply_e11_if_needed(config: &AppConfig, light_accent: Rgb) {
    let dark_accent = match &config.custom_accent_hex {
        Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_dark_system_accent),
        None => get_current_dark_system_accent(),
    };

    let packages = find_e11_packages();
    if packages.is_empty() {
        return;
    }

    if is_elevated() {
        let _ = apply_e11_colorization(&packages, light_accent, dark_accent);
    } else {
        // Run elevated helper without interrupting user
        run_elevated_self(&["apply", "--e11-only"]);
    }
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DWMCOLORIZATIONCOLORCHANGED | WM_DPICHANGED | WM_THEMECHANGED | WM_SETTINGCHANGE => {
            let debounce = {
                let guard = STATE.lock().unwrap();
                guard.as_ref().map(|s| s.config.debounce_ms).unwrap_or(300) as u32
            };
            unsafe {
                SetTimer(hwnd, TIMER_COLOR_UPDATE, debounce, None);
            }
            0
        }
        WM_WTSSESSION_CHANGE => {
            if wparam == WTS_SESSION_UNLOCK {
                let debounce = {
                    let guard = STATE.lock().unwrap();
                    guard.as_ref().map(|s| s.config.debounce_ms).unwrap_or(300) as u32
                };
                unsafe {
                    SetTimer(hwnd, TIMER_COLOR_UPDATE, debounce, None);
                }
            }
            0
        }
        WM_TIMER => {
            if wparam == TIMER_COLOR_UPDATE {
                unsafe {
                    KillTimer(hwnd, TIMER_COLOR_UPDATE);
                }
                handle_color_change_event(true);
            }
            0
        }
        WM_DESTROY => {
            RUNNING.store(false, Ordering::SeqCst);
            unsafe {
                KillTimer(hwnd, TIMER_COLOR_UPDATE);
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn handle_color_change_event(force_reapply: bool) {
    let mut guard = STATE.lock().unwrap();
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };

    let current_accent = match &state.config.custom_accent_hex {
        Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_system_accent),
        None => get_current_system_accent(),
    };

    if !force_reapply && state.last_accent == Some(current_accent) {
        return;
    }

    state.last_accent = Some(current_accent);
    apply_all_in_memory(&state.config, state.win_ver, current_accent);

    if state.config.colorize_e11_svg && state.win_ver >= WindowsVersion::Win11 {
        apply_e11_if_needed(&state.config, current_accent);
    }
}
