//! Application state, animation and input handling. All of this runs on the
//! single UI thread, driven purely by window messages and a few short-lived
//! timers. When nothing is animating the process sits in GetMessage().

use std::sync::Arc;
use std::time::Instant;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Diagnostics::Debug::{Beep, MessageBeep};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::*;
use crate::media::{self, Cmd, MediaState};
use crate::radio::{self, Kind, Radios, Req};
use crate::store::{self, AppEntry, Config, Todo};
use crate::sys;

// ------------------------------------------------------------------ layout

pub const EXP_W: f32 = 600.0;
pub const EXP_H: f32 = 250.0;
pub const COL_H: f32 = 34.0;
pub const COL_IDLE_W: f32 = 128.0;
pub const COL_MEDIA_W: f32 = 296.0;
pub const COL_TIMER_W: f32 = 132.0;
pub const MARGIN: f32 = 24.0; // room for spring overshoot
pub const MAX_TODO: usize = 5;
pub const CLIP_ROWS: usize = 5;

const T_ANIM: usize = 1;
const T_SEC: usize = 2;
const T_MIN: usize = 3;
const T_LEAVE: usize = 4;
const T_HOVER: usize = 5;
const T_PASTE: usize = 6;
const T_EDGE: usize = 8;
const T_PEEK: usize = 9;
const T_FSCHK: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Media,
    Controls,
    Clipboard,
    Apps,
    Tasks,
}

pub const PAGES: [Page; 5] = [
    Page::Media,
    Page::Controls,
    Page::Clipboard,
    Page::Apps,
    Page::Tasks,
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    Collapse,
    Pin,
    Tab(Page),
    Prev,
    PlayPause,
    Next,
    Seek,
    VolMute,
    VolSlider,
    BriSlider,
    Quick(u8),
    ClipItem(usize),
    ClipClear,
    Launch(usize),
    TimerPreset(u32),
    TimerToggle,
    TimerReset,
    TodoToggle(usize),
    TodoDel(usize),
    TodoAdd,
}

#[derive(Clone, Copy)]
pub struct Hit {
    pub r: Rect,
    pub act: Act,
}

#[derive(Clone, Copy)]
enum Drag {
    Seek(Rect),
    Vol(Rect),
    Bri(Rect),
}

// ------------------------------------------------------------------ spring

#[derive(Clone, Copy)]
pub struct Spring {
    pub x: f32,
    v: f32,
}

impl Spring {
    fn new(x: f32) -> Self {
        Spring { x, v: 0.0 }
    }
    fn step(&mut self, target: f32, dt: f32) {
        const K: f32 = 520.0; // stiffness
        const ZETA: f32 = 0.82; // slightly under-damped: a hint of overshoot
        let c = 2.0 * ZETA * K.sqrt();
        let a = K * (target - self.x) - c * self.v;
        self.v += a * dt;
        self.x += self.v * dt;
    }
    fn settled(&self, target: f32) -> bool {
        (target - self.x).abs() < 0.25 && self.v.abs() < 3.0
    }
}

// --------------------------------------------------------------------- app

pub struct App {
    pub hwnd: HWND,
    pub gfx: Gfx,
    pub scale: f32,
    pub mon: RECT,
    pub cfg: Config,

    // window animation
    pub sw: Spring,
    pub sh: Spring,
    pub expanded: bool,
    pub pinned: bool,
    pub hidden: bool,
    pub tucked: bool,
    /// A full-screen app (or tuck) is hiding the bar and notch, ignoring any peek.
    fs_hidden: bool,
    /// The bar/notch are temporarily shown over a full-screen video/browser.
    peek: bool,
    fs_peekable: bool,
    pub bar: Option<crate::bar::Bar>,
    pub pop: Option<crate::pop::Pop>,
    pub page: Page,
    pub eq_phase: f32,
    last_tick: Instant,
    anim_ms: u32,
    sec_on: bool,

    // input
    pub hits: Vec<Hit>,
    pub hot: Option<Act>,
    drag: Option<Drag>,
    pub seek_frac: Option<f32>,
    tracking: bool,

    // media
    pub media: media::Handle,
    pub mstate: MediaState,
    pub art: Option<ID2D1Bitmap>,
    art_bytes: Option<Arc<Vec<u8>>>,
    art_gen: u64,
    pub accent: Color,

    // controls
    pub volume: f32,
    pub muted: bool,
    pub has_volume: bool,
    pub radio: radio::Handle,
    pub radios: Radios,
    pub brightness: Option<u32>,
    bri_asked: bool,

    // clipboard
    pub clips: Vec<String>,
    pub clip_scroll: usize,
    ignore_clip: Option<String>,

    // launcher
    pub apps: Vec<AppEntry>,
    pub app_icons: Vec<Option<ID2D1Bitmap>>,

