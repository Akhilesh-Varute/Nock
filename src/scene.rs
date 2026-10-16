//! Fight scenes: now and then (rarely, in short bursts) the four original
//! characters from cast.rs act out a tiny story on the free stretch of the top
//! bar, then walk off and the buddy comes back. Everything is scripted with a
//! few keyframes per actor, so it costs nothing while idle and about 30 frames
//! per second for ~10 seconds while it plays.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::*;
use crate::bar::theme;
use crate::cast::{self, Act, Draw, Who};
use crate::gfx::*;

pub const T_SCENE: usize = 16;
pub const T_SCENEANIM: usize = 17;

/// Stage window height (logical px): the bar plus a little room underneath.
const SH: f32 = 32.0;
/// Ground line inside the stage.
const GROUND: f32 = 28.0;
const MAX_W: f32 = 380.0;
const MIN_W: f32 = 230.0;


// ------------------------------------------------------------------ scripts --

#[derive(Clone, Copy)]
struct Key {
    t: f32,
    x: f32,
    act: Act,
    dir: f32,
}

struct Track {
    who: Who,
    keys: Vec<Key>,
}

#[derive(Clone, Copy, PartialEq)]
enum Fx {
    Bolt,
    Fire,
    Gem,
    Soul,
    Dagger,
    Ring,
    Burst(u8),
    Arc(u8),
}

struct Ev {
    t: f32,
    dur: f32,
    fx: Fx,
    a: usize,
    b: usize,
}

struct Say {
    t: f32,
    dur: f32,
    who: usize,
    text: &'static str,
}

struct Script {
    tracks: Vec<Track>,
    evs: Vec<Ev>,
    says: Vec<Say>,
    len: f32,
}

fn tr(who: Who, k: &[(f32, f32, Act, f32)]) -> Track {
    Track {
        who,
        keys: k.iter().map(|&(t, x, act, dir)| Key { t, x, act, dir }).collect(),
    }
}

fn ev(t: f32, dur: f32, fx: Fx, a: usize, b: usize) -> Ev {
    Ev { t, dur, fx, a, b }
}

fn say(t: f32, dur: f32, who: usize, text: &'static str) -> Say {
    Say { t, dur, who, text }
}

use Act::*;

