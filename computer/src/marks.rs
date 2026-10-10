//! Set-of-marks: numbered boxes drawn on the screen image over each control, so a model that has
//! to look at the picture answers `tap e12` instead of guessing pixel coordinates.
use crate::visual::Image;

/// A control's handle and its bounds in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mark {
    pub handle: u16,
    pub rect: (u32, u32, u32, u32),
}

/// 3×5 digit glyphs, one row per entry, bit 2 on the left.
const GLYPHS: [[u8; 5]; 10] =
    [[7, 5, 5, 5, 7], [2, 6, 2, 2, 7], [7, 1, 7, 4, 7], [7, 1, 7, 1, 7], [5, 5, 7, 1, 1], [7, 4, 7, 1, 7], [7, 4, 7, 5, 7], [7, 1, 1, 2, 2], [7, 5, 7, 5, 7], [7, 5, 7, 1, 7]];
const RED: [u8; 4] = [220, 30, 30, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

fn fill(image: &mut Image, x: u32, y: u32, w: u32, h: u32, color: [u8; 4]) {
    for row in y..(y + h).min(image.h) {
        for col in x..(x + w).min(image.w) {
            let at = (row as usize * image.w as usize + col as usize) * 4;
            image.pixels[at..at + 4].copy_from_slice(&color);
        }
    }
}

/// Draws every mark: a red outline, and its number on a red tab at the top-left corner. Sizes
/// follow the screen width so the digits stay readable after the image is scaled down.
pub(crate) fn draw(image: &mut Image, marks: &[Mark]) {
    let unit = (image.w / 120).max(3); // pixel size of one glyph cell
    let line = (unit / 2).max(2);
    for mark in marks {
        let (x, y, right, bottom) = mark.rect;
        let (w, h) = (right.saturating_sub(x), bottom.saturating_sub(y));
        if w < line * 2 || h < line * 2 {
            continue;
        }
        for (rx, ry, rw, rh) in [(x, y, w, line), (x, bottom - line, w, line), (x, y, line, h), (right - line, y, line, h)] {
            fill(image, rx, ry, rw, rh, RED);
        }
        let digits: Vec<u8> = mark.handle.to_string().bytes().map(|b| b - b'0').collect();
        let tab_w = (digits.len() as u32 * 4 + 1) * unit;
        fill(image, x, y, tab_w, 7 * unit, RED);
        for (i, digit) in digits.iter().enumerate() {
            for (row, bits) in GLYPHS[*digit as usize].iter().enumerate() {
                for col in 0..3 {
                    if bits & (4 >> col) != 0 {
                        fill(image, x + (i as u32 * 4 + 1 + col) * unit, y + (row as u32 + 1) * unit, unit, unit, WHITE);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_is_an_outline_with_a_numbered_tab() {
        let mut image = Image { w: 360, h: 360, pixels: vec![255; 360 * 360 * 4] };
        draw(&mut image, &[Mark { handle: 12, rect: (60, 100, 300, 200) }]);
        let at = |x: usize, y: usize| image.pixels[(y * 360 + x) * 4..(y * 360 + x) * 4 + 3].to_vec();
        assert_eq!(at(150, 100), RED[..3], "top edge");
        assert_eq!(at(150, 150), [255, 255, 255], "inside stays untouched");
        assert!((60..120).any(|x| (100..130).any(|y| at(x, y) == WHITE[..3])), "digits are drawn on the tab");
        let before = image.pixels.clone();
        draw(&mut image, &[Mark { handle: 5, rect: (10, 10, 12, 12) }]);
        assert_eq!(image.pixels, before, "marks too small to read are skipped");
    }
}
