//! Buddy: a tiny original character that lives on the top bar. It naps most of
//! the time and wakes up in short bursts: it strolls along the free stretch of
//! the bar, dances when music starts, follows your pointer when you move it
//! nearby, cheers when the focus timer ends, and shows small speech bubbles.
//!
//! It is drawn from simple shapes (no image files) into its own small
//! click-through layered window, so animating it never repaints the whole bar.
//! The animation timer runs only while it is awake.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::*;
use crate::bar::{theme, BAR_H};
use crate::cast;
use crate::gfx::*;
use crate::sys;

/// Window width (logical px). The character stands at `SPR_X`; speech bubbles
/// grow to its left.
const BW: f32 = 380.0;
const SPR_X: f32 = 342.0;

pub const T_BUDDY: usize = 14;
pub const T_BWAKE: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuddyKind {
    Blob,
    Cat,
    Bunny,
    Hero,
    Robot,
    // pixel-art characters that also talk (see cast.rs / pixdata.rs)
    Ronin,
    Courier,
    Necro,
    Clippy,
    Merlin,
    Genie,
}

impl BuddyKind {
    fn pixel_who(self) -> Option<crate::cast::Who> {
        match self {
            BuddyKind::Ronin => Some(crate::cast::Who::Ronin),
            BuddyKind::Courier => Some(crate::cast::Who::Courier),
            BuddyKind::Necro => Some(crate::cast::Who::Necro),
            _ => None,
        }
    }

