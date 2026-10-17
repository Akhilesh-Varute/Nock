//! Immediate-mode drawing of the pill and the expanded panel. Every frame the
//! visible controls re-register themselves in `hits`, which `App` uses for
//! hover and click handling. All coordinates are logical (DPI-independent).

use crate::app::*;
use crate::gfx::*;
use crate::sys;

const BLACK: Color = Color(0.0, 0.0, 0.0, 1.0);
const T1: Color = Color::white(0.96);
const T2: Color = Color::white(0.62);
const T3: Color = Color::white(0.38);
const SURF: Color = Color::white(0.09);
const HOVER: Color = Color::white(0.17);

/// How far the page area sits below the top of the open panel (the tab row is above it).
pub const CONTENT_Y: f32 = 40.0;

pub fn draw(a: &App, w: f32, h: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let t = a.t_expand();
    let rad = a.col_rad() + (30.0 - a.col_rad()) * t;

    // Body. It starts above the window so the top corners are clipped away and
    // the shape hangs flush from the top edge of the screen like a notch.
    // Click squish, heartbeat and hover jiggle nudge it by a pixel or two.
    let beat = a.beat_factor();
    let inset = (a.squish * a.squish * 3.0).max(beat * 2.0);
    let jig = a.jiggle * (a.fx_phase).sin() * 1.5;
    let body = Rect::new(inset + jig, -rad, w - 2.0 * inset, h + rad - inset);
    g.fill_rrect(body, rad, BLACK);
    g.stroke_rrect(
        Rect::new(body.x + 0.5, -rad, body.w - 1.0, body.h - 0.5),
        rad,
        Color::white(0.10),
        1.0,
    );
    if beat > 0.01 {
        g.stroke_rrect(body, rad, Color(1.0, 0.4, 0.5, 0.85 * beat), 2.5);
    }
    if a.level > 0.01 && t < 0.05 {
        // music glow: the pill's rim and fill breathe with the output level
        g.fill_rrect(body, rad, a.accent.fade(0.03 + 0.15 * a.level));
        g.stroke_rrect(body, rad, a.accent.fade(0.2 + 0.7 * a.level), 1.0 + 2.2 * a.level);
    }

    if a.alerting() && t < 0.05 {
        let pulse = 0.5 + 0.5 * (a.eq_phase * 0.9).sin();
        g.fill_rrect(
            Rect::new(0.0, -rad, w, h + rad),
            rad,
            a.accent.fade(0.25 + 0.45 * pulse),
        );
    }
    let c_alpha = (1.0 - t * 3.2).clamp(0.0, 1.0);
    let e_alpha = ((t - 0.45) / 0.55).clamp(0.0, 1.0);
    if c_alpha > 0.0 {
        draw_collapsed(a, w, c_alpha);
    }
    if e_alpha > 0.0 {
        draw_expanded(a, e_alpha, hits);
    }
    if let Some(ct) = a.confetti_t() {
        draw_confetti(a, w, h, ct);
    }
}
/// Confetti burst from the pill (or panel) after the focus timer ends: pure
/// functions of time, so there is no particle state to keep.
fn draw_confetti(a: &App, w: f32, h: f32, t: f32) {
    let g = &a.gfx;
    const COLORS: [Color; 6] = [
        Color(1.0, 0.45, 0.5, 1.0),
        Color(1.0, 0.8, 0.3, 1.0),
        Color(0.5, 0.9, 0.6, 1.0),
        Color(0.45, 0.75, 1.0, 1.0),
        Color(0.8, 0.6, 1.0, 1.0),
        Color(1.0, 1.0, 1.0, 1.0),
    ];
    let fade = (1.0 - (t - 2.4).max(0.0) / 0.6).clamp(0.0, 1.0);
    for i in 0..40u32 {
        let r = |k: u32| {
            let mut x = (i + 1).wrapping_mul(2654435761).wrapping_add(k.wrapping_mul(40503));
            x ^= x >> 15;
            x = x.wrapping_mul(2246822519);
            x ^= x >> 13;
            (x % 1000) as f32 / 1000.0
        };
        let x = w * (0.2 + 0.6 * r(1)) + (r(2) - 0.5) * 150.0 * t;
        let y = h * 0.5 - (30.0 + 80.0 * r(3)) * t + 0.5 * 220.0 * t * t;
        if y > h + 6.0 || x < -4.0 || x > w + 4.0 {
            continue;
        }
        let spin = (t * (6.0 + 8.0 * r(4)) + r(5) * 6.0).sin().abs();
        let c = COLORS[(r(6) * 6.0) as usize % 6];
        g.fill_rrect(Rect::new(x, y, 2.0 + 3.0 * spin, 4.0 - 2.0 * spin), 0.5, c.fade(fade));
    }
}


// ------------------------------------------------------------- collapsed ----

