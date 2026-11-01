//! Dropdown panels under the top-bar items on Linux (Wi-Fi, Bluetooth, volume,
//! brightness, battery, calendar). Model and drawing only: the window and the
//! system calls that fill `PopData` live in `linux_main`.

use crate::cal::{days_in_month, first_of_month};
use crate::gfx::*;

pub const POP_W: f32 = 300.0;
pub const POP_MAX_H: f32 = 340.0;
const ROW: f32 = 34.0;
const MAX_ROWS: usize = 6;

const T1: Color = Color::white(0.96);
const T2: Color = Color::white(0.62);
const T3: Color = Color::white(0.38);
const HOVER: Color = Color::white(0.16);
const LINE: Color = Color::white(0.10);
const ACCENT: Color = Color(0.40, 0.60, 1.0, 1.0);
const GREEN: Color = Color(0.45, 0.85, 0.5, 1.0);
const RED: Color = Color(1.0, 0.45, 0.4, 1.0);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Wifi,
    Bluetooth,
    Volume,
    Brightness,
    Battery,
    Clock,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Net {
    pub ssid: String,
    pub signal: u8,
    pub secure: bool,
    pub active: bool,
    pub known: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Dev {
    pub mac: String,
    pub name: String,
    pub connected: bool,
}

#[derive(Clone, Default, PartialEq)]
pub struct PopData {
    pub wifi_on: bool,
    pub nets: Vec<Net>,
    pub wifi_loaded: bool,
    pub bt_on: bool,
    pub devs: Vec<Dev>,
    pub bt_loaded: bool,
    pub volume: Option<(u8, bool)>,
    pub brightness: Option<u8>,
    pub battery: Option<(u8, bool)>,
    /// something is connecting: what to show next to the spinner
    pub busy: Option<String>,
    /// last result, with whether it worked
    pub msg: Option<(String, bool)>,
}

/// A Wi-Fi password being typed.
#[derive(Clone, Default)]
pub struct Pw {
    pub ssid: String,
    pub buf: String,
    pub show: bool,
}

#[derive(Clone, Default)]
pub struct PopState {
    pub hover: Option<usize>,
    /// months the calendar is shown away from the current month
    pub cal_off: i32,
    pub pw: Option<Pw>,
    pub spin: u32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    /// 0 = Wi-Fi radio, 1 = Bluetooth power
    Toggle(u8),
    Mute,
    Vol(Rect),
    Bri(Rect),
    Wifi(usize),
    Bt(usize),
    /// system settings page (see `sys::open_settings`)
    Link(u8),
    CalMonth(i32),
    CalToday,
    PwShow,
    PwJoin,
    PwCancel,
}

type Hits = Vec<(Rect, Act)>;
type Op = Box<dyn Fn(&Gfx, &mut Hits, Option<usize>)>;

fn spinner(g: &Gfx, cx: f32, cy: f32, phase: u32, c: Color) {
    for i in 0..8u32 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let behind = (phase + 8 - i % 8) % 8;
        let k = 1.0 - behind as f32 / 8.0;
        g.fill_circle(cx + 6.5 * a.cos(), cy + 6.5 * a.sin(), 1.8, Color(c.0, c.1, c.2, c.3 * (0.2 + 0.8 * k)));
    }
}

/// Four signal bars, filled by strength.
fn signal(g: &Gfx, x: f32, y: f32, pct: u8, c: Color) {
    let lit = (pct as usize * 4).div_ceil(100).clamp(1, 4);
    for i in 0..4 {
        let h = 4.0 + 3.0 * i as f32;
        g.fill_rrect(Rect::new(x + i as f32 * 5.0, y + 13.0 - h, 3.0, h), 1.0, if i < lit { c } else { Color::white(0.16) });
    }
}

