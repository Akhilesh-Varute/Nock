//! Linux entry point (work in progress): a borderless X11 winit window that draws
//! the notch pill and, on click, the tabbed panel through the shared `gfx` API.
//! Pages are placeholders except Today and Stats; they move over from `ui.rs`
//! as `App` sheds its Windows types.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::platform::x11::{EventLoopBuilderExtX11, WindowAttributesExtX11, WindowType};
use winit::window::{Window, WindowId, WindowLevel};

use crate::cal::{days_in_month, first_weekday, fmt_time};
use crate::gfx::*;
use crate::store::{self, AppEntry, Todo};
use crate::sys;

const PILL_W: f32 = 176.0;
const PILL_H: f32 = 34.0;
const EXP_W: f32 = 600.0;
const EXP_H: f32 = 250.0;
const CONTENT_Y: f32 = 40.0;
const MAX_TODO: usize = 5;
const PICK_ROWS: usize = 5;

const T1: Color = Color::white(0.96);
const T2: Color = Color::white(0.62);
const T3: Color = Color::white(0.38);
const SURF: Color = Color::white(0.09);
const HOVER: Color = Color::white(0.17);
const ACCENT: Color = Color(0.40, 0.60, 1.0, 1.0);
const C_CPU: Color = Color(0.30, 0.78, 0.98, 1.0);
const C_MEM: Color = Color(0.98, 0.70, 0.20, 1.0);
const C_BAT: Color = Color(0.40, 0.85, 0.50, 1.0);
const C_UP: Color = Color(0.72, 0.58, 1.0, 1.0);
const C_FOC: Color = Color(1.0, 0.58, 0.42, 1.0);

const TAB_LABELS: [&str; 9] =
    ["Media", "Controls", "Today", "Stats", "Clipboard", "Shelf", "Apps", "Games", "Power"];
const CONTROLS: usize = 1;
const APPS: usize = 6;
const TODAY: usize = 2;
const STATS: usize = 3;
const POWER: usize = 8;

/// Where each tab sits in the top row (same layout rule as the Windows panel).
fn tab_rects() -> Vec<Rect> {
    let text_w = |s: &str| -> f32 {
        s.chars()
            .map(|c| match c {
                'i' | 'l' | 't' | 'f' | 'j' | 'r' | 'I' => 3.4,
                'm' | 'w' | 'M' | 'W' => 9.6,
                _ => 6.7,
            })
            .sum()
    };
    let widths: Vec<f32> = TAB_LABELS.iter().map(|l| text_w(l) + 16.0).collect();
    let total: f32 = widths.iter().sum();
    let (left, right) = (46.0, EXP_W - 46.0);
    let n = TAB_LABELS.len() as f32 - 1.0;
    let gap = ((right - left - total) / n).clamp(2.0, 12.0);
    let used = total + gap * n;
    let mut x = left + (right - left - used) / 2.0;
    widths
        .iter()
        .map(|w| {
            let r = Rect::new(x, 8.0, *w, 28.0);
            x += w + gap;
            r
        })
        .collect()
}

/// (used fraction, total MiB, used MiB) from /proc/meminfo.
fn mem_info() -> Option<(f32, u64, u64)> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = |key: &str| -> Option<u64> {
        s.lines().find(|l| l.starts_with(key))?.split_whitespace().nth(1)?.parse().ok()
    };
    let (total, avail) = (kb("MemTotal:")?, kb("MemAvailable:")?);
    let used = total.saturating_sub(avail);
    Some((used as f32 / total as f32, total / 1024, used / 1024))
}

fn uptime() -> String {
    let secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0) as u64;
    format!("{}h {:02}m", secs / 3600, secs / 60 % 60)
}

// ------------------------------------------------------------------ state ----

#[derive(Clone, Copy, PartialEq, Debug)]
enum Act {
    Tab(usize),
    Collapse,
    TimerPreset(u32),
    TimerCustom,
    TimerToggle,
    TimerReset,
    TodoToggle(usize),
    TodoDel(usize),
    TodoFocus(usize),
    TodoAdd,
    Launch(usize),
    AppUnpin(usize),
    AppAdd,
    AppDone,
    AppSearch,
    AppRefresh,
    AppPick(usize),
    Screenshot,
    Settings(u8),
    Lock,
    Sleep,
    Restart,
    Shutdown,
}

struct Hit {
    r: Rect,
    act: Act,
}

/// Everything the panel shows; no window types, so it is testable and portable.
struct Panel {
    page: usize,
    hover: Option<Act>,
    timer_total_s: u32,
    /// time left when not running
    rem_ms: i64,
    run_since: Option<Instant>,
    timer_task: Option<String>,
    todos: Vec<Todo>,
    focus_today: u32,
    done_today: u32,
    /// text being typed, and whether it is for the timer length (else a new to-do)
    entering: Option<String>,
    entry_timer: bool,
    /// restart / shut down wait for a second click: (which, when armed)
    power_confirm: Option<(u8, Instant)>,
    /// Apps tab: pinned apps, and the picker (open?, installed apps, search text, scroll)
    apps: Vec<AppEntry>,
    picking: bool,
    app_list: Vec<(String, String)>,
    app_filter: String,
    entry_search: bool,
    app_scroll: usize,
}

impl Panel {
    fn new() -> Panel {
        let (focus_today, done_today) = store::load_focus_today();
        Panel {
            page: TODAY,
            hover: None,
            timer_total_s: 25 * 60,
            rem_ms: 25 * 60 * 1000,
            run_since: None,
            timer_task: None,
            todos: store::load_todos(),
            focus_today,
            done_today,
            entering: None,
            entry_timer: false,
            power_confirm: None,
            apps: store::load_apps(),
            picking: false,
            app_list: Vec::new(),
            app_filter: String::new(),
            entry_search: false,
            app_scroll: 0,
        }
    }

