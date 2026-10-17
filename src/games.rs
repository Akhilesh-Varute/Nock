//! The notch arcade: a small hub with four mouse-only games that fit the 544x180
//! strip (brick breaker, a fruit slicer, a Viking runner, and the flying game).
//! Everything is drawn from shapes; rounds are short, the speed climbs, and each
//! game keeps a best score.

use crate::game::{Game, State as FlightState, H, W};
use crate::gfx::*;

pub const NAMES: [&str; 4] = ["Thor Flight", "Brick Breaker", "Slicer", "Viking Run"];

// ---------------------------------------------------------------- helpers --

struct Rng(u32);

impl Rng {
    fn new() -> Rng {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(3);
        Rng(nanos | 1)
    }
    /// 0..1
    fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 % 100_000) as f32 / 100_000.0
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
}

fn lerp(a: f32, b: f32, u: f32) -> f32 {
    a + (b - a) * u
}

fn mixc(a: Color, b: Color, u: f32) -> Color {
    Color(lerp(a.0, b.0, u), lerp(a.1, b.1, u), lerp(a.2, b.2, u), lerp(a.3, b.3, u))
}

struct Part {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    max: f32,
    size: f32,
    col: Color,
}

fn burst(parts: &mut Vec<Part>, rng: &mut Rng, x: f32, y: f32, n: usize, col: Color, speed: f32) {
    for _ in 0..n {
        let a = rng.range(0.0, std::f32::consts::TAU);
        let s = rng.range(0.3, 1.0) * speed;
        let life = rng.range(0.35, 0.8);
        parts.push(Part { x, y, vx: a.cos() * s, vy: a.sin() * s, life, max: life, size: rng.range(1.5, 3.2), col });
    }
}

fn step_parts(parts: &mut Vec<Part>, dt: f32, gravity: f32) {
    for p in parts.iter_mut() {
        p.life -= dt;
        p.vy += gravity * dt;
        p.x += p.vx * dt;
        p.y += p.vy * dt;
    }
    parts.retain(|p| p.life > 0.0);
}

fn draw_parts(g: &Gfx, parts: &[Part], ox: f32, oy: f32, ca: f32) {
    for p in parts {
        let k = (p.life / p.max).clamp(0.0, 1.0);
        g.fill_rrect(Rect::new(ox + p.x - p.size / 2.0, oy + p.y - p.size / 2.0, p.size, p.size), 0.8, p.col.fade(k * ca));
    }
}

fn field_bg(g: &Gfx, ox: f32, oy: f32, ca: f32, top: Color, bot: Color) {
    let r = 14.0f32;
    let mut y = 0.0f32;
    while y < H {
        let edge = if y < r { r - y } else if y > H - r { y - (H - r) } else { 0.0 };
        let inset = if edge > 0.0 { r - (r * r - edge * edge).max(0.0).sqrt() } else { 0.0 };
        g.fill_rrect(Rect::new(ox + inset, oy + y, W - 2.0 * inset, 1.4), 0.0, mixc(top, bot, y / H).fade(ca));
        y += 1.0;
    }
}

fn shake_off(t: f32, shake: f32) -> (f32, f32) {
    ((t * 83.0).sin() * shake * 3.0, (t * 71.0).cos() * shake * 3.0)
}

fn centered(g: &Gfx, s: &str, ox: f32, y: f32, f: &IDWriteTextFormatRef, col: Color) {
    g.text(s, Rect::new(ox, y, W, 22.0), f, col, Align::Center);
}

/// Large headline text: a taller box so the glyphs are not clipped.
fn headline(g: &Gfx, s: &str, ox: f32, y: f32, f: &IDWriteTextFormatRef, col: Color) {
    g.text(s, Rect::new(ox, y - 14.0, W, 50.0), f, col, Align::Center);
}

// Fonts live in Gfx as IDWriteTextFormat; keep one alias so signatures stay short.
type IDWriteTextFormatRef = windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;

// ----------------------------------------------------------------- input ---

/// Pointer state in field coordinates (0..W, 0..H).
#[derive(Clone, Copy, Default)]
pub struct Input {
    pub x: f32,
    pub y: f32,
    pub down: bool,
}

// -------------------------------------------------------------- breakout ---

struct Brick {
    x: f32,
    y: f32,
    hp: u8,
    col: Color,
    flash: f32,
}

struct Ball {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

struct Cap {
    x: f32,
    y: f32,
    kind: u8, // 0 wide, 1 multi-ball, 2 slow, 3 extra life
}

const BR_COLS: i32 = 12;
const BR_W: f32 = 40.0;
const BR_H: f32 = 10.0;
const PAD_Y: f32 = H - 16.0;

pub struct Breakout {
    paddle_x: f32,
    paddle_w: f32,
    balls: Vec<Ball>,
    bricks: Vec<Brick>,
    caps: Vec<Cap>,
    parts: Vec<Part>,
    lives: u32,
    score: u32,
    level: u32,
    combo: u32,
    serve: bool,
    over: bool,
    best: u32,
    wide_t: f32,
    slow_t: f32,
    banner: f32,
    shake: f32,
    t: f32,
    rng: Rng,
    sfx: Vec<(u32, u32)>,
}

impl Breakout {
    fn new(best: u32) -> Breakout {
        let mut b = Breakout {
            paddle_x: W / 2.0,
            paddle_w: 64.0,
            balls: Vec::new(),
            bricks: Vec::new(),
            caps: Vec::new(),
            parts: Vec::new(),
            lives: 3,
            score: 0,
            level: 1,
            combo: 0,
            serve: true,
            over: false,
            best,
            wide_t: 0.0,
            slow_t: 0.0,
            banner: 1.4,
            shake: 0.0,
            t: 0.0,
            rng: Rng::new(),
            sfx: Vec::new(),
        };
        b.build();
        b.reserve();
        b
    }

    fn build(&mut self) {
        self.bricks.clear();
        let rows = (2 + self.level as i32).min(6);
        let pal = [
            Color(1.0, 0.35, 0.50, 1.0),
            Color(1.0, 0.60, 0.25, 1.0),
            Color(1.0, 0.88, 0.30, 1.0),
            Color(0.40, 0.90, 0.50, 1.0),
            Color(0.30, 0.75, 1.0, 1.0),
            Color(0.70, 0.50, 1.0, 1.0),
        ];
        let left = (W - (BR_COLS as f32 * (BR_W + 4.0) - 4.0)) / 2.0;
        for r in 0..rows {
            for c in 0..BR_COLS {
                if self.level > 1 && self.rng.f() < 0.10 {
                    continue;
                }
                self.bricks.push(Brick {
                    x: left + c as f32 * (BR_W + 4.0),
                    y: 30.0 + r as f32 * (BR_H + 4.0),
                    hp: if self.level >= 3 && r == 0 { 2 } else { 1 },
                    col: pal[r as usize % 6],
                    flash: 0.0,
                });
            }
        }
    }

    /// Put a fresh ball on the paddle.
    fn reserve(&mut self) {
        self.balls = vec![Ball { x: self.paddle_x, y: PAD_Y - 6.0, vx: 0.0, vy: 0.0 }];
        self.serve = true;
    }

    fn speed(&self) -> f32 {
        (205.0 + 16.0 * (self.level as f32 - 1.0)).min(330.0) * if self.slow_t > 0.0 { 0.68 } else { 1.0 }
    }

