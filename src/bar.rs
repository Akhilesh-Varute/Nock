//! The top bar: a thin full-width strip (like the macOS menu bar) that the notch
//! hangs from. Left: name of the app you are in. Right: Wi-Fi, Bluetooth,
//! volume, battery and the date/time. Redrawn only on events (foreground change,
//! radio change, once a minute), never on a fast timer.

use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::*;
use crate::gfx::*;
use crate::sys;

pub const BAR_H: f32 = 30.0;

/// Colours for the dark and light variants. The notch itself stays black.
#[derive(Clone, Copy)]
pub(crate) struct Theme {
    pub bg: Color,
    pub line: Color,
    pub t1: Color,
    pub t2: Color,
    pub hover: Color,
}

pub(crate) fn theme(light: bool) -> Theme {
    if light {
        Theme {
            bg: Color(0.96, 0.96, 0.97, 0.66),
            line: Color(0.0, 0.0, 0.0, 0.14),
            t1: Color(0.07, 0.07, 0.08, 0.96),
            t2: Color(0.07, 0.07, 0.08, 0.50),
            hover: Color(0.0, 0.0, 0.0, 0.10),
        }
    } else {
        Theme {
            bg: Color(0.05, 0.05, 0.06, 0.62),
            line: Color(1.0, 1.0, 1.0, 0.10),
            t1: Color::white(0.96),
            t2: Color::white(0.55),
            hover: Color::white(0.16),
        }
    }
}
const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Clone, Copy, PartialEq)]
pub enum BarAct {
    Wifi,
    Bluetooth,
    Volume,
    Brightness,
    Battery,
    Clock,
}

pub struct Bar {
    pub hwnd: HWND,
    gfx: Gfx,
    hits: Vec<(Rect, BarAct)>,
    hot: Option<BarAct>,
    tracking: bool,
    pub app_name: String,
    pub battery: Option<(u8, bool)>,
    pub light: bool,
}

impl Bar {
    pub fn new(hwnd: HWND, mon_w_px: i32, scale: f32) -> Option<Bar> {
        let gfx = Gfx::new(mon_w_px.max(200), (BAR_H * scale).ceil() as i32 + 2, scale).ok()?;
        Some(Bar {
            hwnd,
            gfx,
            hits: Vec::new(),
            hot: None,
            tracking: false,
            app_name: String::new(),
            battery: None,
            light: sys::light_theme(),
        })
    }
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

impl App {
    pub fn is_bar(&self, h: HWND) -> bool {
        self.bar.as_ref().map(|b| b.hwnd == h).unwrap_or(false)
    }

    pub fn bar_visible(&self) -> bool {
        self.cfg.top_bar && !self.hidden
    }

    pub fn rebuild_bar(&mut self) {
        let Some(old) = self.bar.take() else { return };
        let w = self.mon.right - self.mon.left;
        let mut nb = Bar::new(old.hwnd, w, self.scale);
        if let Some(nb) = nb.as_mut() {
            nb.app_name = old.app_name;
            nb.battery = old.battery;
            nb.light = old.light;
        }
        self.bar = nb;
    }

