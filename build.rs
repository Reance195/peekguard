//! Embed the application icon into the exe at build time.
//!
//! Windows reads an executable's file icon (Explorer, the taskbar, shortcuts)
//! from an `RT_GROUP_ICON` / `RT_ICON` resource baked into the PE file — it
//! cannot be supplied at runtime, which is why the tray icon (generated live in
//! `icon.rs`) never showed up on the desktop shortcut.
//!
//! Instead of pulling in a resource compiler or a crates.io helper, this script
//! reuses the very same drawing code as the runtime icon
//! ([`icon_raster`](src/icon_raster.rs)) and hand-writes the two binary formats
//! involved:
//!
//! * each size as an **icon image** (`BITMAPINFOHEADER` + 32bpp XOR + AND mask),
//! * the whole ladder as a **group icon** directory,
//! * and both wrapped in a `.res` container that the MSVC linker embeds when the
//!   file is passed as an extra link input.
//!
//! The `.res` layout mirrors what `rc.exe` emits (captured as a reference during
//! development), so the result is byte-for-byte a normal resource section.

use std::io::Write;
use std::path::PathBuf;

#[path = "src/icon_raster.rs"]
mod icon_raster;

/// Sizes baked into the exe. The desktop shortcut and Explorer pick whichever
/// fits; 128 covers the large-icon views well without the ~270 KB a 256 px
/// entry would add.
const SIZES: &[usize] = &[16, 24, 32, 48, 64, 128];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon_raster.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let res_path = out_dir.join("peekguard.res");

    let mut images = Vec::new();
    for (i, &size) in SIZES.iter().enumerate() {
        images.push((i as u16 + 1, size, icon_image(size)));
    }

    let mut res = Vec::new();
    // rc.exe always opens the file with a 32-byte null marker record.
    push_header(&mut res, 0, 0, 0, 0);
    // RT_ICON (3) for each size, then RT_GROUP_ICON (14) tying them together.
    for (id, _size, image) in &images {
        push_header(&mut res, image.len() as u32, 3, *id as u32, 0x1010);
        push_padded(&mut res, image);
    }
    let group = group_icon(&images);
    push_header(&mut res, group.len() as u32, 14, 1, 0x1030);
    push_padded(&mut res, &group);

    let mut f = std::fs::File::create(&res_path).expect("create .res");
    f.write_all(&res).expect("write .res");
    drop(f);

    // Hand the .res to the linker as an extra input; MSVC folds it into .rsrc.
    println!("cargo:rustc-link-arg={}", res_path.display());
}

/// One icon image: `BITMAPINFOHEADER` + bottom-up BGRA XOR + bottom-up AND mask.
fn icon_image(size: usize) -> Vec<u8> {
    let px = icon_raster::render_rgba(size);
    let stride = size.div_ceil(32) * 4; // AND rows are DWORD aligned
    let xor_len = size * size * 4;
    let and_len = stride * size;

    let mut b = Vec::with_capacity(40 + xor_len + and_len);
    b.extend_from_slice(&40u32.to_le_bytes()); // biSize
    b.extend_from_slice(&(size as i32).to_le_bytes()); // biWidth
    b.extend_from_slice(&((size * 2) as i32).to_le_bytes()); // biHeight = XOR + AND
    b.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    b.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    b.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    b.extend_from_slice(&((xor_len + and_len) as u32).to_le_bytes()); // biSizeImage
    b.extend_from_slice(&[0u8; 16]); // ppm x/y, clrUsed, clrImportant

    for y in (0..size).rev() {
        b.extend_from_slice(&px[y * size * 4..y * size * 4 + size * 4]);
    }
    for y in (0..size).rev() {
        let mut row = vec![0u8; stride];
        for x in 0..size {
            if px[(y * size + x) * 4 + 3] < 128 {
                row[x / 8] |= 1 << (7 - x % 8); // bit set == transparent
            }
        }
        b.extend_from_slice(&row);
    }
    b
}

/// `GRPICONDIR`: the icon directory that lists the individual `RT_ICON` entries.
fn group_icon(images: &[(u16, usize, Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&0u16.to_le_bytes()); // reserved
    b.extend_from_slice(&1u16.to_le_bytes()); // type = icon
    b.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (id, size, image) in images {
        b.push(if *size >= 256 { 0 } else { *size as u8 }); // width (0 == 256)
        b.push(if *size >= 256 { 0 } else { *size as u8 }); // height
        b.push(0); // colour count
        b.push(0); // reserved
        b.extend_from_slice(&1u16.to_le_bytes()); // planes
        b.extend_from_slice(&32u16.to_le_bytes()); // bit count
        b.extend_from_slice(&(image.len() as u32).to_le_bytes()); // bytes in resource
        b.extend_from_slice(&id.to_le_bytes()); // RT_ICON id
    }
    b
}

/// A `.res` resource header. An ordinal is packed as `(id << 16) | 0xFFFF`.
fn push_header(out: &mut Vec<u8>, data_size: u32, ty: u32, name: u32, flags: u16) {
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend_from_slice(&0x20u32.to_le_bytes()); // header size
    out.extend_from_slice(&((ty << 16) | 0xFFFF).to_le_bytes());
    out.extend_from_slice(&((name << 16) | 0xFFFF).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // data version
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0x0409u16.to_le_bytes()); // language (en-US)
    out.extend_from_slice(&0u32.to_le_bytes()); // version
    out.extend_from_slice(&0u32.to_le_bytes()); // characteristics
}

/// Append resource data, zero-padded to a 4-byte boundary (padding is not part
/// of the header's `data_size`).
fn push_padded(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}