fn script(id: usize) -> Script {
    match id {
        // Cinder (0) vs Krag (1)
        0 => Script {
            tracks: vec![
                tr(Who::Cinder, &[
                    (0.0, -0.08, Run, 1.0), (1.5, 0.27, Stand, 1.0), (2.6, 0.27, Jump, 1.0),
                    (3.3, 0.30, Run, 1.0), (3.9, 0.52, Slash, 1.0), (4.8, 0.52, Stand, 1.0),
                    (5.0, 0.52, Block, 1.0), (5.8, 0.46, Throw, 1.0), (6.5, 0.46, Stand, 1.0),
                    (7.9, 0.46, Cheer, 1.0), (9.3, 0.46, Walk, -1.0), (10.8, -0.1, Stand, -1.0),
                ]),
                tr(Who::Krag, &[
                    (0.0, 1.08, Walk, -1.0), (1.5, 0.72, Stand, -1.0), (2.2, 0.72, Slam, -1.0),
                    (3.0, 0.72, Stand, -1.0), (3.95, 0.72, Hurt, -1.0), (4.8, 0.74, Stand, -1.0),
                    (4.85, 0.74, Cast, -1.0), (5.5, 0.74, Stand, -1.0), (6.3, 0.74, Hurt, -1.0),
                    (6.9, 0.74, Kneel, -1.0), (9.0, 0.74, Stand, -1.0), (9.3, 0.74, Walk, 1.0),
                    (10.8, 1.1, Stand, 1.0),
                ]),
            ],
            evs: vec![
                ev(2.55, 0.9, Fx::Ring, 1, 0),
                ev(4.1, 0.3, Fx::Arc(0), 0, 1),
                ev(4.1, 0.4, Fx::Burst(1), 0, 1),
                ev(5.1, 0.45, Fx::Gem, 1, 0),
                ev(5.55, 0.4, Fx::Burst(2), 1, 0),
                ev(6.15, 0.4, Fx::Fire, 0, 1),
                ev(6.55, 0.5, Fx::Burst(0), 0, 1),
            ],
            says: vec![
                say(1.6, 1.4, 1, "Kneel, knight."),
                say(3.0, 1.3, 0, "Not today."),
                say(7.9, 1.6, 0, "Ha! Embers."),
                say(5.0, 0.0, 0, ""),
            ],
            len: 10.8,
        },
        // Veil (0) steals from Spark (1)
        1 => Script {
            tracks: vec![
                tr(Who::Veil, &[
                    (0.0, 0.1, Run, 1.0), (1.8, 0.7, Stand, 1.0), (2.0, 0.7, Throw, -1.0),
                    (2.7, 0.7, Stand, -1.0), (3.5, 0.7, Hurt, -1.0), (4.2, 0.66, Stand, -1.0),
                    (4.3, 0.62, Slash, -1.0), (5.1, 0.62, Stand, -1.0), (5.5, 0.62, Hurt, -1.0),
                    (6.3, 0.64, Kneel, -1.0), (7.3, 0.64, Walk, 1.0), (9.0, 1.1, Stand, 1.0),
                ]),
                tr(Who::Spark, &[
                    (0.4, -0.1, Run, 1.0), (2.0, 0.3, Run, 1.0), (2.3, 0.36, Jump, 1.0),
                    (3.0, 0.43, Stand, 1.0), (3.1, 0.43, Cast, 1.0), (3.9, 0.43, Stand, 1.0),
                    (4.3, 0.43, Block, 1.0), (5.0, 0.4, Stand, 1.0), (5.1, 0.4, Cast, 1.0),
                    (5.9, 0.4, Stand, 1.0), (6.4, 0.4, Cheer, 1.0), (8.0, 0.4, Walk, -1.0),
                    (9.0, -0.1, Stand, -1.0),
                ]),
            ],
            evs: vec![
                ev(2.25, 0.4, Fx::Dagger, 0, 1),
                ev(3.4, 0.35, Fx::Bolt, 1, 0),
                ev(3.4, 0.5, Fx::Burst(1), 1, 0),
                ev(4.4, 0.3, Fx::Arc(1), 0, 1),
                ev(5.3, 0.3, Fx::Bolt, 1, 0),
                ev(5.3, 0.5, Fx::Burst(1), 1, 0),
            ],
            says: vec![
                say(0.9, 1.3, 0, "Mine now!"),
                say(1.8, 1.3, 1, "My gadget!"),
                say(6.4, 1.6, 1, "Gotcha!"),
            ],
            len: 9.0,
        },
        // Ronin (0) vs Courier (1)
        3 => Script {
            tracks: vec![
                tr(Who::Ronin, &[
                    (0.0, -0.1, Walk, 1.0), (1.8, 0.3, Stand, 1.0), (3.6, 0.3, Block, 1.0),
                    (4.6, 0.3, Run, 1.0), (5.3, 0.58, Slash, 1.0), (6.1, 0.58, Stand, 1.0),
                    (6.9, 0.58, Block, 1.0), (7.8, 0.58, Run, 1.0), (8.1, 0.76, Slash, 1.0),
                    (8.9, 0.76, Stand, 1.0), (10.3, 0.76, Walk, -1.0), (12.0, -0.1, Stand, -1.0),
                ]),
                tr(Who::Courier, &[
                    (0.0, 1.1, Run, -1.0), (1.5, 0.72, Stand, -1.0), (3.5, 0.72, Cast, -1.0),
                    (4.2, 0.72, Stand, -1.0), (5.6, 0.72, Hurt, -1.0), (6.2, 0.74, Stand, -1.0),
                    (6.3, 0.74, Jump, -1.0), (6.9, 0.86, Cast, -1.0), (7.5, 0.86, Stand, -1.0),
                    (8.4, 0.86, Hurt, -1.0), (9.0, 0.86, Kneel, -1.0), (10.6, 0.86, Stand, -1.0),
                    (11.0, 0.86, Run, 1.0), (12.0, 1.15, Stand, 1.0),
                ]),
            ],
            evs: vec![
                ev(3.75, 0.3, Fx::Bolt, 1, 0),
                ev(3.9, 0.45, Fx::Burst(1), 1, 0),
                ev(5.55, 0.3, Fx::Arc(1), 0, 1),
                ev(5.65, 0.45, Fx::Burst(0), 0, 1),
                ev(7.05, 0.3, Fx::Bolt, 1, 0),
                ev(7.2, 0.45, Fx::Burst(1), 1, 0),
                ev(8.35, 0.3, Fx::Arc(1), 0, 1),
                ev(8.45, 0.5, Fx::Burst(0), 0, 1),
            ],
            says: vec![
                say(1.6, 1.7, 1, "Nice hat, old man."),
                say(3.2, 1.0, 0, "Hm."),
                say(6.2, 1.5, 1, "Okay, ouch."),
                say(9.0, 1.4, 0, "Patience."),
                say(10.7, 1.3, 1, "Worth it..."),
            ],
            len: 12.0,
        },
        // Necromancer (0) vs Ronin (1)
        4 => Script {
            tracks: vec![
                tr(Who::Necro, &[
                    (0.0, 1.1, Walk, -1.0), (1.5, 0.74, Stand, -1.0), (3.9, 0.74, Cast, -1.0),
                    (4.6, 0.74, Stand, -1.0), (6.0, 0.74, Hurt, -1.0), (6.6, 0.76, Stand, -1.0),
                    (6.7, 0.76, Cast, -1.0), (7.4, 0.76, Stand, -1.0), (9.5, 0.76, Hurt, -1.0),
                    (10.1, 0.78, Kneel, -1.0), (11.6, 0.78, Stand, -1.0), (12.0, 0.78, Walk, 1.0),
                    (13.0, 1.1, Stand, 1.0),
                ]),
                tr(Who::Ronin, &[
                    (0.2, -0.1, Walk, 1.0), (1.8, 0.3, Stand, 1.0), (4.1, 0.3, Block, 1.0),
                    (5.0, 0.3, Run, 1.0), (5.5, 0.56, Slash, 1.0), (6.3, 0.56, Stand, 1.0),
                    (7.3, 0.56, Hurt, 1.0), (7.9, 0.54, Kneel, 1.0), (8.8, 0.54, Stand, 1.0),
                    (8.9, 0.54, Run, 1.0), (9.1, 0.64, Slash, 1.0), (9.9, 0.64, Stand, 1.0),
                    (11.6, 0.64, Walk, -1.0), (13.0, -0.1, Stand, -1.0),
                ]),
            ],
            evs: vec![
                ev(4.05, 0.5, Fx::Soul, 0, 1),
                ev(4.5, 0.4, Fx::Burst(3), 0, 1),
                ev(5.95, 0.3, Fx::Arc(1), 1, 0),
                ev(6.05, 0.45, Fx::Burst(0), 1, 0),
                ev(6.85, 0.5, Fx::Soul, 0, 1),
                ev(7.3, 0.5, Fx::Burst(3), 0, 1),
                ev(9.45, 0.3, Fx::Arc(1), 1, 0),
                ev(9.55, 0.5, Fx::Burst(0), 1, 0),
            ],
            says: vec![
                say(1.7, 1.9, 0, "Death is a pause."),
                say(3.6, 1.2, 1, "Then pause."),
                say(7.9, 1.2, 0, "Rise... rise."),
                say(8.8, 1.2, 1, "Not yet."),
                say(10.3, 1.5, 0, "Until next time."),
            ],
            len: 13.0,
        },
        // Ronin (0) + Courier (1) vs Necromancer (2)
        // Ronin (0) + Courier (1) vs Necromancer (2)
        5 => Script {
            tracks: vec![
                tr(Who::Ronin, &[
                    (0.0, -0.1, Walk, 1.0), (1.7, 0.3, Stand, 1.0), (3.4, 0.3, Block, 1.0),
                    (4.4, 0.3, Run, 1.0), (4.9, 0.52, Slash, 1.0), (5.7, 0.52, Stand, 1.0),
                    (7.8, 0.52, Cheer, 1.0), (9.2, 0.52, Walk, -1.0), (10.6, -0.1, Stand, -1.0),
                ]),
                tr(Who::Courier, &[
                    (0.2, -0.1, Run, 1.0), (1.5, 0.2, Stand, 1.0), (3.9, 0.2, Cast, 1.0),
                    (4.6, 0.2, Stand, 1.0), (5.7, 0.2, Cast, 1.0), (6.4, 0.2, Stand, 1.0),
                    (7.8, 0.2, Cheer, 1.0), (9.2, 0.2, Run, -1.0), (10.6, -0.1, Stand, -1.0),
                ]),
                tr(Who::Necro, &[
                    (0.0, 1.1, Walk, -1.0), (1.6, 0.74, Stand, -1.0), (3.0, 0.74, Cast, -1.0),
                    (3.7, 0.74, Stand, -1.0), (4.15, 0.75, Hurt, -1.0), (4.8, 0.74, Stand, -1.0),
                    (5.0, 0.76, Hurt, -1.0), (5.6, 0.76, Stand, -1.0), (5.85, 0.78, Hurt, -1.0),
                    (6.5, 0.78, Kneel, -1.0), (8.6, 0.78, Stand, -1.0), (8.9, 0.78, Walk, 1.0),
                    (10.2, 1.1, Stand, 1.0),
                ]),
            ],
            evs: vec![
                ev(3.15, 0.5, Fx::Soul, 2, 0),
                ev(3.6, 0.4, Fx::Burst(3), 2, 0),
                ev(4.05, 0.3, Fx::Bolt, 1, 2),
                ev(4.1, 0.45, Fx::Burst(1), 1, 2),
                ev(4.95, 0.3, Fx::Arc(1), 0, 2),
                ev(5.0, 0.45, Fx::Burst(0), 0, 2),
                ev(5.85, 0.3, Fx::Bolt, 1, 2),
                ev(5.9, 0.6, Fx::Burst(1), 1, 2),
            ],
            says: vec![
                say(1.7, 1.2, 1, "Cool skull."),
                say(2.0, 0.0, 2, ""),
                say(2.9, 1.0, 2, "Join me."),
                say(3.6, 0.8, 0, "No."),
                say(6.9, 1.5, 2, "I'll be back."),
                say(7.9, 1.4, 1, "Nailed it!"),
            ],
            len: 10.6,
        },
        // Spark (0) + Cinder (1) vs Krag (2)
        _ => Script {
            tracks: vec![
                tr(Who::Spark, &[
                    (0.0, -0.1, Run, 1.0), (1.5, 0.2, Stand, 1.0), (2.4, 0.2, Jump, 1.0),
                    (3.1, 0.22, Stand, 1.0), (3.4, 0.22, Cast, 1.0), (4.2, 0.22, Stand, 1.0),
                    (5.5, 0.22, Cast, 1.0), (6.3, 0.22, Stand, 1.0), (7.9, 0.22, Cheer, 1.0),
                    (9.3, 0.22, Walk, -1.0), (10.5, -0.1, Stand, -1.0),
                ]),
                tr(Who::Cinder, &[
                    (0.1, -0.1, Run, 1.0), (1.6, 0.32, Stand, 1.0), (2.4, 0.32, Jump, 1.0),
                    (3.0, 0.34, Run, 1.0), (3.4, 0.5, Slash, 1.0), (4.2, 0.5, Stand, 1.0),
                    (4.5, 0.4, Run, 1.0), (4.7, 0.4, Block, 1.0), (5.3, 0.4, Stand, 1.0),
                    (5.4, 0.4, Throw, 1.0), (6.2, 0.4, Stand, 1.0), (7.9, 0.4, Cheer, 1.0),
                    (9.3, 0.4, Walk, -1.0), (10.5, -0.1, Stand, -1.0),
                ]),
                tr(Who::Krag, &[
                    (0.0, 1.1, Walk, -1.0), (1.7, 0.75, Stand, -1.0), (2.0, 0.75, Slam, -1.0),
                    (2.8, 0.75, Stand, -1.0), (3.65, 0.76, Hurt, -1.0), (4.3, 0.76, Stand, -1.0),
                    (4.4, 0.76, Cast, -1.0), (5.0, 0.76, Stand, -1.0), (5.9, 0.78, Hurt, -1.0),
                    (6.5, 0.78, Kneel, -1.0), (9.0, 0.78, Stand, -1.0), (9.3, 0.78, Walk, 1.0),
                    (10.5, 1.1, Stand, 1.0),
                ]),
            ],
            evs: vec![
                ev(2.35, 0.9, Fx::Ring, 2, 0),
                ev(3.7, 0.3, Fx::Arc(0), 1, 2),
                ev(3.75, 0.35, Fx::Bolt, 0, 2),
                ev(3.75, 0.5, Fx::Burst(1), 0, 2),
                ev(4.6, 0.4, Fx::Gem, 2, 1),
                ev(5.0, 0.4, Fx::Burst(2), 2, 1),
                ev(5.6, 0.35, Fx::Bolt, 0, 2),
                ev(5.6, 0.45, Fx::Fire, 1, 2),
                ev(6.0, 0.7, Fx::Burst(0), 1, 2),
            ],
            says: vec![
                say(1.7, 1.4, 2, "Puny heroes."),
                say(2.9, 1.2, 0, "Together!"),
                say(7.9, 1.6, 1, "Team!"),
            ],
            len: 10.5,
        },
    }
}

