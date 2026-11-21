use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM};
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, KEY_NOTIFY,
    REG_NOTIFY_CHANGE_LAST_SET,
};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilter, ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW,
    DispatchMessageW, GetMessageW, KillTimer, PostMessageW, PostQuitMessage, RegisterClassExW,
    SetTimer, TranslateMessage, MSG, MSGFLT_ADD, MSGFLT_ALLOW, WM_DESTROY, WM_DPICHANGED,
    WNDCLASSEXW,
};

use crate::color::{encode_wide, get_current_dark_system_accent, get_current_system_accent, Rgb};
use crate::e11_svg::{apply_e11_colorization, find_e11_packages};
use crate::system::{detect_windows_version, restart_explorer, WindowsVersion};
use crate::win32_theme::{modify_styles, modify_sys_colors};
use crate::{log_err, log_info};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub colorize_menus: bool,
    pub colorize_progress_bar: bool,
    pub colorize_e11_svg: bool,
    pub restart_explorer: bool,
    pub custom_accent_hex: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            colorize_menus: true,
            colorize_progress_bar: true,
            colorize_e11_svg: true,
            restart_explorer: false,
            custom_accent_hex: None,
        }
    }
}

const WM_SETTINGCHANGE: u32 = 0x001A;
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
const WM_THEMECHANGED: u32 = 0x031A;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WM_TIMER: u32 = 0x0113;
const WM_CLOSE: u32 = 0x0010;
const WM_QUERYENDSESSION: u32 = 0x0011;
const WM_SYSCOLORCHANGE: u32 = 0x0015;
const WM_ENDSESSION: u32 = 0x0016;

const WTS_SESSION_UNLOCK: usize = 0x8;
const NOTIFY_FOR_THIS_SESSION: u32 = 0;
const TIMER_TRAILING_CHECK: usize = 1002;

static RUNNING: AtomicBool = AtomicBool::new(true);
static IS_APPLYING: AtomicBool = AtomicBool::new(false);
static CLEANED_UP: AtomicBool = AtomicBool::new(false);
static CONFIG: Mutex<Option<AppConfig>> = Mutex::new(None);
static LAST_ACCENT: Mutex<Option<Rgb>> = Mutex::new(None);
static LAST_E11_ACCENT: Mutex<Option<Rgb>> = Mutex::new(None);
static LAST_APPLIED_TIME: Mutex<Option<Instant>> = Mutex::new(None);

type FnWTSRegisterSessionNotification = unsafe extern "system" fn(HWND, u32) -> i32;
type FnWTSUnRegisterSessionNotification = unsafe extern "system" fn(HWND) -> i32;

pub fn cleanup_on_exit() {
    if CLEANED_UP.swap(true, Ordering::SeqCst) {
        return;
    }
    log_info!("[Shutdown] Restoring factory icons and system colors...");
    crate::win32_theme::restore_sys_colors();
    let packages = find_e11_packages();
    if !packages.is_empty() {
        let _ = crate::e11_svg::revert_e11_colorization(&packages);
    }
}

unsafe extern "system" fn ctrl_handler(_ctrl_type: u32) -> i32 {
    cleanup_on_exit();
    0
}

