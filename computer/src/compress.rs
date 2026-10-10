//! Content-aware compression of screen images for models.
//!
//! A raw Android screenshot is a 1–4 MB PNG at 1080×2400 or more. Models
//! downscale anything over ~1.15 megapixels on their side and then guess
//! coordinates in *their* smaller copy, so sending the full image costs bytes
//! and time and makes `tap point X Y` less accurate. This module does the
//! shrinking up front, where the exact scale factor is known:
//!
//! 1. **Analyze** a sample of pixels to classify the screen as dense UI text,
//!    ordinary UI, or sparse/flat content (`analyze`).
//! 2. **Resize** to a size that suits the class with an area-averaging filter
//!    (`target_size`, `resample`). Dense text keeps the most resolution.
//! 3. **Quantize** to an indexed palette: exact (lossless) when the image has
//!    few colors, median-cut otherwise (`quantize`). UI is flat color, so a
//!    256-color PNG is visually identical and 3–6× smaller.
//! 4. **Meet a byte budget** by trying fewer colors, then a bit smaller, and
//!    keeping the smallest result if nothing fits (`for_model`).
//!
//! The result carries an [`ImageSpace`]: the device and image sizes needed to
//! map coordinates the model reads off the image back to device pixels, plus a
//! one-line description to give the model so it knows which space it is in.
use crate::{
    api::{Code, Error, Result},
    visual::{self, Image},
};
use png::{AdaptiveFilterType, BitDepth, ColorType, Compression, Encoder, FilterType};
use serde_json::{json, Value};
use std::{borrow::Cow, collections::HashMap, env};

/// Pixel-count and byte limits for images shown to a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Longest side in pixels, an upper bound for every content class.
    pub max_edge: u32,
    /// Pixel count above which models downscale anyway.
    pub max_pixels: u32,
    /// Target encoded size in bytes. Missed only when even the smallest
    /// attempt is larger; the smallest attempt is then returned.
    pub budget: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self { max_edge: 1568, max_pixels: 1_150_000, budget: 350 * 1024 }
    }
}

impl Policy {
    /// Defaults, adjusted by `AU_IMAGE_MAX_EDGE` (pixels, 256–4096) and
    /// `AU_IMAGE_BUDGET_KB` (32–4096). Invalid values are ignored.
    pub fn from_env() -> Self {
        let read = |name: &str, range: std::ops::RangeInclusive<usize>| env::var(name).ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|v| range.contains(v));
        let mut policy = Self::default();
        if let Some(edge) = read("AU_IMAGE_MAX_EDGE", 256..=4096) {
            policy.max_edge = edge as u32;
        }
        if let Some(kb) = read("AU_IMAGE_BUDGET_KB", 32..=4096) {
            policy.budget = kb * 1024;
        }
        policy
    }
}

/// What a screen mostly is, which decides how much resolution it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    /// Edge-rich flat-color UI: small text that needs resolution.
    Dense,
    /// Ordinary UI, photos, or wallpaper.
    Normal,
    /// Mostly one color or almost no edges: little to lose by shrinking.
    Sparse,
}

