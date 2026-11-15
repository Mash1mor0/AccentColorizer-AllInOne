use std::fs;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::color::{blend_colors, Rgb};
use crate::config::get_exe_dir;

const CREATE_NO_WINDOW: u32 = 0x08000000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x00000400;

const DEFAULT_LIGHT_HEX: &str = "#0078D4";
const DEFAULT_DARK_HEX: &str = "#4CC2FF";
const SPOTLIGHT_DARK_BASE: &str = "#251840";
const SPOTLIGHT_DEFAULT_LIGHT: &str = "#0C59A4";

pub struct DiscoveredPackage {
    pub package_name: String,
    pub contrast_standard_dir: PathBuf,
}

pub fn get_backup_root_dir() -> PathBuf {
    get_exe_dir().join("backup")
}

pub fn find_e11_packages() -> Vec<DiscoveredPackage> {
    let mut packages = Vec::new();

    let win_dir = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let sys_apps = Path::new(&win_dir).join("SystemApps");

    if !sys_apps.exists() {
        return packages;
    }

    let mut scan_dirs = vec![sys_apps.clone()];
    let sxs = sys_apps.join("SxS");
    if sxs.exists() {
        scan_dirs.push(sxs);
    }

    for base_dir in scan_dirs {
        if let Ok(entries) = fs::read_dir(&base_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                let target = path
                    .join("FileExplorerExtensions")
                    .join("Assets")
                    .join("images")
                    .join("contrast-standard");

                if target.exists()
                    && (get_theme_dir(&target, "light").is_some()
                        || get_theme_dir(&target, "dark").is_some())
                {
                    let name = entry.file_name().to_string_lossy().to_string();
                    packages.push(DiscoveredPackage {
                        package_name: name,
                        contrast_standard_dir: target,
                    });
                }
            }
        }
    }

    packages
}

pub fn get_theme_dir(contrast_standard: &Path, theme: &str) -> Option<PathBuf> {
    let p1 = contrast_standard.join(format!("theme-{}", theme));
    if p1.exists() {
        return Some(p1);
    }
    let p2 = contrast_standard.join(theme);
    if p2.exists() {
        return Some(p2);
    }
    None
}

