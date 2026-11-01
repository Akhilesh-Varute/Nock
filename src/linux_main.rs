//! Linux entry point (work in progress): a borderless X11 winit window that draws
//! the notch pill and, on click, the tabbed panel through the shared `gfx` API.
//! Pages are placeholders except Today and Stats; they move over from `ui.rs`
//! as `App` sheds its Windows types.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::platform::x11::{EventLoopBuilderExtX11, WindowAttributesExtX11, WindowType};
use winit::window::{Window, WindowId, WindowLevel};

const BAR_H: f32 = 30.0;
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

use crate::cal::{days_in_month, first_weekday, fmt_time};
use crate::gfx::*;
use crate::store::{self, clip_kind_of, preview, AppEntry, ClipKind, Todo};
use crate::media_linux::{fetch_art, Mpris, Track};
use crate::linux_pop::{self, Act as PopAct, Dev, Kind as PopKind, Net, PopData, PopState, Pw, POP_MAX_H, POP_W};
use crate::sys;

const PILL_W: f32 = 176.0;
const PILL_H: f32 = 34.0;
const EXP_W: f32 = 600.0;
const EXP_H: f32 = 250.0;
const CONTENT_Y: f32 = 40.0;
const MAX_TODO: usize = 5;
const PICK_ROWS: usize = 5;
const CLIP_ROWS: usize = 5;
const SHELF_MAX: usize = 12;

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
const C_GOLD: Color = Color(1.0, 0.82, 0.28, 1.0);
const C_PINK: Color = Color(1.0, 0.50, 0.62, 1.0);
const C_FOC: Color = Color(1.0, 0.58, 0.42, 1.0);

const MEDIA_BAR: Rect = Rect::new(182.0, 104.0, EXP_W - 182.0 - 28.0, 18.0);

const TAB_LABELS: [&str; 9] =
    ["Media", "Controls", "Today", "Stats", "Clipboard", "Shelf", "Apps", "Games", "Power"];
