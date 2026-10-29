//! Dropdown panels under the top-bar icons (Wi-Fi, Bluetooth, volume, battery,
//! calendar). One small layered window that is
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
/// Connecting animation tick (only runs while a Wi-Fi / Bluetooth attempt is in progress).
pub const T_LINK: usize = 11;
/// Hover delay before the Wi-Fi / Bluetooth tooltip appears.
pub const T_TIP: usize = 12;
/// Clears the "Connected / Couldn't connect" message after a few seconds.
pub const T_LINKMSG: usize = 13;
const TIP_W: f32 = 250.0;

const GREEN: Color = Color(0.45, 0.85, 0.5, 1.0);
const RED: Color = Color(1.0, 0.45, 0.4, 1.0);

/// A Wi-Fi password being typed into the dropdown.
#[derive(Clone)]
pub struct PwEntry {
    pub ssid: String,
    pub auth: String,
    pub buf: Vec<u16>,
    pub show: bool,
}

/// Eight dots chasing each other, the usual "working" indicator.
fn spinner(g: &Gfx, cx: f32, cy: f32, phase: u32, c: Color) {
    for i in 0..8u32 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let behind = (phase + 8 - i % 8) % 8;
        let k = 1.0 - behind as f32 / 8.0;
        g.fill_circle(
            cx + 6.5 * a.cos(),
            cy + 6.5 * a.sin(),
            1.8,
            Color(c.0, c.1, c.2, c.3 * (0.2 + 0.8 * k)),
        );
    }
}

#[derive(Clone, Copy)]
enum PopAct {
    Toggle(u8), // 0 = Wi-Fi, 1 = Bluetooth
    Mute,
    Vol(Rect),
    Bri(usize, Rect),
    Wifi(usize),
    Bt(usize),
    Link(&'static str),
    CalMonth(i32),
    CalToday,
    PwShow,
    PwJoin,
    PwCancel,
}

pub struct Pop {
    pub hwnd: HWND,
    gfx: Gfx,
    pub kind: Option<BarAct>,
    /// Showing the small hover tooltip instead of the full dropdown.
    pub tip: bool,
    anchor_px: i32,
    hits: Vec<(Rect, PopAct)>,
    hot: Option<usize>,
    tracking: bool,
    drag: Option<Rect>,
    drag_bri_idx: Option<usize>,
    h: f32,
}

impl Pop {
    /// (width, height, stride in px, pixels) of the last drawn dropdown, for the demo assets.
    #[cfg(test)]
    pub fn demo_snapshot(&self, scale: f32) -> (i32, i32, i32, Vec<u8>) {
        let (bw, _bh, buf) = self.gfx.snapshot();
        ((POP_W * scale).round() as i32, (self.h * scale).round() as i32, bw, buf)
    }

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
            tip: false,
            anchor_px: 0,
            hits: Vec::new(),
            hot: None,
            tracking: false,
            drag: None,
            drag_bri_idx: None,
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
        self.kill_timer(T_TIP);
        if self.pop_kind() == Some(kind) && !self.pop_is_tip() {
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
        self.cal_off = 0;
        let Some(p) = self.pop.as_mut() else { return };
        p.tip = false;
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
        if self.pw.is_some() {
            self.end_pw(false);
        }
        let Some(p) = self.pop.as_mut() else { return };
        if p.kind.is_none() {
            return;
        }
        p.kind = None;
        p.tip = false;
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
        let res = sys::WIFI.lock().unwrap().last.take();
        if let Some((t, ok)) = res {
            self.notify_result(BarAct::Wifi, t, ok);
        }
        self.sync_link_timer();
        if self.pop_kind().is_some() {
            self.render_pop();
        }
    }

    pub fn pop_is_tip(&self) -> bool {
        self.pop.as_ref().map(|p| p.tip && p.kind.is_some()).unwrap_or(false)
    }

    // ------------------------------------------- connecting / results / tooltip --

    /// Start or stop the connecting animation to match what is in progress.
    pub fn sync_link_timer(&mut self) {
        let busy = self.bt_busy.is_some()
            || sys::WIFI.lock().map(|w| w.busy.is_some()).unwrap_or(false);
        if busy {
            if self.link_phase == 0 {
                self.link_phase = 1;
                self.render_bar();
            }
            self.set_timer(T_LINK, 100);
        } else if self.link_phase != 0 {
            self.kill_timer(T_LINK);
            self.link_phase = 0;
            self.render_bar();
        }
    }