    fn click(&mut self) {
        if self.over {
            let best = self.best;
            *self = Breakout::new(best);
        } else if self.serve {
            let a = self.rng.range(-0.5, 0.5);
            let s = self.speed();
            if let Some(b) = self.balls.first_mut() {
                b.vx = a.sin() * s;
                b.vy = -a.cos() * s;
            }
            self.serve = false;
        }
    }

    fn step(&mut self, dt: f32, inp: &Input) -> bool {
        self.t += dt;
        self.banner = (self.banner - dt).max(0.0);
        self.shake = (self.shake - dt * 3.0).max(0.0);
        self.wide_t = (self.wide_t - dt).max(0.0);
        self.slow_t = (self.slow_t - dt).max(0.0);
        step_parts(&mut self.parts, dt, 260.0);
        for b in &mut self.bricks {
            b.flash = (b.flash - dt * 5.0).max(0.0);
        }
        let tw = if self.wide_t > 0.0 { 108.0 } else { 64.0 };
        self.paddle_w += (tw - self.paddle_w) * (1.0 - (-10.0 * dt).exp());
        let half = self.paddle_w / 2.0;
        self.paddle_x += (inp.x.clamp(half, W - half) - self.paddle_x) * (1.0 - (-30.0 * dt).exp());
        if self.over {
            return false;
        }
        if self.serve {
            if let Some(b) = self.balls.first_mut() {
                b.x = self.paddle_x;
                b.y = PAD_Y - 6.0;
            }
            return false;
        }
        let speed = self.speed();
        let (mut new_caps, mut broke): (Vec<Cap>, Vec<(f32, f32, Color)>) = (Vec::new(), Vec::new());
        let mut paddle_hit = false;
        for b in &mut self.balls {
            let n = (b.vx.hypot(b.vy)).max(1.0);
            b.vx = b.vx / n * speed;
            b.vy = b.vy / n * speed;
            b.x += b.vx * dt;
            b.y += b.vy * dt;
            if b.x < 4.0 {
                b.x = 4.0;
                b.vx = b.vx.abs();
            } else if b.x > W - 4.0 {
                b.x = W - 4.0;
                b.vx = -b.vx.abs();
            }
            if b.y < 4.0 {
                b.y = 4.0;
                b.vy = b.vy.abs();
            }
            if b.vy > 0.0 && b.y + 4.0 >= PAD_Y && b.y - 4.0 <= PAD_Y + 7.0 && (b.x - self.paddle_x).abs() <= half + 4.0 {
                let a = ((b.x - self.paddle_x) / half).clamp(-1.0, 1.0) * 1.05;
                b.vx = a.sin() * speed;
                b.vy = -a.cos() * speed;
                b.y = PAD_Y - 4.5;
                paddle_hit = true;
            }
            for br in self.bricks.iter_mut() {
                if br.hp == 0 {
                    continue;
                }
                let cx = b.x.clamp(br.x, br.x + BR_W);
                let cy = b.y.clamp(br.y, br.y + BR_H);
                let (dx, dy) = (b.x - cx, b.y - cy);
                if dx * dx + dy * dy < 16.0 {
                    if dx.abs() > dy.abs() {
                        b.vx = if dx > 0.0 { b.vx.abs() } else { -b.vx.abs() };
                    } else {
                        b.vy = if dy > 0.0 { b.vy.abs() } else { -b.vy.abs() };
                    }
                    br.hp -= 1;
                    br.flash = 1.0;
                    if br.hp == 0 {
                        broke.push((br.x + BR_W / 2.0, br.y + BR_H / 2.0, br.col));
                        if self.rng.f() < 0.17 {
                            new_caps.push(Cap { x: br.x + BR_W / 2.0, y: br.y, kind: (self.rng.f() * 4.0) as u8 % 4 });
                        }
                    }
                    break;
                }
            }
        }
        if paddle_hit {
            self.combo = 0;
            self.sfx.push((300, 25));
        }
        for (x, y, col) in broke {
            self.score += 10 * (1 + self.combo / 3);
            self.combo += 1;
            burst(&mut self.parts, &mut self.rng, x, y, 9, col, 110.0);
            self.sfx.push((480 + (self.combo.min(12) * 35), 22));
        }
        self.bricks.retain(|b| b.hp > 0);
        self.caps.extend(new_caps);
        // falling power-ups
        let mut caught = Vec::new();
        for c in &mut self.caps {
            c.y += 90.0 * dt;
            if c.y + 5.0 >= PAD_Y && c.y <= PAD_Y + 8.0 && (c.x - self.paddle_x).abs() < half + 9.0 {
                caught.push(c.kind);
                c.y = H + 50.0;
            }
        }
        self.caps.retain(|c| c.y < H + 10.0);
        for k in caught {
            self.sfx.push((760, 60));
            match k {
                0 => self.wide_t = 10.0,
                1 => {
                    let (x, y, vx, vy) = self.balls.first().map(|b| (b.x, b.y, b.vx, b.vy)).unwrap_or((self.paddle_x, PAD_Y - 10.0, 0.0, -speed));
                    for rot in [-0.5f32, 0.5] {
                        let (s, c) = rot.sin_cos();
                        self.balls.push(Ball { x, y, vx: vx * c - vy * s, vy: vx * s + vy * c });
                    }
                }
                2 => self.slow_t = 8.0,
                _ => self.lives = (self.lives + 1).min(5),
            }
        }
        // lost balls
        self.balls.retain(|b| b.y < H + 8.0);
        if self.balls.is_empty() {
            self.lives = self.lives.saturating_sub(1);
            self.shake = 1.0;
            self.combo = 0;
            self.sfx.push((140, 160));
            if self.lives == 0 {
                self.over = true;
            } else {
                self.reserve();
            }
        }
        if self.bricks.is_empty() && !self.over {
            self.level += 1;
            self.build();
            self.reserve();
            self.banner = 1.6;
            self.sfx.push((660, 70));
            self.sfx.push((880, 90));
        }
        if self.score > self.best {
            self.best = self.score;
            return self.over; // saved when the round ends
        }
        false
    }