fn draw_collapsed(a: &App, w: f32, ca: f32) {
    let g = &a.gfx;
    let f = &g.fonts;
    let m = &a.mstate;
    let ch = a.col_h();
    let timer = a.timer_running();

    if a.alerting() {
        let white = Color(1.0, 1.0, 1.0, ca);
        if a.alert_msg == "Look away" {
            // a drawn eye
            let (ex, ey) = (w / 2.0 - 50.0, ch / 2.0);
            g.fill_rrect(Rect::new(ex - 8.0, ey - 5.0, 16.0, 10.0), 5.0, Color(1.0, 1.0, 1.0, ca));
            g.fill_circle(ex, ey, 3.4, Color(0.25, 0.55, 0.9, ca));
            g.fill_circle(ex, ey, 1.5, Color(0.0, 0.0, 0.0, ca));
        } else if a.alert_msg == "Drink water" {
            // a drawn water drop
            let (dx, dy) = (w / 2.0 - 52.0, ch / 2.0 + 1.0);
            g.fill_circle(dx, dy + 2.0, 5.0, Color(0.45, 0.8, 1.0, ca));
            g.fill_poly(&[(dx - 4.4, dy), (dx + 4.4, dy), (dx, dy - 8.0)], Color(0.45, 0.8, 1.0, ca));
        } else {
            g.icon('\u{E916}', Rect::new(w / 2.0 - 58.0, 0.0, 20.0, ch), &f.icon14, white);
        }
        g.text(
            a.alert_msg,
            Rect::new(w / 2.0 - 34.0, 0.0, 100.0, ch),
            &f.body,
            white,
            Align::Left,
        );
        return;
    }

    if m.has_track() {
        let ar = Rect::new(8.0, (ch - 22.0) / 2.0, 22.0, 22.0);
        match &a.art {
            Some(bmp) => g.draw_bitmap(bmp, ar, 6.0, ca),
            None => {
                g.fill_rrect(ar, 6.0, Color::white(0.14 * ca));
                g.text("\u{266A}", ar, &f.small, T2.fade(ca), Align::Center);
            }
        }
        // equaliser bars on the right
        let bx = w - 10.0 - 20.0;
        draw_bars(a, bx, ch / 2.0, ca);
        let mut right = bx - 8.0;
        if timer {
            let r = Rect::new(right - 50.0, 0.0, 50.0, ch);
            g.text(
                &fmt_time(a.timer_remaining_ms() as f64 / 1000.0),
                r,
                &f.small,
                T1.fade(ca),
                Align::Right,
            );
            right -= 58.0;
        }
        let title = if m.title.is_empty() {
            "Unknown"
        } else {
            &m.title
        };
        let label = if m.artist.is_empty() {
            title.to_string()
        } else {
            format!("{}  \u{00B7}  {}", title, m.artist)
        };
        g.text(
            &label,
            Rect::new(38.0, 0.0, (right - 38.0 - 4.0).max(10.0), ch),
            &f.small,
            T1.fade(ca),
            Align::Left,
        );
    } else if timer {
        g.icon('\u{E916}', Rect::new(w / 2.0 - 36.0, 0.0, 18.0, ch), &f.icon14, T1.fade(ca));
        g.text(
            &fmt_time(a.timer_remaining_ms() as f64 / 1000.0),
            Rect::new(w / 2.0 - 14.0, 0.0, 56.0, ch),
            &f.small,
            T1.fade(ca),
            Align::Left,
        );
    } else {
        let (_, _, _, _, hh, mm, _, _) = sys::local_time();
        let label = if a.cfg.clock_24h {
            format!("{:02}:{:02}", hh, mm)
        } else {
            let (h12, ampm) = match hh {
                0 => (12, "AM"),
                1..=11 => (hh, "AM"),
                12 => (12, "PM"),
                _ => (hh - 12, "PM"),
            };
            format!("{}:{:02} {}", h12, mm, ampm)
        };
        g.text(
            &label,
            Rect::new(0.0, 0.0, w, ch),
            &f.small,
            T2.fade(ca),
            Align::Center,
        );
    }
}

fn draw_bars(a: &App, x: f32, cy: f32, ca: f32) {
    let g = &a.gfx;
    let playing = a.mstate.playing;
    let col = a.accent.fade(ca * if playing { 1.0 } else { 0.45 });
    for i in 0..4 {
        let hgt = if playing && a.cfg.eq_animation {
            4.0 + 10.0 * (0.5 + 0.5 * (a.eq_phase + i as f32 * 1.7).sin())
        } else if playing {
            [9.0, 13.0, 8.0, 11.0][i]
        } else {
            [5.0, 8.0, 6.0, 4.0][i]
        };
        let bx = x + i as f32 * 5.5;
        g.fill_rrect(Rect::new(bx, cy - hgt * 0.5, 3.0, hgt), 1.5, col);
    }
}

// -------------------------------------------------------------- expanded ----

fn hot(a: &App, act: Act) -> bool {
    a.hot == Some(act)
}

