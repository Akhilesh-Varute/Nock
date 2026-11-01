//! Linux entry point (work in progress): a borderless X11 winit window that draws
//! the notch pill and, on click, the tabbed panel through the shared `gfx` API.
//! Pages are placeholders except Stats; they move over from `ui.rs` as `App`
//! sheds its Windows types.

use std::num::NonZeroU32;
use std::rc::Rc;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::x11::{EventLoopBuilderExtX11, WindowAttributesExtX11, WindowType};
use winit::window::{Window, WindowId, WindowLevel};

use crate::gfx::*;

const PILL_W: f32 = 176.0;
const PILL_H: f32 = 34.0;
const EXP_W: f32 = 600.0;
const EXP_H: f32 = 250.0;
const CONTENT_Y: f32 = 40.0;

const T1: Color = Color::white(0.96);
const T2: Color = Color::white(0.62);
const T3: Color = Color::white(0.38);
const ACCENT: Color = Color(0.40, 0.60, 1.0, 1.0);

const TAB_LABELS: [&str; 9] =
    ["Media", "Controls", "Today", "Stats", "Clipboard", "Shelf", "Apps", "Games", "Power"];

/// Where each tab sits in the top row (same layout rule as the Windows panel).
fn tab_rects() -> Vec<Rect> {
    let text_w = |s: &str| -> f32 {
        s.chars()
            .map(|c| match c {
                'i' | 'l' | 't' | 'f' | 'j' | 'r' | 'I' => 3.4,
                'm' | 'w' | 'M' | 'W' => 9.6,
                _ => 6.7,
            })
            .sum()
    };
    let widths: Vec<f32> = TAB_LABELS.iter().map(|l| text_w(l) + 16.0).collect();
    let total: f32 = widths.iter().sum();
    let (left, right) = (46.0, EXP_W - 46.0);
    let n = TAB_LABELS.len() as f32 - 1.0;
    let gap = ((right - left - total) / n).clamp(2.0, 12.0);
    let used = total + gap * n;
    let mut x = left + (right - left - used) / 2.0;
    widths
        .iter()
        .map(|w| {
            let r = Rect::new(x, 8.0, *w, 28.0);
            x += w + gap;
            r
        })
        .collect()
}

fn collapse_rect() -> Rect {
    Rect::new(EXP_W - 40.0, 8.0, 28.0, 28.0)
}

/// (used fraction, total MiB, used MiB) from /proc/meminfo.
fn mem_info() -> Option<(f32, u64, u64)> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = |key: &str| -> Option<u64> {
        s.lines().find(|l| l.starts_with(key))?.split_whitespace().nth(1)?.parse().ok()
    };
    let (total, avail) = (kb("MemTotal:")?, kb("MemAvailable:")?);
    let used = total.saturating_sub(avail);
    Some((used as f32 / total as f32, total / 1024, used / 1024))
}

fn uptime() -> String {
    let secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0) as u64;
    format!("{}h {:02}m", secs / 3600, secs / 60 % 60)
}

#[derive(Default)]
struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    gfx: Option<Gfx>,
    expanded: bool,
    page: usize,
    hover: Option<usize>,
    /// pointer in logical (DIP) coordinates
    cursor: (f32, f32),
}

impl App {
    fn size(&self) -> (f32, f32) {
        if self.expanded { (EXP_W, EXP_H) } else { (PILL_W, PILL_H) }
    }

    /// Resize to the pill or the panel and keep the window centred on the top edge.
    fn apply_size(&mut self) {
        let Some(win) = &self.window else { return };
        let (w, h) = self.size();
        let _ = win.request_inner_size(LogicalSize::new(w as f64, h as f64));
        if let Some(m) = win.current_monitor() {
            let (mp, ms) = (m.position(), m.size());
            let pw = (w * win.scale_factor() as f32) as i32;
            win.set_outer_position(PhysicalPosition::new(mp.x + (ms.width as i32 - pw) / 2, mp.y));
        }
        win.request_redraw();
    }

    fn click(&mut self) {
        let (x, y) = self.cursor;
        if !self.expanded {
            self.expanded = true;
        } else if collapse_rect().contains(x, y) {
            self.expanded = false;
        } else if let Some(i) = tab_rects().iter().position(|r| r.contains(x, y)) {
            self.page = i;
        } else {
            return;
        }
        self.hover = None;
        self.apply_size();
    }

    fn draw(&mut self) {
        let (w, h) = self.size();
        let (expanded, page, hover) = (self.expanded, self.page, self.hover);
        let (Some(win), Some(surface)) = (&self.window, &mut self.surface) else { return };
        let scale = win.scale_factor() as f32;
        let (pw, ph) = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
        if self.gfx.is_none() {
            // sized once for the biggest state; each frame only uses the top-left part
            self.gfx = Gfx::new((EXP_W * scale).ceil() as i32, (EXP_H * scale).ceil() as i32, scale).ok();
        }
        let Some(g) = &self.gfx else { return };
        let _ = g.begin(pw as i32, ph as i32);
        if expanded {
            draw_panel(g, page, hover);
        } else {
            g.fill_rrect(Rect::new(0.0, -20.0, w, h + 20.0), 17.0, Color::rgb(0, 0, 0));
            g.text("Nock", Rect::new(0.0, 0.0, w, h), &g.fonts.menu, Color::white(0.9), Align::Center);
        }
        let _ = g.end();

        let (gw, _gh, rgba) = g.pixels();
        let (Some(nw), Some(nh)) = (NonZeroU32::new(pw), NonZeroU32::new(ph)) else { return };
        if surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        for y in 0..nh.get() {
            for x in 0..nw.get() {
                let s = ((y * gw + x) * 4) as usize;
                let (r, gr, b, a) = (rgba[s] as u32, rgba[s + 1] as u32, rgba[s + 2] as u32, rgba[s + 3] as u32);
                buf[(y * nw.get() + x) as usize] = a << 24 | r << 16 | gr << 8 | b;
            }
        }
        let _ = buf.present();
    }

}