const MEDIA: usize = 0;
const CONTROLS: usize = 1;
const CLIPBOARD: usize = 4;
const SHELF: usize = 5;
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
    PlayPause,
    Prev,
    Next,
    Seek,
    ShelfItem(usize),
    ShelfDel(usize),
    ShelfClear,
    ClipItem(usize),
    ClipPin(usize),
    ClipSearch,
    ClipSearchClear,
    ClipClear,
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
    /// Clipboard tab: recent copies, pinned snippets, filter and scroll
    clips: Vec<String>,
    pins: Vec<String>,
    clip_filter: String,
    clip_scroll: usize,
    /// last text seen on the system clipboard, and one we set ourselves and should not record
    clip_seen: Option<String>,
    ignore_clip: Option<String>,
    /// Shelf tab: kept files, and whether a file is being dragged over the notch
    shelf: Vec<String>,
    shelf_hover: bool,
    /// Media tab: the player shown, when it was read, its cover art and the colour taken from it
    mpris: Mpris,
    track: Option<Track>,
    track_at: Instant,
    last_media_poll: Instant,
    art: Option<Bitmap>,
    art_wanted: String,
    art_slot: Arc<Mutex<Option<(String, Vec<u8>)>>>,
    accent: Color,
    /// pointer in logical coordinates (a seek click needs where on the bar it landed)
    cursor: (f32, f32),
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
            clips: Vec::new(),
            pins: store::load_pins(),
            clip_filter: String::new(),
            clip_scroll: 0,
            clip_seen: sys::clipboard_text(),
            ignore_clip: None,
            shelf: store::load_shelf(),
            shelf_hover: false,
            mpris: Mpris::new(),
            track: None,
            track_at: Instant::now(),
            last_media_poll: Instant::now() - Duration::from_secs(10),
            art: None,
            art_wanted: String::new(),
            art_slot: Arc::new(Mutex::new(None)),
            accent: ACCENT,
            cursor: (0.0, 0.0),
        }
    }

    /// Installed apps matching the search text.
    fn app_view(&self) -> Vec<&(String, String)> {
        let q = self.app_filter.to_lowercase();
        self.app_list.iter().filter(|(n, _)| q.is_empty() || n.to_lowercase().contains(&q)).collect()
    }

    /// What the clipboard page lists: pinned items first, then recent ones, filtered
    /// by the search text. Each entry is (text, pinned).
    fn clip_view(&self) -> Vec<(&str, bool)> {
        let q = self.clip_filter.to_lowercase();
        self.pins
            .iter()
            .map(|t| (t.as_str(), true))
            .chain(self.clips.iter().map(|t| (t.as_str(), false)))
            .filter(|(t, _)| q.is_empty() || t.to_lowercase().contains(&q))
            .collect()
    }

    /// Record a new copy from the system clipboard. Returns true if the list changed.
    fn poll_clipboard(&mut self) -> bool {
        let Some(text) = sys::clipboard_text() else { return false };
        if self.clip_seen.as_deref() == Some(text.as_str()) {
            return false;
        }
        self.clip_seen = Some(text.clone());
        if self.ignore_clip.as_deref() == Some(text.as_str()) {
            self.ignore_clip = None;
            return false;
        }
        if text.trim().is_empty() || self.pins.contains(&text) {
            return false;
        }
        self.clips.retain(|c| *c != text);
        self.clips.insert(0, text);
        self.clips.truncate(20);
        self.clip_scroll = 0;
        true
    }

    /// Position of the playing track now, extrapolated since it was read.
    fn media_pos(&self) -> f64 {
        let Some(t) = &self.track else { return 0.0 };
        let p = t.pos_s + if t.playing { self.track_at.elapsed().as_secs_f64() } else { 0.0 };
        if t.dur_s > 0.0 { p.clamp(0.0, t.dur_s) } else { p.max(0.0) }
    }

    /// Width of the collapsed notch: wide for a track, narrow for the timer, medium for the clock.
    fn pill_w(&self) -> f32 {
        if self.has_track() { 296.0 } else if self.running() { 132.0 } else { PILL_W }
    }

    fn has_track(&self) -> bool {
        self.track.as_ref().is_some_and(|t| !(t.title.is_empty() && t.artist.is_empty()))
    }

    /// The collapsed notch: what is playing, else the focus timer, else the date and time.
    fn draw_pill(&self, g: &Gfx, w: f32, h: f32, clock_24h: bool) {
        let f = &g.fonts;
        g.fill_rrect(Rect::new(0.0, -20.0, w, h + 20.0), 17.0, Color::rgb(0, 0, 0));
        let timer = fmt_time(self.left_ms() as f64 / 1000.0);
        match &self.track {
            Some(m) if self.has_track() => {
                let ar = Rect::new(8.0, (h - 22.0) / 2.0, 22.0, 22.0);
                match &self.art {
                    Some(bmp) => g.draw_bitmap(bmp, ar, 6.0, 1.0),
                    None => {
                        g.fill_rrect(ar, 6.0, Color::white(0.14));
                        g.text("\u{266A}", ar, &f.small, T2, Align::Center);
                    }
                }
                // equaliser bars on the right: they dance while the track plays
                let bx = w - 10.0 - 20.0;
                let t = self.track_at.elapsed().as_secs_f32() + self.last_media_poll.elapsed().as_secs_f32() * 0.0;
                for i in 0..4 {
                    let k = if m.playing { 0.5 + 0.5 * (t * 3.1 + i as f32 * 1.7).sin() } else { 0.25 };
                    let bh = 3.0 + 11.0 * k;
                    g.fill_rrect(Rect::new(bx + i as f32 * 5.0, h / 2.0 - bh / 2.0, 3.0, bh), 1.5, self.accent);
                }
                let mut right = bx - 8.0;
                if self.running() {
                    g.text(&timer, Rect::new(right - 50.0, 0.0, 50.0, h), &f.small, T1, Align::Right);
                    right -= 58.0;
                }
                let title = if m.title.is_empty() { "Unknown" } else { &m.title };
                let label = if m.artist.is_empty() { title.to_string() } else { format!("{title}  \u{00B7}  {}", m.artist) };
                g.text(&label, Rect::new(38.0, 0.0, (right - 42.0).max(10.0), h), &f.small, T1, Align::Left);
            }
            _ if self.running() => {
                g.text(&timer, Rect::new(0.0, 0.0, w, h), &f.small, T1, Align::Center);
            }
            _ => {
                let (_, _, day, wday, hh, mm, ..) = sys::local_time();
                let label = if clock_24h {
                    format!("{hh:02}:{mm:02}")
                } else {
                    let (h12, ampm) = match hh {
                        0 => (12, "AM"),
                        1..=11 => (hh, "AM"),
                        12 => (12, "PM"),
                        _ => (hh - 12, "PM"),
                    };
                    format!("{h12}:{mm:02} {ampm}")
                };
                // date | time, centred as a pair; the time is the bold part
                let (dw, tw) = (50.0, if clock_24h { 44.0 } else { 70.0 });
                let x0 = (w - (dw + 15.0 + tw)) / 2.0;
                g.text(&format!("{} {day}", DAYS[(wday as usize).min(6)]), Rect::new(x0, 0.0, dw, h), &f.small, T3, Align::Right);
                g.fill_rrect(Rect::new(x0 + dw + 7.0, h / 2.0 - 7.0, 1.0, 14.0), 0.5, Color::white(0.18));
                g.text(&label, Rect::new(x0 + dw + 15.0, 0.0, tw, h), &f.menu, T1, Align::Left);
            }
        }
    }

    fn media_playing(&self) -> bool {
        self.track.as_ref().is_some_and(|t| t.playing)
    }

    /// Read the player once a second while the Media tab is open. Returns true if the page changed.
    fn poll_media(&mut self, gfx: Option<&Gfx>, every: Duration) -> bool {
        let mut changed = false;
        if self.last_media_poll.elapsed() >= every {
            self.last_media_poll = Instant::now();
            let t = self.mpris.poll();
            if t != self.track {
                changed = true;
            }
            if let Some(t) = &t {
                if t.art_url != self.art_wanted {
                    self.art_wanted = t.art_url.clone();
                    self.art = None;
                    self.accent = ACCENT;
                    if !t.art_url.is_empty() {
                        let (url, slot) = (t.art_url.clone(), self.art_slot.clone());
                        std::thread::spawn(move || {
                            if let Some(bytes) = fetch_art(&url) {
                                *slot.lock().unwrap() = Some((url, bytes));
                            }
                        });
                    }
                }
            }
            self.track = t;
            self.track_at = Instant::now();
        }
        let arrived = self.art_slot.lock().unwrap().take();
        if let (Some((url, bytes)), Some(g)) = (arrived, gfx) {
            if url == self.art_wanted {
                if let Ok((bmp, col)) = g.decode_image(&bytes) {
                    self.art = Some(bmp);
                    self.accent = col;
                    changed = true;
                }
            }
        }
        changed
    }

    fn shelf_add(&mut self, paths: impl IntoIterator<Item = String>) {
        for p in paths {
            if self.shelf.len() < SHELF_MAX && !self.shelf.contains(&p) {
                self.shelf.push(p);
            }
        }
        store::save_shelf(&self.shelf);
    }

    fn scroll(&mut self, rows: isize) -> bool {
        let step = |pos: usize, len: usize, rows_shown: usize| {
            let max = len.saturating_sub(rows_shown);
            (pos as isize + rows).clamp(0, max as isize) as usize
        };
        match (self.page, self.picking) {
            (APPS, true) => self.app_scroll = step(self.app_scroll, self.app_view().len(), PICK_ROWS),
            (CLIPBOARD, _) => self.clip_scroll = step(self.clip_scroll, self.clip_view().len(), CLIP_ROWS),
            _ => return false,
        }
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
            Act::PlayPause | Act::Prev | Act::Next => {
                if let Some(t) = &self.track {
                    match act {
                        Act::PlayPause => self.mpris.play_pause(t),
                        Act::Prev => self.mpris.prev(t),
                        _ => self.mpris.next(t),
                    }
                    // look again soon: players apply the change a moment later
                    self.last_media_poll = Instant::now() - Duration::from_millis(650);
                }
            }
            Act::Seek => {
                let frac = ((self.cursor.0 - MEDIA_BAR.x) / MEDIA_BAR.w).clamp(0.0, 1.0) as f64;
                if let Some(t) = &mut self.track {
                    if t.can_seek && t.dur_s > 0.0 {
                        self.mpris.seek_to(t, frac * t.dur_s);
                        t.pos_s = frac * t.dur_s;
                        self.track_at = Instant::now();
                    }
                }
            }
            Act::ShelfItem(i) => {
                if let Some(p) = self.shelf.get(i).cloned() {
                    if std::path::Path::new(&p).exists() {
                        sys::open(&p, "");
                        return true;
                    }
                    // gone from disk: drop it from the shelf
                    self.shelf.remove(i);
                    store::save_shelf(&self.shelf);
                }
            }
            Act::ShelfDel(i) => {
                if i < self.shelf.len() {
                    self.shelf.remove(i);
                    store::save_shelf(&self.shelf);
                }
            }
            Act::ShelfClear => {
                self.shelf.clear();
                store::save_shelf(&self.shelf);
            }
            Act::ClipItem(i) => {
                let item = self.clip_view().get(i).map(|(t, p)| (t.to_string(), *p));
                if let Some((t, pinned)) = item {
                    self.ignore_clip = Some(t.clone());
                    if sys::set_clipboard_text(&t) {
                        self.clip_seen = Some(t.clone());
                        if !pinned {
                            self.clips.retain(|c| *c != t);
                            self.clips.insert(0, t);
                        }
                        self.clip_scroll = 0;
                        sys::paste_soon();
                        return true;
                    }
                    self.ignore_clip = None;
                }
            }
            Act::ClipPin(i) => {
                let item = self.clip_view().get(i).map(|(t, p)| (t.to_string(), *p));
                if let Some((t, pinned)) = item {
                    if pinned {
                        self.pins.retain(|p| *p != t);
                        self.clips.insert(0, t);
                    } else {
                        self.clips.retain(|c| *c != t);
                        self.pins.insert(0, t);
                        self.pins.truncate(20);
                    }
                    store::save_pins(&self.pins);
                }
            }
            Act::ClipSearch => self.entry_search = true,
            Act::ClipSearchClear => {
                self.clip_filter.clear();
                self.clip_scroll = 0;
            }
            Act::ClipClear => {
                self.clips.clear();
                self.clip_scroll = 0;
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
            let clip = self.page == CLIPBOARD;
            let filter = if clip { &mut self.clip_filter } else { &mut self.app_filter };
            let mut changed = false;
            match key {
                // Esc clears the clipboard filter (as on Windows); Enter keeps it
                Key::Named(NamedKey::Escape) => {
                    self.entry_search = false;
                    if clip {
                        filter.clear();
                        changed = true;
                    }
                }
                Key::Named(NamedKey::Enter) => self.entry_search = false,
                Key::Named(NamedKey::Backspace) => changed = filter.pop().is_some(),
                _ => {
                    if let Some(t) = text.filter(|t| t.chars().all(|c| !c.is_control())) {
                        if filter.chars().count() < 40 {
                            filter.push_str(t);
                            changed = true;
                        }
                    }
                }
            }
            if changed {
                self.app_scroll = 0;
                self.clip_scroll = 0;
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
            MEDIA => self.page_media(g, hits),
            CONTROLS => self.page_controls(g, hits),
            APPS => self.page_apps(g, hits),
            CLIPBOARD => self.page_clipboard(g, hits),
            SHELF => self.page_shelf(g, hits),
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

    fn page_media(&self, g: &Gfx, hits: &mut Vec<Hit>) {
        let f = &g.fonts;
        let Some(m) = &self.track else {
            g.text("\u{266A}", Rect::new(0.0, 50.0, EXP_W, 50.0), &f.big, T3, Align::Center);
            g.text("Nothing playing", Rect::new(0.0, 98.0, EXP_W, 24.0), &f.body, T2, Align::Center);
            g.text("Start music or a video and it shows up here", Rect::new(0.0, 122.0, EXP_W, 20.0), &f.small, T3, Align::Center);
            return;
        };
        let ar = Rect::new(28.0, 22.0, 132.0, 132.0);
        match &self.art {
            Some(bmp) => g.draw_bitmap(bmp, ar, 16.0, 1.0),
            None => {
                g.fill_rrect(ar, 16.0, Color::white(0.10));
                g.text("\u{266A}", ar, &f.big, T3, Align::Center);
            }
        }
        let tx = 182.0;
        let tw = EXP_W - tx - 56.0;
        let title = if m.title.is_empty() { "Unknown title" } else { &m.title };
        g.text(title, Rect::new(tx, 22.0, tw, 26.0), &f.title, T1, Align::Left);
        let sub = if m.artist.is_empty() { &m.album } else { &m.artist };
        g.text(sub, Rect::new(tx, 49.0, tw, 22.0), &f.body, T2, Align::Left);
        if !m.app.is_empty() {
            g.text(&m.app, Rect::new(tx, 72.0, tw, 18.0), &f.tiny, T3, Align::Left);
        }

        // progress
        let (pos, dur) = (self.media_pos(), m.dur_s);
        let frac = if dur > 0.0 { (pos / dur) as f32 } else { 0.0 };
        let bar = MEDIA_BAR;
        let track = Rect::new(bar.x, bar.cy() - 3.0, bar.w, 6.0);
        g.fill_rrect(track, 3.0, Color::white(0.16));
        let fw = bar.w * frac.clamp(0.0, 1.0);
        if fw > 0.5 {
            g.fill_rrect(Rect::new(bar.x, track.y, fw, 6.0), 3.0, self.accent);
        }
        if dur > 0.0 && m.can_seek {
            hits.push(Hit { r: bar, act: Act::Seek });
            if self.hot(Act::Seek) {
                g.fill_circle(bar.x + fw, bar.cy(), 7.0, Color::white(1.0));
            }
        }
        g.text(&fmt_time(pos), Rect::new(tx, 122.0, 70.0, 16.0), &f.tiny, T3, Align::Left);
        if dur > 0.0 {
            g.text(&fmt_time(dur), Rect::new(bar.x + bar.w - 70.0, 122.0, 70.0, 16.0), &f.tiny, T3, Align::Right);
        }

        // transport, drawn as shapes (no icon font)
        let cx = tx + (EXP_W - tx - 28.0) / 2.0;
        let cy = 168.0;
        let prev = Rect::new(cx - 92.0, cy - 20.0, 40.0, 40.0);
        let play = Rect::new(cx - 26.0, cy - 26.0, 52.0, 52.0);
        let next = Rect::new(cx + 52.0, cy - 20.0, 40.0, 40.0);
        for (r, act, ok) in [(prev, Act::Prev, m.can_prev), (next, Act::Next, m.can_next)] {
            hits.push(Hit { r, act });
            if self.hot(act) {
                g.fill_rrect(r, 20.0, Color::white(0.10));
            }
            let c = if ok { T1 } else { T3 };
            let (x, y) = (r.cx(), r.cy());
            if act == Act::Prev {
                g.fill_rrect(Rect::new(x - 8.0, y - 7.0, 2.5, 14.0), 1.0, c);
                g.fill_poly(&[(x + 7.0, y - 7.0), (x + 7.0, y + 7.0), (x - 4.0, y)], c);
            } else {
                g.fill_rrect(Rect::new(x + 5.5, y - 7.0, 2.5, 14.0), 1.0, c);
                g.fill_poly(&[(x - 7.0, y - 7.0), (x - 7.0, y + 7.0), (x + 4.0, y)], c);
            }
        }
        hits.push(Hit { r: play, act: Act::PlayPause });
        g.fill_circle(play.cx(), play.cy(), 24.0, Color::white(if self.hot(Act::PlayPause) { 1.0 } else { 0.92 }));
        let (x, y) = (play.cx(), play.cy());
        let black = Color(0.0, 0.0, 0.0, 1.0);
        if m.playing {
            g.fill_rrect(Rect::new(x - 7.0, y - 8.0, 4.5, 16.0), 1.5, black);
            g.fill_rrect(Rect::new(x + 2.5, y - 8.0, 4.5, 16.0), 1.5, black);
        } else {
            g.fill_poly(&[(x - 6.0, y - 9.0), (x - 6.0, y + 9.0), (x + 9.0, y)], black);
        }
    }

    fn page_shelf(&self, g: &Gfx, hits: &mut Vec<Hit>) {
        let f = &g.fonts;
        let area = Rect::new(20.0, 2.0, EXP_W - 40.0, 202.0);
        if self.shelf_hover {
            // a file is being dragged over the notch
            g.fill_rrect(area, 18.0, ACCENT.fade(0.14));
            g.stroke_rrect(area, 18.0, ACCENT.fade(0.9), 2.0);
            g.text("Drop to add to the shelf", area, &f.body, T1, Align::Center);
            return;
        }
        if self.shelf.is_empty() {
            g.stroke_rrect(area, 18.0, Color::white(0.14), 1.0);
            g.icon('\u{21E9}', Rect::new(area.cx() - 20.0, 62.0, 40.0, 40.0), &f.icon22, ACCENT);
            g.text("Drop files here to keep them handy", Rect::new(area.x, 108.0, area.w, 24.0), &f.body, T1, Align::Center);
            g.text("Click one to open it.", Rect::new(area.x, 132.0, area.w, 22.0), &f.small, T3, Align::Center);
            return;
        }
        let (cols, tw, th) = (6usize, 88.0, 86.0);
        let gap = (EXP_W - 56.0 - cols as f32 * tw) / (cols as f32 - 1.0);
        for (i, path) in self.shelf.iter().enumerate() {
            let (c, r) = (i % cols, i / cols);
            let rect = Rect::new(28.0 + c as f32 * (tw + gap), 4.0 + r as f32 * (th + 6.0), tw, th);
            let (act, xact) = (Act::ShelfItem(i), Act::ShelfDel(i));
            hits.push(Hit { r: rect, act });
            let x = Rect::new(rect.x + rect.w - 22.0, rect.y + 3.0, 19.0, 19.0);
            hits.push(Hit { r: x, act: xact });
            let over = self.hot(act) || self.hot(xact);
            g.fill_rrect(rect, 16.0, Color::white(if over { 0.12 } else { 0.05 }));
            let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
            draw_file_icon(g, Rect::new(rect.cx() - 18.0, rect.y + 10.0, 36.0, 36.0), std::path::Path::new(path).is_dir());
            g.text(&name, Rect::new(rect.x + 4.0, rect.y + 52.0, rect.w - 8.0, 24.0), &f.tiny, T2, Align::Center);
            if over {
                g.fill_circle(x.cx(), x.cy(), 8.5, Color(0.0, 0.0, 0.0, 0.75));
                g.icon('\u{2715}', x, &f.icon14, if self.hot(xact) { Color(1.0, 0.5, 0.5, 1.0) } else { T1 });
            }
        }
        let clear = Rect::new(EXP_W - 24.0 - 78.0, 190.0, 78.0, 18.0);
        hits.push(Hit { r: clear, act: Act::ShelfClear });
        g.text("Clear all", clear, &f.tiny, if self.hot(Act::ShelfClear) { T1 } else { T3 }, Align::Right);
    }

    fn page_clipboard(&self, g: &Gfx, hits: &mut Vec<Hit>) {
        let f = &g.fonts;
        let view = self.clip_view();
        let total = self.pins.len() + self.clips.len();
        let head = if total == 0 { "Clipboard".to_string() } else { format!("Clipboard  \u{00B7}  {total}") };
        g.text(&head, Rect::new(28.0, 12.0, 200.0, 24.0), &f.small, C_MEM, Align::Left);
        if !self.clips.is_empty() {
            let r = Rect::new(EXP_W - 112.0, 12.0, 64.0, 24.0);
            hits.push(Hit { r, act: Act::ClipClear });
            if self.hot(Act::ClipClear) {
                g.fill_rrect(r, 12.0, Color::white(0.10));
            }
            g.text("Clear", r, &f.small, C_PINK, Align::Center);
        }
        // search chip: click, type to filter, Enter keeps it, Esc clears it
        let sr = Rect::new(EXP_W - 252.0, 12.0, 132.0, 24.0);
        let typing = self.entry_search;
        if typing || !self.clip_filter.is_empty() {
            let mut t = self.clip_filter.clone();
            if typing {
                t.push('\u{2502}');
            }
            g.fill_rrect(sr, 12.0, Color::white(0.14));
            g.text(&t, Rect::new(sr.x + 10.0, sr.y, sr.w - 30.0, sr.h), &f.small, T1, Align::Left);
            if !typing {
                let x = Rect::new(sr.x + sr.w - 24.0, sr.y, 24.0, sr.h);
                hits.push(Hit { r: sr, act: Act::ClipSearch });
                hits.push(Hit { r: x, act: Act::ClipSearchClear });
                g.icon('\u{2715}', x, &f.icon14, if self.hot(Act::ClipSearchClear) { T1 } else { T2 });
            }
        } else {
            hits.push(Hit { r: sr, act: Act::ClipSearch });
            if self.hot(Act::ClipSearch) {
                g.fill_rrect(sr, 12.0, Color::white(0.10));
            }
            g.text("Search", Rect::new(sr.x + 10.0, sr.y, sr.w - 14.0, sr.h), &f.small, T3, Align::Left);
        }

        if view.is_empty() {
            let (l1, l2) = if self.clip_filter.is_empty() {
                ("Copy something and it will appear here.", "Click an item to paste it. Click the star to pin it as a saved snippet.")
            } else {
                ("No matches.", "Press Esc in the search box to clear the filter.")
            };
            g.text(l1, Rect::new(0.0, 80.0, EXP_W, 24.0), &f.body, T3, Align::Center);
            g.text(l2, Rect::new(0.0, 104.0, EXP_W, 20.0), &f.small, T3.fade(0.8), Align::Center);
            return;
        }
        for row in 0..CLIP_ROWS {
            let idx = self.clip_scroll + row;
            let Some((text, pinned)) = view.get(idx) else { break };
            let r = Rect::new(24.0, 40.0 + row as f32 * 31.0, EXP_W - 48.0, 28.0);
            let (act, pact) = (Act::ClipItem(idx), Act::ClipPin(idx));
            hits.push(Hit { r, act });
            let pin = Rect::new(r.x + r.w - 32.0, r.y, 30.0, r.h);
            hits.push(Hit { r: pin, act: pact });
            let (kind, kc) = match clip_kind_of(text) {
                ClipKind::Link => ('\u{2197}', C_CPU),
                ClipKind::Mail => ('\u{2709}', C_BAT),
                ClipKind::Path => ('\u{2302}', C_MEM),
                ClipKind::Code => ('\u{2328}', C_PINK),
                ClipKind::Text => ('\u{2261}', C_UP),
            };
            let over = self.hot(act) || self.hot(pact);
            g.fill_rrect(r, 10.0, Color::white(if over { 0.12 } else { 0.04 }));
            g.fill_rrect(r, 10.0, kc.fade(0.06));
            let chip = Rect::new(r.x + 6.0, r.y + 3.0, 22.0, 22.0);
            g.fill_circle(chip.cx(), chip.cy(), 11.0, kc.fade(0.20));
            g.icon(kind, chip, &f.icon14, kc);
            g.text(&preview(text), Rect::new(r.x + 38.0, r.y, r.w - 76.0, r.h), &f.body, T1, Align::Left);
            let (glyph, col) = if *pinned {
                ('\u{2605}', C_GOLD)
            } else if self.hot(pact) {
                ('\u{2606}', T1)
            } else {
                ('\u{2606}', T3)
            };
            g.icon(glyph, pin, &f.icon14, col.fade(if *pinned || over { 1.0 } else { 0.6 }));
        }
        if view.len() > CLIP_ROWS {
            let (top, h) = (40.0, CLIP_ROWS as f32 * 31.0 - 3.0);
            let track = Rect::new(EXP_W - 17.0, top, 4.0, h);
            g.fill_rrect(track, 2.0, Color::white(0.07));
            let thumb_h = (h * CLIP_ROWS as f32 / view.len() as f32).max(18.0);
            let frac = self.clip_scroll as f32 / (view.len() - CLIP_ROWS) as f32;
            g.fill_rrect(Rect::new(track.x, top + (h - thumb_h) * frac.clamp(0.0, 1.0), 4.0, thumb_h), 2.0, C_MEM.fade(0.8));
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

/// A file (page with a folded corner) or a folder (tabbed), drawn so no icon font is needed.
fn draw_file_icon(g: &Gfx, r: Rect, folder: bool) {
    let (cx, cy) = (r.cx(), r.cy());
    if folder {
        g.fill_rrect(Rect::new(cx - 16.0, cy - 12.0, 14.0, 8.0), 3.0, C_MEM.fade(0.8));
        g.fill_rrect(Rect::new(cx - 16.0, cy - 8.0, 32.0, 22.0), 4.0, C_MEM);
    } else {
        g.fill_poly(&[(cx - 11.0, cy - 15.0), (cx + 3.0, cy - 15.0), (cx + 11.0, cy - 7.0), (cx + 11.0, cy + 15.0), (cx - 11.0, cy + 15.0)], C_CPU);
        g.fill_poly(&[(cx + 3.0, cy - 15.0), (cx + 3.0, cy - 7.0), (cx + 11.0, cy - 7.0)], Color::white(0.55));
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
    /// the drag itself opened the panel
    shelf_auto_open: bool,
    clock_24h: bool,
    /// the collapsed width last applied, and the minute last drawn
    last_pill_w: f32,
    last_minute: u32,
    bar: Option<BarWin>,
    pop: Option<PopWin>,
}

impl App {
    fn size(&self) -> (f32, f32) {
        if self.expanded { (EXP_W, EXP_H) } else { (self.panel.pill_w(), PILL_H) }
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
        if let Some(p) = self.home() {
            win.set_outer_position(p);
        }
        win.request_redraw();
    }

    /// Where the notch belongs: top centre of its monitor.
    fn home(&self) -> Option<PhysicalPosition<i32>> {
        let win = self.window.as_ref()?;
        let m = win.current_monitor()?;
        let (mp, ms) = (m.position(), m.size());
        let pw = (self.size().0 * win.scale_factor() as f32) as i32;
        Some(PhysicalPosition::new(mp.x + (ms.width as i32 - pw) / 2, mp.y))
    }

    fn act_at_cursor(&self) -> Option<Act> {
        let (x, y) = self.cursor;
        self.hits.iter().rev().find(|h| h.r.contains(x, y)).map(|h| h.act)
    }

    /// A file drag reached the notch: open the Shelf tab (and fold it again if nothing is dropped).
    fn shelf_enter(&mut self) {
        self.panel.shelf_hover = true;
        if !self.expanded {
            self.shelf_auto_open = true;
            self.expanded = true;
            self.apply_size();
        }
        self.panel.page = SHELF;
        self.redraw();
    }

    fn shelf_leave(&mut self) {
        self.panel.shelf_hover = false;
        if std::mem::take(&mut self.shelf_auto_open) {
            self.expanded = false;
            self.apply_size();
        }
        self.redraw();
    }

    fn click(&mut self) {
        self.panel.cursor = self.cursor;
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
            self.panel.draw_pill(g, w, h, self.clock_24h);
        }
        let _ = g.end();
        blit(surface, g, pw, ph);
    }

    // ------------------------------------------------------------ top bar ----

    fn bar_event(&mut self, el: &ActiveEventLoop, ev: WindowEvent) {
        let Some(bar) = &mut self.bar else { return };
        match ev {
            WindowEvent::RedrawRequested => {
                let data = bar.data.lock().unwrap().clone();
                bar.draw(&data, self.clock_24h);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let k = bar.window.scale_factor() as f32;
                let (x, y) = (position.x as f32 / k, position.y as f32 / k);
                let hover = bar.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| *a);
                if hover != bar.hover {
                    bar.hover = hover;
                    bar.window.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                bar.hover = None;
                bar.window.request_redraw();
            }
            // an item opens its dropdown; clicking it again closes it
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                let Some(act) = bar.hover else { return };
                let Some(anchor) = bar.hits.iter().find(|(_, a)| *a == act).map(|(r, _)| *r) else { return };
                let kind = match act {
                    BarAct::Wifi => PopKind::Wifi,
                    BarAct::Bluetooth => PopKind::Bluetooth,
                    BarAct::Volume => PopKind::Volume,
                    BarAct::Brightness => PopKind::Brightness,
                    BarAct::Battery => PopKind::Battery,
                    BarAct::Clock => PopKind::Clock,
                };
                if self.pop.as_ref().is_some_and(|p| p.kind == kind) {
                    self.pop = None;
                } else {
                    self.open_pop(el, kind, anchor);
                }
            }
            _ => {}
        }
    }

    /// Refresh what the bar shows: the focused app every couple of seconds, the rest
    /// every ten, all read on a worker thread so the window never waits for a process.
    fn bar_tick(&mut self, now_minute: u32) {
        let Some(bar) = &mut self.bar else { return };
        let mut redraw = bar.last_minute != now_minute;
        bar.last_minute = now_minute;
        if bar.last_fetch.elapsed() >= Duration::from_secs(2) && !bar.fetching.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let full = bar.last_full.elapsed() >= Duration::from_secs(10);
            bar.last_fetch = Instant::now();
            if full {
                bar.last_full = Instant::now();
            }
            let (data, busy) = (bar.data.clone(), bar.fetching.clone());
            std::thread::spawn(move || {
                let app = sys::active_app();
                let mut d = data.lock().unwrap().clone();
                d.app = app;
                if full {
                    d.wifi_on = sys::wifi_present().then(sys::wifi_enabled);
                    d.wifi = sys::wifi_name();
                    d.bt = sys::bt_powered();
                    d.volume = sys::volume();
                    d.brightness = sys::brightness();
                    d.battery = sys::battery_info();
                }
                *data.lock().unwrap() = d;
                busy.store(false, std::sync::atomic::Ordering::SeqCst);
            });
        }
        if *bar.data.lock().unwrap() != bar.shown {
            redraw = true;
        }
        if redraw {
            bar.window.request_redraw();
        }
    }
}

/// Copy the drawn frame (premultiplied RGBA) into the window's buffer.
fn blit(surface: &mut softbuffer::Surface<Rc<Window>, Rc<Window>>, g: &Gfx, pw: u32, ph: u32) {
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

#[derive(Clone, Copy, PartialEq, Debug)]
enum BarAct {
    Wifi,
    Bluetooth,
    Volume,
    Brightness,
    Battery,
    Clock,
}

#[derive(Clone, Default, PartialEq)]
struct BarData {
    app: String,
    /// Wi-Fi radio (None without NetworkManager) and the network joined
    wifi_on: Option<bool>,
    wifi: Option<String>,
    /// Bluetooth power (None without BlueZ)
    bt: Option<bool>,
    volume: Option<(u8, bool)>,
    brightness: Option<u8>,
    battery: Option<(u8, bool)>,
}

/// The thin full-width strip at the top of the screen.
struct BarWin {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    gfx: Option<Gfx>,
    hits: Vec<(Rect, BarAct)>,
    hover: Option<BarAct>,
    data: Arc<Mutex<BarData>>,
    /// what the last frame showed, to redraw only on change
    shown: BarData,
    fetching: Arc<std::sync::atomic::AtomicBool>,
    last_fetch: Instant,
    last_full: Instant,
    last_minute: u32,
}

impl BarWin {
    fn new(el: &ActiveEventLoop) -> Option<BarWin> {
        let mon = el.primary_monitor()?;
        let scale = mon.scale_factor();
        let attrs = Window::default_attributes()
            .with_title("Nock bar")
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_x11_window_type(vec![WindowType::Dock])
            .with_position(mon.position())
            .with_inner_size(LogicalSize::new(mon.size().width as f64 / scale, BAR_H as f64));
        let window = Rc::new(el.create_window(attrs).ok()?);
        let ctx = softbuffer::Context::new(window.clone()).ok()?;
        let surface = softbuffer::Surface::new(&ctx, window.clone()).ok()?;
        Some(BarWin {
            window,
            surface,
            gfx: None,
            hits: Vec::new(),
            hover: None,
            data: Arc::new(Mutex::new(BarData::default())),
            shown: BarData::default(),
            fetching: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_fetch: Instant::now() - Duration::from_secs(60),
            last_full: Instant::now() - Duration::from_secs(60),
            last_minute: 99,
        })
    }

    fn draw(&mut self, d: &BarData, clock_24h: bool) {
        let k = self.window.scale_factor() as f32;
        let size = self.window.inner_size();
        let (pw, ph) = (size.width.max(1), size.height.max(1));
        let w = pw as f32 / k;
        if self.gfx.is_none() {
            self.gfx = Gfx::new(pw as i32, ph as i32, k).ok();
        }
        let Some(g) = &self.gfx else { return };
        let _ = g.begin(pw as i32, ph as i32);
        paint_bar(g, w, d, self.hover, clock_24h, &mut self.hits);
        let _ = g.end();
        blit(&mut self.surface, g, pw, ph);
        self.shown = d.clone();
    }
}

/// Everything the bar shows, drawn into `g`; fills `hits` with where the items are.
fn paint_bar(g: &Gfx, w: f32, d: &BarData, hover: Option<BarAct>, clock_24h: bool, hits: &mut Vec<(Rect, BarAct)>) {
    let f = &g.fonts;
    g.fill_rrect(Rect::new(0.0, -10.0, w, BAR_H + 10.0), 0.0, Color(0.05, 0.05, 0.06, 0.62));
    g.fill_rrect(Rect::new(0.0, BAR_H - 1.0, w, 1.0), 0.0, Color::white(0.10));

    let bar = Rect::new(0.0, 0.0, w, BAR_H);
    if !d.app.is_empty() {
        g.text(&d.app, Rect::new(14.0, 0.0, w / 2.0 - 160.0, BAR_H), &f.menu, T1, Align::Left);
    }

    // right side, built from the edge inwards
    hits.clear();
    let mut x = w - 14.0;
    let hot = hover;
    let mut item = |g: &Gfx, act: BarAct, text: &str, extra: f32, hits: &mut Vec<(Rect, BarAct)>| -> Rect {
        let tw = g.text_width(text, &g.fonts.small);
        let r = Rect::new(x - tw - extra - 8.0, 3.0, tw + extra + 8.0, BAR_H - 6.0);
        hits.push((r, act));
        if hot == Some(act) {
            g.fill_rrect(r, 8.0, Color::white(0.16));
        }
        x = r.x - 6.0;
        Rect::new(r.x + 4.0, bar.y, r.w - 8.0, BAR_H)
    };

    let (_, month, day, wday, hh, mm, ..) = sys::local_time();
    let clock = if clock_24h {
        format!("{hh:02}:{mm:02}")
    } else {
        let (h12, ampm) = match hh {
            0 => (12, "AM"),
            1..=11 => (hh, "AM"),
            12 => (12, "PM"),
            _ => (hh - 12, "PM"),
        };
        format!("{h12}:{mm:02} {ampm}")
    };
    let text = format!("{} {} {}   {clock}", DAYS[(wday as usize).min(6)], day, MONTHS[(month as usize - 1).min(11)]);
    let r = item(g, BarAct::Clock, &text, 0.0, hits);
    g.text(&text, r, &f.small, T1, Align::Right);

    if let Some((pct, plugged)) = d.battery {
        let label = format!("{pct}%");
        let r = item(g, BarAct::Battery, &label, 24.0, hits);
        g.text(&label, r, &f.small, T1, Align::Right);
        // a drawn battery: outline, fill by charge (amber when low), a nub
        let (bx, by) = (r.x, BAR_H / 2.0 - 5.0);
        g.stroke_rrect(Rect::new(bx, by, 18.0, 10.0), 2.5, T2, 1.0);
        g.fill_rrect(Rect::new(bx + 18.5, by + 3.0, 1.8, 4.0), 0.9, T2);
        let col = if plugged { C_BAT } else if pct <= 20 { Color(1.0, 0.72, 0.2, 1.0) } else { T1 };
        g.fill_rrect(Rect::new(bx + 2.0, by + 2.0, 14.0 * pct as f32 / 100.0, 6.0), 1.2, col);
    }
    if let Some(pct) = d.brightness {
        let label = format!("{pct}%");
        let r = item(g, BarAct::Brightness, &label, 16.0, hits);
        g.text("\u{263C}", Rect::new(r.x, 0.0, 14.0, BAR_H), &f.small, T2, Align::Left);
        g.text(&label, r, &f.small, T1, Align::Right);
    }
    if let Some((pct, muted)) = d.volume {
        let label = if muted { "Muted".to_string() } else { format!("{pct}%") };
        let r = item(g, BarAct::Volume, &label, 16.0, hits);
        g.text("\u{266A}", Rect::new(r.x, 0.0, 14.0, BAR_H), &f.small, if muted { T3 } else { T2 }, Align::Left);
        g.text(&label, r, &f.small, if muted { T3 } else { T1 }, Align::Right);
    }
    if let Some(on) = d.bt {
        let label = if on { "BT" } else { "BT off" };
        let r = item(g, BarAct::Bluetooth, label, 0.0, hits);
        g.text(label, r, &f.small, if on { T1 } else { T3 }, Align::Right);
    }
    if let Some(on) = d.wifi_on {
        let label: String = match (&d.wifi, on) {
            (Some(name), true) => name.chars().take(18).collect(),
            (None, true) => "Not connected".into(),
            _ => "Wi-Fi off".into(),
        };
        let r = item(g, BarAct::Wifi, &label, 16.0, hits);
        g.text("\u{21C4}", Rect::new(r.x, 0.0, 14.0, BAR_H), &f.small, T2, Align::Left);
        g.text(&label, r, &f.small, if on && d.wifi.is_some() { T1 } else { T3 }, Align::Right);
    }
}

/// A dropdown under a bar item. Closes when the pointer leaves it, on Esc, or when its item is clicked again.
struct PopWin {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    gfx: Gfx,
    kind: PopKind,
    data: Arc<Mutex<PopData>>,
    state: PopState,
    hits: Vec<(Rect, PopAct)>,
    /// height last drawn, in DIPs
    h: f32,
    home: PhysicalPosition<i32>,
    moves: u8,
    entered: bool,
    opened: Instant,
    cursor: (f32, f32),
    /// slider being dragged, the value waiting to be sent and when one was last sent
    drag: Option<Rect>,
    pending: Option<u8>,
    last_send: Instant,
    last_spin: Instant,
    msg_at: Option<Instant>,
}

/// Read everything `kind` shows from the system (runs on a worker thread).
fn refresh_pop(kind: PopKind, data: &Arc<Mutex<PopData>>) {
    match kind {
        PopKind::Wifi => {
            let on = sys::wifi_enabled();
            let saved = sys::saved_connections();
            let nets = if on {
                sys::wifi_networks()
                    .into_iter()
                    .map(|(active, ssid, signal, secure)| Net { known: saved.contains(&ssid), ssid, signal, secure, active })
                    .collect()
            } else {
                Vec::new()
            };
            let mut d = data.lock().unwrap();
            (d.wifi_on, d.nets, d.wifi_loaded) = (on, nets, true);
        }
        PopKind::Bluetooth => {
            let on = sys::bt_powered().unwrap_or(false);
            let devs = if on { sys::bt_devices().into_iter().map(|(mac, name, connected)| Dev { mac, name, connected }).collect() } else { Vec::new() };
            let mut d = data.lock().unwrap();
            (d.bt_on, d.devs, d.bt_loaded) = (on, devs, true);
        }
        PopKind::Volume => data.lock().unwrap().volume = sys::volume(),
        PopKind::Brightness => data.lock().unwrap().brightness = sys::brightness(),
        PopKind::Battery => data.lock().unwrap().battery = sys::battery_info(),
        PopKind::Clock => {}
    }
}

impl App {
    fn open_pop(&mut self, el: &ActiveEventLoop, kind: PopKind, anchor: Rect) {
        self.pop = None;
        let Some(bar) = &self.bar else { return };
        let Some(mon) = bar.window.current_monitor().or_else(|| el.primary_monitor()) else { return };
        let k = mon.scale_factor();
        let Ok(gfx) = Gfx::new((POP_W * k as f32).ceil() as i32, (POP_MAX_H * k as f32).ceil() as i32, k as f32) else { return };
        let data = Arc::new(Mutex::new(PopData::default()));
        let state = PopState::default();
        let _ = gfx.begin((POP_W * k as f32).ceil() as i32, (POP_MAX_H * k as f32).ceil() as i32);
        let h = linux_pop::paint(&gfx, kind, &data.lock().unwrap(), &state, &mut Vec::new(), self.clock_24h);
        let _ = gfx.end();

        let mon_w = mon.size().width as f64 / k;
        let x = (anchor.cx() as f64 - POP_W as f64 / 2.0).clamp(4.0, (mon_w - POP_W as f64 - 4.0).max(4.0));
        let home = PhysicalPosition::new(mon.position().x + (x * k) as i32, mon.position().y + ((BAR_H + 4.0) as f64 * k) as i32);
        let attrs = Window::default_attributes()
            .with_title("Nock dropdown")
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_x11_window_type(vec![WindowType::Dock])
            .with_position(home)
            .with_inner_size(LogicalSize::new(POP_W as f64, h as f64));
        let Ok(window) = el.create_window(attrs).map(Rc::new) else { return };
        let Ok(ctx) = softbuffer::Context::new(window.clone()) else { return };
        let Ok(surface) = softbuffer::Surface::new(&ctx, window.clone()) else { return };
        window.focus_window();

        let d2 = data.clone();
        std::thread::spawn(move || refresh_pop(kind, &d2));
        self.pop = Some(PopWin {
            window,
            surface,
            gfx,
            kind,
            data,
            state,
            hits: Vec::new(),
            h,
            home,
            moves: 0,
            entered: false,
            opened: Instant::now(),
            cursor: (0.0, 0.0),
            drag: None,
            pending: None,
            last_send: Instant::now(),
            last_spin: Instant::now(),
            msg_at: None,
        });
    }

    /// Housekeeping while a dropdown is open. Returns true if one is.
    fn pop_tick(&mut self) -> bool {
        let Some(p) = &mut self.pop else { return false };
        // never entered: the pointer went elsewhere, so let it go
        if !p.entered && p.opened.elapsed() > Duration::from_millis(2500) {
            self.pop = None;
            return false;
        }
        let snap = p.data.lock().unwrap().clone();
        let spinning = snap.busy.is_some() || (!snap.wifi_loaded && p.kind == PopKind::Wifi && snap.wifi_on != false) || (!snap.bt_loaded && p.kind == PopKind::Bluetooth);
        let mut redraw = false;
        if spinning && p.last_spin.elapsed() >= Duration::from_millis(110) {
            p.last_spin = Instant::now();
            p.state.spin = (p.state.spin + 1) % 8;
            redraw = true;
        }
        match (&snap.msg, p.msg_at) {
            (Some(_), None) => p.msg_at = Some(Instant::now()),
            (Some(_), Some(t)) if t.elapsed() > Duration::from_secs(4) => {
                p.data.lock().unwrap().msg = None;
                p.msg_at = None;
                redraw = true;
            }
            (None, Some(_)) => p.msg_at = None,
            _ => {}
        }
        if let Some(v) = p.pending {
            if p.last_send.elapsed() >= Duration::from_millis(80) {
                p.pending = None;
                p.last_send = Instant::now();
                Self::send_level(p.kind, v);
            }
        }
        if redraw {
            p.window.request_redraw();
        }
        true
    }

    fn send_level(kind: PopKind, pct: u8) {
        std::thread::spawn(move || match kind {
            PopKind::Volume => sys::set_volume(pct),
            _ => sys::set_brightness(pct),
        });
    }

    fn pop_event(&mut self, ev: WindowEvent) {
        let Some(p) = &mut self.pop else { return };
        match ev {
            WindowEvent::RedrawRequested => {
                let snap = p.data.lock().unwrap().clone();
                let k = p.window.scale_factor() as f32;
                let (cw, ch) = ((POP_W * k).ceil() as i32, (POP_MAX_H * k).ceil() as i32);
                let _ = p.gfx.begin(cw, ch);
                let h = linux_pop::paint(&p.gfx, p.kind, &snap, &p.state, &mut p.hits, self.clock_24h);
                let _ = p.gfx.end();
                if (h - p.h).abs() > 0.5 {
                    p.h = h;
                    let _ = p.window.request_inner_size(LogicalSize::new(POP_W as f64, h as f64));
                }
                blit(&mut p.surface, &p.gfx, cw as u32, (h * k).ceil() as u32);
            }
            // the window manager may place a new window itself: put it back (a few tries)
            WindowEvent::Moved(pos) => {
                if pos != p.home && p.moves < 3 {
                    p.moves += 1;
                    p.window.set_outer_position(p.home);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                p.entered = true;
                let k = p.window.scale_factor();
                p.cursor = (position.x as f32 / k as f32, position.y as f32 / k as f32);
                if let Some(r) = p.drag {
                    Self::slide(p, r);
                }
                let hover = p.hits.iter().position(|(r, _)| r.contains(p.cursor.0, p.cursor.1));
                if hover != p.state.hover {
                    p.state.hover = hover;
                    p.window.request_redraw();
                }
            }
            WindowEvent::CursorEntered { .. } => p.entered = true,
            WindowEvent::CursorLeft { .. } if p.entered && p.drag.is_none() => self.pop = None,
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                p.drag = None;
                if let Some(v) = p.pending.take() {
                    Self::send_level(p.kind, v);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                let Some(act) = p.hits.iter().find(|(r, _)| r.contains(p.cursor.0, p.cursor.1)).map(|(_, a)| *a) else { return };
                self.pop_act(act);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let Some(pw) = &mut p.state.pw else {
                    if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
                        self.pop = None;
                    }
                    return;
                };
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => p.state.pw = None,
                    Key::Named(NamedKey::Enter) => self.pop_act(PopAct::PwJoin),
                    Key::Named(NamedKey::Backspace) => {
                        pw.buf.pop();
                    }
                    _ => {
                        if let Some(t) = event.text.as_deref().filter(|t| t.chars().all(|c| !c.is_control())) {
                            if pw.buf.chars().count() < 63 {
                                pw.buf.push_str(t);
                            }
                        }
                    }
                }
                if let Some(p) = &self.pop {
                    p.window.request_redraw();
                }
            }
            _ => {}
        }
    }

    /// Move a slider to the pointer; the value goes to the system at most every 80 ms.
    fn slide(p: &mut PopWin, r: Rect) {
        let pct = (((p.cursor.0 - r.x) / r.w).clamp(0.0, 1.0) * 100.0).round() as u8;
        {
            let mut d = p.data.lock().unwrap();
            match p.kind {
                PopKind::Volume => d.volume = Some((pct, d.volume.is_some_and(|v| v.1) && pct == 0)),
                _ => d.brightness = Some(pct),
            }
        }
        p.pending = Some(pct);
        p.window.request_redraw();
    }

    fn pop_act(&mut self, act: PopAct) {
        let Some(p) = &mut self.pop else { return };
        let (kind, data) = (p.kind, p.data.clone());
        let again = |f: Box<dyn FnOnce() + Send>| {
            let d = data.clone();
            std::thread::spawn(move || {
                f();
                refresh_pop(kind, &d);
            });
        };
        match act {
            PopAct::Toggle(0) => {
                let on = !data.lock().unwrap().wifi_on;
                {
                    let mut d = data.lock().unwrap();
                    d.wifi_on = on;
                    d.wifi_loaded = !on;
                    d.nets.clear();
                }
                again(Box::new(move || sys::set_wifi(on)));
            }
            PopAct::Toggle(_) => {
                let on = !data.lock().unwrap().bt_on;
                {
                    let mut d = data.lock().unwrap();
                    d.bt_on = on;
                    d.bt_loaded = !on;
                    d.devs.clear();
                }
                again(Box::new(move || sys::set_bt(on)));
            }
            PopAct::Mute => {
                {
                    let mut d = data.lock().unwrap();
                    if let Some(v) = &mut d.volume {
                        v.1 = !v.1;
                    }
                }
                again(Box::new(sys::toggle_mute));
            }
            PopAct::Vol(r) | PopAct::Bri(r) => {
                p.drag = Some(r);
                Self::slide(p, r);
            }
            PopAct::Wifi(i) => {
                let Some(n) = data.lock().unwrap().nets.get(i).cloned() else { return };
                if n.active || data.lock().unwrap().busy.is_some() {
                    return;
                }
                if n.secure && !n.known {
                    p.state.pw = Some(Pw { ssid: n.ssid, ..Pw::default() });
                } else {
                    Self::join(&data, n.ssid, None);
                }
            }
            PopAct::PwShow => {
                if let Some(pw) = &mut p.state.pw {
                    pw.show = !pw.show;
                }
            }
            PopAct::PwCancel => p.state.pw = None,
            PopAct::PwJoin => {
                if let Some(pw) = p.state.pw.take() {
                    if !pw.buf.is_empty() {
                        Self::join(&data, pw.ssid, Some(pw.buf));
                    }
                }
            }
            PopAct::Bt(i) => {
                let Some(dev) = data.lock().unwrap().devs.get(i).cloned() else { return };
                if data.lock().unwrap().busy.is_some() {
                    return;
                }
                {
                    let mut d = data.lock().unwrap();
                    d.busy = Some(dev.name.clone());
                    d.msg = None;
                }
                let d2 = data.clone();
                std::thread::spawn(move || {
                    let ok = sys::bt_connect(&dev.mac, !dev.connected);
                    {
                        let mut d = d2.lock().unwrap();
                        d.busy = None;
                        d.msg = Some((
                            match (ok, dev.connected) {
                                (true, false) => format!("Connected to {}", dev.name),
                                (true, true) => format!("Disconnected from {}", dev.name),
                                _ => format!("Couldn't connect to {}", dev.name),
                            },
                            ok,
                        ));
                    }
                    refresh_pop(PopKind::Bluetooth, &d2);
                });
            }
            PopAct::Link(page) => {
                sys::open_settings(page);
                self.pop = None;
                return;
            }
            PopAct::CalMonth(n) => p.state.cal_off += n,
            PopAct::CalToday => p.state.cal_off = 0,
        }
        if let Some(p) = &self.pop {
            p.window.request_redraw();
        }
    }

    /// Join a Wi-Fi network on a worker thread, showing a spinner on its row meanwhile.
    fn join(data: &Arc<Mutex<PopData>>, ssid: String, password: Option<String>) {
        {
            let mut d = data.lock().unwrap();
            d.busy = Some(ssid.clone());
            d.msg = None;
        }
        let d2 = data.clone();
        std::thread::spawn(move || {
            let ok = sys::wifi_connect(&ssid, password.as_deref());
            {
                let mut d = d2.lock().unwrap();
                d.busy = None;
                d.msg = Some((if ok { format!("Connected to {ssid}") } else { format!("Couldn't connect to {ssid}") }, ok));
            }
            refresh_pop(PopKind::Wifi, &d2);
        });
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
        self.bar = BarWin::new(el);
        let Ok(win) = el.create_window(attrs) else { return };
        let win = Rc::new(win);
        if let Ok(ctx) = softbuffer::Context::new(win.clone()) {
            self.surface = softbuffer::Surface::new(&ctx, win.clone()).ok();
        }
        self.window = Some(win);
        self.apply_size();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, ev: WindowEvent) {
        if self.bar.as_ref().is_some_and(|b| b.window.id() == id) {
            self.bar_event(el, ev);
            return;
        }
        if self.pop.as_ref().is_some_and(|p| p.window.id() == id) {
            self.pop_event(ev);
            return;
        }
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
            // the window manager may place a new window itself: put the notch back
            WindowEvent::Moved(pos) => {
                if let (Some(home), Some(win)) = (self.home(), &self.window) {
                    if pos != home && pos.y != home.y {
                        win.set_outer_position(home);
                    }
                }
            }
            WindowEvent::HoveredFile(_) => self.shelf_enter(),
            WindowEvent::HoveredFileCancelled => self.shelf_leave(),
            WindowEvent::DroppedFile(path) => {
                self.shelf_auto_open = false;
                self.panel.shelf_hover = false;
                self.panel.shelf_add([path.to_string_lossy().into_owned()]);
                self.redraw();
            }
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
        // ponytail: X11 has no cheap clipboard-change event here, so look once a second
        // (event-driven XFixes selection notify would be the zero-idle upgrade)
        let clip_changed = self.panel.poll_clipboard();
        let media_on = self.expanded && self.panel.page == MEDIA;
        // ponytail: polled (1 s on the Media tab, 2 s otherwise); subscribing to the
        // MPRIS PropertiesChanged signal would make this event-driven
        let every = Duration::from_millis(if media_on { 900 } else { 2000 });
        if self.panel.poll_media(self.gfx.as_ref(), every) {
            self.redraw();
        }
        let (_, _, _, _, hh, mm, ..) = sys::local_time();
        let minute = hh * 60 + mm;
        if minute != self.last_minute {
            self.last_minute = minute;
            self.redraw();
        }
        let pw = self.panel.pill_w();
        if pw != self.last_pill_w {
            self.last_pill_w = pw;
            if !self.expanded {
                self.apply_size();
            }
        }
        self.bar_tick(minute);
        let pop_open = self.pop_tick();
        if !self.expanded && self.panel.media_playing() {
            self.redraw(); // equaliser bars
        }
        if media_on {
            // the progress bar moves while a track plays, and the player may change by itself
            if self.panel.media_playing() {
                self.redraw();
            }
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(500)));
        } else if self.panel.animating() {
            self.panel.tick();
            self.redraw();
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(500)));
        } else {
            if clip_changed && self.expanded && self.panel.page == CLIPBOARD {
                self.redraw();
            }
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(1000)));
        }
        if pop_open {
            // spinners and slow connects: look again soon
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(120)));
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
        shelf_auto_open: false,
        clock_24h: store::load_config().clock_24h,
        last_pill_w: PILL_W,
        last_minute: 99,
        bar: None,
        pop: None,
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
        p.track = Some(Track { title: "Song of the Day".into(), artist: "The Band".into(), app: "Player".into(), playing: true, can_seek: true, can_prev: true, can_next: true, pos_s: 83.0, dur_s: 215.0, ..Track::default() });
        p.shelf = vec!["/etc".into(), "/etc/hostname".into(), "/home/akhil/report-final-v2.pdf".into()];
        p.pins = vec!["ssh dev@example.com".into()];
        p.clips = vec![
            "https://example.com/some/long/link/to/read/later".into(),
            "Meeting at 11:00 in room 4".into(),
            "cargo build --release".into(),
            "/home/akhil/notes.txt".into(),
            "akhil@example.com".into(),
            "one more".into(),
        ];
        for page in [MEDIA, CONTROLS, APPS, CLIPBOARD, SHELF, TODAY, STATS, POWER] {
            p.page = page;
            p.hover = Some(Act::Tab(0));
            let _ = g.begin(EXP_W as i32, EXP_H as i32);
            p.draw(&g, &mut Vec::new());
            let _ = g.end();
            g.save_png(&format!("{dir}/page{page}.png"));
        }
        // the collapsed notch in each state
        for (name, track, timer) in [("pill_clock", false, false), ("pill_track", true, false), ("pill_timer", false, true)] {
            let mut q = Panel::new();
            q.track = track.then(|| Track { title: "Song of the Day".into(), artist: "The Band".into(), playing: true, ..Track::default() });
            if timer {
                q.toggle_timer();
            }
            let w = q.pill_w();
            let _ = g.begin(w as i32, PILL_H as i32);
            q.draw_pill(&g, w, PILL_H, false);
            let _ = g.end();
            g.save_png(&format!("{dir}/{name}.png"));
        }
        // the top bar with sample data
        let d = BarData { app: "Firefox".into(), wifi_on: Some(true), wifi: Some("Home Network".into()), bt: Some(true), volume: Some((65, false)), brightness: Some(70), battery: Some((82, false)) };
        let _ = g.begin(EXP_W as i32, BAR_H as i32);
        paint_bar(&g, EXP_W, &d, Some(BarAct::Volume), false, &mut Vec::new());
        let _ = g.end();
        g.save_png(&format!("{dir}/bar.png"));
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

    /// Needs an X11 display and xclip: copy from another process, expect it to be recorded.
    #[test]
    #[ignore]
    fn clipboard_is_recorded() {
        sandbox();
        let mut p = Panel::new();
        let mut xclip = std::process::Command::new("xclip")
            .args(["-selection", "clipboard", "-i"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("xclip");
        use std::io::Write;
        xclip.stdin.take().unwrap().write_all(b"copied elsewhere").unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert!(p.poll_clipboard());
        assert_eq!(p.clips[0], "copied elsewhere");
        assert!(!p.poll_clipboard(), "the same text is not recorded twice");
        // pasting one of our own entries does not re-record it
        assert!(p.act(Act::ClipItem(0)));
        assert!(!p.poll_clipboard());
        let _ = xclip.kill();
    }

    #[test]
    fn shelf_keeps_files_once_up_to_the_limit() {
        sandbox();
        let mut p = Panel::new();
        p.shelf.clear();
        p.shelf_add((0..20).map(|i| format!("/tmp/f{}", i % 15)));
        assert_eq!(p.shelf.len(), SHELF_MAX);
        p.shelf_add(["/tmp/f0".to_string()]);
        assert_eq!(p.shelf.len(), SHELF_MAX, "no duplicates");
        // a file that has vanished is dropped when clicked, not opened
        p.shelf = vec!["/definitely/not/here".into()];
        assert!(!p.act(Act::ShelfItem(0)));
        assert!(p.shelf.is_empty());
    }

    #[test]
    fn cover_art_decodes_with_its_average_colour() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let g = Gfx::new(10, 10, 1.0).unwrap();
        let (_, col) = g.decode_image(&png).unwrap();
        assert!(col.0 > 0.99 && col.1 < 0.01 && col.2 < 0.01);
        assert!(g.decode_image(b"not an image").is_err());
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
