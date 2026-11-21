use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use crate::color::{blend_colors, Rgb};
use crate::system::get_exe_dir;

const DEFAULT_LIGHT_HEX: &str = "#0078D4";
const DEFAULT_DARK_HEX: &str = "#4CC2FF";
const SPOTLIGHT_DARK_BASE: &str = "#251840";
const SPOTLIGHT_DEFAULT_LIGHT: &str = "#0C59A4";

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x00000400;
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone)]
pub struct DiscoveredPackage {
    pub package_name: String,
    pub images_dir: PathBuf,
    pub contrast_standard_dir: PathBuf,
    pub genuine_orig_dir: PathBuf,
}

#[derive(Clone)]
struct SvgTemplate {
    rel_path: PathBuf,
    content: String,
    theme: &'static str,
    is_spotlight: bool,
}

static TEMPLATES: Mutex<Option<HashMap<String, Vec<SvgTemplate>>>> = Mutex::new(None);

pub fn is_reparse_point(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| (m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0)
        .unwrap_or(false)
}

pub fn create_junction(junction: &Path, target: &Path) -> io::Result<()> {
    let junction_str = junction.to_string_lossy();
    let target_str = target.to_string_lossy();
    let status = Command::new("cmd.exe")
        .args(["/C", "mklink", "/J", &junction_str, &target_str])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "mklink /J failed with exit code: {:?}",
            status.code()
        )))
    }
}

pub fn grant_app_packages_read(dir: &Path) {
    let dir_str = dir.to_string_lossy();
    // Grant ALL APPLICATION PACKAGES (S-1-15-2-1)
    let _ = Command::new("icacls.exe")
        .args([&dir_str, "/grant", "*S-1-15-2-1:(OI)(CI)(RX)", "/T", "/C", "/Q"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    // Grant BUILTIN\Users (S-1-5-32-545)
    let _ = Command::new("icacls.exe")
        .args([&dir_str, "/grant", "*S-1-5-32-545:(OI)(CI)(RX)", "/T", "/C", "/Q"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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

                let images_dir = path
                    .join("FileExplorerExtensions")
                    .join("Assets")
                    .join("images");
                let contrast_standard_dir = images_dir.join("contrast-standard");
                let genuine_orig_dir = images_dir.join("contrast-standard.original");

                if contrast_standard_dir.exists() || genuine_orig_dir.exists() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    packages.push(DiscoveredPackage {
                        package_name: name,
                        images_dir,
                        contrast_standard_dir,
                        genuine_orig_dir,
                    });
                }
            }
        }
    }

    packages
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)?.flatten() {
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            let _ = fs::copy(&from, &to);
        }
    }
    Ok(())
}

pub fn ensure_junction_setup(pkg: &DiscoveredPackage) -> Result<PathBuf, String> {
    let local_storage = get_exe_dir()
        .join("contrast-standard")
        .join(&pkg.package_name);

    // 1. Ensure genuine_orig_dir exists with untouched factory SVGs
    if !pkg.genuine_orig_dir.exists() {
        if pkg.contrast_standard_dir.exists() {
            if is_reparse_point(&pkg.contrast_standard_dir) {
                return Err(format!(
                    "contrast-standard is already a junction, but genuine original '{}' is missing.",
                    pkg.genuine_orig_dir.display()
                ));
            } else {
                // contrast_standard_dir is the genuine factory folder
                let _ = crate::security::take_ownership_and_grant_admin(&pkg.images_dir);
                let _ = crate::security::take_ownership_and_grant_admin_recursive(&pkg.contrast_standard_dir);
                fs::rename(&pkg.contrast_standard_dir, &pkg.genuine_orig_dir)
                    .map_err(|e| format!("Failed to rename genuine directory to .original: {}", e))?;
            }
        } else {
            return Err("Neither contrast-standard nor contrast-standard.original found.".to_string());
        }
    }

    // 2. Ensure local_storage exists and has initial copy of files
    let theme_light_local = local_storage.join("theme-light");
    if !local_storage.exists() || !theme_light_local.exists() {
        fs::create_dir_all(&local_storage)
            .map_err(|e| format!("Failed to create local storage directory: {}", e))?;
        let _ = copy_dir_all(&pkg.genuine_orig_dir, &local_storage);
        grant_app_packages_read(&get_exe_dir().join("contrast-standard"));
        grant_app_packages_read(&local_storage);
    }

    // 3. Ensure junction exists at contrast_standard_dir
    if !pkg.contrast_standard_dir.exists() {
        create_junction(&pkg.contrast_standard_dir, &local_storage)
            .map_err(|e| format!("Failed to create junction: {}", e))?;
    } else if !is_reparse_point(&pkg.contrast_standard_dir) {
        let _ = fs::remove_dir_all(&pkg.contrast_standard_dir);
        create_junction(&pkg.contrast_standard_dir, &local_storage)
            .map_err(|e| format!("Failed to create junction: {}", e))?;
    }

    Ok(local_storage)
}