    /// Installed apps matching the search text.
    fn app_view(&self) -> Vec<&(String, String)> {
        let q = self.app_filter.to_lowercase();
        self.app_list.iter().filter(|(n, _)| q.is_empty() || n.to_lowercase().contains(&q)).collect()
    }

    fn scroll(&mut self, rows: isize) -> bool {
        if self.page != APPS || !self.picking {
            return false;
        }
        let max = self.app_view().len().saturating_sub(PICK_ROWS);
        self.app_scroll = (self.app_scroll as isize + rows).clamp(0, max as isize) as usize;
        true
    }

    fn armed(&self, k: u8) -> bool {
        matches!(self.power_confirm, Some((p, t)) if p == k && t.elapsed().as_secs_f32() < 3.5)
    }

    /// Restart / shut down only on a second click within a few seconds.
    fn power_press(&mut self, k: u8) {
        if self.armed(k) {
            self.power_confirm = None;
            if k == 1 { sys::restart_pc() } else { sys::shutdown_pc() }
        } else {
            self.power_confirm = Some((k, Instant::now()));
        }
    }

    /// Something on screen changes by itself (running timer, expiring confirmation).
    fn animating(&self) -> bool {
        self.running() || self.power_confirm.is_some()
    }

    fn running(&self) -> bool {
        self.run_since.is_some()
    }

    fn left_ms(&self) -> i64 {
        match self.run_since {
            Some(t) => (self.rem_ms - t.elapsed().as_millis() as i64).max(0),
            None => self.rem_ms,
        }
    }

    fn set_timer(&mut self, mins: u32) {
        self.timer_total_s = mins * 60;
        self.rem_ms = mins as i64 * 60_000;
        self.run_since = None;
    }

    fn toggle_timer(&mut self) {
        if self.running() {
            self.rem_ms = self.left_ms();
            self.run_since = None;
        } else {
            if self.rem_ms <= 0 {
                self.rem_ms = self.timer_total_s as i64 * 1000;
            }
            self.run_since = Some(Instant::now());
        }
    }

    /// Credit a finished session once; returns true when it just ended.
    fn tick(&mut self) -> bool {
        if matches!(self.power_confirm, Some((_, t)) if t.elapsed().as_secs_f32() >= 3.5) {
            self.power_confirm = None;
        }
        if self.running() && self.left_ms() == 0 {
            self.rem_ms = 0;
            self.run_since = None;
            self.focus_today += self.timer_total_s / 60;
            store::save_focus_today(self.focus_today, self.done_today);
            return true;
        }
        false
    }

    /// Returns true when the window should collapse.
    fn act(&mut self, act: Act) -> bool {
        match act {
            Act::Tab(i) => self.page = i,
            Act::Collapse => return true,
            Act::TimerPreset(m) => self.set_timer(m),
            Act::TimerCustom => {
                self.entering = Some(String::new());
                self.entry_timer = true;
            }
            Act::TimerToggle => self.toggle_timer(),
            Act::TimerReset => {
                let m = self.timer_total_s / 60;
                self.set_timer(m);
                self.timer_task = None;
            }
            Act::TodoToggle(i) => {
                if let Some(t) = self.todos.get_mut(i) {
                    t.done = !t.done;
                    self.done_today = if t.done { self.done_today + 1 } else { self.done_today.saturating_sub(1) };
                    store::save_todos(&self.todos);
                    store::save_focus_today(self.focus_today, self.done_today);
                }
            }
            Act::TodoDel(i) => {
                if i < self.todos.len() {
                    self.todos.remove(i);
                    store::save_todos(&self.todos);
                }
            }
            Act::TodoFocus(i) => {
                if let Some(t) = self.todos.get(i) {
                    self.timer_task = Some(t.text.clone());
                    let m = self.timer_total_s / 60;
                    self.set_timer(m);
                    self.toggle_timer();
                }
            }
            Act::TodoAdd => {
                self.entering = Some(String::new());
                self.entry_timer = false;
            }
            Act::Launch(i) => {
                if let Some(e) = self.apps.get(i) {
                    sys::open(&e.target, &e.args);
                    return true;
                }
            }
            Act::AppUnpin(i) => {
                if i < self.apps.len() {
                    self.apps.remove(i);
                    store::save_apps(&self.apps);
                }
            }
            Act::AppAdd | Act::AppRefresh => {
                // a directory scan is quick, so it runs inline (the list is a few hundred files)
                self.picking = true;
                self.app_list = sys::list_apps();
                if act == Act::AppAdd {
                    self.app_filter.clear();
                    self.app_scroll = 0;
                }
            }
            Act::AppDone => {
                self.picking = false;
                self.entry_search = false;
                self.app_filter.clear();
            }
            Act::AppSearch => self.entry_search = true,
            Act::AppPick(i) => {
                let item = self.app_view().get(i).map(|(n, p)| (n.clone(), p.clone()));
                if let Some((name, target)) = item {
                    if self.apps.len() < 12 && !self.apps.iter().any(|a| a.target == target) {
                        self.apps.push(AppEntry { name, target, args: String::new() });
                        store::save_apps(&self.apps);
                    }
                }
            }
            Act::Screenshot => {
                sys::screenshot();
            }
            Act::Settings(i) => {
                sys::open_settings(i);
            }
            Act::Lock => sys::lock_pc(),
            Act::Sleep => sys::sleep_pc(),
            Act::Restart => self.power_press(1),
            Act::Shutdown => self.power_press(2),
        }
        false
    }

