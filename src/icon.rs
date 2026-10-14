//! Runtime `HICON` builder — turns the shared rasteriser into an icon handle.
//!
//! The pixels come from [`icon_raster`](icon_raster), which `build.rs` also
//! uses to embed a file icon into the exe. The same `render_rgba` output is
//! wrapped here into an `HICON` (via a DIB + `CreateIconIndirect`) for the
//! tray, the message window's class and the About box. Falls back to the stock
//! application icon if anything fails.
//!
//! The struct fields keep their Win32 names (`biWidth`, `fIcon`, …) on purpose:
//! they mirror the C layout field-for-field, which is the whole point of a
//! hand-written FFI binding.

#![allow(non_snake_case)]
#![allow(clippy::upper_case_acronyms)]

use crate::win::*;
use std::ffi::c_void;

// The pure renderer is shared with build.rs; including it by path keeps a
// single copy of the drawing code without a library target.
#[path = "icon_raster.rs"]
mod icon_raster;
use icon_raster::render_rgba;

// ---- GDI bits we need ------------------------------------------------------

#[repr(C)]
struct BITMAPINFOHEADER {
    biSize: u32,
    biWidth: i32,
    biHeight: i32,
    biPlanes: u16,
    biBitCount: u16,
    biCompression: u32,
    biSizeImage: u32,
    biXPelsPerMeter: i32,
    biYPelsPerMeter: i32,
    biClrUsed: u32,
    biClrImportant: u32,
}

#[repr(C)]
struct BITMAPINFO {
    bmiHeader: BITMAPINFOHEADER,
    bmiColors: [u32; 1],
}

#[repr(C)]
struct ICONINFO {
    fIcon: BOOL,
    xHotspot: u32,
    yHotspot: u32,
    hbmMask: *mut c_void,
    hbmColor: *mut c_void,
}

const DIB_RGB_COLORS: u32 = 0;
const BI_RGB: u32 = 0;

#[link(name = "user32")]
unsafe extern "system" {
    fn CreateIconIndirect(info: *const ICONINFO) -> HICON;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateDIBSection(
        hdc: *mut c_void,
        info: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut c_void,
        section: *mut c_void,
        offset: u32,
    ) -> *mut c_void;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> *mut c_void;
    fn DeleteObject(o: *mut c_void) -> BOOL;
}

/// Build an HICON from the generated pixels.
pub fn create_icon(size: usize) -> Option<HICON> {
    if size == 0 || size > 256 {
        return None;
    }
    let pixels = render_rgba(size);

    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: size as i32, // top-down so our row order matches
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: (size * size * 4) as u32,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [0],
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = CreateDIBSection(
            std::ptr::null_mut(),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if color.is_null() || bits.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());

        // AND mask: 1bpp, bit set == transparent.
        let mask_row_bytes = size.div_ceil(16) * 2; // 16-bit aligned rows
        let mut mask = vec![0u8; mask_row_bytes * size];
        for y in 0..size {
            for x in 0..size {
                let a = pixels[(y * size + x) * 4 + 3];
                if a < 128 {
                    let bit = 7 - (x % 8);
                    mask[y * mask_row_bytes + x / 8] |= 1 << bit;
                }
            }
        }
        let hbm_mask = CreateBitmap(
            size as i32,
            size as i32,
            1,
            1,
            mask.as_ptr() as *const c_void,
        );
        if hbm_mask.is_null() {
            DeleteObject(color);
            return None;
        }

        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);

        DeleteObject(hbm_mask);
        DeleteObject(color);

        if icon.is_null() {
            None
        } else {
            Some(icon)
        }
    }
}

/// Stock icon, used when generation fails.
pub fn stock_icon() -> HICON {
    unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION as usize as *const u16) }
}
