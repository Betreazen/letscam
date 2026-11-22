//! Picture-in-picture: GUI snapshots (PNG) composed into the bottom-right corner of a frame.

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

/// An RGBA8 image, tightly packed.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.rgba.len() == self.width as usize * self.height as usize * 4
    }
}

pub fn load_png(path: &Path) -> Result<Image, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| format!("{}: слишком большое изображение", path.display()))?;
    let mut buf = vec![0; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let px = info.width as usize * info.height as usize;
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => {
            return Err(format!("{}: палитра не развёрнута", path.display()));
        }
    };
    if rgba.len() != px * 4 {
        return Err(format!("{}: неожиданный размер данных", path.display()));
    }
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

pub fn save_png(path: &Path, image: &Image) -> Result<(), String> {
    if !image.is_valid() {
        return Err(format!("{}: неверный размер изображения", path.display()));
    }
    let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(BufWriter::new(file), image.width, image.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let err = |e: png::EncodingError| format!("{}: {e}", path.display());
    let mut w = enc.write_header().map_err(err)?;
    w.write_image_data(&image.rgba).map_err(err)?;
    w.finish().map_err(err)
}

/// Halves the size with a 2x2 box filter; output is RGBA.
pub fn downscale2(width: u32, height: u32, pixels: &[[u8; 4]], bgra: bool) -> Option<Image> {
    let (w, h) = (width as usize, height as usize);
    if w < 2 || h < 2 || pixels.len() != w * h {
        return None;
    }
    let (ow, oh) = (w / 2, h / 2);
    let mut rgba = Vec::with_capacity(ow * oh * 4);
    for y in 0..oh {
        for x in 0..ow {
            let i = 2 * y * w + 2 * x;
            let quad = [pixels[i], pixels[i + 1], pixels[i + w], pixels[i + w + 1]];
            let avg = |c: usize| {
                let s: u32 = quad.iter().map(|p| u32::from(p[c])).sum();
                ((s + 2) / 4) as u8
            };
            let (r, b) = if bgra {
                (avg(2), avg(0))
            } else {
                (avg(0), avg(2))
            };
            rgba.extend_from_slice(&[r, avg(1), b, avg(3)]);
        }
    }
    Some(Image {
        width: ow as u32,
        height: oh as u32,
        rgba,
    })
}

/// Bilinear sample of `img` at continuous pixel coordinates (centers at +0.5).
fn sample(img: &Image, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (img.width as i64, img.height as i64);
    let (x, y) = (u - 0.5, v - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let px = |xi: i64, yi: i64| {
        let i = (yi.clamp(0, h - 1) * w + xi.clamp(0, w - 1)) as usize * 4;
        [
            img.rgba[i],
            img.rgba[i + 1],
            img.rgba[i + 2],
            img.rgba[i + 3],
        ]
        .map(f32::from)
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (a, b, c, d) = (
        px(x0, y0),
        px(x0 + 1, y0),
        px(x0, y0 + 1),
        px(x0 + 1, y0 + 1),
    );
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (d[k] - c[k]) * fx;
        top + (bottom - top) * fy
    })
}

struct Rect {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

/// Distance from pixel center to the rectangle (0 inside).
fn rect_dist(r: &Rect, x: i64, y: i64) -> f32 {
    let dx = (r.x0 - x).max(x - (r.x1 - 1)).max(0) as f32;
    let dy = (r.y0 - y).max(y - (r.y1 - 1)).max(0) as f32;
    dx.hypot(dy)
}

const SHADOW_ALPHA: f32 = 0.45;
const BORDER: [f32; 3] = [18.0, 18.0, 22.0];

/// Composes `overlay` scaled to `scale_of_width` of the frame width into the bottom-right
/// corner with a 2 px dark border and a soft shadow. `frame` is 4 bytes per pixel.
pub fn compose(
    frame: &mut [u8],
    frame_w: u32,
    frame_h: u32,
    frame_bgra: bool,
    overlay: &Image,
    scale_of_width: f32,
    margin_px: u32,
) {
    if frame.len() < frame_w as usize * frame_h as usize * 4 {
        log::warn!("LetsCam: картинка в картинке пропущена (неверные размеры)");
        return;
    }
    match PipLayer::new(
        frame_w,
        frame_h,
        frame_bgra,
        overlay,
        scale_of_width,
        margin_px,
    ) {
        Some(layer) => layer.apply(frame),
        None => log::warn!("LetsCam: картинка в картинке пропущена (неверные размеры)"),
    }
}

/// The picture-in-picture of one GUI snapshot for one frame size, worked out once: each pixel of
/// its box (the overlay, its border and its shadow) becomes `out = (add + frame * mul) / 256` per
/// colour channel. A snapshot lasts a quarter of a second (`gui_fps`), so the bilinear scaling and
/// the shadow are done once for its ~15 video frames instead of on every frame.
pub struct PipLayer {
    frame_w: u32,
    frame_h: u32,
    /// The box, in frame pixels: `x0..x0 + w`, `y0..y0 + h`.
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    /// Per box pixel, the frame's channels in order: what to add (0..=255×256) and how much of
    /// the frame to keep (0..=256).
    add: Vec<[u16; 3]>,
    mul: Vec<u16>,
}

impl PipLayer {
    /// None when the overlay or the frame has no usable size.
    pub fn new(
        frame_w: u32,
        frame_h: u32,
        frame_bgra: bool,
        overlay: &Image,
        scale_of_width: f32,
        margin_px: u32,
    ) -> Option<PipLayer> {
        let (fw, fh) = (i64::from(frame_w), i64::from(frame_h));
        if !overlay.is_valid() || fw < 8 || fh < 8 {
            return None;
        }
        let mut layer = PipLayer {
            frame_w,
            frame_h,
            x0: 0,
            y0: 0,
            w: 0,
            h: 0,
            add: Vec::new(),
            mul: Vec::new(),
        };
        layer.build(frame_bgra, overlay, scale_of_width, margin_px);
        Some(layer)
    }

    /// Whether this layer was made for frames of this size.
    pub fn fits(&self, frame_w: u32, frame_h: u32) -> bool {
        (self.frame_w, self.frame_h) == (frame_w, frame_h)
    }

    /// Blends the layer into `frame` (4 bytes per pixel, of the size it was made for).
    pub fn apply(&self, frame: &mut [u8]) {
        let fw = self.frame_w as usize;
        if frame.len() < fw * self.frame_h as usize * 4 {
            return;
        }
        for row in 0..self.h {
            let at = ((self.y0 + row) * fw + self.x0) * 4;
            let pixels = frame[at..at + self.w * 4].as_chunks_mut::<4>().0;
            let k = row * self.w;
            for ((px, add), &mul) in pixels
                .iter_mut()
                .zip(&self.add[k..k + self.w])
                .zip(&self.mul[k..k + self.w])
            {
                if mul == 256 {
                    continue;
                }
                for c in 0..3 {
                    px[c] =
                        ((u32::from(add[c]) + u32::from(px[c]) * u32::from(mul) + 128) >> 8) as u8;
                }
            }
        }
    }

    fn build(&mut self, frame_bgra: bool, overlay: &Image, scale_of_width: f32, margin_px: u32) {
        let (fw, fh) = (i64::from(self.frame_w), i64::from(self.frame_h));
        let margin = i64::from(margin_px);
        let scale = if scale_of_width.is_finite() {
            scale_of_width.clamp(0.05, 1.0)
        } else {
            0.45
        };
        let max_w = (fw - 2 * margin - 4).max(1);
        let max_h = (fh - 2 * margin - 4).max(1);
        let aspect = overlay.height as f32 / overlay.width as f32;
        let mut w = ((fw as f32 * scale).round() as i64).clamp(1, max_w);
        let mut h = ((w as f32 * aspect).round() as i64).max(1);
        if h > max_h {
            h = max_h;
            w = ((h as f32 / aspect).round() as i64).clamp(1, max_w);
        }
        let r = Rect {
            x0: fw - margin - w,
            y0: fh - margin - h,
            x1: fw - margin,
            y1: fh - margin,
        };
        let (ri, bi) = if frame_bgra { (2, 0) } else { (0, 2) };
        let offset = (w / 120).max(3);
        let soft = (w / 40).max(6);
        let shadow = Rect {
            x0: r.x0 + offset,
            y0: r.y0 + offset,
            x1: r.x1 + offset,
            y1: r.y1 + offset,
        };
        let border = 2;
        let (bx0, bx1) = ((r.x0 - border).max(0), (shadow.x1 + soft).min(fw));
        let (by0, by1) = ((r.y0 - border).max(0), (shadow.y1 + soft).min(fh));
        (self.x0, self.y0) = (bx0 as usize, by0 as usize);
        (self.w, self.h) = ((bx1 - bx0).max(0) as usize, (by1 - by0).max(0) as usize);
        self.add = Vec::with_capacity(self.w * self.h);
        self.mul = Vec::with_capacity(self.w * self.h);
        let fixed = |v: f32| (v * 256.0).round() as u16;
        for y in by0..by1 {
            for x in bx0..bx1 {
                let inside_border = x >= r.x0 - border
                    && x < r.x1 + border
                    && y >= r.y0 - border
                    && y < r.y1 + border;
                let (add, mul) = if x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1 {
                    let u = (x - r.x0) as f32 + 0.5;
                    let v = (y - r.y0) as f32 + 0.5;
                    let s = sample(
                        overlay,
                        u * overlay.width as f32 / w as f32,
                        v * overlay.height as f32 / h as f32,
                    );
                    let a = s[3] / 255.0;
                    let mut add = [0; 3];
                    for (dst, src) in [(ri, s[0]), (1, s[1]), (bi, s[2])] {
                        add[dst] = fixed(src * a);
                    }
                    (add, fixed(1.0 - a))
                } else if inside_border {
                    let mut add = [0; 3];
                    for (dst, src) in [(ri, BORDER[0]), (1, BORDER[1]), (bi, BORDER[2])] {
                        add[dst] = fixed(src);
                    }
                    (add, 0)
                } else {
                    let d = rect_dist(&shadow, x, y);
                    let dark = if d < soft as f32 {
                        let k = 1.0 - d / soft as f32;
                        1.0 - SHADOW_ALPHA * k * k
                    } else {
                        1.0
                    };
                    ([0; 3], fixed(dark))
                };
                self.add.push(add);
                self.mul.push(mul);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Image {
        Image {
            width: w,
            height: h,
            rgba: rgba.repeat((w * h) as usize),
        }
    }

    fn px(frame: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    }

    #[test]
    fn half_transparent_overlay_blends_as_the_float_formula() {
        let (fw, fh) = (200u32, 120u32);
        let dst = [40u8, 80, 120, 255];
        let mut frame = dst.repeat((fw * fh) as usize);
        let src = [200u8, 100, 50, 128];
        compose(&mut frame, fw, fh, false, &solid(20, 12, src), 0.45, 6);
        let got = px(&frame, fw, 160, 90);
        let a = f32::from(src[3]) / 255.0;
        for c in 0..3 {
            let want = f32::from(src[c]) * a + f32::from(dst[c]) * (1.0 - a);
            assert!(
                (f32::from(got[c]) - want).abs() <= 1.0,
                "channel {c}: {got:?} vs {want}"
            );
        }
        assert_eq!(got[3], 255, "alpha of the frame is kept");
    }

    #[test]
    fn one_layer_serves_every_frame_of_its_size() {
        let (fw, fh) = (320u32, 180u32);
        let overlay = solid(32, 18, [10, 200, 30, 200]);
        let layer = PipLayer::new(fw, fh, false, &overlay, 0.45, 8).expect("layer");
        assert!(layer.fits(fw, fh) && !layer.fits(fw, fh + 1));
        for shade in [0u8, 90, 255] {
            let mut by_layer = [shade, shade, shade, 255].repeat((fw * fh) as usize);
            let mut by_compose = by_layer.clone();
            layer.apply(&mut by_layer);
            compose(&mut by_compose, fw, fh, false, &overlay, 0.45, 8);
            assert_eq!(by_layer, by_compose, "shade {shade}");
        }
        assert!(PipLayer::new(4, 4, false, &overlay, 0.45, 0).is_none());
    }

    #[test]
    fn overlay_lands_bottom_right_with_right_size() {
        let (fw, fh) = (400u32, 200u32);
        let mut frame = [0u8, 0, 0, 255].repeat((fw * fh) as usize);
        let overlay = solid(160, 90, [255, 0, 0, 255]);
        compose(&mut frame, fw, fh, false, &overlay, 0.45, 10);
        // 0.45 * 400 = 180 wide, 180 * 90/160 = 101 high, ending 10 px from the edges.
        let red = |x, y| px(&frame, fw, x, y) == [255, 0, 0, 255];
        assert!(red(390 - 180, 190 - 101));
        assert!(red(389, 189));
        assert!(!red(389, 190), "margin below");
        assert!(!red(390 - 181, 150), "left of overlay");
        assert_eq!(px(&frame, fw, 5, 5), [0, 0, 0, 255], "far pixels untouched");
        assert_eq!(&px(&frame, fw, 391, 150)[..3], &[18, 18, 22], "border");
    }

    #[test]
    fn channel_order_follows_frame_format() {
        let (fw, fh) = (100u32, 100u32);
        let overlay = solid(10, 10, [200, 100, 50, 255]);
        let mut rgba = vec![0u8; (fw * fh * 4) as usize];
        compose(&mut rgba, fw, fh, false, &overlay, 0.5, 4);
        assert_eq!(&px(&rgba, fw, 90, 90)[..3], &[200, 100, 50]);
        let mut bgra = vec![0u8; (fw * fh * 4) as usize];
        compose(&mut bgra, fw, fh, true, &overlay, 0.5, 4);
        assert_eq!(&px(&bgra, fw, 90, 90)[..3], &[50, 100, 200]);
    }

    #[test]
    fn shadow_darkens_below_right() {
        let (fw, fh) = (400u32, 300u32);
        let mut frame = [200u8, 200, 200, 255].repeat((fw * fh) as usize);
        compose(
            &mut frame,
            fw,
            fh,
            false,
            &solid(16, 9, [0, 255, 0, 255]),
            0.45,
            20,
        );
        let p = px(&frame, fw, 383, 283);
        assert!(p[0] < 200, "shadow expected, got {p:?}");
    }

    #[test]
    fn bad_sizes_are_ignored() {
        let mut frame = vec![7u8; 16];
        compose(
            &mut frame,
            100,
            100,
            false,
            &solid(4, 4, [1, 2, 3, 255]),
            0.45,
            0,
        );
        assert_eq!(frame, vec![7u8; 16]);
    }

    #[test]
    fn png_round_trip_and_downscale() {
        let tmp = TempDir::new("pip");
        let pixels: Vec<[u8; 4]> = (0..16).map(|i| [i as u8 * 10, 0, 255, 255]).collect();
        let small = downscale2(4, 4, &pixels, true).expect("downscale");
        assert_eq!((small.width, small.height), (2, 2));
        // BGRA in: red channel is the third byte (255), blue the first (averaged).
        assert_eq!(small.rgba[0], 255);
        assert_eq!(small.rgba[2], 25);
        let path = tmp.path().join("a.png");
        save_png(&path, &small).expect("save");
        assert_eq!(load_png(&path).expect("load"), small);
        assert!(downscale2(3, 3, &pixels, false).is_none());
        assert!(load_png(&tmp.path().join("missing.png")).is_err());
    }
}
