//! Dropdown panels under the top-bar icons (Wi-Fi, Bluetooth, volume, battery,
//! calendar), like the macOS menu-bar popovers. One small layered window that is
//! shown only while a dropdown is open. Closes when the pointer leaves it, when
//! you click into another window, or when you click its icon again.

use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::*;
use crate::bar::{theme, BarAct, BAR_H};
use crate::gfx::*;
use crate::sys;

const POP_W: f32 = 300.0;
const POP_MAX_H: f32 = 340.0;
const GAP: f32 = 4.0;
const WM_MOUSELEAVE: u32 = 0x02A3;
pub const T_POPLEAVE: usize = 7;

#[derive(Clone, Copy)]
enum PopAct {
    Toggle(u8), // 0 = Wi-Fi, 1 = Bluetooth
    Mute,
    Vol(Rect),
    Bri(Rect),
    Wifi(usize),
    Bt(usize),
    Link(&'static str),
}

pub struct Pop {
    pub hwnd: HWND,
    gfx: Gfx,
    pub kind: Option<BarAct>,
    anchor_px: i32,
    hits: Vec<(Rect, PopAct)>,
    hot: Option<usize>,
    tracking: bool,
    drag: Option<Rect>,
    h: f32,
}

impl Pop {
    pub fn new(hwnd: HWND, scale: f32) -> Option<Pop> {
        let gfx = Gfx::new(
            ((POP_W + 2.0) * scale).ceil() as i32,
            ((POP_MAX_H + 2.0) * scale).ceil() as i32,
            scale,
        )
        .ok()?;
        Some(Pop {
            hwnd,
            gfx,
            kind: None,
            anchor_px: 0,
            hits: Vec::new(),
            hot: None,
            tracking: false,
            drag: None,
            h: 100.0,
        })
    }
}

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September",
    "October", "November", "December",
];
const DAYS_LONG: [&str; 7] = [
    "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
];

impl App {
    pub fn is_pop(&self, h: HWND) -> bool {
        self.pop.as_ref().map(|p| p.hwnd == h).unwrap_or(false)
    }

    pub fn rebuild_pop(&mut self) {
        let Some(old) = self.pop.take() else { return };
        let hwnd = old.hwnd;
        self.pop = Pop::new(hwnd, self.scale);
        if let Some(p) = self.pop.as_mut() {
            p.kind = None;
        }
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }

    pub fn pop_kind(&self) -> Option<BarAct> {
        self.pop.as_ref().and_then(|p| p.kind)
    }

    /// Open the dropdown for `kind` under `anchor_px` (screen x of the icon's centre);
    /// opening the one that is already open closes it.
    pub fn open_pop(&mut self, kind: BarAct, anchor_px: i32) {
        if self.pop_kind() == Some(kind) {
            self.close_pop();
            return;
        }
        if self.expanded && !self.pinned {
            self.set_expanded(false);
        }
        match kind {
            BarAct::Wifi => sys::wifi_scan_async(self.hwnd.0 as isize),
            BarAct::Bluetooth => {
                let _ = self.radio.tx.send(crate::radio::Req::Devices);
            }
            BarAct::Volume => self.read_volume(),
            BarAct::Brightness => sys::brightness_load(self.hwnd.0 as isize),
            BarAct::Battery | BarAct::Clock => {}
        }
        let _ = self.radio.tx.send(crate::radio::Req::Refresh);
        let Some(p) = self.pop.as_mut() else { return };
        p.kind = Some(kind);
        p.anchor_px = anchor_px;
        p.hot = None;
        p.drag = None;
        self.render_pop();
        unsafe {
            if let Some(p) = &self.pop {
                let _ = ShowWindow(p.hwnd, SW_SHOWNOACTIVATE);
            }
        }
        self.raise_windows();
        self.set_timer(T_POPLEAVE, 2500);
        self.render_bar();
    }

    pub fn close_pop(&mut self) {
        let Some(p) = self.pop.as_mut() else { return };
        if p.kind.is_none() {
            return;
        }
        p.kind = None;
        p.drag = None;
        unsafe {
            if GetCapture() == p.hwnd {
                let _ = ReleaseCapture();
            }
            let _ = ShowWindow(p.hwnd, SW_HIDE);
        }
        self.kill_timer(T_POPLEAVE);
        self.render_bar();
    }

    /// Data for the open dropdown changed (Wi-Fi name, device list, radio state).
    pub fn on_pop_data(&mut self) {
        if self.pop_kind().is_some() {
            self.render_pop();
        }
    }