fn draw_expanded(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;

    // top row: pin, labelled tabs, collapse. The pages sit below it.
    let pin = Rect::new(12.0, 8.0, 28.0, 28.0);
    hits.push(Hit { r: pin, act: Act::Pin });
    let pc = if a.pinned { a.accent } else if hot(a, Act::Pin) { T1 } else { T3 };
    g.icon('\u{E718}', pin, &f.icon14, pc.fade(ca));

    let chev = Rect::new(EXP_W - 40.0, 8.0, 28.0, 28.0);
    hits.push(Hit { r: chev, act: Act::Collapse });
    g.icon('\u{E70E}', chev, &f.icon14, (if hot(a, Act::Collapse) { T1 } else { T3 }).fade(ca));

    let labels = ["Media", "Controls", "Today", "Stats", "Clipboard", "Apps", "Games", "Power"];
    // rough text width: narrow letters count less, so the gaps between tabs look even
    let text_w = |s: &str| -> f32 {
        s.chars().map(|c| match c { 'i' | 'l' | 't' | 'f' | 'j' | 'r' | 'I' => 3.4, 'm' | 'w' | 'M' | 'W' => 9.6, _ => 6.7 }).sum()
    };
    let widths: Vec<f32> = labels.iter().map(|l| text_w(l) + 24.0).collect();
    let total: f32 = widths.iter().sum();
    let (left, right) = (46.0, EXP_W - 46.0);
    let gap = ((right - left - total) / (labels.len() as f32 - 1.0)).clamp(2.0, 12.0);
    let used = total + gap * (labels.len() as f32 - 1.0);
    let mut x = left + (right - left - used) / 2.0;
    for (i, p) in PAGES.iter().enumerate() {
        let r = Rect::new(x, 8.0, widths[i], 28.0);
        x += widths[i] + gap;
        let act = Act::Tab(*p);
        hits.push(Hit { r, act });
        let active = a.page == *p;
        if active {
            g.fill_rrect(r, 14.0, a.accent.fade(0.20 * ca));
        } else if hot(a, act) {
            g.fill_rrect(r, 14.0, Color::white(0.07 * ca));
        }
        let col = if active { a.accent } else if hot(a, act) { T2 } else { T3 };
        g.text(labels[i], r, &f.small, col.fade(ca), Align::Center);
    }

    // the page itself, moved below the tab row; its hit boxes move with it
    let start = hits.len();
    g.shifted(0.0, CONTENT_Y, || match a.page {
        Page::Media => page_media(a, ca, hits),
        Page::Controls => page_controls(a, ca, hits),
        Page::Clipboard => page_clipboard(a, ca, hits),
        Page::Apps => page_apps(a, ca, hits),
        Page::Tasks => page_tasks(a, ca, hits),
        Page::Stats => page_stats(a, ca),
        Page::Power => page_power(a, ca, hits),
        Page::Game => {
            let (ox, oy) = ((EXP_W - crate::game::W) / 2.0, 14.0);
            if a.hub.is_playing() {
                hits.push(Hit { r: Rect::new(ox, oy, crate::game::W, crate::game::H), act: Act::GameClick });
                a.hub.draw(g, ox, oy, ca);
                let r = Rect::new(ox + crate::game::W / 2.0 - 36.0, oy + 3.0, 72.0, 18.0);
                hits.push(Hit { r, act: Act::GameBack });
                g.fill_rrect(r, 9.0, (if hot(a, Act::GameBack) { HOVER } else { Color(0.0, 0.0, 0.0, 0.35) }).fade(ca));
                g.text("\u{2039} Games", r, &f.tiny, T1.fade(ca), Align::Center);
            } else {
                let hot_tile = (0..4).find(|i| hot(a, Act::GamePick(*i)));
                a.hub.draw_hub(g, ox, oy, ca, hot_tile);
                for (i, r) in crate::games::Hub::tiles(ox, oy).iter().enumerate() {
                    hits.push(Hit { r: *r, act: Act::GamePick(i) });
                }
            }
        }
    });
    for h in &mut hits[start..] {
        h.r.y += CONTENT_Y;
    }
}

// ----------------------------------------------------------------- media ----

fn page_media(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;
    let m = &a.mstate;

    if !m.has_track() {
        g.icon(
            '\u{E768}',
            Rect::new(0.0, 56.0, EXP_W, 40.0),
            &f.icon22,
            T3.fade(ca),
        );
        g.text(
            "Nothing playing",
            Rect::new(0.0, 98.0, EXP_W, 24.0),
            &f.body,
            T2.fade(ca),
            Align::Center,
        );
        g.text(
            "Start music or a video and it shows up here",
            Rect::new(0.0, 122.0, EXP_W, 20.0),
            &f.small,
            T3.fade(ca),
            Align::Center,
        );
        return;
    }

    // artwork
    let ar = Rect::new(28.0, 22.0, 132.0, 132.0);
    match &a.art {
        Some(bmp) => g.draw_bitmap(bmp, ar, 16.0, ca),
        None => {
            g.fill_rrect(ar, 16.0, Color::white(0.10 * ca));
            g.text("\u{266A}", ar, &f.big, T3.fade(ca), Align::Center);
        }
    }

    let tx = 182.0;
    let tw = EXP_W - tx - 56.0;
    let title = if m.title.is_empty() {
        "Unknown title"
    } else {
        &m.title
    };
    g.text(
        title,
        Rect::new(tx, 22.0, tw, 26.0),
        &f.title,
        T1.fade(ca),
        Align::Left,
    );
    let sub = if m.artist.is_empty() {
        &m.album
    } else {
        &m.artist
    };
    g.text(
        sub,
        Rect::new(tx, 49.0, tw, 22.0),
        &f.body,
        T2.fade(ca),
        Align::Left,
    );
    if !m.app.is_empty() {
        g.text(
            &m.app,
            Rect::new(tx, 72.0, tw, 18.0),
            &f.tiny,
            T3.fade(ca),
            Align::Left,
        );
    }

    // progress
    let (pos, dur) = m.progress(sys::filetime_now());
    let frac = match a.seek_frac {
        Some(fr) => fr,
        None if dur > 0.0 => (pos / dur) as f32,
        None => 0.0,
    };
    let shown = if a.seek_frac.is_some() {
        frac as f64 * dur
    } else {
        pos
    };
    let bar = Rect::new(tx, 104.0, EXP_W - tx - 28.0, 18.0);
    slider(a, bar, frac, Act::Seek, a.accent, ca, hits, dur > 0.0);
    g.text(
        &fmt_time(shown),
        Rect::new(tx, 122.0, 70.0, 16.0),
        &f.tiny,
        T3.fade(ca),
        Align::Left,
    );
    if dur > 0.0 {
        g.text(
            &fmt_time(dur),
            Rect::new(bar.x + bar.w - 70.0, 122.0, 70.0, 16.0),
            &f.tiny,
            T3.fade(ca),
            Align::Right,
        );
    }

    // transport
    let cx = tx + (EXP_W - tx - 28.0) / 2.0;
    let cy = 168.0;
    let prev = Rect::new(cx - 92.0, cy - 20.0, 40.0, 40.0);
    let play = Rect::new(cx - 26.0, cy - 26.0, 52.0, 52.0);
    let next = Rect::new(cx + 52.0, cy - 20.0, 40.0, 40.0);
    hits.push(Hit {
        r: prev,
        act: Act::Prev,
    });
    hits.push(Hit {
        r: play,
        act: Act::PlayPause,
    });
    hits.push(Hit {
        r: next,
        act: Act::Next,
    });
    for (r, act, glyph) in [(prev, Act::Prev, '\u{E892}'), (next, Act::Next, '\u{E893}')] {
        if hot(a, act) {
            g.fill_rrect(r, 20.0, Color::white(0.10 * ca));
        }
        g.icon(glyph, r, &f.icon18, T1.fade(ca));
    }
    let big = if hot(a, Act::PlayPause) { 1.0 } else { 0.92 };
    g.fill_circle(play.cx(), play.cy(), 24.0, Color::white(big * ca));
    let glyph = if m.playing { '\u{E769}' } else { '\u{E768}' };
    g.icon(glyph, play, &f.icon22, Color(0.0, 0.0, 0.0, ca));
}

