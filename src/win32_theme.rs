use windows_sys::Win32::Graphics::Gdi::{
    GetBitmapBits, GetObjectW, GetSysColor, SetBitmapBits, SetSysColors, BITMAP, COLOR_ACTIVECAPTION,
    COLOR_GRADIENTACTIVECAPTION, COLOR_HIGHLIGHT, COLOR_HOTLIGHT, COLOR_MENUHILIGHT, HBITMAP,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::Controls::{CloseThemeData, GetThemeBitmap, OpenThemeData, HTHEME};

use crate::color::{encode_wide, Rgb};
use crate::system::WindowsVersion;

const GBF_DIRECT: u32 = 1;
const TMT_DIBDATA: i32 = 2;

// Visual style control constants
const VSCLASS_BUTTON: &str = "Button";
const BP_PUSHBUTTON: i32 = 1;
const BP_RADIOBUTTON: i32 = 2;
const BP_CHECKBOX: i32 = 3;
const BP_GROUPBOX: i32 = 4;

const VSCLASS_COMBOBOX: &str = "ComboBox";
const CP_DROPDOWNBUTTON: i32 = 1;
const CP_DROPDOWNBUTTONLEFT: i32 = 3;

const VSCLASS_EDIT: &str = "Edit";
const EP_EDITBORDER_NOSCROLL: i32 = 6;
const EP_EDITBORDER_HVSCROLL: i32 = 7;

const VSCLASS_TAB: &str = "Tab";
const TABP_TABITEM: i32 = 1;
const TABP_TOPTABITEMBOTHEDGE: i32 = 10;

const VSCLASS_TRACKBAR: &str = "Trackbar";
const TKP_THUMB: i32 = 3;
const TKP_THUMBRIGHT: i32 = 5;

const VSCLASS_LISTBOX: &str = "ListBox";
const LBCP_BORDER_HSCROLL: i32 = 1;
const LBCP_ITEM: i32 = 5;

const VSCLASS_SPIN: &str = "Spin";
const SPNP_UP: i32 = 1;
const SPNP_DOWNHORZ: i32 = 4;


const TP_BUTTON: i32 = 1;
const TP_SPLITBUTTONDROPDOWN: i32 = 6;

const VSCLASS_MONTHCAL: &str = "MonthCal";
const MC_GRIDCELLBACKGROUND: i32 = 1;

const SYS_ELEMENTS: [u32; 5] = [
    COLOR_ACTIVECAPTION as u32,
    COLOR_GRADIENTACTIVECAPTION as u32,
    COLOR_HIGHLIGHT as u32,
    COLOR_HOTLIGHT as u32,
    COLOR_MENUHILIGHT as u32,
];

pub fn modify_sys_colors(accent_h: f64) {
    let mut new_colors = [0u32; SYS_ELEMENTS.len()];

    for (i, &el) in SYS_ELEMENTS.iter().enumerate() {
        let current = unsafe { GetSysColor(el as i32) };
        let rgb = Rgb::from_colorref(current);
        let mut hsv = rgb.to_hsv();
        hsv.h = accent_h;
        let recolored = hsv.to_rgb();
        new_colors[i] = recolored.to_colorref();
    }

    unsafe {
        SetSysColors(
            SYS_ELEMENTS.len() as i32,
            SYS_ELEMENTS.as_ptr() as *const i32,
            new_colors.as_ptr(),
        );
    }
}

pub fn restore_sys_colors() {
    let default_colors: [u32; 5] = [
        Rgb::new(153, 180, 209).to_colorref(), // COLOR_ACTIVECAPTION
        Rgb::new(185, 209, 234).to_colorref(), // COLOR_GRADIENTACTIVECAPTION
        Rgb::new(0, 120, 215).to_colorref(),   // COLOR_HIGHLIGHT (Windows Default Blue #0078D7)
        Rgb::new(0, 102, 204).to_colorref(),   // COLOR_HOTLIGHT
        Rgb::new(0, 120, 215).to_colorref(),   // COLOR_MENUHILIGHT
    ];

    unsafe {
        SetSysColors(
            SYS_ELEMENTS.len() as i32,
            SYS_ELEMENTS.as_ptr() as *const i32,
            default_colors.as_ptr(),
        );
    }

    // Reset uxtheme controls (buttons, progress bars, menus) back to default Windows blue hue (207.0 deg)
    let win_ver = crate::system::detect_windows_version();
    modify_styles(207.0, win_ver, true, true);
}

fn set_preferred_app_mode(mode: i32) {
    unsafe {
        let uxtheme = LoadLibraryW(encode_wide("uxtheme.dll").as_ptr());
        if !uxtheme.is_null()
            && let Some(proc) = GetProcAddress(uxtheme, 135 as *const u8) {
                let func: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(proc);
                func(mode);
            }
    }
}

fn init_dark_mode() {
    set_preferred_app_mode(1); // AllowDark
}

struct ThemeSession {
    theme: HTHEME,
}

impl ThemeSession {
    fn new(class_name: &str) -> Option<Self> {
        let wide = encode_wide(class_name);
        let theme = unsafe { OpenThemeData(std::ptr::null_mut(), wide.as_ptr()) };
        if theme == 0 {
            None
        } else {
            Some(Self { theme })
        }
    }

    fn modify_style(&self, part_id: i32, state_id: i32, prop_id: i32, accent_h: f64) -> bool {
        self.modify_style_ext(part_id, state_id, prop_id, accent_h, false)
    }

    fn modify_style_ext(
        &self,
        part_id: i32,
        state_id: i32,
        prop_id: i32,
        accent_h: f64,
        tint_gray_fill: bool,
    ) -> bool {
        let mut hbm: HBITMAP = std::ptr::null_mut();
        let hr = unsafe {
            GetThemeBitmap(
                self.theme,
                part_id,
                state_id,
                prop_id,
                GBF_DIRECT,
                &mut hbm,
            )
        };
        if hr != 0 || hbm.is_null() {
            return false;
        }
        iterate_bitmap(hbm, accent_h, tint_gray_fill)
    }

    fn modify_style_light_pargb(
        &self,
        part_id: i32,
        state_id: i32,
        prop_id: i32,
        base_accent: Rgb,
    ) -> bool {
        let mut hbm: HBITMAP = std::ptr::null_mut();
        let hr = unsafe {
            GetThemeBitmap(
                self.theme,
                part_id,
                state_id,
                prop_id,
                GBF_DIRECT,
                &mut hbm,
            )
        };
        if hr != 0 || hbm.is_null() {
            return false;
        }
        iterate_light_pargb_bitmap(hbm, base_accent)
    }
}

impl Drop for ThemeSession {
    fn drop(&mut self) {
        if self.theme != 0 {
            unsafe {
                CloseThemeData(self.theme);
            }
            self.theme = 0;
        }
    }
}

fn iterate_bitmap(hbm: HBITMAP, accent_h: f64, tint_gray_fill: bool) -> bool {
    if hbm.is_null() {
        return false;
    }

    unsafe {
        let mut bm: BITMAP = std::mem::zeroed();
        let ret = GetObjectW(
            hbm as *mut _,
            std::mem::size_of::<BITMAP>() as i32,
            &mut bm as *mut _ as *mut _,
        );
        if ret == 0 || bm.bmBitsPixel != 32 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
            return false;
        }

        let size = (bm.bmWidth as usize) * (bm.bmHeight as usize) * 4;
        let mut bits = vec![0u8; size];
        if GetBitmapBits(hbm, size as i32, bits.as_mut_ptr() as *mut _) == 0 {
            return false;
        }

        let base_accent_rgb = crate::color::Hsv {
            h: accent_h,
            s: 1.0,
            v: 0.85,
        }
        .to_rgb();

        for chunk in bits.as_chunks_mut::<4>().0 {
            let b = chunk[0];
            let g = chunk[1];
            let r = chunk[2];
            let a = chunk[3];

            if a == 0 {
                continue;
            }

            let rgb = Rgb::new(r, g, b);
            let mut hsv = rgb.to_hsv();

            if hsv.s > 0.05 {
                // Colored pixel (e.g. blue borders, colored checkmarks/arrows) -> shift hue to accent
                hsv.h = accent_h;
                let new_rgb = hsv.to_rgb();
                // Ensure premultiplied alpha: RGB cannot exceed Alpha in 32-bit PARGB bitmaps
                chunk[0] = new_rgb.b.min(a);
                chunk[1] = new_rgb.g.min(a);
                chunk[2] = new_rgb.r.min(a);
            } else if tint_gray_fill {
                if hsv.v > 0.15 {
                    // Dark Mode neutral gray / white in selection or focus outline -> apply accent!
                    hsv.h = accent_h;
                    if hsv.v >= 0.40 {
                        // Border / focus rectangle outline: vibrant accent!
                        hsv.s = 0.75;
                    } else {
                        // Interior fill: subtle tint
                        hsv.s = 0.35;
                    }
                    let new_rgb = hsv.to_rgb();
                    chunk[0] = new_rgb.b.min(a);
                    chunk[1] = new_rgb.g.min(a);
                    chunk[2] = new_rgb.r.min(a);
                } else {
                    // Light Mode translucent black/dark pixels (unfocused selection, focus rectangle):
                    // Premultiply base accent RGB by pixel alpha!
                    chunk[0] = ((base_accent_rgb.b as u32 * a as u32) / 255) as u8;
                    chunk[1] = ((base_accent_rgb.g as u32 * a as u32) / 255) as u8;
                    chunk[2] = ((base_accent_rgb.r as u32 * a as u32) / 255) as u8;
                }
            }
        }

        SetBitmapBits(hbm, size as u32, bits.as_ptr() as *const _);
        true
    }
}

