use std::env;
use windows_sys::Win32::System::Console::AllocConsole;

use crate::color::{get_current_dark_system_accent, get_current_system_accent, Rgb};
use crate::config::{create_default_config_file, get_config_path, load_config, AppConfig};
use crate::e11_svg::{
    apply_e11_colorization, ensure_backup_created, find_e11_packages, get_backup_root_dir,
    revert_e11_colorization,
};
use crate::listener::{apply_all_in_memory, run_listener};
use crate::system::{
    acquire_single_instance, detect_windows_version, release_single_instance, restart_explorer,
    WindowsVersion,
};
use crate::win32_theme::restore_sys_colors;

pub fn run_cli() {
    let args: Vec<String> = env::args().collect();
    let config = load_config();
    let win_ver = detect_windows_version();

    if args.len() <= 1 {
        // Default: Silent background daemon
        start_daemon_mode(config, false);
        return;
    }

    let command = args[1].to_ascii_lowercase();

    match command.as_str() {
        "daemon" | "--daemon" | "-d" => {
            let foreground = args.iter().any(|a| a == "--foreground" || a == "-f");
            start_daemon_mode(config, foreground);
        }
        "apply" | "--apply" | "-apply" => {
            let mut win32 = true;
            let mut e11 = config.colorize_e11_svg && win_ver >= WindowsVersion::Win11;
            let mut force_restart_explorer = config.restart_explorer;
            let mut custom_color = config.custom_accent_hex.clone();

            let mut i = 2;
            while i < args.len() {
                match args[i].to_ascii_lowercase().as_str() {
                    "--all" => {
                        win32 = true;
                        e11 = true;
                    }
                    "--win32-only" => {
                        win32 = true;
                        e11 = false;
                    }
                    "--e11-only" => {
                        win32 = false;
                        e11 = true;
                    }
                    "--restart-explorer" => {
                        force_restart_explorer = true;
                    }
                    "--color" if i + 1 < args.len() => {
                        i += 1;
                        custom_color = Some(args[i].clone());
                    }
                    _ => {}
                }
                i += 1;
            }

            let light_accent = match &custom_color {
                Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_system_accent),
                None => get_current_system_accent(),
            };

            if win32 {
                crate::log_info!("[Win32] Applying theme & system colors in memory...");
                apply_all_in_memory(&config, win_ver, light_accent);
                crate::log_info!("[Win32] Done.");
            }

            if e11 {
                let packages = find_e11_packages();
                let dark_accent = match &custom_color {
                    Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_dark_system_accent),
                    None => get_current_dark_system_accent(),
                };

                if !packages.is_empty() {
                    crate::log_info!("[E11 SVG] Applying icon colors...");
                    if let Ok(count) = apply_e11_colorization(&packages, light_accent, dark_accent) {
                        crate::log_info!("[E11 SVG] Patched {} SVG icon(s).", count);
                        if force_restart_explorer {
                            restart_explorer();
                        }
                    }
                }
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
            crate::log_info!("[Revert] Restoring pristine originals from backup & resetting TrustedInstaller ownership...");
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
        "backup" | "--backup" => {
            crate::log_info!("[Backup] Discovering packages...");
            let packages = find_e11_packages();
            let backup_dir = get_backup_root_dir();
            crate::log_info!("[Backup] Target backup directory: {}", backup_dir.display());
            match ensure_backup_created(&packages) {
                Ok(_) => crate::log_info!("[Backup] Pristine backup verified/created successfully."),
                Err(e) => crate::log_err!("[Backup] Failed to create backup: {}", e),
            }
        }
        "--init-config" => match create_default_config_file() {
            Ok(path) => crate::log_info!("Created config file at: {}", path.display()),
            Err(e) => crate::log_err!("Failed to create config file: {}", e),
        },
        "--help" | "-h" | "help" | "/?" => {
            print_help();
        }
        cmd => {
            crate::log_err!("Unknown command: '{}'. Use --help for usage.", cmd);
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
        let cfg_path = get_config_path();
        if cfg_path.exists() {
            crate::log_info!("Loaded config: {}", cfg_path.display());
        }
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

Commands:
  daemon [--foreground] Run background resident listener (default mode when no command specified)
  apply                 Apply accent colors immediately
  revert, restore       Restore original untouched SVGs from backup and restore TrustedInstaller ownership (DISM/SFC safe)
  backup                Create pristine backup of Windows 11 Explorer SVG icons without modifying anything
  --init-config         Generate default config.toml file next to executable (optional)
  --help, -h            Show this help message

Options for 'apply':
  --all                 Apply both Win32 memory theme/syscolors and Windows 11 SVG icons
  --win32-only          Apply only Win32 in-memory theme and system colors
  --e11-only            Apply only Windows 11 File Explorer SVG icons
  --restart-explorer    Restart explorer.exe after applying
  --color <#RRGGBB>     Override accent color with custom hex code

Options for 'revert':
  --no-restart-explorer Do not restart explorer.exe after restoring

Storage:
  The program runs as a single self-contained .exe file with Administrator privileges.
  The 'backup/' folder is stored in the same directory as AccentColorizer.exe.
"#
    );
}