// -------------------------------------------------------------- controls ----

/// One rounded tile: round icon, title, small state line. `on` fills it with the accent.
#[allow(clippy::too_many_arguments)]
fn tile(a: &App, ca: f32, hits: &mut Vec<Hit>, r: Rect, act: Act, glyph: char, title: &str, sub: &str, on: bool) {
    let g = &a.gfx;
    let f = &g.fonts;
    hits.push(Hit { r, act });
    let hovered = hot(a, act);
    let bg = if on { a.accent } else if hovered { HOVER } else { SURF };
    g.fill_rrect(r, 18.0, bg.fade(ca));
    if on && hovered {
        g.fill_rrect(r, 18.0, Color::white(0.10 * ca));
    }
    let ic = Rect::new(r.x + 12.0, r.y + (r.h - 34.0) / 2.0, 34.0, 34.0);
    g.fill_circle(ic.cx(), ic.cy(), 17.0, (if on { Color(0.0, 0.0, 0.0, 0.14) } else { Color::white(0.08) }).fade(ca));
    g.icon(glyph, ic, &f.icon14, (if on { BLACK } else { T1 }).fade(ca));
    let (tc, sc) = if on { (BLACK, Color(0.0, 0.0, 0.0, 0.65)) } else { (T1, T3) };
    g.text(title, Rect::new(r.x + 56.0, r.y + 9.0, r.w - 64.0, 22.0), &f.body, tc.fade(ca), Align::Left);
    g.text(sub, Rect::new(r.x + 56.0, r.y + r.h - 28.0, r.w - 64.0, 18.0), &f.tiny, sc.fade(ca), Align::Left);
}

fn page_controls(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let (w, h, gap) = (270.0, 54.0, 12.0);
    let (x0, y0) = (24.0, 6.0);
    let at = |col: f32, row: f32| Rect::new(x0 + col * (w + gap), y0 + row * (h + 8.0), w, h);
    let state = |s: Option<bool>| match s {
        Some(true) => ("On", true),
        Some(false) => ("Off", false),
        None => ("Not available", false),
    };
    let (ws, won) = state(a.radios.wifi);
    tile(a, ca, hits, at(0.0, 0.0), Act::Quick(0), '\u{E701}', "Wi-Fi", ws, won);
    let (bs, bon) = state(a.radios.bluetooth);
    tile(a, ca, hits, at(1.0, 0.0), Act::Quick(1), '\u{E702}', "Bluetooth", bs, bon);
    tile(a, ca, hits, at(0.0, 1.0), Act::DarkMode, '\u{E708}', "Dark mode", if a.dark_mode { "On" } else { "Off" }, a.dark_mode);
    tile(a, ca, hits, at(1.0, 1.0), Act::Open(0), '\u{E706}', "Night light", "Open settings", false);
    tile(a, ca, hits, at(0.0, 2.0), Act::Quick(2), '\u{E722}', "Screenshot", "Snip a region", false);
    tile(a, ca, hits, at(1.0, 2.0), Act::Open(1), '\u{E713}', "Settings", "Windows settings", false);
}

// ----------------------------------------------------------------- power ----

fn page_power(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;
    let armed = |k: u8| matches!(a.power_confirm, Some((p, t)) if p == k && t.elapsed().as_secs_f32() < 3.5);
    let items: [(char, &str, Act, u8); 4] = [
        ('\u{E72E}', "Lock", Act::Quick(3), 0),
        ('\u{E708}', "Sleep", Act::Quick(4), 0),
        ('\u{E777}', "Restart", Act::PowerRestart, 1),
        ('\u{E7E8}', "Shut down", Act::PowerShutdown, 2),
    ];
    let (w, gap) = (129.0, 12.0);
    for (i, (glyph, label, act, k)) in items.iter().enumerate() {
        let r = Rect::new(24.0 + i as f32 * (w + gap), 28.0, w, 130.0);
        hits.push(Hit { r, act: *act });
        let danger = *k != 0 && armed(*k);
        let bg = if danger { Color(0.85, 0.25, 0.28, 1.0) } else if hot(a, *act) { HOVER } else { SURF };
        g.fill_rrect(r, 20.0, bg.fade(ca));
        g.icon(*glyph, Rect::new(r.x, r.y + 22.0, r.w, 40.0), &f.icon22, (if danger { Color::white(1.0) } else { T1 }).fade(ca));
        g.text(label, Rect::new(r.x, r.y + 70.0, r.w, 24.0), &f.body, (if danger { Color::white(1.0) } else { T1 }).fade(ca), Align::Center);
        if *k != 0 {
            g.text(
                if danger { "Click again to confirm" } else { "Asks to confirm" },
                Rect::new(r.x + 4.0, r.y + 96.0, r.w - 8.0, 20.0),
                &f.tiny,
                (if danger { Color::white(0.95) } else { T3 }).fade(ca),
                Align::Center,
            );
        }
    }
}

// ----------------------------------------------------------------- stats ----