    fn draw(&self, g: &Gfx, ox: f32, oy: f32, ca: f32) {
        let f = &g.fonts;
        let (sx, sy) = shake_off(self.t, self.shake);
        let (ox, oy) = (ox + sx, oy + sy);
        field_bg(g, ox, oy, ca, Color(0.04, 0.05, 0.12, 1.0), Color(0.09, 0.06, 0.20, 1.0));
        for b in &self.bricks {
            let col = mixc(b.col, Color(1.0, 1.0, 1.0, 1.0), b.flash * 0.7);
            g.fill_rrect(Rect::new(ox + b.x, oy + b.y, BR_W, BR_H), 2.5, col.fade(ca));
            g.fill_rrect(Rect::new(ox + b.x + 2.0, oy + b.y + 1.0, BR_W - 4.0, 2.0), 1.0, Color::white(0.30 * ca));
            if b.hp > 1 {
                g.stroke_rrect(Rect::new(ox + b.x, oy + b.y, BR_W, BR_H), 2.5, Color(0.9, 0.95, 1.0, 0.9 * ca), 1.4);
            }
        }
        for c in &self.caps {
            let (col, label) = match c.kind {
                0 => (Color(0.35, 0.8, 1.0, 1.0), "W"),
                1 => (Color(1.0, 0.7, 0.3, 1.0), "M"),
                2 => (Color(0.6, 0.9, 0.5, 1.0), "S"),
                _ => (Color(1.0, 0.4, 0.55, 1.0), "+"),
            };
            let r = Rect::new(ox + c.x - 9.0, oy + c.y - 5.0, 18.0, 10.0);
            g.fill_rrect(r, 5.0, col.fade(ca));
            g.text(label, r, &f.tiny, Color(0.0, 0.0, 0.0, ca), Align::Center);
        }
        let pw = self.paddle_w;
        g.fill_rrect(Rect::new(ox + self.paddle_x - pw / 2.0 - 3.0, oy + PAD_Y - 3.0, pw + 6.0, 13.0), 6.0, Color(0.5, 0.8, 1.0, 0.14 * ca));
        g.fill_rrect(Rect::new(ox + self.paddle_x - pw / 2.0, oy + PAD_Y, pw, 7.0), 3.5, Color(0.88, 0.96, 1.0, ca));
        for b in &self.balls {
            g.line((ox + b.x - b.vx * 0.03, oy + b.y - b.vy * 0.03), (ox + b.x, oy + b.y), 4.0, Color(1.0, 1.0, 1.0, 0.18 * ca));
            g.fill_circle(ox + b.x, oy + b.y, 6.5, Color(0.7, 0.9, 1.0, 0.22 * ca));
            g.fill_circle(ox + b.x, oy + b.y, 4.0, Color(1.0, 1.0, 1.0, ca));
        }
        draw_parts(g, &self.parts, ox, oy, ca);
        g.text(&self.score.to_string(), Rect::new(ox + 14.0, oy + 4.0, 110.0, 22.0), &f.body, Color::white(0.95 * ca), Align::Left);
        if self.combo >= 3 {
            g.text(&format!("x{}", 1 + self.combo / 3), Rect::new(ox + 70.0, oy + 4.0, 50.0, 22.0), &f.body, Color(1.0, 0.8, 0.3, ca), Align::Left);
        }
        for i in 0..self.lives {
            g.fill_circle(ox + W - 18.0 - i as f32 * 14.0, oy + 15.0, 4.6, Color(1.0, 0.4, 0.55, ca));
        }
        if self.wide_t > 0.0 || self.slow_t > 0.0 {
            let s = format!("{}{}", if self.wide_t > 0.0 { "WIDE " } else { "" }, if self.slow_t > 0.0 { "SLOW" } else { "" });
            g.text(&s, Rect::new(ox, oy + 6.0, W, 18.0), &f.tiny, Color::white(0.6 * ca), Align::Center);
        }
        if self.banner > 0.0 {
            headline(g, &format!("Level {}", self.level), ox, oy + 96.0, &f.big, Color::white((self.banner.min(1.0)) * 0.9 * ca));
        }
        if self.serve && !self.over {
            centered(g, "Click to launch", ox, oy + 124.0, &f.small, Color::white(0.7 * ca));
        }
        if self.over {
            g.fill_rrect(Rect::new(ox, oy, W, H), 14.0, Color(0.0, 0.0, 0.0, 0.5 * ca));
            headline(g, &format!("Game over \u{00B7} {}", self.score), ox, oy + 62.0, &f.big, Color::white(ca));
            centered(g, &format!("best {} \u{00B7} click to retry", self.best), ox, oy + 106.0, &f.small, Color::white(0.8 * ca));
        }
    }
}

// ---------------------------------------------------------------- slicer ---

struct Obj {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    r: f32,
    kind: u8, // 0..3 fruit, 4 bomb, 5 golden
    rot: f32,
    spin: f32,
}

struct Half {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    r: f32,
    rot: f32,
    spin: f32,
    kind: u8,
    life: f32,
}

pub struct Slicer {
    objs: Vec<Obj>,
    halves: Vec<Half>,
    parts: Vec<Part>,
    trail: Vec<(f32, f32, f32)>,
    lives: u32,
    score: u32,
    spawn_t: f32,
    t: f32,
    over: bool,
    best: u32,
    flash: f32,
    shake: f32,
    miss_flash: f32,
    prev: (f32, f32),
    rng: Rng,
    sfx: Vec<(u32, u32)>,
    last_combo: u32,
    combo_t: f32,
}

const FRUIT: [(Color, Color); 4] = [
    (Color(1.0, 0.55, 0.15, 1.0), Color(1.0, 0.85, 0.45, 1.0)),
    (Color(0.9, 0.15, 0.2, 1.0), Color(1.0, 0.85, 0.8, 1.0)),
    (Color(0.45, 0.8, 0.2, 1.0), Color(0.8, 1.0, 0.55, 1.0)),
    (Color(0.55, 0.2, 0.7, 1.0), Color(0.95, 0.6, 0.85, 1.0)),
];

impl Slicer {
    fn new(best: u32) -> Slicer {
        Slicer {
            objs: Vec::new(),
            halves: Vec::new(),
            parts: Vec::new(),
            trail: Vec::new(),
            lives: 3,
            score: 0,
            spawn_t: 0.6,
            t: 0.0,
            over: false,
            best,
            flash: 0.0,
            shake: 0.0,
            miss_flash: 0.0,
            prev: (W / 2.0, H / 2.0),
            rng: Rng::new(),
            sfx: Vec::new(),
            last_combo: 0,
            combo_t: 0.0,
        }
    }

    fn click(&mut self) {
        if self.over {
            let best = self.best;
            *self = Slicer::new(best);
        }
    }

    fn spawn(&mut self) {
        let g = 430.0;
        let bomb_p = (0.10 + self.t * 0.002).min(0.24);
        let n = 1 + (self.rng.f() * (1.0 + (self.t / 25.0).min(2.0))) as usize;
        for _ in 0..n {
            let kind = if self.rng.f() < bomb_p {
                4
            } else if self.rng.f() < 0.07 {
                5
            } else {
                (self.rng.f() * 4.0) as u8 % 4
            };
            let apex = self.rng.range(105.0, 160.0);
            let vy = -(2.0 * g * apex).sqrt();
            self.objs.push(Obj {
                x: self.rng.range(70.0, W - 70.0),
                y: H + 16.0,
                vx: self.rng.range(-70.0, 70.0),
                vy,
                r: if kind == 4 { 11.0 } else { 13.0 },
                kind,
                rot: self.rng.range(0.0, 6.28),
                spin: self.rng.range(-4.0, 4.0),
            });
        }
    }