fn ease(u: f32) -> f32 {
    u * u * (3.0 - 2.0 * u)
}

/// State of one track at time `t`: x as a fraction of the stage, action, etc.
fn sample(tk: &Track, t: f32) -> Option<(f32, Act, f32, f32, f32)> {
    let first = tk.keys.first()?;
    let last = tk.keys.last()?;
    if t < first.t || t > last.t {
        return None;
    }
    let mut i = 0;
    while i + 1 < tk.keys.len() && tk.keys[i + 1].t <= t {
        i += 1;
    }
    let k = tk.keys[i];
    let n = tk.keys.get(i + 1).copied().unwrap_or(k);
    let u = if n.t > k.t { ((t - k.t) / (n.t - k.t)).clamp(0.0, 1.0) } else { 0.0 };
    let x = k.x + (n.x - k.x) * if matches!(k.act, Run | Walk) { u } else { ease(u) };
    Some((x, k.act, k.dir, u, (t - k.t).max(0.0)))
}

// -------------------------------------------------------------------- stage --

pub struct Stage {
    pub hwnd: HWND,
    gfx: Gfx,
    pub active: bool,
    id: usize,
    start: Instant,
    /// stage width in logical px
    w: f32,
    left: f32,
    rng: u32,
}

impl Stage {
    pub fn new(hwnd: HWND, scale: f32) -> Option<Stage> {
        let gfx = Gfx::new(
            ((MAX_W + 2.0) * scale).ceil() as i32,
            ((SH + 2.0) * scale).ceil() as i32,
            scale,
        )
        .ok()?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(7);
        Some(Stage {
            hwnd,
            gfx,
            active: false,
            id: 0,
            start: Instant::now(),
            w: MAX_W,
            left: 0.0,
            rng: nanos | 1,
        })
    }