    pub fn on_link_tick(&mut self) {
        self.link_phase = self.link_phase.wrapping_add(1).max(1);
        self.render_bar();
        if matches!(self.pop_kind(), Some(BarAct::Wifi) | Some(BarAct::Bluetooth)) {
            self.render_pop();
        }
    }

    /// Remember the outcome and show it: inside an open dropdown, otherwise as a
    /// small tooltip under the icon for a few seconds.
    pub fn notify_result(&mut self, kind: BarAct, text: String, ok: bool) {
        self.link_msg = Some((kind, text, ok));
        self.buddy_event(crate::buddy::BuddyEv::Result(ok));
        self.set_timer(T_LINKMSG, 4000);
        let full_open = self.pop_kind().is_some() && !self.pop_is_tip();
        if !full_open {
            if let Some(a) = self.bar_anchor(kind) {
                self.show_tip(kind, a);
            }
        }
    }

    pub fn on_linkmsg_timer(&mut self) {
        self.kill_timer(T_LINKMSG);
        self.link_msg = None;
        if self.pop_is_tip() {
            let hovering = self.bar.as_ref().and_then(|b| b.hot) == self.pop_kind();
            if hovering {
                self.render_pop();
            } else {
                self.close_pop();
            }
        } else if self.pop_kind().is_some() {
            self.render_pop();
        }
    }

    /// A Bluetooth connect/disconnect attempt finished.
    pub fn on_bt_done(&mut self, ok: bool) {
        let Some((_, on, name)) = self.bt_busy.take() else { return };
        let text = match (ok, on) {
            (true, true) => format!("Connected to {name}"),
            (true, false) => format!("Disconnected from {name}"),
            (false, true) => format!("Couldn't connect to {name}"),
            (false, false) => format!("Couldn't disconnect {name}"),
        };
        self.notify_result(BarAct::Bluetooth, text, ok);
        self.sync_link_timer();
        self.render_bar();
        if self.pop_kind().is_some() {
            self.render_pop();
        }
    }

    /// The pointer moved onto / off a bar item.
    pub fn bar_hover_changed(&mut self, hot: Option<BarAct>) {
        let full_open = self.pop_kind().is_some() && !self.pop_is_tip();
        match hot {
            Some(BarAct::Wifi) | Some(BarAct::Bluetooth) if !full_open => {
                self.set_timer(T_TIP, 350);
            }
            _ => {
                self.kill_timer(T_TIP);
                if self.pop_is_tip() {
                    self.set_timer(T_POPLEAVE, 250);
                }
            }
        }
    }

    pub fn on_tip_timer(&mut self) {
        self.kill_timer(T_TIP);
        if self.pop_kind().is_some() && !self.pop_is_tip() {
            return;
        }
        let hot = self.bar.as_ref().and_then(|b| b.hot);
        if let Some(k @ (BarAct::Wifi | BarAct::Bluetooth)) = hot {
            if let Some(a) = self.bar_anchor(k) {
                self.show_tip(k, a);
            }
        }
    }

    /// Small read-only box under an icon: what is connected right now.
    fn show_tip(&mut self, kind: BarAct, anchor_px: i32) {
        match kind {
            BarAct::Wifi => sys::wifi_status_async(self.hwnd.0 as isize),
            BarAct::Bluetooth => {
                let _ = self.radio.tx.send(crate::radio::Req::Devices);
            }
            _ => return,
        }
        let Some(p) = self.pop.as_mut() else { return };
        p.kind = Some(kind);
        p.tip = true;
        p.anchor_px = anchor_px;
        p.hot = None;
        p.drag = None;
        self.kill_timer(T_POPLEAVE);
        self.render_pop();
        unsafe {
            if let Some(p) = &self.pop {
                let _ = ShowWindow(p.hwnd, SW_SHOWNOACTIVATE);
            }
        }
        self.raise_windows();
    }

    // ------------------------------------------------------ Wi-Fi password --

    fn pop_activatable(&self, on: bool) {
        if let Some(p) = &self.pop {
            unsafe {
                let ex = GetWindowLongPtrW(p.hwnd, GWL_EXSTYLE);
                let flag = WS_EX_NOACTIVATE.0 as isize;
                let new = if on { ex & !flag } else { ex | flag };
                SetWindowLongPtrW(p.hwnd, GWL_EXSTYLE, new);
            }
        }
    }