    /// Typed text goes to the open entry box. Returns true if the key was used.
    fn key(&mut self, key: &Key, text: Option<&str>) -> bool {
        if self.entry_search {
            match key {
                Key::Named(NamedKey::Escape | NamedKey::Enter) => self.entry_search = false,
                Key::Named(NamedKey::Backspace) => {
                    self.app_filter.pop();
                    self.app_scroll = 0;
                }
                _ => {
                    if let Some(t) = text.filter(|t| t.chars().all(|c| !c.is_control())) {
                        if self.app_filter.chars().count() < 40 {
                            self.app_filter.push_str(t);
                            self.app_scroll = 0;
                        }
                    }
                }
            }
            return true;
        }
        let Some(buf) = &mut self.entering else { return false };
        match key {
            Key::Named(NamedKey::Escape) => self.entering = None,
            Key::Named(NamedKey::Backspace) => {
                buf.pop();
            }
            Key::Named(NamedKey::Enter) => {
                let s = std::mem::take(buf);
                self.entering = None;
                let s = s.trim();
                if self.entry_timer {
                    if let Ok(m) = s.parse::<u32>() {
                        self.set_timer(m.clamp(1, 600));
                    }
                } else if !s.is_empty() && self.todos.len() < MAX_TODO {
                    self.todos.push(Todo { done: false, text: s.chars().take(60).collect() });
                    store::save_todos(&self.todos);
                }
            }
            _ => {
                if let Some(t) = text.filter(|t| t.chars().all(|c| !c.is_control())) {
                    if !(self.entry_timer && !t.chars().all(|c| c.is_ascii_digit())) && buf.chars().count() < 60 {
                        buf.push_str(t);
                    }
                }
            }
        }
        true
    }

    fn hot(&self, act: Act) -> bool {
        self.hover == Some(act)
    }

    // ---------------------------------------------------------------- draw ----

    fn draw(&self, g: &Gfx, hits: &mut Vec<Hit>) {
        let f = &g.fonts;
        g.fill_rrect(Rect::new(0.0, -24.0, EXP_W, EXP_H + 24.0), 24.0, Color::rgb(0, 0, 0));

        let chev = Rect::new(EXP_W - 40.0, 8.0, 28.0, 28.0);
        hits.push(Hit { r: chev, act: Act::Collapse });
        g.icon('\u{2303}', chev, &f.icon14, if self.hot(Act::Collapse) { T1 } else { T3 });

        let rects = tab_rects();
        g.fill_rrect(rects[self.page], 14.0, ACCENT.fade(0.20));
        for (i, r) in rects.iter().enumerate() {
            hits.push(Hit { r: *r, act: Act::Tab(i) });
            let hot = self.hot(Act::Tab(i));
            if i != self.page && hot {
                g.fill_rrect(*r, 14.0, Color::white(0.07));
            }
            let col = if i == self.page { ACCENT } else if hot { T2 } else { T3 };
            g.text(TAB_LABELS[i], *r, &f.small, col, Align::Center);
        }

        let start = hits.len();
        g.shifted(0.0, CONTENT_Y, || match self.page {
            CONTROLS => self.page_controls(g, hits),
            APPS => self.page_apps(g, hits),
            TODAY => self.page_today(g, hits),
            STATS => page_stats(g),
            POWER => self.page_power(g, hits),
            p => {
                let r = Rect::new(0.0, 0.0, EXP_W, EXP_H - CONTENT_Y);
                g.text(&format!("{} is not ported to Linux yet", TAB_LABELS[p]), r, &f.body, T3, Align::Center);
            }
        });
        for h in &mut hits[start..] {
            h.r.y += CONTENT_Y;
        }
    }

    fn page_apps(&self, g: &Gfx, hits: &mut Vec<Hit>) {
        if self.picking {
            return self.page_app_picker(g, hits);
        }
        let f = &g.fonts;
        let (cols, tw, th) = (6usize, 88.0, 86.0);
        let gap = (EXP_W - 56.0 - cols as f32 * tw) / (cols as f32 - 1.0);
        let cell = |i: usize| {
            let (c, r) = (i % cols, i / cols);
            Rect::new(28.0 + c as f32 * (tw + gap), 6.0 + r as f32 * (th + 8.0), tw, th)
        };
        for (i, e) in self.apps.iter().enumerate().take(12) {
            let rect = cell(i);
            let (act, xact) = (Act::Launch(i), Act::AppUnpin(i));
            hits.push(Hit { r: rect, act });
            let x = Rect::new(rect.x + rect.w - 22.0, rect.y + 3.0, 19.0, 19.0);
            hits.push(Hit { r: x, act: xact });
            let over = self.hot(act) || self.hot(xact);
            g.fill_rrect(rect, 16.0, Color::white(if over { 0.12 } else { 0.05 }));
            let ic = Rect::new(rect.cx() - 18.0, rect.y + 10.0, 36.0, 36.0);
            g.fill_circle(ic.cx(), ic.cy(), 18.0, name_color(&e.name));
            let letter: String = e.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            g.text(&letter, ic, &f.title, Color(0.0, 0.0, 0.0, 0.75), Align::Center);
            g.text(&e.name, Rect::new(rect.x + 4.0, rect.y + 52.0, rect.w - 8.0, 24.0), &f.tiny, T2, Align::Center);
            if over {
                g.fill_circle(x.cx(), x.cy(), 8.5, Color(0.0, 0.0, 0.0, 0.75));
                g.icon('\u{2715}', x, &f.icon14, if self.hot(xact) { Color(1.0, 0.5, 0.5, 1.0) } else { T1 });
            }
        }
        if self.apps.len() < 12 {
            let rect = cell(self.apps.len());
            hits.push(Hit { r: rect, act: Act::AppAdd });
            let on = self.hot(Act::AppAdd);
            g.fill_rrect(rect, 16.0, Color::white(if on { 0.12 } else { 0.04 }));
            g.stroke_rrect(rect, 16.0, Color::white(if on { 0.35 } else { 0.14 }), 1.0);
            g.fill_circle(rect.cx(), rect.y + 28.0, 18.0, ACCENT.fade(if on { 0.4 } else { 0.22 }));
            g.icon('+', Rect::new(rect.cx() - 18.0, rect.y + 10.0, 36.0, 36.0), &f.icon14, ACCENT);
            g.text("Add app", Rect::new(rect.x + 4.0, rect.y + 52.0, rect.w - 8.0, 24.0), &f.tiny, T2, Align::Center);
        }
    }