    fn rnd(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }
}

fn polyline(g: &Gfx, pts: &[(f32, f32)], w: f32, c: Color) {
    for i in 0..pts.len().saturating_sub(1) {
        g.line(pts[i], pts[i + 1], w, c);
    }
}

fn fx_color(kind: u8) -> Color {
    match kind {
        0 => Color(1.0, 0.65, 0.2, 1.0),
        1 => Color(0.7, 0.95, 1.0, 1.0),
        3 => Color(0.4, 1.0, 0.55, 1.0),
        _ => Color(1.0, 0.4, 0.9, 1.0),
    }
}

fn lerp2(a: (f32, f32), b: (f32, f32), u: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u)
}

fn draw_fx(g: &Gfx, e: &Ev, u: f32, draws: &[Option<Draw>], tick: u32) {
    let (Some(da), Some(db)) = (draws.get(e.a).copied().flatten(), draws.get(e.b).copied().flatten()) else {
        // target already left the stage: still show impact effects on the ground
        return;
    };
    let (hand, _, _) = cast::anchors(&da);
    let (_, chest_b, _) = cast::anchors(&db);
    let fade = 1.0 - u;
    match e.fx {
        Fx::Bolt => {
            let n = 7;
            let mut pts = vec![hand];
            for i in 1..n {
                let p = lerp2(hand, chest_b, i as f32 / n as f32);
                let j = (((tick * 7 + i as u32 * 13) % 9) as f32 - 4.0) * 0.9;
                pts.push((p.0, p.1 + j));
            }
            pts.push(chest_b);
            polyline(g, &pts, 4.5, Color(0.4, 0.85, 1.0, 0.35 * fade.max(0.3)));
            polyline(g, &pts, 1.6, Color(1.0, 1.0, 1.0, 0.95));
            g.fill_circle(chest_b.0, chest_b.1, 4.0 * (1.0 - u * 0.5), Color(0.8, 1.0, 1.0, 0.6 * fade));
        }
        Fx::Fire | Fx::Gem | Fx::Soul | Fx::Dagger => {
            let p = lerp2(hand, chest_b, u);
            for k in 1..4 {
                let q = lerp2(hand, chest_b, (u - k as f32 * 0.06).max(0.0));
                let col = match e.fx {
                    Fx::Fire => Color(1.0, 0.5, 0.1, 0.5 / k as f32),
                    Fx::Gem => Color(1.0, 0.3, 0.85, 0.45 / k as f32),
                    Fx::Soul => Color(0.3, 1.0, 0.5, 0.5 / k as f32),
                    _ => Color(0.9, 0.97, 1.0, 0.4 / k as f32),
                };
                g.fill_circle(q.0, q.1, 3.2 - k as f32 * 0.6, col);
            }
            match e.fx {
                Fx::Fire => {
                    g.fill_circle(p.0, p.1, 4.6, Color(1.0, 0.45, 0.1, 0.5));
                    g.fill_circle(p.0, p.1, 3.2, Color(1.0, 0.7, 0.2, 1.0));
                    g.fill_circle(p.0, p.1, 1.6, Color(1.0, 1.0, 0.8, 1.0));
                }
                Fx::Soul => {
                    let w = (tick as f32 * 0.6).sin() * 1.2;
                    g.fill_circle(p.0, p.1 + w, 4.6, Color(0.3, 1.0, 0.5, 0.35));
                    g.fill_circle(p.0, p.1 + w, 2.6, Color(0.5, 1.0, 0.65, 1.0));
                    g.fill_circle(p.0, p.1 + w, 1.2, Color(0.9, 1.0, 0.92, 1.0));
                }
                Fx::Gem => {
                    g.fill_circle(p.0, p.1, 5.0, Color(1.0, 0.3, 0.85, 0.3));
                    g.fill_poly(
                        &[(p.0, p.1 - 4.0), (p.0 + 2.6, p.1), (p.0, p.1 + 4.0), (p.0 - 2.6, p.1)],
                        Color(1.0, 0.55, 0.92, 1.0),
                    );
                }
                _ => {
                    let d = (chest_b.0 - hand.0).signum();
                    g.line((p.0 - d * 4.0, p.1), (p.0 + d * 4.0, p.1), 1.6, Color(0.92, 0.97, 1.0, 1.0));
                }
            }
        }
        Fx::Ring => {
            let rx = 6.0 + u * 90.0;
            let ry = rx * 0.1;
            let cx = da.x;
            let cy = da.y - 1.0;
            let mut pts = Vec::new();
            for i in 0..=20 {
                let a = i as f32 / 20.0 * std::f32::consts::TAU;
                pts.push((cx + a.cos() * rx, cy + a.sin() * ry));
            }
            polyline(g, &pts, 2.4, Color(0.95, 0.88, 0.75, 0.8 * fade));
            for k in 0..5 {
                let dx = ((k as f32) - 2.0) * rx * 0.45;
                g.fill_circle(cx + dx, cy - 1.0 - u * 9.0 * (1.0 - (k as f32 - 2.0).abs() * 0.25), 1.3, Color(0.9, 0.8, 0.65, 0.6 * fade));
            }
        }
        Fx::Burst(kind) => {
            let col = fx_color(kind);
            let r = 3.0 + u * 12.0;
            g.fill_circle(chest_b.0, chest_b.1, r * 0.8, Color(col.0, col.1, col.2, 0.35 * fade));
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0 + 0.3;
                let (s, c) = a.sin_cos();
                g.line(
                    (chest_b.0 + c * r * 0.5, chest_b.1 + s * r * 0.5),
                    (chest_b.0 + c * r, chest_b.1 + s * r),
                    1.6 * fade.max(0.3),
                    Color(col.0, col.1, col.2, fade),
                );
            }
            g.fill_circle(chest_b.0, chest_b.1, (6.0 * (1.0 - u)).max(0.0), Color(1.0, 1.0, 1.0, 0.8 * fade));
        }
        Fx::Arc(kind) => {
            let col = if kind == 0 { Color(1.0, 0.7, 0.25, 1.0) } else { Color(0.9, 0.97, 1.0, 1.0) };
            let r = 17.0;
            let c0 = (da.x + da.dir * 4.0, da.y - 17.0);
            let mut pts = Vec::new();
            let span = 0.4 + u * 1.6;
            for i in 0..=10 {
                let a = -1.3 + span * (i as f32 / 10.0);
                pts.push((c0.0 + a.cos() * r * da.dir, c0.1 + a.sin() * r));
            }
            polyline(g, &pts, 3.4, Color(col.0, col.1, col.2, 0.35 * fade));
            polyline(g, &pts, 1.4, Color(col.0, col.1, col.2, fade));
        }
    }
}

