//! Linux backend: tiny-skia (CPU) for shapes and fontdue for text, drawing into
//! a premultiplied RGBA pixmap. The window layer presents the pixmap.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use tiny_skia::{FillRule, FilterQuality, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::gfx::{Align, Color, Rect};

pub type Result<T> = std::result::Result<T, String>;

/// A font face at a fixed size (in DIPs, like the Windows text formats).
#[derive(Clone)]
pub struct Font {
    face: Rc<fontdue::Font>,
    size: f32,
}

#[derive(Clone)]
pub struct Bitmap(Rc<Pixmap>);

pub struct Fonts {
    pub title: Font,
    pub body: Font,
    pub small: Font,
    pub menu: Font,
    pub tiny: Font,
    pub big: Font,
    pub icon14: Font,
    pub icon18: Font,
    pub icon22: Font,
}

pub struct Gfx {
    pix: RefCell<Pixmap>,
    ts: Cell<Transform>,
    scale: f32,
    pub fonts: Fonts,
}

fn load_font(paths: &[&str]) -> Option<Rc<fontdue::Font>> {
    paths.iter().find_map(|p| {
        let bytes = std::fs::read(p).ok()?;
        fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok().map(Rc::new)
    })
}

fn make_fonts() -> Result<Fonts> {
    // ponytail: fixed font paths, fontconfig lookup if distros other than Debian/Ubuntu matter
    let ui = load_font(&[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ])
    .ok_or("no UI font found")?;
    let f = |size: f32| Font { face: ui.clone(), size };
    Ok(Fonts {
        title: f(17.0),
        body: f(14.0),
        small: f(12.0),
        menu: f(13.0),
        tiny: f(11.0),
        big: f(34.0),
        // ponytail: icons share the UI font until a Linux icon font is chosen
        icon14: f(14.0),
        icon18: f(18.0),
        icon22: f(22.0),
    })
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(
        (c.0 * 255.0).round() as u8,
        (c.1 * 255.0).round() as u8,
        (c.2 * 255.0).round() as u8,
        (c.3.clamp(0.0, 1.0) * 255.0).round() as u8,
    );
    p.anti_alias = true;
    p
}

fn rrect_path(r: Rect, rad: f32) -> Option<tiny_skia::Path> {
    let rad = rad.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let k = rad * 0.552_284_75;
    let mut b = PathBuilder::new();
    b.move_to(x0 + rad, y0);
    b.line_to(x1 - rad, y0);
    b.cubic_to(x1 - rad + k, y0, x1, y0 + rad - k, x1, y0 + rad);
    b.line_to(x1, y1 - rad);
    b.cubic_to(x1, y1 - rad + k, x1 - rad + k, y1, x1 - rad, y1);
    b.line_to(x0 + rad, y1);
    b.cubic_to(x0 + rad - k, y1, x0, y1 - rad + k, x0, y1 - rad);
    b.line_to(x0, y0 + rad);
    b.cubic_to(x0, y0 + rad - k, x0 + rad - k, y0, x0 + rad, y0);
    b.close();
    b.finish()
}

impl Gfx {
    /// `max_w` / `max_h` are the largest window size in *physical* pixels.
    pub fn new(max_w: i32, max_h: i32, scale: f32) -> Result<Gfx> {
        let pix = Pixmap::new(max_w.max(1) as u32, max_h.max(1) as u32).ok_or("bad pixmap size")?;
        Ok(Gfx {
            pix: RefCell::new(pix),
            ts: Cell::new(Transform::from_scale(scale, scale)),
            scale,
            fonts: make_fonts()?,
        })
    }

    /// Test helper: the whole backing bitmap as top-down BGRA (premultiplied), plus its size.
    #[cfg(test)]
    pub fn snapshot(&self) -> (i32, i32, Vec<u8>) {
        let p = self.pix.borrow();
        let mut buf = p.data().to_vec();
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        (p.width() as i32, p.height() as i32, buf)
    }

    /// The pixels (premultiplied RGBA) and their size, for the window layer to present.
    #[allow(dead_code)]
    pub fn pixels(&self) -> (u32, u32, Vec<u8>) {
        let p = self.pix.borrow();
        (p.width(), p.height(), p.data().to_vec())
    }

    /// Test helper: write the backing bitmap as a PNG.
    #[cfg(test)]
    pub fn save_png(&self, path: &str) {
        self.pix.borrow().save_png(path).unwrap();
    }