fn set_class_dividers(cls: &str, divider_val: u8) {
    if let Some(t) = ThemeSession::new(cls) {
        for s in 0..=16 {
            let mut hbm: HBITMAP = std::ptr::null_mut();
            let hr = unsafe {
                GetThemeBitmap(t.theme, 1, s, TMT_DIBDATA, GBF_DIRECT, &mut hbm)
            };
            if hr == 0 && !hbm.is_null() {
                unsafe {
                    let mut bm: BITMAP = std::mem::zeroed();
                    if GetObjectW(
                        hbm as *mut _,
                        std::mem::size_of::<BITMAP>() as i32,
                        &mut bm as *mut _ as *mut _,
                    ) != 0
                        && bm.bmBitsPixel == 32
                        && bm.bmWidth > 0
                        && bm.bmHeight > 0
                    {
                        let size = (bm.bmWidth as usize) * (bm.bmHeight as usize) * 4;
                        let mut bits = vec![0u8; size];
                        if GetBitmapBits(hbm, size as i32, bits.as_mut_ptr() as *mut _) != 0 {
                            let divider_x = (bm.bmWidth - 1) as usize;
                            for y in 0..(bm.bmHeight as usize) {
                                let idx = (y * (bm.bmWidth as usize) + divider_x) * 4;
                                bits[idx] = divider_val;
                                bits[idx + 1] = divider_val;
                                bits[idx + 2] = divider_val;
                                bits[idx + 3] = 0xFF;
                            }
                            SetBitmapBits(hbm, size as u32, bits.as_ptr() as *const _);
                        }
                    }
                }
            }
        }
    }
}