/// A 270 degree ring gauge from `frac` 0..1, drawn as a trail of dots.
fn ring(g: &Gfx, cx: f32, cy: f32, r: f32, frac: f32, col: Color, ca: f32) {
    let n = 120;
    for i in 0..=n {
        let u = i as f32 / n as f32;
        let ang = (135.0 + 270.0 * u).to_radians();
        let lit = u < frac.clamp(0.0, 1.0);
        let c = if lit { col } else { Color::white(0.10) };
        g.fill_circle(cx + r * ang.cos(), cy + r * ang.sin(), 3.6, c.fade(ca));
    }
}

fn page_stats(a: &App, ca: f32) {
    let g = &a.gfx;
    let f = &g.fonts;
    let card = |x: f32, w: f32| {
        let r = Rect::new(x, 4.0, w, 190.0);
        g.fill_rrect(r, 20.0, SURF.fade(ca));
        r
    };
    let gauge = |r: Rect, label: &str, pct: u32, hist: Option<&Vec<f32>>| {
        let (cx, cy) = (r.cx(), r.y + 84.0);
        ring(g, cx, cy, 48.0, pct as f32 / 100.0, a.accent, ca);
        g.text(&format!("{pct}%"), Rect::new(cx - 40.0, cy - 20.0, 80.0, 40.0), &f.big, T1.fade(ca), Align::Center);
        g.text(label, Rect::new(r.x, r.y + 138.0, r.w, 22.0), &f.body, T2.fade(ca), Align::Center);
        if let Some(h) = hist {
            let n = h.len();
            for (i, v) in h.iter().enumerate() {
                let bh = 2.0 + v.clamp(0.0, 1.0) * 16.0;
                g.fill_rrect(Rect::new(r.x + 20.0 + i as f32 * 3.5, r.y + r.h - 12.0 - bh, 2.4, bh), 1.0, a.accent.fade(((i + 1) as f32 / n.max(1) as f32) * 0.8 * ca));
            }
        }
    };
    gauge(card(20.0, 180.0), "CPU", a.stat_cpu, Some(&a.stat_hist));
    gauge(card(210.0, 180.0), "Memory", a.stat_mem, None);
    let r = card(400.0, 180.0);
    // battery
    g.text("Battery", Rect::new(r.x + 16.0, r.y + 12.0, r.w - 32.0, 20.0), &f.small, T3.fade(ca), Align::Left);
    match sys::battery_info() {
        Some(b) => {
            g.text(&format!("{}%", b.percent), Rect::new(r.x + 16.0, r.y + 32.0, 90.0, 40.0), &f.big, T1.fade(ca), Align::Left);
            let state = if b.charging { "Charging" } else if b.plugged { "Plugged in" } else { "On battery" };
            g.text(state, Rect::new(r.x + 16.0, r.y + 74.0, r.w - 32.0, 18.0), &f.tiny, T2.fade(ca), Align::Left);
            let bar = Rect::new(r.x + 16.0, r.y + 98.0, r.w - 32.0, 8.0);
            g.fill_rrect(bar, 4.0, Color::white(0.10 * ca));
            let col = if b.percent <= 20 && !b.charging { Color(1.0, 0.45, 0.4, 1.0) } else { a.accent };
            g.fill_rrect(Rect::new(bar.x, bar.y, bar.w * (b.percent as f32 / 100.0), bar.h), 4.0, col.fade(ca));
        }
        None => {
            g.text("Plugged in", Rect::new(r.x + 16.0, r.y + 38.0, r.w - 32.0, 30.0), &f.body, T1.fade(ca), Align::Left);
            g.text("No battery", Rect::new(r.x + 16.0, r.y + 66.0, r.w - 32.0, 18.0), &f.tiny, T3.fade(ca), Align::Left);
        }
    }
    // uptime
    let up = sys::uptime_secs();
    let (d, h, m) = (up / 86400, (up % 86400) / 3600, (up % 3600) / 60);
    let text = if d > 0 { format!("{d}d {h}h {m}m") } else { format!("{h}h {m}m") };
    g.text("Uptime", Rect::new(r.x + 16.0, r.y + 128.0, r.w - 32.0, 20.0), &f.small, T3.fade(ca), Align::Left);
    g.text(&text, Rect::new(r.x + 16.0, r.y + 148.0, r.w - 32.0, 30.0), &f.body, T1.fade(ca), Align::Left);
}

// ------------------------------------------------------------- clipboard ----