    pub fn on_pop_leave_timer(&mut self) {
        self.kill_timer(T_POPLEAVE);
        let Some(p) = self.pop.as_ref() else { return };
        if p.kind.is_none() || p.drag.is_some() {
            return;
        }
        let inside = unsafe {
            let mut pt = POINT::default();
            let mut r = RECT::default();
            GetCursorPos(&mut pt).is_ok()
                && GetWindowRect(p.hwnd, &mut r).is_ok()
                && pt.x >= r.left
                && pt.x < r.right
                && pt.y >= r.top
                && pt.y < r.bottom
        };
        if !inside {
            self.close_pop();
        }
    }

    // ------------------------------------------------------------ drawing --

    pub fn render_pop(&mut self) {
        let scale = self.scale;
        let light = self.bar.as_ref().map(|b| b.light).unwrap_or(false);
        let th = theme(light);
        let kind = match self.pop_kind() {
            Some(k) => k,
            None => return,
        };
        let radios = self.radios;
        let wifi = sys::WIFI.lock().unwrap().clone();
        let devices = self.radio.devices.lock().unwrap().clone();
        let (vol, muted) = (self.volume, self.muted);
        let bri = self.brightness;
        let accent = self.accent;
        let binfo = sys::battery_info();
        let (year, month, day, wday, hh, mm, ..) = sys::local_time();
        let clock24 = self.cfg.clock_24h;
        let mon = self.mon;
        let Some(p) = self.pop.as_mut() else { return };
        let g = &p.gfx;
        let w = POP_W;
        let surf = if light {
            Color(0.97, 0.97, 0.98, 0.95)
        } else {
            Color(0.11, 0.11, 0.12, 0.94)
        };
        let mut hits: Vec<(Rect, PopAct)> = Vec::new();

        // Rows are laid out first so the window height can match the content.
        let mut ops: Vec<Box<dyn Fn(&Gfx, &mut Vec<(Rect, PopAct)>, Option<usize>)>> = Vec::new();
        let mut y = 14.0f32;

        let title = |t: &'static str, y: f32| -> Box<dyn Fn(&Gfx, &mut Vec<(Rect, PopAct)>, Option<usize>)> {
            Box::new(move |g, _h, _| {
                g.text(t, Rect::new(16.0, y, 200.0, 24.0), &g.fonts.menu, th.t1, Align::Left);
            })
        };
        let line = |y: f32| -> Box<dyn Fn(&Gfx, &mut Vec<(Rect, PopAct)>, Option<usize>)> {
            Box::new(move |g, _h, _| {
                g.fill_rrect(Rect::new(16.0, y, w - 32.0, 1.0), 0.0, th.line);
            })
        };
        let note = |t: String, y: f32, c: Color| -> Box<dyn Fn(&Gfx, &mut Vec<(Rect, PopAct)>, Option<usize>)> {
            Box::new(move |g, _h, _| {
                g.text(&t, Rect::new(16.0, y, w - 32.0, 22.0), &g.fonts.small, c, Align::Left);
            })
        };
        let link = |t: &'static str, url: &'static str, y: f32| -> Box<dyn Fn(&Gfx, &mut Vec<(Rect, PopAct)>, Option<usize>)> {
            Box::new(move |g, h, hot| {
                let r = Rect::new(8.0, y, w - 16.0, 30.0);
                let idx = h.len();
                if hot == Some(idx) {
                    g.fill_rrect(r, 7.0, th.hover);
                }
                g.text(t, Rect::new(16.0, y, w - 32.0, 30.0), &g.fonts.small, th.t1, Align::Left);
                h.push((r, PopAct::Link(url)));
            })
        };
        let switch = |on: bool, which: u8, y: f32| -> Box<dyn Fn(&Gfx, &mut Vec<(Rect, PopAct)>, Option<usize>)> {
            Box::new(move |g, h, _| {
                let r = Rect::new(w - 16.0 - 40.0, y + 1.0, 40.0, 22.0);
                let track = if on { accent } else { th.hover };
                g.fill_rrect(r, 11.0, track);
                let kx = if on { r.x + r.w - 11.0 } else { r.x + 11.0 };
                g.fill_circle(kx, r.cy(), 9.0, Color::white(1.0));
                h.push((Rect::new(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0), PopAct::Toggle(which)));
            })
        };

