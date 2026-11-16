#![windows_subsystem = "windows"]

use std::env;
use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

mod cli;
mod color;
mod config;
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

    // If run from a command prompt with CLI commands (apply, revert, --help, etc.),
    // attach to the caller's console so stdout/stderr print cleanly without popping up a new window.
    if args.len() > 1 && args[1] != "daemon" && args[1] != "--daemon" && args[1] != "-d" {
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }

    cli::run_cli();
}