    /// Show/hide the bar window to match settings, then repaint it.
    pub fn sync_bar(&mut self) {
        let vis = self.bar_visible();
        if let Some(b) = &self.bar {
            unsafe {
                let _ = ShowWindow(b.hwnd, if vis { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
        }
        if vis {
            self.refresh_bar_data();
            self.render_bar();
            self.raise_windows();
        }
    }

    /// Keep both windows topmost, the notch above the bar.
    pub fn raise_windows(&self) {
        unsafe {
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
            if let Some(b) = &self.bar {
                let _ = SetWindowPos(b.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
            }
            if let Some(p) = &self.pop {
                if p.kind.is_some() {
                    let _ = SetWindowPos(p.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
                }
            }
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
        }
    }

    /// Refresh the cheap-to-read values (called on events and once a minute).
    pub fn refresh_bar_data(&mut self) {
        self.read_volume();
        let _ = self.radio.tx.send(crate::radio::Req::Refresh);
        let batt = sys::battery();
        let light = sys::light_theme();
        let fg = sys::foreground_app_name(&[self.hwnd]);
        if let Some(b) = self.bar.as_mut() {
            b.battery = batt;
            b.light = light;
            if let Some(n) = fg {
                b.app_name = n;
            }
        }
    }

    /// Windows switched light/dark (or another system setting changed).
    pub fn on_theme_change(&mut self) {
        if !self.bar_visible() {
            return;
        }
        let light = sys::light_theme();
        let changed = self.bar.as_ref().map(|b| b.light != light).unwrap_or(false);
        if changed {
            if let Some(b) = self.bar.as_mut() {
                b.light = light;
            }
            self.render_bar();
        }
    }

    pub fn on_bar_foreground(&mut self) {
        if !self.bar_visible() {
            return;
        }
        let bh = self.bar.as_ref().map(|b| b.hwnd);
        let mut skip = vec![self.hwnd];
        skip.extend(bh);
        if let Some(n) = sys::foreground_app_name(&skip) {
            if let Some(b) = self.bar.as_mut() {
                if b.app_name == n {
                    return;
                }
                b.app_name = n;
            }
            self.render_bar();
        }
    }

    pub fn render_bar(&mut self) {
        if !self.bar_visible() {
            return;
        }
        let scale = self.scale;
        let mon_w = self.mon.right - self.mon.left;
        let (pw, ph) = (mon_w, (BAR_H * scale).round() as i32);
        let w = pw as f32 / scale;
        let (wifi, bt) = (self.radios.wifi, self.radios.bluetooth);
        let (vol, muted, has_vol) = (self.volume, self.muted, self.has_volume);
        let clock24 = self.cfg.clock_24h;
        let accent = self.accent;
        let active = self.pop_kind();
        let Some(bar) = self.bar.as_mut() else { return };
        let g = &bar.gfx;
        let f = &g.fonts;
        let is_hot = |a: BarAct| bar.hot == Some(a) || active == Some(a);
        if g.begin(pw, ph).is_err() {
            return;
        }
        let mut hits: Vec<(Rect, BarAct)> = Vec::new();
        let th = theme(bar.light);
        g.fill_rrect(Rect::new(0.0, 0.0, w, BAR_H), 0.0, th.bg);
        g.fill_rrect(Rect::new(0.0, BAR_H - 1.0, w, 1.0), 0.0, th.line);

        // left: app name (kept clear of the notch in the middle)
        let name_w = (w / 2.0 - 170.0).max(40.0);
        g.text(
            &bar.app_name,
            Rect::new(14.0, 0.0, name_w, BAR_H),
            &f.menu,
            th.t1,
            Align::Left,
        );

        // right: laid out right-to-left
        let (_, mo, day, wd, hh, mm, _, _) = sys::local_time();
        let time = if clock24 {
            format!("{:02}:{:02}", hh, mm)
        } else {
            let h12 = match hh % 12 {
                0 => 12,
                n => n,
            };
            format!("{}:{:02} {}", h12, mm, if hh < 12 { "AM" } else { "PM" })
        };
        let clock = format!(
            "{} {} {}  {}",
            DAYS[(wd % 7) as usize],
            day,
            MONTHS[((mo.max(1) - 1) % 12) as usize],
            time
        );
        let mut x = w - 10.0;
        let item_y = 4.0;
        let item_h = BAR_H - 8.0;
        let mut item = |width: f32| {
            x -= width;
            Rect::new(x, item_y, width, item_h)
        };

        let r = item(if clock24 { 128.0 } else { 148.0 });
        draw_hover(g, th.hover, r, is_hot(BarAct::Clock));
        g.text(&clock, r, &f.small, th.t1, Align::Center);
        hits.push((r, BarAct::Clock));

        if let Some((pct, charging)) = bar.battery {
            let r = item(66.0);
            draw_hover(g, th.hover, r, is_hot(BarAct::Battery));
            let col = if charging {
                Color(0.45, 0.85, 0.5, 1.0)
            } else if pct <= 20 {
                Color(1.0, 0.45, 0.4, 1.0)
            } else {
                th.t1
            };
            g.icon('\u{E83F}', Rect::new(r.x + 4.0, r.y, 20.0, r.h), &f.icon14, col);
            g.text(
                &format!("{}%", pct),
                Rect::new(r.x + 24.0, r.y, 40.0, r.h),
                &f.small,
                th.t1,
                Align::Left,
            );
            hits.push((r, BarAct::Battery));
        }

        if has_vol {
            let r = item(30.0);
            draw_hover(g, th.hover, r, is_hot(BarAct::Volume));
            let glyph = if muted || vol <= 0.001 { '\u{E74F}' } else { '\u{E767}' };
            g.icon(glyph, r, &f.icon14, th.t1);
            hits.push((r, BarAct::Volume));
        }

        {
            // always shown: the slider works even if Windows would not tell us the
            // current level (we can still set it)
            let r = item(30.0);
            draw_hover(g, th.hover, r, is_hot(BarAct::Brightness));
            g.icon('\u{E706}', r, &f.icon14, th.t1);
            hits.push((r, BarAct::Brightness));
        }

        for (state, glyph, act) in [
            (bt, '\u{E702}', BarAct::Bluetooth),
            (wifi, '\u{E701}', BarAct::Wifi),
        ] {
            if state.is_none() {
                continue;
            }
            let r = item(30.0);
            draw_hover(g, th.hover, r, is_hot(act));
            let col = if state == Some(true) { accent } else { th.t2 };
            g.icon(glyph, r, &f.icon14, col);
            hits.push((r, act));
        }

        let _ = g.end();
        bar.hits = hits;
        let _ = g.present(bar.hwnd, self.mon.left, self.mon.top, pw, ph);
    }

    // ------------------------------------------------------------- input --

    pub fn bar_msg(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let lo = (lp.0 & 0xffff) as i16 as i32;
        let hi = ((lp.0 >> 16) & 0xffff) as i16 as i32;
        match msg {
            WM_MOUSEMOVE => {
                let s = self.scale;
                let (px, py) = (lo as f32 / s, hi as f32 / s);
                let mut hot = None;
                if let Some(b) = self.bar.as_mut() {
                    hot = b.hits.iter().find(|(r, _)| r.contains(px, py)).map(|h| h.1);
                    if !b.tracking {
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        unsafe {
                            if TrackMouseEvent(&mut t).is_ok() {
                                b.tracking = true;
                            }
                        }
                    }
                }
                self.set_bar_hot(hot);
                LRESULT(0)
            }
            m if m == WM_MOUSELEAVE => {
                if let Some(b) = self.bar.as_mut() {
                    b.tracking = false;
                }
                self.set_bar_hot(None);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let s = self.scale;
                let (px, py) = (lo as f32 / s, hi as f32 / s);
                let hit = self
                    .bar
                    .as_ref()
                    .and_then(|b| b.hits.iter().find(|(r, _)| r.contains(px, py)).copied());
                if let Some((r, act)) = hit {
                    let anchor = self.mon.left + (r.cx() * self.scale).round() as i32;
                    self.bar_click(act, anchor);
                }
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
        }
    }

    fn set_bar_hot(&mut self, hot: Option<BarAct>) {
        let changed = match self.bar.as_mut() {
            Some(b) if b.hot != hot => {
                b.hot = hot;
                true
            }
            _ => false,
        };
        if changed {
            self.render_bar();
        }
    }

    fn bar_click(&mut self, act: BarAct, anchor: i32) {
        self.open_pop(act, anchor);
    }
}

fn draw_hover(g: &Gfx, c: Color, r: Rect, on: bool) {
    if on {
        g.fill_rrect(r, 6.0, c);
    }
}