impl Content {
    fn max_edge(self) -> u32 {
        match self {
            Self::Dense => 1568,
            Self::Normal => 1280,
            Self::Sparse => 960,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Analysis {
    pub content: Content,
    /// Fraction of sampled neighbor pairs with a strong luminance step.
    pub edge_density: f32,
    /// Share of samples in the single most common color bucket.
    pub dominant: f32,
    /// Distinct color buckets holding at least 0.01% of the samples.
    pub colors: usize,
}

const SAMPLE_STEP: usize = 3;
const EDGE_STEP: i32 = 40;

fn bucket(r: u8, g: u8, b: u8) -> usize {
    (usize::from(r >> 3) << 10) | (usize::from(g >> 3) << 5) | usize::from(b >> 3)
}

fn luma(p: &[u8]) -> i32 {
    (i32::from(p[0]) * 3 + i32::from(p[1]) * 6 + i32::from(p[2])) / 10
}

/// Classifies an image from a sparse sample of its pixels.
///
/// Sampling is strided, and UI is periodic (tile grids, dividers, text on a
/// baseline), so a fixed stride can line up with the pattern and see no edges
/// at all. Each sample is therefore compared with *both* horizontal neighbors,
/// and the sampled columns shift on successive row bands so every phase of
/// the stride is covered.
pub(crate) fn analyze(image: &Image) -> Analysis {
    let (w, h) = (image.w as usize, image.h as usize);
    let mut counts = vec![0u32; 1 << 15];
    let (mut samples, mut edges) = (0u32, 0u32);
    for y in (0..h).step_by(SAMPLE_STEP) {
        for x in ((y / SAMPLE_STEP) % SAMPLE_STEP..w).step_by(SAMPLE_STEP) {
            let at = (y * w + x) * 4;
            let p = &image.pixels[at..at + 4];
            counts[bucket(p[0], p[1], p[2])] += 1;
            samples += 1;
            let steps = |other: usize| (luma(p) - luma(&image.pixels[other..other + 4])).abs() > EDGE_STEP;
            // Horizontal stripes have no horizontal steps, so vertical neighbors count too.
            let row = w * 4;
            if (x > 0 && steps(at - 4)) || (x + 1 < w && steps(at + 4)) || (y > 0 && steps(at - row)) || (y + 1 < h && steps(at + row)) {
                edges += 1;
            }
        }
    }
    let samples_f = samples.max(1) as f32;
    let edge_density = edges as f32 / samples_f;
    let dominant = counts.iter().copied().max().unwrap_or(0) as f32 / samples_f;
    let floor = (samples / 10_000).max(2);
    let colors = counts.iter().filter(|&&n| n >= floor).count();
    let content = if dominant >= 0.90 || edge_density < 0.006 {
        Content::Sparse
    } else if edge_density >= 0.03 && colors <= 600 {
        Content::Dense
    } else {
        Content::Normal
    };
    Analysis { content, edge_density, dominant, colors }
}

/// Output size for an image: never larger than the input, never zero, aspect
/// ratio kept, within the class's edge limit and the policy's limits.
pub fn target_size(width: u32, height: u32, content: Content, policy: &Policy) -> (u32, u32) {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let edge = f64::from(content.max_edge().min(policy.max_edge));
    let factor = (edge / w.max(h)).min((f64::from(policy.max_pixels) / (w * h)).sqrt()).min(1.0);
    (((w * factor).round() as u32).clamp(1, width.max(1)), ((h * factor).round() as u32).clamp(1, height.max(1)))
}

/// Where the model's image sits relative to the device screen. Coordinates the
/// model reads off the image map back through this to device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSpace {
    pub device: (u32, u32),
    pub image: (u32, u32),
}

impl ImageSpace {
    pub fn is_scaled(&self) -> bool {
        self.device != self.image
    }

    /// Maps a point on the image to the device pixel under its center. A point
    /// outside the image is an error that restates the image's size, since the
    /// model is evidently working from a different picture than this one.
    pub fn to_device(self, x: u16, y: u16) -> Result<(u16, u16)> {
        let ((iw, ih), (dw, dh)) = (self.image, self.device);
        if u32::from(x) >= iw || u32::from(y) >= ih {
            return Err(Error::plain(
                Code::Bounds,
                format!("Point {x} {y} is outside the {iw}×{ih} screen image; nothing was run. Use x from 0 to {} and y from 0 to {} on the latest screen image, or capture screen again.", iw - 1, ih - 1),
            ));
        }
        let axis = |v: u16, image: u32, device: u32| {
            ((f64::from(v) + 0.5) * f64::from(device) / f64::from(image) - 0.5).round().clamp(0.0, f64::from(device.saturating_sub(1).min(65_535))) as u16
        };
        Ok((axis(x, iw, dw), axis(y, ih, dh)))
    }

    /// Device pixels per image pixel, for messages.
    pub fn factor(&self) -> f64 {
        f64::from(self.device.0) / f64::from(self.image.0.max(1))
    }

    pub fn to_json(self) -> Value {
        json!({"dw":self.device.0,"dh":self.device.1,"w":self.image.0,"h":self.image.1})
    }

    pub fn from_json(value: &Value) -> Option<Self> {
        let field = |key: &str| value.get(key)?.as_u64().and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0);
        Some(Self { device: (field("dw")?, field("dh")?), image: (field("w")?, field("h")?) })
    }

    /// Tells the model about a screen whose shape differs from the one it saw
    /// last (rotation, fold, or a different display), because every
    /// coordinate it remembers is then wrong.
    pub fn change_note(&self, previous: Option<Self>) -> Option<String> {
        let previous = previous.filter(|p| p.device != self.device)?;
        let shape = |d: (u32, u32)| if d.0 > d.1 { "landscape" } else { "portrait" };
        Some(format!(
            "The screen is now {} {}×{} (was {} {}×{}); coordinates from earlier images no longer apply.",
            shape(self.device),
            self.device.0,
            self.device.1,
            shape(previous.device),
            previous.device.0,
            previous.device.1
        ))
    }
}

#[derive(Debug)]
pub struct Compressed {
    pub png: Vec<u8>,
    pub space: ImageSpace,
    /// Palette size, or 0 for a truecolor/unchanged image (read by the tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub colors: usize,
}

impl Compressed {
    /// One line telling the model exactly what it is looking at.
    pub fn note(&self) -> String {
        let (w, h) = self.space.image;
        let kb = (self.png.len() + 512) / 1024;
        let size = if self.space.is_scaled() {
            format!(
                "It is {w}×{h} pixels, scaled down from the {}×{} device screen (1 image pixel = {:.2} device pixels), {kb} KB.",
                self.space.device.0,
                self.space.device.1,
                self.space.factor()
            )
        } else {
            format!("It is {w}×{h} pixels at full device resolution, {kb} KB.")
        };
        format!("{size} tap point and swipe take coordinates on this image; they are mapped to the device for you.")
    }
}

/// Compresses a PNG screen capture for a model. See the module docs.
pub fn for_model(bytes: &[u8], policy: &Policy) -> Result<Compressed> {
    let image = visual::decode(bytes)?;
    let analysis = analyze(&image);
    let (tw, th) = target_size(image.w, image.h, analysis.content, policy);
    let space = |w: u32, h: u32| ImageSpace { device: (image.w, image.h), image: (w, h) };
    let unchanged = |colors| Compressed { png: bytes.to_vec(), space: space(image.w, image.h), colors };
    if (tw, th) == (image.w, image.h) && bytes.len() <= policy.budget {
        return Ok(unchanged(0));
    }
    let opaque = image.pixels.chunks_exact(4).all(|p| p[3] == 255);
    // (palette size, extra shrink): lossless-if-possible first, then smaller.
    let attempts: &[(usize, f64)] = if opaque { &[(256, 1.0), (64, 1.0), (32, 0.8), (16, 0.65)] } else { &[(0, 1.0), (0, 0.8)] };
    let mut best: Option<Compressed> = None;
    let mut resized: Option<((u32, u32), Vec<u8>)> = None;
    for &(colors, shrink) in attempts {
        let (w, h) = ((f64::from(tw) * shrink).round() as u32, (f64::from(th) * shrink).round() as u32);
        let (w, h) = (w.clamp(1, tw), h.clamp(1, th));
        if shrink < 1.0 && w.min(h) < 240 {
            continue;
        }
        if resized.as_ref().map(|(size, _)| *size) != Some((w, h)) {
            resized = Some(((w, h), if (w, h) == (image.w, image.h) { image.pixels.clone() } else { resample(&image, w, h) }));
        }
        let pixels = &resized.as_ref().map(|(_, p)| p).ok_or_else(|| Error::new(Code::Protocol, "resampled image missing"))?[..];
        let (png, used) = if opaque {
            let indexed = quantize(pixels, colors);
            (encode_indexed(w, h, &indexed)?, indexed.palette.len())
        } else {
            (encode_rgba(w, h, pixels)?, 0)
        };
        let candidate = Compressed { png, space: space(w, h), colors: used };
        let fits = candidate.png.len() <= policy.budget;
        if best.as_ref().is_none_or(|b| candidate.png.len() < b.png.len()) {
            best = Some(candidate);
        }
        if fits {
            break;
        }
    }
    let best = best.ok_or_else(|| Error::new(Code::Protocol, "image compression produced no result"))?;
    // Never make an already-small, correctly sized capture larger.
    if best.space.image == (image.w, image.h) && best.png.len() >= bytes.len() {
        return Ok(unchanged(0));
    }
    Ok(best)
}

/// Area-averaging (box filter) downscale. Every source pixel contributes to
/// exactly one output pixel, so thin UI text degrades gracefully instead of
/// aliasing the way nearest-neighbour sampling does.
fn resample(image: &Image, width: u32, height: u32) -> Vec<u8> {
    let (sw, sh, w, h) = (image.w as usize, image.h as usize, width as usize, height as usize);
    let mut out = vec![0u8; w * h * 4];
    for oy in 0..h {
        let y0 = oy * sh / h;
        let y1 = ((oy + 1) * sh / h).max(y0 + 1);
        for ox in 0..w {
            let x0 = ox * sw / w;
            let x1 = ((ox + 1) * sw / w).max(x0 + 1);
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for p in image.pixels[(y * sw + x0) * 4..(y * sw + x1) * 4].chunks_exact(4) {
                    for (total, value) in sum.iter_mut().zip(p) {
                        *total += u32::from(*value);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (oy * w + ox) * 4;
            for (slot, total) in out[o..o + 4].iter_mut().zip(sum) {
                *slot = ((total + n / 2) / n) as u8;
            }
        }
    }
    out
}

pub(crate) struct Indexed {
    pub palette: Vec<[u8; 3]>,
    pub pixels: Vec<u8>,
}

/// Reduces opaque RGBA pixels to at most `max_colors` (≤ 256) palette entries.
/// Exact when the image already has that few colors; median-cut otherwise.
pub(crate) fn quantize(rgba: &[u8], max_colors: usize) -> Indexed {
    let max_colors = max_colors.clamp(1, 256);
    exact_palette(rgba, max_colors).unwrap_or_else(|| median_cut(rgba, max_colors))
}

fn exact_palette(rgba: &[u8], max: usize) -> Option<Indexed> {
    let mut seen: HashMap<u32, u8> = HashMap::new();
    let mut palette = Vec::new();
    let mut pixels = Vec::with_capacity(rgba.len() / 4);
    // UI is long runs of one color; skipping the map for repeats is most of the speed.
    let (mut last_key, mut last_index) = (u32::MAX, 0u8);
    for p in rgba.chunks_exact(4) {
        let key = u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]);
        if key != last_key {
            last_index = match seen.get(&key) {
                Some(&index) => index,
                None if palette.len() < max => {
                    let index = palette.len() as u8;
                    seen.insert(key, index);
                    palette.push([p[0], p[1], p[2]]);
                    index
                }
                None => return None,
            };
            last_key = key;
        }
        pixels.push(last_index);
    }
    Some(Indexed { palette, pixels })
}

struct Item {
    key: u16,
    n: u32,
    sum: [u64; 3],
    mean: [u8; 3],
}

struct Cell {
    start: usize,
    end: usize,
    channel: usize,
    score: u64,
}

fn measure(items: &[Item], start: usize, end: usize) -> Cell {
    let (mut lo, mut hi, mut total) = ([255u8; 3], [0u8; 3], 0u64);
    for item in &items[start..end] {
        total += u64::from(item.n);
        for c in 0..3 {
            lo[c] = lo[c].min(item.mean[c]);
            hi[c] = hi[c].max(item.mean[c]);
        }
    }
    let (channel, range) = (0..3).map(|c| (c, hi[c] - lo[c])).max_by_key(|&(_, r)| r).unwrap_or((0, 0));
    // Range keeps rare accent colors in the palette; population keeps the big areas accurate.
    Cell { start, end, channel, score: if end - start < 2 { 0 } else { u64::from(range) * ((total as f64).sqrt() as u64 + 1) } }
}

/// Median-cut over a 15-bit color histogram. Palette entries are averages of
/// the real pixels in each box, so flat colors stay exact.
fn median_cut(rgba: &[u8], max_colors: usize) -> Indexed {
    let mut counts = vec![0u32; 1 << 15];
    let mut sums = vec![[0u32; 3]; 1 << 15];
    for p in rgba.chunks_exact(4) {
        let k = bucket(p[0], p[1], p[2]);
        counts[k] += 1;
        for c in 0..3 {
            sums[k][c] += u32::from(p[c]);
        }
    }
    let mut items: Vec<Item> = (0..counts.len())
        .filter(|&k| counts[k] > 0)
        .map(|k| {
            let n = counts[k];
            let mean = [0, 1, 2].map(|c| ((u64::from(sums[k][c]) + u64::from(n) / 2) / u64::from(n)) as u8);
            Item { key: k as u16, n, sum: sums[k].map(u64::from), mean }
        })
        .collect();
    let mut cells = vec![measure(&items, 0, items.len())];
    while cells.len() < max_colors {
        let Some(at) = cells.iter().enumerate().filter(|(_, c)| c.score > 0).max_by_key(|(_, c)| c.score).map(|(i, _)| i) else { break };
        let cell = cells.swap_remove(at);
        items[cell.start..cell.end].sort_unstable_by_key(|item| item.mean[cell.channel]);
        let half = items[cell.start..cell.end].iter().map(|i| u64::from(i.n)).sum::<u64>() / 2;
        let (mut running, mut split) = (0u64, cell.start + 1);
        for (offset, item) in items[cell.start..cell.end].iter().enumerate() {
            running += u64::from(item.n);
            if running >= half {
                split = cell.start + offset + 1;
                break;
            }
        }
        let split = split.clamp(cell.start + 1, cell.end - 1);
        cells.push(measure(&items, cell.start, split));
        cells.push(measure(&items, split, cell.end));
    }
    let mut lut = vec![0u8; 1 << 15];
    let mut palette = Vec::with_capacity(cells.len());
    for (index, cell) in cells.iter().enumerate() {
        let (mut sum, mut n) = ([0u64; 3], 0u64);
        for item in &items[cell.start..cell.end] {
            lut[usize::from(item.key)] = index as u8;
            n += u64::from(item.n);
            for (total, part) in sum.iter_mut().zip(item.sum) {
                *total += part;
            }
        }
        palette.push([0, 1, 2].map(|c| ((sum[c] + n / 2) / n.max(1)) as u8));
    }
    let mut pixels = Vec::with_capacity(rgba.len() / 4);
    let (mut last_key, mut last_index) = (usize::MAX, 0u8);
    for p in rgba.chunks_exact(4) {
        let k = bucket(p[0], p[1], p[2]);
        if k != last_key {
            (last_key, last_index) = (k, lut[k]);
        }
        pixels.push(last_index);
    }
    Indexed { palette, pixels }
}

fn encode_indexed(w: u32, h: u32, indexed: &Indexed) -> Result<Vec<u8>> {
    let depth = match indexed.palette.len() {
        0..=2 => BitDepth::One,
        3..=4 => BitDepth::Two,
        5..=16 => BitDepth::Four,
        _ => BitDepth::Eight,
    };
    let bits = depth as usize;
    let data: Cow<[u8]> = if bits == 8 {
        Cow::Borrowed(&indexed.pixels)
    } else {
        let row = (w as usize * bits).div_ceil(8);
        let mut packed = vec![0u8; row * h as usize];
        for (y, line) in indexed.pixels.chunks_exact(w as usize).enumerate() {
            for (x, &index) in line.iter().enumerate() {
                let shift = 8 - bits - (x * bits) % 8;
                packed[y * row + x * bits / 8] |= index << shift;
            }
        }
        Cow::Owned(packed)
    };
    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, w, h);
    encoder.set_color(ColorType::Indexed);
    encoder.set_depth(depth);
    encoder.set_palette(indexed.palette.iter().flatten().copied().collect::<Vec<u8>>());
    // Palette images compress best unfiltered: runs of equal indexes are what deflate is good at.
    encoder.set_filter(FilterType::NoFilter);
    encoder.set_adaptive_filter(AdaptiveFilterType::NonAdaptive);
    encoder.set_compression(Compression::Default);
    let mut writer = encoder.write_header().map_err(|_| Error::new(Code::Io, "PNG encoder failed"))?;
    writer.write_image_data(&data).map_err(|_| Error::new(Code::Io, "PNG encoder failed"))?;
    writer.finish().map_err(|_| Error::new(Code::Io, "PNG encoder failed"))?;
    Ok(out)
}