/// Speech bubble beside the speaker (toward the open side of the stage), since
/// the bar leaves no room above them.
fn draw_bubble(g: &Gfx, th: &crate::bar::Theme, light: bool, text: &str, sx: f32, sw: f32) {
    let n = text.chars().count() as f32;
    let w = (n * 5.6 + 14.0).clamp(26.0, 130.0);
    let right = sx < sw * 0.5;
    let x = if right { sx + 14.0 } else { sx - 14.0 - w };
    let x = x.clamp(1.0, (sw - w - 1.0).max(1.0));
    let r = Rect::new(x, 4.0, w, 16.0);
    let bg = if light { Color(1.0, 1.0, 1.0, 0.97) } else { Color(0.17, 0.17, 0.2, 0.97) };
    if right {
        g.fill_poly(&[(r.x + 1.0, r.y + 5.0), (r.x - 5.0, r.y + 10.0), (r.x + 1.0, r.y + 12.0)], bg);
    } else {
        g.fill_poly(&[(r.x + r.w - 1.0, r.y + 5.0), (r.x + r.w + 5.0, r.y + 10.0), (r.x + r.w - 1.0, r.y + 12.0)], bg);
    }
    g.fill_rrect(r, 8.0, bg);
    g.stroke_rrect(r, 8.0, th.line, 1.0);
    g.text(text, r, &g.fonts.tiny, th.t1, Align::Center);
}

