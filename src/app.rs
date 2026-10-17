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
pub const COL_IDLE_W: f32 = 176.0;
pub const COL_MEDIA_W: f32 = 296.0;
pub const COL_TIMER_W: f32 = 132.0;
pub const MARGIN: f32 = 24.0; // room for spring overshoot
pub const MAX_TODO: usize = 5;

/// Rows the app picker shows at once.
pub const PICK_ROWS: usize = 5;

/// Settings pages the Controls tab links to (display, sound, network, power, all settings).
pub const SETTINGS_PAGES: [&str; 5] = [
    "ms-settings:display",
    "ms-settings:sound",
    "ms-settings:network",
    "ms-settings:powersleep",
    "ms-settings:",
];
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
const T_STATS: usize = 18;
const T_GAME: usize = 19;
const T_WATER: usize = 20;
const T_EYES: usize = 21;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Media,
    Controls,
    Clipboard,
    Apps,
    Tasks,
    Game,
    Stats,
    Power,
}

pub const PAGES: [Page; 8] = [
    Page::Media,
    Page::Controls,
    Page::Tasks,
    Page::Stats,
    Page::Clipboard,
    Page::Apps,
    Page::Game,
    Page::Power,
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
    Quick(u8),
    ClipItem(usize),
    ClipClear,
    Launch(usize),
    TimerPreset(u32),
    /// type any number of minutes
    TimerCustom,
    /// pin / unpin the n-th clipboard row
    ClipPin(usize),
    /// type to filter the clipboard list
    ClipSearch,
    ClipSearchClear,
    /// start a focus session on the n-th to-do
    TodoFocus(usize),
    /// open a Windows Settings page: an index into `SETTINGS_PAGES`
    OpenSettings(usize),
    /// Apps tab: open the picker / pin the n-th listed app / unpin the n-th pinned app / leave
    AppAdd,
    AppPick(usize),
    AppUnpin(usize),
    AppDone,
    AppSearch,
    /// the app list took too long: ask again
    AppRetry,
    /// look for new / changed apps again (keeps showing the current list meanwhile)
    AppRefresh,
    /// restart / shut down (asks to click twice)
    PowerRestart,
    PowerShutdown,
    /// click inside the running game
    GameClick,
    /// pick a game on the arcade screen
    GamePick(usize),
    /// back to the arcade screen
    GameBack,
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
    /// CPU and memory percent for the top bar (when enabled)
    pub stats: Option<(u32, u32)>,
    cpu_prev: (u64, u64),
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
    /// pinned items: kept first, never cleared, saved between runs (also works as snippets)
    pub pins: Vec<String>,
    pub clip_filter: String,
    pub entry_search: bool,
    pub clip_scroll: usize,
    ignore_clip: Option<String>,

    // launcher
    pub apps: Vec<AppEntry>,
    pub app_icons: Vec<Option<ID2D1Bitmap>>,

    // tasks
    pub timer_total_s: u32,
    /// the task being focused on, and whether the timer is running a break
    pub timer_task: Option<String>,
    pub break_mode: bool,
    /// focus length to come back to after an automatic break
    cycle_focus_s: u32,
    /// today's focus minutes and finished tasks
    pub focus_today: u32,
    pub done_today: u32,
    timer_left_ms: i64,
    timer_start: Option<Instant>,
    pub todos: Vec<Todo>,
    pub entering: Option<Vec<u16>>,
    /// the text being typed is a timer length in minutes, not a task
    pub entry_timer: bool,
    /// the notch was opened by the quick-add hotkey, so close it again afterwards
    capture_open: bool,
    /// output level 0..1 driving the music glow, smoothed
    pub level: f32,
    /// click squish and hover jiggle, both decaying 1 -> 0
    pub squish: f32,
    pub jiggle: f32,
    pub fx_phase: f32,
    beat_start: Option<Instant>,
    confetti_start: Option<Instant>,
    /// what the pill says while it pulses ("Time's up", "Drink water")
    pub alert_msg: &'static str,
    pub hub: crate::games::Hub,
    /// Apps tab picker: open?, the installed apps (None while loading), search text, scroll
    pub picking: bool,
    pub app_list: Option<Vec<(String, String)>>,
    pub app_loading: bool,
    /// when the current app-list request started (for the "taking too long" retry)
    pub app_load_t0: Option<Instant>,
    pub app_filter: String,
    pub app_scroll: usize,
    /// icons of the rows shown so far, by app id
    pub picker_icons: Vec<(String, Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap>)>,
    /// seconds, for spinners
    pub spin_t: f32,
    /// which power button (1 restart, 2 shut down) was clicked once, and when
    pub power_confirm: Option<(u8, Instant)>,
    /// live numbers for the Stats page
    pub stat_cpu: u32,
    pub stat_mem: u32,
    stat_prev: (u64, u64),
    pub stat_hist: Vec<f32>,
    /// pointer inside the open panel (logical px) and whether the button is held, for the games
    cursor_x: f32,
    cursor_y: f32,
    mouse_down: bool,
    /// interval of the page animation timer (0 = off)
    page_ms: u32,
    game_last: Instant,
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
    pub buddy: Option<crate::buddy::Buddy>,
    pub stage: Option<crate::scene::Stage>,
    pub scene_seen: bool,
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
            stats: None,
            cpu_prev: (0, 0),
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
            pins: store::load_pins(),
            clip_filter: String::new(),
            entry_search: false,
            clip_scroll: 0,
            ignore_clip: None,
            apps: store::load_apps(),
            app_icons: Vec::new(),
            timer_total_s: 25 * 60,
            timer_task: None,
            break_mode: false,
            cycle_focus_s: 25 * 60,
            focus_today: store::load_focus_today().0,
            done_today: store::load_focus_today().1,
            timer_left_ms: 25 * 60 * 1000,
            timer_start: None,
            todos: store::load_todos(),
            entering: None,
            entry_timer: false,
            capture_open: false,
            level: 0.0,
            squish: 0.0,
            jiggle: 0.0,
            fx_phase: 0.0,
            beat_start: None,
            confetti_start: None,
            alert_msg: "Time's up",
            hub: crate::games::Hub::new(store::load_bests()),
            picking: false,
            app_list: None,
            app_loading: false,
            app_load_t0: None,
            app_filter: String::new(),
            app_scroll: 0,
            picker_icons: Vec::new(),
            spin_t: 0.0,
            power_confirm: None,
            stat_cpu: 0,
            stat_mem: 0,
            stat_prev: (0, 0),
            stat_hist: Vec::new(),
            cursor_x: 0.0,
            cursor_y: 0.0,
            mouse_down: false,
            page_ms: 0,
            game_last: Instant::now(),
            prev_fg: HWND::default(),
            bt_busy: None,
            link_phase: 0,
            link_msg: None,
            pw: None,
            pw_prev_fg: HWND::default(),
            buddy: None,
            stage: None,
            scene_seen: false,
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

    /// Heartbeat after copying a link: 0..1, two quick bumps ("lub-dub").
    pub fn beat_factor(&self) -> f32 {
        let Some(t0) = self.beat_start else { return 0.0 };
        let t = t0.elapsed().as_secs_f32();
        let bump = |c: f32, w: f32| (-((t - c) / w).powi(2)).exp();
        (bump(0.08, 0.06) + 0.8 * bump(0.36, 0.07)).min(1.0)
    }

    /// Seconds since the confetti burst, while it is still falling.
    pub fn confetti_t(&self) -> Option<f32> {
        self.confetti_start.map(|t| t.elapsed().as_secs_f32()).filter(|t| *t < 3.0)
    }

    fn fx_active(&self) -> bool {
        self.squish > 0.0
            || self.jiggle > 0.0
            || self.beat_start.is_some_and(|t| t.elapsed().as_secs_f32() < 1.0)
            || self.confetti_t().is_some()
    }

    /// What the clipboard page lists: pinned items first, then recent ones, filtered
    /// by the search text. Each entry is (text, pinned).
    pub fn clip_view(&self) -> Vec<(&str, bool)> {
        let q = self.clip_filter.to_lowercase();
        self.pins
            .iter()
            .map(|t| (t.as_str(), true))
            .chain(self.clips.iter().map(|t| (t.as_str(), false)))
            .filter(|(t, _)| q.is_empty() || t.to_lowercase().contains(&q))
            .collect()
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
            && ((self.expanded && self.mstate.playing)
                || self.timer_running());
        if need && !self.sec_on {
            self.set_timer(T_SEC, 1000);
            self.sec_on = true;
        } else if !need && self.sec_on {
            self.kill_timer(T_SEC);
            self.sec_on = false;
        }
        // the mini game ticks at 60 Hz only while its tab is open
        let want: u32 = if self.expanded && !self.hidden {
            match self.page {
                Page::Game if self.hub.is_playing() => 16,
                Page::Stats => 1000,
                Page::Apps if self.picking && (self.app_list.is_none() || self.app_loading) => 33,
                _ => 0,
            }
        } else {
            0
        };
        if want != self.page_ms {
            if want == 0 {
                self.kill_timer(T_GAME);
            } else {
                self.game_last = Instant::now();
                self.set_timer(T_GAME, want);
            }
            self.page_ms = want;
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
        self.fx_phase += dt * 45.0;
        self.squish = (self.squish - dt * 4.5).max(0.0);
        self.jiggle = (self.jiggle - dt * 2.0).max(0.0);
        if self.eq_active() {
            // music glow follows the real output level (with a slow release)
            let peak = sys::audio_peak().unwrap_or(0.5 + 0.4 * self.eq_phase.sin());
            self.level = peak.max(self.level - dt * 2.2);
        } else {
            self.level = (self.level - dt * 4.0).max(0.0);
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
        let next = if moving || self.fx_active() {
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
            T_GAME => self.on_game_tick(),
            T_WATER => self.water_reminder(),
            T_EYES => self.eye_reminder(),
            T_STATS => self.on_stats_tick(),
            T_FSCHK => {
                self.kill_timer(T_FSCHK);
                self.on_foreground();
            }
            crate::pop::T_POPLEAVE => self.on_pop_leave_timer(),
            crate::pop::T_LINK => self.on_link_tick(),
            crate::buddy::T_BUDDY => self.on_buddy_tick(),
            crate::buddy::T_BWAKE => self.on_buddy_wake(),
            crate::scene::T_SCENE => self.on_scene_timer(),
            crate::scene::T_SCENEANIM => self.on_scene_anim(),
            crate::pop::T_TIP => self.on_tip_timer(),
            crate::pop::T_LINKMSG => self.on_linkmsg_timer(),
            T_SEC => {
                if self.timer_running() && self.timer_remaining_ms() <= 0 {
                    self.timer_done();
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
    /// Ctrl+Alt+T: open the notch straight into the to-do box. Enter saves, Esc
    /// cancels, and the notch closes again if the hotkey was what opened it.
    pub fn quick_capture(&mut self) {
        if self.todos.len() >= MAX_TODO {
            self.buddy_say("To-do list is full", 3000);
            return;
        }
        self.tucked = false;
        let was_open = self.expanded;
        self.set_expanded(true);
        self.select_page(Page::Tasks);
        self.start_entry();
        self.capture_open = !was_open && self.entering.is_some();
        self.render();
    }

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

    /// Start or stop the 3 s CPU/RAM sampling to match the setting.
    pub fn sync_stats(&mut self) {
        if self.cfg.stats {
            self.cpu_prev = sys::cpu_times();
            self.set_timer(T_STATS, 3000);
        } else {
            self.kill_timer(T_STATS);
            self.stats = None;
        }
    }

    pub fn set_clock_style(&mut self, n: u8) {
        self.cfg.clock_style = n.min(3);
        store::save_config(&self.cfg);
        self.render_bar();
    }

    /// (Re)arm the repeating water reminder to match the setting.
    pub fn sync_water(&mut self) {
        if self.cfg.water_min > 0 {
            self.set_timer(T_WATER, self.cfg.water_min as u32 * 60_000);
        } else {
            self.kill_timer(T_WATER);
        }
    }

    /// (Re)arm the repeating look-away reminder (the 20-20-20 rule) to match the setting.
    pub fn sync_eyes(&mut self) {
        if self.cfg.eye_min > 0 {
            self.set_timer(T_EYES, self.cfg.eye_min as u32 * 60_000);
        } else {
            self.kill_timer(T_EYES);
        }
    }

    pub fn set_eye_min(&mut self, mins: u16) {
        self.cfg.eye_min = mins;
        store::save_config(&self.cfg);
        self.sync_eyes();
    }

    fn eye_reminder(&mut self) {
        self.buddy_say("Look at something far away for 20 seconds", 7000);
        if !self.expanded && !self.timer_running() {
            self.alert_msg = "Look away";
            self.alert_until = Some(Instant::now() + std::time::Duration::from_secs(20));
            self.retarget();
            self.render();
        }
    }

    pub fn set_water_min(&mut self, mins: u16) {
        self.cfg.water_min = mins;
        store::save_config(&self.cfg);
        self.sync_water();
    }

    fn water_reminder(&mut self) {
        self.buddy_say("Time to drink some water!", 6000);
        if !self.expanded && !self.timer_running() {
            self.alert_msg = "Drink water";
            self.alert_until = Some(Instant::now() + std::time::Duration::from_secs(10));
            self.retarget();
            self.render();
        }
    }

    pub fn toggle_stats(&mut self) {
        self.cfg.stats = !self.cfg.stats;
        store::save_config(&self.cfg);
        self.sync_stats();
        self.render_bar();
    }

    fn on_stats_tick(&mut self) {
        if !self.cfg.stats || !self.bar_visible() {
            return;
        }
        let (idle, total) = sys::cpu_times();
        let (di, dt) = (idle.wrapping_sub(self.cpu_prev.0), total.wrapping_sub(self.cpu_prev.1));
        self.cpu_prev = (idle, total);
        let cpu = if dt == 0 { 0 } else { (100 - (di.min(dt) * 100 / dt)) as u32 };
        let now = Some((cpu, sys::mem_percent()));
        if now != self.stats {
            self.stats = now;
            self.render_bar();
        }
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
                Act::Seek => {
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
        self.cursor_x = x;
        self.cursor_y = y;
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
                    if !self.expanded {
                        self.jiggle = 1.0;
                        self.retarget();
                    }
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
        self.mouse_down = true;
        let (x, y) = (px as f32 / self.scale, py as f32 / self.scale);
        if !self.expanded {
            self.squish = 1.0;
            self.retarget();
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
        self.mouse_down = false;
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
        }
        self.render();
    }

    fn apply_drag(&mut self, x: f32) {
        match self.drag {
            Some(Drag::Seek(r)) => {
                self.seek_frac = Some(((x - r.x) / r.w).clamp(0.0, 1.0));
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
        if y < crate::ui::CONTENT_Y {
            // over the tab row: switch pages
            let i = PAGES.iter().position(|p| *p == self.page).unwrap_or(0) as i32;
            let n = (i + if delta < 0 { 1 } else { -1 }).clamp(0, PAGES.len() as i32 - 1);
            self.select_page(PAGES[n as usize]);
        } else if self.page == Page::Apps && self.picking {
            let max = self.app_view().len().saturating_sub(PICK_ROWS);
            self.app_scroll = if delta < 0 { (self.app_scroll + 1).min(max) } else { self.app_scroll.saturating_sub(1) };
            self.ensure_picker_icons();
            self.render();
        } else if self.page == Page::Clipboard {
            let max = self.clip_view().len().saturating_sub(CLIP_ROWS);
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
        if p != Page::Apps {
            self.picking = false;
            self.app_filter.clear();
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
                let item = self.clip_view().get(i).map(|(t, p)| (t.to_string(), *p));
                if let Some((t, pinned)) = item {
                    self.ignore_clip = Some(t.clone());
                    if sys::set_clipboard_text(self.hwnd, &t) {
                        if !pinned {
                            self.clips.retain(|c| *c != t);
                            self.clips.insert(0, t);
                        }
                        self.clip_scroll = 0;
                        self.pinned = false;
                        self.set_expanded(false);
                        // give the target window a moment to regain input focus
                        self.set_timer(T_PASTE, 170);
                    }
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
            Act::ClipSearch => {
                self.start_entry();
                if self.entering.is_some() {
                    self.entry_search = true;
                    self.entering = Some(self.clip_filter.encode_utf16().collect());
                }
            }
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
                    self.pinned = false;
                    self.set_expanded(false);
                }
            }
            Act::TimerPreset(mins) => {
                self.timer_task = None;
                self.break_mode = false;
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
                self.break_mode = false;
                self.timer_total_s = self.cycle_focus_s;
                self.timer_start = None;
                self.timer_left_ms = self.timer_total_s as i64 * 1000;
                self.after_timer_change();
            }
            Act::TodoToggle(i) => {
                if let Some(t) = self.todos.get_mut(i) {
                    t.done = !t.done;
                    if t.done {
                        self.done_today += 1;
                    } else {
                        self.done_today = self.done_today.saturating_sub(1);
                    }
                    store::save_todos(&self.todos);
                    store::save_focus_today(self.focus_today, self.done_today);
                }
            }
            Act::TodoFocus(i) => {
                if let Some(t) = self.todos.get(i) {
                    self.timer_task = Some(t.text.clone());
                    self.break_mode = false;
                    self.cycle_focus_s = 25 * 60;
                    self.timer_total_s = 25 * 60;
                    self.timer_left_ms = 25 * 60_000;
                    self.timer_start = Some(Instant::now());
                    self.after_timer_change();
                }
            }
            Act::TodoDel(i) => {
                if i < self.todos.len() {
                    self.todos.remove(i);
                    store::save_todos(&self.todos);
                }
            }
            Act::TodoAdd => self.start_entry(),
            Act::AppAdd => {
                self.picking = true;
                self.app_filter.clear();
                self.app_scroll = 0;
                // always look again, so apps installed (or icons fixed) since last time show up;
                // the old list stays visible until the new one arrives
                if !self.app_loading {
                    self.app_loading = true;
                    self.app_load_t0 = Some(Instant::now());
                    sys::list_start_apps(self.hwnd.0 as isize);
                }
                self.ensure_picker_icons();
                self.sync_timers();
            }
            Act::AppRefresh => {
                if !self.app_loading {
                    self.app_loading = true;
                    self.app_load_t0 = Some(Instant::now());
                    sys::list_start_apps(self.hwnd.0 as isize);
                    self.sync_timers();
                }
            }
            Act::AppRetry => {
                self.app_list = None;
                self.app_loading = true;
                self.app_load_t0 = Some(Instant::now());
                sys::list_start_apps(self.hwnd.0 as isize);
            }
            Act::AppDone => {
                self.picking = false;
                self.app_filter.clear();
                self.sync_timers();
            }
            Act::AppSearch => {
                self.start_entry();
                if self.entering.is_some() {
                    self.entry_search = true;
                    self.entering = Some(self.app_filter.encode_utf16().collect());
                }
            }
            Act::AppPick(i) => {
                let item = self.app_view().get(i).map(|(n, id)| (n.clone(), id.clone()));
                if let Some((name, id)) = item {
                    let target = format!("shell:AppsFolder\\{id}");
                    if !self.apps.iter().any(|a| a.target == target) {
                        self.apps.push(store::AppEntry { name, target, args: String::new() });
                        store::save_apps(&self.apps);
                        self.load_app_icons();
                    }
                }
            }
            Act::AppUnpin(i) => {
                if i < self.apps.len() {
                    self.apps.remove(i);
                    store::save_apps(&self.apps);
                    self.load_app_icons();
                }
            }
            Act::OpenSettings(k) => sys::open(SETTINGS_PAGES[k.min(SETTINGS_PAGES.len() - 1)], ""),
            Act::PowerRestart => self.power_press(1),
            Act::PowerShutdown => self.power_press(2),
            Act::GameClick => self.hub.click(),
            Act::GamePick(k) => {
                self.hub.pick(k);
                self.sync_timers();
            }
            Act::GameBack => {
                self.hub.back();
                self.sync_timers();
            }
            Act::TimerCustom => {
                self.start_entry();
                self.entry_timer = self.entering.is_some();
            }
        }
    }

    fn on_game_tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.game_last).as_secs_f32().min(0.05);
        self.game_last = now;
        self.spin_t += dt;
        match self.page {
            Page::Game => {
                let ox = (EXP_W - crate::game::W) / 2.0;
                let inp = crate::games::Input { x: self.cursor_x - ox, y: self.cursor_y - 14.0 - crate::ui::CONTENT_Y, down: self.mouse_down };
                let out = self.hub.step(dt, &inp);
                if out.new_best {
                    store::save_bests(&self.hub.best);
                }
                play_sfx(out.sfx);
            }
            Page::Stats => {
                let (idle, total) = sys::cpu_times();
                if self.stat_prev != (0, 0) {
                    let (di, dt) = (idle.wrapping_sub(self.stat_prev.0), total.wrapping_sub(self.stat_prev.1));
                    self.stat_cpu = if dt == 0 { 0 } else { (100 - (di.min(dt) * 100 / dt)) as u32 };
                }
                self.stat_prev = (idle, total);
                self.stat_mem = sys::mem_percent();
                self.stat_hist.push(self.stat_cpu as f32 / 100.0);
                if self.stat_hist.len() > 40 {
                    self.stat_hist.remove(0);
                }
            }
            _ => {}
        }
        self.render();
    }

    /// Restart / shut down only on a second click within a few seconds.
    fn power_press(&mut self, k: u8) {
        let armed = matches!(self.power_confirm, Some((p, t)) if p == k && t.elapsed().as_secs_f32() < 3.5);
        if armed {
            self.power_confirm = None;
            if k == 1 {
                sys::restart_pc();
            } else {
                sys::shutdown_pc();
            }
        } else {
            self.power_confirm = Some((k, Instant::now()));
        }
    }

    fn after_timer_change(&mut self) {
        self.retarget(); // pill width depends on the timer
        self.sync_timers();
    }

    fn timer_done(&mut self) {
        self.timer_start = None;
        self.timer_left_ms = 0;
        let was_break = self.break_mode;
        if !was_break {
            self.focus_today += self.timer_total_s / 60;
            store::save_focus_today(self.focus_today, self.done_today);
        }
        alarm_sound();
        self.confetti_start = Some(Instant::now());
        self.buddy_event(crate::buddy::BuddyEv::TimerDone);
        // celebrate with the Quidditch scene (the Snitch is caught) when there is room
        if self.cfg.scenes && self.cfg.buddy != 0 && self.stage_has_room() {
            self.play_scene(10);
        }
        self.page = Page::Tasks; // so a click on the pill lands on the timer
        if !self.expanded {
            self.alert_msg = "Time's up";
            // pulse the pill for 30 s (or until it is clicked)
            self.alert_until = Some(Instant::now() + std::time::Duration::from_secs(30));
        }
        if self.cfg.auto_break {
            // focus -> 5 minute break -> focus again, until paused or reset
            if was_break {
                self.break_mode = false;
                self.timer_total_s = self.cycle_focus_s;
            } else {
                self.cycle_focus_s = self.timer_total_s;
                self.break_mode = true;
                self.timer_total_s = 5 * 60;
            }
            self.timer_left_ms = self.timer_total_s as i64 * 1000;
            self.timer_start = Some(Instant::now());
            self.alert_msg = if was_break { "Back to focus" } else { "Break time" };
        }
        self.retarget();
        self.sync_timers();
        self.render();
    }

    pub fn toggle_auto_break(&mut self) {
        self.cfg.auto_break = !self.cfg.auto_break;
        store::save_config(&self.cfg);
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
        self.entry_timer = false;
        self.entry_search = false;
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
        let timer = std::mem::take(&mut self.entry_timer);
        let search = std::mem::take(&mut self.entry_search);
        if search && !commit {
            // Esc drops the filter; Enter keeps it
            if self.page == Page::Apps {
                self.app_filter.clear();
            } else {
                self.clip_filter.clear();
            }
        }
        if commit && timer {
            let mins = String::from_utf16_lossy(&buf).parse::<u32>().unwrap_or(0).min(600);
            if mins > 0 {
                self.timer_task = None;
                self.break_mode = false;
                self.timer_total_s = mins * 60;
                self.timer_left_ms = mins as i64 * 60_000;
                self.timer_start = Some(Instant::now());
                self.after_timer_change();
            }
        } else if commit && !search {
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
        if std::mem::take(&mut self.capture_open) {
            self.set_expanded(false);
        }
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
                c if self.entry_timer && !(0x30..=0x39).contains(&c) && c >= 0x20 => Do::Redraw,
                c if c >= 0x20 && buf.len() < if self.entry_timer { 3 } else { 80 } => {
                    buf.push(c);
                    Do::Redraw
                }
                _ => Do::Redraw,
            }
        };
        if self.entry_search {
            if let Some(buf) = &self.entering {
                let t = String::from_utf16_lossy(buf);
                if self.page == Page::Apps {
                    self.app_filter = t;
                    self.app_scroll = 0;
                } else {
                    self.clip_filter = t;
                    self.clip_scroll = 0;
                }
            }
            if self.page == Page::Apps {
                self.ensure_picker_icons();
            }
        }
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
        let started = st.playing && !self.mstate.playing;
        self.mstate = st;
        if started {
            self.buddy_event(crate::buddy::BuddyEv::MusicStarted);
        }
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
                let icon = if e.target.starts_with("shell:AppsFolder") {
                    sys::shell_icon(&e.target)?
                } else {
                    sys::file_icon(&sys::resolve_path(&e.target)?)?
                };
                let bmp = self.gfx.bitmap_from_hicon(icon).ok();
                unsafe {
                    let _ = DestroyIcon(icon);
                }
                bmp
            })
            .collect();
    }

    /// The installed apps matching the search text.
    pub fn app_view(&self) -> Vec<&(String, String)> {
        let q = self.app_filter.to_lowercase();
        self.app_list
            .as_ref()
            .map(|l| l.iter().filter(|(n, _)| q.is_empty() || n.to_lowercase().contains(&q)).collect())
            .unwrap_or_default()
    }

    /// Load the icons of the rows the picker is showing (a few at a time, not all 150).
    pub fn ensure_picker_icons(&mut self) {
        let ids: Vec<String> = self.app_view().iter().skip(self.app_scroll).take(PICK_ROWS).map(|(_, id)| id.clone()).collect();
        for id in ids {
            if self.picker_icons.iter().any(|(k, _)| *k == id) {
                continue;
            }
            let bmp = sys::shell_icon(&format!("shell:AppsFolder\\{id}")).and_then(|icon| {
                let b = self.gfx.bitmap_from_hicon(icon).ok();
                unsafe {
                    let _ = DestroyIcon(icon);
                }
                b
            });
            self.picker_icons.push((id, bmp));
        }
    }

    /// The Start-menu list arrived.
    pub fn on_app_list(&mut self) {
        self.app_loading = false;
        self.app_load_t0 = None;
        self.app_list = Some(sys::START_APPS.lock().unwrap().clone());
        self.app_scroll = 0;
        self.picker_icons.clear();
        self.load_app_icons();
        self.ensure_picker_icons();
        self.sync_timers();
        self.render();
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
        if text.starts_with("http://") || text.starts_with("https://") {
            self.beat_start = Some(Instant::now());
            self.retarget();
        }
        if self.pins.contains(&text) {
            return; // already pinned
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
        self.rebuild_buddy();
        self.rebuild_stage();
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
/// Short game sounds, played one batch at a time so they never pile up.
fn play_sfx(list: Vec<(u32, u32)>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);
    if list.is_empty() || BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(move || {
        for (hz, ms) in list.into_iter().take(3) {
            unsafe {
                let _ = Beep(hz.clamp(37, 32_000), ms.clamp(10, 400));
            }
        }
        BUSY.store(false, Ordering::Release);
    });
}

fn alarm_sound() {
    std::thread::spawn(|| unsafe {
        for _ in 0..3 {
            let _ = Beep(1175, 170);
            std::thread::sleep(std::time::Duration::from_millis(90));
        }
        let _ = MessageBeep(MB_ICONEXCLAMATION);
    });
}

/// Renders the open panel's pages to raw files for eyeballing (run with --ignored and
/// PREVIEW_DIR set). Test-only: it builds the real app state without a window.
#[cfg(test)]
mod preview {
    use super::*;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

    #[test]
    #[ignore]
    fn render_pages() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let dir = std::env::var("PREVIEW_DIR").expect("PREVIEW_DIR");
        let scale = 2.0f32;
        let mut a = App::new(HWND::default(), scale).expect("app");
        a.expanded = true;
        a.sw = Spring::new(EXP_W);
        a.sh = Spring::new(EXP_H);
        a.todos = vec![
            Todo { done: false, text: "Review the pull request".into() },
            Todo { done: true, text: "Buy groceries".into() },
            Todo { done: false, text: "Call the dentist".into() },
        ];
        a.clips = vec!["https://example.com/some/long/link/to/read/later".into(), "Meeting at 11:00 in room 4".into(), "cargo build --release".into()];
        a.pins = vec!["hello@example.com".into()];
        a.mstate.has_session = true;
        a.mstate.title = "lofi beats to study and relax".into();
        a.mstate.artist = "lofi radio".into();
        a.mstate.playing = true;
        a.mstate.dur_ticks = 62 * 60 * 10_000_000 / 60;
        a.radios.wifi = Some(true);
        a.radios.bluetooth = Some(false);
        a.stats = Some((23, 61));
        let render = |a: &App, name: &str| {
            let (pw, ph) = ((EXP_W * scale) as i32, (EXP_H * scale) as i32);
            a.gfx.begin(pw, ph).unwrap();
            let mut hits = Vec::new();
            crate::ui::draw(a, EXP_W, EXP_H, &mut hits);
            let _ = a.gfx.end();
            let (bw, _bh, buf) = a.gfx.snapshot();
            let mut out = Vec::new();
            out.extend_from_slice(&(pw as u32).to_le_bytes());
            out.extend_from_slice(&(ph as u32).to_le_bytes());
            for y in 0..ph {
                let row = (y * bw * 4) as usize;
                out.extend_from_slice(&buf[row..row + (pw * 4) as usize]);
            }
            std::fs::write(format!("{dir}/panel_{name}.raw"), out).unwrap();
        };
        let pages: Vec<(String, Page)> = PAGES.iter().map(|p| (format!("{p:?}").to_lowercase(), *p)).collect();
        for (name, page) in pages {
            a.page = page;
            render(&a, &name);
        }
        // each game while it is running
        a.page = Page::Game;
        for k in 0..4 {
            a.hub.pick(k);
            a.hub.click();
            for i in 0..100 {
                let inp = crate::games::Input { x: 150.0 + (i as f32 * 3.0), y: 80.0 + (i as f32 * 0.5).sin() * 40.0, down: i % 50 < 10 };
                a.hub.step(0.016, &inp);
            }
            render(&a, &format!("play{k}"));
        }
        // the app picker: loading, then the real list from Windows with icons
        a.page = Page::Apps;
        a.picking = true;
        a.spin_t = 1.0;
        render(&a, "apps_loading");
        sys::list_start_apps(0);
        for _ in 0..200 {
            if !sys::START_APPS.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        a.app_list = Some(sys::START_APPS.lock().unwrap().clone());
        a.ensure_picker_icons();
        render(&a, "apps_picker");
        a.app_filter = "micro".into();
        a.app_scroll = 0;
        a.ensure_picker_icons();
        render(&a, "apps_search");
        a.app_filter.clear();
        a.app_loading = true;
        a.spin_t = 0.7;
        render(&a, "apps_refreshing");
        a.app_filter = "brawl".into();
        a.picker_icons.clear();
        a.ensure_picker_icons();
        render(&a, "apps_brawl");
        a.app_loading = false;
        a.app_filter = "micro".into();
        a.picking = false;
        a.app_filter.clear();
        render(&a, "apps_grid");
        // the collapsed pill, idle and with media (cropped to the pill)
        a.expanded = false;
        a.mstate.has_session = false;
        a.sh = Spring::new(a.col_h());
        a.sw = Spring::new(COL_IDLE_W);
        a.gfx.begin((EXP_W * scale) as i32, (EXP_H * scale) as i32).unwrap();
        let mut hits = Vec::new();
        crate::ui::draw(&a, COL_IDLE_W, a.col_h(), &mut hits);
        let _ = a.gfx.end();
        let (bw, _bh, buf) = a.gfx.snapshot();
        let (pw, ph) = ((COL_IDLE_W * scale) as i32, (a.col_h() * scale) as i32);
        let mut out = Vec::new();
        out.extend_from_slice(&(pw as u32).to_le_bytes());
        out.extend_from_slice(&(ph as u32).to_le_bytes());
        for y in 0..ph {
            let row = (y * bw * 4) as usize;
            out.extend_from_slice(&buf[row..row + (pw * 4) as usize]);
        }
        std::fs::write(format!("{dir}/panel_idle.raw"), out).unwrap();
    }

    /// Draws the frames of the README demo GIF from the real UI code (no screen capture).
    /// PREVIEW_DIR=... cargo test --release preview::render_gif -- --ignored
    #[test]
    #[ignore]
    fn render_gif() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let dir = std::env::var("PREVIEW_DIR").expect("PREVIEW_DIR");
        let scale = 2.0f32;
        let mut a = App::new(HWND::default(), scale).expect("app");
        a.expanded = true;
        a.todos = vec![
            Todo { done: false, text: "Review the pull request".into() },
            Todo { done: false, text: "Reply to Sam".into() },
            Todo { done: true, text: "Buy groceries".into() },
        ];
        a.clips = vec![
            "https://github.com/Akhilesh-Varute/Nock".into(),
            "Standup at 10:30, room 4".into(),
            "cargo build --release".into(),
            "hello@example.com".into(),
        ];
        a.pins = vec!["C:\\Projects\\notes.txt".into()];
        a.stat_cpu = 23;
        a.stat_mem = 61;
        a.stat_hist = (0..40).map(|i| 0.15 + 0.12 * ((i as f32) * 0.5).sin().abs() + (i as f32 % 7.0) * 0.02).collect();
        a.radios.wifi = Some(true);
        a.radios.bluetooth = Some(false);
        a.stats = Some((23, 61));
        let mut list: Vec<String> = Vec::new();
        let mut n = 0;
        let mut shot = |a: &App, w: f32, h: f32, ms: u32, caption: &str| {
            let (pw, ph) = ((w * scale) as i32, (h * scale) as i32);
            a.gfx.begin(pw, ph).unwrap();
            let mut hits = Vec::new();
            crate::ui::draw(a, w, h, &mut hits);
            let _ = a.gfx.end();
            let (bw, _bh, buf) = a.gfx.snapshot();
            let mut out = Vec::new();
            out.extend_from_slice(&(pw as u32).to_le_bytes());
            out.extend_from_slice(&(ph as u32).to_le_bytes());
            for y in 0..ph {
                let row = (y * bw * 4) as usize;
                out.extend_from_slice(&buf[row..row + (pw * 4) as usize]);
            }
            std::fs::write(format!("{dir}/gif_{n:03}.raw"), out).unwrap();
            list.push(format!("{n:03}|{ms}|{caption}"));
            n += 1;
        };
        let ch = a.col_h();
        let full = |a: &mut App, page: Page| {
            a.page = page;
            a.expanded = true;
            a.sh = Spring::new(EXP_H);
            a.sw = Spring::new(EXP_W);
        };

        // 1. the idle pill, then it opens
        a.expanded = false;
        a.sh = Spring::new(ch);
        shot(&a, COL_IDLE_W, ch, 1400, "A quiet little clock at the top of your screen...");
        a.expanded = true;
        a.page = Page::Tasks;
        for e in [0.35f32, 0.7, 0.93] {
            let (w, h) = (COL_IDLE_W + (EXP_W - COL_IDLE_W) * e, ch + (EXP_H - ch) * e);
            a.sh = Spring::new(h);
            shot(&a, w, h, 70, "A quiet little clock at the top of your screen...");
        }
        full(&mut a, Page::Tasks);
        shot(&a, EXP_W, EXP_H, 1100, "...that is also your task list and focus timer");

        // 2. quick add: type a task
        let text = "Ship the demo";
        for k in [0usize, 4, 8, 13] {
            a.entering = Some(text.chars().take(k).collect::<String>().encode_utf16().collect());
            shot(&a, EXP_W, EXP_H, if k == 13 { 700 } else { 380 }, "Press Ctrl+Alt+T from any app and type: a task in two seconds");
        }
        a.entering = None;
        a.todos.insert(0, Todo { done: false, text: text.into() });
        shot(&a, EXP_W, EXP_H, 1000, "Press Ctrl+Alt+T from any app and type: a task in two seconds");
        a.todos[1].done = true;
        shot(&a, EXP_W, EXP_H, 1000, "Tick it off when it is done");

        // 3. focus timer on that task
        a.timer_task = Some(text.into());
        a.timer_left_ms = 25 * 60 * 1000;
        a.timer_start = Some(Instant::now());
        shot(&a, EXP_W, EXP_H, 700, "One click starts a focus timer on a task");
        a.timer_start = None;
        a.timer_left_ms = (24 * 60 + 41) * 1000;
        a.timer_start = Some(Instant::now());
        shot(&a, EXP_W, EXP_H, 700, "One click starts a focus timer on a task");
        a.expanded = false;
        a.sh = Spring::new(ch);
        shot(&a, COL_TIMER_W, ch, 1600, "Close the panel: the countdown rides in the notch");
        a.timer_start = None;
        a.timer_left_ms = 25 * 60 * 1000;
        a.timer_task = None;

        // 4. the other tabs
        full(&mut a, Page::Stats);
        shot(&a, EXP_W, EXP_H, 1000, "Live CPU, memory and battery");
        full(&mut a, Page::Clipboard);
        shot(&a, EXP_W, EXP_H, 1300, "Clipboard history with pins and search");
        full(&mut a, Page::Apps);
        a.picking = false;
        shot(&a, EXP_W, EXP_H, 900, "Pin your apps");
        full(&mut a, Page::Game);
        shot(&a, EXP_W, EXP_H, 1100, "...and a quick game for your break");
        std::fs::write(format!("{dir}/frames.txt"), list.join("\n")).unwrap();
    }

    /// Scene frames for the README demo GIF: PREVIEW_DIR=... cargo test --release preview::render_scenes -- --ignored
    #[test]
    #[ignore]
    fn render_scenes() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let dir = std::env::var("PREVIEW_DIR").expect("PREVIEW_DIR");
        let scale = 3.0f32;
        let g = crate::gfx::Gfx::new((400.0 * scale) as i32, (34.0 * scale) as i32, scale).expect("gfx");
        for id in [0usize, 3, 5, 7] {
            for k in 0..30 {
                let t = 0.3 + k as f32 * 0.35;
                let (pw, ph) = ((380.0 * scale) as i32, (32.0 * scale) as i32);
                g.begin(pw, ph).unwrap();
                crate::scene::demo_frame(&g, id, t, 380.0, false);
                let _ = g.end();
                let (bw, _bh, buf) = g.snapshot();
                let mut out = Vec::new();
                out.extend_from_slice(&(pw as u32).to_le_bytes());
                out.extend_from_slice(&(ph as u32).to_le_bytes());
                for y in 0..ph {
                    let row = (y * bw * 4) as usize;
                    out.extend_from_slice(&buf[row..row + (pw * 4) as usize]);
                }
                std::fs::write(format!("{dir}/scene_{id}_{k:02}.raw"), out).unwrap();
            }
        }
    }
}