    // ------------------------------------------------------------ frame ----

    pub fn begin(&self, _pw: i32, _ph: i32) -> Result<()> {
        self.pix.borrow_mut().fill(tiny_skia::Color::TRANSPARENT);
        self.ts.set(Transform::from_scale(self.scale, self.scale));
        Ok(())
    }

    pub fn end(&self) -> Result<()> {
        Ok(())
    }

    // -------------------------------------------------------- primitives ----

    pub fn fill_rrect(&self, r: Rect, rad: f32, c: Color) {
        if let Some(path) = rrect_path(r, rad) {
            self.pix.borrow_mut().fill_path(&path, &paint(c), FillRule::Winding, self.ts.get(), None);
        }
    }

    pub fn stroke_rrect(&self, r: Rect, rad: f32, c: Color, width: f32) {
        if let Some(path) = rrect_path(r, rad) {
            let stroke = Stroke { width, ..Stroke::default() };
            self.pix.borrow_mut().stroke_path(&path, &paint(c), &stroke, self.ts.get(), None);
        }
    }

    /// Filled polygon (ears, capes, tails...).
    pub fn fill_poly(&self, pts: &[(f32, f32)], c: Color) {
        if pts.len() < 3 {
            return;
        }
        let mut b = PathBuilder::new();
        b.move_to(pts[0].0, pts[0].1);
        for p in &pts[1..] {
            b.line_to(p.0, p.1);
        }
        b.close();
        if let Some(path) = b.finish() {
            self.pix.borrow_mut().fill_path(&path, &paint(c), FillRule::Winding, self.ts.get(), None);
        }
    }

    pub fn line(&self, a: (f32, f32), b: (f32, f32), width: f32, c: Color) {
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        if let Some(path) = pb.finish() {
            let stroke = Stroke { width, ..Stroke::default() };
            self.pix.borrow_mut().stroke_path(&path, &paint(c), &stroke, self.ts.get(), None);
        }
    }

    pub fn fill_circle(&self, cx: f32, cy: f32, rad: f32, c: Color) {
        if let Some(path) = PathBuilder::from_circle(cx, cy, rad) {
            self.pix.borrow_mut().fill_path(&path, &paint(c), FillRule::Winding, self.ts.get(), None);
        }
    }

    pub fn text(&self, s: &str, r: Rect, f: &Font, c: Color, align: Align) {
        if s.is_empty() || c.3 <= 0.003 {
            return;
        }
        // Text honours translation and uniform scale only (nothing rotates text).
        let t = self.ts.get();
        let k = t.sx;
        let px = f.size * k;
        let mut glyphs: Vec<_> = s.chars().map(|ch| f.face.rasterize(ch, px)).collect();
        let mut width: f32 = glyphs.iter().map(|(m, _)| m.advance_width).sum();
        let (box_x, box_w) = (r.x * k + t.tx, r.w * k);
        if width > box_w {
            // too long: keep what fits and end with an ellipsis
            let dots = f.face.rasterize('\u{2026}', px);
            let mut used = dots.0.advance_width;
            let keep = glyphs
                .iter()
                .take_while(|(m, _)| {
                    used += m.advance_width;
                    used <= box_w
                })
                .count();
            glyphs.truncate(keep);
            glyphs.push(dots);
            width = glyphs.iter().map(|(m, _)| m.advance_width).sum();
        }
        let mut x = match align {
            Align::Left => box_x,
            Align::Center => box_x + (box_w - width) * 0.5,
            Align::Right => box_x + box_w - width,
        };
        let lm = f.face.horizontal_line_metrics(px);
        let (asc, desc) = lm.map(|m| (m.ascent, m.descent)).unwrap_or((px * 0.8, -px * 0.2));
        let base = r.y * k + t.ty + (r.h * k + asc + desc) * 0.5;
        let clip_r = box_x + box_w;
        let mut pm = self.pix.borrow_mut();
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let data = pm.data_mut();
        for (m, bmp) in glyphs {
            if x + m.advance_width > clip_r + 0.5 {
                break;
            }
            let gx = (x + m.xmin as f32).round() as i32;
            let gy = (base - m.height as f32 - m.ymin as f32).round() as i32;
            for row in 0..m.height as i32 {
                for col in 0..m.width as i32 {
                    let (dx, dy) = (gx + col, gy + row);
                    if dx < 0 || dy < 0 || dx >= pw || dy >= ph {
                        continue;
                    }
                    let a = bmp[(row * m.width as i32 + col) as usize] as f32 / 255.0 * c.3;
                    if a <= 0.0 {
                        continue;
                    }
                    let i = ((dy * pw + dx) * 4) as usize;
                    let src = [c.0 * a, c.1 * a, c.2 * a, a];
                    for ch in 0..4 {
                        let d = data[i + ch] as f32 / 255.0;
                        data[i + ch] = ((src[ch] + d * (1.0 - a)) * 255.0).round() as u8;
                    }
                }
            }
            x += m.advance_width;
        }
    }