// ------------------------------------------------------------------ App side --

impl App {
    pub fn is_stage(&self, h: HWND) -> bool {
        self.stage.as_ref().map(|s| s.hwnd == h).unwrap_or(false)
    }

    pub fn scene_active(&self) -> bool {
        self.stage.as_ref().map(|s| s.active).unwrap_or(false)
    }

    pub fn rebuild_stage(&mut self) {
        let Some(old) = self.stage.take() else { return };
        self.stage = Stage::new(old.hwnd, self.scale);
        unsafe {
            let _ = ShowWindow(old.hwnd, SW_HIDE);
        }
        self.kill_timer(T_SCENEANIM);
        self.kill_timer(T_SCENE);
        self.plan_scene();
    }

    /// Stage area on the bar: (left, width) in logical px, if there is room.
    fn stage_area(&self) -> Option<(f32, f32)> {
        let b = self.bar.as_ref()?;
        let w = (self.mon.right - self.mon.left) as f32 / self.scale;
        let right = b.hits.iter().map(|(r, _)| r.x).fold(w, f32::min) - 10.0;
        let left = w / 2.0 + 200.0;
        let width = (right - left).min(MAX_W);
        if width < MIN_W {
            None
        } else {
            Some((right - width, width))
        }
    }

    /// Arm the next automatic scene (rarely: every 6 to 14 minutes).
    pub fn plan_scene(&mut self) {
        self.kill_timer(T_SCENE);
        if self.cfg.scenes && self.cfg.buddy != 0 && self.stage.is_some() {
            let d = match self.stage.as_mut() {
                Some(s) => 360_000 + s.rnd() % 480_000,
                None => return,
            };
            self.set_timer(T_SCENE, d);
        }
    }