    fn start_pw(&mut self, ssid: String, auth: String) {
        unsafe {
            self.pw_prev_fg = GetForegroundWindow();
        }
        self.pw = Some(PwEntry {
            ssid,
            auth,
            buf: Vec::new(),
            show: false,
        });
        self.pop_activatable(true);
        self.kill_timer(T_POPLEAVE);
        if let Some(p) = &self.pop {
            unsafe {
                let _ = SetForegroundWindow(p.hwnd);
            }
        }
        self.render_pop();
    }

    /// Stop typing a password. `refocus` hands the keyboard back to the previous window.
    pub fn end_pw(&mut self, refocus: bool) {
        if self.pw.take().is_none() {
            return;
        }
        self.pop_activatable(false);
        if refocus && !self.pw_prev_fg.0.is_null() {
            unsafe {
                let _ = SetForegroundWindow(self.pw_prev_fg);
            }
        }
        self.pw_prev_fg = HWND::default();
    }

    fn pw_join(&mut self) {
        let Some(pw) = self.pw.as_ref() else { return };
        let key = String::from_utf16_lossy(&pw.buf);
        let (ssid, auth) = (pw.ssid.clone(), pw.auth.clone());
        if key.chars().count() < 8 {
            self.link_msg = Some((
                BarAct::Wifi,
                "Password must be at least 8 characters".into(),
                false,
            ));
            self.set_timer(T_LINKMSG, 4000);
            self.render_pop();
            return;
        }
        self.end_pw(true);
        sys::wifi_join_new_async(self.hwnd.0 as isize, ssid, auth, Some(key));
        self.sync_link_timer();
        self.render_bar();
        self.render_pop();
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
        let hovering_icon = p.tip && self.bar.as_ref().and_then(|b| b.hot) == p.kind;
        if !inside && !hovering_icon {
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
        let tip_mode = self.pop_is_tip() && matches!(kind, BarAct::Wifi | BarAct::Bluetooth);
        let phase = self.link_phase;
        let link_msg = self.link_msg.clone();
        let bt_busy = self.bt_busy.clone();
        let pw = self.pw.clone();
        let radios = self.radios;
        let wifi = sys::WIFI.lock().unwrap().clone();
        let devices = self.radio.devices.lock().unwrap().clone();
        let (vol, muted) = (self.volume, self.muted);
        let bri = self.brightness;
        let displays = self.displays.clone();
        let accent = self.accent;
        let binfo = sys::battery_info();
        let (year, month, day, wday, hh, mm, ..) = sys::local_time();
        let clock24 = self.cfg.clock_24h;
        let cal_off = self.cal_off;
        let mon = self.mon;
        let Some(p) = self.pop.as_mut() else { return };
        let g = &p.gfx;
        let w = if tip_mode { TIP_W } else { POP_W };
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
            BarAct::Wifi if tip_mode => {
                ops.push(title("Wi-Fi", y));
                y += 28.0;
                let msg = match &link_msg {
                    Some((BarAct::Wifi, t, ok)) => Some((t.clone(), if *ok { GREEN } else { RED })),
                    _ => None,
                };
                let (txt, col) = if let Some(m) = msg {
                    m
                } else if let Some(n) = &wifi.busy {
                    (format!("Connecting to {}\u{2026}", n), th.t2)
                } else {
                    match (radios.wifi, &wifi.connected) {
                        (None, _) => ("Not available".to_string(), th.t2),
                        (Some(false), _) => ("Off".to_string(), th.t2),
                        (Some(true), Some(n)) => (format!("Connected to {}", n), th.t1),
                        (Some(true), None) => ("Not connected".to_string(), th.t2),
                    }
                };
                ops.push(note(txt, y, col));
                y += 26.0;
                if wifi.busy.is_none() {
                    if let Some(c) = &wifi.connected {
                        if let Some(n) = wifi.nets.iter().find(|n| &n.ssid == c) {
                            ops.push(note(format!("Signal {}%", n.signal), y, th.t2));
                            y += 24.0;
                        }
                    }
                }
                y += 6.0;
            }
            BarAct::Bluetooth if tip_mode => {
                ops.push(title("Bluetooth", y));
                y += 28.0;
                let msg = match &link_msg {
                    Some((BarAct::Bluetooth, t, ok)) => Some((t.clone(), if *ok { GREEN } else { RED })),
                    _ => None,
                };
                if let Some((t, c)) = msg {
                    ops.push(note(t, y, c));
                    y += 26.0;
                } else if let Some((_, on, name)) = &bt_busy {
                    let v = if *on { "Connecting to" } else { "Disconnecting" };
                    ops.push(note(format!("{} {}\u{2026}", v, name), y, th.t2));
                    y += 26.0;
                }
                if radios.bluetooth != Some(true) {
                    let t = if radios.bluetooth.is_none() { "Not available" } else { "Off" };
                    ops.push(note(t.into(), y, th.t2));
                    y += 26.0;
                } else {
                    let conn: Vec<String> = devices
                        .iter()
                        .filter(|d| d.connected)
                        .map(|d| d.name.clone())
                        .collect();
                    if conn.is_empty() {
                        ops.push(note("No devices connected".into(), y, th.t2));
                        y += 26.0;
                    }
                    for name in conn {
                        let yy = y;
                        ops.push(Box::new(move |g, _h, _| {
                            g.fill_circle(22.0, yy + 11.0, 3.5, accent);
                            g.text(&name, Rect::new(34.0, yy, w - 50.0, 22.0), &g.fonts.small, th.t1, Align::Left);
                        }));
                        y += 24.0;
                    }
                }
                y += 6.0;
            }
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
                y += 26.0;
                if let Some((BarAct::Wifi, t, ok)) = &link_msg {
                    ops.push(note(t.clone(), y, if *ok { GREEN } else { RED }));
                    y += 26.0;
                }
                y += 4.0;
                if radios.wifi == Some(true) {
                    if let Some(entry) = &pw {
                        // password form
                        let ssid = entry.ssid.clone();
                        ops.push(note(format!("Password for {}", ssid), y, th.t1));
                        y += 26.0;
                        let shown: String = if entry.show {
                            String::from_utf16_lossy(&entry.buf)
                        } else {
                            "\u{2022}".repeat(entry.buf.len())
                        };
                        let empty = entry.buf.is_empty();
                        let field = Rect::new(16.0, y, w - 32.0, 32.0);
                        ops.push(Box::new(move |g, _h, _| {
                            g.fill_rrect(field, 8.0, th.hover);
                            g.stroke_rrect(field, 8.0, accent, 1.5);
                            let (t, c) = if empty {
                                ("Enter password".to_string(), th.t2)
                            } else {
                                (format!("{}|", shown), th.t1)
                            };
                            g.text(&t, Rect::new(field.x + 10.0, field.y, field.w - 20.0, field.h), &g.fonts.small, c, Align::Left);
                        }));
                        y += 42.0;
                        let ready = entry.buf.len() >= 8;
                        let show_label = if entry.show { "Hide" } else { "Show" };
                        let by = y;
                        ops.push(Box::new(move |g, h, hot| {
                            let sr = Rect::new(12.0, by, 56.0, 28.0);
                            let i = h.len();
                            if hot == Some(i) {
                                g.fill_rrect(sr, 7.0, th.hover);
                            }
                            g.text(show_label, sr, &g.fonts.small, th.t2, Align::Center);
                            h.push((sr, PopAct::PwShow));
                            let cr = Rect::new(w - 16.0 - 80.0 - 8.0 - 72.0, by, 72.0, 28.0);
                            let i = h.len();
                            if hot == Some(i) {
                                g.fill_rrect(cr, 7.0, th.hover);
                            }
                            g.text("Cancel", cr, &g.fonts.small, th.t1, Align::Center);
                            h.push((cr, PopAct::PwCancel));
                            let jr = Rect::new(w - 16.0 - 80.0, by, 80.0, 28.0);
                            let bg = if ready { accent } else { th.hover };
                            g.fill_rrect(jr, 7.0, bg);
                            let tc = if ready { Color::white(1.0) } else { th.t2 };
                            g.text("Join", jr, &g.fonts.small, tc, Align::Center);
                            h.push((jr, PopAct::PwJoin));
                        }));
                        y += 36.0;
                    } else {
                        if wifi.nets.is_empty() {
                            ops.push(note("Looking for networks\u{2026}".into(), y, th.t2));
                            y += 28.0;
                        }
                        let busy_ssid = wifi.busy.clone();
                        for (i, n) in wifi.nets.iter().enumerate() {
                            let is_conn = wifi.connected.as_deref() == Some(n.ssid.as_str());
                            let is_busy = busy_ssid.as_deref() == Some(n.ssid.as_str());
                            let dim = busy_ssid.is_some() && !is_busy;
                            let (name, secure, signal) = (n.ssid.clone(), n.secure, n.signal);
                            let yy = y;
                            ops.push(Box::new(move |g, h, hot| {
                                let r = Rect::new(8.0, yy, w - 16.0, 28.0);
                                let idx = h.len();
                                if hot == Some(idx) && !dim {
                                    g.fill_rrect(r, 7.0, th.hover);
                                }
                                if is_conn && !is_busy {
                                    g.fill_circle(22.0, yy + 14.0, 3.5, accent);
                                }
                                let nc = if dim { th.t2 } else { th.t1 };
                                g.text(&name, Rect::new(34.0, yy, w - 120.0, 28.0), &g.fonts.small, nc, Align::Left);
                                if is_busy {
                                    spinner(g, w - 30.0, yy + 14.0, phase, accent);
                                } else {
                                    if secure {
                                        g.icon('\u{E72E}', Rect::new(w - 84.0, yy, 20.0, 28.0), &g.fonts.icon14, th.t2);
                                    }
                                    g.text(&format!("{}%", signal), Rect::new(w - 62.0, yy, 46.0, 28.0), &g.fonts.tiny, th.t2, Align::Right);
                                }
                                if !dim && !is_busy {
                                    h.push((r, PopAct::Wifi(i)));
                                }
                            }));
                            y += 28.0;
                        }
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
                if let Some((BarAct::Bluetooth, t, ok)) = &link_msg {
                    ops.push(note(t.clone(), y - 4.0, if *ok { GREEN } else { RED }));
                    y += 22.0;
                }
                if radios.bluetooth == Some(true) {
                    if devices.is_empty() {
                        ops.push(note("No paired devices".into(), y, th.t2));
                        y += 28.0;
                    }
                    let busy_addr = bt_busy.as_ref().map(|b| (b.0, b.1));
                    for (i, d) in devices.iter().enumerate() {
                        let (name, connected, addr) = (d.name.clone(), d.connected, d.address);
                        let is_busy = busy_addr.map(|b| b.0) == Some(addr);
                        let connecting = busy_addr.map(|b| b.1).unwrap_or(true);
                        let dim = busy_addr.is_some() && !is_busy;
                        let yy = y;
                        ops.push(Box::new(move |g, h, hot| {
                            let r = Rect::new(8.0, yy, w - 16.0, 28.0);
                            let idx = h.len();
                            let hovered = hot == Some(idx) && !dim;
                            if hovered {
                                g.fill_rrect(r, 7.0, th.hover);
                            }
                            if connected && !is_busy {
                                g.fill_circle(22.0, yy + 14.0, 3.5, accent);
                            }
                            let nc = if dim { th.t2 } else { th.t1 };
                            g.text(&name, Rect::new(34.0, yy, w - 150.0, 28.0), &g.fonts.small, nc, Align::Left);
                            if is_busy {
                                let t = if connecting { "Connecting\u{2026}" } else { "Disconnecting\u{2026}" };
                                g.text(t, Rect::new(w - 150.0, yy, 100.0, 28.0), &g.fonts.tiny, th.t2, Align::Right);
                                spinner(g, w - 30.0, yy + 14.0, phase, accent);
                            } else {
                                let label = match (connected, hovered) {
                                    (true, true) => "Disconnect",
                                    (true, false) => "Connected",
                                    (false, true) => "Connect",
                                    (false, false) => "",
                                };
                                let c = if connected && !hovered { accent } else { th.t2 };
                                g.text(label, Rect::new(w - 106.0, yy, 90.0, 28.0), &g.fonts.tiny, c, Align::Right);
                            }
                            if !dim && !is_busy {
                                h.push((r, PopAct::Bt(i)));
                            }
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
                y += 34.0;
                if displays.len() > 1 {
                    for (i, d) in displays.iter().enumerate() {
                        let disp_name = d.name.clone();
                        let b_val = d.brightness;
                        ops.push(Box::new(move |g, _h, _| {
                            g.text(
                                &format!("{} ({}%)", disp_name, b_val),
                                Rect::new(20.0, y, w - 40.0, 18.0),
                                &g.fonts.small,
                                th.t2,
                                Align::Left,
                            );
                        }));
                        y += 20.0;
                        let sr = Rect::new(20.0, y, w - 40.0, 24.0);
                        let frac = b_val as f32 / 100.0;
                        ops.push(Box::new(move |g, h, hot| {
                            let track = Rect::new(sr.x, sr.cy() - 3.0, sr.w, 6.0);
                            g.fill_rrect(track, 3.0, th.hover);
                            let fw = (sr.w * frac.clamp(0.0, 1.0)).max(0.0);
                            if fw > 0.5 {
                                g.fill_rrect(Rect::new(sr.x, track.y, fw, 6.0), 3.0, accent);
                            }
                            let idx = h.len();
                            g.fill_circle(sr.x + fw, sr.cy(), if hot == Some(idx) { 8.0 } else { 7.0 }, Color::white(1.0));
                            h.push((Rect::new(sr.x - 8.0, sr.y, sr.w + 16.0, sr.h), PopAct::Bri(i, sr)));
                        }));
                        y += 32.0;
                    }
                    y += 6.0;
                } else {
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
                            h.push((Rect::new(sr.x - 8.0, sr.y, sr.w + 16.0, sr.h), PopAct::Bri(0, sr)));
                        }
                    }));
                    y += 36.0;
                    let display_title = displays.first().map(|d| d.name.as_str()).unwrap_or("Brightness");
                    let t = match bri {
                        Some(b) => format!("{} {}%", display_title, b),
                        None => "Drag to set brightness".to_string(),
                    };
                    ops.push(note(t, y - 4.0, th.t2));
                    y += 22.0;
                }
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
                // the month on show: today's, or moved by the arrows / mouse wheel
                let total = year * 12 + (month.max(1) as i32 - 1) + cal_off;
                let (dy, dm) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
                let first = crate::ui::first_of_month(dy, dm);
                let dim = crate::ui::days_in_month(dy, dm);
                let rows = (first + dim + 6) / 7;
                let cw = (w - 32.0) / 7.0;
                let top = y;
                ops.push(Box::new(move |g, h, hot| {
                    g.text(
                        &format!("{} {}", MONTHS[(dm.max(1) as usize - 1) % 12], dy),
                        Rect::new(16.0, top, w - 32.0, 20.0),
                        &g.fonts.small,
                        th.t1,
                        Align::Left,
                    );
                    // previous / next month, and a way back to today
                    let btn = |r: Rect, glyph: Option<char>, label: &str, act: PopAct, g: &Gfx, h: &mut Vec<(Rect, PopAct)>| {
                        // always a visible chip; brighter under the pointer
                        g.fill_rrect(r, 8.0, if hot == Some(h.len()) { th.hover } else { th.hover.fade(0.55) });
                        match glyph {
                            Some(c) => g.icon(c, r, &g.fonts.icon14, th.t1),
                            None => g.text(label, r, &g.fonts.small, th.t1, Align::Center),
                        }
                        h.push((r, act));
                    };
                    btn(Rect::new(w - 16.0 - 32.0, top - 2.0, 32.0, 24.0), Some('\u{E76C}'), "", PopAct::CalMonth(1), g, h);
                    btn(Rect::new(w - 16.0 - 68.0, top - 2.0, 32.0, 24.0), Some('\u{E76B}'), "", PopAct::CalMonth(-1), g, h);
                    if cal_off != 0 {
                        btn(Rect::new(w - 16.0 - 132.0, top - 2.0, 58.0, 24.0), None, "Today", PopAct::CalToday, g, h);
                    }
                    for (i, d) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
                        g.text(d, Rect::new(16.0 + i as f32 * cw, top + 22.0, cw, 18.0), &g.fonts.tiny, th.t2, Align::Center);
                    }
                    for d in 1..=dim {
                        let slot = first + d - 1;
                        let (c, r) = (slot % 7, slot / 7);
                        let cell = Rect::new(16.0 + c as f32 * cw, top + 42.0 + r as f32 * 28.0, cw, 28.0);
                        if cal_off == 0 && d == day {
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
                        let idx = self.pop.as_ref().and_then(|p| p.drag_bri_idx).unwrap_or(0);
                        self.set_display_bri(idx, pct);
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
                if let Some(PopAct::Vol(r)) = act {
                    if let Some(p) = self.pop.as_mut() {
                        p.drag = Some(r);
                        p.drag_bri_idx = None;
                    }
                    unsafe {
                        SetCapture(hwnd);
                    }
                    self.set_vol_from(r, x);
                    self.render_pop();
                } else if let Some(PopAct::Bri(idx, r)) = act {
                    if let Some(p) = self.pop.as_mut() {
                        p.drag = Some(r);
                        p.drag_bri_idx = Some(idx);
                    }
                    unsafe {
                        SetCapture(hwnd);
                    }
                    let v = ((x - r.x) / r.w).clamp(0.0, 1.0);
                    let pct = (v * 100.0).round() as u32;
                    self.set_display_bri(idx, pct);
                    self.render_pop();
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let dragging = self.pop.as_ref().map(|p| p.drag.is_some()).unwrap_or(false);
                if dragging {
                    let bri_idx = self.pop.as_ref().and_then(|p| p.drag_bri_idx);
                    if let Some(p) = self.pop.as_mut() {
                        p.drag = None;
                        p.drag_bri_idx = None;
                    }
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    if self.pop_kind() == Some(BarAct::Brightness) {
                        if let Some(idx) = bri_idx {
                            if let Some(d) = self.displays.get(idx) {
                                sys::brightness_set_display(idx, d.brightness);
                            }
                        } else if let Some(b) = self.brightness {
                            sys::brightness_set(b);
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
                            let hw = self.hwnd.0 as isize;
                            if cur.as_deref() == Some(n.ssid.as_str()) {
                                sys::wifi_connect_async(hw, None);
                            } else if n.saved {
                                sys::wifi_connect_async(hw, Some(n.ssid));
                            } else if !n.secure {
                                sys::wifi_join_new_async(hw, n.ssid, n.auth, None);
                            } else if sys::wifi_auth_supported(&n.auth) {
                                self.start_pw(n.ssid, n.auth);
                                return LRESULT(0);
                            } else {
                                // enterprise etc.: let Windows handle it
                                sys::open("ms-settings:network-wifi", "");
                                self.close_pop();
                                return LRESULT(0);
                            }
                            self.sync_link_timer();
                            self.render_bar();
                            self.render_pop();
                        }
                    }
                    Some(PopAct::Bt(i)) => {
                        let dev = self.radio.devices.lock().unwrap().get(i).cloned();
                        if let Some(d) = dev {
                            if d.address != 0 && self.bt_busy.is_none() {
                                let _ = self
                                    .radio
                                    .tx
                                    .send(crate::radio::Req::BtConnect(d.address, !d.connected));
                                self.bt_busy = Some((d.address, !d.connected, d.name.clone()));
                                self.sync_link_timer();
                                self.render_bar();
                                self.render_pop();
                            }
                        }
                    }
                    Some(PopAct::PwShow) => {
                        if let Some(pw) = self.pw.as_mut() {
                            pw.show = !pw.show;
                        }
                        self.render_pop();
                    }
                    Some(PopAct::PwJoin) => self.pw_join(),
                    Some(PopAct::PwCancel) => {
                        self.end_pw(true);
                        self.render_pop();
                    }
                    Some(PopAct::Link(url)) => {
                        sys::open(url, "");
                        self.close_pop();
                    }
                    Some(PopAct::CalMonth(d)) => {
                        self.cal_off = (self.cal_off + d).clamp(-1200, 1200);
                        self.render_pop();
                    }
                    Some(PopAct::CalToday) => {
                        self.cal_off = 0;
                        self.render_pop();
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => {
                let r = if self.pw.is_some() { MA_ACTIVATE } else { MA_NOACTIVATE };
                LRESULT(r as isize)
            }
            WM_CHAR => {
                if self.pw.is_some() {
                    let c = wp.0 as u16;
                    match c {
                        0x0D => self.pw_join(),
                        0x1B => {
                            self.end_pw(true);
                        }
                        0x16 => {
                            // Ctrl+V
                            if let Some(t) = sys::clipboard_text(hwnd) {
                                if let Some(pw) = self.pw.as_mut() {
                                    for u in t.trim().encode_utf16() {
                                        if u >= 0x20 && pw.buf.len() < 63 {
                                            pw.buf.push(u);
                                        }
                                    }
                                }
                            }
                        }
                        0x08 => {
                            if let Some(pw) = self.pw.as_mut() {
                                pw.buf.pop();
                            }
                        }
                        c if c >= 0x20 => {
                            if let Some(pw) = self.pw.as_mut() {
                                if pw.buf.len() < 63 {
                                    pw.buf.push(c);
                                }
                            }
                        }
                        _ => {}
                    }
                    self.render_pop();
                }
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                if self.pw.is_some() {
                    self.end_pw(false);
                    self.render_pop();
                }
                LRESULT(0)
            }
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
