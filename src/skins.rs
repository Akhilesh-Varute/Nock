//! The notch games' heroes: Thor (running) and a caped flyer (flight), drawn by the
//! same skeleton cast as the scenes (see cast.rs), so they get the same outline,
//! cape and proper poses.

use crate::cast::{self, Act, Draw, Who};
use crate::gfx::*;

/// Size of the hero in the games relative to the top-bar scenes.
const K: f32 = 1.1;
/// ...and on the hub tiles, where there is room for a closer look.
pub const HUB_K: f32 = 1.55;

fn hero(g: &Gfx, who: Who, x: f32, y: f32, t: f32, act: Act, p: f32, k: f32) {
    cast::draw_actor_k(g, &Draw { who, x, y, dir: 1.0, act, p, t, charge: 0.0 }, k);
}

/// The flyer, fist forward, flying right, centred on (x, y).
pub fn draw(g: &Gfx, x: f32, y: f32, t: f32) {
    draw_k(g, x, y, t, K * 1.25); // lies flat, so he can be a bit bigger
}

pub fn draw_k(g: &Gfx, x: f32, y: f32, t: f32, k: f32) {
    hero(g, Who::Superman, x - 2.0, y + 2.0, t, Act::Fly, 0.0, k);
}

/// Thor running upright, facing right, feet at (x, y); jumps with his arms up.
pub fn draw_run(g: &Gfx, x: f32, y: f32, t: f32, air: bool, _vy: f32) {
    draw_run_k(g, x, y, t, air, K);
}

pub fn draw_run_k(g: &Gfx, x: f32, y: f32, t: f32, air: bool, k: f32) {
    if air {
        hero(g, Who::Thor, x, y - 2.0, t, Act::Jump, 1.0, k);
    } else {
        hero(g, Who::Thor, x, y, t, Act::Run, 0.0, k);
    }
}