fn encode_rgba(w: u32, h: u32, pixels: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, w, h);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(Compression::Fast);
    let mut writer = encoder.write_header().map_err(|_| Error::new(Code::Io, "PNG encoder failed"))?;
    writer.write_image_data(pixels).map_err(|_| Error::new(Code::Io, "PNG encoder failed"))?;
    writer.finish().map_err(|_| Error::new(Code::Io, "PNG encoder failed"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// A phone-like UI: white page, colored header, rows of dark "text" runs
    /// with a soft edge, and an accent button.
    fn ui(w: u32, h: u32) -> Image {
        let mut pixels = vec![255u8; (w * h * 4) as usize];
        let mut set = |x: u32, y: u32, c: [u8; 3]| {
            let at = ((y * w + x) * 4) as usize;
            pixels[at..at + 3].copy_from_slice(&c);
        };
        for y in 0..h {
            for x in 0..w {
                if y < h / 12 {
                    set(x, y, [30 + (x * 60 / w) as u8, 90, 200]);
                } else if (y / 6) % 5 < 3 && (x / 5) % 3 != 2 && x > w / 10 && x < w * 3 / 4 && y > h / 8 {
                    let ink = if (x + y) % 11 == 0 { 140 } else { 25 };
                    set(x, y, [ink, ink, ink]);
                } else if y > h * 9 / 10 && x > w / 4 && x < w * 3 / 4 {
                    set(x, y, [0, 160, 120]);
                }
            }
        }
        Image { w, h, pixels }
    }

    fn noise(w: u32, h: u32) -> Image {
        let mut state = 0x9e37_79b9_u32;
        let pixels = (0..w * h)
            .flat_map(|i| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let base = (i % w * 255 / w) as u8;
                [base.wrapping_add((state >> 24) as u8 / 4), (state >> 16) as u8, (state >> 8) as u8 / 2 + 60, 255]
            })
            .collect();
        Image { w, h, pixels }
    }

    fn flat(w: u32, h: u32) -> Image {
        Image { w, h, pixels: [18, 18, 22, 255].repeat((w * h) as usize) }
    }

    fn png(image: &Image) -> Vec<u8> {
        visual::encode(image.w, image.h, &image.pixels, usize::MAX).unwrap()
    }

    #[test]
    fn content_classes_follow_edges_and_color_variety() {
        assert_eq!(analyze(&ui(540, 1200)).content, Content::Dense);
        assert_eq!(analyze(&flat(540, 1200)).content, Content::Sparse);
        assert_eq!(analyze(&noise(540, 1200)).content, Content::Normal);
    }

    #[test]
    fn periodic_patterns_are_not_missed_at_any_stride_phase() {
        // Edges every 6 px: a fixed stride of 3 can sit exactly between them.
        for shift in 0..SAMPLE_STEP as u32 {
            let (w, h) = (300u32, 300u32);
            let pixels = (0..w * h).flat_map(|i| if ((i % w + shift) / 6) % 2 == 0 { [20, 20, 20, 255] } else { [240, 240, 240, 255] }).collect();
            let stripes = analyze(&Image { w, h, pixels });
            assert!(stripes.edge_density > 0.1, "shift {shift}: {}", stripes.edge_density);
            assert_eq!(stripes.content, Content::Dense, "shift {shift}");
        }
    }

    #[test]
    fn horizontal_stripes_are_not_mistaken_for_flat_content() {
        let (w, h) = (300u32, 300u32);
        let pixels = (0..w * h).flat_map(|i| if ((i / w) / 6) % 2 == 0 { [20, 20, 20, 255] } else { [240, 240, 240, 255] }).collect();
        let stripes = analyze(&Image { w, h, pixels });
        assert!(stripes.edge_density > 0.1, "{}", stripes.edge_density);
        assert_ne!(stripes.content, Content::Sparse);
    }

    #[test]
    fn target_sizes_are_bounded_and_keep_aspect() {
        let policy = Policy::default();
        assert_eq!(target_size(1080, 2400, Content::Dense, &policy), (706, 1568));
        let (w, h) = target_size(1080, 2400, Content::Sparse, &policy);
        assert!(h <= 960 && w < 706);
        assert_eq!(target_size(400, 800, Content::Dense, &policy), (400, 800), "never upscaled");
        let small_edge = Policy { max_edge: 512, ..policy };
        assert!(target_size(1080, 2400, Content::Dense, &small_edge).1 <= 512);
        for (w, h) in [(1, 1), (1, 5000), (3000, 2), (2560, 1600), (1440, 3120)] {
            let (tw, th) = target_size(w, h, Content::Dense, &policy);
            assert!(tw >= 1 && th >= 1 && tw <= w && th <= h && u64::from(tw) * u64::from(th) <= (u64::from(w) * u64::from(h)).min(1_200_000));
        }
    }

    #[test]
    fn ui_screens_shrink_in_pixels_and_bytes_and_stay_decodable() {
        let image = ui(1080, 2400);
        let original = png(&image);
        let out = for_model(&original, &Policy::default()).unwrap();
        assert_eq!(out.space, ImageSpace { device: (1080, 2400), image: (706, 1568) });
        assert!(out.png.len() * 5 < original.len(), "{} vs {}", out.png.len(), original.len());
        assert!(out.png.len() <= Policy::default().budget);
        let decoded = visual::decode(&out.png).expect("indexed output must decode for crop, hash, and diff");
        assert_eq!((decoded.w, decoded.h), (706, 1568));
        assert!(out.note().contains("706×1568") && out.note().contains("1080×2400") && out.note().contains("mapped to the device"));
    }

    #[test]
    fn few_color_images_round_trip_exactly_through_the_palette() {
        let image = ui(360, 800);
        let distinct: std::collections::HashSet<_> = image.pixels.chunks_exact(4).map(|p| [p[0], p[1], p[2]]).collect();
        assert!(distinct.len() <= 256, "fixture should be few-color, has {}", distinct.len());
        let indexed = quantize(&image.pixels, 256);
        assert_eq!(indexed.palette.len(), distinct.len(), "exact palette, nothing merged");
        let decoded = visual::decode(&encode_indexed(360, 800, &indexed).unwrap()).unwrap();
        assert!(image.pixels.chunks_exact(4).zip(decoded.pixels.chunks_exact(4)).all(|(a, b)| a[..3] == b[..3]), "indexed PNG must reproduce every pixel");
    }

    #[test]
    fn palette_bit_depths_pack_without_corrupting_rows() {
        for colors in [2usize, 3, 4, 9, 16, 17] {
            let (w, h) = (13u32, 7u32);
            let pixels: Vec<u8> = (0..w * h)
                .flat_map(|i| {
                    let c = (i as usize % colors) as u8 * 13;
                    [c, 255 - c, c / 2, 255]
                })
                .collect();
            let indexed = quantize(&pixels, colors);
            assert_eq!(indexed.palette.len(), colors);
            let decoded = visual::decode(&encode_indexed(w, h, &indexed).unwrap()).unwrap();
            assert_eq!(decoded.pixels, pixels, "{colors} colors, odd width");
        }
    }

    #[test]
    fn median_cut_stays_close_on_many_colors() {
        let image = noise(200, 200);
        let indexed = quantize(&image.pixels, 256);
        assert!(indexed.palette.len() <= 256);
        let error: u64 =
            image.pixels.chunks_exact(4).zip(&indexed.pixels).map(|(p, &i)| (0..3).map(|c| u64::from(p[c].abs_diff(indexed.palette[usize::from(i)][c]))).sum::<u64>()).sum();
        let mean = error as f64 / (200.0 * 200.0 * 3.0);
        assert!(mean < 14.0, "mean per-channel error {mean}");
        let few = quantize(&image.pixels, 16);
        assert!(few.palette.len() <= 16 && few.pixels.len() == 200 * 200);
    }

    #[test]
    fn tight_budgets_trade_colors_and_size_before_failing() {
        let original = png(&noise(1080, 1600));
        let loose = for_model(&original, &Policy { budget: 4 << 20, ..Policy::default() }).unwrap();
        let tight = for_model(&original, &Policy { budget: 20 * 1024, ..Policy::default() }).unwrap();
        assert!(tight.png.len() < loose.png.len() && tight.space.image.0 <= loose.space.image.0);
        assert!(tight.colors <= loose.colors);
        assert!(visual::decode(&tight.png).is_ok());
    }

    #[test]
    fn small_images_under_budget_pass_through_untouched() {
        let original = png(&flat(320, 480));
        let out = for_model(&original, &Policy::default()).unwrap();
        assert_eq!(out.png, original);
        assert!(!out.space.is_scaled());
        assert!(out.note().contains("full device resolution"));
    }

    #[test]
    fn transparent_pixels_survive_as_truecolor() {
        let mut image = ui(120, 120);
        image.pixels[3] = 0;
        let out = for_model(&png(&image), &Policy { budget: 1, ..Policy::default() }).unwrap();
        assert_eq!(out.colors, 0);
        assert!(visual::decode(&out.png).is_ok());
    }

    #[test]
    fn image_space_maps_points_and_rejects_foreign_coordinates() {
        let space = ImageSpace { device: (1080, 2400), image: (706, 1568) };
        let (x, y) = space.to_device(353, 784).unwrap();
        assert!((i32::from(x) - 540).abs() <= 2 && (i32::from(y) - 1200).abs() <= 2);
        assert_eq!(space.to_device(0, 0).unwrap(), (0, 0));
        assert_eq!(space.to_device(705, 1567).unwrap(), (1079, 2399));
        let error = space.to_device(900, 100).unwrap_err();
        assert!(error.plain && error.message.contains("706×1568") && error.message.contains("nothing was run"));
        assert_eq!(ImageSpace { device: (100, 200), image: (100, 200) }.to_device(7, 9).unwrap(), (7, 9));
        assert_eq!(ImageSpace::from_json(&space.to_json()), Some(space));
        assert_eq!(ImageSpace::from_json(&json!({"dw":0,"dh":1,"w":1,"h":1})), None);
    }

    #[test]
    fn rotation_is_called_out_with_both_shapes() {
        let portrait = ImageSpace { device: (1080, 2400), image: (706, 1568) };
        let landscape = ImageSpace { device: (2400, 1080), image: (1568, 706) };
        let note = landscape.change_note(Some(portrait)).unwrap();
        assert!(note.contains("landscape 2400×1080") && note.contains("portrait 1080×2400"));
        assert!(portrait.change_note(Some(portrait)).is_none() && portrait.change_note(None).is_none());
    }

    #[test]
    fn env_overrides_ignore_garbage() {
        assert_eq!(Policy::from_env().max_edge, Policy::default().max_edge);
    }

    /// Writes every pipeline layer for one capture, for inspecting what each does:
    /// `AU_LAYERS_IN=shot.png AU_LAYERS_OUT=dir cargo test --release -p au compress::tests::layers -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn layers() {
        let (Ok(input), Ok(out)) = (env::var("AU_LAYERS_IN"), env::var("AU_LAYERS_OUT")) else { return };
        let bytes = std::fs::read(&input).unwrap();
        let image = visual::decode(&bytes).unwrap();
        let policy = Policy::default();
        let analysis = analyze(&image);
        let (tw, th) = target_size(image.w, image.h, analysis.content, &policy);
        println!("input {}x{} {} bytes", image.w, image.h, bytes.len());
        println!("analysis: {:?}, edge {:.3}, dominant {:.3}, colors {}", analysis.content, analysis.edge_density, analysis.dominant, analysis.colors);
        println!("target size {tw}x{th} (never above {}x{})", image.w, image.h);
        let save = |name: &str, w: u32, h: u32, png: &[u8], note: &str| {
            std::fs::write(format!("{out}/{name}.png"), png).unwrap();
            println!("{name}: {w}x{h} {} bytes ({:.1}x smaller) {note}", png.len(), bytes.len() as f64 / png.len() as f64);
        };
        let full = resample(&image, tw, th);
        save("01-resized-truecolor", tw, th, &encode_rgba(tw, th, &full).unwrap(), "layer 2 only: area-averaged resize");
        for (name, colors, shrink) in [("02-palette-256", 256, 1.0), ("03-palette-64", 64, 1.0), ("04-palette-32-smaller", 32, 0.8), ("05-palette-16-smallest", 16, 0.65)] {
            let (w, h) = (((f64::from(tw) * shrink).round() as u32).max(1), ((f64::from(th) * shrink).round() as u32).max(1));
            let pixels = if (w, h) == (tw, th) { full.clone() } else { resample(&image, w, h) };
            let indexed = quantize(&pixels, colors);
            save(name, w, h, &encode_indexed(w, h, &indexed).unwrap(), &format!("palette {}", indexed.palette.len()));
        }
        let started = Instant::now();
        let chosen = for_model(&bytes, &policy).unwrap();
        std::fs::write(format!("{out}/06-chosen-by-for_model.png"), &chosen.png).unwrap();
        println!(
            "06-chosen-by-for_model: {}x{} {} bytes, {} colors, {:?}\n{}",
            chosen.space.image.0,
            chosen.space.image.1,
            chosen.png.len(),
            chosen.colors,
            started.elapsed(),
            chosen.note()
        );
    }

    /// `cargo test --release -p au compress::tests::speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn speed() {
        for (name, image) in [("ui 1080x2400", ui(1080, 2400)), ("ui 1440x3120", ui(1440, 3120)), ("noise 1080x2400", noise(1080, 2400))] {
            let original = png(&image);
            let started = Instant::now();
            let out = for_model(&original, &Policy::default()).unwrap();
            println!(
                "{name}: {} -> {} bytes ({:.1}x), {}x{} -> {}x{}, {} colors, {:?}",
                original.len(),
                out.png.len(),
                original.len() as f64 / out.png.len() as f64,
                image.w,
                image.h,
                out.space.image.0,
                out.space.image.1,
                out.colors,
                started.elapsed()
            );
        }
    }
}
