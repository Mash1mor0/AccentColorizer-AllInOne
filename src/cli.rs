use std::env;
use windows_sys::Win32::System::Console::AllocConsole;

use crate::color::{get_current_dark_system_accent, get_current_system_accent, Rgb};
use crate::e11_svg::{apply_e11_colorization, find_e11_packages, revert_e11_colorization};
use crate::listener::{apply_all_in_memory, run_listener, AppConfig};
use crate::system::{
    acquire_single_instance, detect_windows_version, release_single_instance, restart_explorer,
    WindowsVersion,
};
use crate::win32_theme::restore_sys_colors;

pub fn parse_flags(args: &[String]) -> (AppConfig, bool) {
    let mut config = AppConfig::default();
    let mut foreground = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].to_ascii_lowercase().as_str() {
            "--no-progress" => {
                config.colorize_progress_bar = false;
            }
            "--no-win32" | "--no-menus" => {
                config.colorize_menus = false;
            }
            "--no-svg" | "--no-e11" => {
                config.colorize_e11_svg = false;
            }
            "--restart-explorer" => {
                config.restart_explorer = true;
            }
            "--foreground" | "-f" => {
                foreground = true;
            }
            "--color" if i + 1 < args.len() => {
                i += 1;
                config.custom_accent_hex = Some(args[i].clone());
            }
            _ => {}
        }
        i += 1;
    }

    (config, foreground)
}

pub fn run_cli() {
    let args: Vec<String> = env::args().collect();
    let win_ver = detect_windows_version();

    if args.len() <= 1 {
        // Default: Silent background daemon with all defaults
        start_daemon_mode(AppConfig::default(), false);
        return;
    }

    let first_arg = args[1].to_ascii_lowercase();

    match first_arg.as_str() {
        "apply" | "--apply" | "-apply" => {
            let (mut config, _) = parse_flags(&args[2..]);

            // Handle legacy/convenience apply-only flags
            let mut i = 2;
            while i < args.len() {
                match args[i].to_ascii_lowercase().as_str() {
                    "--all" => {
                        config.colorize_menus = true;
                        config.colorize_progress_bar = true;
                        config.colorize_e11_svg = true;
                    }
                    "--win32-only" => {
                        config.colorize_menus = true;
                        config.colorize_progress_bar = true;
                        config.colorize_e11_svg = false;
                    }
                    "--e11-only" => {
                        config.colorize_menus = false;
                        config.colorize_progress_bar = false;
                        config.colorize_e11_svg = true;
                    }
                    _ => {}
                }
                i += 1;
            }

            let light_accent = match &config.custom_accent_hex {
                Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_system_accent),
                None => get_current_system_accent(),
            };

            let win32 = config.colorize_menus || config.colorize_progress_bar;
            let e11 = config.colorize_e11_svg && win_ver >= WindowsVersion::Win11;

            if win32 {
                crate::log_info!("[Win32] Applying theme & system colors in memory...");
                apply_all_in_memory(&config, win_ver, light_accent);
                crate::log_info!("[Win32] Done.");
            }

            if e11 {
                let packages = find_e11_packages();
                let dark_accent = match &config.custom_accent_hex {
                    Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_dark_system_accent),
                    None => get_current_dark_system_accent(),
                };

                if !packages.is_empty() {
                    crate::log_info!("[E11 SVG] Applying icon colors...");
                    if let Ok(count) = apply_e11_colorization(&packages, light_accent, dark_accent) {
                        crate::log_info!("[E11 SVG] Patched {} SVG icon(s).", count);
                        if config.restart_explorer {
                            restart_explorer();
                        }
                    }
                }
            }

            if win32 || e11 {
                crate::system::broadcast_theme_refresh();
            }
        }
        "revert" | "restore" | "--revert" | "-revert" => {
            let mut force_restart_explorer = true; // Auto-restart on revert to flush cached icons
            for arg in &args[2..] {
                if arg.eq_ignore_ascii_case("--no-restart-explorer") {
                    force_restart_explorer = false;
                }
            }

            crate::log_info!("[Revert] Resetting system and theme colors in memory...");
            restore_sys_colors();

            let packages = find_e11_packages();
            crate::log_info!("[Revert] Restoring pristine originals (.original) & resetting TrustedInstaller ownership...");
            if let Ok(count) = revert_e11_colorization(&packages) {
                crate::log_info!(
                    "[Revert] Successfully restored {} file(s). TrustedInstaller ownership intact.",
                    count
                );
            }

            if force_restart_explorer {
                crate::log_info!("[System] Restarting explorer.exe to flush icon cache...");
                restart_explorer();
            }
        }
        "--help" | "-h" | "help" | "/?" => {
            print_help();
        }
        "--version" | "-v" => {
            println!("AccentColorizer v{}", env!("CARGO_PKG_VERSION"));
        }
        "daemon" | "--daemon" | "-d" => {
            let (config, foreground) = parse_flags(&args[2..]);
            start_daemon_mode(config, foreground);
        }
        _ => {
            // User launched with flags directly (e.g. from shortcut Target: AccentColorizer.exe --no-progress)
            let (config, foreground) = parse_flags(&args[1..]);
            start_daemon_mode(config, foreground);
        }
    }
}

fn start_daemon_mode(config: AppConfig, foreground: bool) {
    const MUTEX_NAME: &str = "ACCENTCOLORIZER_ALLINONE_MUTEX";
    let mutex = match acquire_single_instance(MUTEX_NAME) {
        Some(m) => m,
        None => {
            if foreground {
                unsafe {
                    if windows_sys::Win32::System::Console::AttachConsole(
                        windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
                    ) == 0
                    {
                        AllocConsole();
                    }
                }
                crate::log_err!("AccentColorizer daemon is already running in background.");
            }
            return;
        }
    };

    if foreground {
        unsafe {
            if windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            ) == 0
            {
                AllocConsole();
            }
        }
        crate::log_info!("Starting AccentColorizer in foreground daemon mode...");
    }

    run_listener(config);

    release_single_instance(mutex);
}

fn print_help() {
    println!(
        r#"AccentColorizer (Rust) - All-In-One Single Executable
Unified Accent Colorizer for Windows 10 & 11

Usage:
  AccentColorizer.exe [COMMAND] [OPTIONS]

By default (without arguments or with options), runs as a silent resident background daemon.
All features are ENABLED by default (Win32 menus, progress bars, Win11 SVG icons).
Accent color is detected AUTOMATICALLY from Windows personalization settings.

Commands:
  apply                 Apply accent colors immediately once and exit
  revert, restore       Restore pristine originals (.original) and restore TrustedInstaller ownership (DISM/SFC safe)
  --help, -h            Show this help message
  --version, -v         Show version

Options (for background daemon and 'apply'):
  --no-progress         Disable progress bar colorization
  --no-win32            Disable classic Win32 menu & control colorization
  --no-svg              Disable Windows 11 File Explorer SVG icon patching
  --color <#RRGGBB>     Override Windows accent color with a fixed custom hex code
  --restart-explorer    Restart explorer.exe after patching icons
  --foreground, -f      Run daemon attached to console (shows live log output)

Options for 'revert':
  --no-restart-explorer Do not restart explorer.exe after restoring

Shortcut usage:
  To customize when running on startup, add options to the shortcut's Target field:
  "C:\Personal\AccentColorizer\AccentColorizer.exe" --no-progress --color #0078D4
"#
    );
}