    fn step(&mut self, dt: f32, inp: &Input) -> bool {
        self.t += dt;
        self.flash = (self.flash - dt * 3.0).max(0.0);
        self.shake = (self.shake - dt * 3.0).max(0.0);
        self.miss_flash = (self.miss_flash - dt * 2.5).max(0.0);
        self.combo_t = (self.combo_t - dt).max(0.0);
        step_parts(&mut self.parts, dt, 300.0);
        for h in &mut self.halves {
            h.life -= dt;
            h.vy += 430.0 * dt;
            h.x += h.vx * dt;
            h.y += h.vy * dt;
            h.rot += h.spin * dt;
        }
        self.halves.retain(|h| h.life > 0.0);
        for p in &mut self.trail {
            p.2 -= dt;
        }
        self.trail.retain(|p| p.2 > 0.0);
        let (mx, my) = (inp.x, inp.y);
        let moved = (mx - self.prev.0).hypot(my - self.prev.1);
        let speed = moved / dt.max(0.001);
        if moved > 0.5 {
            self.trail.push((mx, my, 0.22));
        }
        let prev = self.prev;
        self.prev = (mx, my);
        if self.over {
            return false;
        }
        self.spawn_t -= dt;
        if self.spawn_t <= 0.0 {
            self.spawn();
            self.spawn_t = (1.25 - self.t * 0.008).max(0.55) * self.rng.range(0.8, 1.2);
        }
        let mut sliced: Vec<usize> = Vec::new();
        for (i, o) in self.objs.iter_mut().enumerate() {
            o.vy += 430.0 * dt;
            o.x += o.vx * dt;
            o.y += o.vy * dt;
            o.rot += o.spin * dt;
            if speed > 380.0 && seg_dist(prev, (mx, my), (o.x, o.y)) < o.r + 3.0 {
                sliced.push(i);
            }
        }
        let mut hits = 0u32;
        let mut points = 0u32;
        let mut blew = false;
        for &i in sliced.iter().rev() {
            let o = self.objs.remove(i);
            if o.kind == 4 {
                blew = true;
                burst(&mut self.parts, &mut self.rng, o.x, o.y, 26, Color(1.0, 0.6, 0.2, 1.0), 220.0);
                continue;
            }
            hits += 1;
            points += if o.kind == 5 { 5 } else { 1 };
            let (skin, flesh) = if o.kind == 5 { (Color(1.0, 0.82, 0.25, 1.0), Color(1.0, 0.95, 0.6, 1.0)) } else { FRUIT[o.kind as usize] };
            burst(&mut self.parts, &mut self.rng, o.x, o.y, 11, flesh, 150.0);
            burst(&mut self.parts, &mut self.rng, o.x, o.y, 5, skin, 120.0);
            let (dx, dy) = ((mx - prev.0), (my - prev.1));
            let n = dx.hypot(dy).max(1.0);
            let (nx, ny) = (-dy / n, dx / n);
            for s in [-1.0f32, 1.0] {
                self.halves.push(Half {
                    x: o.x,
                    y: o.y,
                    vx: o.vx * 0.5 + nx * s * 90.0,
                    vy: o.vy * 0.4 + ny * s * 90.0 - 30.0,
                    r: o.r,
                    rot: dy.atan2(dx) + if s > 0.0 { 0.0 } else { std::f32::consts::PI },
                    spin: s * 3.0,
                    kind: o.kind,
                    life: 1.1,
                });
            }
        }
        if blew {
            self.over = true;
            self.flash = 1.0;
            self.shake = 1.0;
            self.sfx.push((120, 260));
            return self.score >= self.best && self.score > 0;
        }
        if hits > 0 {
            self.score += points + if hits >= 3 { hits } else { 0 };
            self.sfx.push((600 + 80 * hits.min(5), 28));
            if hits >= 2 {
                self.last_combo = hits;
                self.combo_t = 0.9;
            }
        }
        // fruit that fell out of sight costs a life
        let mut missed = 0;
        self.objs.retain(|o| {
            let gone = o.y > H + 24.0 && o.vy > 0.0;
            if gone && o.kind != 4 {
                missed += 1;
            }
            !gone
        });
        if missed > 0 {
            self.lives = self.lives.saturating_sub(missed);
            self.miss_flash = 1.0;
            self.sfx.push((170, 120));
            if self.lives == 0 {
                self.over = true;
                self.sfx.push((110, 240));
            }
        }
        if self.score > self.best {
            self.best = self.score;
            return self.over;
        }
        false
    }

    fn draw(&self, g: &Gfx, ox: f32, oy: f32, ca: f32) {
        let f = &g.fonts;
        let (sx, sy) = shake_off(self.t, self.shake);
        let (ox, oy) = (ox + sx, oy + sy);
        field_bg(g, ox, oy, ca, Color(0.07, 0.06, 0.14, 1.0), Color(0.16, 0.08, 0.20, 1.0));
        // faint dojo planks
        for i in 0..8 {
            g.fill_rrect(Rect::new(ox + i as f32 * 70.0, oy + 120.0, 1.0, 60.0), 0.0, Color::white(0.03 * ca));
        }
        for h in &self.halves {
            let (skin, flesh) = if h.kind == 5 { (Color(1.0, 0.82, 0.25, 1.0), Color(1.0, 0.95, 0.6, 1.0)) } else { FRUIT[h.kind as usize % 4] };
            let fade = (h.life / 1.1).min(1.0) * ca;
            for (rr, col) in [(h.r, skin), (h.r * 0.78, flesh)] {
                let pts: Vec<(f32, f32)> = (0..=10)
                    .map(|k| {
                        let a = h.rot + std::f32::consts::PI * k as f32 / 10.0;
                        (ox + h.x + a.cos() * rr, oy + h.y + a.sin() * rr)
                    })
                    .collect();
                g.fill_poly(&pts, col.fade(fade));
            }
        }
        for o in &self.objs {
            self.draw_obj(g, ox + o.x, oy + o.y, o, ca);
        }
        draw_parts(g, &self.parts, ox, oy, ca);
        // blade trail
        for (i, w) in self.trail.windows(2).enumerate() {
            let k = (w[1].2 / 0.22).clamp(0.0, 1.0);
            let _ = i;
            g.line((ox + w[0].0, oy + w[0].1), (ox + w[1].0, oy + w[1].1), 1.5 + 4.0 * k, Color(0.7, 0.95, 1.0, 0.8 * k * ca));
            g.line((ox + w[0].0, oy + w[0].1), (ox + w[1].0, oy + w[1].1), 1.0, Color(1.0, 1.0, 1.0, k * ca));
        }
        g.text(&self.score.to_string(), Rect::new(ox + 14.0, oy + 4.0, 110.0, 22.0), &f.body, Color::white(0.95 * ca), Align::Left);
        if self.combo_t > 0.0 {
            centered(g, &format!("{}x combo!", self.last_combo), ox, oy + 52.0, &f.body, Color(1.0, 0.85, 0.3, self.combo_t.min(0.5) * 2.0 * ca));
        }
        for i in 0..3u32 {
            let alive = i < self.lives;
            let c = if alive { Color(1.0, 0.4, 0.55, ca) } else { Color(1.0, 1.0, 1.0, 0.18 * ca) };
            g.fill_circle(ox + W - 18.0 - i as f32 * 14.0, oy + 15.0, 4.6, c);
        }
        if self.miss_flash > 0.0 {
            g.fill_rrect(Rect::new(ox, oy, W, H), 14.0, Color(1.0, 0.2, 0.3, 0.16 * self.miss_flash * ca));
        }
        if self.flash > 0.0 {
            g.fill_rrect(Rect::new(ox, oy, W, H), 14.0, Color(1.0, 0.9, 0.7, 0.7 * self.flash * ca));
        }
        if self.t < 4.0 && !self.over {
            centered(g, "Swipe fast through the fruit \u{00B7} avoid the bombs", ox, oy + 146.0, &f.small, Color::white((1.0 - self.t / 4.0) * 0.75 * ca));
        }
        if self.over {
            g.fill_rrect(Rect::new(ox, oy, W, H), 14.0, Color(0.0, 0.0, 0.0, 0.5 * ca));
            headline(g, &format!("Game over \u{00B7} {}", self.score), ox, oy + 62.0, &f.big, Color::white(ca));
            centered(g, &format!("best {} \u{00B7} click to retry", self.best), ox, oy + 106.0, &f.small, Color::white(0.8 * ca));
        }
    }