fn page_clipboard(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;
    let view = a.clip_view();

    let total = a.pins.len() + a.clips.len();
    let head = if total == 0 {
        "Clipboard".to_string()
    } else {
        format!("Clipboard  \u{00B7}  {}", total)
    };
    g.text(&head, Rect::new(28.0, 12.0, 200.0, 24.0), &f.small, T3.fade(ca), Align::Left);
    if !a.clips.is_empty() {
        let r = Rect::new(EXP_W - 112.0, 12.0, 64.0, 24.0);
        hits.push(Hit { r, act: Act::ClipClear });
        if hot(a, Act::ClipClear) {
            g.fill_rrect(r, 12.0, Color::white(0.10 * ca));
        }
        g.text("Clear", r, &f.small, T2.fade(ca), Align::Center);
    }
    // search chip: click, type to filter, Enter keeps it, Esc clears it
    let sr = Rect::new(EXP_W - 252.0, 12.0, 132.0, 24.0);
    let typing = a.entry_search;
    if typing || !a.clip_filter.is_empty() {
        let mut t = a.clip_filter.clone();
        if typing {
            t.push('\u{2502}');
        }
        g.fill_rrect(sr, 12.0, Color::white(0.14 * ca));
        g.text(&t, Rect::new(sr.x + 10.0, sr.y, sr.w - 30.0, sr.h), &f.small, T1.fade(ca), Align::Left);
        if !typing {
            let x = Rect::new(sr.x + sr.w - 24.0, sr.y, 24.0, sr.h);
            hits.push(Hit { r: sr, act: Act::ClipSearch });
            hits.push(Hit { r: x, act: Act::ClipSearchClear });
            g.icon('\u{E711}', x, &f.icon14, (if hot(a, Act::ClipSearchClear) { T1 } else { T2 }).fade(ca));
        }
    } else {
        hits.push(Hit { r: sr, act: Act::ClipSearch });
        if hot(a, Act::ClipSearch) {
            g.fill_rrect(sr, 12.0, Color::white(0.10 * ca));
        }
        g.icon('\u{E721}', Rect::new(sr.x + 4.0, sr.y, 24.0, sr.h), &f.icon14, T3.fade(ca));
        g.text("Search", Rect::new(sr.x + 30.0, sr.y, sr.w - 34.0, sr.h), &f.small, T3.fade(ca), Align::Left);
    }

    if view.is_empty() {
        let (l1, l2) = if a.clip_filter.is_empty() {
            ("Copy something and it will appear here.", "Click an item to paste it. Click the star to pin it as a saved snippet.")
        } else {
            ("No matches.", "Press Esc in the search box to clear the filter.")
        };
        g.text(l1, Rect::new(0.0, 80.0, EXP_W, 24.0), &f.body, T3.fade(ca), Align::Center);
        g.text(l2, Rect::new(0.0, 104.0, EXP_W, 20.0), &f.small, T3.fade(ca * 0.8), Align::Center);
        return;
    }

    for row in 0..CLIP_ROWS {
        let idx = a.clip_scroll + row;
        let Some((text, pinned)) = view.get(idx) else { break };
        let r = Rect::new(24.0, 40.0 + row as f32 * 31.0, EXP_W - 48.0, 28.0);
        let act = Act::ClipItem(idx);
        hits.push(Hit { r, act });
        let pin = Rect::new(r.x + r.w - 32.0, r.y, 30.0, r.h);
        let pact = Act::ClipPin(idx);
        hits.push(Hit { r: pin, act: pact });
        if hot(a, act) || hot(a, pact) {
            g.fill_rrect(r, 10.0, Color::white(0.12 * ca));
        } else if row % 2 == 0 {
            g.fill_rrect(r, 10.0, Color::white(0.04 * ca));
        }
        g.text(&preview(text), Rect::new(r.x + 14.0, r.y, r.w - 52.0, r.h), &f.body, T1.fade(ca), Align::Left);
        let (glyph, col) = if *pinned {
            ('\u{E735}', a.accent)
        } else if hot(a, pact) {
            ('\u{E734}', T1)
        } else {
            ('\u{E734}', T3)
        };
        g.icon(glyph, pin, &f.icon14, col.fade(ca * if *pinned || hot(a, act) || hot(a, pact) { 1.0 } else { 0.6 }));
    }
}

fn preview(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 160 {
        one.chars().take(160).collect()
    } else {
        one
    }
}

// ------------------------------------------------------------------ apps ----

fn page_apps(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;
    if a.apps.is_empty() {
        g.text(
            "No pinned apps. Edit apps.txt from the tray menu.",
            Rect::new(0.0, 90.0, EXP_W, 24.0),
            &f.body,
            T3.fade(ca),
            Align::Center,
        );
        return;
    }
    let (cols, tw, th) = (6usize, 88.0, 86.0);
    let gap = (EXP_W - 56.0 - cols as f32 * tw) / (cols as f32 - 1.0);
    for (i, e) in a.apps.iter().enumerate() {
        let (c, r) = (i % cols, i / cols);
        let rect = Rect::new(
            28.0 + c as f32 * (tw + gap),
            6.0 + r as f32 * (th + 8.0),
            tw,
            th,
        );
        let act = Act::Launch(i);
        hits.push(Hit { r: rect, act });
        g.fill_rrect(rect, 16.0, Color::white((if hot(a, act) { 0.12 } else { 0.05 }) * ca));
        let ic = Rect::new(rect.cx() - 18.0, rect.y + 10.0, 36.0, 36.0);
        match a.app_icons.get(i).and_then(|b| b.as_ref()) {
            Some(bmp) => g.draw_bitmap(bmp, ic, 0.0, ca),
            None => {
                let hue = name_color(&e.name);
                g.fill_circle(ic.cx(), ic.cy(), 18.0, hue.fade(ca));
                let letter: String = e
                    .name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().collect())
                    .unwrap_or_default();
                g.text(
                    &letter,
                    ic,
                    &g.fonts.title,
                    Color(0.0, 0.0, 0.0, 0.75 * ca),
                    Align::Center,
                );
            }
        }
        g.text(
            &e.name,
            Rect::new(rect.x + 4.0, rect.y + 52.0, rect.w - 8.0, 24.0),
            &f.tiny,
            T2.fade(ca),
            Align::Center,
        );
    }
}

fn name_color(name: &str) -> Color {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    const P: [Color; 6] = [
        Color::rgb(0xFF, 0xB4, 0x6B),
        Color::rgb(0x8F, 0xD3, 0xFF),
        Color::rgb(0xB9, 0xA4, 0xFF),
        Color::rgb(0x9B, 0xE8, 0xA8),
        Color::rgb(0xFF, 0x9E, 0xB8),
        Color::rgb(0xF2, 0xE0, 0x7A),
    ];
    P[(h % 6) as usize]
}

// ----------------------------------------------------------------- tasks ----