pub fn ensure_backup_created(packages: &[DiscoveredPackage]) -> io::Result<()> {
    let backup_root = get_backup_root_dir();

    for pkg in packages {
        let pkg_backup_dir = backup_root.join(&pkg.package_name);

        for theme in ["light", "dark"] {
            let src_theme_dir = match get_theme_dir(&pkg.contrast_standard_dir, theme) {
                Some(dir) => dir,
                None => continue,
            };

            let dst_theme_dir = pkg_backup_dir.join(format!("theme-{}", theme));
            fs::create_dir_all(&dst_theme_dir)?;

            if let Ok(files) = fs::read_dir(&src_theme_dir) {
                for file_entry in files.flatten() {
                    let file_path = file_entry.path();
                    if file_path.extension().and_then(|s| s.to_str()) == Some("svg") {
                        let file_name = file_entry.file_name();
                        let backup_file = dst_theme_dir.join(&file_name);

                        // Only copy if backup file does NOT exist yet
                        // Preserves untouched Windows original
                        if !backup_file.exists() {
                            fs::copy(&file_path, &backup_file)?;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

pub fn is_reparse_point(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| (m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0)
        .unwrap_or(false)
}

pub fn is_dir_writable(dir: &Path) -> bool {
    if !dir.exists() {
        return false;
    }
    let test_file = dir.join(".ac_write_test");
    match fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&test_file)
    {
        Ok(_) => {
            let _ = fs::remove_file(test_file);
            true
        }
        Err(_) => false,
    }
}

fn write_file_with_retry(path: &Path, content: &str, retries: u32, delay_ms: u64) -> io::Result<()> {
    let mut last_err = None;
    for i in 0..retries {
        match fs::write(path, content) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if i + 1 < retries {
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                }
            }
        }
    }
    Err(last_err.unwrap())
}

pub fn apply_e11_colorization(
    packages: &[DiscoveredPackage],
    light_rgb: Rgb,
    dark_rgb: Rgb,
) -> Result<usize, String> {
    ensure_backup_created(packages).map_err(|e| format!("Failed to create backup: {}", e))?;

    let backup_root = get_backup_root_dir();
    let target_light_hex = light_rgb.to_hex();
    let target_dark_hex = dark_rgb.to_hex();
    let target_spotlight_hex = blend_colors(SPOTLIGHT_DARK_BASE, &target_light_hex, 82);

    let mut modified_count = 0;

    for pkg in packages {
        let pkg_backup_dir = backup_root.join(&pkg.package_name);

        for theme in ["light", "dark"] {
            let target_theme_dir = match get_theme_dir(&pkg.contrast_standard_dir, theme) {
                Some(dir) => dir,
                None => continue,
            };
            let backup_theme_dir = pkg_backup_dir.join(format!("theme-{}", theme));

            if !backup_theme_dir.exists() {
                continue;
            }

            // Only take ownership & grant write if not already writable!
            // When Junction is active or permissions exist, takeown/icacls are skipped completely.
            if !is_dir_writable(&target_theme_dir) {
                if let Err(_) = take_ownership_and_grant_write(&target_theme_dir) {
                    return Err(format!(
                        "Permission denied for '{}'. Run with Administrator privileges.",
                        target_theme_dir.display()
                    ));
                }
            }

            if let Ok(files) = fs::read_dir(&backup_theme_dir) {
                for file_entry in files.flatten() {
                    let file_name = file_entry.file_name();
                    let backup_file_path = file_entry.path();
                    let target_file_path = target_theme_dir.join(&file_name);

                    if backup_file_path.extension().and_then(|s| s.to_str()) != Some("svg") {
                        continue;
                    }

                    let original_content = match fs::read_to_string(&backup_file_path) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!(
                                "[E11 SVG] Failed to read backup {}: {}",
                                backup_file_path.display(),
                                e
                            );
                            continue;
                        }
                    };

                    let patched_content = if theme == "light" {
                        replace_color_ignore_case(&original_content, DEFAULT_LIGHT_HEX, &target_light_hex)
                            .replace(SPOTLIGHT_DEFAULT_LIGHT, &target_spotlight_hex)
                    } else {
                        replace_color_ignore_case(&original_content, DEFAULT_DARK_HEX, &target_dark_hex)
                    };

                    // Write with 3 retries (50ms interval) to avoid sharing violation locks
                    match write_file_with_retry(&target_file_path, &patched_content, 3, 50) {
                        Ok(_) => modified_count += 1,
                        Err(e) => {
                            eprintln!(
                                "[E11 SVG] Warning: failed to write {}: {}",
                                target_file_path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(modified_count)
}

pub fn revert_e11_colorization(packages: &[DiscoveredPackage]) -> Result<usize, String> {
    let backup_root = get_backup_root_dir();
    if !backup_root.exists() {
        return Err("No backup directory found. Nothing to restore.".to_string());
    }

    let mut restored_count = 0;

    for pkg in packages {
        let pkg_backup_dir = backup_root.join(&pkg.package_name);
        let target_junction = &pkg.contrast_standard_dir;

        // If junction exists, remove junction link and recreate genuine directory
        if is_reparse_point(target_junction) {
            if let Some(parent) = target_junction.parent() {
                let _ = take_ownership_and_grant_write(parent);
            }
            let _ = fs::remove_dir(target_junction);
            let _ = fs::create_dir_all(target_junction);
        }

        for theme in ["light", "dark"] {
            let target_theme_dir = match get_theme_dir(&pkg.contrast_standard_dir, theme) {
                Some(dir) => dir,
                None => {
                    let fallback = pkg.contrast_standard_dir.join(format!("theme-{}", theme));
                    let _ = fs::create_dir_all(&fallback);
                    fallback
                }
            };
            let backup_theme_dir = pkg_backup_dir.join(format!("theme-{}", theme));

            if !backup_theme_dir.exists() {
                continue;
            }

            // Ensure write access to restore files
            let _ = take_ownership_and_grant_write(&target_theme_dir);

            if let Ok(files) = fs::read_dir(&backup_theme_dir) {
                for file_entry in files.flatten() {
                    let file_name = file_entry.file_name();
                    let backup_file_path = file_entry.path();
                    let target_file_path = target_theme_dir.join(&file_name);

                    if backup_file_path.extension().and_then(|s| s.to_str()) != Some("svg") {
                        continue;
                    }

                    if fs::copy(&backup_file_path, &target_file_path).is_ok() {
                        restored_count += 1;
                    }
                }
            }

            // Crucial for DISM and SFC: Restore ownership back to TrustedInstaller and reset permissions
            restore_trusted_installer(&target_theme_dir);
        }

        restore_trusted_installer(target_junction);
        if let Some(parent) = target_junction.parent() {
            restore_trusted_installer(parent);
        }
    }

    // Clean up local contrast-standard storage folder next to exe if left over from previous versions
    let local_storage = get_exe_dir().join("contrast-standard");
    if local_storage.exists() {
        let _ = fs::remove_dir_all(local_storage);
    }

    Ok(restored_count)
}

fn replace_color_ignore_case(src: &str, old_hex: &str, new_hex: &str) -> String {
    let old_upper = old_hex.to_ascii_uppercase();
    let old_lower = old_hex.to_ascii_lowercase();

    src.replace(&old_upper, new_hex).replace(&old_lower, new_hex)
}

fn take_ownership_and_grant_write(dir: &Path) -> Result<(), io::Error> {
    let path_str = dir.to_string_lossy();

    // takeown /F "path" /R /D Y
    let _ = Command::new("takeown.exe")
        .args(["/F", &path_str, "/R", "/D", "Y"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // icacls "path" /grant *S-1-5-32-544:F /T /C /Q (Administrators group SID)
    // and *S-1-5-11:F (Authenticated Users SID)
    let status = Command::new("icacls.exe")
        .args([&path_str, "/grant", "*S-1-5-32-544:F", "/T", "/C", "/Q"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    let _ = Command::new("icacls.exe")
        .args([&path_str, "/grant", "*S-1-5-11:F", "/T", "/C", "/Q"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Failed to grant write permissions",
        ));
    }

    Ok(())
}

fn restore_trusted_installer(dir: &Path) {
    let path_str = dir.to_string_lossy();

    // Set owner back to TrustedInstaller:
    // icacls "path" /setowner "NT SERVICE\TrustedInstaller" /T /C /Q
    let _ = Command::new("icacls.exe")
        .args([
            &path_str,
            "/setowner",
            "NT SERVICE\\TrustedInstaller",
            "/T",
            "/C",
            "/Q",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // Reset ACLs back to inheritance:
    // icacls "path" /reset /T /C /Q
    let _ = Command::new("icacls.exe")
        .args([&path_str, "/reset", "/T", "/C", "/Q"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