fn restore_header_dividers() {
    // 1. Force Light mode to access and restore Light theme headers to default #E5E5E5
    set_preferred_app_mode(3); // ForceLight
    set_class_dividers("ItemsView::Header", 0xE5);
    set_class_dividers("Header", 0xE5);

    // 2. Force Dark mode to access and restore Dark theme headers to default #636363
    set_preferred_app_mode(2); // ForceDark
    set_class_dividers("DarkMode_ItemsView::Header", 0x63);
    set_class_dividers("DarkMode_Header", 0x63);
    set_class_dividers("ItemsView::Header", 0x63);
    set_class_dividers("Header", 0x63);

    // 3. Return to AllowDark (1) for standard dark mode operation
    set_preferred_app_mode(1); // AllowDark
}

fn iterate_light_pargb_bitmap(hbm: HBITMAP, base_accent: Rgb) -> bool {
    if hbm.is_null() {
        return false;
    }

    unsafe {
        let mut bm: BITMAP = std::mem::zeroed();
        let ret = GetObjectW(
            hbm as *mut _,
            std::mem::size_of::<BITMAP>() as i32,
            &mut bm as *mut _ as *mut _,
        );
        if ret == 0 || bm.bmBitsPixel != 32 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
            return false;
        }

        let size = (bm.bmWidth as usize) * (bm.bmHeight as usize) * 4;
        let mut bits = vec![0u8; size];
        if GetBitmapBits(hbm, size as i32, bits.as_mut_ptr() as *mut _) == 0 {
            return false;
        }

        for chunk in bits.as_chunks_mut::<4>().0 {
            let a = chunk[3];
            if a > 0 {
                chunk[0] = ((base_accent.b as u32 * a as u32) / 255) as u8;
                chunk[1] = ((base_accent.g as u32 * a as u32) / 255) as u8;
                chunk[2] = ((base_accent.r as u32 * a as u32) / 255) as u8;
            }
        }

        SetBitmapBits(hbm, size as u32, bits.as_ptr() as *const _);
        true
    }
}