    fn page_app_picker(&self, g: &Gfx, hits: &mut Vec<Hit>) {
        let f = &g.fonts;
        g.text("Add an app", Rect::new(28.0, 4.0, 160.0, 26.0), &f.body, T1, Align::Left);
        let done = Rect::new(EXP_W - 24.0 - 76.0, 4.0, 76.0, 26.0);
        hits.push(Hit { r: done, act: Act::AppDone });
        g.fill_rrect(done, 13.0, Color::white(if self.hot(Act::AppDone) { 1.0 } else { 0.92 }));
        g.text("Done", done, &f.small, Color(0.0, 0.0, 0.0, 1.0), Align::Center);
        let sr = Rect::new(done.x - 12.0 - 210.0, 4.0, 210.0, 26.0);
        if self.entry_search || !self.app_filter.is_empty() {
            let mut t = self.app_filter.clone();
            if self.entry_search {
                t.push('\u{2502}');
            }
            g.fill_rrect(sr, 13.0, Color::white(0.14));
            g.text(&t, Rect::new(sr.x + 12.0, sr.y, sr.w - 16.0, sr.h), &f.small, T1, Align::Left);
        } else {
            g.fill_rrect(sr, 13.0, Color::white(if self.hot(Act::AppSearch) { 0.12 } else { 0.07 }));
            g.text("Search apps", Rect::new(sr.x + 12.0, sr.y, sr.w - 16.0, sr.h), &f.small, T3, Align::Left);
        }
        hits.push(Hit { r: sr, act: Act::AppSearch });
        let rf = Rect::new(sr.x - 36.0, 4.0, 26.0, 26.0);
        hits.push(Hit { r: rf, act: Act::AppRefresh });
        g.fill_rrect(rf, 13.0, Color::white(if self.hot(Act::AppRefresh) { 0.14 } else { 0.07 }));
        g.icon('\u{21BA}', rf, &f.icon14, T2);

        let list = Rect::new(24.0, 40.0, EXP_W - 48.0, PICK_ROWS as f32 * 32.0);
        let view = self.app_view();
        if view.is_empty() {
            let msg = if self.app_filter.is_empty() { "No apps found" } else { "No apps match that search" };
            g.text(msg, Rect::new(0.0, 90.0, EXP_W, 24.0), &f.body, T3, Align::Center);
            return;
        }
        for row in 0..PICK_ROWS {
            let idx = self.app_scroll + row;
            let Some((name, path)) = view.get(idx) else { break };
            let r = Rect::new(list.x, list.y + row as f32 * 32.0, list.w - 10.0, 28.0);
            let pinned = self.apps.iter().any(|p| &p.target == path);
            let act = Act::AppPick(idx);
            if !pinned {
                hits.push(Hit { r, act });
            }
            let on = self.hot(act) && !pinned;
            g.fill_rrect(r, 10.0, Color::white(if on { 0.12 } else if row % 2 == 0 { 0.05 } else { 0.0 }));
            let ic = Rect::new(r.x + 10.0, r.y + 3.0, 22.0, 22.0);
            g.fill_circle(ic.cx(), ic.cy(), 11.0, name_color(name));
            let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            g.text(&letter, ic, &f.tiny, Color(0.0, 0.0, 0.0, 0.75), Align::Center);
            g.text(name, Rect::new(r.x + 44.0, r.y, r.w - 150.0, r.h), &f.body, if pinned { T3 } else { T1 }, Align::Left);
            let (label, col) = if pinned { ("\u{2713} Pinned", T3) } else if on { ("Pin to panel", ACCENT) } else { ("", T3) };
            g.text(label, Rect::new(r.x + r.w - 130.0, r.y, 120.0, r.h), &f.small, col, Align::Right);
        }
        if view.len() > PICK_ROWS {
            let track = Rect::new(list.x + list.w - 5.0, list.y, 3.0, PICK_ROWS as f32 * 32.0 - 4.0);
            g.fill_rrect(track, 1.5, Color::white(0.08));
            let hgt = track.h * PICK_ROWS as f32 / view.len() as f32;
            let y = track.y + (track.h - hgt) * self.app_scroll as f32 / (view.len() - PICK_ROWS) as f32;
            g.fill_rrect(Rect::new(track.x, y, 3.0, hgt), 1.5, Color::white(0.3));
        }
    }

    /// One rounded tile: round icon, title, small state line.
    #[allow(clippy::too_many_arguments)]
    fn tile(&self, g: &Gfx, hits: &mut Vec<Hit>, r: Rect, act: Act, glyph: char, tint: Color, title: &str, sub: &str) {
        let f = &g.fonts;
        hits.push(Hit { r, act });
        g.fill_rrect(r, 18.0, if self.hot(act) { HOVER } else { SURF });
        let ic = Rect::new(r.x + 12.0, r.y + (r.h - 34.0) / 2.0, 34.0, 34.0);
        g.fill_circle(ic.cx(), ic.cy(), 17.0, tint.fade(0.16));
        g.icon(glyph, ic, &f.icon14, tint);
        let ty = r.y + (r.h - 44.0) / 2.0;
        g.text(title, Rect::new(r.x + 56.0, ty, r.w - 64.0, 22.0), &f.body, T1, Align::Left);
        g.text(sub, Rect::new(r.x + 56.0, ty + 24.0, r.w - 64.0, 18.0), &f.tiny, T3, Align::Left);
    }