    fn draw_obj(&self, g: &Gfx, x: f32, y: f32, o: &Obj, ca: f32) {
        match o.kind {
            4 => {
                g.fill_circle(x, y, o.r, Color(0.12, 0.12, 0.16, ca));
                g.fill_circle(x - 3.0, y - 3.5, 3.0, Color::white(0.18 * ca));
                g.line((x + 3.0, y - o.r + 1.0), (x + 7.0, y - o.r - 5.0), 1.6, Color(0.7, 0.55, 0.3, ca));
                if (self.t * 14.0).sin() > 0.0 {
                    g.fill_circle(x + 7.5, y - o.r - 5.5, 2.4, Color(1.0, 0.85, 0.3, ca));
                } else {
                    g.fill_circle(x + 7.5, y - o.r - 5.5, 1.6, Color(1.0, 0.45, 0.2, ca));
                }
                g.fill_circle(x, y + 1.0, 3.2, Color(0.9, 0.2, 0.2, 0.9 * ca));
            }
            5 => {
                g.fill_circle(x, y, o.r + 5.0, Color(1.0, 0.85, 0.3, 0.18 * ca));
                g.fill_circle(x, y, o.r, Color(1.0, 0.82, 0.25, ca));
                g.fill_circle(x - 4.0, y - 4.0, 3.4, Color(1.0, 1.0, 0.85, 0.7 * ca));
                let sp = (self.t * 6.0 + o.rot).sin() * 0.5 + 0.5;
                g.line((x + 9.0, y - 11.0), (x + 9.0, y - 11.0 - 4.0 * sp), 1.0, Color(1.0, 1.0, 1.0, ca));
            }
            k => {
                let (skin, flesh) = FRUIT[k as usize % 4];
                g.fill_circle(x, y, o.r, skin.fade(ca));
                g.fill_circle(x - 4.0, y - 4.5, 3.6, flesh.fade(0.55 * ca));
                g.fill_poly(&[(x, y - o.r + 1.0), (x + 7.0, y - o.r - 3.0), (x + 5.0, y - o.r + 3.0)], Color(0.3, 0.7, 0.25, ca));
            }
        }
    }
}

/// Distance from point `p` to the segment a-b.
fn seg_dist(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let u = if len2 < 1e-6 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    ((a.0 + u * dx - p.0).hypot(a.1 + u * dy - p.1)).abs()
}

// ------------------------------------------------------------------- run ---

const GROUND: f32 = H - 30.0;
const RUN_X: f32 = 92.0;

struct Obs {
    x: f32,
    kind: u8, // 0 rock, 1 spikes, 2 pillar, 3 bird
    w: f32,
    h: f32,
    lift: f32, // bottom edge above the ground
    scored: bool,
}

struct Coin {
    x: f32,
    y: f32,
    got: bool,
}

pub struct Runner {
    lift: f32,
    vy: f32,
    jumps: u8,
    obs: Vec<Obs>,
    coins: Vec<Coin>,
    parts: Vec<Part>,
    speed: f32,
    dist: f32,
    coin_n: u32,
    bonus: u32,
    t: f32,
    over: bool,
    best: u32,
    shake: f32,
    next_x: f32,
    toast: f32,
    toast_s: &'static str,
    rng: Rng,
    sfx: Vec<(u32, u32)>,
    was_down: bool,
}

impl Runner {
    fn new(best: u32) -> Runner {
        Runner {
            lift: 0.0,
            vy: 0.0,
            jumps: 2,
            obs: Vec::new(),
            coins: Vec::new(),
            parts: Vec::new(),
            speed: 210.0,
            dist: 0.0,
            coin_n: 0,
            bonus: 0,
            t: 0.0,
            over: false,
            best,
            shake: 0.0,
            next_x: 420.0,
            toast: 0.0,
            toast_s: "",
            rng: Rng::new(),
            sfx: Vec::new(),
            was_down: false,
        }
    }

    fn score(&self) -> u32 {
        (self.dist / 10.0) as u32 + self.coin_n * 5 + self.bonus
    }

    fn jump(&mut self) {
        if self.jumps > 0 {
            let first = self.jumps == 2;
            self.vy = if first { -470.0 } else { -430.0 };
            self.jumps -= 1;
            self.sfx.push((if first { 420 } else { 540 }, 30));
            let (x, y) = (RUN_X, GROUND - self.lift);
            burst(&mut self.parts, &mut self.rng, x, y, 5, Color(0.85, 0.9, 1.0, 1.0), 60.0);
        }
    }

    fn click(&mut self) {
        if self.over {
            let best = self.best;
            *self = Runner::new(best);
        }
    }

    fn step(&mut self, dt: f32, inp: &Input) -> bool {
        self.t += dt;
        self.shake = (self.shake - dt * 3.0).max(0.0);
        self.toast = (self.toast - dt).max(0.0);
        step_parts(&mut self.parts, dt, 300.0);
        // a press (also when held down) jumps; holding while rising jumps higher
        if inp.down && !self.was_down && !self.over {
            self.jump();
        }
        self.was_down = inp.down;
        if self.over {
            return false;
        }
        self.speed = (210.0 + self.t * 4.5).min(430.0);
        self.dist += self.speed * dt;
        let g = if inp.down && self.vy < 0.0 { 820.0 } else { 1500.0 };
        self.vy += g * dt;
        self.lift -= self.vy * dt;
        if self.lift <= 0.0 {
            if self.lift < -0.01 && self.vy > 120.0 {
                burst(&mut self.parts, &mut self.rng, RUN_X, GROUND, 4, Color(0.85, 0.9, 1.0, 1.0), 50.0);
            }
            self.lift = 0.0;
            self.vy = 0.0;
            self.jumps = 2;
        }
        // spawn
        while self.next_x < self.dist + W + 60.0 {
            self.spawn_at(self.next_x);
            let gap = self.speed * self.rng.range(0.85, 1.5) + 60.0;
            self.next_x += gap;
        }
        let d = self.dist;
        // obstacles live in world units (x - dist = screen offset from RUN_X)
        let mut crashed = false;
        let (px0, px1) = (RUN_X - 7.0, RUN_X + 7.0);
        let (py0, py1) = (self.lift, self.lift + 28.0); // bottom, top above the ground
        for o in &mut self.obs {
            let sx = o.x - d + RUN_X;
            let (ox0, ox1) = (sx + 2.0, sx + o.w - 2.0);
            let (oy0, oy1) = (o.lift + 2.0, o.lift + o.h - 2.0);
            if px1 > ox0 && px0 < ox1 && py1 > oy0 && py0 < oy1 {
                crashed = true;
            }
            if !o.scored && sx + o.w < px0 {
                o.scored = true;
                if o.kind != 3 && py0 - oy1 >= 0.0 && py0 - oy1 < 6.0 {
                    self.bonus += 2;
                    self.toast = 0.8;
                    self.toast_s = "close call +2";
                }
            }
        }
        for c in &mut self.coins {
            let sx = c.x - d + RUN_X;
            if !c.got && (sx - RUN_X).abs() < 12.0 && (GROUND - c.y - (GROUND - self.lift - 14.0)).abs() < 18.0 {
                c.got = true;
                self.coin_n += 1;
                self.sfx.push((900, 18));
                burst(&mut self.parts, &mut self.rng, sx, c.y, 5, Color(1.0, 0.85, 0.3, 1.0), 80.0);
            }
        }
        self.obs.retain(|o| o.x - d + RUN_X > -60.0);
        self.coins.retain(|c| c.x - d + RUN_X > -20.0 && !c.got);
        if crashed {
            self.over = true;
            self.shake = 1.0;
            self.sfx.push((130, 220));
            burst(&mut self.parts, &mut self.rng, RUN_X, GROUND - self.lift - 14.0, 22, Color(1.0, 0.5, 0.3, 1.0), 200.0);
            if self.score() > self.best {
                self.best = self.score();
                return true;
            }
        }
        false
    }

