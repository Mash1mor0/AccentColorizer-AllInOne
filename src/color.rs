use windows_sys::Win32::Graphics::Dwm::DwmGetColorizationColor;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RegCloseKey, RegOpenKeyExW, RegQueryValueExW, KEY_READ};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    pub h: f64, // 0.0 .. 360.0
    pub s: f64, // 0.0 .. 1.0
    pub v: f64, // 0.0 .. 1.0
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_hsv(self) -> Hsv {
        let r = self.r as f64 / 255.0;
        let g = self.g as f64 / 255.0;
        let b = self.b as f64 / 255.0;

        let min = r.min(g).min(b);
        let max = r.max(g).max(b);
        let delta = max - min;

        let v = max;
        if delta < 1e-5 || max <= 0.0 {
            return Hsv { h: 0.0, s: 0.0, v };
        }

        let s = delta / max;
        let mut h = if (r - max).abs() < 1e-5 {
            (g - b) / delta
        } else if (g - max).abs() < 1e-5 {
            2.0 + (b - r) / delta
        } else {
            4.0 + (r - g) / delta
        };

        h *= 60.0;
        if h < 0.0 {
            h += 360.0;
        }

        Hsv { h, s, v }
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
        if hex.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(Self::new(r, g, b))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    pub fn to_colorref(self) -> u32 {
        (self.r as u32) | ((self.g as u32) << 8) | ((self.b as u32) << 16)
    }

    pub fn from_colorref(color: u32) -> Self {
        Self {
            r: (color & 0xFF) as u8,
            g: ((color >> 8) & 0xFF) as u8,
            b: ((color >> 16) & 0xFF) as u8,
        }
    }
}

impl Hsv {
    pub fn to_rgb(self) -> Rgb {
        if self.s <= 0.0 {
            let val = (self.v * 255.0).round().clamp(0.0, 255.0) as u8;
            return Rgb::new(val, val, val);
        }

        let mut hh = self.h;
        if hh >= 360.0 {
            hh = 0.0;
        }
        hh /= 60.0;
        let i = hh.floor() as i64;
        let ff = hh - i as f64;

        let p = self.v * (1.0 - self.s);
        let q = self.v * (1.0 - (self.s * ff));
        let t = self.v * (1.0 - (self.s * (1.0 - ff)));

        let (r, g, b) = match i {
            0 => (self.v, t, p),
            1 => (q, self.v, p),
            2 => (p, self.v, t),
            3 => (p, q, self.v),
            4 => (t, p, self.v),
            _ => (self.v, p, q),
        };

        Rgb::new(
            (r * 255.0).round().clamp(0.0, 255.0) as u8,
            (g * 255.0).round().clamp(0.0, 255.0) as u8,
            (b * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    }
}

pub fn blend_colors(color1_hex: &str, color2_hex: &str, intensity: u32) -> String {
    let c1 = Rgb::from_hex(color1_hex).unwrap_or(Rgb::new(0, 0, 0));
    let c2 = Rgb::from_hex(color2_hex).unwrap_or(Rgb::new(0, 0, 0));

    let intensity = intensity.min(255);
    let inv = 255 - intensity;

    let r = ((c1.r as u32 * intensity + c2.r as u32 * inv) / 255) as u8;
    let g = ((c1.g as u32 * intensity + c2.g as u32 * inv) / 255) as u8;
    let b = ((c1.b as u32 * intensity + c2.b as u32 * inv) / 255) as u8;

    Rgb::new(r, g, b).to_hex()
}

type FnGetImmersiveUserColorSetPreference = unsafe extern "system" fn(bool, bool) -> u32;
type FnGetImmersiveColorTypeFromName = unsafe extern "system" fn(*const u16) -> u32;
type FnGetImmersiveColorFromColorSetEx = unsafe extern "system" fn(u32, u32, bool, u32) -> u32;

pub fn get_immersive_color(name: &str) -> Option<Rgb> {
    unsafe {
        let uxtheme = LoadLibraryW(encode_wide("uxtheme.dll").as_ptr());
        if uxtheme.is_null() {
            return None;
        }

        let p_pref = GetProcAddress(uxtheme, 98 as *const u8)?;
        let p_type = GetProcAddress(uxtheme, 96 as *const u8)?;
        let p_color = GetProcAddress(uxtheme, 95 as *const u8)?;

        let get_user_color_set_pref: FnGetImmersiveUserColorSetPreference =
            std::mem::transmute(p_pref);
        let get_color_type_from_name: FnGetImmersiveColorTypeFromName =
            std::mem::transmute(p_type);
        let get_color_from_color_set_ex: FnGetImmersiveColorFromColorSetEx =
            std::mem::transmute(p_color);

        // Pass true for bForceCheckRegistry so uxtheme re-queries the registry
        // and does NOT return stale in-memory cached colors from the previous theme!
        let color_set = get_user_color_set_pref(true, false);
        let wide_name = encode_wide(name);
        let color_type = get_color_type_from_name(wide_name.as_ptr());
        let raw = get_color_from_color_set_ex(color_set, color_type, false, 0);

        // Raw color is in ABGR / RGBA format: bytes [R, G, B, A] in low-to-high order
        let bytes = raw.to_le_bytes();
        Some(Rgb::new(bytes[0], bytes[1], bytes[2]))
    }
}

pub fn get_dwm_accent_color() -> Option<Rgb> {
    unsafe {
        let mut color: u32 = 0;
        let mut opaque: i32 = 0;
        if DwmGetColorizationColor(&mut color, &mut opaque) == 0 {
            // DWM returns 0xAARRGGBB
            let r = ((color >> 16) & 0xFF) as u8;
            let g = ((color >> 8) & 0xFF) as u8;
            let b = (color & 0xFF) as u8;
            return Some(Rgb::new(r, g, b));
        }
    }
    None
}

pub fn get_registry_accent_color() -> Option<Rgb> {
    unsafe {
        let mut hkey = std::ptr::null_mut();
        let subkey = encode_wide(r"Software\Microsoft\Windows\DWM");
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return None;
        }

        let val_name = encode_wide("AccentColor");
        let mut val_type = 0u32;
        let mut data = 0u32;
        let mut data_len = std::mem::size_of::<u32>() as u32;

        let status = RegQueryValueExW(
            hkey,
            val_name.as_ptr(),
            std::ptr::null(),
            &mut val_type,
            &mut data as *mut u32 as *mut u8,
            &mut data_len,
        );
        RegCloseKey(hkey);

        if status == 0 {
            // AccentColor is ABGR: 0xAABBGGRR
            let r = (data & 0xFF) as u8;
            let g = ((data >> 8) & 0xFF) as u8;
            let b = ((data >> 16) & 0xFF) as u8;
            return Some(Rgb::new(r, g, b));
        }
    }
    None
}

pub fn get_current_system_accent() -> Rgb {
    if let Some(c) = get_immersive_color("ImmersiveSystemAccent") {
        return c;
    }
    if let Some(c) = get_dwm_accent_color() {
        return c;
    }
    if let Some(c) = get_registry_accent_color() {
        return c;
    }
    // Default Windows 10/11 fallback blue: #0078D4
    Rgb::new(0x00, 0x78, 0xD4)
}

pub fn get_current_dark_system_accent() -> Rgb {
    if let Some(c) = get_immersive_color("ImmersiveSystemAccentLight2") {
        return c;
    }
    // Default Windows 11 dark mode fallback cyan: #4CC2FF
    Rgb::new(0x4C, 0xC2, 0xFF)
}

pub fn encode_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
