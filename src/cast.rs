//! The cast: four original 2D characters (two heroes, two villains) drawn from
//! simple shapes on a tiny skeleton, so they can run, jump, punch, cast and get
//! knocked back. Used by the top-bar scenes (see scene.rs).
//!
//!   Sparkplug  – goggled young inventor who shoots lightning
//!   Cinder     – ember knight with a burning plume, shield and fire sword
//!   Krag       – horned obsidian brute with magenta crystals
//!   Veil       – hooded shadow thief with twin daggers

use crate::gfx::*;

/// Characters are drawn at this fraction of their design size so they fit the bar.
pub const K: f32 = 0.62;

type V = (f32, f32);

#[derive(Clone, Copy, PartialEq)]
pub enum Who {
    Spark,
    Cinder,
    Krag,
    Veil,
    // pixel-art cast (see pixdata.rs)
    Ronin,
    Courier,
    Necro,
    // wizard-school cast (fan art, drawn by wizard())
    Harry,
    Hermione,
    Ron,
    Dumbledore,
    Voldemort,
    // press-conference scene (drawn in scene.rs): the photo head and reporters
    Face,
    Press(u8),
    Owl,
    Snitch,
    Dementor,
    // the games' heroes (see skins.rs)
    Thor,
    Superman,
}

impl Who {
    fn pixel_id(self) -> Option<u8> {
        match self {
            Who::Ronin => Some(0),
            Who::Courier => Some(1),
            Who::Necro => Some(2),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Act {
    Stand,
    Walk,
    Run,
    Slam,
    #[allow(dead_code)]
    Punch,
    Cast,
    Hurt,
    Jump,
    Cheer,
    Kneel,
    Block,
    Slash,
    Throw,
    #[allow(dead_code)]
    Crouch,
    /// flying forward, nearly horizontal (the flight game)
    Fly,
}

#[derive(Clone, Copy, PartialEq)]
enum Face {
    Normal,
    Shout,
    Hurt,
    Grin,
}

/// Everything needed to draw one actor.
#[derive(Clone, Copy)]
pub struct Draw {
    pub who: Who,
    /// screen position of the feet (ground contact)
    pub x: f32,
    pub y: f32,
    /// +1 faces right, -1 faces left
    pub dir: f32,
    pub act: Act,
    /// progress inside the current action, 0..1
    pub p: f32,
    /// time in seconds (drives cycles and flicker)
    pub t: f32,
    /// 0..1 weapon / hands glowing
    pub charge: f32,
}

// ------------------------------------------------------------------ helpers --

fn c(r: f32, g: f32, b: f32) -> Color {
    Color(r, g, b, 1.0)
}
fn ca(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color(r, g, b, a)
}
fn dim(c: Color, k: f32) -> Color {
    Color(c.0 * k, c.1 * k, c.2 * k, c.3)
}
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color(
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
        a.3 + (b.3 - a.3) * t,
    )
}
fn lerp(a: f32, b: f32, u: f32) -> f32 {
    a + (b - a) * u
}
fn sm(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}
fn seg(p: f32, a: f32, b: f32) -> f32 {
    if b > a {
        sm((p - a) / (b - a))
    } else {
        1.0
    }
}

struct Ctx<'a> {
    g: &'a Gfx,
    x: f32,
    y: f32,
    d: f32,
    k: f32,
    /// when set, everything is drawn in this colour (the dark outline pass)
    sil: Option<Color>,
}

impl<'a> Ctx<'a> {
    fn p(&self, v: V) -> V {
        (self.x + v.0 * self.d * self.k, self.y + v.1 * self.k)
    }
    fn col(&self, c: Color) -> Color {
        self.sil.unwrap_or(c)
    }
    fn circ(&self, v: V, r: f32, c: Color) {
        let p = self.p(v);
        self.g.fill_circle(p.0, p.1, r * self.k, self.col(c));
    }
    fn line(&self, a: V, b: V, w: f32, c: Color) {
        let (pa, pb) = (self.p(a), self.p(b));
        let c = self.col(c);
        let w = w * self.k;
        self.g.line(pa, pb, w, c);
        self.g.fill_circle(pa.0, pa.1, w / 2.0, c);
        self.g.fill_circle(pb.0, pb.1, w / 2.0, c);
    }
    fn poly(&self, pts: &[V], c: Color) {
        let v: Vec<V> = pts.iter().map(|p| self.p(*p)).collect();
        self.g.fill_poly(&v, self.col(c));
    }
    fn limb(&self, a: V, b: V, w: f32, c: Color) {
        self.line(a, b, w, c);
    }
}

// -------------------------------------------------------------------- poses --

#[derive(Clone, Copy)]
struct Sk {
    lean: f32,
    head: f32,
    arm_f: (f32, f32),
    arm_b: (f32, f32),
    leg_f: (f32, f32),
    leg_b: (f32, f32),
    weap: f32,
    face: Face,
    bob: f32,
}

fn pose(act: Act, p: f32, t: f32, who: Who) -> Sk {
    let krag = who == Who::Krag;
    let mut s = Sk {
        lean: 0.05,
        head: 0.0,
        arm_f: (0.3, 0.45),
        arm_b: (-0.2, 0.35),
        leg_f: (0.07, -0.04),
        leg_b: (-0.09, 0.0),
        weap: 2.55,
        face: Face::Normal,
        bob: 0.0,
    };
    let br = (t * 3.0).sin() * 0.04;
    match act {
        Act::Stand => {
            s.arm_f = (0.3 + br, 0.45);
            s.arm_b = (-0.2 - br, 0.35);
            if krag {
                s.lean = 0.28;
                s.arm_f = (0.35, 0.2);
                s.arm_b = (0.05, 0.2);
            }
        }
        Act::Walk | Act::Run => {
            let run = act == Act::Run;
            let f = if krag { 0.75 } else { 1.0 };
            let ph = t * if run { 12.5 * f } else { 7.5 * f };
            let amp = if run { 1.05 } else { 0.62 };
            let flex = if run { 1.5 } else { 0.9 };
            let leg = |ph: f32| (amp * ph.sin(), -flex * ph.cos().max(0.0) - 0.08);
            s.leg_f = leg(ph);
            s.leg_b = leg(ph + std::f32::consts::PI);
            let aa = if run { 1.0 } else { 0.55 };
            let el = if run { 1.3 } else { 0.5 };
            s.arm_f = (-aa * ph.sin() + 0.25, el);
            s.arm_b = (aa * ph.sin() + 0.1, el);
            s.lean = if run { 0.38 } else { 0.1 };
            s.weap = if run { 1.2 } else { 2.2 };
            if krag {
                s.lean += 0.18;
            }
        }
        Act::Slam => {
            let a = seg(p, 0.0, 0.5);
            let b = seg(p, 0.5, 0.68);
            let up = lerp(0.3, 2.85, a);
            let arm = lerp(up, 0.45, b);
            s.arm_f = (arm, lerp(0.3, 0.0, b));
            s.arm_b = (arm - 0.15, 0.1);
            s.lean = if p < 0.5 { lerp(0.1, -0.22, a) } else { lerp(-0.22, 0.75, b) };
            s.leg_f = (0.45, -0.25);
            s.leg_b = (-0.4, 0.0);
            s.face = Face::Shout;
            if krag {
                s.lean += 0.1;
            }
        }
        Act::Punch => {
            let a = seg(p, 0.0, 0.38);
            let b = seg(p, 0.38, 0.62);
            let cc = seg(p, 0.75, 1.0);
            s.arm_f = (
                lerp(lerp(0.3, -0.9, a), 1.5, b) * (1.0 - cc) + 0.3 * cc,
                lerp(lerp(0.45, 1.6, a), 0.0, b),
            );
            s.arm_b = (-0.5, 1.0);
            s.lean = lerp(lerp(0.05, -0.12, a), 0.5, b) * (1.0 - cc) + 0.05 * cc;
            s.leg_f = (0.6, -0.3);
            s.leg_b = (-0.55, 0.1);
            s.face = Face::Shout;
        }
        Act::Cast => {
            let u = seg(p, 0.0, 0.3);
            s.arm_f = (lerp(0.3, 1.35, u), lerp(0.45, 0.05, u));
            s.arm_b = (lerp(-0.2, -0.7, u), 0.35);
            s.lean = lerp(0.05, 0.18, u);
            s.leg_f = (0.4, -0.15);
            s.leg_b = (-0.45, 0.0);
            s.face = Face::Shout;
            s.weap = 1.5;
        }
        Act::Hurt => {
            let u = seg(p, 0.0, 0.25);
            s.lean = lerp(0.05, -0.5, u);
            s.head = lerp(0.0, -0.3, u);
            s.arm_f = (lerp(0.3, 1.0, u), 0.7);
            s.arm_b = (lerp(-0.2, -1.0, u), 0.5);
            s.leg_f = (0.35, -0.2);
            s.leg_b = (-0.55, 0.35);
            s.face = Face::Hurt;
            if krag {
                s.lean -= 0.1;
            }
        }
        Act::Jump => {
            let u = seg(p, 0.0, 0.3);
            s.arm_f = (lerp(0.3, 2.3, u), 0.3);
            s.arm_b = (lerp(-0.2, 2.0, u), 0.3);
            s.leg_f = (lerp(0.07, 0.95, u), lerp(-0.04, -1.5, u));
            s.leg_b = (lerp(-0.09, 0.25, u), lerp(0.0, -1.3, u));
            s.lean = 0.1;
            s.weap = 1.0;
            s.face = Face::Shout;
        }
        Act::Cheer => {
            let wv = (t * 9.0).sin();
            s.arm_f = (2.75 + 0.3 * wv, 0.25);
            s.arm_b = (2.55 - 0.3 * wv, 0.25);
            s.bob = (t * 6.0).sin().abs() * 2.2;
            s.lean = 0.0;
            s.face = Face::Grin;
            s.weap = 0.2;
            s.leg_f = (0.25, -0.3);
            s.leg_b = (-0.25, -0.3);
        }
        Act::Kneel => {
            s.leg_f = (1.45, -2.35);
            s.leg_b = (-0.5, -1.7);
            s.arm_f = (0.15, 0.2);
            s.arm_b = (0.05, 0.25);
            s.lean = 0.38;
            s.head = 0.35;
            s.face = Face::Hurt;
            s.weap = 3.0;
        }
        Act::Block => {
            s.arm_b = (1.3, 0.15);
            s.arm_f = (-0.5, 0.9);
            s.lean = -0.04;
            s.leg_f = (0.55, -0.1);
            s.leg_b = (-0.45, 0.1);
            s.weap = 2.9;
            s.face = Face::Shout;
        }
        Act::Slash => {
            let a = seg(p, 0.0, 0.35);
            let b = seg(p, 0.35, 0.62);
            let cc = seg(p, 0.8, 1.0);
            let arm = lerp(lerp(0.3, 2.5, a), 1.35, b);
            s.arm_f = (arm * (1.0 - cc) + 0.3 * cc, lerp(0.45, 0.15, b));
            s.arm_b = (-0.6, 0.6);
            s.weap = lerp(lerp(2.5, 4.1, a), 1.5, b);
            s.lean = lerp(lerp(0.05, -0.1, a), 0.5, b) * (1.0 - cc) + 0.05 * cc;
            s.leg_f = (0.55, -0.25);
            s.leg_b = (-0.5, 0.1);
            s.face = Face::Shout;
        }
        Act::Throw => {
            let u = seg(p, 0.0, 0.4);
            s.arm_f = (lerp(-0.8, 1.6, u), lerp(1.2, 0.0, u));
            s.arm_b = (-0.6, 0.4);
            s.lean = lerp(-0.1, 0.4, u);
            s.leg_f = (0.5, -0.2);
            s.leg_b = (-0.5, 0.1);
            s.face = Face::Shout;
        }
        Act::Fly => {
            let fl = (t * 14.0).sin() * 0.12;
            s.lean = 1.28;
            s.head = -0.95;
            s.arm_f = (1.62, 0.1);
            s.arm_b = (1.2 + fl, 0.35);
            s.leg_f = (-1.38 + fl, 0.3);
            s.leg_b = (-1.55 - fl, 0.1);
            s.weap = 1.45;
            s.face = Face::Shout;
        }
        Act::Crouch => {
            s.leg_f = (0.95, -1.5);
            s.leg_b = (-0.2, -1.2);
            s.arm_f = (0.7, 1.0);
            s.arm_b = (0.3, 1.0);
            s.lean = 0.45;
            s.face = Face::Shout;
        }
    }
    s
}

// ----------------------------------------------------------------- skeleton --

struct Spec {
    h: f32,
    hr: f32,
    t: f32,
    hw: f32,
    sw: f32,
    th: f32,
    ath: f32,
}

fn spec(who: Who) -> Spec {
    match who {
        Who::Spark => Spec { h: 32.0, hr: 5.6, t: 0.30, hw: 2.7, sw: 3.5, th: 3.1, ath: 2.8 },
        Who::Cinder => Spec { h: 34.0, hr: 6.0, t: 0.30, hw: 3.4, sw: 4.6, th: 3.8, ath: 3.5 },
        Who::Krag => Spec { h: 40.0, hr: 5.8, t: 0.31, hw: 4.4, sw: 6.8, th: 5.0, ath: 5.0 },
        Who::Veil => Spec { h: 33.0, hr: 5.3, t: 0.31, hw: 2.2, sw: 3.0, th: 2.7, ath: 2.4 },
        Who::Harry => Spec { h: 30.0, hr: 5.4, t: 0.30, hw: 2.8, sw: 3.4, th: 2.8, ath: 2.6 },
        Who::Hermione => Spec { h: 30.0, hr: 5.2, t: 0.30, hw: 2.7, sw: 3.3, th: 2.7, ath: 2.5 },
        Who::Ron => Spec { h: 33.0, hr: 5.4, t: 0.30, hw: 3.0, sw: 3.7, th: 2.9, ath: 2.7 },
        Who::Dumbledore => Spec { h: 36.0, hr: 5.6, t: 0.31, hw: 3.4, sw: 4.0, th: 3.1, ath: 2.9 },
        Who::Superman => Spec { h: 34.0, hr: 5.8, t: 0.30, hw: 3.2, sw: 5.0, th: 3.5, ath: 3.3 },
        Who::Thor => Spec { h: 34.0, hr: 6.2, t: 0.30, hw: 3.5, sw: 5.0, th: 3.8, ath: 3.5 },
        Who::Voldemort => Spec { h: 36.0, hr: 5.3, t: 0.31, hw: 2.6, sw: 3.4, th: 2.5, ath: 2.3 },
        _ => Spec { h: 30.0, hr: 5.0, t: 0.3, hw: 3.0, sw: 4.0, th: 3.0, ath: 3.0 },
    }
}

struct J {
    hip: V,
    sh: V,
    head: V,
    e_f: V,
    h_f: V,
    e_b: V,
    h_b: V,
    k_f: V,
    f_f: V,
    k_b: V,
    f_b: V,
    lean: f32,
    hr: f32,
    fa_f: f32,
}

fn solve(sk: &Sk, sp: &Spec) -> J {
    let (l1, l2) = (0.215 * sp.h, 0.215 * sp.h);
    let t = sp.t * sp.h;
    let (u, f) = (0.15 * sp.h, 0.15 * sp.h);
    let leg = |a: (f32, f32)| -> (V, V) {
        let k = (a.0.sin() * l1, a.0.cos() * l1);
        let ft = (k.0 + (a.0 + a.1).sin() * l2, k.1 + (a.0 + a.1).cos() * l2);
        (k, ft)
    };
    let (kf, ff) = leg(sk.leg_f);
    let (kb, fb) = leg(sk.leg_b);
    let low = ff.1.max(fb.1);
    let hip = (0.0, -low - sk.bob);
    let sh = (hip.0 + sk.lean.sin() * t, hip.1 - sk.lean.cos() * t);
    let ha = sk.lean + sk.head;
    let head = (sh.0 + ha.sin() * sp.hr * 0.95, sh.1 - ha.cos() * sp.hr * 0.95);
    let arm = |a: (f32, f32)| -> (V, V, f32) {
        let e = (sh.0 + a.0.sin() * u, sh.1 + a.0.cos() * u);
        let h = (e.0 + (a.0 + a.1).sin() * f, e.1 + (a.0 + a.1).cos() * f);
        (e, h, a.0 + a.1)
    };
    let (e_f, h_f, fa_f) = arm(sk.arm_f);
    let (e_b, h_b, _) = arm(sk.arm_b);
    J {
        hip,
        sh,
        head,
        e_f,
        h_f,
        e_b,
        h_b,
        k_f: (hip.0 + kf.0, hip.1 + kf.1),
        f_f: (hip.0 + ff.0, hip.1 + ff.1),
        k_b: (hip.0 + kb.0, hip.1 + kb.1),
        f_b: (hip.0 + fb.0, hip.1 + fb.1),
        lean: sk.lean,
        hr: sp.hr,
        fa_f,
    }
}

fn torso_quad(j: &J, hw: f32, sw: f32) -> [V; 4] {
    let (nx, ny) = (j.lean.cos(), j.lean.sin());
    [
        (j.hip.0 - nx * hw, j.hip.1 - ny * hw),
        (j.hip.0 + nx * hw, j.hip.1 + ny * hw),
        (j.sh.0 + nx * sw, j.sh.1 + ny * sw),
        (j.sh.0 - nx * sw, j.sh.1 - ny * sw),
    ]
}

// --------------------------------------------------------------------- draw --

/// Draw one actor. Shadow first, then a thin dark outline (so it reads on any
/// window behind it), then the figure itself.
pub fn draw_actor(g: &Gfx, d: &Draw) {
    draw_actor_k(g, d, K);
}

/// Same as `draw_actor` at another size (the games draw bigger than the top bar).
pub fn draw_actor_k(g: &Gfx, d: &Draw, k: f32) {
    if let Some(id) = d.who.pixel_id() {
        draw_pixel(g, d, id);
        return;
    }
    let sk = pose(d.act, d.p, d.t, d.who);
    let sp = spec(d.who);
    let j = solve(&sk, &sp);
    if d.act != Act::Fly {
        g.fill_rrect(
            Rect::new(d.x - sp.h * 0.2 * k, d.y - 1.0, sp.h * 0.4 * k, 2.2),
            1.1,
            ca(0.0, 0.0, 0.0, 0.28),
        );
    }
    let sil = ca(0.04, 0.04, 0.07, 0.55);
    for (ox, oy) in [(-0.8, 0.0), (0.8, 0.0), (0.0, -0.8), (0.0, 0.8)] {
        let cx = Ctx { g, x: d.x + ox, y: d.y + oy, d: d.dir, k, sil: Some(sil) };
        body(&cx, d, &sk, &sp, &j);
    }
    let cx = Ctx { g, x: d.x, y: d.y, d: d.dir, k, sil: None };
    body(&cx, d, &sk, &sp, &j);
}

fn body(cx: &Ctx, d: &Draw, sk: &Sk, sp: &Spec, j: &J) {
    match d.who {
        Who::Spark => spark(cx, d, sk, sp, j),
        Who::Thor => thor(cx, d, sk, sp, j),
        Who::Superman => superman(cx, d, sk, sp, j),
        Who::Cinder => cinder(cx, d, sk, sp, j),
        Who::Krag => krag(cx, d, sk, sp, j),
        Who::Veil => veil(cx, d, sk, sp, j),
        Who::Harry => wizard(cx, d, sk, sp, j, &Wiz {
            robe: c(0.09, 0.09, 0.12), dark: c(0.05, 0.05, 0.07), skin: c(0.98, 0.82, 0.68), hat: None,
            trim: c(0.75, 0.12, 0.15), beard: None, glow: c(1.0, 0.35, 0.3), wand: c(0.45, 0.30, 0.18), staff: false,
            hair: Some(c(0.07, 0.06, 0.07)), hair_style: 1, glasses: 1, scar: true,
            scarf: Some((c(0.75, 0.10, 0.12), c(0.95, 0.78, 0.25))), snake: false,
        }),
        Who::Hermione => wizard(cx, d, sk, sp, j, &Wiz {
            robe: c(0.09, 0.09, 0.12), dark: c(0.05, 0.05, 0.07), skin: c(0.96, 0.78, 0.64), hat: None,
            trim: c(0.75, 0.12, 0.15), beard: None, glow: c(0.6, 0.9, 1.0), wand: c(0.50, 0.33, 0.20), staff: false,
            hair: Some(c(0.45, 0.28, 0.15)), hair_style: 2, glasses: 0, scar: false,
            scarf: Some((c(0.75, 0.10, 0.12), c(0.95, 0.78, 0.25))), snake: false,
        }),
        Who::Ron => wizard(cx, d, sk, sp, j, &Wiz {
            robe: c(0.09, 0.09, 0.12), dark: c(0.05, 0.05, 0.07), skin: c(1.0, 0.86, 0.74), hat: None,
            trim: c(0.75, 0.12, 0.15), beard: None, glow: c(1.0, 0.65, 0.2), wand: c(0.55, 0.40, 0.25), staff: false,
            hair: Some(c(0.85, 0.35, 0.12)), hair_style: 0, glasses: 0, scar: false,
            scarf: Some((c(0.75, 0.10, 0.12), c(0.95, 0.78, 0.25))), snake: false,
        }),
        Who::Dumbledore => wizard(cx, d, sk, sp, j, &Wiz {
            robe: c(0.40, 0.20, 0.55), dark: c(0.28, 0.12, 0.40), skin: c(0.97, 0.82, 0.70), hat: Some(c(0.34, 0.16, 0.48)),
            trim: c(0.95, 0.78, 0.25), beard: Some(c(0.96, 0.96, 0.98)), glow: c(1.0, 0.88, 0.4), wand: c(0.9, 0.85, 0.7), staff: false,
            hair: Some(c(0.96, 0.96, 0.98)), hair_style: 3, glasses: 2, scar: false,
            scarf: None, snake: false,
        }),
        Who::Voldemort => wizard(cx, d, sk, sp, j, &Wiz {
            robe: c(0.07, 0.07, 0.09), dark: c(0.03, 0.03, 0.05), skin: c(0.86, 0.90, 0.84), hat: None,
            trim: c(0.25, 0.30, 0.25), beard: None, glow: c(0.4, 1.0, 0.5), wand: c(0.93, 0.91, 0.86), staff: false,
            hair: None, hair_style: 0, glasses: 0, scar: false, scarf: None, snake: true,
        }),
        _ => {}
    }
}

fn spark(cx: &Ctx, d: &Draw, sk: &Sk, sp: &Spec, j: &J) {
    let t = d.t;
    let jk = c(0.10, 0.60, 0.66);
    let jd = c(0.06, 0.40, 0.46);
    let pant = c(0.16, 0.18, 0.32);
    let boot = c(1.0, 0.62, 0.15);
    let glove = c(1.0, 0.86, 0.28);
    let skin = c(1.0, 0.83, 0.66);
    let hair = c(1.0, 0.86, 0.2);
    let ink = c(0.13, 0.12, 0.17);
    let hr = j.hr;
    let (sh, hip) = (j.sh, j.hip);
    // backpack + antennae
    let bp = ((sh.0 * 0.55 + hip.0 * 0.45) - 3.8, sh.1 * 0.55 + hip.1 * 0.45);
    cx.poly(
        &[(bp.0 - 2.2, bp.1 - 4.5), (bp.0 + 1.6, bp.1 - 4.5), (bp.0 + 1.6, bp.1 + 4.5), (bp.0 - 2.2, bp.1 + 4.5)],
        jd,
    );
    for (k, sx) in [-1.2f32, 0.2].iter().enumerate() {
        let kf = k as f32;
        let tip = (bp.0 + sx - 2.0 - kf * 1.2, bp.1 - 10.5 - kf * 1.5 + (t * 9.0 + kf).sin() * 0.6);
        cx.line((bp.0 + sx, bp.1 - 4.5), tip, 0.7, c(0.8, 0.85, 0.9));
        cx.circ(tip, 1.1, if (t * 6.0) as i32 % 2 == 0 { c(1.0, 0.9, 0.3) } else { c(1.0, 0.6, 0.2) });
    }
    // back limbs
    cx.limb(j.sh, j.e_b, sp.ath, dim(jk, 0.7));
    cx.limb(j.e_b, j.h_b, sp.ath - 0.2, dim(jk, 0.7));
    cx.circ(j.h_b, 2.1, dim(glove, 0.75));
    cx.limb(j.hip, j.k_b, sp.th, dim(pant, 0.7));
    cx.limb(j.k_b, j.f_b, sp.th - 0.1, dim(pant, 0.7));
    cx.circ(j.f_b, 2.5, dim(boot, 0.75));
    cx.circ((j.f_b.0 + 3.2, j.f_b.1 + 0.6), 2.0, dim(boot, 0.75));
    // torso
    cx.poly(&torso_quad(j, sp.hw, sp.sw), jk);
    cx.line(j.sh, (j.hip.0 + 0.2, j.hip.1), 0.6, ca(0.75, 1.0, 1.0, 0.8));
    cx.line((j.hip.0 - sp.hw, j.hip.1 - 0.3), (j.hip.0 + sp.hw, j.hip.1 + 0.3), 1.4, c(1.0, 0.62, 0.15));
    // front leg
    cx.limb(j.hip, j.k_f, sp.th, pant);
    cx.limb(j.k_f, j.f_f, sp.th - 0.1, pant);
    cx.circ(j.f_f, 2.6, boot);
    cx.circ((j.f_f.0 + 3.4, j.f_f.1 + 0.6), 2.1, boot);
    cx.line((j.f_f.0 - 1.2, j.f_f.1 - 2.4), (j.f_f.0 + 1.2, j.f_f.1 - 2.4), 0.7, c(1.0, 0.95, 0.7));
    // head
    let hc = j.head;
    cx.circ(hc, hr, skin);
    let sp_pts: [(f32, f32); 9] = [
        (-0.9, -0.65), (-0.95, -1.9), (-0.35, -1.0), (-0.2, -2.2), (0.25, -1.1),
        (0.75, -2.05), (0.8, -0.95), (1.3, -1.5), (1.15, -0.2),
    ];
    let mut pts: Vec<V> = vec![(hc.0 - hr * 0.95, hc.1 - hr * 0.1)];
    pts.extend(sp_pts.iter().map(|(x, y)| (hc.0 + x * hr, hc.1 + y * hr)));
    pts.push((hc.0 + hr * 1.0, hc.1 - hr * 0.15));
    cx.poly(&pts, hair);
    // goggles
    cx.line((hc.0 - hr * 0.95, hc.1 - hr * 0.38), (hc.0 + hr * 0.95, hc.1 - hr * 0.38), 1.5, c(0.2, 0.2, 0.3));
    for gx in [-0.35f32, 0.55] {
        cx.circ((hc.0 + gx * hr, hc.1 - hr * 0.5), 2.0, c(0.2, 0.2, 0.3));
        cx.circ((hc.0 + gx * hr, hc.1 - hr * 0.5), 1.35, c(0.6, 0.97, 1.0));
    }
    // face
    let (ex, ey) = (hc.0 + hr * 0.36, hc.1 + hr * 0.22);
    match sk.face {
        Face::Hurt => {
            cx.line((ex - 1.2, ey - 0.6), (ex + 1.0, ey + 0.6), 0.8, ink);
            cx.line((ex - 1.2, ey + 0.6), (ex + 1.0, ey - 0.6), 0.8, ink);
        }
        Face::Grin => {
            cx.line((ex - 1.4, ey + 0.5), (ex, ey - 0.7), 0.9, ink);
            cx.line((ex, ey - 0.7), (ex + 1.4, ey + 0.5), 0.9, ink);
        }
        _ => {
            cx.circ((ex, ey), 1.5, c(1.0, 1.0, 1.0));
            cx.circ((ex + 0.4, ey), 0.8, ink);
        }
    }
    cx.circ((ex - 2.6, ey + 0.1), 1.1, c(1.0, 1.0, 1.0));
    cx.circ((ex - 2.4, ey + 0.1), 0.6, ink);
    if matches!(sk.face, Face::Shout | Face::Grin) {
        cx.circ((ex + 0.6, ey + 2.6), 1.15, ink);
    } else {
        cx.line((ex - 0.2, ey + 2.5), (ex + 1.4, ey + 2.3), 0.7, ink);
    }
    cx.circ((ex + 1.3, ey + 1.4), 0.9, ca(1.0, 0.55, 0.55, 0.5));
    // front arm
    cx.limb(j.sh, j.e_f, sp.ath, jk);
    cx.limb(j.e_f, j.h_f, sp.ath - 0.2, jk);
    cx.circ(j.h_f, 2.2, glove);
    if d.charge > 0.0 || d.act == Act::Cast {
        cx.circ(j.h_f, 3.6, ca(0.6, 0.95, 1.0, 0.35));
        cx.circ(j.h_f, 2.0, ca(1.0, 1.0, 1.0, 0.8));
    }
}

fn thor(cx: &Ctx, d: &Draw, sk: &Sk, sp: &Spec, j: &J) {
    let t = d.t;
    let skin = c(0.98, 0.78, 0.62);
    let hair = c(0.86, 0.60, 0.22);
    let tun = c(0.20, 0.40, 0.74);
    let tund = c(0.12, 0.26, 0.52);
    let cape = c(0.80, 0.12, 0.16);
    let caped = c(0.52, 0.07, 0.12);
    let gold = c(0.98, 0.78, 0.28);
    let iron = c(0.78, 0.82, 0.90);
    let irond = c(0.46, 0.50, 0.60);
    let pant = c(0.30, 0.22, 0.17);
    let boot = c(0.42, 0.27, 0.16);
    let ink = c(0.12, 0.10, 0.14);
    let (hc, hr) = (j.head, j.hr);
    let (sh, hip) = (j.sh, j.hip);

    // cape streaming behind, with a rippling hem
    let rip = (t * 8.0).sin();
    let back = 13.0 + 9.0 * sk.lean.abs().min(1.4);
    let tip = (sh.0 - back + rip * 1.5, hip.1 + 3.0 - sk.lean * 3.0 + (t * 6.0).cos());
    let low = (hip.0 - back * 0.55, hip.1 + 5.0 + rip);
    cx.poly(&[(sh.0 - 0.5, sh.1 - 2.2), (sh.0 + 1.2, sh.1 + 1.5), low, tip, ((sh.0 + tip.0) / 2.0, sh.1 - 2.8 + rip * 0.8)], caped);
    cx.poly(&[(sh.0 - 0.5, sh.1 - 2.0), (sh.0 + 0.8, sh.1 + 0.8), ((low.0 + sh.0) / 2.0, low.1 - 1.6), (tip.0 + 3.0, tip.1 - 0.8), ((sh.0 + tip.0) / 2.0 + 1.5, sh.1 - 2.2)], cape);
    // back arm and leg
    cx.limb(j.sh, j.e_b, sp.ath, dim(tund, 0.9));
    cx.limb(j.e_b, j.h_b, sp.ath - 0.3, dim(skin, 0.8));
    cx.limb(j.hip, j.k_b, sp.th, dim(pant, 0.8));
    cx.limb(j.k_b, j.f_b, sp.th - 0.1, dim(boot, 0.8));
    cx.circ((j.f_b.0 + 2.2, j.f_b.1 + 0.5), 2.2, dim(boot, 0.7));
    // torso: blue tunic, silver chest plate, gold belt
    cx.poly(&torso_quad(j, sp.hw, sp.sw), tun);
    let q = torso_quad(j, sp.hw * 0.8, sp.sw * 0.95);
    let m = |a: V, b: V| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (m12, m03) = (m(q[1], q[2]), m(q[0], q[3]));
    cx.poly(&[m03, m12, q[2], q[3]], iron);
    cx.line(m(m03, q[3]), m(m12, q[2]), 0.9, irond);
    cx.circ(m(m(m03, q[3]), m(m12, q[2])), 1.4, gold);
    cx.line((hip.0 - sp.hw, hip.1 - 0.6), (hip.0 + sp.hw, hip.1 + 0.6), 1.9, gold);
    // front leg
    cx.limb(j.hip, j.k_f, sp.th, pant);
    cx.limb(j.k_f, j.f_f, sp.th - 0.1, boot);
    cx.circ(j.k_f, 1.9, irond);
    cx.circ((j.f_f.0 + 2.4, j.f_f.1 + 0.5), 2.4, boot);
    cx.circ(j.f_f, 2.6, boot);
    // head: braid, face, beard, winged helm
    cx.line((hc.0 - hr * 0.7, hc.1 + hr * 0.1), (hc.0 - hr * 1.6, hc.1 + hr * 1.5 + rip * 0.4), 1.8, hair);
    cx.circ(hc, hr, skin);
    cx.poly(
        &[
            (hc.0 + hr * 1.0, hc.1 + hr * 0.0),
            (hc.0 + hr * 0.95, hc.1 + hr * 1.05),
            (hc.0 + hr * 0.25, hc.1 + hr * 1.75),
            (hc.0 - hr * 0.7, hc.1 + hr * 0.85),
            (hc.0 - hr * 0.2, hc.1 + hr * 0.15),
        ],
        hair,
    );
    cx.circ((hc.0 + hr * 0.55, hc.1 + hr * 0.5), hr * 0.32, skin); // mouth gap
    let hcap = (hc.0 - hr * 0.05, hc.1 - hr * 0.3);
    cx.circ(hcap, hr * 1.02, iron);
    cx.poly(&[(hc.0 - hr * 1.0, hc.1 - hr * 0.15), (hc.0 + hr * 1.05, hc.1 - hr * 0.2), (hc.0 + hr * 1.05, hc.1 + hr * 0.12), (hc.0 - hr * 1.0, hc.1 + hr * 0.12)], irond);
    cx.line((hc.0, hc.1 - hr * 1.2), (hc.0, hc.1 - hr * 0.2), 0.9, irond);
    // two curled horns
    for (sx, up) in [(-0.95f32, -1.0f32), (0.9, -0.9)] {
        let base = (hc.0 + hr * sx, hc.1 - hr * 0.55);
        let mid = (base.0 + sx * hr * 0.65, base.1 - hr * 0.55);
        let end = (mid.0 + sx * hr * 0.15, mid.1 + up * hr * 0.75);
        cx.line(base, mid, 2.4, c(0.96, 0.94, 0.84));
        cx.line(mid, end, 1.5, c(0.96, 0.94, 0.84));
    }
    let (ex, ey) = (hc.0 + hr * 0.5, hc.1 + hr * 0.05);
    match sk.face {
        Face::Hurt => {
            cx.line((ex - 1.1, ey - 0.6), (ex + 1.0, ey + 0.6), 0.8, ink);
            cx.line((ex - 1.1, ey + 0.6), (ex + 1.0, ey - 0.6), 0.8, ink);
        }
        Face::Grin => {
            cx.line((ex - 1.3, ey + 0.5), (ex, ey - 0.6), 0.9, ink);
            cx.line((ex, ey - 0.6), (ex + 1.3, ey + 0.5), 0.9, ink);
        }
        _ => {
            cx.circ((ex, ey), 1.35, c(1.0, 1.0, 1.0));
            cx.circ((ex + 0.35, ey), 0.8, c(0.25, 0.55, 0.9));
            cx.circ((ex + 0.4, ey), 0.4, ink);
            cx.line((ex - 1.5, ey - 1.7), (ex + 1.6, ey - 1.2), 0.9, hair);
        }
    }
    // front arm and the hammer
    cx.circ(j.sh, 2.9, iron);
    cx.limb(j.sh, j.e_f, sp.ath, tund);
    cx.limb(j.e_f, j.h_f, sp.ath - 0.3, skin);
    cx.line((j.e_f.0 + (j.h_f.0 - j.e_f.0) * 0.45, j.e_f.1 + (j.h_f.1 - j.e_f.1) * 0.45), j.h_f, sp.ath, gold);
    let w = sk.weap;
    let hf = j.h_f;
    let (dx, dy) = (w.sin(), w.cos());
    let (px, py) = (dy, -dx);
    let end = (hf.0 + dx * 11.0, hf.1 + dy * 11.0);
    cx.line((hf.0 - dx * 2.0, hf.1 - dy * 2.0), end, 1.7, c(0.45, 0.30, 0.18));
    let hd = |a: f32, b: f32| (end.0 + dx * a + px * b, end.1 + dy * a + py * b);
    cx.poly(&[hd(-3.0, -5.2), hd(3.0, -5.2), hd(3.0, 5.2), hd(-3.0, 5.2)], irond);
    cx.poly(&[hd(-2.0, -4.4), hd(2.4, -4.4), hd(2.4, 4.4), hd(-2.0, 4.4)], iron);
    cx.line(hd(-0.2, -4.4), hd(-0.2, 4.4), 0.9, irond);
    if d.act == Act::Fly || d.charge > 0.0 {
        // lightning crackling off the hammer
        let z = (t * 18.0).sin();
        cx.line(hd(3.0, 5.0), hd(7.0, 7.5 + z), 0.9, c(0.7, 0.9, 1.0));
        cx.line(hd(3.0, -5.0), hd(8.0, -6.5 - z), 0.9, c(0.7, 0.9, 1.0));
        cx.circ(hd(0.0, 0.0), 6.0, ca(0.6, 0.85, 1.0, 0.18));
    }
}

fn superman(cx: &Ctx, d: &Draw, sk: &Sk, sp: &Spec, j: &J) {
    let t = d.t;
    let skin = c(1.0, 0.82, 0.68);
    let blue = c(0.12, 0.34, 0.80);
    let red = c(0.85, 0.10, 0.14);
    let redd = c(0.55, 0.06, 0.10);
    let gold = c(1.0, 0.82, 0.20);
    let hair = c(0.09, 0.08, 0.12);
    let ink = c(0.12, 0.10, 0.14);
    let (hc, hr) = (j.head, j.hr);
    let (sh, hip) = (j.sh, j.hip);

    // cape streaming behind, rippling
    let rip = (t * 9.0).sin();
    let back = 14.0 + 10.0 * sk.lean.abs().min(1.4);
    let tip = (sh.0 - back + rip * 1.6, hip.1 + 3.0 - sk.lean * 3.5 + (t * 6.0).cos());
    let low = (hip.0 - back * 0.55, hip.1 + 5.5 + rip);
    cx.poly(&[(sh.0 - 0.5, sh.1 - 2.4), (sh.0 + 1.2, sh.1 + 1.6), low, tip, ((sh.0 + tip.0) / 2.0, sh.1 - 3.0 + rip * 0.8)], redd);
    cx.poly(&[(sh.0 - 0.5, sh.1 - 2.1), (sh.0 + 0.8, sh.1 + 0.9), ((low.0 + sh.0) / 2.0, low.1 - 1.7), (tip.0 + 3.2, tip.1 - 0.8), ((sh.0 + tip.0) / 2.0 + 1.5, sh.1 - 2.3)], red);
    // back arm and leg
    cx.limb(j.sh, j.e_b, sp.ath, dim(blue, 0.8));
    cx.limb(j.e_b, j.h_b, sp.ath - 0.2, dim(blue, 0.8));
    cx.circ(j.h_b, 2.0, dim(skin, 0.8));
    cx.limb(j.hip, j.k_b, sp.th, dim(blue, 0.8));
    cx.limb(j.k_b, j.f_b, sp.th - 0.1, dim(red, 0.8));
    cx.circ((j.f_b.0 + 2.0, j.f_b.1 + 0.4), 2.1, dim(red, 0.7));
    // torso, chest shield with an S, belt and trunks
    cx.poly(&torso_quad(j, sp.hw, sp.sw), blue);
    let m = |a: V, b: V| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let chest = m(m(j.sh, j.hip), j.sh);
    let (fx, fy) = ((j.sh.0 - j.hip.0) * 0.3, (j.sh.1 - j.hip.1) * 0.3); // up the body
    let (sx, sy) = (-fy, fx); // across the body
    let pt = |a: f32, b: f32| (chest.0 + sx * a + fx * b, chest.1 + sy * a + fy * b);
    cx.poly(&[pt(-0.15, 0.5), pt(0.55, 0.5), pt(0.62, -0.05), pt(0.2, -0.6), pt(-0.2, -0.05)], gold);
    cx.poly(&[pt(-0.05, 0.38), pt(0.42, 0.38), pt(0.48, -0.03), pt(0.2, -0.46), pt(-0.08, -0.03)], red);
    cx.line(pt(0.36, 0.22), pt(0.08, 0.14), 0.9, gold);
    cx.line(pt(0.08, 0.14), pt(0.34, -0.12), 0.9, gold);
    cx.line(pt(0.34, -0.12), pt(0.06, -0.24), 0.9, gold);
    cx.poly(&[(hip.0 - sp.hw - 0.4, hip.1 - 1.6), (hip.0 + sp.hw + 0.4, hip.1 + 1.6), (hip.0 + sp.hw * 0.8, hip.1 + 4.2), (hip.0 - sp.hw * 0.8, hip.1 + 3.6)], red);
    cx.line((hip.0 - sp.hw - 0.2, hip.1 - 1.0), (hip.0 + sp.hw + 0.2, hip.1 + 1.0), 1.6, gold);
    // front leg and boot
    cx.limb(j.hip, j.k_f, sp.th, blue);
    cx.limb(j.k_f, j.f_f, sp.th - 0.1, red);
    cx.circ(j.f_f, 2.5, red);
    cx.circ((j.f_f.0 + 2.2, j.f_f.1 + 0.4), 2.3, red);
    // head: face, black hair with the curl, eyes
    cx.circ(hc, hr, skin);
    cx.poly(
        &[
            (hc.0 - hr * 1.05, hc.1 + hr * 0.35),
            (hc.0 - hr * 1.0, hc.1 - hr * 0.7),
            (hc.0 - hr * 0.2, hc.1 - hr * 1.15),
            (hc.0 + hr * 0.8, hc.1 - hr * 0.85),
            (hc.0 + hr * 1.0, hc.1 - hr * 0.35),
            (hc.0 + hr * 0.25, hc.1 - hr * 0.45),
            (hc.0 - hr * 0.35, hc.1 - hr * 0.15),
            (hc.0 - hr * 0.6, hc.1 + hr * 0.4),
        ],
        hair,
    );
    cx.circ((hc.0 + hr * 0.45, hc.1 - hr * 0.62), hr * 0.2, hair); // the curl
    let (ex, ey) = (hc.0 + hr * 0.5, hc.1 + hr * 0.12);
    match sk.face {
        Face::Hurt => {
            cx.line((ex - 1.1, ey - 0.6), (ex + 1.0, ey + 0.6), 0.8, ink);
            cx.line((ex - 1.1, ey + 0.6), (ex + 1.0, ey - 0.6), 0.8, ink);
        }
        _ => {
            cx.circ((ex, ey), 1.3, c(1.0, 1.0, 1.0));
            cx.circ((ex + 0.35, ey), 0.75, c(0.2, 0.45, 0.85));
            cx.circ((ex + 0.4, ey), 0.38, ink);
            cx.line((ex - 1.5, ey - 1.7), (ex + 1.6, ey - 1.1), 0.9, hair);
        }
    }
    cx.line((ex - 0.4, ey + 2.4), (ex + 1.6, ey + 2.2), 0.7, ink);
    // front arm: fist leading the flight
    cx.limb(j.sh, j.e_f, sp.ath, blue);
    cx.limb(j.e_f, j.h_f, sp.ath - 0.2, blue);
    cx.line((j.e_f.0 + (j.h_f.0 - j.e_f.0) * 0.55, j.e_f.1 + (j.h_f.1 - j.e_f.1) * 0.55), j.h_f, sp.ath - 0.1, red);
    cx.circ(j.h_f, 2.5, skin);
    if d.act == Act::Fly {
        // speed streaks behind
        for k in 0..3 {
            let kf = k as f32;
            let y = hip.1 - 4.0 + kf * 4.5 + (t * 12.0 + kf).sin() * 0.6;
            let l = 8.0 + kf * 3.0 + (t * 15.0 + kf * 2.0).sin() * 2.0;
            cx.line((hip.0 - 14.0 - kf * 3.0, y), (hip.0 - 14.0 - kf * 3.0 - l, y), 0.9, ca(1.0, 1.0, 1.0, 0.35));
        }
    }
}

fn cinder(cx: &Ctx, d: &Draw, sk: &Sk, sp: &Spec, j: &J) {
    let t = d.t;
    let ar = c(0.80, 0.30, 0.14);
    let ard = c(0.45, 0.15, 0.10);
    let und = c(0.18, 0.14, 0.16);
    let gold = c(0.98, 0.74, 0.25);
    let ember = c(1.0, 0.62, 0.15);
    let flame = c(1.0, 0.9, 0.4);
    let (hc, hr) = (j.head, j.hr);
    // back (shield) arm and leg
    cx.limb(j.sh, j.e_b, sp.ath, dim(ard, 0.8));
    cx.limb(j.e_b, j.h_b, sp.ath - 0.2, dim(ard, 0.8));
    cx.limb(j.hip, j.k_b, sp.th, dim(und, 0.9));
    cx.limb(j.k_b, j.f_b, sp.th - 0.1, dim(ard, 0.7));
    cx.circ(j.f_b, 2.9, dim(und, 0.9));
    cx.circ((j.f_b.0 + 2.8, j.f_b.1 + 0.5), 2.1, dim(und, 0.9));
    // burning plume
    for (k, (bx, ln, col)) in [(-0.2f32, 15.0f32, ember), (-0.7, 12.0, c(1.0, 0.45, 0.12)), (0.3, 11.0, flame)]
        .iter()
        .enumerate()
    {
        let kf = k as f32;
        let fl = (t * 11.0 + kf * 2.1).sin() * 1.6;
        let base = (hc.0 + bx * hr * 0.5 - 0.5, hc.1 - hr * 0.8);
        let tip = (base.0 - ln * 0.45 + fl, base.1 - ln * 0.55 - (t * 8.0 + kf).sin() * 1.2);
        cx.poly(
            &[
                (base.0 - 2.4, base.1 + 1.2),
                (base.0 + 2.0, base.1 + 1.0),
                ((base.0 + tip.0) / 2.0 + 1.5 + fl * 0.3, (base.1 + tip.1) / 2.0 - 1.0),
                tip,
                ((base.0 + tip.0) / 2.0 - 1.8, (base.1 + tip.1) / 2.0 + 0.4),
            ],
            *col,
        );
    }
    // torso armour
    cx.poly(&torso_quad(j, sp.hw, sp.sw), und);
    let q = torso_quad(j, sp.hw - 0.3, sp.sw + 0.3);
    let mid = ((j.hip.0 + j.sh.0) / 2.0, (j.hip.1 + j.sh.1) / 2.0);
    let m12 = ((q[1].0 + q[2].0) / 2.0, (q[1].1 + q[2].1) / 2.0);
    let m03 = ((q[0].0 + q[3].0) / 2.0, (q[0].1 + q[3].1) / 2.0);
    cx.poly(&[q[0], q[1], (m12.0 + 0.3, m12.1), (m03.0 - 0.3, m03.1)], ar);
    cx.poly(&[m03, m12, q[2], q[3]], ar);
    cx.line((mid.0 - 1.5, mid.1 - 3.2), (mid.0 + 0.4, mid.1 - 0.6), 0.8, ember);
    cx.line((mid.0 + 0.4, mid.1 - 0.6), (mid.0 - 0.6, mid.1 + 2.4), 0.8, ember);
    cx.line((j.hip.0 - sp.hw, j.hip.1 - 0.2), (j.hip.0 + sp.hw, j.hip.1 + 0.2), 1.8, gold);
    // front leg
    cx.limb(j.hip, j.k_f, sp.th, und);
    cx.limb(j.k_f, j.f_f, sp.th - 0.1, ar);
    cx.circ(j.k_f, 2.2, gold);
    cx.circ(j.f_f, 3.0, c(0.22, 0.16, 0.18));
    cx.circ((j.f_f.0 + 3.0, j.f_f.1 + 0.5), 2.2, c(0.22, 0.16, 0.18));
    // helm
    cx.circ(hc, hr, ar);
    cx.poly(
        &[
            (hc.0 + hr * 0.1, hc.1 - hr * 0.35),
            (hc.0 + hr * 1.0, hc.1 - hr * 0.3),
            (hc.0 + hr * 0.9, hc.1 + hr * 0.25),
            (hc.0 + hr * 0.1, hc.1 + hr * 0.25),
        ],
        c(0.08, 0.06, 0.08),
    );
    let lit = if sk.face != Face::Hurt { c(1.0, 0.95, 0.5) } else { c(0.6, 0.4, 0.2) };
    cx.circ((hc.0 + hr * 0.35, hc.1 - hr * 0.05), 1.1, lit);
    cx.circ((hc.0 + hr * 0.8, hc.1 - hr * 0.05), 1.0, lit);
    cx.line((hc.0 - hr * 0.1, hc.1 - hr * 0.95), (hc.0 - hr * 0.2, hc.1 - hr * 0.15), 1.0, gold);
    cx.poly(
        &[
            (hc.0 + hr * 0.1, hc.1 + hr * 0.3),
            (hc.0 + hr * 0.95, hc.1 + hr * 0.3),
            (hc.0 + hr * 0.7, hc.1 + hr * 0.95),
            (hc.0 - hr * 0.1, hc.1 + hr * 0.8),
        ],
        dim(ar, 0.85),
    );
    // pauldron + front arm
    cx.circ(j.sh, 3.7, gold);
    cx.circ(j.sh, 3.0, ar);
    cx.limb(j.sh, j.e_f, sp.ath, ard);
    cx.limb(j.e_f, j.h_f, sp.ath - 0.2, ard);
    cx.circ(j.h_f, 2.6, gold);
    // sword
    let w = sk.weap;
    let hf = j.h_f;
    let tip = (hf.0 + w.sin() * 13.0, hf.1 + w.cos() * 13.0);
    cx.line(hf, tip, 3.2 + 3.5 * d.charge, ca(1.0, 0.55, 0.15, 0.35 + 0.4 * d.charge));
    cx.line(hf, tip, 1.5, if d.charge > 0.5 { c(1.0, 1.0, 1.0) } else { c(0.92, 0.94, 1.0) });
    let gx = (w + 1.57).sin() * 2.2;
    let gy = (w + 1.57).cos() * 2.2;
    cx.line((hf.0 - gx, hf.1 - gy), (hf.0 + gx, hf.1 + gy), 1.1, gold);
    // shield
    let hb = j.h_b;
    let sc = (hb.0 + 1.8, hb.1 - 0.5);
    cx.circ(sc, 6.3, c(0.30, 0.12, 0.10));
    cx.circ(sc, 5.4, ar);
    cx.circ(sc, 3.4, c(0.35, 0.14, 0.10));
    cx.circ(sc, 2.0, ember);
    for k in 0..6 {
        let aa = k as f32 * std::f32::consts::PI / 3.0;
        cx.line(
            (sc.0 + aa.cos() * 3.6, sc.1 + aa.sin() * 3.6),
            (sc.0 + aa.cos() * 5.0, sc.1 + aa.sin() * 5.0),
            0.8,
            gold,
        );
    }
    cx.circ(hb, 1.8, gold);
}

fn krag(cx: &Ctx, d: &Draw, sk: &Sk, sp: &Spec, j: &J) {
    let t = d.t;
    let skin = c(0.52, 0.56, 0.70);
    let skd = c(0.37, 0.40, 0.55);
    let obs = c(0.20, 0.19, 0.29);
    let obs2 = c(0.30, 0.28, 0.42);
    let cr = c(1.0, 0.3, 0.85);
    let crl = c(1.0, 0.7, 0.96);
    let bone = c(0.92, 0.88, 0.78);
    let gold = c(0.72, 0.56, 0.2);
    let hc = (j.head.0 + 3.2, j.head.1 + 1.0);
    let hr = j.hr;
    // back arm / leg
    cx.limb(j.sh, j.e_b, sp.ath, dim(skd, 0.9));
    cx.limb(j.e_b, j.h_b, sp.ath - 0.2, dim(skd, 0.9));
    cx.circ(j.h_b, 4.0, dim(skd, 0.8));
    cx.limb(j.hip, j.k_b, sp.th, dim(obs, 0.9));
    cx.limb(j.k_b, j.f_b, sp.th - 0.2, dim(obs, 0.9));
    cx.circ(j.f_b, 3.6, dim(obs2, 0.8));
    cx.circ((j.f_b.0 + 3.4, j.f_b.1 + 0.6), 2.6, dim(obs2, 0.8));
    // torso
    cx.poly(&torso_quad(j, sp.hw, sp.sw), skin);
    let q = torso_quad(j, sp.hw - 0.4, sp.sw - 0.2);
    cx.poly(&q, obs);
    let mid = ((j.hip.0 + j.sh.0) / 2.0, (j.hip.1 + j.sh.1) / 2.0);
    let pul = 0.5 + 0.5 * (t * 4.0).sin();
    let glow = ca(1.0, 0.3, 0.85, 0.6 + 0.4 * pul);
    cx.line((mid.0 - 2.5, mid.1 - 5.0), (mid.0 + 0.5, mid.1 - 1.0), 1.0, glow);
    cx.line((mid.0 + 0.5, mid.1 - 1.0), (mid.0 - 1.0, mid.1 + 3.5), 1.0, glow);
    cx.line((mid.0 + 3.0, mid.1 - 3.0), (mid.0 + 1.5, mid.1 + 0.5), 0.8, glow);
    cx.line((j.hip.0 - sp.hw, j.hip.1 - 0.4), (j.hip.0 + sp.hw, j.hip.1 + 0.4), 2.4, gold);
    for k in 0..3 {
        let kf = k as f32;
        cx.poly(
            &[
                (j.hip.0 - 3.0 + kf * 3.0, j.hip.1 - 1.2),
                (j.hip.0 - 2.0 + kf * 3.0, j.hip.1 - 3.4),
                (j.hip.0 - 1.0 + kf * 3.0, j.hip.1 - 1.2),
            ],
            bone,
        );
    }
    // front leg
    cx.limb(j.hip, j.k_f, sp.th, obs);
    cx.limb(j.k_f, j.f_f, sp.th - 0.2, obs2);
    cx.circ(j.k_f, 2.8, obs2);
    cx.circ(j.f_f, 3.7, c(0.2, 0.18, 0.27));
    cx.circ((j.f_f.0 + 3.6, j.f_f.1 + 0.6), 2.7, c(0.2, 0.18, 0.27));
    // pauldron + crystals (behind the head)
    let sh = j.sh;
    cx.circ(sh, 5.4, obs2);
    cx.circ(sh, 4.4, obs);
    for (dx, dy, h2) in [(-3.4f32, -3.0f32, 7.0f32), (-0.4, -4.0, 9.0), (2.2, -2.8, 5.5)] {
        cx.poly(
            &[(sh.0 + dx - 1.6, sh.1 + dy + 1.0), (sh.0 + dx + 1.6, sh.1 + dy + 1.0), (sh.0 + dx + 0.3, sh.1 + dy - h2 * 0.55)],
            mix(cr, crl, 0.2),
        );
        cx.poly(
            &[(sh.0 + dx - 0.6, sh.1 + dy + 0.6), (sh.0 + dx + 0.9, sh.1 + dy + 0.6), (sh.0 + dx + 0.2, sh.1 + dy - h2 * 0.4)],
            crl,
        );
    }
    // horns, head
    for (ox, oy) in [(-0.45f32, -0.5f32), (0.45, -0.6)] {
        let b = (hc.0 + ox * hr, hc.1 + oy * hr);
        let pts = [
            b,
            (b.0 + ox * 5.5 - 0.5, b.1 - 3.2),
            (b.0 + ox * 5.8 + 2.5, b.1 - 7.2),
            (b.0 + ox * 3.0 + 4.6, b.1 - 9.5),
        ];
        for i in 0..3 {
            cx.line(pts[i], pts[i + 1], 3.0 - i as f32 * 0.9, bone);
        }
    }
    cx.circ(hc, hr, skin);
    cx.poly(
        &[(hc.0 - hr * 0.2, hc.1 - hr * 0.95), (hc.0 + hr * 1.0, hc.1 - hr * 0.4), (hc.0 + hr * 0.8, hc.1 - hr * 0.9)],
        obs,
    );
    cx.circ((hc.0 + hr * 0.1, hc.1 - hr * 0.62), hr * 0.45, obs);
    cx.poly(
        &[
            (hc.0 - hr * 0.6, hc.1 + hr * 0.35),
            (hc.0 + hr * 1.0, hc.1 + hr * 0.2),
            (hc.0 + hr * 0.8, hc.1 + hr * 0.95),
            (hc.0 - hr * 0.2, hc.1 + hr * 0.9),
        ],
        skd,
    );
    cx.poly(
        &[(hc.0 + hr * 0.45, hc.1 + hr * 0.3), (hc.0 + hr * 0.65, hc.1 - hr * 0.25), (hc.0 + hr * 0.8, hc.1 + hr * 0.3)],
        bone,
    );
    cx.poly(
        &[(hc.0 - hr * 0.05, hc.1 + hr * 0.3), (hc.0 + hr * 0.12, hc.1 - hr * 0.15), (hc.0 + hr * 0.28, hc.1 + hr * 0.3)],
        bone,
    );
    let eye = if sk.face != Face::Hurt { c(1.0, 0.4, 0.9) } else { c(0.7, 0.4, 0.7) };
    cx.circ((hc.0 + hr * 0.45, hc.1 - hr * 0.05), 1.5, eye);
    cx.circ((hc.0 + hr * 0.45, hc.1 - hr * 0.05), 0.7, c(1.0, 1.0, 1.0));
    cx.circ((hc.0 - hr * 0.1, hc.1 - hr * 0.05), 1.2, eye);
    cx.line((hc.0 - hr * 0.35, hc.1 - hr * 0.45), (hc.0 + hr * 0.8, hc.1 - hr * 0.18), 1.0, obs);
    // front arm
    cx.limb(j.sh, j.e_f, sp.ath, skin);
    cx.limb(j.e_f, j.h_f, sp.ath + 0.2, obs2);
    cx.circ(j.h_f, 4.3, skd);
    if d.charge > 0.0 {
        cx.circ(j.h_f, 7.0, ca(1.0, 0.3, 0.85, 0.35 * d.charge));
    }
}

/// Look of one wizard: they share a body and differ in colours and props.
struct Wiz {
    robe: Color,
    dark: Color,
    skin: Color,
    /// pointed hat colour
    hat: Option<Color>,
    trim: Color,
    beard: Option<Color>,
    glow: Color,
    wand: Color,
    /// walking staff with an orb instead of a wand
    staff: bool,
    hair: Option<Color>,
    /// 0 neat, 1 messy and spiky, 2 big and bushy, 3 long
    hair_style: u8,
    /// 0 none, 1 round, 2 half-moon
    glasses: u8,
    scar: bool,
    /// house scarf stripes
    scarf: Option<(Color, Color)>,
    /// pale, noseless, red-eyed
    snake: bool,
}

fn wizard(cx: &Ctx, d: &Draw, sk: &Sk, sp: &Spec, j: &J, w: &Wiz) {
    let ink = c(0.10, 0.10, 0.14);
    let boot = c(0.16, 0.11, 0.09);
    let (hc, hr) = (j.head, j.hr);
    let gold = c(0.95, 0.78, 0.25);
    // back arm and leg
    cx.limb(j.sh, j.e_b, sp.ath, dim(w.robe, 0.7));
    cx.limb(j.e_b, j.h_b, sp.ath - 0.2, dim(w.robe, 0.7));
    cx.circ(j.h_b, 1.8, dim(w.skin, 0.8));
    cx.limb(j.hip, j.k_b, sp.th, dim(w.dark, 0.8));
    cx.limb(j.k_b, j.f_b, sp.th - 0.1, dim(w.dark, 0.8));
    cx.circ(j.f_b, 2.4, dim(boot, 0.8));
    // front leg
    cx.limb(j.hip, j.k_f, sp.th, w.dark);
    cx.limb(j.k_f, j.f_f, sp.th - 0.1, w.dark);
    cx.circ(j.f_f, 2.5, boot);
    cx.circ((j.f_f.0 + 2.6, j.f_f.1 + 0.5), 1.9, boot);
    // robe: torso plus a skirt that hides the legs down to near the boots
    let hem = j.hip.1 * 0.2;
    cx.poly(&torso_quad(j, sp.hw, sp.sw), w.robe);
    cx.poly(
        &[
            (j.hip.0 - sp.hw - 0.5, j.hip.1 - 1.0),
            (j.hip.0 + sp.hw + 0.5, j.hip.1 - 1.0),
            (j.hip.0 + sp.hw + 3.0, hem),
            (j.hip.0 - sp.hw - 3.0, hem),
        ],
        w.robe,
    );
    cx.line((j.hip.0 - sp.hw - 3.0, hem), (j.hip.0 + sp.hw + 3.0, hem), 1.0, w.trim);
    cx.line((j.hip.0 - sp.hw, j.hip.1 - 0.5), (j.hip.0 + sp.hw, j.hip.1 + 0.5), 1.2, w.trim);
    // hair behind the head
    if let Some(h) = w.hair {
        match w.hair_style {
            2 => cx.circ((hc.0 - hr * 0.55, hc.1 + hr * 0.1), hr * 1.35, h),
            3 => cx.poly(
                &[
                    (hc.0 - hr * 0.9, hc.1),
                    (hc.0 - hr * 1.1, hc.1 + hr * 2.4),
                    (hc.0 - hr * 0.1, hc.1 + hr * 1.7),
                ],
                h,
            ),
            _ => {}
        }
    }
    // head
    cx.circ(hc, hr, w.skin);
    if let Some(b) = w.beard {
        cx.poly(
            &[
                (hc.0 - hr * 0.3, hc.1 + hr * 0.3),
                (hc.0 + hr * 0.9, hc.1 + hr * 0.25),
                (hc.0 + hr * 0.5, hc.1 + hr * 2.4),
                (hc.0 - hr * 0.1, hc.1 + hr * 1.8),
            ],
            b,
        );
    }
    // hair on top: a cap sweeping down to a fringe
    if let Some(h) = w.hair {
        cx.poly(
            &[
                (hc.0 - hr * 1.1, hc.1 + hr * 0.2),
                (hc.0 - hr * 1.0, hc.1 - hr * 0.9),
                (hc.0 + hr * 0.2, hc.1 - hr * 1.15),
                (hc.0 + hr * 1.0, hc.1 - hr * 0.6),
                (hc.0 + hr * 0.55, hc.1 - hr * 0.25),
                (hc.0 + hr * 0.0, hc.1 - hr * 0.45),
                (hc.0 - hr * 0.5, hc.1 + hr * 0.1),
            ],
            h,
        );
        if w.hair_style == 1 {
            for k in 0..3 {
                let x0 = hc.0 - hr * 0.7 + k as f32 * hr * 0.65;
                cx.poly(
                    &[
                        (x0 - hr * 0.3, hc.1 - hr * 0.9),
                        (x0 + hr * 0.3, hc.1 - hr * 0.9),
                        (x0 + hr * 0.15 - k as f32 * hr * 0.15, hc.1 - hr * 1.6),
                    ],
                    h,
                );
            }
        }
    }
    // face
    let (ex, ey) = (hc.0 + hr * 0.42, hc.1 + hr * 0.05);
    let eye = if w.snake { c(1.0, 0.15, 0.1) } else { ink };
    if sk.face == Face::Hurt {
        for gx in [ex, ex - 2.4] {
            cx.line((gx - 0.9, ey - 0.6), (gx + 0.9, ey + 0.6), 0.8, eye);
            cx.line((gx - 0.9, ey + 0.6), (gx + 0.9, ey - 0.6), 0.8, eye);
        }
    } else if w.glasses == 1 {
        for gx in [ex, ex - 2.6] {
            cx.circ((gx, ey), 1.9, ink);
            cx.circ((gx, ey), 1.35, c(0.85, 0.95, 1.0));
            cx.circ((gx + 0.25, ey), 0.7, ink);
        }
    } else {
        for gx in [ex, ex - 2.4] {
            cx.circ((gx, ey), if w.snake { 1.2 } else { 0.95 }, eye);
            if w.glasses == 2 {
                cx.line((gx - 1.5, ey + 1.5), (gx + 1.5, ey + 1.5), 0.7, gold);
            }
        }
    }
    if w.snake {
        // slits where the nose should be
        cx.circ((hc.0 + hr * 0.8, hc.1 + hr * 0.45), 0.4, ink);
        cx.circ((hc.0 + hr * 0.8, hc.1 + hr * 0.7), 0.4, ink);
        cx.line((ex - 0.8, ey + 3.4), (ex + 1.2, ey + 3.2), 0.6, c(0.45, 0.2, 0.25));
    } else if matches!(sk.face, Face::Shout | Face::Grin) {
        cx.circ((ex + 0.2, ey + 2.6), 1.0, ink);
    }
    if w.scar {
        cx.line((ex + 0.1, ey - 3.9), (ex + 0.9, ey - 3.0), 0.6, c(0.85, 0.15, 0.15));
        cx.line((ex + 0.9, ey - 3.0), (ex + 0.3, ey - 2.6), 0.6, c(0.85, 0.15, 0.15));
    }
    if let Some(h) = w.hat {
        let y = hc.1 - hr * 0.55;
        cx.poly(
            &[(hc.0 - hr * 1.1, y), (hc.0 + hr * 1.1, y), (hc.0 - hr * 1.0, hc.1 - hr * 3.1)],
            h,
        );
        cx.line((hc.0 - hr * 1.6, y), (hc.0 + hr * 1.6, y), 1.6, h);
        cx.line((hc.0 - hr * 1.05, y - 0.6), (hc.0 + hr * 1.05, y - 0.6), 1.0, w.trim);
    }
    // house scarf: wrapped round the neck with a tail down the chest
    if let Some((s1, s2)) = w.scarf {
        for k in 0..3 {
            let x = j.sh.0 - 2.4 + k as f32 * 1.7;
            cx.line((x, j.sh.1 - 1.2), (x + 0.4, j.sh.1 + 1.6), 1.7, if k % 2 == 0 { s1 } else { s2 });
        }
        for k in 0..4 {
            let y = j.sh.1 + 2.6 + k as f32 * 2.0;
            cx.line((j.sh.0 + 0.2, y), (j.sh.0 + 2.8, y), 1.7, if k % 2 == 0 { s1 } else { s2 });
        }
    }
    // front arm and hand
    cx.limb(j.sh, j.e_f, sp.ath, w.robe);
    cx.limb(j.e_f, j.h_f, sp.ath - 0.2, mix(w.robe, w.trim, 0.2));
    cx.circ(j.h_f, 1.9, w.skin);
    let hf = j.h_f;
    let lit = d.charge > 0.0 || d.act == Act::Cast;
    if w.staff {
        let top = (hf.0 + 0.8, hf.1 - 11.0);
        cx.line((hf.0 + 0.8, 0.0), top, 1.4, c(0.45, 0.3, 0.18));
        cx.circ(top, 2.0, w.glow);
        if lit {
            cx.circ(top, 4.2, ca(w.glow.0, w.glow.1, w.glow.2, 0.4));
        }
    } else {
        let a = sk.weap;
        let tip = (hf.0 + a.sin() * 9.0, hf.1 + a.cos() * 9.0);
        cx.line(hf, tip, 1.1, w.wand);
        if lit {
            cx.circ(tip, 2.0 + 2.5 * d.charge, ca(w.glow.0, w.glow.1, w.glow.2, 0.45));
            cx.circ(tip, 1.1, c(1.0, 1.0, 1.0));
        }
    }
}

fn veil(cx: &Ctx, d: &Draw, sk: &Sk, sp: &Spec, j: &J) {
    let t = d.t;
    let cl = c(0.15, 0.33, 0.45);
    let cd = c(0.09, 0.20, 0.29);
    let cle = c(0.35, 0.70, 0.85);
    let mask = c(0.94, 0.94, 0.97);
    let amb = c(1.0, 0.72, 0.15);
    let scarf = c(0.86, 0.20, 0.30);
    let blade = c(0.88, 0.96, 1.0);
    let ink = c(0.07, 0.08, 0.12);
    let (hc, hr, sh, hip) = (j.head, j.hr, j.sh, j.hip);
    let spd = if matches!(d.act, Act::Run) { 1.0 } else if d.act == Act::Walk { 0.35 } else { 0.15 };
    // cloak behind
    let base = (sh.0 - 0.5, sh.1 + 0.5);
    let n = 4;
    let mut tips: Vec<V> = Vec::new();
    for k in 0..n {
        let kf = k as f32;
        let u = kf / (n as f32 - 1.0);
        let ex = -6.0 - spd * 13.0 - u * 2.0;
        let ey = hip.1 + (u * 9.0 + if k == n - 1 { 3.0 } else { 0.0 }) + 2.0;
        tips.push((base.0 + ex * (1.0 + 0.2 * (t * 10.0 + kf).sin()), ey + (t * 9.0 + kf * 1.7).sin() * 1.2));
    }
    let mut pts: Vec<V> = vec![(sh.0 + 0.8, sh.1 - 0.5), base];
    pts.extend(tips.iter().copied());
    pts.push((hip.0 - 1.0, hip.1 + 8.0));
    pts.push((hip.0 + 0.5, hip.1 + 6.0));
    cx.poly(&pts, cd);
    cx.poly(&[(sh.0 + 0.8, sh.1 - 0.5), base, tips[0], tips[1], (hip.0 - 0.5, hip.1 + 3.0)], cl);
    // scarf
    let nk = (sh.0 + 0.3, sh.1 - 1.2);
    let scp = [
        nk,
        (nk.0 - 3.0, nk.1 + 0.5 + (t * 8.0).sin() * 0.6),
        (nk.0 - 7.0 - spd * 8.0, nk.1 + 1.5 + (t * 8.0 + 1.0).sin() * 1.4),
        (nk.0 - 11.0 - spd * 12.0, nk.1 + 0.5 + (t * 8.0 + 2.0).sin() * 2.0),
    ];
    for i in 0..3 {
        cx.line(scp[i], scp[i + 1], 2.0 - i as f32 * 0.4, scarf);
    }
    // back limbs + dagger
    cx.limb(j.sh, j.e_b, sp.ath, dim(cd, 0.9));
    cx.limb(j.e_b, j.h_b, sp.ath - 0.2, dim(cd, 0.9));
    let wb = sk.weap - 0.9;
    let hb = j.h_b;
    cx.line(hb, (hb.0 + wb.sin() * 8.5, hb.1 + wb.cos() * 8.5), 1.2, dim(blade, 0.75));
    cx.circ(hb, 1.5, ink);
    cx.limb(j.hip, j.k_b, sp.th, dim(cd, 0.9));
    cx.limb(j.k_b, j.f_b, sp.th - 0.1, dim(cd, 0.9));
    cx.circ(j.f_b, 2.2, ink);
    cx.circ((j.f_b.0 + 2.8, j.f_b.1 + 0.5), 1.7, ink);
    // torso
    cx.poly(&torso_quad(j, sp.hw, sp.sw), cd);
    let q = torso_quad(j, sp.hw, sp.sw);
    cx.line(
        ((q[3].0 + q[0].0) / 2.0 + 0.5, (q[3].1 + q[0].1) / 2.0),
        ((q[2].0 + q[1].0) / 2.0 + 0.5, (q[2].1 + q[1].1) / 2.0),
        0.9,
        cle,
    );
    cx.line((hip.0 - sp.hw, hip.1 - 0.3), (hip.0 + sp.hw, hip.1 + 0.3), 1.5, scarf);
    // front leg
    cx.limb(j.hip, j.k_f, sp.th, cl);
    cx.limb(j.k_f, j.f_f, sp.th - 0.1, cd);
    cx.circ(j.f_f, 2.4, ink);
    cx.circ((j.f_f.0 + 3.0, j.f_f.1 + 0.5), 1.8, ink);
    cx.line((j.f_f.0 - 1.0, j.f_f.1 - 2.6), (j.f_f.0 + 1.2, j.f_f.1 - 2.6), 0.8, cle);
    // hood + face
    cx.poly(
        &[
            (hc.0 - hr * 0.2, hc.1 - hr * 1.0),
            (hc.0 - hr * 1.9 - 1.0, hc.1 - hr * 0.25 + (t * 6.0).sin() * 0.5),
            (hc.0 - hr * 0.9, hc.1 + hr * 0.9),
        ],
        cl,
    );
    cx.circ(hc, hr * 1.12, cl);
    cx.circ((hc.0 + hr * 0.35, hc.1 + hr * 0.15), hr * 0.78, cd);
    cx.poly(
        &[
            (hc.0 + hr * 0.15, hc.1 - hr * 0.3),
            (hc.0 + hr * 1.0, hc.1 - hr * 0.15),
            (hc.0 + hr * 0.95, hc.1 + hr * 0.85),
            (hc.0 + hr * 0.1, hc.1 + hr * 0.75),
        ],
        mask,
    );
    let ev = if sk.face != Face::Hurt { amb } else { c(0.7, 0.5, 0.2) };
    cx.circ((hc.0 + hr * 0.62, hc.1 + hr * 0.05), 2.0, ca(1.0, 0.72, 0.15, 0.35));
    cx.circ((hc.0 + hr * 0.62, hc.1 + hr * 0.05), 1.15, ev);
    cx.line((hc.0 + hr * 0.2, hc.1 - hr * 0.22), (hc.0 + hr * 0.95, hc.1 - hr * 0.05), 0.8, ink);
    cx.line((hc.0 + hr * 0.35, hc.1 + hr * 0.55), (hc.0 + hr * 0.9, hc.1 + hr * 0.55), 0.7, ink);
    // front arm + dagger
    cx.limb(j.sh, j.e_f, sp.ath, cl);
    cx.limb(j.e_f, j.h_f, sp.ath - 0.2, cd);
    let w = sk.weap;
    let hf = j.h_f;
    cx.line(hf, (hf.0 + w.sin() * 9.5, hf.1 + w.cos() * 9.5), 1.4, blade);
    cx.circ(hf, 1.6, ink);
    let gx = (w + 1.57).sin() * 1.6;
    let gy = (w + 1.57).cos() * 1.6;
    cx.line((hf.0 - gx, hf.1 - gy), (hf.0 + gx, hf.1 + gy), 0.9, cle);
    let _ = j.fa_f;
}

// ------------------------------------------------------------------ anchors --

/// Where things are on an actor, for aiming effects: (hand, chest, head) in
/// screen coordinates.
pub fn anchors(d: &Draw) -> (V, V, V) {
    if d.who.pixel_id().is_some() {
        let hand = if matches!(d.act, Act::Cast | Act::Throw) { (11.0, -15.0) } else { (8.0, -14.0) };
        let m = |v: V| (d.x + v.0 * d.dir, d.y + v.1);
        return (m(hand), m((2.0, -16.0)), m((0.0, -22.0)));
    }
    let sk = pose(d.act, d.p, d.t, d.who);
    let sp = spec(d.who);
    let j = solve(&sk, &sp);
    let m = |v: V| (d.x + v.0 * d.dir * K, d.y + v.1 * K);
    let chest = ((j.hip.0 + j.sh.0) / 2.0, (j.hip.1 + j.sh.1) / 2.0);
    (m(j.h_f), m(chest), m(j.head))
}

/// Visual height of the actor (for speech bubbles).
#[allow(dead_code)]
pub fn height(who: Who) -> f32 {
    if who.pixel_id().is_some() {
        return 29.0;
    }
    (spec(who).h + 6.0) * K
}

// -------------------------------------------------------------- pixel art ----

fn paint(g: &Gfx, layer: &crate::pixdata::Layer, id: u8, x: f32, y: f32, dir: f32, flash: f32) {
    for (row, line) in layer.iter().enumerate() {
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let ch = b[i] as char;
            let Some(col) = crate::pixdata::pal(id, ch) else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            while j < b.len() && b[j] == b[i] {
                j += 1;
            }
            let col = if flash > 0.0 { mix(col, Color(1.0, 1.0, 1.0, 1.0), flash) } else { col };
            let (c0, n) = (i as f32 - 9.0, (j - i) as f32);
            let px = if dir >= 0.0 { x + c0 } else { x - c0 - n };
            g.fill_rrect(Rect::new(px, y + row as f32, n, 1.0), 0.0, col);
            i = j;
        }
    }
}