pub fn run_listener(config: AppConfig) {
    let win_ver = detect_windows_version();

    unsafe {
        SetConsoleCtrlHandler(Some(ctrl_handler), 1);
    }

    // Initial apply on daemon startup
    let initial_accent = match &config.custom_accent_hex {
        Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_system_accent),
        None => get_current_system_accent(),
    };

    log_info!("[Listener] Initializing colors (Accent: {})...", initial_accent.to_hex());
    apply_all_in_memory(&config, win_ver, initial_accent);
    log_info!("[Win32] Applied system and theme colors in memory.");

    // Initial check for E11 SVG if enabled
    if config.colorize_e11_svg && win_ver >= WindowsVersion::Win11 {
        apply_e11_if_needed(&config, initial_accent);
    }

    {
        *CONFIG.lock().unwrap() = Some(config);
        *LAST_ACCENT.lock().unwrap() = Some(initial_accent);
        *LAST_E11_ACCENT.lock().unwrap() = Some(initial_accent);
        *LAST_APPLIED_TIME.lock().unwrap() = Some(Instant::now());
    }

    // Broadcast theme refresh pulse so running Explorer flushes its cache immediately
    crate::system::broadcast_theme_refresh();
    log_info!("[System] Broadcasted theme refresh pulse to flush Explorer cache.");

    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = encode_wide("ACCENTCOLORIZER_RUST_LISTENER");

        let mut wcex: WNDCLASSEXW = std::mem::zeroed();
        wcex.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        wcex.lpfnWndProc = Some(wndproc);
        wcex.hInstance = instance;
        wcex.lpszClassName = class_name.as_ptr();

        if RegisterClassExW(&wcex) == 0 {
            log_err!("Failed to register window class.");
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
            log_err!("Failed to create listener window.");
            return;
        }

        // CRITICAL FOR ELEVATED DAEMON (UIPI Bypass):
        // Allow broadcast messages from medium/lower integrity processes (e.g. explorer.exe, SystemSettings.exe)
        const ALLOWED_MESSAGES: [u32; 6] = [
            WM_SETTINGCHANGE,
            WM_THEMECHANGED,
            WM_DWMCOLORIZATIONCOLORCHANGED,
            WM_SYSCOLORCHANGE,
            WM_DPICHANGED,
            WM_WTSSESSION_CHANGE,
        ];

        for &m in &ALLOWED_MESSAGES {
            ChangeWindowMessageFilter(m, MSGFLT_ADD);
            ChangeWindowMessageFilterEx(hwnd, m, MSGFLT_ALLOW, std::ptr::null_mut());
        }

        // Secondary bulletproof mechanism: Monitor Windows Theme & Personalization registry keys
        let hwnd_raw = hwnd as usize;
        std::thread::spawn(move || {
            watch_registry_changes(hwnd_raw);
        });

        // Register session notification (lock/unlock)
        let wts_mod = LoadLibraryW(encode_wide("wtsapi32.dll").as_ptr());
        if !wts_mod.is_null()
            && let Some(proc) = GetProcAddress(wts_mod, c"WTSRegisterSessionNotification".as_ptr().cast()) {
                let register: FnWTSRegisterSessionNotification = std::mem::transmute(proc);
                register(hwnd, NOTIFY_FOR_THIS_SESSION);
            }

        log_info!("[Listener] Message loop active. Listening for Windows theme & color events...");

        let mut msg: MSG = std::mem::zeroed();
        while RUNNING.load(Ordering::SeqCst) && GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Cleanup WTS registration
        if !wts_mod.is_null()
            && let Some(proc) = GetProcAddress(wts_mod, c"WTSUnRegisterSessionNotification".as_ptr().cast()) {
                let unregister: FnWTSUnRegisterSessionNotification = std::mem::transmute(proc);
                unregister(hwnd);
            }

        cleanup_on_exit();
    }
}

