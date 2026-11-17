//! Turns Theora's Y'CbCr planes into RGBA pixels.

use theora_rs::{Info, PixelFmt, YCbCrRef};

/// Writes the visible part of a decoded frame to `rgba`, four bytes a
/// pixel in rows from the top. `rgba` must hold the picture exactly.
///
/// Theora's colorspaces use the BT.601 matrix with luma from 16 to 235
/// and chroma from 16 to 240, so that is what is undone here.
pub(super) fn to_rgba(planes: &YCbCrRef<'_>, info: &Info, rgba: &mut [u8]) {
    let (x_shift, y_shift) = match info.pixel_fmt {
        PixelFmt::Pf444 => (0, 0),
        PixelFmt::Pf422 => (1, 0),
        PixelFmt::Pf420 | PixelFmt::Reserved => (1, 1),
    };
    let width = info.pic_width as usize;
    let (left, top) = (info.pic_x as usize, info.pic_y as usize);
    let [luma, cb, cr] = planes;
    for (row, out) in rgba.chunks_exact_mut(width * 4).enumerate() {
        let y = top + row;
        let luma_row = plane_row(luma, y);
        let cb_row = plane_row(cb, y >> y_shift);
        let cr_row = plane_row(cr, y >> y_shift);
        for (column, pixel) in out.chunks_exact_mut(4).enumerate() {
            let x = left + column;
            let c = 298 * (i32::from(luma_row[x]) - 16);
            let d = i32::from(cb_row[x >> x_shift]) - 128;
            let e = i32::from(cr_row[x >> x_shift]) - 128;
            pixel[0] = clamp((c + 409 * e + 128) >> 8);
            pixel[1] = clamp((c - 100 * d - 208 * e + 128) >> 8);
            pixel[2] = clamp((c + 516 * d + 128) >> 8);
            pixel[3] = 255;
        }
    }
}

/// Row `y` of a plane, whose stride may be negative.
fn plane_row<'a>(plane: &theora_rs::ImgPlaneRef<'a>, y: usize) -> &'a [u8] {
    let start = plane.data_offset as isize + y as isize * plane.stride as isize;
    let start = start as usize;
    &plane.data[start..start + plane.width as usize]
}

fn clamp(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}