        match kind {
            BarAct::Wifi => {
                ops.push(title("Wi-Fi", y));
                ops.push(switch(radios.wifi == Some(true), 0, y));
                y += 36.0;
                let status = match radios.wifi {
                    None => "Wi-Fi isn't available on this PC".to_string(),
                    Some(false) => "Off".to_string(),
                    Some(true) => match (&wifi.busy, &wifi.connected) {
                        (Some(n), _) => format!("Connecting to {}\u{2026}", n),
                        (None, Some(n)) => format!("Connected to {}", n),
                        (None, None) => "Not connected".to_string(),
                    },
                };
                ops.push(note(status, y, th.t2));
                y += 30.0;
                if radios.wifi == Some(true) {
                    if wifi.nets.is_empty() {
                        ops.push(note("Looking for networks\u{2026}".into(), y, th.t2));
                        y += 28.0;
                    }
                    for (i, n) in wifi.nets.iter().enumerate() {
                        let is_conn = wifi.connected.as_deref() == Some(n.ssid.as_str());
                        let (name, secure, signal) = (n.ssid.clone(), n.secure, n.signal);
                        let yy = y;
                        ops.push(Box::new(move |g, h, hot| {
                            let r = Rect::new(8.0, yy, w - 16.0, 28.0);
                            let idx = h.len();
                            if hot == Some(idx) {
                                g.fill_rrect(r, 7.0, th.hover);
                            }
                            if is_conn {
                                g.fill_circle(22.0, yy + 14.0, 3.5, accent);
                            }
                            g.text(&name, Rect::new(34.0, yy, w - 120.0, 28.0), &g.fonts.small, th.t1, Align::Left);
                            if secure {
                                g.icon('\u{E72E}', Rect::new(w - 84.0, yy, 20.0, 28.0), &g.fonts.icon14, th.t2);
                            }
                            g.text(&format!("{}%", signal), Rect::new(w - 62.0, yy, 46.0, 28.0), &g.fonts.tiny, th.t2, Align::Right);
                            h.push((r, PopAct::Wifi(i)));
                        }));
                        y += 28.0;
                    }
                    y += 2.0;
                }
                ops.push(line(y));
                y += 6.0;
                ops.push(link("Wi-Fi settings\u{2026}", "ms-settings:network-wifi", y));
                y += 36.0;
            }
            BarAct::Bluetooth => {
                ops.push(title("Bluetooth", y));
                ops.push(switch(radios.bluetooth == Some(true), 1, y));
                y += 36.0;
                if radios.bluetooth == Some(true) {
                    if devices.is_empty() {
                        ops.push(note("No paired devices".into(), y, th.t2));
                        y += 28.0;
                    }
                    for (i, d) in devices.iter().enumerate() {
                        let (name, connected) = (d.name.clone(), d.connected);
                        let yy = y;
                        ops.push(Box::new(move |g, h, hot| {
                            let r = Rect::new(8.0, yy, w - 16.0, 28.0);
                            let idx = h.len();
                            let hovered = hot == Some(idx);
                            if hovered {
                                g.fill_rrect(r, 7.0, th.hover);
                            }
                            if connected {
                                g.fill_circle(22.0, yy + 14.0, 3.5, accent);
                            }
                            g.text(&name, Rect::new(34.0, yy, w - 130.0, 28.0), &g.fonts.small, th.t1, Align::Left);
                            let label = match (connected, hovered) {
                                (true, true) => "Disconnect",
                                (true, false) => "Connected",
                                (false, true) => "Connect",
                                (false, false) => "",
                            };
                            let c = if connected && !hovered { accent } else { th.t2 };
                            g.text(label, Rect::new(w - 106.0, yy, 90.0, 28.0), &g.fonts.tiny, c, Align::Right);
                            h.push((r, PopAct::Bt(i)));
                        }));
                        y += 28.0;
                    }
                    y += 2.0;
                } else {
                    let t = if radios.bluetooth.is_none() { "Bluetooth isn't available on this PC" } else { "Off" };
                    ops.push(note(t.into(), y, th.t2));
                    y += 28.0;
                }
                ops.push(line(y));
                y += 6.0;
                ops.push(link("Bluetooth settings\u{2026}", "ms-settings:bluetooth", y));
                y += 36.0;
            }
            BarAct::Volume => {
                ops.push(title("Sound", y));
                let mr = Rect::new(w - 16.0 - 28.0, y - 2.0, 28.0, 28.0);
                ops.push(Box::new(move |g, h, hot| {
                    let idx = h.len();
                    if hot == Some(idx) {
                        g.fill_rrect(mr, 7.0, th.hover);
                    }
                    let glyph = if muted { '\u{E74F}' } else { '\u{E767}' };
                    g.icon(glyph, mr, &g.fonts.icon14, th.t1);
                    h.push((mr, PopAct::Mute));
                }));
                y += 40.0;
                let sr = Rect::new(20.0, y, w - 40.0, 24.0);
                let frac = if muted { 0.0 } else { vol };
                ops.push(Box::new(move |g, h, hot| {
                    let track = Rect::new(sr.x, sr.cy() - 3.0, sr.w, 6.0);
                    g.fill_rrect(track, 3.0, th.hover);
                    let fw = (sr.w * frac.clamp(0.0, 1.0)).max(0.0);
                    if fw > 0.5 {
                        g.fill_rrect(Rect::new(sr.x, track.y, fw, 6.0), 3.0, accent);
                    }
                    let idx = h.len();
                    g.fill_circle(sr.x + fw, sr.cy(), if hot == Some(idx) { 8.0 } else { 7.0 }, Color::white(1.0));
                    h.push((Rect::new(sr.x - 8.0, sr.y, sr.w + 16.0, sr.h), PopAct::Vol(sr)));
                }));
                y += 36.0;
                ops.push(note(format!("{}%", (frac * 100.0).round() as i32), y - 4.0, th.t2));
                y += 22.0;
            }
            BarAct::Brightness => {
                ops.push(title("Display", y));
                y += 40.0;
                let sr = Rect::new(20.0, y, w - 40.0, 24.0);
                let frac = bri.map(|b| b as f32 / 100.0).unwrap_or(0.5);
                let ok = true;
                ops.push(Box::new(move |g, h, hot| {
                    let track = Rect::new(sr.x, sr.cy() - 3.0, sr.w, 6.0);
                    g.fill_rrect(track, 3.0, th.hover);
                    let fw = (sr.w * frac.clamp(0.0, 1.0)).max(0.0);
                    if ok {
                        if fw > 0.5 {
                            g.fill_rrect(Rect::new(sr.x, track.y, fw, 6.0), 3.0, accent);
                        }
                        let idx = h.len();
                        g.fill_circle(sr.x + fw, sr.cy(), if hot == Some(idx) { 8.0 } else { 7.0 }, Color::white(1.0));
                        h.push((Rect::new(sr.x - 8.0, sr.y, sr.w + 16.0, sr.h), PopAct::Bri(sr)));
                    }
                }));
                y += 36.0;
                let t = match bri {
                    Some(b) => format!("Brightness {}%", b),
                    None => "Drag to set brightness".to_string(),
                };
                ops.push(note(t, y - 4.0, th.t2));
                y += 22.0;
            }
            BarAct::Battery => {
                ops.push(title("Battery", y));
                y += 34.0;
                match binfo {
                    Some(b) => {
                        ops.push(Box::new({
                            let pct = b.percent;
                            move |g, _h, _| {
                                g.text(&format!("{}%", pct), Rect::new(16.0, y, 120.0, 44.0), &g.fonts.big, th.t1, Align::Left);
                            }
                        }));
                        let status = if b.charging {
                            "Charging"
                        } else if b.plugged {
                            "Plugged in, not charging"
                        } else {
                            "On battery"
                        };
                        let mut st = status.to_string();
                        if let (false, Some(s)) = (b.plugged, b.secs_left) {
                            let (h, m) = (s / 3600, (s % 3600) / 60);
                            st = if h > 0 {
                                format!("{} \u{00B7} {} h {} min left", status, h, m)
                            } else {
                                format!("{} \u{00B7} {} min left", status, m)
                            };
                        }
                        if b.saver {
                            st.push_str(" \u{00B7} Battery saver on");
                        }
                        ops.push(note(st, y + 46.0, th.t2));
                        let frac = b.percent as f32 / 100.0;
                        let by = y + 74.0;
                        let col = if b.charging || b.plugged {
                            Color(0.45, 0.85, 0.5, 1.0)
                        } else if b.percent <= 20 {
                            Color(1.0, 0.45, 0.4, 1.0)
                        } else {
                            accent
                        };
                        ops.push(Box::new(move |g, _h, _| {
                            g.fill_rrect(Rect::new(16.0, by, w - 32.0, 6.0), 3.0, th.hover);
                            g.fill_rrect(Rect::new(16.0, by, (w - 32.0) * frac, 6.0), 3.0, col);
                        }));
                        y += 92.0;
                    }
                    None => {
                        ops.push(note("No battery detected".into(), y, th.t2));
                        y += 30.0;
                    }
                }
                ops.push(line(y));
                y += 6.0;
                ops.push(link("Battery settings\u{2026}", "ms-settings:batterysaver", y));
                y += 36.0;
            }
            BarAct::Clock => {
                ops.push(Box::new({
                    let t = format!("{}, {} {}", DAYS_LONG[(wday % 7) as usize], day, MONTHS[(month.max(1) as usize - 1) % 12]);
                    move |g, _h, _| {
                        g.text(&t, Rect::new(16.0, 14.0, w - 32.0, 24.0), &g.fonts.menu, th.t1, Align::Left);
                    }
                }));
                let tm = if clock24 {
                    format!("{:02}:{:02}", hh, mm)
                } else {
                    format!("{}:{:02} {}", if hh % 12 == 0 { 12 } else { hh % 12 }, mm, if hh < 12 { "AM" } else { "PM" })
                };
                ops.push(Box::new(move |g, _h, _| {
                    g.text(&tm, Rect::new(w - 116.0, 14.0, 100.0, 24.0), &g.fonts.small, th.t2, Align::Right);
                }));
                y += 40.0;
                let first = crate::ui::first_weekday(day, wday);
                let dim = crate::ui::days_in_month(year, month);
                let rows = (first + dim + 6) / 7;
                let cw = (w - 32.0) / 7.0;
                let top = y;
                ops.push(Box::new(move |g, _h, _| {
                    g.text(
                        &format!("{} {}", MONTHS[(month.max(1) as usize - 1) % 12], year),
                        Rect::new(16.0, top, w - 32.0, 20.0),
                        &g.fonts.tiny,
                        th.t2,
                        Align::Left,
                    );
                    for (i, d) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
                        g.text(d, Rect::new(16.0 + i as f32 * cw, top + 22.0, cw, 18.0), &g.fonts.tiny, th.t2, Align::Center);
                    }
                    for d in 1..=dim {
                        let slot = first + d - 1;
                        let (c, r) = (slot % 7, slot / 7);
                        let cell = Rect::new(16.0 + c as f32 * cw, top + 42.0 + r as f32 * 28.0, cw, 28.0);
                        if d == day {
                            g.fill_circle(cell.cx(), cell.cy(), 12.0, accent);
                            g.text(&d.to_string(), cell, &g.fonts.small, Color(0.0, 0.0, 0.0, 1.0), Align::Center);
                        } else {
                            g.text(&d.to_string(), cell, &g.fonts.small, th.t1, Align::Center);
                        }
                    }
                }));
                y += 46.0 + rows as f32 * 28.0 + 4.0;
                ops.push(line(y));
                y += 6.0;
                ops.push(link("Date & time settings\u{2026}", "ms-settings:dateandtime", y));
                y += 36.0;
            }
        }
        let h = (y + 4.0).min(POP_MAX_H);
        p.h = h;

        let (pw, ph) = ((w * scale).round() as i32, (h * scale).round() as i32);
        if g.begin(pw, ph).is_err() {
            return;
        }
        g.fill_rrect(Rect::new(0.5, 0.5, w - 1.0, h - 1.0), 12.0, surf);
        g.stroke_rrect(Rect::new(0.5, 0.5, w - 1.0, h - 1.0), 12.0, th.line, 1.0);
        let hot = p.hot;
        for op in &ops {
            op(g, &mut hits, hot);
        }
        let _ = g.end();
        p.hits = hits;

        let half = pw / 2;
        let x = (p.anchor_px - half).clamp(mon.left + 8, (mon.right - pw - 8).max(mon.left + 8));
        let yy = mon.top + ((BAR_H + GAP) * scale).round() as i32;
        let _ = g.present(p.hwnd, x, yy, pw, ph);
    }

    // -------------------------------------------------------------- input --

    pub fn pop_msg(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let lox = (lp.0 & 0xffff) as i16 as i32;
        let hix = ((lp.0 >> 16) & 0xffff) as i16 as i32;
        let s = self.scale;
        let (x, y) = (lox as f32 / s, hix as f32 / s);
        match msg {
            WM_MOUSEMOVE => {
                self.kill_timer(T_POPLEAVE);
                let mut redraw = false;
                let mut vol_drag = None;
                if let Some(p) = self.pop.as_mut() {
                    if !p.tracking {
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        unsafe {
                            if TrackMouseEvent(&mut t).is_ok() {
                                p.tracking = true;
                            }
                        }
                    }
                    if let Some(r) = p.drag {
                        vol_drag = Some(r);
                    } else {
                        let hot = p.hits.iter().position(|(r, _)| r.contains(x, y));
                        if hot != p.hot {
                            p.hot = hot;
                            redraw = true;
                        }
                    }
                }
                if let Some(r) = vol_drag {
                    if self.pop_kind() == Some(BarAct::Brightness) {
                        let v = ((x - r.x) / r.w).clamp(0.0, 1.0);
                        let pct = (v * 100.0).round() as u32;
                        self.brightness = Some(pct);
                        sys::brightness_set(pct); // coalesced, so cheap to call per move
                    } else {
                        self.set_vol_from(r, x);
                    }
                    redraw = true;
                }
                if redraw {
                    self.render_pop();
                }
                LRESULT(0)
            }
            m if m == WM_MOUSELEAVE => {
                if let Some(p) = self.pop.as_mut() {
                    p.tracking = false;
                    p.hot = None;
                }
                let dragging = self.pop.as_ref().map(|p| p.drag.is_some()).unwrap_or(false);
                if !dragging {
                    self.set_timer(T_POPLEAVE, 500);
                    self.render_pop();
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let act = self
                    .pop
                    .as_ref()
                    .and_then(|p| p.hits.iter().find(|(r, _)| r.contains(x, y)).map(|h| h.1));
                if let Some(PopAct::Vol(r)) | Some(PopAct::Bri(r)) = act {
                    if let Some(p) = self.pop.as_mut() {
                        p.drag = Some(r);
                    }
                    unsafe {
                        SetCapture(hwnd);
                    }
                    if matches!(act, Some(PopAct::Bri(_))) {
                        let v = ((x - r.x) / r.w).clamp(0.0, 1.0);
                        self.brightness = Some((v * 100.0).round() as u32);
                    } else {
                        self.set_vol_from(r, x);
                    }
                    self.render_pop();
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let dragging = self.pop.as_ref().map(|p| p.drag.is_some()).unwrap_or(false);
                if dragging {
                    if let Some(p) = self.pop.as_mut() {
                        p.drag = None;
                    }
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    if self.pop_kind() == Some(BarAct::Brightness) {
                        if let Some(b) = self.brightness {
                            sys::brightness_set(b); // slow call, so only on release
                        }
                    }
                    self.render_bar();
                    return LRESULT(0);
                }
                let act = self
                    .pop
                    .as_ref()
                    .and_then(|p| p.hits.iter().find(|(r, _)| r.contains(x, y)).map(|h| h.1));
                match act {
                    Some(PopAct::Toggle(i)) => {
                        self.do_act(Act::Quick(i), 0.0);
                        self.render_pop();
                    }
                    Some(PopAct::Mute) => {
                        self.muted = !self.muted;
                        sys::set_mute(self.muted);
                        self.render_pop();
                        self.render_bar();
                    }
                    Some(PopAct::Wifi(i)) => {
                        let net = sys::WIFI.lock().unwrap().nets.get(i).cloned();
                        let cur = sys::WIFI.lock().unwrap().connected.clone();
                        if let Some(n) = net {
                            if cur.as_deref() == Some(n.ssid.as_str()) {
                                sys::wifi_connect_async(self.hwnd.0 as isize, None);
                            } else if n.saved {
                                sys::wifi_connect_async(self.hwnd.0 as isize, Some(n.ssid));
                            } else {
                                // needs a password: let Windows ask for it
                                sys::open("ms-settings:network-wifi", "");
                                self.close_pop();
                            }
                        }
                    }
                    Some(PopAct::Bt(i)) => {
                        let dev = self.radio.devices.lock().unwrap().get(i).cloned();
                        if let Some(d) = dev {
                            if d.address != 0 {
                                let _ = self
                                    .radio
                                    .tx
                                    .send(crate::radio::Req::BtConnect(d.address, !d.connected));
                                // show the expected result right away
                                if let Some(x) = self.radio.devices.lock().unwrap().get_mut(i) {
                                    x.connected = !d.connected;
                                }
                                self.render_pop();
                            }
                        }
                    }
                    Some(PopAct::Link(url)) => {
                        sys::open(url, "");
                        self.close_pop();
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
        }
    }

    fn set_vol_from(&mut self, r: Rect, x: f32) {
        let v = ((x - r.x) / r.w).clamp(0.0, 1.0);
        self.volume = v;
        if self.muted {
            self.muted = false;
            sys::set_mute(false);
        }
        sys::set_volume(v);
    }
}