    pub fn on_scene_timer(&mut self) {
        self.kill_timer(T_SCENE);
        let busy = self.pop.as_ref().map(|p| p.kind.is_some()).unwrap_or(false);
        if self.scene_active() {
            return;
        }
        if busy || !self.bar_visible() {
            // try again a bit later
            self.set_timer(T_SCENE, 60_000);
            return;
        }
        let id = match self.stage.as_mut() {
            Some(s) => (s.rnd() % 6) as usize,
            None => return,
        };
        self.play_scene(id);
        if !self.scene_active() {
            self.plan_scene();
        }
    }

    pub fn play_scene(&mut self, id: usize) {
        if self.scene_active() || !self.bar_visible() || self.cfg.buddy == 0 {
            return;
        }
        let Some((left, w)) = self.stage_area() else { return };
        let mon = self.mon;
        let scale = self.scale;
        let Some(s) = self.stage.as_mut() else { return };
        s.id = id.min(5);
        s.start = Instant::now();
        s.w = w;
        s.left = left;
        s.active = true;
        unsafe {
            let _ = ShowWindow(s.hwnd, SW_SHOWNOACTIVATE);
        }
        // the buddy steps out of the way
        if let Some(b) = self.buddy.as_ref() {
            unsafe {
                let _ = ShowWindow(b.hwnd, SW_HIDE);
            }
        }
        self.kill_timer(crate::buddy::T_BUDDY);
        self.kill_timer(crate::buddy::T_BWAKE);
        self.raise_windows();
        let _ = (mon, scale);
        self.set_timer(T_SCENEANIM, 33);
        self.scene_render();
    }

    pub fn stop_scene(&mut self) {
        let was = self.scene_active();
        if let Some(s) = self.stage.as_mut() {
            s.active = false;
            unsafe {
                let _ = ShowWindow(s.hwnd, SW_HIDE);
            }
        }
        self.kill_timer(T_SCENEANIM);
        if was {
            self.buddy_return();
            self.plan_scene();
        }
    }

    pub fn on_scene_anim(&mut self) {
        let done = match self.stage.as_ref() {
            Some(s) if s.active => s.start.elapsed() >= Duration::from_secs_f32(script(s.id).len + 0.3),
            _ => true,
        };
        if done || !self.bar_visible() {
            self.stop_scene();
            return;
        }
        self.scene_render();
    }