fn page_tasks(a: &App, ca: f32, hits: &mut Vec<Hit>) {
    let g = &a.gfx;
    let f = &g.fonts;

    // ---- cards
    for r in [Rect::new(14.0, 2.0, 208.0, 198.0), Rect::new(230.0, 2.0, 190.0, 198.0), Rect::new(428.0, 2.0, 158.0, 198.0)] {
        g.fill_rrect(r, 20.0, Color::white(0.05 * ca));
    }

    // ---- focus timer
    let title = if a.break_mode {
        "Break".to_string()
    } else if let Some(t) = &a.timer_task {
        let short: String = t.chars().take(20).collect();
        format!("Focus \u{00B7} {short}")
    } else {
        "Focus timer".to_string()
    };
    g.text(
        &title,
        Rect::new(28.0, 12.0, 185.0, 22.0),
        &f.small,
        (if a.break_mode { a.accent } else { T3 }).fade(ca),
        Align::Left,
    );
    g.text(
        &format!("Today: {} min focus \u{00B7} {} done", a.focus_today, a.done_today),
        Rect::new(28.0, 176.0, 190.0, 16.0),
        &f.tiny,
        T3.fade(ca),
        Align::Left,
    );
    let left = a.timer_remaining_ms();
    let done = !a.timer_running() && left == 0;
    let face = match (&a.entering, a.entry_timer) {
        (Some(buf), true) => format!("{}\u{2502} min", String::from_utf16_lossy(buf)),
        _ => fmt_time(left as f64 / 1000.0),
    };
    g.text(
        &face,
        Rect::new(28.0, 34.0, 175.0, 52.0),
        &f.big,
        (if done && !a.entry_timer { a.accent } else { T1 }).fade(ca),
        Align::Left,
    );
    if a.entry_timer {
        g.text(
            "Enter to start \u{00B7} Esc to cancel",
            Rect::new(28.0, 76.0, 190.0, 16.0),
            &f.tiny,
            T3.fade(ca),
            Align::Left,
        );
    }
    if done {
        g.text(
            "Time's up",
            Rect::new(110.0, 12.0, 90.0, 22.0),
            &f.small,
            a.accent.fade(ca),
            Align::Right,
        );
    }
    for (i, mins) in [5u32, 15, 25, 50].iter().enumerate() {
        let r = Rect::new(28.0 + i as f32 * 38.0, 94.0, 34.0, 26.0);
        let act = Act::TimerPreset(*mins);
        hits.push(Hit { r, act });
        let sel = a.timer_total_s == mins * 60;
        let bg = if hot(a, act) {
            HOVER
        } else if sel {
            Color::white(0.13)
        } else {
            SURF
        };
        g.fill_rrect(r, 13.0, bg.fade(ca));
        g.text(
            &format!("{}m", mins),
            r,
            &f.small,
            (if sel { T1 } else { T2 }).fade(ca),
            Align::Center,
        );
    }
    // custom length: type minutes
    let cr = Rect::new(28.0 + 4.0 * 38.0, 94.0, 34.0, 26.0);
    hits.push(Hit { r: cr, act: Act::TimerCustom });
    g.fill_rrect(cr, 13.0, (if hot(a, Act::TimerCustom) || a.entry_timer { HOVER } else { SURF }).fade(ca));
    g.icon('\u{E70F}', cr, &f.icon14, T2.fade(ca));
    let sb = Rect::new(28.0, 130.0, 108.0, 36.0);
    hits.push(Hit {
        r: sb,
        act: Act::TimerToggle,
    });
    g.fill_rrect(
        sb,
        18.0,
        Color::white((if hot(a, Act::TimerToggle) { 1.0 } else { 0.92 }) * ca),
    );
    g.text(
        if a.timer_running() { "Pause" } else { "Start" },
        sb,
        &f.body,
        Color(0.0, 0.0, 0.0, ca),
        Align::Center,
    );
    let rb = Rect::new(144.0, 130.0, 36.0, 36.0);
    hits.push(Hit {
        r: rb,
        act: Act::TimerReset,
    });
    g.fill_rrect(
        rb,
        18.0,
        (if hot(a, Act::TimerReset) { HOVER } else { SURF }).fade(ca),
    );
    g.icon('\u{E72C}', rb, &f.icon14, T1.fade(ca));

    // ---- to-do
    let tx = 240.0;
    let tw = 170.0;
    g.text(
        "To-do",
        Rect::new(tx, 12.0, 100.0, 22.0),
        &f.small,
        T3.fade(ca),
        Align::Left,
    );
    for (i, t) in a.todos.iter().enumerate() {
        let y = 38.0 + i as f32 * 28.0;
        let row = Rect::new(tx, y, tw, 26.0);
        let cb = Rect::new(tx, y, 26.0, 26.0);
        let del = Rect::new(tx + tw - 24.0, y, 24.0, 26.0);
        let fb = Rect::new(tx + tw - 50.0, y, 24.0, 26.0);
        if !t.done {
            hits.push(Hit { r: fb, act: Act::TodoFocus(i) });
        }
        hits.push(Hit {
            r: cb,
            act: Act::TodoToggle(i),
        });
        hits.push(Hit {
            r: del,
            act: Act::TodoDel(i),
        });
        let over = hot(a, Act::TodoToggle(i)) || hot(a, Act::TodoDel(i)) || hot(a, Act::TodoFocus(i));
        if over {
            g.fill_rrect(row, 9.0, Color::white(0.07 * ca));
        }
        let glyph = if t.done { '\u{E73A}' } else { '\u{E739}' };
        let gc = if t.done { a.accent } else { T2 };
        g.icon(glyph, cb, &f.icon14, gc.fade(ca));
        g.text(
            &t.text,
            Rect::new(tx + 30.0, y, tw - 30.0 - 26.0, 26.0),
            &f.small,
            (if t.done { T3 } else { T1 }).fade(ca),
            Align::Left,
        );
        if over {
            g.fill_rrect(Rect::new(tx + tw - 52.0, y + 1.0, 52.0, 24.0), 8.0, Color(0.117, 0.117, 0.117, 1.0).fade(ca));
        }
        if !t.done {
            g.icon(
                '\u{E768}',
                fb,
                &f.icon14,
                (if hot(a, Act::TodoFocus(i)) { a.accent } else { T3 }).fade(ca * if over { 1.0 } else { 0.0 }),
            );
        }
        g.icon(
            '\u{E711}',
            del,
            &f.icon14,
            (if hot(a, Act::TodoDel(i)) { T1 } else { T3 }).fade(ca * if over { 1.0 } else { 0.0 }),
        );
    }
    let y = 38.0 + a.todos.len() as f32 * 28.0;
    if let (Some(buf), false) = (&a.entering, a.entry_timer) {
        let r = Rect::new(tx, y, tw, 26.0);
        g.fill_rrect(r, 9.0, Color::white(0.14 * ca));
        let mut s = String::from_utf16_lossy(buf);
        s.push('\u{2502}'); // caret
        g.text(
            &s,
            Rect::new(r.x + 10.0, r.y, r.w - 16.0, r.h),
            &f.small,
            T1.fade(ca),
            Align::Left,
        );
        g.text(
            "Enter to add \u{00B7} Esc to cancel",
            Rect::new(tx, y + 28.0, tw, 18.0),
            &f.tiny,
            T3.fade(ca),
            Align::Left,
        );
    } else if a.todos.len() < MAX_TODO {
        let r = Rect::new(tx, y, tw, 26.0);
        hits.push(Hit {
            r,
            act: Act::TodoAdd,
        });
        if hot(a, Act::TodoAdd) {
            g.fill_rrect(r, 9.0, Color::white(0.07 * ca));
        }
        g.icon(
            '\u{E710}',
            Rect::new(tx, y, 26.0, 26.0),
            &f.icon14,
            T3.fade(ca),
        );
        g.text(
            "Add task",
            Rect::new(tx + 30.0, y, 140.0, 26.0),
            &f.small,
            T3.fade(ca),
            Align::Left,
        );
    }

    // ---- calendar
    draw_calendar(a, ca);
}