    // tasks
    pub timer_total_s: u32,
    timer_left_ms: i64,
    timer_start: Option<Instant>,
    pub todos: Vec<Todo>,
    pub entering: Option<Vec<u16>>,
    prev_fg: HWND,
    /// Bluetooth device being connected/disconnected: (address, connect?, name).
    pub bt_busy: Option<(u64, bool, String)>,
    /// Frame counter for the connecting animations.
    pub link_phase: u32,
    /// Result of the last Wi-Fi/Bluetooth attempt, shown briefly: (which, text, ok).
    pub link_msg: Option<(crate::bar::BarAct, String, bool)>,
    /// Wi-Fi password being typed in the dropdown.
    pub pw: Option<crate::pop::PwEntry>,
    pub pw_prev_fg: HWND,
    /// Set when the focus timer finishes: the pill pulses and shows "Time's up".
    alert_until: Option<Instant>,
}

pub const DEFAULT_ACCENT: Color = Color(0.40, 0.62, 1.0, 1.0);

impl App {
    pub fn new(hwnd: HWND, scale: f32) -> Result<App> {
        let max_w = ((EXP_W + MARGIN) * scale).ceil() as i32 + 2;
        let max_h = ((EXP_H + MARGIN) * scale).ceil() as i32 + 2;
        let gfx = Gfx::new(max_w, max_h, scale)?;
        let mut a = App {
            hwnd,
            gfx,
            scale,
            mon: RECT::default(),
            cfg: store::load_config(),
            sw: Spring::new(COL_IDLE_W),
            sh: Spring::new(COL_H), // corrected in new() once cfg is known
            expanded: false,
            pinned: false,
            hidden: false,
            tucked: false,
            fs_hidden: false,
            peek: false,
            fs_peekable: false,
            bar: None,
            pop: None,
            page: Page::Media,
            eq_phase: 0.0,
            last_tick: Instant::now(),
            anim_ms: 0,
            sec_on: false,
            hits: Vec::new(),
            hot: None,
            drag: None,
            seek_frac: None,
            tracking: false,
            media: media::start(hwnd.0 as isize),
            mstate: MediaState::default(),
            art: None,
            art_bytes: None,
            art_gen: 0,
            accent: DEFAULT_ACCENT,
            volume: 0.0,
            muted: false,
            has_volume: false,
            radio: radio::start(hwnd.0 as isize),
            radios: Radios::default(),
            brightness: None,
            bri_asked: false,
            clips: Vec::new(),
            clip_scroll: 0,
            ignore_clip: None,
            apps: store::load_apps(),
            app_icons: Vec::new(),
            timer_total_s: 25 * 60,
            timer_left_ms: 25 * 60 * 1000,
            timer_start: None,
            todos: store::load_todos(),
            entering: None,
            prev_fg: HWND::default(),
            bt_busy: None,
            link_phase: 0,
            link_msg: None,
            pw: None,
            pw_prev_fg: HWND::default(),
            alert_until: None,
        };
        a.update_monitor();
        a.load_app_icons();
        let (w, h) = a.target_size();
        a.sw = Spring::new(w);
        a.sh = Spring::new(h);
        Ok(a)
    }

    // ------------------------------------------------------------ helpers --