fn recolor_light_explorer_classes(base_accent: Rgb) {
    if let Some(t) = ThemeSession::new("Explorer::TreeView") {
        for i in 1..=6 {
            for j in 1..=16 {
                t.modify_style_light_pargb(i, j, TMT_DIBDATA, base_accent);
            }
        }
    }

    if let Some(t) = ThemeSession::new("Explorer::ListView") {
        for i in [1, 6] {
            for j in 1..=16 {
                t.modify_style_light_pargb(i, j, TMT_DIBDATA, base_accent);
            }
        }
    }

    if let Some(t) = ThemeSession::new("ItemsView") {
        for i in [1, 3, 6] {
            for j in 0..=16 {
                t.modify_style_light_pargb(i, j, TMT_DIBDATA, base_accent);
            }
        }
    }

    if let Some(t) = ThemeSession::new("ItemsView::ListView") {
        for i in [1, 6] {
            for j in 1..=16 {
                t.modify_style_light_pargb(i, j, TMT_DIBDATA, base_accent);
            }
        }
    }
}

fn recolor_explorer_classes(accent_h: f64) {
    if let Some(t) = ThemeSession::new("Explorer::TreeView") {
        for i in 1..=6 {
            for j in 1..=16 {
                let tint_fill = i == 1 || i == 6;
                t.modify_style_ext(i, j, TMT_DIBDATA, accent_h, tint_fill);
            }
        }
        for j in 1..=7 {
            for k in 1..=7 {
                t.modify_style(4, j, k, accent_h);
            }
        }
    }

    if let Some(t) = ThemeSession::new("Explorer::ListView") {
        for i in 1..=10 {
            for j in 1..=16 {
                let tint_fill = i == 1 || i == 6;
                t.modify_style_ext(i, j, TMT_DIBDATA, accent_h, tint_fill);
            }
        }
        for i in 8..=9 {
            for j in 1..=7 {
                for k in 1..=7 {
                    t.modify_style(i, j, k, accent_h);
                }
            }
        }
    }

    if let Some(t) = ThemeSession::new("ItemsView") {
        for i in 1..=10 {
            for j in 0..=16 {
                t.modify_style_ext(i, j, TMT_DIBDATA, accent_h, true);
            }
        }
    }

    if let Some(t) = ThemeSession::new("ItemsView::ListView") {
        for i in 1..=16 {
            for j in 1..=16 {
                let tint_fill = i == 1 || i == 6;
                t.modify_style_ext(i, j, TMT_DIBDATA, accent_h, tint_fill);
            }
        }
        for i in 8..=9 {
            for j in 1..=7 {
                for k in 1..=7 {
                    t.modify_style(i, j, k, accent_h);
                }
            }
        }
    }

    if let Some(t) = ThemeSession::new("ListView") {
        for i in 8..=9 {
            for j in 1..=7 {
                for k in 1..=7 {
                    t.modify_style(i, j, k, accent_h);
                }
            }
        }
    }
}