    /// Shortcuts to the screenshot tool and the settings pages people go looking for.
    fn page_controls(&self, g: &Gfx, hits: &mut Vec<Hit>) {
        let (w, h, gap) = (270.0, 54.0, 12.0);
        let (x0, y0) = (24.0, 6.0);
        let at = |col: f32, row: f32| Rect::new(x0 + col * (w + gap), y0 + row * (h + 8.0), w, h);
        let pink = Color(0.98, 0.45, 0.55, 1.0);
        self.tile(g, hits, at(0.0, 0.0), Act::Screenshot, '\u{2702}', pink, "Screenshot", "Snip a region");
        self.tile(g, hits, at(1.0, 0.0), Act::Settings(0), '\u{25A3}', C_MEM, "Display", "Resolution, scale, night light");
        self.tile(g, hits, at(0.0, 1.0), Act::Settings(1), '\u{266A}', C_UP, "Sound", "Output, devices, volume");
        self.tile(g, hits, at(1.0, 1.0), Act::Settings(2), '\u{21C4}', C_CPU, "Network", "Wi-Fi, Ethernet, VPN");
        self.tile(g, hits, at(0.0, 2.0), Act::Settings(3), '\u{26A1}', C_BAT, "Power & battery", "Sleep, plan, battery saver");
        self.tile(g, hits, at(1.0, 2.0), Act::Settings(4), '\u{2699}', T1, "All settings", "Open system settings");
    }

    fn page_power(&self, g: &Gfx, hits: &mut Vec<Hit>) {
        let f = &g.fonts;
        let items: [(char, &str, Act, u8); 4] = [
            ('\0', "Lock", Act::Lock, 0),
            ('\u{263E}', "Sleep", Act::Sleep, 0),
            ('\u{27F3}', "Restart", Act::Restart, 1),
            ('\0', "Shut down", Act::Shutdown, 2),
        ];
        let (w, gap) = (129.0, 12.0);
        for (i, (glyph, label, act, k)) in items.iter().enumerate() {
            let r = Rect::new(24.0 + i as f32 * (w + gap), 28.0, w, 130.0);
            hits.push(Hit { r, act: *act });
            let danger = *k != 0 && self.armed(*k);
            // colour says how serious it is: calm blue and violet, amber for restart, red for shut down
            let tint = [C_CPU, C_UP, C_MEM, Color(1.0, 0.42, 0.45, 1.0)][i];
            let hot = self.hot(*act);
            let bg = if danger { Color(0.85, 0.25, 0.28, 1.0) } else if hot { HOVER } else { SURF };
            g.fill_rrect(r, 20.0, bg);
            if !danger {
                g.fill_rrect(r, 20.0, tint.fade(if hot { 0.14 } else { 0.07 }));
                g.fill_circle(r.cx(), r.y + 42.0, 24.0, tint.fade(0.16));
            }
            let ic = if danger { Color::white(1.0) } else { tint };
            let (cx, cy) = (r.cx(), r.y + 42.0);
            match act {
                // DejaVu has no lock or power symbols, so these two are drawn
                Act::Lock => {
                    g.stroke_rrect(Rect::new(cx - 6.0, cy - 12.0, 12.0, 16.0), 6.0, ic, 2.4);
                    g.fill_rrect(Rect::new(cx - 9.0, cy - 3.0, 18.0, 14.0), 3.0, ic);
                }
                Act::Shutdown => {
                    let seg = |a: f32| (cx + 10.0 * a.to_radians().sin(), cy + 2.0 - 10.0 * a.to_radians().cos());
                    for a in (40..320).step_by(10) {
                        g.line(seg(a as f32), seg(a as f32 + 10.0), 2.6, ic);
                    }
                    g.line((cx, cy - 12.0), (cx, cy), 2.6, ic);
                }
                _ => g.icon(*glyph, Rect::new(r.x, r.y + 22.0, r.w, 40.0), &f.icon22, ic),
            }
            g.text(label, Rect::new(r.x, r.y + 70.0, r.w, 24.0), &f.body, if danger { Color::white(1.0) } else { T1 }, Align::Center);
            if *k != 0 {
                g.text(
                    if danger { "Click again to confirm" } else { "Asks to confirm" },
                    Rect::new(r.x + 4.0, r.y + 96.0, r.w - 8.0, 20.0),
                    &f.tiny,
                    if danger { Color::white(0.95) } else { T3 },
                    Align::Center,
                );
            }
        }
    }