fn watch_registry_changes(hwnd_raw: usize) {
    let hwnd = hwnd_raw as HWND;
    let keys_to_watch = [
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        r"Software\Microsoft\Windows\DWM",
        r"Control Panel\Desktop",
    ];

    let mut opened_keys = Vec::new();
    let mut events: Vec<HANDLE> = Vec::new();

    unsafe {
        for subkey in &keys_to_watch {
            let wide_key = encode_wide(subkey);
            let mut hkey = std::ptr::null_mut();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide_key.as_ptr(),
                0,
                KEY_NOTIFY,
                &mut hkey,
            ) == 0 {
                let event = CreateEventW(std::ptr::null(), 0, 0, std::ptr::null());
                if !event.is_null() {
                    opened_keys.push(hkey);
                    events.push(event);
                } else {
                    RegCloseKey(hkey);
                }
            }
        }

        if events.is_empty() {
            return;
        }

        for i in 0..events.len() {
            RegNotifyChangeKeyValue(
                opened_keys[i],
                1,
                REG_NOTIFY_CHANGE_LAST_SET,
                events[i],
                1,
            );
        }

        while RUNNING.load(Ordering::SeqCst) {
            let res = WaitForMultipleObjects(
                events.len() as u32,
                events.as_ptr(),
                0,
                1000,
            );

            if res < WAIT_OBJECT_0 + events.len() as u32 {
                let idx = (res - WAIT_OBJECT_0) as usize;
                // Post WM_THEMECHANGED into our window message queue.
                // Because this is posted from within our own process, UIPI never blocks it.
                PostMessageW(hwnd, WM_THEMECHANGED, 0, 0);

                // Re-register notification for this key
                RegNotifyChangeKeyValue(
                    opened_keys[idx],
                    1,
                    REG_NOTIFY_CHANGE_LAST_SET,
                    events[idx],
                    1,
                );
            }
        }

        for hkey in opened_keys {
            RegCloseKey(hkey);
        }
        for ev in events {
            CloseHandle(ev);
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
    // Avoid redundant disk I/O if SVGs are already patched with this exact color
    {
        let mut e11_guard = LAST_E11_ACCENT.lock().unwrap();
        if *e11_guard == Some(light_accent) {
            return;
        }
        *e11_guard = Some(light_accent);
    }

    let dark_accent = match &config.custom_accent_hex {
        Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_dark_system_accent),
        None => get_current_dark_system_accent(),
    };

    let packages = find_e11_packages();
    if !packages.is_empty() {
        match apply_e11_colorization(&packages, light_accent, dark_accent) {
            Ok(count) => log_info!("[E11 SVG] Successfully patched {} SVG glyph(s).", count),
            Err(e) => log_err!("[E11 SVG] Warning patching glyphs: {}", e),
        }
        if config.restart_explorer {
            log_info!("[Explorer] Restarting explorer.exe to flush icon cache...");
            restart_explorer();
        }
    }
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DWMCOLORIZATIONCOLORCHANGED
        | WM_DPICHANGED
        | WM_THEMECHANGED
        | WM_SETTINGCHANGE
        | WM_SYSCOLORCHANGE => {
            // If already applying, prevent re-entrancy (e.g. SetSysColors sending WM_SYSCOLORCHANGE)
            if IS_APPLYING.load(Ordering::SeqCst) {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }

            let msg_name = match msg {
                WM_DWMCOLORIZATIONCOLORCHANGED => "WM_DWMCOLORIZATIONCOLORCHANGED",
                WM_DPICHANGED => "WM_DPICHANGED",
                WM_THEMECHANGED => "WM_THEMECHANGED",
                WM_SETTINGCHANGE => "WM_SETTINGCHANGE",
                WM_SYSCOLORCHANGE => "WM_SYSCOLORCHANGE",
                _ => "UNKNOWN",
            };

            log_info!("[Event] Theme/accent change event: {} (0x{:04X})", msg_name, msg);

            let is_theme_changed = msg == WM_THEMECHANGED;

            // Trigger color update IMMEDIATELY on the very first message!
            // No debounce delay: runs simultaneously as Windows switches theme.
            trigger_immediate_color_update(is_theme_changed, msg_name);

            // Schedule a trailing check to catch any late palette adjustments from Windows
            unsafe {
                SetTimer(hwnd, TIMER_TRAILING_CHECK, 200, None);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_WTSSESSION_CHANGE => {
            if wparam == WTS_SESSION_UNLOCK && !IS_APPLYING.load(Ordering::SeqCst) {
                log_info!("[Event] Session unlock event");
                trigger_immediate_color_update(true, "WTS_SESSION_UNLOCK");
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_TIMER => {
            if wparam == TIMER_TRAILING_CHECK {
                unsafe {
                    KillTimer(hwnd, TIMER_TRAILING_CHECK);
                }
                if !IS_APPLYING.load(Ordering::SeqCst) {
                    trigger_immediate_color_update(false, "TRAILING_CHECK");
                }
            }
            0
        }
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION => {
            if wparam != 0 {
                RUNNING.store(false, Ordering::SeqCst);
                cleanup_on_exit();
            }
            0
        }
        WM_CLOSE | WM_DESTROY => {
            RUNNING.store(false, Ordering::SeqCst);
            cleanup_on_exit();
            unsafe {
                KillTimer(hwnd, TIMER_TRAILING_CHECK);
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn trigger_immediate_color_update(force: bool, source: &str) {
    // Atomic check to prevent re-entrant execution
    if IS_APPLYING.swap(true, Ordering::SeqCst) {
        return;
    }

    let config = match CONFIG.lock().unwrap().clone() {
        Some(c) => c,
        None => {
            IS_APPLYING.store(false, Ordering::SeqCst);
            return;
        }
    };
    let win_ver = detect_windows_version();

    let current_accent = match &config.custom_accent_hex {
        Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_system_accent),
        None => get_current_system_accent(),
    };

    let should_apply = {
        let mut last_guard = LAST_ACCENT.lock().unwrap();
        let mut last_time_guard = LAST_APPLIED_TIME.lock().unwrap();
        let now = Instant::now();

        let accent_changed = *last_guard != Some(current_accent);
        let time_since_last = last_time_guard
            .map(|t| now.duration_since(t).as_millis())
            .unwrap_or(u128::MAX);

        // Apply if accent color actually changed, or if forced (WM_THEMECHANGED) and not repeated within 1000ms
        if accent_changed || (force && time_since_last > 1000) {
            *last_guard = Some(current_accent);
            *last_time_guard = Some(now);
            true
        } else {
            false
        }
    }; // ALL MUTEXES ARE DROPPED HERE! NO LOCKS HELD DURING APPLICATION!

    if should_apply {
        log_info!(
            "[Listener] Theme switch detected ({})! Applying colors immediately (Accent: {})...",
            source,
            current_accent.to_hex()
        );

        // 1. In-memory Win32 theme & syscolors
        apply_all_in_memory(&config, win_ver, current_accent);
        log_info!("[Win32] Applied system and theme colors in memory.");

        // 2. Windows 11 SVG glyphs
        if config.colorize_e11_svg && win_ver >= WindowsVersion::Win11 {
            apply_e11_if_needed(&config, current_accent);
        }
    }

    IS_APPLYING.store(false, Ordering::SeqCst);
}