fn switch(on: bool, which: u8, y: f32) -> Op {
    Box::new(move |g, h, _| {
        let r = Rect::new(POP_W - 16.0 - 40.0, y + 1.0, 40.0, 22.0);
        g.fill_rrect(r, 11.0, if on { ACCENT } else { HOVER });
        let kx = if on { r.x + r.w - 11.0 } else { r.x + 11.0 };
        g.fill_circle(kx, r.cy(), 9.0, Color::white(1.0));
        h.push((Rect::new(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0), Act::Toggle(which)));
    })
}

fn title(t: &'static str, y: f32) -> Op {
    Box::new(move |g, _, _| g.text(t, Rect::new(16.0, y, 200.0, 24.0), &g.fonts.menu, T1, Align::Left))
}

fn note(t: String, y: f32, c: Color) -> Op {
    Box::new(move |g, _, _| g.text(&t, Rect::new(16.0, y, POP_W - 32.0, 22.0), &g.fonts.small, c, Align::Left))
}

fn line(y: f32) -> Op {
    Box::new(move |g, _, _| g.fill_rrect(Rect::new(16.0, y, POP_W - 32.0, 1.0), 0.0, LINE))
}

fn link(t: &'static str, page: u8, y: f32) -> Op {
    Box::new(move |g, h, hot| {
        let r = Rect::new(8.0, y, POP_W - 16.0, 30.0);
        if hot == Some(h.len()) {
            g.fill_rrect(r, 7.0, HOVER);
        }
        g.text(t, Rect::new(16.0, y, POP_W - 32.0, 30.0), &g.fonts.small, T1, Align::Left);
        h.push((r, Act::Link(page)));
    })
}

/// Horizontal slider with a percentage on the right; `mk` builds the hit action.
fn slider(frac: f32, y: f32, mk: fn(Rect) -> Act, label: String, muted: bool) -> Op {
    Box::new(move |g, h, _| {
        let r = Rect::new(16.0, y, POP_W - 32.0 - 48.0, 24.0);
        let track = Rect::new(r.x, r.cy() - 3.0, r.w, 6.0);
        g.fill_rrect(track, 3.0, Color::white(0.16));
        let fw = r.w * frac.clamp(0.0, 1.0);
        if fw > 0.5 {
            g.fill_rrect(Rect::new(r.x, track.y, fw, 6.0), 3.0, if muted { T3 } else { ACCENT });
        }
        g.fill_circle(r.x + fw, r.cy(), 8.0, Color::white(1.0));
        g.text(&label, Rect::new(r.x + r.w + 6.0, y, 42.0, 24.0), &g.fonts.small, T2, Align::Right);
        h.push((Rect::new(r.x - 8.0, r.y, r.w + 16.0, r.h), mk(r)));
    })
}