    fn spawn_at(&mut self, wx: f32) {
        let roll = self.rng.f();
        let (kind, w, h, lift) = if roll < 0.32 {
            (0, 18.0, 15.0, 0.0)
        } else if roll < 0.55 {
            (1, 26.0, 12.0, 0.0)
        } else if roll < 0.75 {
            (2, 14.0, 52.0, 0.0)
        } else {
            (3, 22.0, 13.0, 31.0)
        };
        self.obs.push(Obs { x: wx, kind, w, h, lift, scored: false });
        // a little arc of coins above ground obstacles, or a low row under birds
        if kind == 3 {
            for i in 0..4 {
                self.coins.push(Coin { x: wx - 10.0 + i as f32 * 18.0, y: GROUND - 12.0, got: false });
            }
        } else if self.rng.f() < 0.75 {
            for i in 0..5 {
                let u = i as f32 / 4.0;
                self.coins.push(Coin { x: wx + w / 2.0 - 40.0 + u * 80.0, y: GROUND - (h + 22.0) - (1.0 - (2.0 * u - 1.0).powi(2)) * 22.0, got: false });
            }
        }
    }

    fn draw(&self, g: &Gfx, ox: f32, oy: f32, ca: f32) {
        let f = &g.fonts;
        let (sx, sy) = shake_off(self.t, self.shake);
        let (ox, oy) = (ox + sx, oy + sy);
        field_bg(g, ox, oy, ca, Color(0.05, 0.07, 0.18, 1.0), Color(0.22, 0.12, 0.28, 1.0));
        for i in 0..30 {
            g.fill_circle(ox + ((i as f32 * 61.7 - self.dist * 0.02).rem_euclid(W)), oy + 6.0 + (i as f32 * 37.3) % 70.0, 0.9, Color::white(0.45 * ca));
        }
        g.fill_circle(ox + W * 0.78, oy + 34.0, 12.0, Color(0.96, 0.95, 0.85, 0.9 * ca));
        // two mountain layers scrolling at different speeds
        for (par, col, hmax, wd) in [(0.08f32, Color(0.18, 0.15, 0.34, 1.0), 62.0f32, 120.0f32), (0.22, Color(0.13, 0.11, 0.26, 1.0), 44.0, 80.0)] {
            let shift = self.dist * par;
            let first = (shift / wd).floor() as i32 - 1;
            for i in first..first + (W / wd) as i32 + 3 {
                let h = 20.0 + ((i as u32).wrapping_mul(2654435761) % 1000) as f32 / 1000.0 * (hmax - 20.0);
                let bx = ox + i as f32 * wd - shift;
                let cl = |x: f32| x.clamp(ox, ox + W);
                g.fill_poly(&[(cl(bx - 10.0), oy + GROUND), (cl(bx + wd * 0.5), oy + GROUND - h), (cl(bx + wd + 10.0), oy + GROUND)], col.fade(ca));
            }
        }
        // pines
        let shift = self.dist * 0.6;
        let first = (shift / 70.0).floor() as i32 - 1;
        for i in first..first + (W / 70.0) as i32 + 3 {
            let bx = ox + i as f32 * 70.0 - shift + ((i as u32).wrapping_mul(97) % 30) as f32;
            for k in 0..3 {
                g.fill_poly(&[(bx - 8.0 + k as f32 * 1.5, oy + GROUND - 8.0 - k as f32 * 8.0), (bx + 8.0 - k as f32 * 1.5, oy + GROUND - 8.0 - k as f32 * 8.0), (bx, oy + GROUND - 20.0 - k as f32 * 8.0)], Color(0.08, 0.2, 0.2, ca));
            }
        }
        g.fill_rrect(Rect::new(ox, oy + GROUND, W, H - GROUND), 0.0, Color(0.85, 0.9, 1.0, 0.9 * ca));
        g.fill_rrect(Rect::new(ox, oy + GROUND + 5.0, W, H - GROUND - 5.0 - 14.0), 0.0, Color(0.55, 0.62, 0.8, ca));
        g.fill_rrect(Rect::new(ox, oy + H - 28.0, W, 28.0), 14.0, Color(0.55, 0.62, 0.8, ca));
        let stripe = -(self.dist.rem_euclid(40.0));
        let mut x = stripe;
        while x < W {
            if x >= 14.0 && x + 16.0 <= W - 14.0 {
                g.fill_rrect(Rect::new(ox + x, oy + GROUND + 12.0, 16.0, 2.0), 1.0, Color::white(0.25 * ca));
            }
            x += 40.0;
        }
        // obstacles
        for o in &self.obs {
            let x = ox + o.x - self.dist + RUN_X;
            let top = oy + GROUND - o.lift - o.h;
            match o.kind {
                0 => {
                    g.fill_poly(&[(x, oy + GROUND), (x + 3.0, top + 3.0), (x + o.w * 0.5, top), (x + o.w - 2.0, top + 4.0), (x + o.w, oy + GROUND)], Color(0.45, 0.46, 0.55, ca));
                    g.fill_poly(&[(x + 3.0, top + 3.0), (x + o.w * 0.5, top), (x + o.w * 0.5, top + 6.0)], Color::white(0.18 * ca));
                }
                1 => {
                    for k in 0..3 {
                        let bx = x + k as f32 * (o.w / 3.0);
                        g.fill_poly(&[(bx, oy + GROUND), (bx + o.w / 6.0, top), (bx + o.w / 3.0, oy + GROUND)], Color(0.85, 0.35, 0.4, ca));
                    }
                }
                2 => {
                    g.fill_rrect(Rect::new(x, top, o.w, o.h), 2.0, Color(0.5, 0.52, 0.62, ca));
                    g.fill_rrect(Rect::new(x - 2.0, top, o.w + 4.0, 5.0), 2.0, Color(0.62, 0.64, 0.74, ca));
                    g.line((x + 3.0, top + 12.0), (x + o.w - 3.0, top + 16.0), 1.0, Color(0.3, 0.3, 0.4, ca));
                }
                _ => {
                    let flap = (self.t * 16.0).sin() * 5.0;
                    g.fill_circle(x + o.w / 2.0, top + o.h / 2.0, 6.0, Color(0.2, 0.18, 0.28, ca));
                    g.line((x + o.w / 2.0, top + 5.0), (x + 1.0, top - 2.0 + flap), 2.0, Color(0.2, 0.18, 0.28, ca));
                    g.line((x + o.w / 2.0, top + 5.0), (x + o.w - 1.0, top - 2.0 - flap), 2.0, Color(0.2, 0.18, 0.28, ca));
                    g.fill_circle(x + 6.0, top + 5.0, 1.2, Color(1.0, 0.4, 0.3, ca));
                }
            }
        }
        for c in &self.coins {
            let x = ox + c.x - self.dist + RUN_X;
            let sq = ((self.t * 6.0 + c.x * 0.05).sin()).abs().max(0.25);
            g.fill_circle(x, oy + c.y, 6.5, Color(1.0, 0.85, 0.3, 0.2 * ca));
            g.fill_rrect(Rect::new(x - 4.2 * sq, oy + c.y - 4.5, 8.4 * sq, 9.0), 4.0, Color(1.0, 0.82, 0.25, ca));
        }
        // the hero
        let alive = !self.over;
        if alive || (self.t * 12.0) as i32 % 2 == 0 {
            crate::skins::draw_run(g, ox + RUN_X, oy + GROUND - self.lift, self.t, self.lift > 0.5, self.vy);
        }
        draw_parts(g, &self.parts, ox, oy, ca);
        g.text(&format!("{} m", self.score()), Rect::new(ox + 14.0, oy + 4.0, 120.0, 22.0), &f.body, Color::white(0.95 * ca), Align::Left);
        g.text(&format!("\u{25CF} {}", self.coin_n), Rect::new(ox + W - 110.0, oy + 4.0, 98.0, 22.0), &f.body, Color(1.0, 0.85, 0.3, ca), Align::Right);
        if self.toast > 0.0 {
            centered(g, self.toast_s, ox, oy + 40.0, &f.small, Color(1.0, 0.9, 0.5, self.toast.min(0.4) * 2.5 * ca));
        }
        if self.t < 3.5 && !self.over {
            centered(g, "Click to jump \u{00B7} click again in the air to double jump \u{00B7} hold to jump higher", ox, oy + 60.0, &f.small, Color::white((1.0 - self.t / 3.5) * 0.8 * ca));
        }
        if self.over {
            g.fill_rrect(Rect::new(ox, oy, W, H), 14.0, Color(0.0, 0.0, 0.0, 0.5 * ca));
            headline(g, &format!("Game over \u{00B7} {}", self.score()), ox, oy + 62.0, &f.big, Color::white(ca));
            centered(g, &format!("best {} \u{00B7} click to retry", self.best), ox, oy + 106.0, &f.small, Color::white(0.8 * ca));
        }
    }
}

// ------------------------------------------------------------------- hub ---

pub enum Active {
    Flight(Game),
    Brick(Breakout),
    Slice(Slicer),
    Run(Runner),
}

pub struct Hub {
    pub active: Option<(usize, Active)>,
    pub best: [u32; 4],
    saved: [u32; 4],
}

/// What a step produced for the app: a new best to save, and sounds to play.
#[derive(Default)]
pub struct Out {
    pub new_best: bool,
    pub sfx: Vec<(u32, u32)>,
}

impl Hub {
    pub fn new(best: [u32; 4]) -> Hub {
        Hub { active: None, best, saved: best }
    }

