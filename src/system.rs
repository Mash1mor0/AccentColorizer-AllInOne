use std::env;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, SYSTEMTIME,
};
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, OpenProcessToken,
};
use windows_sys::Win32::UI::Shell::{ShellExecuteW, SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, SW_HIDE,
};

use crate::color::encode_wide;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WindowsVersion {
    WinVista,
    Win7,
    Win8,
    Win10,
    Win11,
}

#[repr(C)]
struct OsVersionInfoExW {
    os_version_info_size: u32,
    major_version: u32,
    minor_version: u32,
    build_number: u32,
    platform_id: u32,
    csd_version: [u16; 128],
    service_pack_major: u16,
    service_pack_minor: u16,
    suite_mask: u16,
    product_type: u8,
    reserved: u8,
}

type RtlGetVersionFn = unsafe extern "system" fn(*mut OsVersionInfoExW) -> i32;

pub fn detect_windows_version() -> WindowsVersion {
    unsafe {
        let ntdll = GetModuleHandleW(encode_wide("ntdll.dll").as_ptr());
        if !ntdll.is_null() {
            let proc = GetProcAddress(ntdll, c"RtlGetVersion".as_ptr().cast());
            if let Some(proc) = proc {
                let rtl_get_version: RtlGetVersionFn = std::mem::transmute(proc);
                let mut info: OsVersionInfoExW = std::mem::zeroed();
                info.os_version_info_size = std::mem::size_of::<OsVersionInfoExW>() as u32;
                if rtl_get_version(&mut info) == 0 {
                    if info.major_version >= 10 {
                        if info.build_number >= 22000 {
                            return WindowsVersion::Win11;
                        }
                        return WindowsVersion::Win10;
                    } else if info.major_version == 6 {
                        if info.minor_version == 2 || info.minor_version == 3 {
                            return WindowsVersion::Win8;
                        } else if info.minor_version == 1 {
                            return WindowsVersion::Win7;
                        }
                    }
                }
            }
        }
    }
    WindowsVersion::WinVista
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret_len: u32 = 0;
        let success = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);

        success != 0 && elevation.TokenIsElevated != 0
    }
}

pub fn run_elevated_self(args: &[&str]) -> bool {
    if let Ok(exe_path) = env::current_exe() {
        let exe_str = exe_path.to_string_lossy().to_string();
        let args_joined = args
            .iter()
            .map(|a| {
                if a.contains(' ') {
                    format!("\"{}\"", a)
                } else {
                    a.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        let verb = encode_wide("runas");
        let file = encode_wide(&exe_str);
        let params = encode_wide(&args_joined);

        unsafe {
            let res = ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                params.as_ptr(),
                std::ptr::null(),
                SW_HIDE,
            );
            return (res as usize) > 32;
        }
    }
    false
}

pub fn acquire_single_instance(name: &str) -> Option<HANDLE> {
    unsafe {
        let wide_name = encode_wide(name);
        let mutex = CreateMutexW(std::ptr::null(), 1, wide_name.as_ptr());
        if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            if !mutex.is_null() {
                CloseHandle(mutex);
            }
            return None;
        }
        Some(mutex)
    }
}

pub fn release_single_instance(mutex: HANDLE) {
    unsafe {
        if !mutex.is_null() {
            windows_sys::Win32::System::Threading::ReleaseMutex(mutex);
            CloseHandle(mutex);
        }
    }
}

pub fn restart_explorer() {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    // Taskkill explorer.exe
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "explorer.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // Small delay
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Spawn new explorer.exe
    let _ = Command::new("explorer.exe")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn broadcast_theme_refresh() {
    const HWND_BROADCAST: windows_sys::Win32::Foundation::HWND = 0xFFFF as windows_sys::Win32::Foundation::HWND;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const WM_THEMECHANGED: u32 = 0x031A;
    const WM_SYSCOLORCHANGE: u32 = 0x0015;

    unsafe {
        let mut result = 0usize;
        let immersive_param = encode_wide("ImmersiveColorSet");
        let theme_param = encode_wide("WindowsThemeElement");

        // 1. Notify Windows Shell of association/icon change
        SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null());

        // 2. Broadcast ImmersiveColorSet setting change
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            immersive_param.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            150,
            &mut result,
        );

        // 3. Broadcast WindowsThemeElement setting change
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            theme_param.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            150,
            &mut result,
        );

        // 4. Broadcast WM_THEMECHANGED to refresh visual styles & XAML resources
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_THEMECHANGED,
            0,
            0,
            SMTO_ABORTIFHUNG,
            150,
            &mut result,
        );

        // 5. Broadcast WM_SYSCOLORCHANGE
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SYSCOLORCHANGE,
            0,
            0,
            SMTO_ABORTIFHUNG,
            150,
            &mut result,
        );

        // 6. Invalidate desktop view so icon selection plates pick up the new style immediately
        let progman = windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW(
            encode_wide("Progman").as_ptr(),
            std::ptr::null(),
        );
        if !progman.is_null() {
            let def_view = windows_sys::Win32::UI::WindowsAndMessaging::FindWindowExW(
                progman,
                std::ptr::null_mut(),
                encode_wide("SHELLDLL_DefView").as_ptr(),
                std::ptr::null(),
            );
            if !def_view.is_null() {
                let list_view = windows_sys::Win32::UI::WindowsAndMessaging::FindWindowExW(
                    def_view,
                    std::ptr::null_mut(),
                    encode_wide("SysListView32").as_ptr(),
                    std::ptr::null(),
                );
                if !list_view.is_null() {
                    windows_sys::Win32::Graphics::Gdi::InvalidateRect(list_view, std::ptr::null(), 1);
                }
                windows_sys::Win32::Graphics::Gdi::InvalidateRect(def_view, std::ptr::null(), 1);
            }
        }
    }
}

pub fn get_timestamp() -> String {
    unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut st);
        format!("{:02}:{:02}:{:02}", st.wHour, st.wMinute, st.wSecond)
    }
}

pub fn get_exe_dir() -> PathBuf {
    if let Ok(exe_path) = env::current_exe()
        && let Some(parent) = exe_path.parent()
    {
        return parent.to_path_buf();
    }
    PathBuf::from(".")
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        println!("[{}] {}", $crate::system::get_timestamp(), format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_err {
    ($($arg:tt)*) => {
        eprintln!("[{}] {}", $crate::system::get_timestamp(), format_args!($($arg)*))
    };
}