    fn page_today(&self, g: &Gfx, hits: &mut Vec<Hit>) {
        let f = &g.fonts;
        for (r, c) in [
            (Rect::new(14.0, 2.0, 208.0, 198.0), C_FOC),
            (Rect::new(230.0, 2.0, 190.0, 198.0), C_BAT),
            (Rect::new(428.0, 2.0, 158.0, 198.0), C_UP),
        ] {
            g.fill_rrect(r, 20.0, Color::white(0.05));
            g.fill_rrect(r, 20.0, c.fade(0.07));
        }

        // ---- focus timer
        let title = match &self.timer_task {
            Some(t) => format!("Focus \u{00B7} {}", t.chars().take(20).collect::<String>()),
            None => "Focus timer".to_string(),
        };
        g.text(&title, Rect::new(28.0, 12.0, 185.0, 22.0), &f.small, C_FOC, Align::Left);
        g.text(
            &format!("Today: {} min focus \u{00B7} {} done", self.focus_today, self.done_today),
            Rect::new(28.0, 176.0, 190.0, 16.0),
            &f.tiny,
            T3,
            Align::Left,
        );
        let left = self.left_ms();
        let done = !self.running() && left == 0;
        let face = match (&self.entering, self.entry_timer) {
            (Some(buf), true) => format!("{buf}\u{2502} min"),
            _ => fmt_time(left as f64 / 1000.0),
        };
        g.text(&face, Rect::new(28.0, 34.0, 175.0, 52.0), &f.big, if done && !self.entry_timer { C_FOC } else { T1 }, Align::Left);
        if self.entry_timer {
            g.text("Enter to start \u{00B7} Esc to cancel", Rect::new(28.0, 76.0, 190.0, 16.0), &f.tiny, T3, Align::Left);
        }
        if done {
            g.text("Time's up", Rect::new(110.0, 12.0, 90.0, 22.0), &f.small, C_FOC, Align::Right);
        }
        for (i, mins) in [5u32, 15, 25, 50].iter().enumerate() {
            let r = Rect::new(28.0 + i as f32 * 38.0, 94.0, 34.0, 26.0);
            let act = Act::TimerPreset(*mins);
            hits.push(Hit { r, act });
            let sel = self.timer_total_s == mins * 60;
            let bg = if self.hot(act) { HOVER } else if sel { C_FOC.fade(0.28) } else { SURF };
            g.fill_rrect(r, 13.0, bg);
            g.text(&format!("{mins}m"), r, &f.small, if sel { C_FOC } else { T2 }, Align::Center);
        }
        let cr = Rect::new(28.0 + 4.0 * 38.0, 94.0, 34.0, 26.0);
        hits.push(Hit { r: cr, act: Act::TimerCustom });
        g.fill_rrect(cr, 13.0, if self.hot(Act::TimerCustom) || self.entry_timer { HOVER } else { SURF });
        g.icon('\u{270E}', cr, &f.icon14, T2);
        let sb = Rect::new(28.0, 130.0, 108.0, 36.0);
        hits.push(Hit { r: sb, act: Act::TimerToggle });
        g.fill_rrect(sb, 18.0, C_FOC.fade(if self.hot(Act::TimerToggle) { 1.0 } else { 0.9 }));
        g.text(if self.running() { "Pause" } else { "Start" }, sb, &f.body, Color(0.0, 0.0, 0.0, 1.0), Align::Center);
        let rb = Rect::new(144.0, 130.0, 36.0, 36.0);
        hits.push(Hit { r: rb, act: Act::TimerReset });
        g.fill_rrect(rb, 18.0, if self.hot(Act::TimerReset) { HOVER } else { SURF });
        g.icon('\u{21BA}', rb, &f.icon14, T1);

        // ---- to-do
        let (tx, tw) = (240.0, 170.0);
        g.text("To-do", Rect::new(tx, 12.0, 100.0, 22.0), &f.small, C_BAT, Align::Left);
        for (i, t) in self.todos.iter().enumerate() {
            let y = 38.0 + i as f32 * 28.0;
            let row = Rect::new(tx, y, tw, 26.0);
            let cb = Rect::new(tx, y, 26.0, 26.0);
            let del = Rect::new(tx + tw - 24.0, y, 24.0, 26.0);
            let fb = Rect::new(tx + tw - 50.0, y, 24.0, 26.0);
            if !t.done {
                hits.push(Hit { r: fb, act: Act::TodoFocus(i) });
            }
            hits.push(Hit { r: cb, act: Act::TodoToggle(i) });
            hits.push(Hit { r: del, act: Act::TodoDel(i) });
            let over = self.hot(Act::TodoToggle(i)) || self.hot(Act::TodoDel(i)) || self.hot(Act::TodoFocus(i));
            if over {
                g.fill_rrect(row, 9.0, Color::white(0.07));
            }
            g.icon(if t.done { '\u{2611}' } else { '\u{2610}' }, cb, &f.icon14, if t.done { C_BAT } else { T2 });
            g.text(
                &t.text,
                Rect::new(tx + 30.0, y, tw - 30.0 - 26.0, 26.0),
                &f.small,
                if t.done { T3 } else { T1 },
                Align::Left,
            );
            if over {
                g.fill_rrect(Rect::new(tx + tw - 52.0, y + 1.0, 52.0, 24.0), 8.0, Color(0.117, 0.117, 0.117, 1.0));
                if !t.done {
                    g.icon('\u{25CE}', fb, &f.icon14, if self.hot(Act::TodoFocus(i)) { ACCENT } else { T3 });
                }
                g.icon('\u{2715}', del, &f.icon14, if self.hot(Act::TodoDel(i)) { T1 } else { T3 });
            }
        }
        let y = 38.0 + self.todos.len() as f32 * 28.0;
        if let (Some(buf), false) = (&self.entering, self.entry_timer) {
            let r = Rect::new(tx, y, tw, 26.0);
            g.fill_rrect(r, 9.0, Color::white(0.14));
            g.text(&format!("{buf}\u{2502}"), Rect::new(r.x + 10.0, r.y, r.w - 16.0, r.h), &f.small, T1, Align::Left);
            g.text("Enter to add \u{00B7} Esc to cancel", Rect::new(tx, y + 28.0, tw, 18.0), &f.tiny, T3, Align::Left);
        } else if self.todos.len() < MAX_TODO {
            let r = Rect::new(tx, y, tw, 26.0);
            hits.push(Hit { r, act: Act::TodoAdd });
            if self.hot(Act::TodoAdd) {
                g.fill_rrect(r, 9.0, Color::white(0.07));
            }
            g.icon('+', Rect::new(tx, y, 26.0, 26.0), &f.icon14, C_BAT);
            g.text("Add task", Rect::new(tx + 30.0, y, 140.0, 26.0), &f.small, T3, Align::Left);
        }

        draw_calendar(g);
    }
}

