#![windows_subsystem = "windows"]

use std::env;
use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

mod cli;
mod color;
mod e11_svg;
mod listener;
mod security;
mod system;
mod win32_theme;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Auto-elevate to Administrator by default if not already elevated
    if !system::is_elevated() {
        let args_str: Vec<&str> = args.iter().skip(1).map(|s| s.as_str()).collect();
        system::run_elevated_self(&args_str);
        return;
    }

    // If run from a command prompt with interactive CLI commands (apply, revert, --help, etc.),
    // attach to the caller's console so stdout/stderr print cleanly without popping up a new window.
    let is_interactive = args.len() > 1
        && matches!(
            args[1].to_ascii_lowercase().as_str(),
            "apply"
                | "--apply"
                | "-apply"
                | "revert"
                | "restore"
                | "--revert"
                | "-revert"
                | "autostart"
                | "--autostart"
                | "install"
                | "--install"
                | "uninstall"
                | "--uninstall"
                | "remove-autostart"
                | "--remove-autostart"
                | "--help"
                | "-h"
                | "help"
                | "/?"
                | "--version"
                | "-v"
        );
    if is_interactive {
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }

    cli::run_cli();
}
