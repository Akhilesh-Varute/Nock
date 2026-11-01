//! Drawing API shared by every platform. The backend (`gfx_win`: Direct2D,
//! `gfx_linux`: tiny-skia + fontdue) provides `Gfx`, `Fonts`, `Font` and `Bitmap`.

#[cfg(windows)]
pub use crate::gfx_win::*;
#[cfg(not(windows))]
pub use crate::gfx_linux::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w * 0.5
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h * 0.5
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }
    pub const fn white(a: f32) -> Self {
        Color(1.0, 1.0, 1.0, a)
    }
    /// Multiply alpha (used for fading content in and out).
    pub fn fade(self, k: f32) -> Self {
        Color(self.0, self.1, self.2, self.3 * k.clamp(0.0, 1.0))
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}