fn draw_calendar(g: &Gfx) {
    let f = &g.fonts;
    let (year, month, day, wday, ..) = crate::sys::local_time();
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
        "November", "December",
    ];
    let (x0, cw) = (432.0, 20.0);
    g.text(
        &format!("{} {}", MONTHS[(month as usize - 1).min(11)], year),
        Rect::new(x0 - 4.0, 12.0, cw * 7.0 + 8.0, 22.0),
        &f.small,
        C_UP,
        Align::Center,
    );
    for (i, d) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
        g.text(d, Rect::new(x0 + i as f32 * cw, 36.0, cw, 18.0), &f.tiny, C_UP.fade(0.55), Align::Center);
    }
    let first = first_weekday(day, wday);
    for d in 1..=days_in_month(year, month) {
        let slot = first + d - 1;
        let (c, r) = (slot % 7, slot / 7);
        let cell = Rect::new(x0 + c as f32 * cw, 56.0 + r as f32 * 20.0, cw, 20.0);
        if d == day {
            g.fill_circle(cell.cx(), cell.cy(), 9.5, C_UP);
            g.text(&d.to_string(), cell, &f.tiny, Color(0.0, 0.0, 0.0, 1.0), Align::Center);
        } else {
            g.text(&d.to_string(), cell, &f.tiny, if c == 0 || c == 6 { T3 } else { T2 }, Align::Center);
        }
    }
}

fn page_stats(g: &Gfx) {
    let f = &g.fonts;
    let tile = |x: f32, label: &str, value: &str, frac: Option<f32>| {
        let r = Rect::new(x, 20.0, 170.0, 96.0);
        g.fill_rrect(r, 14.0, SURF);
        g.text(label, Rect::new(r.x + 14.0, r.y + 10.0, r.w - 28.0, 16.0), &f.tiny, T2, Align::Left);
        g.text(value, Rect::new(r.x + 14.0, r.y + 30.0, r.w - 28.0, 36.0), &f.big, T1, Align::Left);
        if let Some(k) = frac {
            let bar = Rect::new(r.x + 14.0, r.y + r.h - 18.0, r.w - 28.0, 5.0);
            g.fill_rrect(bar, 2.5, Color::white(0.12));
            g.fill_rrect(Rect::new(bar.x, bar.y, bar.w * k.clamp(0.0, 1.0), bar.h), 2.5, ACCENT);
        }
    };
    let m = mem_info();
    tile(24.0, "MEMORY", &m.map_or("n/a".into(), |m| format!("{:.0}%", m.0 * 100.0)), m.map(|m| m.0));
    tile(215.0, "MEMORY USED", &m.map_or("n/a".into(), |m| format!("{:.1} GB", m.2 as f32 / 1024.0)), None);
    tile(406.0, "UPTIME", &uptime(), None);
}

// ----------------------------------------------------------------- window ----

struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    gfx: Option<Gfx>,
    expanded: bool,
    panel: Panel,
    /// hit boxes from the last frame (logical coordinates)
    hits: Vec<Hit>,
    /// pointer in logical (DIP) coordinates
    cursor: (f32, f32),
}

impl App {
    fn size(&self) -> (f32, f32) {
        if self.expanded { (EXP_W, EXP_H) } else { (PILL_W, PILL_H) }
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Resize to the pill or the panel and keep the window centred on the top edge.
    fn apply_size(&mut self) {
        let Some(win) = &self.window else { return };
        let (w, h) = self.size();
        let _ = win.request_inner_size(LogicalSize::new(w as f64, h as f64));
        if let Some(m) = win.current_monitor() {
            let (mp, ms) = (m.position(), m.size());
            let pw = (w * win.scale_factor() as f32) as i32;
            win.set_outer_position(PhysicalPosition::new(mp.x + (ms.width as i32 - pw) / 2, mp.y));
        }
        win.request_redraw();
    }

    fn act_at_cursor(&self) -> Option<Act> {
        let (x, y) = self.cursor;
        self.hits.iter().rev().find(|h| h.r.contains(x, y)).map(|h| h.act)
    }

    fn click(&mut self) {
        if !self.expanded {
            self.expanded = true;
        } else if let Some(act) = self.act_at_cursor() {
            if self.panel.act(act) {
                self.expanded = false;
            }
        } else {
            return;
        }
        self.panel.hover = None;
        self.apply_size();
        self.redraw();
    }

    fn draw(&mut self) {
        let (w, h) = self.size();
        let expanded = self.expanded;
        let (Some(win), Some(surface)) = (&self.window, &mut self.surface) else { return };
        let scale = win.scale_factor() as f32;
        let (pw, ph) = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
        if self.gfx.is_none() {
            // sized once for the biggest state; each frame only uses the top-left part
            self.gfx = Gfx::new((EXP_W * scale).ceil() as i32, (EXP_H * scale).ceil() as i32, scale).ok();
        }
        let Some(g) = &self.gfx else { return };
        let _ = g.begin(pw as i32, ph as i32);
        self.hits.clear();
        if expanded {
            self.panel.draw(g, &mut self.hits);
        } else {
            g.fill_rrect(Rect::new(0.0, -20.0, w, h + 20.0), 17.0, Color::rgb(0, 0, 0));
            let label = if self.panel.running() { fmt_time(self.panel.left_ms() as f64 / 1000.0) } else { "Nock".into() };
            g.text(&label, Rect::new(0.0, 0.0, w, h), &g.fonts.menu, Color::white(0.9), Align::Center);
        }
        let _ = g.end();

        let (gw, _gh, rgba) = g.pixels();
        let (Some(nw), Some(nh)) = (NonZeroU32::new(pw), NonZeroU32::new(ph)) else { return };
        if surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        for y in 0..nh.get() {
            for x in 0..nw.get() {
                let s = ((y * gw + x) * 4) as usize;
                let (r, gr, b, a) = (rgba[s] as u32, rgba[s + 1] as u32, rgba[s + 2] as u32, rgba[s + 3] as u32);
                buf[(y * nw.get() + x) as usize] = a << 24 | r << 16 | gr << 8 | b;
            }
        }
        let _ = buf.present();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title("Nock")
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_x11_window_type(vec![WindowType::Dock])
            .with_inner_size(LogicalSize::new(PILL_W as f64, PILL_H as f64));
        let Ok(win) = el.create_window(attrs) else { return };
        let win = Rc::new(win);
        if let Ok(ctx) = softbuffer::Context::new(win.clone()) {
            self.surface = softbuffer::Surface::new(&ctx, win.clone()).ok();
        }
        self.window = Some(win);
        self.apply_size();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => {
                let k = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
                self.cursor = (position.x as f32 / k, position.y as f32 / k);
                let hover = if self.expanded { self.act_at_cursor() } else { None };
                if hover != self.panel.hover {
                    self.panel.hover = hover;
                    self.redraw();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.click(),
            WindowEvent::MouseWheel { delta, .. } => {
                let rows = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y as isize,
                    MouseScrollDelta::PixelDelta(p) => (-p.y / 30.0) as isize,
                };
                if self.expanded && self.panel.scroll(rows) {
                    self.panel.hover = None;
                    self.redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.panel.key(&event.logical_key, event.text.as_deref()) {
                    self.redraw();
                }
            }
            _ => {}
        }
    }

    /// Wake once a second while the timer runs (the only thing that changes by itself).
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.panel.animating() {
            self.panel.tick();
            self.redraw();
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(500)));
        } else {
            el.set_control_flow(ControlFlow::Wait);
        }
    }
}