    pub fn is_playing(&self) -> bool {
        self.active.is_some()
    }

    pub fn pick(&mut self, k: usize) {
        let b = self.best[k.min(3)];
        self.active = Some((
            k,
            match k {
                0 => Active::Flight(Game::new(b)),
                1 => Active::Brick(Breakout::new(b)),
                2 => Active::Slice(Slicer::new(b)),
                _ => Active::Run(Runner::new(b)),
            },
        ));
    }

    pub fn back(&mut self) {
        self.active = None;
    }

    pub fn click(&mut self) {
        match &mut self.active {
            Some((_, Active::Flight(g))) => g.flap(),
            Some((_, Active::Brick(b))) => b.click(),
            Some((_, Active::Slice(s))) => s.click(),
            Some((_, Active::Run(r))) => r.click(),
            None => {}
        }
    }

    pub fn step(&mut self, dt: f32, inp: &Input) -> Out {
        let mut out = Out::default();
        let Some((k, a)) = &mut self.active else { return out };
        let (best, over) = match a {
            Active::Flight(g) => {
                g.step(dt);
                (g.best, g.state == FlightState::Dead)
            }
            Active::Brick(b) => {
                b.step(dt, inp);
                out.sfx = std::mem::take(&mut b.sfx);
                (b.best, b.over)
            }
            Active::Slice(s) => {
                s.step(dt, inp);
                out.sfx = std::mem::take(&mut s.sfx);
                (s.best, s.over)
            }
            Active::Run(r) => {
                r.step(dt, inp);
                out.sfx = std::mem::take(&mut r.sfx);
                (r.best, r.over)
            }
        };
        self.best[*k] = best.max(self.best[*k]);
        // write the file once a round has ended with a better score than what is saved
        if over && self.best[*k] != self.saved[*k] {
            self.saved[*k] = self.best[*k];
            out.new_best = true;
        }
        out
    }

    pub fn draw(&self, g: &Gfx, ox: f32, oy: f32, ca: f32) {
        match &self.active {
            Some((_, Active::Flight(game))) => game.draw(g, ox, oy, ca),
            Some((_, Active::Brick(b))) => b.draw(g, ox, oy, ca),
            Some((_, Active::Slice(s))) => s.draw(g, ox, oy, ca),
            Some((_, Active::Run(r))) => r.draw(g, ox, oy, ca),
            None => {}
        }
    }

    /// Whether the flying game is in its "ready" state (for the hint), unused otherwise.
    #[allow(dead_code)]
    pub fn flight_ready(&self) -> bool {
        matches!(&self.active, Some((_, Active::Flight(g))) if g.state == FlightState::Ready)
    }

    /// Hub screen: four game tiles with a tiny preview and the best score. Returns the
    /// tile rectangles (screen space) for hit testing.
    pub fn tiles(ox: f32, oy: f32) -> [Rect; 4] {
        let (tw, gap) = (124.0, 14.0);
        let x0 = ox + (W - (4.0 * tw + 3.0 * gap)) / 2.0;
        [0, 1, 2, 3].map(|i| Rect::new(x0 + i as f32 * (tw + gap), oy + 22.0, tw, 134.0))
    }