    /// In-character remark for the pixel buddies.
    fn quote(self, r: u32) -> Option<&'static str> {
        let list: &[&str] = match self {
            BuddyKind::Ronin => &["Patience.", "The blade is quiet.", "One cut.", "Rain will come.", "Breathe.", "Hm."],
            BuddyKind::Courier => &["Delivery's late. Again.", "Not my problem.", "Neon never sleeps.", "Fast lane.", "Tip me.", "Zero ping."],
            BuddyKind::Necro => &["Rise.", "Death is a pause.", "Mind the bones.", "Souls. So many.", "Hush...", "Not dead yet?"],
            BuddyKind::Clippy => &[
                "It looks like you're writing code.",
                "Need assistance with that function?",
                "Don't forget to commit!",
                "Just checking in on you!",
                "Would you like help with that?",
                "Remember to save your work!",
            ],
            BuddyKind::Merlin => &[
                "By the ancient stars!",
                "A wizard's code is never late.",
                "Ponder the orb of knowledge.",
                "Behold, an enchantment!",
                "Focus turns spells into reality.",
                "Hocus Pocus!",
            ],
            BuddyKind::Genie => &[
                "Your wish is my command!",
                "Phenomenal cosmic power... itty-bitty bar!",
                "Need three wishes or just more coffee?",
                "Rub the lamp anytime, chief!",
                "Poof! What can I do for ya?",
                "Ain't never had a buddy like me!",
            ],
            _ => return None,
        };
        Some(list[(r as usize) % list.len()])
    }

    /// 1..=5 as stored in config.ini (0 means off).
    pub fn from_id(id: u8) -> Option<BuddyKind> {
        match id {
            1 => Some(BuddyKind::Blob),
            2 => Some(BuddyKind::Cat),
            3 => Some(BuddyKind::Bunny),
            4 => Some(BuddyKind::Hero),
            5 => Some(BuddyKind::Robot),
            6 => Some(BuddyKind::Ronin),
            7 => Some(BuddyKind::Courier),
            8 => Some(BuddyKind::Necro),
            9 => Some(BuddyKind::Clippy),
            10 => Some(BuddyKind::Merlin),
            11 => Some(BuddyKind::Genie),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Sleep,
    Idle,
    Walk,
    Dance,
    Cheer,
    Hop,
}

pub enum BuddyEv {
    TimerDone,
    MusicStarted,
    Result(bool),
}

pub struct Buddy {
    pub hwnd: HWND,
    gfx: Gfx,
    kind: BuddyKind,
    pub shown: bool,
    /// Centre of the character, logical px from the monitor's left edge.
    x: f32,
    dir: f32,
    mode: Mode,
    until: Instant,
    tick: u32,
    target: f32,
    /// How many more walks of the current outing.
    plan: u8,
    bubble: Option<(String, Instant)>,
    rng: u32,
    low_warned: bool,
    last_ptr: Instant,
}

impl Buddy {
    pub fn new(hwnd: HWND, scale: f32) -> Option<Buddy> {
        let gfx = Gfx::new(
            ((BW + 2.0) * scale).ceil() as i32,
            ((BAR_H + 2.0) * scale).ceil() as i32,
            scale,
        )
        .ok()?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(1);
        let seed = (hwnd.0 as usize as u32) ^ 0x9E37_79B9 ^ nanos;
        Some(Buddy {
            hwnd,
            gfx,
            kind: BuddyKind::Cat,
            shown: false,
            x: 0.0,
            dir: -1.0,
            mode: Mode::Sleep,
            until: Instant::now(),
            tick: 0,
            target: 0.0,
            plan: 0,
            bubble: None,
            rng: seed | 1,
            low_warned: false,
            last_ptr: Instant::now(),
        })
    }

    /// Back to napping (used when a scene ends).
    pub fn sleep_now(&mut self) {
        self.mode = Mode::Sleep;
        self.bubble = None;
    }

    fn rnd(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    fn rnd_f(&mut self) -> f32 {
        (self.rnd() % 10_000) as f32 / 10_000.0
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

// ---------------------------------------------------------------- App side --

impl App {
    pub fn is_buddy(&self, h: HWND) -> bool {
        self.buddy.as_ref().map(|b| b.hwnd == h).unwrap_or(false)
    }

    pub fn rebuild_buddy(&mut self) {
        let Some(old) = self.buddy.take() else { return };
        self.buddy = Buddy::new(old.hwnd, self.scale);
        unsafe {
            let _ = ShowWindow(old.hwnd, SW_HIDE);
        }
        self.kill_timer(T_BUDDY);
        self.kill_timer(T_BWAKE);
    }

    /// Free stretch of the bar (logical px, centre positions) to the right of the
    /// notch and left of the status icons. None if there is no room.
    fn buddy_zone(&self) -> Option<(f32, f32)> {
        let b = self.bar.as_ref()?;
        let w = (self.mon.right - self.mon.left) as f32 / self.scale;
        let right = b.left_x.min(w) - 24.0;
        let left = w / 2.0 + 214.0;
        if right - left < 70.0 {
            None
        } else {
            Some((left, right))
        }
    }

    /// Show or hide the buddy to match the settings and the bar.
    pub fn sync_buddy(&mut self) {
        let zone = self.buddy_zone();
        let want = self.cfg.buddy != 0 && self.bar_visible() && zone.is_some();
        let kind = BuddyKind::from_id(self.cfg.buddy);
        let Some(b) = self.buddy.as_mut() else { return };
        if want {
            let (zl, zr) = zone.unwrap();
            if let Some(k) = kind {
                b.kind = k;
            }
            if !b.shown {
                b.shown = true;
                b.mode = Mode::Sleep;
                b.x = zl + (zr - zl) * b.rnd_f();
                b.dir = -1.0;
                unsafe {
                    let _ = ShowWindow(b.hwnd, SW_SHOWNOACTIVATE);
                }
                self.raise_windows();
                self.set_timer(T_BWAKE, 6000);
            } else {
                b.x = b.x.clamp(zl, zr);
            }
            self.buddy_render();
        } else if b.shown {
            b.shown = false;
            unsafe {
                let _ = ShowWindow(b.hwnd, SW_HIDE);
            }
            self.kill_timer(T_BUDDY);
            self.kill_timer(T_BWAKE);
        }
    }

    /// Tray menu: 0 = off, 1..=5 = character.
    pub fn set_buddy(&mut self, id: u8) {
        self.cfg.buddy = id;
        crate::store::save_config(&self.cfg);
        if id == 0 {
            self.stop_scene();
        }
        if let Some(b) = self.buddy.as_mut() {
            b.mode = Mode::Sleep;
            b.bubble = None;
        }
        self.kill_timer(T_BUDDY);
        self.sync_buddy();
        // greet the new buddy
        if id != 0 {
            self.buddy_event_bubble("Hi!", 2500);
        }
    }

    fn buddy_start_anim(&mut self) {
        if self.scene_active() {
            return;
        }
        self.kill_timer(T_BWAKE);
        self.set_timer(T_BUDDY, 66);
    }

    fn buddy_sleep_soon(&mut self) {
        let d = match self.buddy.as_mut() {
            Some(b) => 25_000 + b.rnd() % 45_000,
            None => return,
        };
        self.set_timer(T_BWAKE, d);
    }

    /// Wake-up timer: start a short outing (or a dance if music is playing).
    pub fn on_buddy_wake(&mut self) {
        self.kill_timer(T_BWAKE);
        if self.scene_active() {
            return;
        }
        let Some((zl, zr)) = self.buddy_zone() else { return };
        let playing = self.mstate.playing;
        let (_, _, _, _, hh, ..) = sys::local_time();
        let night = hh >= 23 || hh < 5;
        let Some(b) = self.buddy.as_mut() else { return };
        if !b.shown {
            return;
        }
        let now = Instant::now();
        if night {
            if b.rnd() % 3 == 0 {
                b.bubble = Some(("Bedtime?".into(), now + ms(4000)));
                b.mode = Mode::Idle;
                b.until = now + ms(4000);
                self.buddy_start_anim();
            } else {
                self.buddy_sleep_soon();
            }
            return;
        }
        if playing && b.rnd() % 2 == 0 {
            b.mode = Mode::Dance;
            b.until = now + ms(5500);
            b.bubble = Some(("\u{266A}".into(), now + ms(5500)));
        } else {
            b.plan = 1 + (b.rnd() % 2) as u8;
            b.target = zl + (zr - zl) * b.rnd_f();
            b.mode = Mode::Walk;
            let r = b.rnd();
            if let Some(q) = b.kind.quote(r) {
                b.bubble = Some((q.into(), now + ms(3600)));
            }
        }
        self.buddy_start_anim();
    }

    /// One animation frame (about 15 per second, only while awake).
    pub fn on_buddy_tick(&mut self) {
        let Some((zl, zr)) = self.buddy_zone() else {
            self.sync_buddy();
            return;
        };
        let now = Instant::now();
        let mut sleep = false;
        {
            let Some(b) = self.buddy.as_mut() else { return };
            if !b.shown {
                self.kill_timer(T_BUDDY);
                return;
            }
            b.tick = b.tick.wrapping_add(1);
            if matches!(&b.bubble, Some((_, u)) if now >= *u) {
                b.bubble = None;
            }
            b.x = b.x.clamp(zl, zr);
            let mut finished = false;
            match b.mode {
                Mode::Walk => {
                    let d = b.target - b.x;
                    if d.abs() <= 3.0 {
                        b.x = b.target;
                        b.mode = Mode::Idle;
                        b.until = now + ms(900 + (b.rnd() % 900) as u64);
                    } else {
                        b.dir = d.signum();
                        b.x += b.dir * 3.0;
                    }
                }
                Mode::Idle => {
                    if now >= b.until {
                        if b.plan > 0 {
                            b.plan -= 1;
                            let r = b.rnd();
                            if r % 3 == 0 {
                                if let Some(q) = b.kind.quote(r >> 4) {
                                    b.bubble = Some((q.into(), now + ms(3000)));
                                }
                            }
                            b.target = zl + (zr - zl) * b.rnd_f();
                            b.mode = Mode::Walk;
                        } else {
                            finished = true;
                        }
                    }
                }
                Mode::Dance | Mode::Cheer | Mode::Hop => {
                    if now >= b.until {
                        finished = true;
                    }
                }
                Mode::Sleep => finished = true,
            }
            if finished {
                match &b.bubble {
                    // keep awake until the bubble has been read
                    Some((_, u)) => {
                        b.mode = Mode::Idle;
                        b.until = *u;
                        b.plan = 0;
                    }
                    None => {
                        b.mode = Mode::Sleep;
                        sleep = true;
                    }
                }
            }
        }
        self.buddy_render();
        if sleep {
            self.kill_timer(T_BUDDY);
            self.buddy_sleep_soon();
        }
    }

    /// The pointer moved over the bar at logical x: come over and say hello.
    pub fn buddy_pointer(&mut self, px: f32) {
        let Some((zl, zr)) = self.buddy_zone() else { return };
        let now = Instant::now();
        let Some(b) = self.buddy.as_mut() else { return };
        if !b.shown || now.duration_since(b.last_ptr) < ms(180) {
            return;
        }
        // only when the pointer is near the buddy's stretch of the bar
        if px < zl - 60.0 || px > zr + 40.0 {
            return;
        }
        if matches!(b.mode, Mode::Dance | Mode::Cheer | Mode::Hop) {
            return;
        }
        b.last_ptr = now;
        let t = px.clamp(zl, zr);
        if (t - b.x).abs() > 10.0 {
            b.target = t;
            b.mode = Mode::Walk;
            b.plan = 0;
        } else if b.mode == Mode::Sleep {
            b.mode = Mode::Idle;
            b.until = now + ms(1500);
        } else if b.mode == Mode::Idle {
            b.until = now + ms(1500);
        }
        self.buddy_start_anim();
    }

    /// Click on the character: pet it. Returns true if the click was on it.
    pub fn buddy_click(&mut self, px: f32, py: f32) -> bool {
        let now = Instant::now();
        let Some(b) = self.buddy.as_mut() else { return false };
        if !b.shown || (px - b.x).abs() > 13.0 || !(3.0..=29.0).contains(&py) {
            return false;
        }
        b.mode = Mode::Hop;
        b.until = now + ms(1100);
        b.bubble = Some(("\u{2665}".into(), now + ms(1800)));
        self.buddy_start_anim();
        true
    }

    pub fn buddy_say(&mut self, text: &str, dur_ms: u64) {
        self.buddy_event_bubble(text, dur_ms);
    }

    fn buddy_event_bubble(&mut self, text: &str, dur_ms: u64) {
        let now = Instant::now();
        let Some(b) = self.buddy.as_mut() else { return };
        if !b.shown {
            return;
        }
        b.bubble = Some((text.to_string(), now + ms(dur_ms)));
        if matches!(b.mode, Mode::Sleep) {
            b.mode = Mode::Idle;
        }
        if b.mode == Mode::Idle {
            b.until = now + ms(dur_ms);
        }
        self.buddy_start_anim();
    }

    pub fn buddy_event(&mut self, ev: BuddyEv) {
        let now = Instant::now();
        let Some(b) = self.buddy.as_mut() else { return };
        if !b.shown {
            return;
        }
        match ev {
            BuddyEv::TimerDone => {
                b.mode = Mode::Cheer;
                b.until = now + ms(6000);
                b.bubble = Some(("Time's up!".into(), now + ms(6000)));
            }
            BuddyEv::MusicStarted => {
                if matches!(b.mode, Mode::Cheer | Mode::Hop) {
                    return;
                }
                b.mode = Mode::Dance;
                b.until = now + ms(5000);
                b.bubble = Some(("\u{266A}".into(), now + ms(5000)));
            }
            BuddyEv::Result(ok) => {
                if ok {
                    b.mode = Mode::Hop;
                    b.until = now + ms(1200);
                    b.bubble = Some(("Connected!".into(), now + ms(2800)));
                } else {
                    b.mode = Mode::Idle;
                    b.until = now + ms(2800);
                    b.bubble = Some(("Oops".into(), now + ms(2800)));
                }
            }
        }
        self.buddy_start_anim();
    }

    /// Called whenever the battery level is read.
    pub fn buddy_battery(&mut self, pct: u8, charging: bool) {
        let warn = {
            let Some(b) = self.buddy.as_mut() else { return };
            if charging || pct > 20 {
                b.low_warned = false;
                false
            } else if !b.low_warned {
                b.low_warned = true;
                true
            } else {
                false
            }
        };
        if warn {
            self.buddy_event_bubble("Low battery", 5000);
        }
    }

    pub fn buddy_render(&mut self) {
        let light = self.bar.as_ref().map(|b| b.light).unwrap_or(false);
        let th = theme(light);
        let scale = self.scale;
        let mon = self.mon;
        let cursor = unsafe {
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            p
        };
        let Some(b) = self.buddy.as_mut() else { return };
        if !b.shown {
            return;
        }
        let (pw, ph) = ((BW * scale).round() as i32, (BAR_H * scale).round() as i32);
        let cx_px = mon.left as f32 + b.x * scale;
        let cy_px = mon.top as f32 + 14.0 * scale;
        let look = (
            ((cursor.x as f32 - cx_px) / (70.0 * scale)).clamp(-1.0, 1.0),
            ((cursor.y as f32 - cy_px) / (70.0 * scale)).clamp(-1.0, 1.0),
        );
        let pose = pose_for(b.mode, b.tick, b.dir, look);
        let g = &b.gfx;
        if g.begin(pw, ph).is_err() {
            return;
        }
        if let Some(who) = b.kind.pixel_who() {
            let act = match b.mode {
                Mode::Sleep => cast::Act::Crouch,
                Mode::Idle => cast::Act::Stand,
                Mode::Walk => cast::Act::Walk,
                Mode::Dance | Mode::Cheer => cast::Act::Cheer,
                Mode::Hop => cast::Act::Jump,
            };
            let hop = if b.mode == Mode::Hop { ((b.tick as f32) * 0.5).sin().abs() * 5.0 } else { 0.0 };
            cast::draw_actor(
                g,
                &cast::Draw { who, x: SPR_X, y: 28.0 - hop, dir: b.dir, act, p: 0.0, t: b.tick as f32 * 0.066, charge: 0.0 },
            );
        } else {
            draw_sprite(g, b.kind, &pose, b.mode, b.tick);
        }
        if let Some((text, _)) = &b.bubble {
            let n = text.chars().count() as f32;
            let w = (n * 6.0 + 16.0).clamp(26.0, 320.0);
            let r = Rect::new(SPR_X - 16.0 - w, 5.0, w, 18.0);
            let bg = if light {
                Color(1.0, 1.0, 1.0, 0.97)
            } else {
                Color(0.17, 0.17, 0.2, 0.97)
            };
            g.fill_poly(
                &[(r.x + r.w - 1.0, 11.0), (r.x + r.w + 5.0, 14.0), (r.x + r.w - 1.0, 17.0)],
                bg,
            );
            g.fill_rrect(r, 9.0, bg);
            g.stroke_rrect(r, 9.0, th.line, 1.0);
            let c = if text == "\u{2665}" { Color(1.0, 0.4, 0.5, 1.0) } else { th.t1 };
            g.text(text, r, &g.fonts.tiny, c, Align::Center);
        }
        let _ = g.end();
        let _ = g.present(b.hwnd, (cx_px - SPR_X * scale).round() as i32, mon.top, pw, ph);
    }
}

/// One buddy frame for the demo video: `mode` 0 sleep, 1 idle, 2 walk, 3 dance, 4 cheer, 5 hop.
/// The character stands at SPR_X on a BW-wide strip.
#[cfg(test)]
pub fn demo_frame(g: &Gfx, id: u8, mode: u8, tick: u32, dir: f32, bubble: Option<&str>) {
    let kind = BuddyKind::from_id(id).expect("buddy id");
    let mode = [Mode::Sleep, Mode::Idle, Mode::Walk, Mode::Dance, Mode::Cheer, Mode::Hop][mode as usize % 6];
    let th = theme(false);
    if let Some(who) = kind.pixel_who() {
        let act = match mode {
            Mode::Sleep => cast::Act::Crouch,
            Mode::Idle => cast::Act::Stand,
            Mode::Walk => cast::Act::Walk,
            Mode::Dance | Mode::Cheer => cast::Act::Cheer,
            Mode::Hop => cast::Act::Jump,
        };
        cast::draw_actor(g, &cast::Draw { who, x: SPR_X, y: 28.0, dir, act, p: 0.0, t: tick as f32 * 0.066, charge: 0.0 });
    } else {
        draw_sprite(g, kind, &pose_for(mode, tick, dir, (0.0, 0.0)), mode, tick);
    }
    if let Some(text) = bubble {
        let n = text.chars().count() as f32;
        let w = (n * 6.0 + 16.0).clamp(26.0, 320.0);
        let r = Rect::new(SPR_X - 16.0 - w, 5.0, w, 18.0);
        let bg = Color(0.17, 0.17, 0.2, 0.97);
        g.fill_poly(&[(r.x + r.w - 1.0, 11.0), (r.x + r.w + 5.0, 14.0), (r.x + r.w - 1.0, 17.0)], bg);
        g.fill_rrect(r, 9.0, bg);
        g.stroke_rrect(r, 9.0, th.line, 1.0);
        g.text(text, r, &g.fonts.tiny, th.t1, Align::Center);
    }
}

// ------------------------------------------------------------------ drawing --

#[derive(Clone, Copy, PartialEq)]
enum Eyes {
    Open,
    Closed,
    Happy,
}

#[derive(Clone, Copy)]
struct Pose {
    dir: f32,
    /// pixels the body is lifted off the ground (negative = squashed down)
    bob: f32,
    /// foot lift, left / right
    lift: [f32; 2],
    eyes: Eyes,
    look: (f32, f32),
    /// 0..1: how high the arms are raised
    arms: f32,
    /// -1..1 swing of the tail / cape
    wag: f32,
    open_mouth: bool,
}

fn pose_for(mode: Mode, tick: u32, dir: f32, look: (f32, f32)) -> Pose {
    let t = tick as f32;
    let blink = tick % 46 < 3;
    let mut p = Pose {
        dir,
        bob: 0.0,
        lift: [0.0, 0.0],
        eyes: if blink { Eyes::Closed } else { Eyes::Open },
        look,
        arms: 0.0,
        wag: (t * 0.25).sin(),
        open_mouth: false,
    };
    match mode {
        Mode::Sleep => {
            p.bob = -1.2;
            p.eyes = Eyes::Closed;
            p.look = (0.0, 0.0);
            p.wag = 0.0;
        }
        Mode::Idle => {
            p.bob = (t * 0.2).sin() * 0.4;
        }
        Mode::Walk => {
            let s = t * 0.85;
            p.bob = s.sin().abs() * 1.4;
            p.lift = [s.sin().max(0.0) * 2.4, (-s.sin()).max(0.0) * 2.4];
            p.arms = 0.0;
            p.wag = (t * 0.5).sin();
        }
        Mode::Dance => {
            let s = t * 0.7;
            p.bob = s.sin().abs() * 2.4;
            p.arms = 0.5 + 0.5 * (s * 1.0).sin();
            p.eyes = Eyes::Happy;
            p.open_mouth = true;
            p.lift = [s.sin().max(0.0) * 1.5, (-s.sin()).max(0.0) * 1.5];
            p.wag = (t * 0.6).sin();
            // turn left / right to the beat
            p.dir = if (tick / 7) % 2 == 0 { 1.0 } else { -1.0 };
        }
        Mode::Cheer => {
            let s = t * 0.6;
            p.bob = s.sin().abs() * 2.6;
            p.arms = 1.0;
            p.eyes = Eyes::Happy;
            p.open_mouth = true;
            p.wag = (t * 0.7).sin();
        }
        Mode::Hop => {
            let s = (t * 0.45).min(std::f32::consts::PI);
            p.bob = s.sin() * 3.0;
            p.eyes = Eyes::Happy;
            p.arms = 0.4;
        }
    }
    p
}

fn palette(kind: BuddyKind) -> (Color, Color) {
    match kind {
        BuddyKind::Blob => (Color(0.55, 0.78, 1.0, 1.0), Color(0.38, 0.6, 0.9, 1.0)),
        BuddyKind::Cat => (Color(0.99, 0.80, 0.62, 1.0), Color(0.9, 0.6, 0.4, 1.0)),
        BuddyKind::Bunny => (Color(0.97, 0.94, 0.97, 1.0), Color(0.82, 0.76, 0.84, 1.0)),
        BuddyKind::Hero => (Color(0.18, 0.70, 0.76, 1.0), Color(0.1, 0.5, 0.58, 1.0)),
        BuddyKind::Robot => (Color(0.74, 0.8, 0.9, 1.0), Color(0.5, 0.57, 0.7, 1.0)),
        BuddyKind::Ronin | BuddyKind::Courier | BuddyKind::Necro => (Color(0.5, 0.5, 0.5, 1.0), Color(0.3, 0.3, 0.3, 1.0)),
        BuddyKind::Clippy => (Color(0.82, 0.85, 0.90, 1.0), Color(0.48, 0.52, 0.58, 1.0)),
        BuddyKind::Merlin => (Color(0.24, 0.35, 0.72, 1.0), Color(0.96, 0.80, 0.25, 1.0)),
        BuddyKind::Genie => (Color(0.28, 0.72, 0.95, 1.0), Color(0.95, 0.78, 0.20, 1.0)),
    }
}

fn draw_genie(g: &Gfx, cx: f32, bob: f32, p: &Pose, mode: Mode, tick: u32, body: Color, dark: Color, ink: Color) {
    let _t = tick as f32;
    let smoke_wag = p.wag * 2.2;

    // Ground shadow
    g.fill_rrect(
        Rect::new(cx - 8.0, 27.0, 16.0, 2.4),
        1.2,
        Color(0.0, 0.0, 0.0, (0.24 - bob.max(0.0) * 0.03).max(0.08)),
    );

    // Magic Brass Lamp on the bar floor
    let lamp_y = 25.0;
    // Lamp body
    g.fill_rrect(Rect::new(cx - 5.0, lamp_y, 9.5, 3.2), 1.6, dark);
    // Lamp handle
    g.line((cx + 4.2, lamp_y + 0.5), (cx + 6.2, lamp_y + 2.0), 1.2, dark);
    // Lamp spout pointing left
    g.line((cx - 5.0, lamp_y + 1.2), (cx - 7.5, lamp_y - 0.4), 1.4, dark);

    // Swirling mystical smoke tail from spout to torso
    let smoke_col = Color(body.0, body.1, body.2, 0.65);
    g.fill_circle(cx - 6.5 + smoke_wag * 0.2, 23.0 - bob * 0.5, 2.0, smoke_col);
    g.fill_circle(cx - 4.0 + smoke_wag * 0.5, 20.5 - bob * 0.7, 2.8, smoke_col);
    g.fill_circle(cx - 1.2 + smoke_wag * 0.8, 18.0 - bob * 0.9, 3.6, smoke_col);

    // Cyan Torso
    g.fill_rrect(Rect::new(cx - 5.5, 13.5 - bob, 11.0, 6.2), 3.0, body);

    // Folded muscular arms
    g.fill_rrect(Rect::new(cx - 6.6, 16.2 - bob, 13.2, 3.8), 1.9, body);
    // Golden wrist cuffs
    g.fill_rrect(Rect::new(cx - 6.8, 16.2 - bob, 2.2, 3.8), 0.8, dark);
    g.fill_rrect(Rect::new(cx + 4.6, 16.2 - bob, 2.2, 3.8), 0.8, dark);

    // Pointed elf/genie ears
    g.fill_poly(
        &[
            (cx - 4.2, 10.5 - bob),
            (cx - 7.2, 8.5 - bob),
            (cx - 4.0, 8.5 - bob),
        ],
        body,
    );
    g.fill_poly(
        &[
            (cx + 4.2, 10.5 - bob),
            (cx + 7.2, 8.5 - bob),
            (cx + 4.0, 8.5 - bob),
        ],
        body,
    );

    // Head
    g.fill_circle(cx, 10.5 - bob, 4.6, body);

    // Goatee beard
    g.line((cx, 14.8 - bob), (cx + p.dir * 1.2, 17.0 - bob), 1.2, ink);

    // Eyes
    let lx = p.look.0 * 1.2;
    let ly = p.look.1 * 1.0;
    for sx in [-2.2f32, 2.2f32] {
        let ex = cx + sx;
        let ey = 10.0 - bob;
        match p.eyes {
            Eyes::Closed => {
                g.line((ex - 1.3, ey), (ex + 1.3, ey), 1.0, ink);
            }
            Eyes::Happy => {
                g.line((ex - 1.3, ey + 0.6), (ex, ey - 0.7), 1.1, ink);
                g.line((ex, ey - 0.7), (ex + 1.3, ey + 0.6), 1.1, ink);
            }
            Eyes::Open => {
                g.fill_circle(ex, ey, 1.8, Color::white(1.0));
                g.fill_circle(ex + lx * 0.5, ey + ly * 0.5, 1.1, ink);
            }
        }
    }

    // Purple Turban
    let turban_col = Color(0.55, 0.20, 0.65, 1.0);
    g.fill_rrect(Rect::new(cx - 5.6, 5.8 - bob, 11.2, 4.2), 2.1, turban_col);
    // Brooch with ruby
    g.fill_circle(cx, 7.8 - bob, 1.5, dark);
    g.fill_circle(cx, 7.8 - bob, 0.8, Color(0.92, 0.15, 0.22, 1.0));
    // Feather plume
    g.line((cx, 6.8 - bob), (cx - 1.6 + p.wag * 0.4, 1.8 - bob), 1.5, Color::white(0.95));

    // Mode-specific extras
    match mode {
        Mode::Sleep => {
            let c = Color(0.6, 0.65, 0.8, 0.9);
            g.text("z", Rect::new(cx + 7.0, 6.0, 9.0, 9.0), &g.fonts.tiny, c, Align::Left);
            g.text("Z", Rect::new(cx + 11.0, 0.5, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
        }
        Mode::Dance => {
            let c = Color(0.95, 0.55, 0.95, 0.95);
            let side = if (tick / 5) % 2 == 0 { 12.0 } else { -20.0 };
            g.text("\u{266A}", Rect::new(cx + side, 3.0, 12.0, 12.0), &g.fonts.small, c, Align::Left);
        }
        Mode::Cheer => {
            let c = Color(1.0, 0.82, 0.2, 0.95);
            let a = if (tick / 4) % 2 == 0 { 0.0 } else { 3.0 };
            g.text("\u{2726}", Rect::new(cx - 20.0, 2.0 + a, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
            g.text("\u{2726}", Rect::new(cx + 12.0, 5.0 - a, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
        }
        _ => {}
    }
}

fn draw_merlin(g: &Gfx, cx: f32, bob: f32, p: &Pose, mode: Mode, tick: u32, body: Color, dark: Color, ink: Color) {
    let t = tick as f32;
    let wand_dir = p.dir;

    // Ground shadow
    g.fill_rrect(
        Rect::new(cx - 7.5, 27.0, 15.0, 2.4),
        1.2,
        Color(0.0, 0.0, 0.0, (0.24 - bob.max(0.0) * 0.03).max(0.08)),
    );

    // Wizard Robe (flowing blue gown)
    g.fill_poly(
        &[
            (cx - 5.5, 16.5 - bob),
            (cx + 5.5, 16.5 - bob),
            (cx + 7.5, 26.5 - bob),
            (cx - 7.5, 26.5 - bob),
        ],
        body,
    );
    // Gold trim at hem
    g.fill_rrect(Rect::new(cx - 7.5, 25.5 - bob, 15.0, 1.4), 0.7, dark);

    // Wand in hand
    let arm_raise = p.arms * 4.0;
    let hand = (cx + wand_dir * 5.8, 18.0 - bob - arm_raise);
    let wand_tip = (cx + wand_dir * (11.0 + arm_raise * 0.5), 11.0 - bob - arm_raise * 1.5);
    g.line(hand, wand_tip, 1.6, Color(0.55, 0.35, 0.18, 1.0));

    // Wand glowing star tip
    let star_pulse = 1.0 + 0.3 * (t * 0.25).sin();
    g.fill_circle(wand_tip.0, wand_tip.1, 1.8 * star_pulse, dark);
    if mode != Mode::Sleep {
        g.line(
            (wand_tip.0 - 3.0 * star_pulse, wand_tip.1),
            (wand_tip.0 + 3.0 * star_pulse, wand_tip.1),
            0.8,
            Color::white(0.9),
        );
        g.line(
            (wand_tip.0, wand_tip.1 - 3.0 * star_pulse),
            (wand_tip.0, wand_tip.1 + 3.0 * star_pulse),
            0.8,
            Color::white(0.9),
        );
    }

    // Head (face)
    let skin = Color(1.0, 0.86, 0.74, 1.0);
    g.fill_circle(cx, 13.0 - bob, 4.8, skin);

    // Eyes
    let lx = p.look.0 * 1.0;
    let ly = p.look.1 * 0.8;
    for sx in [-2.0f32, 2.0f32] {
        let ex = cx + sx;
        let ey = 12.0 - bob;
        match p.eyes {
            Eyes::Closed => {
                g.line((ex - 1.2, ey), (ex + 1.2, ey), 1.0, ink);
            }
            Eyes::Happy => {
                g.line((ex - 1.2, ey + 0.6), (ex, ey - 0.7), 1.0, ink);
                g.line((ex, ey - 0.7), (ex + 1.2, ey + 0.6), 1.0, ink);
            }
            Eyes::Open => {
                g.fill_circle(ex + lx, ey + ly, 1.1, ink);
            }
        }
    }

    // Fluffy White Beard & Moustache
    let beard_col = Color::white(0.96);
    g.fill_circle(cx - 2.6, 16.0 - bob, 2.4, beard_col);
    g.fill_circle(cx + 2.6, 16.0 - bob, 2.4, beard_col);
    g.fill_circle(cx, 17.5 - bob, 3.0, beard_col);
    g.fill_poly(
        &[
            (cx - 4.5, 14.5 - bob),
            (cx + 4.5, 14.5 - bob),
            (cx, 21.0 - bob),
        ],
        beard_col,
    );

    // Wizard Conical Hat
    let hat_tip_x = cx + 1.0 + p.wag * 0.8;
    let hat_tip_y = 1.0 - bob;
    // Brim
    g.fill_rrect(Rect::new(cx - 8.2, 10.0 - bob, 16.4, 2.4), 1.2, body);
    // Gold band
    g.fill_rrect(Rect::new(cx - 6.2, 9.0 - bob, 12.4, 1.4), 0.7, dark);
    // Cone
    g.fill_poly(
        &[
            (cx - 5.8, 10.0 - bob),
            (cx + 5.5, 10.0 - bob),
            (hat_tip_x, hat_tip_y),
        ],
        body,
    );
    // Star ornament on hat
    g.fill_circle(cx + 0.2, 6.0 - bob, 1.3, dark);

    // Mode-specific extras
    match mode {
        Mode::Sleep => {
            let c = Color(0.6, 0.65, 0.8, 0.9);
            g.text("z", Rect::new(cx + 7.0, 6.0, 9.0, 9.0), &g.fonts.tiny, c, Align::Left);
            g.text("Z", Rect::new(cx + 11.0, 0.5, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
        }
        Mode::Dance => {
            let c = Color(0.95, 0.55, 0.95, 0.95);
            let side = if (tick / 5) % 2 == 0 { 12.0 } else { -20.0 };
            g.text("\u{266A}", Rect::new(cx + side, 3.0, 12.0, 12.0), &g.fonts.small, c, Align::Left);
        }
        Mode::Cheer => {
            let c = Color(1.0, 0.82, 0.2, 0.95);
            let a = if (tick / 4) % 2 == 0 { 0.0 } else { 3.0 };
            g.text("\u{2728}", Rect::new(cx - 20.0, 2.0 + a, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
            g.text("\u{2728}", Rect::new(cx + 12.0, 5.0 - a, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
        }
        _ => {}
    }
}

fn draw_clippy(g: &Gfx, cx: f32, bob: f32, p: &Pose, mode: Mode, tick: u32, body: Color, dark: Color, ink: Color) {
    let flex = p.wag * 1.5;

    // Ground shadow
    g.fill_rrect(
        Rect::new(cx - 7.0, 27.0, 14.0, 2.2),
        1.1,
        Color(0.0, 0.0, 0.0, (0.22 - bob.max(0.0) * 0.03).max(0.08)),
    );

    // Wire loops:
    let r_out = Rect::new(cx - 5.5 + flex * 0.3, 8.0 - bob, 11.0, 18.5);
    g.stroke_rrect(Rect::new(r_out.x + 0.5, r_out.y + 0.5, r_out.w, r_out.h), 5.5, dark, 1.8);
    g.stroke_rrect(r_out, 5.5, body, 1.8);

    let r_in = Rect::new(cx - 3.0 + flex * 0.15, 12.0 - bob, 6.0, 12.5);
    g.stroke_rrect(Rect::new(r_in.x + 0.5, r_in.y + 0.5, r_in.w, r_in.h), 3.0, dark, 1.5);
    g.stroke_rrect(r_in, 3.0, body, 1.5);

    g.line((cx + 3.0 + flex * 0.15, 14.5 - bob), (cx + 3.0 + flex * 0.15, 23.5 - bob), 1.5, body);

    // Googly Eyes
    let eye_y = 13.5 - bob;
    let lx = p.look.0 * 1.4;
    let ly = p.look.1 * 1.2;

    for (sx, look_offset) in [(-3.0f32, -0.2f32), (3.0f32, 0.2f32)] {
        let ex = cx + sx + flex * 0.2;
        g.fill_circle(ex, eye_y, 3.3, Color::white(1.0));
        g.fill_circle(ex, eye_y, 3.3, Color(0.0, 0.0, 0.0, 0.2));
        g.fill_circle(ex, eye_y, 3.0, Color::white(1.0));

        match p.eyes {
            Eyes::Closed => {
                g.line((ex - 2.0, eye_y), (ex + 2.0, eye_y), 1.2, ink);
            }
            Eyes::Happy => {
                g.line((ex - 1.8, eye_y + 0.8), (ex, eye_y - 1.0), 1.3, ink);
                g.line((ex, eye_y - 1.0), (ex + 1.8, eye_y + 0.8), 1.3, ink);
            }
            Eyes::Open => {
                let px = (ex + lx + look_offset).clamp(ex - 1.6, ex + 1.6);
                let py = (eye_y + ly).clamp(eye_y - 1.5, eye_y + 1.5);
                g.fill_circle(px, py, 1.5, ink);
                g.fill_circle(px - 0.5, py - 0.5, 0.5, Color::white(1.0));
            }
        }
    }

    // Eyebrows
    let brow_wag = if mode == Mode::Dance || mode == Mode::Cheer {
        ((tick as f32) * 0.4).sin() * 1.0
    } else {
        0.0
    };
    match p.eyes {
        Eyes::Happy => {
            g.line((cx - 5.5, 8.8 - bob - brow_wag), (cx - 1.2, 7.8 - bob - brow_wag), 1.3, ink);
            g.line((cx + 1.2, 7.8 - bob - brow_wag), (cx + 5.5, 8.8 - bob - brow_wag), 1.3, ink);
        }
        Eyes::Closed => {
            g.line((cx - 5.0, 10.0 - bob), (cx - 1.2, 10.0 - bob), 1.1, dark);
            g.line((cx + 1.2, 10.0 - bob), (cx + 5.0, 10.0 - bob), 1.1, dark);
        }
        Eyes::Open => {
            g.line((cx - 5.6, 9.8 - bob - brow_wag), (cx - 1.0, 8.6 - bob - brow_wag), 1.3, ink);
            g.line((cx + 1.0, 9.2 - bob - brow_wag), (cx + 5.6, 10.2 - bob - brow_wag), 1.3, ink);
        }
    }

    match mode {
        Mode::Sleep => {
            let c = Color(0.6, 0.65, 0.8, 0.9);
            g.text("z", Rect::new(cx + 7.0, 6.0, 9.0, 9.0), &g.fonts.tiny, c, Align::Left);
            g.text("Z", Rect::new(cx + 11.0, 0.5, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
        }
        Mode::Dance => {
            let c = Color(0.95, 0.55, 0.95, 0.95);
            let side = if (tick / 5) % 2 == 0 { 12.0 } else { -20.0 };
            g.text("\u{266A}", Rect::new(cx + side, 3.0, 12.0, 12.0), &g.fonts.small, c, Align::Left);
        }
        Mode::Cheer => {
            let c = Color(1.0, 0.82, 0.2, 0.95);
            let a = if (tick / 4) % 2 == 0 { 0.0 } else { 3.0 };
            g.text("\u{2726}", Rect::new(cx - 20.0, 2.0 + a, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
            g.text("\u{2726}", Rect::new(cx + 12.0, 5.0 - a, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
        }
        _ => {}
    }
}

fn draw_sprite(g: &Gfx, kind: BuddyKind, p: &Pose, mode: Mode, tick: u32) {
    let cx = SPR_X;
    let bob = p.bob;
    let (body, dark) = palette(kind);
    let ink = Color(0.13, 0.12, 0.17, 1.0);

    if kind == BuddyKind::Clippy {
        draw_clippy(g, cx, bob, p, mode, tick, body, dark, ink);
        return;
    }
    if kind == BuddyKind::Merlin {
        draw_merlin(g, cx, bob, p, mode, tick, body, dark, ink);
        return;
    }
    if kind == BuddyKind::Genie {
        draw_genie(g, cx, bob, p, mode, tick, body, dark, ink);
        return;
    }
    let d = p.dir;
    let s = -d; // "behind" the character
    let bob = p.bob;
    let (body, dark) = palette(kind);
    let ink = Color(0.13, 0.12, 0.17, 1.0);
    let pink = Color(1.0, 0.55, 0.7, 1.0);

    // ground shadow
    g.fill_rrect(
        Rect::new(cx - 7.5, 27.0, 15.0, 2.4),
        1.2,
        Color(0.0, 0.0, 0.0, (0.24 - bob.max(0.0) * 0.03).max(0.08)),
    );

    // things behind the body
    let hy = match kind {
        BuddyKind::Bunny => 15.2,
        _ => 12.5,
    } - bob;
    let hr = match kind {
        BuddyKind::Bunny => 7.0,
        _ => 8.0,
    };
    match kind {
        BuddyKind::Hero => {
            let flap = p.wag * 2.0;
            let y0 = 19.0 - bob;
            g.fill_poly(
                &[
                    (cx, y0),
                    (cx + s * 5.0, y0 + 0.5),
                    (cx + s * (10.0 + flap), 26.0 - bob * 0.3),
                    (cx + s * 2.0, 25.5 - bob * 0.3),
                ],
                Color(0.95, 0.5, 0.22, 1.0),
            );
        }
        BuddyKind::Cat => {
            let y0 = 24.0 - bob;
            let tip = (cx + s * 11.0, 17.5 - bob - p.wag * 2.0);
            g.line((cx + s * 5.0, y0), tip, 2.6, body);
            g.fill_circle(tip.0, tip.1, 1.7, dark);
        }
        BuddyKind::Bunny => {
            g.fill_circle(cx + s * 6.5, 23.5 - bob, 2.6, Color::white(1.0));
        }
        _ => {}
    }

    // feet
    for (i, lx) in [-5.2f32, 1.0].iter().enumerate() {
        g.fill_rrect(
            Rect::new(cx + lx, 24.6 - p.lift[i], 4.4, 3.2),
            1.6,
            dark,
        );
    }

    // body
    let body_rect = Rect::new(cx - 6.0, 18.5 - bob, 12.0, 8.4);
    g.fill_rrect(body_rect, 4.0, body);
    match kind {
        BuddyKind::Hero => {
            g.fill_rrect(Rect::new(cx - 6.0, 23.2 - bob, 12.0, 1.6), 0.8, Color(0.98, 0.8, 0.25, 1.0));
            g.fill_circle(cx, 21.0 - bob, 1.8, Color(0.98, 0.8, 0.25, 1.0));
        }
        BuddyKind::Cat | BuddyKind::Bunny => {
            g.fill_rrect(Rect::new(cx - 3.2, 20.5 - bob, 6.4, 5.0), 2.5, Color(1.0, 1.0, 1.0, 0.55));
        }
        BuddyKind::Robot => {
            g.fill_rrect(Rect::new(cx - 3.0, 21.0 - bob, 6.0, 3.0), 1.0, ink);
            g.fill_circle(cx - 1.4, 22.5 - bob, 0.7, Color(0.4, 0.95, 1.0, 1.0));
            g.fill_circle(cx + 1.4, 22.5 - bob, 0.7, Color(1.0, 0.5, 0.5, 1.0));
        }
        _ => {}
    }

    // arms
    if p.arms > 0.02 {
        let up = p.arms;
        for sx in [-1.0f32, 1.0] {
            let sh = (cx + sx * 6.0, 21.0 - bob);
            let hand = (cx + sx * (8.2 + up * 0.8), 21.5 - bob - up * 8.0);
            g.line(sh, hand, 2.2, body);
            g.fill_circle(hand.0, hand.1, 1.4, dark);
        }
    }

    // ears and other head extras (behind the head)
    match kind {
        BuddyKind::Cat => {
            for sx in [-1.0f32, 1.0] {
                g.fill_poly(
                    &[
                        (cx + sx * 7.6, hy - 2.0),
                        (cx + sx * 6.6, hy - 9.4),
                        (cx + sx * 1.8, hy - 6.6),
                    ],
                    body,
                );
                g.fill_poly(
                    &[
                        (cx + sx * 6.4, hy - 3.6),
                        (cx + sx * 5.9, hy - 7.4),
                        (cx + sx * 3.4, hy - 5.8),
                    ],
                    pink,
                );
            }
        }
        BuddyKind::Bunny => {
            let tilt = p.wag * 0.8;
            for (sx, extra) in [(-1.0f32, tilt), (1.0, -tilt)] {
                let x = cx + sx * 3.4 - 1.8 + extra * 0.4;
                g.fill_rrect(Rect::new(x, hy - 12.6, 3.6, 9.0), 1.8, body);
                g.fill_rrect(Rect::new(x + 0.8, hy - 11.2, 2.0, 6.2), 1.0, pink);
            }
        }
        BuddyKind::Robot => {
            g.line((cx, hy - 6.4), (cx, hy - 8.4), 1.2, dark);
            g.fill_circle(cx, hy - 9.0, 1.4, Color(1.0, 0.4, 0.4, 1.0));
        }
        BuddyKind::Blob => {
            // a little sprout
            g.line((cx, hy - 7.5), (cx + 1.0, hy - 9.0), 1.1, Color(0.35, 0.7, 0.4, 1.0));
            g.fill_circle(cx + 2.2, hy - 9.3, 1.5, Color(0.45, 0.82, 0.5, 1.0));
        }
        _ => {}
    }

    // head
    if kind == BuddyKind::Robot {
        g.fill_rrect(Rect::new(cx - 8.0, hy - 6.4, 16.0, 13.0), 3.4, body);
        g.fill_rrect(Rect::new(cx - 6.4, hy - 4.6, 12.8, 9.0), 2.6, ink);
    } else {
        let head = if kind == BuddyKind::Hero { Color(1.0, 0.85, 0.72, 1.0) } else { body };
        g.fill_circle(cx, hy, hr, head);
    }
    if kind == BuddyKind::Hero {
        // hair
        g.fill_rrect(Rect::new(cx - 7.4, hy - 8.0, 14.8, 5.0), 2.6, Color(0.25, 0.18, 0.2, 1.0));
        // mask
        g.fill_rrect(Rect::new(cx - 7.4, hy - 2.6, 14.8, 5.2), 2.4, Color(0.12, 0.12, 0.2, 1.0));
    }

    // face
    let lx = p.look.0 * 1.2;
    let ly = p.look.1 * 0.9;
    let ey = hy + 0.6 + ly;
    for sx in [-1.0f32, 1.0] {
        let ex = cx + sx * 3.3 + lx;
        match (kind, p.eyes) {
            (BuddyKind::Robot, Eyes::Open) => {
                g.fill_rrect(Rect::new(ex - 1.3, ey - 1.8, 2.6, 3.4), 1.0, Color(0.4, 0.95, 1.0, 1.0));
            }
            (BuddyKind::Robot, Eyes::Closed) => {
                g.line((ex - 1.5, ey), (ex + 1.5, ey), 1.0, Color(0.4, 0.95, 1.0, 1.0));
            }
            (BuddyKind::Robot, Eyes::Happy) => {
                g.line((ex - 1.5, ey + 0.8), (ex, ey - 0.8), 1.0, Color(0.4, 0.95, 1.0, 1.0));
                g.line((ex, ey - 0.8), (ex + 1.5, ey + 0.8), 1.0, Color(0.4, 0.95, 1.0, 1.0));
            }
            (BuddyKind::Hero, Eyes::Open) => {
                g.fill_rrect(Rect::new(ex - 1.7, ey - 1.0, 3.4, 2.2), 1.0, Color::white(1.0));
                g.fill_circle(ex + lx * 0.5, ey, 0.7, ink);
            }
            (BuddyKind::Hero, Eyes::Closed) => {
                g.line((ex - 1.5, ey), (ex + 1.5, ey), 1.0, Color::white(0.9));
            }
            (BuddyKind::Hero, Eyes::Happy) => {
                g.line((ex - 1.5, ey + 0.6), (ex, ey - 0.8), 1.0, Color::white(1.0));
                g.line((ex, ey - 0.8), (ex + 1.5, ey + 0.6), 1.0, Color::white(1.0));
            }
            (_, Eyes::Open) => {
                g.fill_circle(ex, ey, 1.45, ink);
                g.fill_circle(ex + 0.5, ey - 0.5, 0.5, Color::white(0.9));
            }
            (_, Eyes::Closed) => {
                g.line((ex - 1.4, ey), (ex + 1.4, ey), 1.0, ink);
            }
            (_, Eyes::Happy) => {
                g.line((ex - 1.4, ey + 0.7), (ex, ey - 0.8), 1.0, ink);
                g.line((ex, ey - 0.8), (ex + 1.4, ey + 0.7), 1.0, ink);
            }
        }
        // blush
        if matches!(kind, BuddyKind::Blob | BuddyKind::Cat | BuddyKind::Bunny) {
            g.fill_circle(cx + sx * 5.4 + lx * 0.5, ey + 2.4, 1.5, Color(1.0, 0.5, 0.6, 0.45));
        }
    }
    // nose / mouth
    let mx = cx + lx * 0.7;
    let my = hy + 4.6 + ly * 0.5;
    let mouth_col = if kind == BuddyKind::Robot { Color(0.4, 0.95, 1.0, 1.0) } else if kind == BuddyKind::Hero { ink } else { ink };
    if kind == BuddyKind::Robot {
        // the robot's mouth sits on its screen
        if p.open_mouth {
            g.fill_rrect(Rect::new(mx - 1.6, hy + 2.4, 3.2, 1.6), 0.8, mouth_col);
        } else if p.eyes != Eyes::Closed {
            g.line((mx - 1.4, hy + 3.0), (mx + 1.4, hy + 3.0), 0.9, mouth_col);
        }
    } else if matches!(kind, BuddyKind::Cat) {
        g.fill_circle(mx, hy + 2.6 + ly * 0.3, 0.8, pink);
        if p.open_mouth {
            g.fill_circle(mx, my, 1.3, ink);
        } else {
            g.line((mx - 1.5, my - 0.2), (mx, my + 0.7), 0.8, ink);
            g.line((mx, my + 0.7), (mx + 1.5, my - 0.2), 0.8, ink);
        }
        for sx in [-1.0f32, 1.0] {
            g.line((cx + sx * 6.2, hy + 3.0), (cx + sx * 9.6, hy + 2.2), 0.7, Color(0.3, 0.25, 0.3, 0.7));
            g.line((cx + sx * 6.2, hy + 4.0), (cx + sx * 9.6, hy + 4.8), 0.7, Color(0.3, 0.25, 0.3, 0.7));
        }
        // the bow
        let bx = cx + 5.2;
        let by = hy - 6.6;
        g.fill_poly(&[(bx, by), (bx - 4.2, by - 2.4), (bx - 4.2, by + 2.4)], pink);
        g.fill_poly(&[(bx, by), (bx + 4.2, by - 2.4), (bx + 4.2, by + 2.4)], pink);
        g.fill_circle(bx, by, 1.5, Color(1.0, 0.75, 0.85, 1.0));
    } else if p.open_mouth {
        g.fill_circle(mx, my, 1.4, ink);
    } else if p.eyes != Eyes::Closed || mode != Mode::Sleep {
        g.line((mx - 1.5, my - 0.3), (mx, my + 0.7), 0.8, ink);
        g.line((mx, my + 0.7), (mx + 1.5, my - 0.3), 0.8, ink);
    }

    // little extras per mode
    match mode {
        Mode::Sleep => {
            let up = (tick as f32 * 0.0).sin();
            let c = Color(0.6, 0.65, 0.8, 0.9);
            g.text("z", Rect::new(cx + 7.0, 6.0 + up, 9.0, 9.0), &g.fonts.tiny, c, Align::Left);
            g.text("Z", Rect::new(cx + 11.0, 0.5, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
        }
        Mode::Dance => {
            let c = Color(0.95, 0.55, 0.95, 0.95);
            let side = if (tick / 5) % 2 == 0 { 12.0 } else { -20.0 };
            g.text("\u{266A}", Rect::new(cx + side, 3.0, 12.0, 12.0), &g.fonts.small, c, Align::Left);
        }
        Mode::Cheer => {
            let c = Color(1.0, 0.82, 0.2, 0.95);
            let a = if (tick / 4) % 2 == 0 { 0.0 } else { 3.0 };
            g.text("\u{2726}", Rect::new(cx - 20.0, 2.0 + a, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
            g.text("\u{2726}", Rect::new(cx + 12.0, 5.0 - a, 10.0, 10.0), &g.fonts.tiny, c, Align::Left);
        }
        _ => {}
    }
}

#[cfg(test)]
mod clippy_tests {
    use super::*;

    #[test]
    fn clippy_id_and_quotes() {
        assert_eq!(BuddyKind::from_id(9), Some(BuddyKind::Clippy));
        let q = BuddyKind::Clippy.quote(0).expect("clippy quote");
        assert!(q.contains("code"));
        assert!(BuddyKind::Clippy.quote(1).is_some());
    }

    #[test]
    fn merlin_id_and_quotes() {
        assert_eq!(BuddyKind::from_id(10), Some(BuddyKind::Merlin));
        let q = BuddyKind::Merlin.quote(0).expect("merlin quote");
        assert!(q.contains("stars"));
        assert!(BuddyKind::Merlin.quote(1).is_some());
    }

    #[test]
    fn genie_id_and_quotes() {
        assert_eq!(BuddyKind::from_id(11), Some(BuddyKind::Genie));
        let q = BuddyKind::Genie.quote(0).expect("genie quote");
        assert!(q.contains("wish"));
        assert!(BuddyKind::Genie.quote(1).is_some());
    }
}
