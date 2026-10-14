//! Native icon generation — no resource compiler, no .ico file needed.
//!
//! Two designs, chosen by the size asked for:
//!
//! * **>= 32 px — detailed eye.** Dark rim, bright cyan iris, a cyan hairline
//!   along the upper lid, and a white highlight in the iris. Reads well at the
//!   sizes Alt+Tab, Task Manager and the About box use.
//! * **< 32 px — disc badge.** A filled cyan disc with a knock-out white eye.
//!   A solid silhouette survives being scaled down to the 16 px the tray
//!   actually draws at, where fine details would smear into mush.
//!
//! Both are rendered into a 32bpp DIB with premultiplied alpha, then wrapped
//! in an HICON. Falls back to the stock application icon if anything fails.
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

const RIM: (f32, f32, f32) = (0.10, 0.11, 0.14); // dark rim: visible on light trays
const CYAN: (f32, f32, f32) = (0.16, 0.72, 1.00); // bright iris: visible on dark trays

/// Antialiased rasteriser. `shader` is sampled 3x3 per output pixel and returns
/// **non-premultiplied** (r, g, b, a); the output is premultiplied BGRA.
fn raster(size: usize, shader: impl Fn(f32, f32) -> (f32, f32, f32, f32)) -> Vec<u8> {
    if size == 0 {
        return Vec::new();
    }
    let mut out = vec![0u8; size * size * 4];
    const SS: usize = 3; // supersampling factor per axis

    for y in 0..size {
        for x in 0..size {
            let mut acc = [0f32; 4]; // r, g, b, a
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32 - 0.5;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32 - 0.5;
                    let (r, g, b, a) = shader(px, py);
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
                out[i] = ((acc[2] / n) * a * 255.0) as u8; // B
                out[i + 1] = ((acc[1] / n) * a * 255.0) as u8; // G
                out[i + 2] = ((acc[0] / n) * a * 255.0) as u8; // R
                out[i + 3] = (a * 255.0) as u8; // A
            }
        }
    }
    out
}

/// Coverage of an ellipse outline, 0..1, with a ~1px soft edge.
fn ring_coverage(px: f32, py: f32, cx: f32, cy: f32, rx: f32, ry: f32, width: f32) -> f32 {
    let dx = (px - cx) / rx;
    let dy = (py - cy) / ry;
    let d = (dx * dx + dy * dy).sqrt(); // 1.0 == on the outline
                                        // one unit of d is roughly ry*1.6 px, so convert before softening
    let dist_px = (d - 1.0).abs() * ry.min(rx) * 1.6;
    (width / 2.0 + 0.5 - dist_px).clamp(0.0, 1.0)
}

/// Coverage of a filled disc, 0..1, with a ~1px soft edge.
fn disc_coverage(px: f32, py: f32, cx: f32, cy: f32, r: f32) -> f32 {
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    (r + 0.5 - d).clamp(0.0, 1.0)
}

/// Render the icon at `size` px. Returns rows top-to-bottom, BGRA, premultiplied.
pub fn render_rgba(size: usize) -> Vec<u8> {
    if size >= 32 {
        render_eye(size)
    } else {
        render_badge(size)
    }
}

/// Detailed eye, for 32 px and up.
fn render_eye(size: usize) -> Vec<u8> {
    let s = size as f32;
    let c = (s - 1.0) / 2.0;
    let rx = s * 0.46; // lens half-width
    let ry = s * 0.30; // lens half-height at centre
    let ring = (s * 0.085).max(1.2);
    let pupil_r = (s * 0.145).max(1.2);

    raster(size, |px, py| {
        let a_rim = ring_coverage(px, py, c, c, rx, ry, ring);
        // hairline: a slightly smaller concentric ellipse, upper half only
        let a_inner = ring_coverage(
            px,
            py,
            c - s * 0.012,
            c - s * 0.012,
            rx * 0.86,
            ry * 0.80,
            ring * 0.34,
        );
        let hairline = if py < c { a_inner * 0.9 } else { 0.0 };
        let a_pupil = disc_coverage(px, py, c, c, pupil_r);
        let a_glint = disc_coverage(
            px,
            py,
            c - pupil_r * 0.34,
            c - pupil_r * 0.36,
            pupil_r * 0.26,
        );

        if a_glint > 0.3 && a_pupil > 0.3 {
            (1.0, 1.0, 1.0, a_pupil)
        } else if a_pupil > a_rim.max(hairline) {
            (CYAN.0, CYAN.1, CYAN.2, a_pupil)
        } else if hairline > a_rim {
            (CYAN.0, CYAN.1, CYAN.2, hairline)
        } else {
            (RIM.0, RIM.1, RIM.2, a_rim)
        }
    })
}

/// Solid disc badge with a knock-out eye, for the small sizes the tray uses.
fn render_badge(size: usize) -> Vec<u8> {
    let s = size as f32;
    let c = (s - 1.0) / 2.0;
    let disc_r = s * 0.48;
    let rx = s * 0.40;
    let ry = s * 0.235;
    let eye_w = (s * 0.055).max(1.0);
    let pupil_r = s * 0.085;

    raster(size, |px, py| {
        let a_disc = disc_coverage(px, py, c, c, disc_r);
        let a_rim = ring_coverage(px, py, c, c, rx, ry, eye_w);
        let a_pupil = disc_coverage(px, py, c, c, pupil_r);
        let a_eye = a_pupil.max(a_rim);

        let alpha = a_disc.max(a_eye);
        // eye punches white through the cyan disc
        let white_mix = if alpha > 0.001 { a_eye / alpha } else { 0.0 };
        (
            CYAN.0 + (1.0 - CYAN.0) * white_mix,
            CYAN.1 + (1.0 - CYAN.1) * white_mix,
            CYAN.2 + (1.0 - CYAN.2) * white_mix,
            alpha,
        )
    })
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
    fn disc_badge_fills_more_than_the_eye() {
        let badge = render_rgba(16);
        let eye = render_rgba(64);
        let filled =
            |px: &[u8]| px.chunks(4).filter(|p| p[3] > 200).count() as f32 / (px.len() / 4) as f32;
        assert!(
            filled(&badge) > 0.5,
            "the badge should read as a solid disc, got {}",
            filled(&badge)
        );
        assert!(
            filled(&eye) < 0.25,
            "the eye should stay outlined rather than filled, got {}",
            filled(&eye)
        );
    }

    #[test]
    fn badge_is_cyan_outside_the_eye() {
        let px = render_rgba(16);
        // (8,2) is inside the disc but clear of the eye, so it stays cyan
        let i = (2 * 16 + 8) * 4;
        assert!(px[i] > 150, "expected a cyan fill, got B={}", px[i]);
    }

    #[test]
    fn preview_written_for_inspection() {
        let px = render_rgba(64);
        let p = std::env::temp_dir().join("peekguard-icon-preview.bmp");
        write_bmp_preview(p.to_str().unwrap(), 64, &px).unwrap();
        assert!(p.exists());
    }
}