fn draw_calendar(a: &App, ca: f32) {
    let g = &a.gfx;
    let f = &g.fonts;
    let (year, month, day, wday, ..) = sys::local_time();
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let x0 = 432.0;
    let cw = 20.0;
    g.text(
        &format!("{} {}", MONTHS[(month as usize - 1).min(11)], year),
        Rect::new(x0 - 4.0, 12.0, cw * 7.0 + 8.0, 22.0),
        &f.small,
        T2.fade(ca),
        Align::Center,
    );
    for (i, d) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
        g.text(
            d,
            Rect::new(x0 + i as f32 * cw, 36.0, cw, 18.0),
            &f.tiny,
            T3.fade(ca),
            Align::Center,
        );
    }
    let first = first_weekday(day, wday);
    let dim = days_in_month(year, month);
    for d in 1..=dim {
        let slot = first + d - 1;
        let (c, r) = (slot % 7, slot / 7);
        let cell = Rect::new(x0 + c as f32 * cw, 56.0 + r as f32 * 20.0, cw, 20.0);
        if d == day {
            g.fill_circle(cell.cx(), cell.cy(), 9.5, a.accent.fade(ca));
            g.text(
                &d.to_string(),
                cell,
                &f.tiny,
                Color(0.0, 0.0, 0.0, ca),
                Align::Center,
            );
        } else {
            let col = if c == 0 || c == 6 { T3 } else { T2 };
            g.text(&d.to_string(), cell, &f.tiny, col.fade(ca), Align::Center);
        }
    }
}

/// Weekday (0 = Sunday) of the 1st of the month, given today's day and weekday.
pub(crate) fn first_weekday(day: u32, wday: u32) -> u32 {
    (wday + 7 - (day - 1) % 7) % 7
}

pub(crate) fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

// --------------------------------------------------------------- helpers ----

fn slider(
    a: &App,
    r: Rect,
    frac: f32,
    act: Act,
    fill: Color,
    ca: f32,
    hits: &mut Vec<Hit>,
    enabled: bool,
) {
    let g = &a.gfx;
    let track = Rect::new(r.x, r.cy() - 3.0, r.w, 6.0);
    g.fill_rrect(track, 3.0, Color::white(0.16 * ca));
    let fw = (r.w * frac.clamp(0.0, 1.0)).max(0.0);
    if enabled {
        if fw > 0.5 {
            g.fill_rrect(Rect::new(r.x, track.y, fw, 6.0), 3.0, fill.fade(ca));
        }
        hits.push(Hit { r, act });
        if hot(a, act) || a.seek_frac.is_some() && act == Act::Seek {
            g.fill_circle(r.x + fw, r.cy(), 7.0, Color::white(ca));
        }
    }
}

pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as i64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_format() {
        assert_eq!(fmt_time(0.0), "0:00");
        assert_eq!(fmt_time(65.4), "1:05");
        assert_eq!(fmt_time(3725.0), "1:02:05");
        assert_eq!(fmt_time(-5.0), "0:00");
    }

    #[test]
    fn month_lengths() {
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2028, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2026, 10), 31);
        assert_eq!(days_in_month(2026, 11), 30);
    }

    #[test]
    fn first_of_month_weekday() {
        // Thu 15 Oct 2026 -> 1 Oct 2026 was a Thursday (4)
        assert_eq!(first_weekday(15, 4), 4);
        // Sun 1 Nov 2026 -> itself
        assert_eq!(first_weekday(1, 0), 0);
        // Fri 30 Oct 2026 (5): 1st is 29 days earlier -> Thursday (4)
        assert_eq!(first_weekday(30, 5), 4);
    }

    #[test]
    fn previews_collapse_whitespace_and_truncate() {
        assert_eq!(preview("a \n\t b   c"), "a b c");
        assert_eq!(preview(&"x".repeat(500)).chars().count(), 160);
    }
}