/// Height of the content for `kind`, or the dropdown cannot be sized before it is drawn.
pub fn paint(g: &Gfx, kind: Kind, d: &PopData, st: &PopState, hits: &mut Hits, clock_24h: bool) -> f32 {
    let mut ops: Vec<Op> = Vec::new();
    let mut y = 14.0f32;
    let _ = clock_24h;

    match kind {
        Kind::Wifi => {
            ops.push(title("Wi-Fi", y));
            ops.push(switch(d.wifi_on, 0, y));
            y += 34.0;
            ops.push(line(y));
            y += 8.0;
            if let Some(pw) = &st.pw {
                let ssid = pw.ssid.clone();
                ops.push(note(format!("Password for {ssid}"), y, T1));
                y += 28.0;
                let (shown, show) = (pw.buf.clone(), pw.show);
                let fy = y;
                ops.push(Box::new(move |g, h, hot| {
                    let r = Rect::new(16.0, fy, POP_W - 32.0, 30.0);
                    g.fill_rrect(r, 8.0, Color::white(0.12));
                    let text = if show { shown.clone() } else { "\u{2022}".repeat(shown.chars().count()) };
                    g.text(&format!("{text}\u{2502}"), Rect::new(r.x + 10.0, r.y, r.w - 60.0, r.h), &g.fonts.body, T1, Align::Left);
                    let eye = Rect::new(r.x + r.w - 52.0, r.y, 48.0, r.h);
                    if hot == Some(h.len()) {
                        g.fill_rrect(eye, 8.0, HOVER);
                    }
                    g.text(if show { "Hide" } else { "Show" }, eye, &g.fonts.tiny, T2, Align::Center);
                    h.push((eye, Act::PwShow));
                }));
                y += 40.0;
                let by = y;
                ops.push(Box::new(move |g, h, hot| {
                    let join = Rect::new(POP_W - 16.0 - 80.0, by, 80.0, 30.0);
                    let cancel = Rect::new(join.x - 8.0 - 80.0, by, 80.0, 30.0);
                    g.fill_rrect(cancel, 15.0, if hot == Some(h.len()) { HOVER } else { Color::white(0.08) });
                    g.text("Cancel", cancel, &g.fonts.small, T1, Align::Center);
                    h.push((cancel, Act::PwCancel));
                    g.fill_rrect(join, 15.0, ACCENT.fade(if hot == Some(h.len()) { 1.0 } else { 0.85 }));
                    g.text("Join", join, &g.fonts.small, Color(0.0, 0.0, 0.0, 1.0), Align::Center);
                    h.push((join, Act::PwJoin));
                }));
                y += 44.0;
            } else if !d.wifi_on {
                ops.push(note("Wi-Fi is off".into(), y, T2));
                y += 30.0;
            } else if !d.wifi_loaded {
                let (sy, ph) = (y, st.spin);
                ops.push(Box::new(move |g, _, _| {
                    spinner(g, 28.0, sy + 14.0, ph, T2);
                    g.text("Looking for networks\u{2026}", Rect::new(44.0, sy, 220.0, 28.0), &g.fonts.small, T2, Align::Left);
                }));
                y += 34.0;
            } else if d.nets.is_empty() {
                ops.push(note("No networks found".into(), y, T2));
                y += 30.0;
            } else {
                for (i, n) in d.nets.iter().take(MAX_ROWS).enumerate() {
                    let (n, ry, busy) = (n.clone(), y, d.busy.as_deref() == Some(n.ssid.as_str()));
                    let ph = st.spin;
                    ops.push(Box::new(move |g, h, hot| {
                        let r = Rect::new(8.0, ry, POP_W - 16.0, ROW - 2.0);
                        if hot == Some(h.len()) {
                            g.fill_rrect(r, 8.0, HOVER);
                        }
                        signal(g, 18.0, ry + 8.0, n.signal, if n.active { GREEN } else { T2 });
                        let name: String = n.ssid.chars().take(26).collect();
                        let lock = if n.secure { "  \u{2022}" } else { "" };
                        g.text(&format!("{name}{lock}"), Rect::new(46.0, ry, POP_W - 46.0 - 90.0, ROW - 2.0), &g.fonts.small, T1, Align::Left);
                        if busy {
                            spinner(g, POP_W - 28.0, ry + 15.0, ph, T2);
                        } else if n.active {
                            g.text("Connected", Rect::new(POP_W - 100.0, ry, 88.0, ROW - 2.0), &g.fonts.tiny, GREEN, Align::Right);
                        } else if n.known {
                            g.text("Saved", Rect::new(POP_W - 100.0, ry, 88.0, ROW - 2.0), &g.fonts.tiny, T3, Align::Right);
                        }
                        h.push((r, Act::Wifi(i)));
                    }));
                    y += ROW;
                }
            }
            y += 4.0;
            ops.push(line(y));
            y += 4.0;
            ops.push(link("Network settings\u{2026}", 2, y));
            y += 32.0;
        }
        Kind::Bluetooth => {
            ops.push(title("Bluetooth", y));
            ops.push(switch(d.bt_on, 1, y));
            y += 34.0;
            ops.push(line(y));
            y += 8.0;
            if !d.bt_on {
                ops.push(note("Bluetooth is off".into(), y, T2));
                y += 30.0;
            } else if !d.bt_loaded {
                let (sy, ph) = (y, st.spin);
                ops.push(Box::new(move |g, _, _| {
                    spinner(g, 28.0, sy + 14.0, ph, T2);
                    g.text("Looking for devices\u{2026}", Rect::new(44.0, sy, 220.0, 28.0), &g.fonts.small, T2, Align::Left);
                }));
                y += 34.0;
            } else if d.devs.is_empty() {
                ops.push(note("No paired devices".into(), y, T2));
                y += 30.0;
            } else {
                for (i, dev) in d.devs.iter().take(MAX_ROWS).enumerate() {
                    let (dev, ry, busy) = (dev.clone(), y, d.busy.as_deref() == Some(dev.name.as_str()));
                    let ph = st.spin;
                    ops.push(Box::new(move |g, h, hot| {
                        let r = Rect::new(8.0, ry, POP_W - 16.0, ROW - 2.0);
                        if hot == Some(h.len()) {
                            g.fill_rrect(r, 8.0, HOVER);
                        }
                        g.fill_circle(26.0, ry + 15.0, 4.0, if dev.connected { GREEN } else { Color::white(0.2) });
                        let name: String = dev.name.chars().take(26).collect();
                        g.text(&name, Rect::new(46.0, ry, POP_W - 46.0 - 90.0, ROW - 2.0), &g.fonts.small, T1, Align::Left);
                        if busy {
                            spinner(g, POP_W - 28.0, ry + 15.0, ph, T2);
                        } else if dev.connected {
                            g.text("Connected", Rect::new(POP_W - 100.0, ry, 88.0, ROW - 2.0), &g.fonts.tiny, GREEN, Align::Right);
                        }
                        h.push((r, Act::Bt(i)));
                    }));
                    y += ROW;
                }
            }
            y += 4.0;
            ops.push(line(y));
            y += 4.0;
            ops.push(link("Bluetooth settings\u{2026}", 5, y));
            y += 32.0;
        }
        Kind::Volume => {
            ops.push(title("Sound", y));
            let (pct, muted) = d.volume.unwrap_or((0, false));
            let my = y;
            ops.push(Box::new(move |g, h, hot| {
                let r = Rect::new(POP_W - 16.0 - 64.0, my, 64.0, 24.0);
                g.fill_rrect(r, 12.0, if hot == Some(h.len()) { HOVER } else { Color::white(0.07) });
                g.text(if muted { "Unmute" } else { "Mute" }, r, &g.fonts.tiny, if muted { RED } else { T2 }, Align::Center);
                h.push((r, Act::Mute));
            }));
            y += 36.0;
            ops.push(slider(pct as f32 / 100.0, y, Act::Vol, format!("{pct}%"), muted));
            y += 34.0;
            ops.push(line(y));
            y += 4.0;
            ops.push(link("Sound settings\u{2026}", 1, y));
            y += 32.0;
        }
        Kind::Brightness => {
            ops.push(title("Brightness", y));
            y += 36.0;
            let pct = d.brightness.unwrap_or(0);
            ops.push(slider(pct as f32 / 100.0, y, Act::Bri, format!("{pct}%"), false));
            y += 34.0;
            ops.push(line(y));
            y += 4.0;
            ops.push(link("Display settings\u{2026}", 0, y));
            y += 32.0;
        }
        Kind::Battery => {
            ops.push(title("Battery", y));
            y += 34.0;
            let (pct, plugged) = d.battery.unwrap_or((0, false));
            let by = y;
            ops.push(Box::new(move |g, _, _| {
                let col = if plugged { GREEN } else if pct <= 20 { Color(1.0, 0.72, 0.2, 1.0) } else { T1 };
                g.stroke_rrect(Rect::new(16.0, by + 4.0, 60.0, 30.0), 6.0, T2, 2.0);
                g.fill_rrect(Rect::new(77.0, by + 13.0, 4.0, 12.0), 2.0, T2);
                g.fill_rrect(Rect::new(20.0, by + 8.0, 52.0 * pct as f32 / 100.0, 22.0), 3.0, col);
                g.text(&format!("{pct}%"), Rect::new(96.0, by, 80.0, 38.0), &g.fonts.big, T1, Align::Left);
            }));
            y += 44.0;
            ops.push(note(if plugged { "Plugged in".into() } else { "On battery".into() }, y, T2));
            y += 28.0;
            ops.push(line(y));
            y += 4.0;
            ops.push(link("Power settings\u{2026}", 3, y));
            y += 32.0;
        }
        Kind::Clock => {
            let (cy0, cm0, today, _, ..) = crate::sys::local_time();
            // the month shown, `cal_off` months from this one
            let idx = cy0 * 12 + (cm0 as i32 - 1) + st.cal_off;
            let (year, month) = (idx.div_euclid(12), idx.rem_euclid(12) as u32 + 1);
            const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
            let head = format!("{} {}", MONTHS[month as usize - 1], year);
            let hy = y;
            ops.push(Box::new(move |g, h, hot| {
                g.text(&head, Rect::new(16.0, hy, 170.0, 24.0), &g.fonts.menu, T1, Align::Left);
                for (i, (glyph, act)) in [("\u{2039}", Act::CalMonth(-1)), ("Today", Act::CalToday), ("\u{203A}", Act::CalMonth(1))].iter().enumerate() {
                    let w = if i == 1 { 46.0 } else { 26.0 };
                    let x = [POP_W - 16.0 - 26.0 - 46.0 - 26.0 - 8.0, POP_W - 16.0 - 26.0 - 46.0 - 4.0, POP_W - 16.0 - 26.0][i];
                    let r = Rect::new(x, hy, w, 24.0);
                    if hot == Some(h.len()) {
                        g.fill_rrect(r, 8.0, HOVER);
                    }
                    g.text(glyph, r, &g.fonts.small, T2, Align::Center);
                    h.push((r, *act));
                }
            }));
            y += 34.0;
            let (first, dim) = (first_of_month(year, month), days_in_month(year, month));
            let gy = y;
            let is_now = st.cal_off == 0;
            ops.push(Box::new(move |g, _, _| {
                let (x0, cw) = (16.0 + (POP_W - 32.0 - 7.0 * 38.0) / 2.0, 38.0);
                for (i, d) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
                    g.text(d, Rect::new(x0 + i as f32 * cw, gy, cw, 20.0), &g.fonts.tiny, T3, Align::Center);
                }
                for d in 1..=dim {
                    let slot = first + d - 1;
                    let (c, r) = (slot % 7, slot / 7);
                    let cell = Rect::new(x0 + c as f32 * cw, gy + 22.0 + r as f32 * 28.0, cw, 28.0);
                    if is_now && d == today {
                        g.fill_circle(cell.cx(), cell.cy(), 12.0, ACCENT);
                        g.text(&d.to_string(), cell, &g.fonts.small, Color(0.0, 0.0, 0.0, 1.0), Align::Center);
                    } else {
                        g.text(&d.to_string(), cell, &g.fonts.small, if c == 0 || c == 6 { T3 } else { T1 }, Align::Center);
                    }
                }
            }));
            let rows = (first + dim).div_ceil(7);
            y += 22.0 + rows as f32 * 28.0 + 8.0;
        }
    }

    // a connection result, or what is being attempted
    if let (Some((m, ok)), Kind::Wifi | Kind::Bluetooth) = (&d.msg, kind) {
        ops.push(note(m.clone(), y, if *ok { GREEN } else { RED }));
        y += 26.0;
    }
    let h = (y + 6.0).min(POP_MAX_H);

    g.fill_rrect(Rect::new(0.0, 0.0, POP_W, h), 14.0, Color(0.11, 0.11, 0.12, 0.96));
    g.stroke_rrect(Rect::new(0.5, 0.5, POP_W - 1.0, h - 1.0), 14.0, LINE, 1.0);
    hits.clear();
    for op in &ops {
        op(g, hits, st.hover);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_lays_out_within_the_window() {
        let g = Gfx::new(POP_W as i32, POP_MAX_H as i32, 1.0).unwrap();
        let d = PopData {
            wifi_on: true,
            wifi_loaded: true,
            nets: (0..9).map(|i| Net { ssid: format!("Net {i}"), signal: 80, secure: i % 2 == 0, active: i == 0, known: i == 1 }).collect(),
            bt_on: true,
            bt_loaded: true,
            devs: vec![Dev { mac: "AA".into(), name: "Buds".into(), connected: true }],
            volume: Some((40, false)),
            brightness: Some(70),
            battery: Some((55, false)),
            ..PopData::default()
        };
        for kind in [Kind::Wifi, Kind::Bluetooth, Kind::Volume, Kind::Brightness, Kind::Battery, Kind::Clock] {
            let mut hits = Vec::new();
            let h = paint(&g, kind, &d, &PopState::default(), &mut hits, false);
            assert!(h > 40.0 && h <= POP_MAX_H, "{kind:?} height {h}");
            assert!(!hits.is_empty(), "{kind:?} has something to click");
            assert!(hits.iter().all(|(r, _)| r.y + r.h <= h + 1.0), "{kind:?} hit boxes stay inside");
        }
    }

    #[test]
    fn password_entry_replaces_the_list() {
        let g = Gfx::new(POP_W as i32, POP_MAX_H as i32, 1.0).unwrap();
        let d = PopData { wifi_on: true, wifi_loaded: true, nets: vec![Net { ssid: "Home".into(), ..Net::default() }], ..PopData::default() };
        let st = PopState { pw: Some(Pw { ssid: "Home".into(), ..Pw::default() }), ..PopState::default() };
        let mut hits = Vec::new();
        paint(&g, Kind::Wifi, &d, &st, &mut hits, false);
        let acts: Vec<_> = hits.iter().map(|(_, a)| *a).collect();
        assert!(acts.contains(&Act::PwJoin) && acts.contains(&Act::PwCancel));
        assert!(!acts.iter().any(|a| matches!(a, Act::Wifi(_))));
    }

    /// Renders each dropdown for eyeballing (NOCK_PREVIEW_DIR=dir cargo test -- --ignored).
    #[test]
    #[ignore]
    fn preview_dropdowns() {
        let dir = std::env::var("NOCK_PREVIEW_DIR").expect("NOCK_PREVIEW_DIR");
        let g = Gfx::new(POP_W as i32, POP_MAX_H as i32, 1.0).unwrap();
        let d = PopData {
            wifi_on: true,
            wifi_loaded: true,
            nets: vec![
                Net { ssid: "Home Network".into(), signal: 90, secure: true, active: true, known: true },
                Net { ssid: "Cafe Guest".into(), signal: 55, secure: false, ..Net::default() },
                Net { ssid: "Neighbour-5G".into(), signal: 30, secure: true, known: true, ..Net::default() },
            ],
            bt_on: true,
            bt_loaded: true,
            devs: vec![Dev { mac: "A".into(), name: "WH-1000XM4".into(), connected: true }, Dev { mac: "B".into(), name: "Keyboard".into(), connected: false }],
            volume: Some((65, false)),
            brightness: Some(70),
            battery: Some((82, false)),
            msg: Some(("Connected to Home Network".into(), true)),
            ..PopData::default()
        };
        for (name, kind) in [("wifi", Kind::Wifi), ("bt", Kind::Bluetooth), ("vol", Kind::Volume), ("bri", Kind::Brightness), ("bat", Kind::Battery), ("cal", Kind::Clock)] {
            let _ = g.begin(POP_W as i32, POP_MAX_H as i32);
            let mut st = PopState::default();
            st.hover = Some(1);
            paint(&g, kind, &d, &st, &mut Vec::new(), false);
            let _ = g.end();
            g.save_png(&format!("{dir}/pop_{name}.png"));
        }
    }
}