    pub(crate) fn set_timer(&self, id: usize, ms: u32) {
        unsafe {
            SetTimer(Some(self.hwnd), id, ms, None);
        }
    }
    pub(crate) fn kill_timer(&self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), id);
        }
    }

    pub fn update_monitor(&mut self) {
        unsafe {
            let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(mon, &mut mi).as_bool() {
                self.mon = mi.rcMonitor;
            }
        }
    }

    /// Rebuild all GPU-side resources (new DPI).
    pub fn rebuild_gfx(&mut self, scale: f32) {
        let max_w = ((EXP_W + MARGIN) * scale).ceil() as i32 + 2;
        let max_h = ((EXP_H + MARGIN) * scale).ceil() as i32 + 2;
        if let Ok(g) = Gfx::new(max_w, max_h, scale) {
            self.gfx = g;
            self.scale = scale;
            self.load_art();
            self.load_app_icons();
        }
        self.update_reserve();
    }

    pub fn has_media(&self) -> bool {
        self.mstate.has_track()
    }
    pub fn timer_running(&self) -> bool {
        self.timer_start.is_some()
    }
    pub fn timer_remaining_ms(&self) -> i64 {
        match self.timer_start {
            Some(t) => self.timer_left_ms - t.elapsed().as_millis() as i64,
            None => self.timer_left_ms,
        }
        .max(0)
    }

    fn eq_active(&self) -> bool {
        !self.expanded
            && !self.hidden
            && self.cfg.eq_animation
            && self.mstate.playing
            && self.mstate.has_track()
    }

    pub fn alerting(&self) -> bool {
        self.alert_until.is_some_and(|t| Instant::now() < t)
    }

    /// Height of the collapsed pill: flush with the top bar when it is on.
    pub fn col_h(&self) -> f32 {
        if self.cfg.top_bar {
            crate::bar::BAR_H
        } else {
            COL_H
        }
    }

    /// Bottom-corner radius of the collapsed pill.
    pub fn col_rad(&self) -> f32 {
        if self.cfg.top_bar {
            12.0
        } else {
            17.0
        }
    }

    fn target_size(&self) -> (f32, f32) {
        if self.expanded {
            (EXP_W, EXP_H)
        } else if self.alerting() {
            (190.0, self.col_h())
        } else if self.has_media() {
            let extra = if self.timer_running() { 58.0 } else { 0.0 };
            (COL_MEDIA_W + extra, self.col_h())
        } else if self.timer_running() {
            (COL_TIMER_W, self.col_h())
        } else {
            (COL_IDLE_W, self.col_h())
        }
    }

    /// 0 = collapsed pill, 1 = fully expanded.
    pub fn t_expand(&self) -> f32 {
        ((self.sh.x - self.col_h()) / (EXP_H - self.col_h())).clamp(0.0, 1.0)
    }

    fn retarget(&mut self) {
        self.last_tick = Instant::now();
        self.set_anim(16);
    }

    fn set_anim(&mut self, ms: u32) {
        if self.anim_ms == ms {
            return;
        }
        if ms == 0 {
            self.kill_timer(T_ANIM);
        } else {
            self.set_timer(T_ANIM, ms);
        }
        self.anim_ms = ms;
    }

    /// Start/stop the 1 Hz timer that is only needed for a moving progress bar,
    /// the volume read-out and a running focus timer.
    pub fn sync_timers(&mut self) {
        let need = !self.hidden
            && ((self.expanded && (self.mstate.playing || self.page == Page::Controls))
                || self.timer_running());
        if need && !self.sec_on {
            self.set_timer(T_SEC, 1000);
            self.sec_on = true;
        } else if !need && self.sec_on {
            self.kill_timer(T_SEC);
            self.sec_on = false;
        }
        // keep the equaliser running (or not)
        if self.eq_active() && self.anim_ms == 0 {
            self.last_tick = Instant::now();
            self.set_anim(50);
        }
    }

    /// One-shot timer that fires at the next minute boundary (for the clock).
    pub fn arm_minute_timer(&self) {
        let (_, _, _, _, _, _, s, ms) = sys::local_time();
        let wait = (60 - s) * 1000 - ms + 30;
        self.set_timer(T_MIN, wait.max(100));
    }

    // -------------------------------------------------------------- render --

    pub fn render(&mut self) {
        if self.hidden {
            return;
        }
        let w = self.sw.x.clamp(60.0, EXP_W + MARGIN);
        let h = self.sh.x.clamp(20.0, EXP_H + MARGIN);
        let pw = (w * self.scale).round() as i32;
        let ph = (h * self.scale).round() as i32;
        if self.gfx.begin(pw, ph).is_err() {
            return;
        }
        let mut hits = std::mem::take(&mut self.hits);
        hits.clear();
        crate::ui::draw(self, w, h, &mut hits);
        self.hits = hits;
        let _ = self.gfx.end();
        let mon_w = self.mon.right - self.mon.left;
        let x = self.mon.left + (mon_w - pw) / 2;
        let y = self.mon.top;
        let _ = self.gfx.present(self.hwnd, x, y, pw, ph);
    }

    // ----------------------------------------------------------- animation --

    fn on_anim_tick(&mut self) {
        let now = Instant::now();
        let mut dt = (now - self.last_tick).as_secs_f32().min(0.05);
        self.last_tick = now;
        if self.alert_until.is_some() && !self.alerting() {
            self.alert_until = None; // alert expired: shrink back
        }
        if self.eq_active() || self.alerting() {
            self.eq_phase += dt * 9.0;
        }
        let (tw, th) = self.target_size();
        while dt > 0.0 {
            let h = dt.min(1.0 / 240.0);
            self.sw.step(tw, h);
            self.sh.step(th, h);
            dt -= h;
        }
        let moving = !(self.sw.settled(tw) && self.sh.settled(th));
        if !moving {
            self.sw = Spring::new(tw);
            self.sh = Spring::new(th);
        }
        self.render();
        let next = if moving {
            16
        } else if self.eq_active() || self.alerting() {
            50
        } else {
            0
        };
        self.set_anim(next);
    }

    pub fn on_timer(&mut self, id: usize) {
        match id {
            T_ANIM => self.on_anim_tick(),
            T_EDGE => {
                self.kill_timer(T_EDGE);
                if self.fs_peekable && !self.peek && self.cursor_in_edge_zone() {
                    self.peek = true;
                    self.on_foreground();
                    self.set_timer(T_PEEK, 450);
                }
            }
            T_PEEK => self.on_peek_tick(),
            T_FSCHK => {
                self.kill_timer(T_FSCHK);
                self.on_foreground();
            }
            crate::pop::T_POPLEAVE => self.on_pop_leave_timer(),
            crate::pop::T_LINK => self.on_link_tick(),
            crate::pop::T_TIP => self.on_tip_timer(),
            crate::pop::T_LINKMSG => self.on_linkmsg_timer(),
            T_SEC => {
                if self.timer_running() && self.timer_remaining_ms() <= 0 {
                    self.timer_done();
                }
                if self.expanded && self.page == Page::Controls && self.drag.is_none() {
                    self.read_volume();
                    let _ = self.radio.tx.send(Req::Refresh);
                }
                if !self.hidden {
                    self.render();
                }
            }
            T_MIN => {
                self.arm_minute_timer();
                if self.bar_visible() {
                    self.refresh_bar_data();
                    self.render_bar();
                }
                if !self.expanded && !self.has_media() && !self.timer_running() {
                    self.render();
                }
            }
            T_LEAVE => {
                self.kill_timer(T_LEAVE);
                if self.expanded && !self.pinned && self.entering.is_none() && self.drag.is_none() {
                    self.set_expanded(false);
                }
            }
            T_HOVER => {
                self.kill_timer(T_HOVER);
                if !self.expanded && self.cursor_inside() {
                    self.set_expanded(true);
                }
            }
            T_PASTE => {
                self.kill_timer(T_PASTE);
                sys::send_ctrl_v();
            }
            _ => {}
        }
    }

    fn cursor_inside(&self) -> bool {
        unsafe {
            let mut p = POINT::default();
            let mut r = RECT::default();
            GetCursorPos(&mut p).is_ok()
                && GetWindowRect(self.hwnd, &mut r).is_ok()
                && p.x >= r.left
                && p.x < r.right
                && p.y >= r.top
                && p.y < r.bottom
        }
    }

    // ------------------------------------------------------ expand/collapse --

    pub fn set_expanded(&mut self, on: bool) {
        if self.expanded == on {
            return;
        }
        self.expanded = on;
        if on {
            self.alert_until = None;
        }
        self.kill_timer(T_LEAVE);
        self.kill_timer(T_HOVER);
        if on {
            if self.page == Page::Controls {
                self.refresh_controls();
            }
        } else {
            self.finish_entry(false, false);
            self.drag = None;
            self.seek_frac = None;
            self.hot = None;
        }
        self.retarget();
        self.sync_timers();
    }

    /// Hide the notch completely (Ctrl+Alt+H / tray) or bring it back.
    pub fn toggle_tucked(&mut self) {
        self.tucked = !self.tucked;
        if self.tucked {
            self.pinned = false;
            self.set_expanded(false);
        }
        self.on_foreground();
    }

    /// Turn the "reserve screen space" strip on or off and remember the choice.
    pub fn toggle_top_bar(&mut self) {
        self.cfg.top_bar = !self.cfg.top_bar;
        store::save_config(&self.cfg);
        self.sync_bar();
        self.update_reserve();
        if !self.expanded {
            let (w, h) = self.target_size();
            self.sw = Spring::new(w);
            self.sh = Spring::new(h);
        }
        self.render();
    }

    pub fn toggle_reserve(&mut self) {
        self.cfg.reserve_space = !self.cfg.reserve_space;
        store::save_config(&self.cfg);
        self.update_reserve();
    }

    pub fn update_reserve(&self) {
        let on = (self.cfg.reserve_space || self.cfg.top_bar) && !self.fs_hidden;
        let strip = if self.cfg.top_bar { crate::bar::BAR_H } else { COL_H + 4.0 };
        let h = if on { (strip * self.scale).round() as i32 } else { 0 };
        sys::reserve_top(self.hwnd, &self.mon, h);
    }

    pub fn toggle_pinned_open(&mut self) {
        if self.tucked {
            self.tucked = false;
            self.on_foreground();
        }
        if self.expanded {
            self.pinned = false;
            self.set_expanded(false);
        } else {
            self.pinned = true;
            self.set_expanded(true);
        }
    }

    // --------------------------------------------------------------- input --

    fn hit_at(&self, x: f32, y: f32) -> Option<Act> {
        self.hits.iter().rev().find_map(|h| {
            let r = match h.act {
                Act::Seek | Act::VolSlider | Act::BriSlider => {
                    Rect::new(h.r.x - 8.0, h.r.y, h.r.w + 16.0, h.r.h)
                }
                _ => h.r,
            };
            r.contains(x, y).then_some(h.act)
        })
    }

    fn rect_of(&self, act: Act) -> Option<Rect> {
        self.hits.iter().rev().find(|h| h.act == act).map(|h| h.r)
    }

    pub fn on_mouse_move(&mut self, px: i32, py: i32) {
        let (x, y) = (px as f32 / self.scale, py as f32 / self.scale);
        self.kill_timer(T_LEAVE);
        if !self.tracking {
            unsafe {
                let mut t = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: self.hwnd,
                    dwHoverTime: 0,
                };
                if TrackMouseEvent(&mut t).is_ok() {
                    self.tracking = true;
                }
            }
        }
        if !self.expanded {
            if self.cfg.hover_expand {
                self.set_timer(T_HOVER, 220);
            }
            return;
        }
        if self.drag.is_some() {
            self.apply_drag(x);
            self.render();
            return;
        }
        let hot = self.hit_at(x, y);
        if hot != self.hot {
            self.hot = hot;
            // Only repaint directly when no animation is already doing it.
            if self.anim_ms == 0 {
                self.render();
            }
        }
    }

    pub fn on_mouse_leave(&mut self) {
        self.tracking = false;
        self.kill_timer(T_HOVER);
        if self.hot.is_some() {
            self.hot = None;
            if self.expanded && self.anim_ms == 0 {
                self.render();
            }
        }
        if self.expanded && !self.pinned && self.entering.is_none() && self.drag.is_none() {
            self.set_timer(T_LEAVE, 450);
        }
    }

    pub fn on_lbutton_down(&mut self, px: i32, py: i32) {
        let (x, y) = (px as f32 / self.scale, py as f32 / self.scale);
        if !self.expanded {
            self.set_expanded(true);
            return;
        }
        if let Some(act) = self.hit_at(x, y) {
            self.do_act(act, x);
        } else if self.entering.is_some() {
            // clicking elsewhere inside the notch keeps the text box open
        }
        if self.anim_ms == 0 {
            self.render();
        }
    }

    pub fn on_lbutton_up(&mut self) {
        let Some(d) = self.drag.take() else { return };
        unsafe {
            let _ = ReleaseCapture();
        }
        match d {
            Drag::Seek(_) => {
                if let Some(f) = self.seek_frac.take() {
                    let (_, dur) = self.mstate.progress(sys::filetime_now());
                    if dur > 0.0 {
                        let secs = f as f64 * dur;
                        let _ = self.media.tx.send(Cmd::SeekSecs(secs));
                        // optimistic local update so the bar doesn't jump back
                        self.mstate.pos_ticks = (secs * 1e7) as i64;
                        self.mstate.pos_stamp = sys::filetime_now();
                    }
                }
            }
            Drag::Bri(_) => {
                if let Some(b) = self.brightness {
                    sys::brightness_set(b);
                }
            }
            Drag::Vol(_) => {}
        }
        self.render();
    }

    fn apply_drag(&mut self, x: f32) {
        match self.drag {
            Some(Drag::Seek(r)) => {
                self.seek_frac = Some(((x - r.x) / r.w).clamp(0.0, 1.0));
            }
            Some(Drag::Vol(r)) => {
                let v = ((x - r.x) / r.w).clamp(0.0, 1.0);
                self.volume = v;
                if self.muted {
                    self.muted = false;
                    sys::set_mute(false);
                }
                sys::set_volume(v);
            }
            Some(Drag::Bri(r)) => {
                let v = ((x - r.x) / r.w).clamp(0.0, 1.0);
                self.brightness = Some((v * 100.0).round() as u32);
                sys::brightness_set(v_pct(v));
            }
            None => {}
        }
    }

    fn start_drag(&mut self, d: Drag, x: f32) {
        self.drag = Some(d);
        unsafe {
            SetCapture(self.hwnd);
        }
        self.apply_drag(x);
    }

    pub fn on_wheel(&mut self, delta: i32, py: i32) {
        let y = py as f32 / self.scale;
        if !self.expanded {
            if delta < 0 {
                self.set_expanded(true);
            }
            return;
        }
        if y > EXP_H - 56.0 {
            // over the tab bar: switch pages
            let i = PAGES.iter().position(|p| *p == self.page).unwrap_or(0) as i32;
            let n = (i + if delta < 0 { 1 } else { -1 }).clamp(0, PAGES.len() as i32 - 1);
            self.select_page(PAGES[n as usize]);
        } else if self.page == Page::Clipboard {
            let max = self.clips.len().saturating_sub(CLIP_ROWS);
            if delta < 0 {
                self.clip_scroll = (self.clip_scroll + 1).min(max);
            } else {
                self.clip_scroll = self.clip_scroll.saturating_sub(1);
            }
            self.render();
        }
    }

    pub(crate) fn select_page(&mut self, p: Page) {
        if self.page == p {
            return;
        }
        self.finish_entry(false, false);
        self.page = p;
        if p == Page::Controls {
            self.refresh_controls();
        }
        self.sync_timers();
        self.render();
    }

    // ------------------------------------------------------------- actions --

    pub(crate) fn do_act(&mut self, act: Act, x: f32) {
        match act {
            Act::Collapse => {
                self.pinned = false;
                self.set_expanded(false);
            }
            Act::Pin => self.pinned = !self.pinned,
            Act::Tab(p) => self.select_page(p),
            Act::Prev => {
                let _ = self.media.tx.send(Cmd::Prev);
            }
            Act::PlayPause => {
                let _ = self.media.tx.send(Cmd::PlayPause);
            }
            Act::Next => {
                let _ = self.media.tx.send(Cmd::Next);
            }
            Act::Seek => {
                if let Some(r) = self.rect_of(Act::Seek) {
                    self.start_drag(Drag::Seek(r), x);
                }
            }
            Act::VolSlider => {
                if let Some(r) = self.rect_of(Act::VolSlider) {
                    self.start_drag(Drag::Vol(r), x);
                }
            }
            Act::BriSlider => {
                if self.brightness.is_some() {
                    if let Some(r) = self.rect_of(Act::BriSlider) {
                        self.start_drag(Drag::Bri(r), x);
                    }
                }
            }
            Act::VolMute => {
                self.muted = !self.muted;
                sys::set_mute(self.muted);
            }
            Act::Quick(i) => match i {
                0 => match self.radios.wifi {
                    Some(on) => {
                        self.radios.wifi = Some(!on); // optimistic; the worker confirms
                        let _ = self.radio.tx.send(Req::Toggle(Kind::Wifi));
                    }
                    None => sys::open("ms-settings:network-wifi", ""),
                },
                1 => match self.radios.bluetooth {
                    Some(on) => {
                        self.radios.bluetooth = Some(!on);
                        let _ = self.radio.tx.send(Req::Toggle(Kind::Bluetooth));
                    }
                    None => sys::open("ms-settings:bluetooth", ""),
                },
                2 => {
                    self.set_expanded(false);
                    sys::open("ms-screenclip:", "");
                }
                3 => sys::lock_pc(),
                _ => sys::sleep_pc(),
            },
            Act::ClipItem(i) => {
                if let Some(t) = self.clips.get(i).cloned() {
                    self.ignore_clip = Some(t.clone());
                    if sys::set_clipboard_text(self.hwnd, &t) {
                        self.clips.remove(i);
                        self.clips.insert(0, t);
                        self.clip_scroll = 0;
                        self.pinned = false;
                        self.set_expanded(false);
                        // give the target window a moment to regain input focus
                        self.set_timer(T_PASTE, 170);
                    }
                }
            }
            Act::ClipClear => {
                self.clips.clear();
                self.clip_scroll = 0;
            }
            Act::Launch(i) => {
                if let Some(e) = self.apps.get(i) {
                    sys::open(&e.target, &e.args);
                    self.pinned = false;
                    self.set_expanded(false);
                }
            }
            Act::TimerPreset(mins) => {
                self.timer_total_s = mins * 60;
                self.timer_left_ms = mins as i64 * 60_000;
                self.timer_start = Some(Instant::now());
                self.after_timer_change();
            }
            Act::TimerToggle => {
                if self.timer_running() {
                    self.timer_left_ms = self.timer_remaining_ms();
                    self.timer_start = None;
                } else {
                    if self.timer_left_ms <= 0 {
                        self.timer_left_ms = self.timer_total_s as i64 * 1000;
                    }
                    self.timer_start = Some(Instant::now());
                }
                self.after_timer_change();
            }
            Act::TimerReset => {
                self.timer_start = None;
                self.timer_left_ms = self.timer_total_s as i64 * 1000;
                self.after_timer_change();
            }
            Act::TodoToggle(i) => {
                if let Some(t) = self.todos.get_mut(i) {
                    t.done = !t.done;
                    store::save_todos(&self.todos);
                }
            }
            Act::TodoDel(i) => {
                if i < self.todos.len() {
                    self.todos.remove(i);
                    store::save_todos(&self.todos);
                }
            }
            Act::TodoAdd => self.start_entry(),
        }
    }

    fn after_timer_change(&mut self) {
        self.retarget(); // pill width depends on the timer
        self.sync_timers();
    }

    fn timer_done(&mut self) {
        self.timer_start = None;
        self.timer_left_ms = 0;
        alarm_sound();
        self.page = Page::Tasks; // so a click on the pill lands on the timer
        if !self.expanded {
            // pulse the pill for 30 s (or until it is clicked)
            self.alert_until = Some(Instant::now() + std::time::Duration::from_secs(30));
        }
        self.retarget();
        self.sync_timers();
        self.render();
    }

    // ------------------------------------------------------ text entry (todo) --

    fn make_activatable(&self, on: bool) {
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let flag = WS_EX_NOACTIVATE.0 as isize;
            let new = if on { ex & !flag } else { ex | flag };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, new);
        }
    }

    pub fn is_entering(&self) -> bool {
        self.entering.is_some()
    }

    fn start_entry(&mut self) {
        if self.todos.len() >= MAX_TODO {
            return;
        }
        unsafe {
            self.prev_fg = GetForegroundWindow();
        }
        self.entering = Some(Vec::new());
        self.make_activatable(true);
        unsafe {
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.kill_timer(T_LEAVE);
    }

    /// `commit`: keep the text as a new task. `refocus`: hand focus back to the
    /// window that had it before.
    pub fn finish_entry(&mut self, commit: bool, refocus: bool) {
        let Some(buf) = self.entering.take() else {
            return;
        };
        self.make_activatable(false);
        if commit {
            let s = String::from_utf16_lossy(&buf).trim().to_string();
            if !s.is_empty() && self.todos.len() < MAX_TODO {
                self.todos.push(Todo {
                    done: false,
                    text: s,
                });
                store::save_todos(&self.todos);
            }
        }
        if refocus && !self.prev_fg.0.is_null() {
            unsafe {
                let _ = SetForegroundWindow(self.prev_fg);
            }
        }
        self.prev_fg = HWND::default();
    }

    pub fn on_char(&mut self, c: u16) {
        enum Do {
            Commit,
            Cancel,
            Redraw,
        }
        let d = {
            let Some(buf) = self.entering.as_mut() else {
                return;
            };
            match c {
                0x0D => Do::Commit,
                0x1B => Do::Cancel,
                0x08 => {
                    buf.pop();
                    Do::Redraw
                }
                c if c >= 0x20 && buf.len() < 80 => {
                    buf.push(c);
                    Do::Redraw
                }
                _ => Do::Redraw,
            }
        };
        match d {
            Do::Commit => self.finish_entry(true, true),
            Do::Cancel => self.finish_entry(false, true),
            Do::Redraw => {}
        }
        self.render();
    }

    /// Our window lost focus while typing a task: cancel quietly.
    pub fn on_kill_focus(&mut self) {
        if self.entering.is_some() {
            self.finish_entry(false, false);
            self.render();
        }
    }

    // ------------------------------------------------------------- events ---

    pub fn on_media(&mut self) {
        let st = self.media.state.lock().unwrap().clone();
        if st.art_gen != self.art_gen {
            self.art_gen = st.art_gen;
            self.art_bytes = st.art.clone();
            self.load_art();
        }
        self.mstate = st;
        self.retarget();
        self.sync_timers();
        if self.anim_ms == 0 {
            self.render();
        }
    }

    fn load_art(&mut self) {
        self.art = None;
        self.accent = DEFAULT_ACCENT;
        if let Some(bytes) = self.art_bytes.clone() {
            if let Ok((bmp, col)) = self.gfx.decode_image(&bytes) {
                self.art = Some(bmp);
                self.accent = boost(col);
            }
        }
    }

    fn load_app_icons(&mut self) {
        self.app_icons = self
            .apps
            .iter()
            .map(|e| {
                let path = sys::resolve_path(&e.target)?;
                let icon = sys::file_icon(&path)?;
                let bmp = self.gfx.bitmap_from_hicon(icon).ok();
                unsafe {
                    let _ = DestroyIcon(icon);
                }
                bmp
            })
            .collect();
    }

    pub fn reload_apps(&mut self) {
        self.apps = store::load_apps();
        self.load_app_icons();
        self.render();
    }

    pub fn on_clipboard(&mut self) {
        if sys::clipboard_excluded() {
            return;
        }
        let Some(text) = sys::clipboard_text(self.hwnd) else {
            return;
        };
        if self.ignore_clip.as_deref() == Some(text.as_str()) {
            self.ignore_clip = None;
            return;
        }
        if text.trim().is_empty() {
            return;
        }
        self.clips.retain(|c| *c != text);
        self.clips.insert(0, text);
        self.clips.truncate(20);
        self.clip_scroll = 0;
        if self.expanded && self.page == Page::Clipboard {
            self.render();
        }
    }

    pub fn on_brightness(&mut self, supported: bool, percent: u32) {
        self.brightness = supported.then_some(percent.min(100));
        if self.expanded && self.page == Page::Controls {
            self.render();
        }
        self.render_bar();
        self.on_pop_data();
    }

    /// Ask for the brightness once so the top bar knows whether to show the icon.
    pub fn load_brightness_once(&mut self) {
        if !self.bri_asked {
            self.bri_asked = true;
            sys::brightness_load(self.hwnd.0 as isize);
        }
    }

    fn edge_zone(&self) -> (i32, i32, i32) {
        let cx = (self.mon.left + self.mon.right) / 2;
        let half = (170.0 * self.scale) as i32;
        (cx - half, cx + half, self.mon.top + 1)
    }

    fn cursor_in_edge_zone(&self) -> bool {
        let (l, r, y) = self.edge_zone();
        unsafe {
            let mut p = POINT::default();
            GetCursorPos(&mut p).is_ok() && p.y <= y && p.x >= l && p.x < r
        }
    }

    /// The pointer touched the top edge while a full-screen app is running.
    pub fn on_edge(&mut self) {
        if self.fs_peekable && !self.peek {
            self.set_timer(T_EDGE, 250); // short dwell so a stray flick does nothing
        }
    }

    fn on_peek_tick(&mut self) {
        if !self.peek {
            self.kill_timer(T_PEEK);
            return;
        }
        let busy = self.expanded || self.pop_kind().is_some() || self.entering.is_some();
        let low = unsafe {
            let mut p = POINT::default();
            GetCursorPos(&mut p).is_ok()
                && p.y > self.mon.top + ((crate::bar::BAR_H + 10.0) * self.scale) as i32
        };
        if low && !busy {
            self.peek = false;
            self.kill_timer(T_PEEK);
            self.on_foreground();
        }
    }

    /// The foreground window moved/resized: re-check full screen shortly (debounced).
    pub fn on_fullscreen_check(&mut self) {
        self.set_timer(T_FSCHK, 150);
    }

    pub fn on_foreground(&mut self) {
        // Never hide because of our own window (typing a task makes us the
        // foreground window), and never hide while a task is being typed.
        let fgw = unsafe { GetForegroundWindow() };
        let ours = fgw == self.hwnd || self.pop.as_ref().map(|p| p.hwnd) == Some(fgw);
        let kind = if self.tucked || ours || self.entering.is_some() {
            0
        } else {
            sys::fullscreen_kind(&[self.hwnd, self.bar.as_ref().map(|b| b.hwnd).unwrap_or_default(), self.pop.as_ref().map(|p| p.hwnd).unwrap_or_default()], &self.mon)
        };
        let fs_all = self.tucked || kind != 0;
        self.fs_peekable = !self.tucked && kind == 1;
        if !self.fs_peekable && self.peek {
            self.peek = false;
            self.kill_timer(T_PEEK);
        }
        let fs = fs_all && !(self.peek && self.fs_peekable);
        let fs_changed = fs_all != self.fs_hidden;
        self.fs_hidden = fs_all;
        sys::edge_watch(self.fs_peekable, self.hwnd, self.edge_zone());
        // Clicking into any other window closes an open dropdown / unpinned panel.
        if !ours && self.entering.is_none() {
            self.close_pop();
            if self.expanded && !self.pinned && self.drag.is_none() && !self.cursor_inside() {
                self.set_expanded(false);
            }
        }
        if fs != self.hidden {
            self.hidden = fs;
            unsafe {
                let _ = ShowWindow(self.hwnd, if fs { SW_HIDE } else { SW_SHOWNOACTIVATE });
            }
            if !fs {
                self.render();
            }
            self.sync_timers();
            self.sync_bar();
        }
        if fs_changed {
            self.update_reserve();
        }
        if !fs {
            // some apps grab TOPMOST; take it back (no activation, no repaint)
            self.raise_windows();
            self.on_bar_foreground();
        }
    }

    pub fn on_display_change(&mut self, new_scale: f32) {
        self.update_monitor();
        if (new_scale - self.scale).abs() > 0.01 {
            self.rebuild_gfx(new_scale);
        }
        self.rebuild_bar();
        self.rebuild_pop();
        self.update_reserve();
        self.render();
        self.render_bar();
    }

    // ------------------------------------------------------------ controls --

    pub(crate) fn read_volume(&mut self) {
        match sys::get_volume() {
            Some((v, m)) => {
                self.volume = v;
                self.muted = m;
                self.has_volume = true;
            }
            None => self.has_volume = false,
        }
    }

    pub fn on_radio(&mut self) {
        self.radios = *self.radio.state.lock().unwrap();
        self.render_bar();
        self.on_pop_data();
        if self.expanded && self.page == Page::Controls && self.anim_ms == 0 {
            self.render();
        }
    }

    fn refresh_controls(&mut self) {
        let _ = self.radio.tx.send(Req::Refresh);
        self.read_volume();
        if !self.bri_asked {
            self.bri_asked = true;
            sys::brightness_load(self.hwnd.0 as isize);
        }
    }
}

/// Make the album colour usable as an accent on black (never too dark/dull).
fn v_pct(v: f32) -> u32 {
    (v * 100.0).round() as u32
}

fn boost(c: Color) -> Color {
    let m = c.0.max(c.1).max(c.2);
    if m < 0.02 {
        return DEFAULT_ACCENT;
    }
    let k = 0.92 / m;
    let (r, g, b) = (c.0 * k, c.1 * k, c.2 * k);
    // pull slightly towards white so very muddy colours still read
    let lift = 0.12;
    Color(
        r + (1.0 - r) * lift,
        g + (1.0 - g) * lift,
        b + (1.0 - b) * lift,
        1.0,
    )
}

/// Three clear tones, independent of the Windows sound scheme (which is often
/// muted). Runs on its own thread because Beep() blocks for its duration.
fn alarm_sound() {
    std::thread::spawn(|| unsafe {
        for _ in 0..3 {
            let _ = Beep(1175, 170);
            std::thread::sleep(std::time::Duration::from_millis(90));
        }
        let _ = MessageBeep(MB_ICONEXCLAMATION);
    });
}