fn draw_panel(g: &Gfx, page: usize, hover: Option<usize>) {
        let f = &g.fonts;
        g.fill_rrect(Rect::new(0.0, -24.0, EXP_W, EXP_H + 24.0), 24.0, Color::rgb(0, 0, 0));

        let chev = collapse_rect();
        g.icon('\u{2303}', chev, &f.icon14, T3);

        let rects = tab_rects();
        g.fill_rrect(rects[page], 14.0, ACCENT.fade(0.20));
        for (i, r) in rects.iter().enumerate() {
            let hot = hover == Some(i);
            if i != page && hot {
                g.fill_rrect(*r, 14.0, Color::white(0.07));
            }
            let col = if i == page { ACCENT } else if hot { T2 } else { T3 };
            g.text(TAB_LABELS[i], *r, &f.small, col, Align::Center);
        }

        g.shifted(0.0, CONTENT_Y, || match TAB_LABELS[page] {
            "Stats" => page_stats(g),
            name => {
                let r = Rect::new(0.0, 0.0, EXP_W, EXP_H - CONTENT_Y);
                g.text(&format!("{name} is not ported to Linux yet"), r, &f.body, T3, Align::Center);
            }
        });
}

fn page_stats(g: &Gfx) {
        let f = &g.fonts;
        let tile = |x: f32, label: &str, value: &str, frac: Option<f32>| {
            let r = Rect::new(x, 20.0, 170.0, 96.0);
            g.fill_rrect(r, 14.0, Color::white(0.09));
            g.text(label, Rect::new(r.x + 14.0, r.y + 10.0, r.w - 28.0, 16.0), &f.tiny, T2, Align::Left);
            g.text(value, Rect::new(r.x + 14.0, r.y + 30.0, r.w - 28.0, 36.0), &f.big, T1, Align::Left);
            if let Some(k) = frac {
                let bar = Rect::new(r.x + 14.0, r.y + r.h - 18.0, r.w - 28.0, 5.0);
                g.fill_rrect(bar, 2.5, Color::white(0.12));
                g.fill_rrect(Rect::new(bar.x, bar.y, bar.w * k.clamp(0.0, 1.0), bar.h), 2.5, ACCENT);
            }
        };
        let m = mem_info();
        tile(24.0, "MEMORY", &m.map_or("n/a".into(), |m| format!("{:.0}%", m.0 * 100.0)), m.map(|m| m.0));
        tile(
            215.0,
            "MEMORY USED",
            &m.map_or("n/a".into(), |m| format!("{:.1} GB", m.2 as f32 / 1024.0)),
            None,
        );
        tile(406.0, "UPTIME", &uptime(), None);
    }

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title("Nock")
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_x11_window_type(vec![WindowType::Dock])
            .with_inner_size(LogicalSize::new(PILL_W as f64, PILL_H as f64));
        let Ok(win) = el.create_window(attrs) else { return };
        let win = Rc::new(win);
        if let Ok(ctx) = softbuffer::Context::new(win.clone()) {
            self.surface = softbuffer::Surface::new(&ctx, win.clone()).ok();
        }
        self.window = Some(win);
        self.apply_size();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => {
                let k = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
                self.cursor = (position.x as f32 / k, position.y as f32 / k);
                let hover = if self.expanded {
                    tab_rects().iter().position(|r| r.contains(self.cursor.0, self.cursor.1))
                } else {
                    None
                };
                if hover != self.hover {
                    self.hover = hover;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.click(),
            _ => {}
        }
    }
}

pub fn run() {
    // ponytail: X11 only (Xwayland on Wayland desktops); native Wayland needs layer-shell
    let el = EventLoop::builder().with_x11().build().expect("event loop");
    if let Err(e) = el.run_app(&mut App::default()) {
        eprintln!("nock: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders each page to PNGs for eyeballing (NOCK_PREVIEW_DIR=dir cargo test -- --ignored).
    #[test]
    #[ignore]
    fn preview_pages() {
        let dir = std::env::var("NOCK_PREVIEW_DIR").expect("NOCK_PREVIEW_DIR");
        let g = Gfx::new(EXP_W as i32, EXP_H as i32, 1.0).unwrap();
        for page in [0, 3] {
            let _ = g.begin(EXP_W as i32, EXP_H as i32);
            draw_panel(&g, page, Some(1));
            let _ = g.end();
            g.save_png(&format!("{dir}/page{page}.png"));
        }
    }
}