fn draw_pixel(g: &Gfx, d: &Draw, id: u8) {
    let ph = (d.t * 7.0) as usize;
    let (mut legs, mut arms, mut dy, mut dx, mut flash) = (0usize, 0usize, 0.0f32, 0.0f32, 0.0f32);
    match d.act {
        Act::Stand => {
            dy = if id == 1 { (d.t * 3.0).sin().round() * -1.0 } else { 0.0 };
        }
        Act::Walk => {
            legs = [1, 0, 3, 0][ph % 4];
            if id == 1 {
                legs = [0, 1, 0, 2][ph % 4];
            }
        }
        Act::Run => {
            let q = (d.t * 11.0) as usize;
            legs = [1, 2, 3, 2][q % 4];
            if id == 1 {
                legs = [1, 0, 2, 0][q % 4];
            }
            dy = if q % 2 == 0 { 0.0 } else { -1.0 };
        }
        Act::Slash | Act::Slam | Act::Punch => {
            arms = if d.p < 0.35 { 1 } else if d.p < 0.75 { 2 } else { 0 };
            dx = if d.p >= 0.35 && d.p < 0.75 { 2.0 } else { 0.0 };
        }
        Act::Cast | Act::Throw => arms = 3,
        Act::Block => arms = 4,
        Act::Cheer => {
            arms = 5;
            dy = -((d.t * 9.0).sin().abs() * 3.0).round();
        }
        Act::Hurt => {
            arms = 6;
            dx = -3.0;
            if d.p < 0.3 {
                flash = 0.65;
            }
        }
        Act::Jump | Act::Fly => {
            legs = 4;
            arms = 5;
        }
        Act::Kneel | Act::Crouch => {
            legs = 5;
            dy = 3.0;
            arms = if d.act == Act::Kneel { 6 } else { 0 };
            dx = -1.0;
        }
    }
    let (x, y) = (d.x, d.y - 27.0);
    // shadow
    g.fill_rrect(Rect::new(d.x - 7.0, d.y, 14.0, 2.0), 1.0, ca(0.0, 0.0, 0.0, 0.3));
    if id == 0 && matches!(d.act, Act::Cast | Act::Throw) {
        // nothing extra: the ronin points his blade
    }
    let bx = x + dx * d.dir;
    let (ln, bd, am) = match id {
        0 => (&crate::pixdata::RONIN_LEGS, &crate::pixdata::RONIN_BODY, &crate::pixdata::RONIN_ARMS),
        1 => (&crate::pixdata::COURIER_LEGS, &crate::pixdata::COURIER_BODY, &crate::pixdata::COURIER_ARMS),
        _ => (&crate::pixdata::NECRO_LEGS, &crate::pixdata::NECRO_BODY, &crate::pixdata::NECRO_ARMS),
    };
    let ly = if matches!(d.act, Act::Kneel | Act::Crouch) { 0.0 } else { dy.min(0.0) };
    paint(g, &ln[legs], id, bx, y + ly, d.dir, flash);
    paint(g, &bd, id, bx, y + dy, d.dir, flash);
    paint(g, &am[arms], id, bx, y + dy, d.dir, flash);
    // glow while casting
    if matches!(d.act, Act::Cast) && d.charge > 0.0 {
        let hand = anchors(d).0;
        let col = match id {
            1 => ca(0.2, 0.9, 1.0, 0.35 * d.charge),
            2 => ca(0.36, 1.0, 0.48, 0.4 * d.charge),
            _ => ca(1.0, 0.8, 0.4, 0.3 * d.charge),
        };
        g.fill_circle(hand.0 + d.dir * 5.0, hand.1, 5.0, col);
    }
}