pub fn run() {
    // ponytail: X11 only (Xwayland on Wayland desktops); native Wayland needs layer-shell
    let el = EventLoop::builder().with_x11().build().expect("event loop");
    let mut app = App {
        window: None,
        surface: None,
        gfx: None,
        expanded: false,
        panel: Panel::new(),
        hits: Vec::new(),
        cursor: (0.0, 0.0),
    };
    if let Err(e) = el.run_app(&mut app) {
        eprintln!("nock: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keep tests away from the real data directory.
    fn sandbox() {
        std::env::set_var("XDG_DATA_HOME", std::env::temp_dir().join("nock-test"));
    }

    /// Renders each page to PNGs for eyeballing (NOCK_PREVIEW_DIR=dir cargo test -- --ignored).
    #[test]
    #[ignore]
    fn preview_pages() {
        sandbox();
        let dir = std::env::var("NOCK_PREVIEW_DIR").expect("NOCK_PREVIEW_DIR");
        let g = Gfx::new(EXP_W as i32, EXP_H as i32, 1.0).unwrap();
        let mut p = Panel::new();
        p.todos = vec![
            Todo { done: false, text: "Write the Linux port".into() },
            Todo { done: true, text: "Ship Nock 0.12.10".into() },
        ];
        p.timer_task = Some("Linux port".into());
        for page in [CONTROLS, APPS, TODAY, STATS, POWER] {
            p.page = page;
            p.hover = Some(Act::Tab(0));
            let _ = g.begin(EXP_W as i32, EXP_H as i32);
            p.draw(&g, &mut Vec::new());
            let _ = g.end();
            g.save_png(&format!("{dir}/page{page}.png"));
        }
        // the app picker, with a fake list so the render does not depend on this machine
        p.page = APPS;
        p.picking = true;
        p.app_list = (1..=9).map(|i| (format!("Sample app {i}"), format!("/x/{i}.desktop"))).collect();
        p.apps[0].target = "/x/2.desktop".into();
        let _ = g.begin(EXP_W as i32, EXP_H as i32);
        p.draw(&g, &mut Vec::new());
        let _ = g.end();
        g.save_png(&format!("{dir}/picker.png"));
    }

    #[test]
    fn todo_entry_and_timer() {
        sandbox();
        let mut p = Panel::new();
        p.todos.clear();
        p.act(Act::TodoAdd);
        for c in ["a", "b"] {
            p.key(&Key::Character(c.into()), Some(c));
        }
        assert_eq!(p.entering.as_deref(), Some("ab"));
        p.key(&Key::Named(NamedKey::Backspace), None);
        p.key(&Key::Named(NamedKey::Enter), None);
        assert_eq!(p.todos.len(), 1);
        assert_eq!(p.todos[0].text, "a");

        p.act(Act::TimerPreset(15));
        assert_eq!(p.left_ms(), 15 * 60_000);
        p.act(Act::TimerCustom);
        p.key(&Key::Character("x".into()), Some("x")); // letters are ignored for the timer length
        p.key(&Key::Character("7".into()), Some("7"));
        p.key(&Key::Named(NamedKey::Enter), None);
        assert_eq!(p.timer_total_s, 7 * 60);
        p.act(Act::TimerToggle);
        assert!(p.running());
        p.act(Act::TimerToggle);
        assert!(!p.running());
    }

    #[test]
    fn power_needs_a_second_click() {
        // only the arming click runs here: a confirmed one would really restart the machine
        sandbox();
        let mut p = Panel::new();
        assert!(!p.armed(1));
        p.act(Act::Restart);
        assert!(p.armed(1) && !p.armed(2));
    }
}
