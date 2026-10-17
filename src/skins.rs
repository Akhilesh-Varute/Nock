//! The notch games' fan-art character: a horned, bearded Thor with a war hammer,
//! drawn from simple shapes (no image files). Flying faces right, centred on (x, y).

use crate::gfx::*;

fn c(r: f32, g: f32, b: f32) -> Color {
    Color(r, g, b, 1.0)
}

pub fn draw(g: &Gfx, x: f32, y: f32, t: f32) {
    thor(g, x, y, t);
}

/// Eyes shared by the humanoids: a dot at the front of the head.
fn eye(g: &Gfx, x: f32, y: f32) {
    g.fill_circle(x, y, 0.9, c(0.1, 0.1, 0.14));
}

fn thor(g: &Gfx, x: f32, y: f32, t: f32) {
    let fl = (t * 16.0).sin() * 1.2;
    let skin = c(0.96, 0.78, 0.62);
    let beard = c(0.82, 0.50, 0.20);
    let fur = c(0.45, 0.30, 0.18);
    let iron = c(0.62, 0.64, 0.70);
    // legs
    g.line((x - 6.0, y + 2.0), (x - 16.0, y + 3.0 + fl * 0.3), 3.4, c(0.30, 0.22, 0.16));
    g.line((x - 6.0, y + 4.0), (x - 16.0, y + 6.5 - fl * 0.3), 3.4, c(0.30, 0.22, 0.16));
    // tunic with fur collar and belt
    g.fill_rrect(Rect::new(x - 8.0, y - 5.5, 15.0, 10.5), 3.5, c(0.20, 0.34, 0.55));
    g.fill_rrect(Rect::new(x - 8.0, y - 6.5, 15.0, 3.5), 1.8, fur);
    g.line((x - 8.0, y + 2.5), (x + 7.0, y + 2.5), 1.6, c(0.25, 0.17, 0.10));
    // sword trailing behind
    g.line((x - 4.0, y - 6.0), (x - 15.0, y - 9.0 + fl * 0.5), 1.6, iron);
    g.line((x - 4.0, y - 6.0), (x - 6.0, y - 4.0), 2.2, c(0.45, 0.30, 0.18));
    // head: horned helmet, big beard
    g.fill_circle(x + 9.5, y - 5.0, 4.4, skin);
    g.fill_poly(&[(x + 6.0, y - 3.0), (x + 13.5, y - 3.5), (x + 12.0, y + 3.5), (x + 8.0, y + 2.5)], beard);
    g.fill_circle(x + 9.0, y - 7.0, 4.0, iron);
    g.fill_poly(&[(x + 5.5, y - 8.0), (x + 3.5, y - 12.5), (x + 7.0, y - 9.5)], c(0.92, 0.90, 0.80));
    g.fill_poly(&[(x + 12.0, y - 8.5), (x + 14.5, y - 12.5), (x + 11.0, y - 9.5)], c(0.92, 0.90, 0.80));
    eye(g, x + 11.5, y - 5.2);
    // war hammer out front
    g.line((x + 4.0, y - 2.0), (x + 15.0, y - 3.0), 3.4, skin);
    g.line((x + 14.0, y - 3.0), (x + 22.0, y - 3.0), 1.5, c(0.45, 0.30, 0.18));
    g.fill_rrect(Rect::new(x + 20.0, y - 9.5, 8.0, 12.0), 1.5, iron);
    g.fill_rrect(Rect::new(x + 20.0, y - 5.5, 8.0, 2.4), 0.5, c(0.40, 0.42, 0.50));
}

/// Thor running upright, facing right, feet at (x, y); tucks his legs in the air.
pub fn draw_run(g: &Gfx, x: f32, y: f32, t: f32, air: bool, vy: f32) {
    let k = 0.82; // overall scale
    let p = |dx: f32, dy: f32| (x + dx * k, y + dy * k);
    let skin = c(0.96, 0.78, 0.62);
    let beard = c(0.82, 0.50, 0.20);
    let fur = c(0.45, 0.30, 0.18);
    let iron = c(0.62, 0.64, 0.70);
    let swing = (t * 15.0).sin();
    // legs
    let hip = p(0.0, -13.0);
    if air {
        let tuck = if vy < 0.0 { 5.0 } else { 2.0 };
        g.line(hip, p(-6.0, -5.0 - tuck * 0.3), 3.6 * k, c(0.30, 0.22, 0.16));
        g.line(hip, p(7.0, -7.0 + tuck * 0.2), 3.6 * k, c(0.30, 0.22, 0.16));
    } else {
        g.line(hip, p(swing * 8.0, 0.0), 3.6 * k, c(0.30, 0.22, 0.16));
        g.line(hip, p(-swing * 8.0, 0.0), 3.6 * k, c(0.30, 0.22, 0.16));
    }
    // sword on his back
    g.line(p(-3.0, -26.0), p(-11.0, -36.0), 1.8 * k, iron);
    // tunic, fur collar, belt
    let (tx, ty) = p(-6.5, -27.0);
    g.fill_rrect(Rect::new(tx, ty, 13.0 * k, 15.0 * k), 3.5 * k, c(0.20, 0.34, 0.55));
    let (fx, fy) = p(-7.5, -29.0);
    g.fill_rrect(Rect::new(fx, fy, 15.0 * k, 5.0 * k), 2.5 * k, fur);
    g.line(p(-6.5, -16.0), p(6.5, -16.0), 1.8 * k, c(0.25, 0.17, 0.10));
    // head: beard, face, horned helmet
    g.fill_circle(p(1.5, -34.0).0, p(1.5, -34.0).1, 5.2 * k, skin);
    g.fill_poly(&[p(-2.0, -33.0), p(6.0, -33.0), p(4.5, -25.5), p(0.0, -26.5)], beard);
    g.fill_circle(p(1.0, -37.0).0, p(1.0, -37.0).1, 4.8 * k, iron);
    g.fill_poly(&[p(-3.0, -38.0), p(-6.0, -44.0), p(-0.5, -40.0)], c(0.92, 0.90, 0.80));
    g.fill_poly(&[p(5.0, -38.5), p(8.5, -44.0), p(2.5, -40.0)], c(0.92, 0.90, 0.80));
    g.fill_circle(p(4.2, -34.5).0, p(4.2, -34.5).1, 0.9, c(0.1, 0.1, 0.14));
    // arm and the war hammer swinging over the shoulder
    let bob = swing * 2.0;
    let hand = p(8.0, -21.0 + bob);
    g.line(p(2.0, -23.0), hand, 3.2 * k, skin);
    let top = p(14.0, -35.0 + bob);
    g.line(hand, top, 1.8 * k, c(0.45, 0.30, 0.18));
    let (hx, hy) = p(10.0, -41.0 + bob);
    g.fill_rrect(Rect::new(hx, hy, 11.0 * k, 9.0 * k), 1.5, iron);
    g.fill_rrect(Rect::new(hx, hy + 3.2 * k, 11.0 * k, 2.4 * k), 0.5, c(0.40, 0.42, 0.50));
}
