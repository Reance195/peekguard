//! Native icon generation — no resource compiler, no .ico file needed.
//!
//! Draws a small "eye" glyph into a 32bpp DIB with alpha and hands back an HICON.
//! Falls back to the stock application icon if anything fails.
//!
//! The struct fields keep their Win32 names (`biWidth`, `fIcon`, …) on purpose:
//! they mirror the C layout field-for-field, which is the whole point of a
//! hand-written FFI binding. Renaming them to snake_case would make the mapping
//! harder to check against the SDK headers. Same for the type names.
#![allow(non_snake_case)]
#![allow(clippy::upper_case_acronyms)]

use crate::win::*;
use std::ffi::c_void;

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

// ---- drawing ---------------------------------------------------------------

/// Render the icon at `size` px. Returns rows top-to-bottom, BGRA, premultiplied.
pub fn render_rgba(size: usize) -> Vec<u8> {
    let s = size as f32;
    let c = (s - 1.0) / 2.0; // centre
    let rx = s * 0.46; // lens half-width
    let ry = s * 0.30; // lens half-height at centre
    let ring = (s * 0.075).max(1.0);
    let pupil_r = (s * 0.16).max(1.2);

    // layout follows the tray size it produced
    let _ = (rx, ry, ring, pupil_r);

    let mut out = vec![0u8; size * size * 4];
    const SS: usize = 3; // supersampling factor per axis

    for y in 0..size {
        for x in 0..size {
            let mut acc = [0f32; 4]; // r, g, b, a
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32 - 0.5;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32 - 0.5;

                    // distance from the lens outline: |(dx/rx)^2 + (dy/ry)^2 - 1|
                    let dx = (px - c) / rx;
                    let dy = (py - c) / ry;
                    let lens_f = (dx * dx + dy * dy).sqrt(); // 1.0 == on the outline
                    let outline_w = ring / (ry.min(rx) * 1.6);
                    let on_outline = (lens_f - 1.0).abs() <= outline_w;

                    let dist_pupil = ((px - c).powi(2) + (py - c).powi(2)).sqrt();
                    let in_pupil = dist_pupil <= pupil_r;

                    let (r, g, b, a) = if in_pupil {
                        (0.16, 0.72, 1.0, 1.0) // bright cyan iris: visible on dark trays too
                    } else if on_outline {
                        (0.10, 0.11, 0.14, 1.0) // dark rim: visible on light trays
                    } else {
                        (0.0, 0.0, 0.0, 0.0)
                    };
                    acc[0] += r;
                    acc[1] += g;
                    acc[2] += b;
                    acc[3] += a;
                }
            }
            let n = (SS * SS) as f32;
            let a = acc[3] / n;
            let i = (y * size + x) * 4;
            if a > 0.001 {
                // premultiply
                let r = (acc[0] / n) / a * a;
                let g = (acc[1] / n) / a * a;
                let b = (acc[2] / n) / a * a;
                out[i] = (b * 255.0) as u8; // B
                out[i + 1] = (g * 255.0) as u8; // G
                out[i + 2] = (r * 255.0) as u8; // R
                out[i + 3] = (a * 255.0) as u8; // A
            }
        }
    }
    out
}

/// Dump a 32bpp BMP so the icon can be eyeballed during development.
#[allow(dead_code)]
pub fn write_bmp_preview(path: &str, size: usize, bgra: &[u8]) -> std::io::Result<()> {
    let row = size * 4;
    let data_size = row * size;
    let file_size = 14 + 40 + data_size;
    let mut f = Vec::with_capacity(file_size);
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&(file_size as u32).to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&(14u32 + 40).to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&(size as i32).to_le_bytes());
    f.extend_from_slice(&(size as i32).to_le_bytes()); // positive height = bottom-up
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&(data_size as u32).to_le_bytes());
    f.extend_from_slice(&2835i32.to_le_bytes());
    f.extend_from_slice(&2835i32.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..size).rev() {
        f.extend_from_slice(&bgra[y * row..y * row + row]);
    }
    std::fs::write(path, f)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_something_visible() {
        let px = render_rgba(16);
        assert_eq!(px.len(), 16 * 16 * 4);
        let opaque = px.chunks(4).filter(|p| p[3] > 200).count();
        assert!(opaque > 10, "icon should have opaque pixels, got {opaque}");
        let transparent = px.chunks(4).filter(|p| p[3] < 10).count();
        assert!(
            transparent > 10,
            "corners should be transparent, got {transparent}"
        );
    }

    #[test]
    fn preview_written_for_inspection() {
        let px = render_rgba(64);
        let p = std::env::temp_dir().join("peekguard-icon-preview.bmp");
        write_bmp_preview(p.to_str().unwrap(), 64, &px).unwrap();
        assert!(p.exists());
    }
}
