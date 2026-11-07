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
    acquire_single_instance, detect_windows_version, is_elevated, release_single_instance,
    restart_explorer, run_elevated_self, WindowsVersion,
};

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
                println!("[Win32] Applying theme & system colors in memory...");
                apply_all_in_memory(&config, win_ver, light_accent);
                println!("[Win32] Done.");
            }

            if e11 {
                if !is_elevated() {
                    let mut elevated_args = vec!["apply", "--e11-only"];
                    if force_restart_explorer {
                        elevated_args.push("--restart-explorer");
                    }
                    let color_arg;
                    if let Some(c) = &custom_color {
                        color_arg = c.clone();
                        elevated_args.push("--color");
                        elevated_args.push(&color_arg);
                    }
                    run_elevated_self(&elevated_args);
                    return;
                }

                let dark_accent = match &custom_color {
                    Some(hex) => Rgb::from_hex(hex).unwrap_or_else(get_current_dark_system_accent),
                    None => get_current_dark_system_accent(),
                };

                let packages = find_e11_packages();
                if !packages.is_empty() {
                    if let Ok(_) = apply_e11_colorization(&packages, light_accent, dark_accent) {
                        if force_restart_explorer {
                            restart_explorer();
                        }
                    }
                }
            }
        }
        "revert" | "restore" | "--revert" | "-revert" => {
            let mut force_restart_explorer = config.restart_explorer;
            for arg in &args[2..] {
                if arg.eq_ignore_ascii_case("--restart-explorer") {
                    force_restart_explorer = true;
                }
            }

            if !is_elevated() {
                let mut elevated_args = vec!["revert"];
                if force_restart_explorer {
                    elevated_args.push("--restart-explorer");
                }
                run_elevated_self(&elevated_args);
                return;
            }

            let packages = find_e11_packages();
            if let Ok(_) = revert_e11_colorization(&packages) {
                if force_restart_explorer {
                    restart_explorer();
                }
            }
        }
        "backup" | "--backup" => {
            println!("[Backup] Discovering packages...");
            let packages = find_e11_packages();
            let backup_dir = get_backup_root_dir();
            println!("[Backup] Target backup directory: {}", backup_dir.display());
            match ensure_backup_created(&packages) {
                Ok(_) => println!("[Backup] Pristine backup verified/created successfully."),
                Err(e) => eprintln!("[Backup] Failed to create backup: {}", e),
            }
        }
        "--init-config" => match create_default_config_file() {
            Ok(path) => println!("Created config file at: {}", path.display()),
            Err(e) => eprintln!("Failed to create config file: {}", e),
        },
        "--help" | "-h" | "help" | "/?" => {
            print_help();
        }
        cmd => {
            eprintln!("Unknown command: '{}'. Use --help for usage.", cmd);
        }
    }
}

fn start_daemon_mode(config: AppConfig, foreground: bool) {
    const MUTEX_NAME: &str = "ACCENTCOLORIZER_ALLINONE_MUTEX";
    let mutex = match acquire_single_instance(MUTEX_NAME) {
        Some(m) => m,
        None => {
            return;
        }
    };

    if foreground {
        unsafe {
            AllocConsole();
        }
        println!("Starting AccentColorizer in foreground daemon mode...");
        let cfg_path = get_config_path();
        if cfg_path.exists() {
            println!("Loaded config: {}", cfg_path.display());
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
  --win32-only          Apply only Win32 in-memory theme and system colors (no admin rights needed)
  --e11-only            Apply only Windows 11 File Explorer SVG icons (requests elevation if needed)
  --restart-explorer    Restart explorer.exe after applying
  --color <#RRGGBB>     Override accent color with custom hex code

Options for 'revert':
  --restart-explorer    Restart explorer.exe after restoring

Storage:
  The program runs as a single self-contained .exe file.
  The 'backup/' folder is stored in the same directory as AccentColorizer.exe.
"#
    );
}