pub fn modify_styles(
    accent_h: f64,
    win_ver: WindowsVersion,
    colorize_menus: bool,
    colorize_progress_bar: bool,
) {
    init_dark_mode();
    restore_header_dividers();

    if let Some(t) = ThemeSession::new(VSCLASS_BUTTON) {
        t.modify_style(BP_PUSHBUTTON, 0, TMT_DIBDATA, accent_h);
        for j in 1..=7 {
            t.modify_style(BP_CHECKBOX, 0, j, accent_h);
            t.modify_style(BP_RADIOBUTTON, 0, j, accent_h);
        }
        for j in 1..=3 {
            t.modify_style(BP_GROUPBOX, 0, j, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new(VSCLASS_COMBOBOX) {
        for i in CP_DROPDOWNBUTTON..=CP_DROPDOWNBUTTONLEFT {
            t.modify_style(i, 0, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new(VSCLASS_EDIT) {
        for i in EP_EDITBORDER_NOSCROLL..=EP_EDITBORDER_HVSCROLL {
            t.modify_style(i, 0, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new(VSCLASS_TAB) {
        for i in TABP_TABITEM..=TABP_TOPTABITEMBOTHEDGE {
            t.modify_style(i, 0, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new(VSCLASS_TRACKBAR) {
        for i in TKP_THUMB..=TKP_THUMBRIGHT {
            for j in 1..=7 {
                t.modify_style(i, 0, j, accent_h);
            }
        }
    }

    if let Some(t) = ThemeSession::new(VSCLASS_LISTBOX) {
        for i in LBCP_BORDER_HSCROLL..=LBCP_ITEM {
            t.modify_style(i, 0, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new(VSCLASS_SPIN) {
        for i in SPNP_UP..=SPNP_DOWNHORZ {
            t.modify_style(i, 0, TMT_DIBDATA, accent_h);
        }
    }


    // Toolbars
    let toolbars = [
        "Toolbar",
        "ExplorerMenu::Toolbar",
        "Alternate::Toolbar",
        "Communications::Toolbar",
        "InfoPaneToolbar::Toolbar",
    ];
    for tb in toolbars {
        if let Some(t) = ThemeSession::new(tb) {
            for i in TP_BUTTON..=TP_SPLITBUTTONDROPDOWN {
                t.modify_style(i, 1, TMT_DIBDATA, accent_h);
            }
        }
    }

    if let Some(t) = ThemeSession::new("BreadcrumbBar") {
        t.modify_style(1, 0, TMT_DIBDATA, accent_h);
    }

    if let Some(t) = ThemeSession::new("PreviewPane") {
        t.modify_style(1, 1, TMT_DIBDATA, accent_h);
    }

    if let Some(t) = ThemeSession::new("CommandModule") {
        for i in 1..=11 {
            if i != 8 {
                t.modify_style(i, 0, TMT_DIBDATA, accent_h);
                t.modify_style(i, 1, TMT_DIBDATA, accent_h);
            }
        }
    }

    let base_accent = crate::color::Hsv {
        h: accent_h,
        s: 1.0,
        v: 0.83,
    }
    .to_rgb();

    // 1. Recolor Explorer selection & focus controls under ForceLight (Light Mode)
    set_preferred_app_mode(3); // ForceLight
    recolor_light_explorer_classes(base_accent);

    // 2. Recolor Explorer selection & focus controls under ForceDark (Dark Mode)
    set_preferred_app_mode(2); // ForceDark
    recolor_explorer_classes(accent_h);

    let dark_classes = [
        "DarkMode_Explorer::ListView",
        "DarkMode_ItemsView::ListView",
        "DarkMode::ListView",
        "DarkMode_Explorer::TreeView",
        "DarkMode::TreeView",
    ];

    for cls in dark_classes {
        if let Some(t) = ThemeSession::new(cls) {
            for i in 1..=16 {
                for j in 1..=16 {
                    let tint_fill = i == 1 || i == 6;
                    t.modify_style_ext(i, j, TMT_DIBDATA, accent_h, tint_fill);
                }
            }
            for i in 8..=9 {
                for j in 1..=7 {
                    for k in 1..=7 {
                        t.modify_style(i, j, k, accent_h);
                    }
                }
            }
        }
    }

    set_preferred_app_mode(1); // AllowDark

    if let Some(t) = ThemeSession::new("BB::Toolbar") {
        for i in 1..=4 {
            t.modify_style(i, 0, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new("Go::Toolbar") {
        t.modify_style(1, 1, TMT_DIBDATA, accent_h);
    }
    if let Some(t) = ThemeSession::new("InactiveGo::Toolbar") {
        t.modify_style(1, 1, TMT_DIBDATA, accent_h);
    }
    if let Some(t) = ThemeSession::new("MaxGo::Toolbar") {
        t.modify_style(1, 1, TMT_DIBDATA, accent_h);
    }
    if let Some(t) = ThemeSession::new("LVPopup::Toolbar") {
        t.modify_style(1, 1, TMT_DIBDATA, accent_h);
    }
    if let Some(t) = ThemeSession::new("LVPopupBottom::Toolbar") {
        t.modify_style(1, 1, TMT_DIBDATA, accent_h);
    }

    if let Some(t) = ThemeSession::new("InactiveBB::Toolbar") {
        t.modify_style(3, 1, TMT_DIBDATA, accent_h);
        for j in 1..=6 {
            t.modify_style(4, j, j, accent_h);
            t.modify_style(4, j, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new("InactiveBBComposited::Toolbar") {
        for j in 1..=6 {
            t.modify_style(4, j, j, accent_h);
            t.modify_style(4, j, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new("DragDrop") {
        t.modify_style(7, 0, TMT_DIBDATA, accent_h);
        t.modify_style(8, 0, TMT_DIBDATA, accent_h);
    }


    if let Some(t) = ThemeSession::new("PAUSE") {
        for j in 1..=7 {
            t.modify_style(1, 0, j, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new("Tab") {
        for i in 1..=8 {
            t.modify_style(i, 1, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new(VSCLASS_MONTHCAL) {
        for i in 1..=4 {
            t.modify_style(i, 0, TMT_DIBDATA, accent_h);
            t.modify_style(i, 1, TMT_DIBDATA, accent_h);
        }
        for j in 1..=6 {
            t.modify_style(MC_GRIDCELLBACKGROUND, j, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new("DatePicker") {
        for i in 1..=2 {
            for j in 0..=1 {
                t.modify_style(i, j, 1, accent_h);
                t.modify_style(i, j, TMT_DIBDATA, accent_h);
            }
        }
    }

    if let Some(t) = ThemeSession::new("Rebar") {
        t.modify_style(6, 0, TMT_DIBDATA, accent_h);
        for i in 4..=6 {
            t.modify_style(i, 1, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new("Navbar::Rebar") {
        t.modify_style(6, 0, TMT_DIBDATA, accent_h);
        t.modify_style(6, 1, TMT_DIBDATA, accent_h);
    }

    if let Some(t) = ThemeSession::new("InactiveNavbar::Rebar") {
        t.modify_style(6, 0, TMT_DIBDATA, accent_h);
        t.modify_style(6, 1, TMT_DIBDATA, accent_h);
    }

    if let Some(t) = ThemeSession::new("Desktop::ListView") {
        for j in 0..=9 {
            t.modify_style(1, j, TMT_DIBDATA, accent_h);
        }
    }

    if let Some(t) = ThemeSession::new("TreeView") {
        for i in 0..=10 {
            for j in 0..=10 {
                for k in 0..=10 {
                    t.modify_style(i, j, k, accent_h);
                }
            }
        }
    }

    if let Some(t) = ThemeSession::new("Navigation") {
        for i in 0..=10 {
            for j in 0..=10 {
                for k in 0..=10 {
                    t.modify_style(i, j, k, accent_h);
                }
            }
        }
    }

    if let Some(t) = ThemeSession::new("CopyClose") {
        for j in 1..=7 {
            for k in 1..=7 {
                t.modify_style(1, j, k, accent_h);
            }
        }
    }

    if let Some(t) = ThemeSession::new("SearchBox") {
        for j in 1..=7 {
            for k in 1..=7 {
                t.modify_style(2, j, k, accent_h);
            }
        }
    }

    if let Some(t) = ThemeSession::new("SearchBoxComposited::SearchBox") {
        for j in 1..=7 {
            for k in 1..=7 {
                t.modify_style(2, j, k, accent_h);
            }
        }
    }

    if let Some(t) = ThemeSession::new("SearchButton::Toolbar") {
        for j in 1..=7 {
            for k in 1..=7 {
                t.modify_style(3, j, k, accent_h);
                t.modify_style(4, j, k, accent_h);
            }
        }
    }

    if let Some(t) = ThemeSession::new("TaskbandExtendedUI") {
        let (start, end) = if win_ver >= WindowsVersion::Win10 {
            (6, 9)
        } else {
            (8, 11)
        };
        for i in start..=end {
            t.modify_style(i, 0, TMT_DIBDATA, accent_h);
            t.modify_style(i, 1, TMT_DIBDATA, accent_h);
        }
    }

    if colorize_menus
        && let Some(t) = ThemeSession::new("Menu") {
            t.modify_style(14, 0, TMT_DIBDATA, accent_h);
            t.modify_style(13, 0, TMT_DIBDATA, accent_h);
            t.modify_style(12, 0, TMT_DIBDATA, accent_h);
            t.modify_style(8, 0, TMT_DIBDATA, accent_h);
            t.modify_style(7, 0, TMT_DIBDATA, accent_h);
            for j in 0..=7 {
                for k in 0..=7 {
                    t.modify_style(11, j, k, accent_h);
                }
            }
        }

    if colorize_progress_bar {
        if let Some(t) = ThemeSession::new("Progress") {
            t.modify_style(5, 4, TMT_DIBDATA, accent_h);
            for i in 3..=10 {
                t.modify_style(i, 1, TMT_DIBDATA, accent_h);
            }
        }
        if let Some(t) = ThemeSession::new("Indeterminate::Progress") {
            for i in 3..=10 {
                t.modify_style(i, 1, TMT_DIBDATA, accent_h);
            }
        }
        if let Some(t) = ThemeSession::new("AB::AddressBand") {
            t.modify_style(1, 1, TMT_DIBDATA, accent_h);
        }
    }

    // Legacy components
    if let Some(t) = ThemeSession::new("Communications::Rebar") {
        t.modify_style(6, 0, TMT_DIBDATA, accent_h);
    }
    if let Some(t) = ThemeSession::new("StartPanel") {
        t.modify_style(1, 1, TMT_DIBDATA, accent_h);
    }
    if let Some(t) = ThemeSession::new("StartMenu::Toolbar") {
        t.modify_style(10, 1, TMT_DIBDATA, accent_h);
        t.modify_style(12, 1, TMT_DIBDATA, accent_h);
    }
    if let Some(t) = ThemeSession::new("StartPanelPriv") {
        for i in 1..=38 {
            t.modify_style(i, 0, TMT_DIBDATA, accent_h);
        }
    }
    if let Some(t) = ThemeSession::new("StartPanelComposited::StartPanelPriv") {
        for i in 1..=38 {
            t.modify_style(i, 0, TMT_DIBDATA, accent_h);
        }
    }

    if win_ver < WindowsVersion::Win8 {
        if let Some(t) = ThemeSession::new("TaskDialog") {
            for i in 1..=8 {
                t.modify_style(13, i, TMT_DIBDATA, accent_h);
            }
        }
        if let Some(t) = ThemeSession::new("ScrollBar") {
            for i in 1..=10 {
                for k in 1..=4 {
                    t.modify_style(i, 0, k, accent_h);
                }
            }
        }
        if let Some(t) = ThemeSession::new("AltTab") {
            for i in 1..=11 {
                t.modify_style(i, 0, TMT_DIBDATA, accent_h);
            }
        }
    }

    restore_header_dividers();
}
