//! A tiny Flappy-style game for the notch panel: click to flap, dodge the pipes.
//! The "bird" is a drawn buddy. Plain physics, stepped from a 60 Hz
//! timer that only runs while the Game tab is open.

use crate::gfx::*;

/// Playfield size (logical px).
pub const W: f32 = 544.0;
pub const H: f32 = 180.0;

const BIRD_X: f32 = 110.0;
const R: f32 = 9.0;
const GRAVITY: f32 = 900.0;
const FLAP: f32 = -290.0;
const SPEED: f32 = 150.0;
const GAP: f32 = 78.0;
const PIPE_W: f32 = 34.0;
const SPACING: f32 = 190.0;

#[derive(Clone, Copy, PartialEq)]
pub enum State {
    Ready,
    Playing,
    Dead,
}

struct Pipe {
    x: f32,
    gap_y: f32,
    passed: bool,
}

pub struct Game {
    pub state: State,
    y: f32,
    vy: f32,
    pipes: Vec<Pipe>,
    pub score: u32,
    pub best: u32,
    t: f32,
    rng: u32,
}

impl Game {
    pub fn new(best: u32) -> Game {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(5);
        Game { state: State::Ready, y: H / 2.0, vy: 0.0, pipes: Vec::new(), score: 0, best, t: 0.0, rng: nanos | 1 }
    }

    fn rnd(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng % 10_000) as f32 / 10_000.0
    }

    /// Click: start, flap, or restart after a crash.
    pub fn flap(&mut self) {
        match self.state {
            State::Ready => {
                self.state = State::Playing;
                self.vy = FLAP;
            }
            State::Playing => self.vy = FLAP,
            State::Dead => {
                let best = self.best;
                *self = Game { rng: self.rng, ..Game::new(best) };
            }
        }
    }

    /// Advance by `dt` seconds. Returns true if a new best score was just set.
    pub fn step(&mut self, dt: f32) -> bool {
        self.t += dt;
        match self.state {
            State::Ready => {
                self.y = H / 2.0 + (self.t * 4.0).sin() * 6.0;
                return false;
            }
            State::Dead => return false,
            State::Playing => {}
        }
        self.vy += GRAVITY * dt;
        self.y += self.vy * dt;
        for p in &mut self.pipes {
            p.x -= SPEED * dt;
            if !p.passed && p.x + PIPE_W < BIRD_X - R {
                p.passed = true;
                self.score += 1;
            }
        }
        self.pipes.retain(|p| p.x > -PIPE_W - 10.0);
        if self.pipes.last().map(|p| p.x < W - SPACING).unwrap_or(true) {
            let gap_y = 54.0 + self.rnd() * (H - 108.0);
            self.pipes.push(Pipe { x: W + 20.0, gap_y, passed: false });
        }
        let hit_pipe = self.pipes.iter().any(|p| {
            BIRD_X + R > p.x && BIRD_X - R < p.x + PIPE_W && (self.y - R < p.gap_y - GAP / 2.0 || self.y + R > p.gap_y + GAP / 2.0)
        });
        if hit_pipe || self.y < R || self.y > H - R {
            self.state = State::Dead;
            if self.score > self.best {
                self.best = self.score;
                return true;
            }
        }
        false
    }

    /// Draw the playfield with its top-left corner at (ox, oy).
    pub fn draw(&self, g: &Gfx, ox: f32, oy: f32, ca: f32) {
        let f = &g.fonts;
        let field = Rect::new(ox, oy, W, H);
        g.fill_rrect(field, 14.0, Color(0.1, 0.12, 0.2, 0.9 * ca));
        let pipe = Color(0.36, 0.78, 0.45, ca);
        let cap = Color(0.28, 0.62, 0.36, ca);
        for p in &self.pipes {
            let x = ox + p.x;
            let (top, bot) = (p.gap_y - GAP / 2.0, p.gap_y + GAP / 2.0);
            // clip to the field: pipes slide in from the right edge
            let (x0, x1) = (x.max(ox), (x + PIPE_W).min(ox + W));
            if x1 <= x0 {
                continue;
            }
            g.fill_rrect(Rect::new(x0, oy, x1 - x0, top), 0.0, pipe);
            g.fill_rrect(Rect::new(x0, oy + bot, x1 - x0, H - bot), 0.0, pipe);
            g.fill_rrect(Rect::new(x0, oy + top - 6.0, x1 - x0, 6.0), 2.0, cap);
            g.fill_rrect(Rect::new(x0, oy + bot, x1 - x0, 6.0), 2.0, cap);
        }
        // bird
        let (bx, by) = (ox + BIRD_X, oy + self.y);
        let tilt = (self.vy / 400.0).clamp(-0.6, 1.0) * 35.0;
        g.rotated(bx, by, tilt * 0.8, || crate::skins::draw(g, bx, by, self.t));
        g.text(
            &self.score.to_string(),
            Rect::new(ox + 18.0, oy + 8.0, 80.0, 36.0),
            &f.big,
            Color(1.0, 1.0, 1.0, ca),
            Align::Left,
        );
        g.text(
            &format!("best {}", self.best),
            Rect::new(ox + W - 110.0, oy + 10.0, 96.0, 20.0),
            &f.small,
            Color(1.0, 1.0, 1.0, 0.6 * ca),
            Align::Right,
        );
        let msg = match self.state {
            State::Ready => Some("Click to flap"),
            State::Dead => Some("Game over \u{00B7} click to retry"),
            State::Playing => None,
        };
        if let Some(m) = msg {
            g.text(m, Rect::new(ox, oy + H - 34.0, W, 24.0), &f.body, Color(1.0, 1.0, 1.0, 0.85 * ca), Align::Center);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flap_lifts_and_gravity_kills() {
        let mut g = Game::new(0);
        g.flap();
        let y0 = g.y;
        g.step(0.05);
        assert!(g.y < y0, "a flap should move the bird up");
        for _ in 0..200 {
            g.step(0.016);
        }
        assert!(g.state == State::Dead, "without flapping the bird hits the floor");
        g.flap();
        assert!(g.state == State::Ready && g.score == 0, "click after a crash restarts");
    }
}