fn get_or_load_templates(pkg: &DiscoveredPackage) -> Result<Vec<SvgTemplate>, String> {
    let mut guard = TEMPLATES.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);

    if let Some(list) = map.get(&pkg.package_name) {
        return Ok(list.clone());
    }

    let src_root = if pkg.genuine_orig_dir.exists() {
        &pkg.genuine_orig_dir
    } else {
        &pkg.contrast_standard_dir
    };

    let mut list = Vec::new();
    for (theme_name, theme_dir_name) in [("light", "theme-light"), ("dark", "theme-dark")] {
        let theme_path = src_root.join(theme_dir_name);
        if let Ok(entries) = fs::read_dir(&theme_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("svg")
                    && let Ok(content) = fs::read_to_string(&path) {
                        let file_name = entry.file_name();
                        let rel_path = PathBuf::from(theme_dir_name).join(file_name);
                        let is_spotlight = content.contains(SPOTLIGHT_DEFAULT_LIGHT);
                        list.push(SvgTemplate {
                            rel_path,
                            content,
                            theme: theme_name,
                            is_spotlight,
                        });
                    }
            }
        }
    }

    if list.is_empty() {
        return Err(format!("No SVG templates found in '{}'", src_root.display()));
    }

    map.insert(pkg.package_name.clone(), list.clone());
    Ok(list)
}

pub fn apply_e11_colorization(
    packages: &[DiscoveredPackage],
    light_rgb: Rgb,
    dark_rgb: Rgb,
) -> Result<usize, String> {
    let target_light_hex = light_rgb.to_hex();
    let target_dark_hex = dark_rgb.to_hex();
    let target_spotlight_hex = blend_colors(SPOTLIGHT_DARK_BASE, &target_light_hex, 82);

    let mut total_modified = 0;

    for pkg in packages {
        let local_storage = ensure_junction_setup(pkg)?;
        let templates = get_or_load_templates(pkg)?;

        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(16);
        let chunk_size = templates.len().div_ceil(num_threads);

        let modified_count = std::sync::atomic::AtomicUsize::new(0);
        let modified_ref = &modified_count;

        std::thread::scope(|s| {
            for chunk in templates.chunks(chunk_size) {
                let light_hex = &target_light_hex;
                let dark_hex = &target_dark_hex;
                let spot_hex = &target_spotlight_hex;
                let base_dir = &local_storage;

                s.spawn(move || {
                    for t in chunk {
                        let patched = if t.theme == "light" {
                            let mut c = replace_color_ignore_case(&t.content, DEFAULT_LIGHT_HEX, light_hex);
                            if t.is_spotlight {
                                c = c.replace(SPOTLIGHT_DEFAULT_LIGHT, spot_hex);
                            }
                            c
                        } else {
                            replace_color_ignore_case(&t.content, DEFAULT_DARK_HEX, dark_hex)
                        };

                        let out_file = base_dir.join(&t.rel_path);
                        if let Ok(()) = fs::write(out_file, patched) {
                            modified_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                });
            }
        });

        total_modified += modified_count.load(std::sync::atomic::Ordering::Relaxed);
    }

    Ok(total_modified)
}

pub fn revert_e11_colorization(packages: &[DiscoveredPackage]) -> Result<usize, String> {
    let mut restored_count = 0;

    for pkg in packages {
        // 1. If contrast_standard_dir is a junction, remove the junction link
        if is_reparse_point(&pkg.contrast_standard_dir) {
            let _ = fs::remove_dir(&pkg.contrast_standard_dir);
        }

        // 2. If genuine_orig_dir exists, move it back to contrast_standard_dir
        if pkg.genuine_orig_dir.exists() && !pkg.contrast_standard_dir.exists() {
            let _ = fs::rename(&pkg.genuine_orig_dir, &pkg.contrast_standard_dir);
        }

        // 3. Restore TrustedInstaller ownership recursively
        if pkg.contrast_standard_dir.exists() {
            crate::security::restore_trusted_installer_recursive(&pkg.contrast_standard_dir);
            let _ = crate::security::restore_trusted_installer(&pkg.images_dir);
            restored_count += 1;
        }
    }

    // Invalidate RAM cache so next apply will re-read fresh originals
    let mut guard = TEMPLATES.lock().unwrap();
    *guard = None;

    Ok(restored_count)
}

fn replace_color_ignore_case(src: &str, old_hex: &str, new_hex: &str) -> String {
    let old_upper = old_hex.to_ascii_uppercase();
    let old_lower = old_hex.to_ascii_lowercase();

    src.replace(&old_upper, new_hex).replace(&old_lower, new_hex)
}
