use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, RegCloseKey, RegOpenKeyExW, RegQueryValueExW, KEY_READ,
};

use crate::color::encode_wide;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_true")]
    pub colorize_menus: bool,

    #[serde(default = "default_false")]
    pub colorize_progress_bar: bool,

    #[serde(default = "default_true")]
    pub colorize_e11_svg: bool,

    #[serde(default = "default_false")]
    pub restart_explorer: bool,

    #[serde(default)]
    pub custom_accent_hex: Option<String>,
}

const fn default_true() -> bool {
    true
}

const fn default_false() -> bool {
    false
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            colorize_menus: true,
            colorize_progress_bar: false,
            colorize_e11_svg: true,
            restart_explorer: false,
            custom_accent_hex: None,
        }
    }
}

pub fn get_exe_dir() -> PathBuf {
    if let Ok(exe_path) = env::current_exe()
        && let Some(parent) = exe_path.parent() {
            return parent.to_path_buf();
        }
    PathBuf::from(".")
}

pub fn get_config_path() -> PathBuf {
    get_exe_dir().join("config.toml")
}

pub fn load_config() -> AppConfig {
    let path = get_config_path();
    if path.exists()
        && let Ok(content) = fs::read_to_string(&path)
            && let Ok(cfg) = toml::from_str::<AppConfig>(&content) {
                return cfg;
            }

    // Fallback: check registry HKCU\Software\AccentColorizer
    let mut config = AppConfig::default();
    if let Some(reg_progress) = read_registry_bool(r"Software\AccentColorizer", "ColorizeProgressBar") {
        config.colorize_progress_bar = reg_progress;
    }

    config
}

pub fn create_default_config_file() -> std::io::Result<PathBuf> {
    let path = get_config_path();
    let config = AppConfig::default();

    let custom_hex_str = match &config.custom_accent_hex {
        Some(hex) => format!("custom_accent_hex = \"{}\"", hex),
        None => "# custom_accent_hex = \"#0078D4\"".to_string(),
    };

    let toml_str = format!(
        r##"# AccentColorizer Configuration

# Colorize classic menus and context menus (Win32)
colorize_menus = {}

# Colorize classic progress bars
colorize_progress_bar = {}

# Colorize Windows 11 File Explorer SVG icons on disk
colorize_e11_svg = {}

# Automatically restart explorer.exe after patching or reverting SVG icons
restart_explorer = {}

# Optional: Override system accent color with a custom hex code (e.g. "#FF5500")
# Leave unset or empty to follow Windows system accent
{}
"##,
        config.colorize_menus,
        config.colorize_progress_bar,
        config.colorize_e11_svg,
        config.restart_explorer,
        custom_hex_str
    );

    fs::write(&path, toml_str)?;
    Ok(path)
}

fn read_registry_bool(subkey: &str, value_name: &str) -> Option<bool> {
    unsafe {
        let mut hkey = std::ptr::null_mut();
        let subkey_w = encode_wide(subkey);
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey_w.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return None;
        }

        let val_w = encode_wide(value_name);
        let mut val_type = 0u32;
        let mut data = 0u32;
        let mut data_len = std::mem::size_of::<u32>() as u32;

        let status = RegQueryValueExW(
            hkey,
            val_w.as_ptr(),
            std::ptr::null(),
            &mut val_type,
            &mut data as *mut u32 as *mut u8,
            &mut data_len,
        );
        RegCloseKey(hkey);

        if status == 0 {
            Some(data != 0)
        } else {
            None
        }
    }
}