    pub fn draw_hub(&self, g: &Gfx, ox: f32, oy: f32, ca: f32, hot: Option<usize>) {
        let f = &g.fonts;
        let hub_t = 0.35; // the hub is still, so the heroes are frozen mid-stride
        g.fill_rrect(Rect::new(ox, oy, W, H), 14.0, Color(0.05, 0.06, 0.11, 0.95 * ca));
        for (i, r) in Self::tiles(ox, oy).iter().enumerate() {
            let on = hot == Some(i);
            g.fill_rrect(*r, 18.0, Color(0.12, 0.14, 0.24, ca).fade(if on { 1.0 } else { 0.85 }));
            if on {
                g.stroke_rrect(*r, 18.0, Color(0.55, 0.75, 1.0, 0.8 * ca), 1.5);
            }
            let (cx, cy) = (r.x + r.w / 2.0, r.y + 46.0);
            match i {
                0 => crate::skins::draw_k(g, cx - 4.0, cy + 2.0, hub_t, crate::skins::HUB_K),
                1 => {
                    // two rows of glossy bricks, a glowing ball with a trail, a glossy paddle
                    let cols = [Color(1.0, 0.35, 0.5, 1.0), Color(1.0, 0.8, 0.3, 1.0), Color(0.4, 0.9, 0.5, 1.0), Color(0.4, 0.75, 1.0, 1.0)];
                    for row in 0..2 {
                        for k in 0..4 {
                            let br = Rect::new(cx - 46.0 + k as f32 * 24.0, cy - 26.0 + row as f32 * 11.0, 22.0, 9.0);
                            g.fill_rrect(br, 3.0, cols[(k + row) % 4].fade(ca));
                            g.fill_rrect(Rect::new(br.x + 2.0, br.y + 1.0, br.w - 4.0, 2.5), 1.2, Color::white(0.35 * ca));
                        }
                    }
                    for k in 0..4 {
                        g.fill_circle(cx + 14.0 - k as f32 * 5.0, cy + 6.0 + k as f32 * 4.5, 3.2 - k as f32 * 0.6, Color::white(0.28 * ca / (k as f32 + 1.0)));
                    }
                    g.fill_circle(cx + 14.0, cy + 6.0, 7.0, Color(0.7, 0.85, 1.0, 0.22 * ca));
                    g.fill_circle(cx + 14.0, cy + 6.0, 4.0, Color::white(ca));
                    g.fill_rrect(Rect::new(cx - 24.0, cy + 22.0, 48.0, 7.0), 3.5, Color(0.55, 0.78, 1.0, ca));
                    g.fill_rrect(Rect::new(cx - 20.0, cy + 23.0, 40.0, 2.4), 1.2, Color::white(0.6 * ca));
                }
                2 => {
                    // a fruit cut in two with juice and a bright slash
                    let (lc, _) = FRUIT[0];
                    g.fill_circle(cx - 18.0, cy + 8.0, 13.5, lc.fade(ca));
                    g.fill_circle(cx - 18.0, cy + 8.0, 10.0, Color(1.0, 0.82, 0.45, ca));
                    g.fill_circle(cx - 20.0, cy + 5.0, 3.0, Color::white(0.45 * ca));
                    g.fill_circle(cx + 18.0, cy - 6.0, 12.0, FRUIT[2].0.fade(ca));
                    g.fill_circle(cx + 18.0, cy - 6.0, 8.5, Color(0.85, 1.0, 0.7, ca));
                    for k in 0..6 {
                        let a = k as f32 * 1.05;
                        g.fill_circle(cx + 18.0 + a.cos() * 5.0, cy - 6.0 + a.sin() * 5.0, 1.0, Color(0.15, 0.4, 0.2, 0.6 * ca));
                    }
                    g.line((cx - 40.0, cy + 26.0), (cx + 40.0, cy - 26.0), 5.0, Color(0.6, 0.9, 1.0, 0.25 * ca));
                    g.line((cx - 40.0, cy + 26.0), (cx + 40.0, cy - 26.0), 2.2, Color::white(ca));
                    g.fill_circle(cx + 2.0, cy - 2.0, 1.6, Color(1.0, 0.7, 0.3, 0.9 * ca));
                    g.fill_circle(cx - 3.0, cy + 3.0, 1.2, Color(1.0, 0.55, 0.3, 0.8 * ca));
                }
                _ => crate::skins::draw_run_k(g, cx, cy + 30.0, 0.6, false, crate::skins::HUB_K),
            }
            g.text(NAMES[i], Rect::new(r.x, r.y + 82.0, r.w, 24.0), &f.body, Color::white(0.95 * ca), Align::Center);
            let b = self.best[i];
            g.text(
                &if b > 0 { format!("best {b}") } else { "not played".to_string() },
                Rect::new(r.x, r.y + 106.0, r.w, 20.0),
                &f.tiny,
                Color::white(0.5 * ca),
                Align::Center,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(x: f32, y: f32, down: bool) -> Input {
        Input { x, y, down }
    }

    #[test]
    fn breakout_launch_break_a_brick_and_lose_a_life() {
        let mut b = Breakout::new(0);
        b.banner = 0.0;
        b.click();
        assert!(!b.serve);
        let bricks0 = b.bricks.len();
        for _ in 0..600 {
            let x = b.balls.first().map(|ball| ball.x).unwrap_or(W / 2.0);
            b.step(0.016, &input(x, 100.0, false)); // paddle follows the ball
        }
        assert!(b.bricks.len() < bricks0, "the ball breaks bricks");
        assert!(b.score > 0);
        let lives = b.lives;
        for _ in 0..400 {
            b.step(0.016, &input(0.0, 100.0, false)); // paddle parked at the far left
            if b.lives < lives || b.over {
                break;
            }
        }
        assert!(b.lives < lives || b.over, "missing the ball costs a life");
    }

    #[test]
    fn slicer_swipe_cuts_fruit_and_a_bomb_ends_the_round() {
        let mut s = Slicer::new(0);
        s.objs.push(Obj { x: 200.0, y: 90.0, vx: 0.0, vy: 0.0, r: 13.0, kind: 0, rot: 0.0, spin: 0.0 });
        s.spawn_t = 100.0;
        s.step(0.016, &input(150.0, 90.0, false));
        s.step(0.016, &input(260.0, 90.0, false)); // 110 px in one frame: a fast swipe
        assert!(s.score >= 1 && s.objs.is_empty(), "a fast swipe through fruit scores");
        s.objs.push(Obj { x: 200.0, y: 90.0, vx: 0.0, vy: 0.0, r: 11.0, kind: 4, rot: 0.0, spin: 0.0 });
        s.step(0.016, &input(150.0, 90.0, false));
        s.step(0.016, &input(260.0, 90.0, false));
        assert!(s.over, "cutting a bomb ends the round");
    }

    #[test]
    fn runner_jumps_and_crashes_into_a_rock() {
        let mut r = Runner::new(0);
        r.next_x = 1e9; // no random obstacles
        r.step(0.016, &input(0.0, 0.0, true));
        for _ in 0..5 {
            r.step(0.016, &input(0.0, 0.0, true));
        }
        assert!(r.lift > 5.0, "pressing jumps");
        for _ in 0..200 {
            r.step(0.016, &input(0.0, 0.0, false));
        }
        assert!(r.lift == 0.0 && !r.over);
        r.obs.push(Obs { x: r.dist + RUN_X + 10.0, kind: 0, w: 18.0, h: 15.0, lift: 0.0, scored: false });
        for _ in 0..60 {
            r.step(0.016, &input(0.0, 0.0, false));
        }
        assert!(r.over, "running into a rock ends the round");
    }

    /// Renders the hub and each game to raw files for eyeballing (run with --ignored).
    #[test]
    #[ignore]
    fn preview_frames() {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let dir = std::env::var("PREVIEW_DIR").expect("PREVIEW_DIR");
        let scale = 2.0f32;
        let (lw, lh) = (W + 20.0, H + 20.0);
        let g = Gfx::new((lw * scale) as i32, (lh * scale) as i32, scale).unwrap();
        let save = |name: &str| {
            let (bw, _bh, buf) = g.snapshot();
            let (pw, ph) = ((lw * scale) as i32, (lh * scale) as i32);
            let mut out = Vec::new();
            out.extend_from_slice(&(pw as u32).to_le_bytes());
            out.extend_from_slice(&(ph as u32).to_le_bytes());
            for y in 0..ph {
                let row = (y * bw * 4) as usize;
                out.extend_from_slice(&buf[row..row + (pw * 4) as usize]);
            }
            std::fs::write(format!("{dir}/{name}.raw"), out).unwrap();
        };
        let frame = |hub: &Hub, name: &str, hubscreen: bool| {
            g.begin((lw * scale) as i32, (lh * scale) as i32).unwrap();
            if hubscreen {
                hub.draw_hub(&g, 10.0, 10.0, 1.0, Some(1));
            } else {
                hub.draw(&g, 10.0, 10.0, 1.0);
            }
            let _ = g.end();
            save(name);
        };
        let mut hub = Hub::new([12, 340, 27, 180]);
        frame(&hub, "hub", true);
        // brick breaker, mid-round
        hub.pick(1);
        hub.click();
        for i in 0..260 {
            let x = 272.0 + (i as f32 * 0.04).sin() * 200.0;
            hub.step(0.016, &Input { x, y: 100.0, down: false });
        }
        frame(&hub, "brick", false);
        // slicer, mid-round with a swipe
        hub.pick(2);
        for i in 0..260 {
            let a = i as f32 * 0.09;
            hub.step(0.016, &Input { x: 272.0 + a.cos() * 200.0, y: 90.0 + a.sin() * 60.0, down: false });
        }
        frame(&hub, "slice", false);
        // runner, mid-run, in the air
        hub.pick(3);
        for i in 0..150 {
            hub.step(0.016, &Input { x: 0.0, y: 0.0, down: i % 60 < 20 });
        }
        frame(&hub, "run", false);
    }
}