    /// Width of `s` in DIPs.
    pub fn text_width(&self, s: &str, f: &Font) -> f32 {
        s.chars().map(|c| f.face.metrics(c, f.size).advance_width).sum()
    }

    /// Draw a glyph from the icon font, centred in `r`.
    pub fn icon(&self, glyph: char, r: Rect, f: &Font, c: Color) {
        let mut buf = [0u8; 4];
        let s = glyph.encode_utf8(&mut buf);
        self.text(s, r, f, c, Align::Center);
    }

    /// Fill a rounded rect with a bitmap (used for album art / app icons).
    pub fn draw_bitmap(&self, bmp: &Bitmap, r: Rect, rad: f32, alpha: f32) {
        let (bw, bh) = (bmp.0.width() as f32, bmp.0.height() as f32);
        let Some(path) = rrect_path(r, rad) else { return };
        let shader = tiny_skia::Pattern::new(
            bmp.0.as_ref().as_ref(),
            tiny_skia::SpreadMode::Pad,
            FilterQuality::Bilinear,
            alpha.clamp(0.0, 1.0),
            Transform::from_row(r.w / bw, 0.0, 0.0, r.h / bh, r.x, r.y),
        );
        let mut p = Paint::default();
        p.shader = shader;
        p.anti_alias = true;
        self.pix.borrow_mut().fill_path(&path, &p, FillRule::Winding, self.ts.get(), None);
    }

    fn with_transform(&self, local: Transform, f: impl FnOnce()) {
        let base = self.ts.get();
        self.ts.set(base.pre_concat(local));
        f();
        self.ts.set(base);
    }

    /// Run `f` with everything it draws moved by (dx, dy).
    pub fn shifted(&self, dx: f32, dy: f32, f: impl FnOnce()) {
        self.with_transform(Transform::from_translate(dx, dy), f);
    }

    /// Run `f` with everything it draws scaled by (sx, sy) about (cx, cy).
    pub fn scaled(&self, cx: f32, cy: f32, sx: f32, sy: f32, f: impl FnOnce()) {
        self.with_transform(Transform::from_row(sx, 0.0, 0.0, sy, cx - cx * sx, cy - cy * sy), f);
    }

    /// Run `f` with everything it draws rotated `deg` degrees about (cx, cy).
    pub fn rotated(&self, cx: f32, cy: f32, deg: f32, f: impl FnOnce()) {
        self.with_transform(Transform::from_rotate_at(deg, cx, cy), f);
    }

    // ------------------------------------------------------------ images ----

    /// Decode encoded image bytes into a bitmap plus its average colour.
    pub fn decode_image(&self, bytes: &[u8]) -> Result<(Bitmap, Color)> {
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
        let (w, h) = img.dimensions();
        let mut pix = Pixmap::new(w, h).ok_or("empty image")?;
        let (mut r, mut g, mut b, mut n) = (0f32, 0f32, 0f32, 0f32);
        for (i, (src, dst)) in img.as_raw().chunks_exact(4).zip(pix.data_mut().chunks_exact_mut(4)).enumerate() {
            let a = src[3] as u32;
            // tiny-skia wants premultiplied pixels
            dst.copy_from_slice(&[
                (src[0] as u32 * a / 255) as u8,
                (src[1] as u32 * a / 255) as u8,
                (src[2] as u32 * a / 255) as u8,
                src[3],
            ]);
            // average colour from a sparse sample of the pixels
            if i % 97 == 0 && src[3] > 8 {
                r += src[0] as f32 / 255.0;
                g += src[1] as f32 / 255.0;
                b += src[2] as f32 / 255.0;
                n += 1.0;
            }
        }
        let col = if n > 0.0 { Color(r / n, g / n, b / n, 1.0) } else { Color(0.4, 0.6, 1.0, 1.0) };
        Ok((Bitmap(Rc::new(pix)), col))
    }
}