    pub fn scene_render(&mut self) {
        let light = self.bar.as_ref().map(|b| b.light).unwrap_or(false);
        let th = theme(light);
        let scale = self.scale;
        let mon = self.mon;
        let Some(s) = self.stage.as_mut() else { return };
        if !s.active {
            return;
        }
        let sc = script(s.id);
        let t = s.start.elapsed().as_secs_f32();
        let tick = (t * 30.0) as u32;
        let (pw, ph) = ((s.w * scale).round() as i32, (SH * scale).round() as i32);
        let g = &s.gfx;
        if g.begin(pw, ph).is_err() {
            return;
        }
        let mut draws: Vec<Option<Draw>> = Vec::new();
        for tk in &sc.tracks {
            draws.push(sample(tk, t).map(|(fx, act, dir, u, _since)| {
                let lift = if act == Act::Jump { (u * std::f32::consts::PI).sin() * 7.0 } else { 0.0 };
                let charge = if matches!(act, Act::Cast | Act::Throw | Act::Slash) {
                    (u * std::f32::consts::PI).sin()
                } else {
                    0.0
                };
                Draw {
                    who: tk.who,
                    x: fx * s.w,
                    y: GROUND - lift,
                    dir,
                    act,
                    p: if matches!(act, Act::Walk | Act::Run | Act::Stand | Act::Cheer | Act::Block | Act::Kneel) {
                        0.0
                    } else {
                        u
                    },
                    t,
                    charge,
                }
            }));
        }
        // far actors first so the nearer one overlaps
        let mut order: Vec<usize> = (0..draws.len()).collect();
        order.sort_by(|a, b| {
            let ya = draws[*a].map(|d| d.x).unwrap_or(0.0);
            let yb = draws[*b].map(|d| d.x).unwrap_or(0.0);
            ya.partial_cmp(&yb).unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in order {
            if let Some(d) = &draws[i] {
                cast::draw_actor(g, d);
            }
        }
        for e in &sc.evs {
            if t >= e.t && t <= e.t + e.dur {
                draw_fx(g, e, ((t - e.t) / e.dur).clamp(0.0, 1.0), &draws, tick);
            }
        }
        for sy in &sc.says {
            if sy.dur > 0.0 && t >= sy.t && t <= sy.t + sy.dur {
                if let Some(Some(d)) = draws.get(sy.who) {
                    draw_bubble(g, &th, light, sy.text, d.x, s.w);
                }
            }
        }
        let _ = g.end();
        let _ = g.present(s.hwnd, (mon.left as f32 + s.left * scale).round() as i32, mon.top, pw, ph);
    }

    /// The buddy comes back after a scene.
    fn buddy_return(&mut self) {
        let Some(b) = self.buddy.as_mut() else { return };
        if b.shown {
            b.sleep_now();
            unsafe {
                let _ = ShowWindow(b.hwnd, SW_SHOWNOACTIVATE);
            }
            self.raise_windows();
            self.buddy_render();
            self.set_timer(crate::buddy::T_BWAKE, 8000);
        }
    }

    pub fn toggle_scenes(&mut self) {
        self.cfg.scenes = !self.cfg.scenes;
        crate::store::save_config(&self.cfg);
        if self.cfg.scenes {
            self.plan_scene();
        } else {
            self.kill_timer(T_SCENE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_well_formed() {
        for id in 0..6 {
            let sc = script(id);
            assert!(!sc.tracks.is_empty());
            for tk in &sc.tracks {
                assert!(tk.keys.len() >= 2, "scene {id}: track too short");
                assert!(tk.keys.windows(2).all(|w| w[0].t <= w[1].t), "scene {id}: keys out of order");
                assert!(tk.keys.last().unwrap().t <= sc.len + 0.01, "scene {id}: key after end");
            }
            for e in &sc.evs {
                assert!(e.a < sc.tracks.len() && e.b < sc.tracks.len(), "scene {id}: bad actor index");
                assert!(e.t + e.dur <= sc.len + 0.5, "scene {id}: effect after end");
            }
            for s in &sc.says {
                assert!(s.who < sc.tracks.len(), "scene {id}: bad speaker");
                assert!(s.text.chars().count() <= 24, "scene {id}: line too long for bubble");
            }
        }
    }

    #[test]
    fn actors_leave_the_stage_at_the_end() {
        for id in 0..6 {
            let sc = script(id);
            for tk in &sc.tracks {
                let last = tk.keys.last().unwrap();
                assert!(last.x < 0.0 || last.x > 1.0, "scene {id}: an actor is left standing on stage");
            }
        }
    }

    #[test]
    fn sample_interpolates_and_hides_outside() {
        let tk = tr(Who::Cinder, &[(1.0, 0.0, Walk, 1.0), (2.0, 1.0, Stand, 1.0)]);
        assert!(sample(&tk, 0.5).is_none());
        let (x, ..) = sample(&tk, 1.5).unwrap();
        assert!((x - 0.5).abs() < 0.01);
        assert!(sample(&tk, 2.5).is_none());
    }
}
