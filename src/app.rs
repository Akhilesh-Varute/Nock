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
/// Files the Shelf tab holds.
pub const SHELF_MAX: usize = 12;

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

/// Posted once per display refresh while something animates (see `pump`).
pub const WM_FRAME: u32 = 0x8000 + 12; // WM_APP + 12

/// Seconds a page cross-fade takes.
const PAGE_FADE_S: f32 = 0.20;

/// Display-synced frame source. WM_TIMER is the lowest-priority message and ticks at
/// the ~15.6 ms system clock, so animation driven by it stutters. This thread sleeps
/// in `DwmFlush` (returns at each vblank) and posts WM_FRAME, which beats WM_TIMER in
/// the queue. It parks when nothing animates, so idle cost stays zero.
mod pump {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;
    use std::thread::{self, Thread};
    use std::time::Duration;

    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::Graphics::Dwm::DwmFlush;
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

    static ON: AtomicBool = AtomicBool::new(false);
    /// A WM_FRAME is queued and not yet handled; never queue a second one.
    static PENDING: AtomicBool = AtomicBool::new(false);
    static THREAD: OnceLock<Thread> = OnceLock::new();

    pub fn set(hwnd: isize, on: bool) {
        ON.store(on, Ordering::Release);
        if !on {
            return;
        }
        let t = THREAD.get_or_init(|| {
            thread::spawn(move || loop {
                while !ON.load(Ordering::Acquire) {
                    thread::park();
                }
                if unsafe { DwmFlush() }.is_err() {
                    thread::sleep(Duration::from_millis(16));
                }
                if ON.load(Ordering::Acquire) && !PENDING.swap(true, Ordering::AcqRel) {
                    unsafe {
                        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), super::WM_FRAME, WPARAM(0), LPARAM(0));
                    }
                }
            })
            .thread()
            .clone()
        });
        t.unpark();
    }

    pub fn handled() {
        PENDING.store(false, Ordering::Release);
    }
}

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
/// One-shot: hides the cycle page dots 2 s after the last scroll.
const T_DOTS: usize = 22;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Media,
    Controls,
    Clipboard,
    Shelf,
    Apps,
    Tasks,
    Game,
    Stats,
    Power,
}

pub const PAGES: [Page; 9] = [
    Page::Media,
    Page::Controls,
    Page::Tasks,
    Page::Stats,
    Page::Clipboard,
    Page::Shelf,
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
    /// Shelf tab: press on the n-th file (click opens it, dragging takes it out) / remove it / remove all
    ShelfItem(usize),
    ShelfDel(usize),
    ShelfClear,
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

/// A Previous / Next press that has not shown up as a new track yet. The panel keeps
/// showing the old track (with a spinner) instead of dropping to "Nothing playing"
/// while the player loads the next one.
pub struct Skip {
    pub t0: Instant,
    pub next: bool,
    pub snap: MediaState,
    pub art: Option<ID2D1Bitmap>,
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
    /// page being faded out, and how far the cross-fade has got (1.0 = finished)
    pub prev_page: Page,
    pub page_t: f32,
    /// waiting for the player to load the track a Previous / Next press asked for
    pub skip: Option<Skip>,
    /// a window covers the notch (only tracked with the smart-hide option)
    smart_hit: bool,
    /// opacity of the notch and bar while they fade in or out, and where that is heading
    fade: f32,
    fade_to: f32,
    /// the fade-out has finished: now really hide
    fade_out_done: bool,
    /// the sliding highlight behind the active tab (x and width, in points)
    pub tab_x: f32,
    pub tab_w: f32,
    /// hover fade per control: (control, 0..1), so highlights ease in and out
    pub hover: Vec<(Act, f32)>,
    /// CPU / memory bars as drawn (they glide to the real value)
    pub bar_cpu: f32,
    pub bar_mem: f32,
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
    /// files parked on the Shelf tab (full paths) and their icons
    pub shelf: Vec<String>,
    pub shelf_icons: Vec<Option<ID2D1Bitmap>>,
    /// a file is being dragged over the notch
    pub shelf_hover: bool,
    /// the drag itself opened the panel, so closing it again if nothing is dropped
    shelf_auto_open: bool,
    /// pressed a shelf file: (index, x, y); becomes a drag once the pointer moves
    shelf_press: Option<(usize, f32, f32)>,
    /// our own drag-out is running (no auto-close meanwhile)
    shelf_out: bool,
    pub clip_filter: String,
    pub entry_search: bool,
    pub clip_scroll: usize,
    /// months the top-bar calendar is shown away from the current month
    pub cal_off: i32,
    ignore_clip: Option<String>,

    // launcher
    pub apps: Vec<AppEntry>,
    pub app_icons: Vec<Option<ID2D1Bitmap>>,

    // tasks
    pub timer_total_s: u32,
    /// Which page the scroll-up cycle is on: 0 time only, 1 date, 2 tasks left, 3 focus today.
    pub cycle: u8,
    /// The page dots stay visible until then.
    pub cycle_until: Option<Instant>,
    /// Width of the glance last time the minute timer looked (to resize the pill when it changes).
    glance_last: f32,
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
    /// smart hints: low-battery warning already shown, minutes of continuous use so far
    batt_warned: bool,
    active_min: u16,
    /// what clicking the pill does while a copied-link/path hint is showing
    alert_act: Option<AlertAct>,
    /// colour dot shown beside a "Colour copied" hint
    pub alert_swatch: Option<Color>,
    /// when the microphone started being used (a call), for the pill
    pub call_since: Option<Instant>,
    pub mic_muted: bool,
    /// text for a meeting hint ("Standup 5m"), shown instead of alert_msg when set
    pub alert_label: String,
    /// start of the meeting a hint is about (local epoch), so its countdown stays live
    pub alert_start: Option<i64>,
    /// minutes since the calendar link was fetched, and the meeting already announced
    cal_age: u16,
    cal_seen: Option<(i64, u8)>,
}

enum AlertAct {
    Open(String),
    Reveal(String),
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
            prev_page: Page::Media,
            page_t: 1.0,
            skip: None,
            smart_hit: false,
            fade: 1.0,
            fade_to: 1.0,
            fade_out_done: false,
            tab_x: 0.0,
            tab_w: 0.0,
            hover: Vec::new(),
            bar_cpu: 0.0,
            bar_mem: 0.0,
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
            shelf: store::load_shelf(),
            shelf_icons: Vec::new(),
            shelf_hover: false,
            shelf_auto_open: false,
            shelf_press: None,
            shelf_out: false,
            clip_filter: String::new(),
            entry_search: false,
            clip_scroll: 0,
            cal_off: 0,
            ignore_clip: None,
            apps: store::load_apps(),
            app_icons: Vec::new(),
            timer_total_s: 25 * 60,
            cycle: 0,
            cycle_until: None,
            glance_last: 0.0,
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
            batt_warned: false,
            alert_act: None,
            alert_swatch: None,
            call_since: None,
            mic_muted: false,
            alert_label: String::new(),
            alert_start: None,
            cal_age: 60,
            cal_seen: None,
            active_min: 0,
        };
        a.update_monitor();
        sys::watch_mic(hwnd.0 as isize);
        sys::watch_mic_sessions(hwnd.0 as isize);
        a.load_app_icons();
        a.load_shelf_icons();
        a.snap_tab();
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
            let mon = sys::pick_monitor(self.cfg.monitor);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(mon, &mut mi).as_bool() {
                self.mon = mi.rcMonitor;
                // for "I can't see it" reports: which monitor Nock drew on, and how many there are
                let count = GetSystemMetrics(SM_CMONITORS);
                sys::log_visibility(&format!(
                    "monitor {},{},{},{} (of {count} monitor(s)), scale {:.2}",
                    mi.rcMonitor.left, mi.rcMonitor.top, mi.rcMonitor.right, mi.rcMonitor.bottom, self.scale
                ));
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
            self.load_shelf_icons();
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
            // a meeting hint grows to fit its text
            let w = if self.alert_msg == "Meeting" { (84.0 + (self.alert_label.chars().count() + 4) as f32 * 7.5).clamp(190.0, 340.0) } else { 190.0 };
            (w, self.col_h())
        } else if self.call_since.is_some() && !self.timer_running() {
            (COL_TIMER_W + 20.0, self.col_h())
        } else if self.has_media() {
            let extra = if self.timer_running() { 58.0 } else { 0.0 };
            (COL_MEDIA_W + extra, self.col_h())
        } else if self.timer_running() {
            (COL_TIMER_W, self.col_h())
        } else {
            (COL_IDLE_W + self.glance_extra_w(), self.col_h())
        }
    }

    /// Text and reserved width (fixed, so numbers never jitter) of the idle glance slot.
    pub fn glance(&self) -> Option<(String, f32)> {
        let kind = self.glance_kind();
        let left = self.todos.iter().filter(|t| !t.done).count();
        let (_, mo, day, wday, hh, mm, ..) = sys::local_time();
        let meeting = if kind == 4 {
            let (now, _) = crate::cal::now();
            let hours = self.cfg.idle_meeting_hours as i64;
            next_meeting(&crate::cal::EVENTS.lock().unwrap(), now, hours).map(|e| meeting_text(&e.title, e.start, self.cfg.clock_24h))
        } else {
            None
        };
        let _ = (hh, mm);
        glance_text(&GlanceIn { kind, has_todos: !self.todos.is_empty(), left, focus_min: self.focus_today, stats: self.stats, meeting, date: date_text(mo, day, wday) })
    }

    /// What the slot shows right now: the cycle page when cycling is on, else the chosen glance.
    /// Kinds: 0 none, 1 tasks left, 2 focus today, 3 CPU and RAM, 4 next meeting, 5 date.
    pub fn glance_kind(&self) -> u8 {
        if self.cfg.idle_cycle {
            [0, 5, 1, 2][(self.cycle % 4) as usize]
        } else {
            self.cfg.idle_glance
        }
    }

    /// Page dots are showing (during and just after a scroll).
    pub fn cycle_dots(&self) -> bool {
        self.cfg.idle_cycle && self.cycle_until.is_some_and(|t| Instant::now() < t)
    }

    /// True when the collapsed pill would show only the clock.
    fn idle_pill(&self) -> bool {
        !self.alerting() && !(self.call_since.is_some() && !self.timer_running()) && !self.has_media() && !self.timer_running()
    }

    /// Scroll up over the idle pill: next page. A one-shot timer hides the dots afterwards.
    fn cycle_next(&mut self) {
        self.cycle = (self.cycle + 1) % 4;
        self.cycle_until = Some(Instant::now() + std::time::Duration::from_secs(2));
        self.set_timer(T_DOTS, 2000);
        self.retarget();
        self.render();
    }

    pub fn toggle_day_line(&mut self) {
        self.cfg.day_line = !self.cfg.day_line;
        store::save_config(&self.cfg);
        if !self.hidden {
            self.render();
        }
    }

    pub fn toggle_idle_cycle(&mut self) {
        self.cfg.idle_cycle = !self.cfg.idle_cycle;
        self.cycle = 0;
        store::save_config(&self.cfg);
        // the CPU and RAM sampling follows what the slot shows
        self.sync_stats();
        if !self.hidden && !self.expanded {
            self.retarget();
            self.render();
        }
    }

    /// Extra pill width for the glance (separator and padding included).
    pub fn glance_extra_w(&self) -> f32 {
        self.glance().map_or(0.0, |(_, w)| w + 16.0)
    }

    pub fn set_idle_glance(&mut self, v: u8) {
        self.cfg.idle_glance = v.min(4);
        store::save_config(&self.cfg);
        self.sync_stats();
        if !self.hidden && !self.expanded {
            self.retarget();
            self.render();
        }
    }

    pub fn toggle_idle_focus_bar(&mut self) {
        self.cfg.idle_focus_bar = !self.cfg.idle_focus_bar;
        store::save_config(&self.cfg);
        if !self.hidden {
            self.render();
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
        // 16 means "every frame": use the display-synced pump instead of WM_TIMER
        pump::set(self.hwnd.0 as isize, ms == 16);
        if ms == 0 || ms == 16 {
            self.kill_timer(T_ANIM);
        } else {
            self.set_timer(T_ANIM, ms);
        }
        self.anim_ms = ms;
    }

    /// One display refresh while animating (posted by the pump).
    pub fn on_frame(&mut self) {
        pump::handled();
        if self.anim_ms == 16 {
            self.on_anim_tick();
        }
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
        // The hover squash grows the pill a few points past its resting size, so the
        // window gets a little spare room (sides and bottom) while it plays.
        let squash = !self.expanded && self.jiggle > 0.0;
        let pad = if squash { (6.0 * self.scale).round() as i32 } else { 0 };
        let padb = if squash { (3.0 * self.scale).round() as i32 } else { 0 };
        let pw = (w * self.scale).round() as i32 + 2 * pad;
        let ph = (h * self.scale).round() as i32 + padb;
        if self.gfx.begin(pw, ph).is_err() {
            return;
        }
        let mut hits = std::mem::take(&mut self.hits);
        hits.clear();
        if squash {
            let (sx, sy) = crate::ui::squash(self.jiggle);
            let g = &self.gfx;
            g.shifted(pad as f32 / self.scale, 0.0, || {
                g.scaled(w / 2.0, 0.0, sx, sy, || crate::ui::draw(self, w, h, &mut hits))
            });
        } else {
            crate::ui::draw(self, w, h, &mut hits);
        }
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
        self.jiggle = (self.jiggle - dt * 2.5).max(0.0); // the 0.4 s hover squash
        if self.eq_active() {
            // music glow follows the real output level (with a slow release)
            let peak = sys::audio_peak().unwrap_or(0.5 + 0.4 * self.eq_phase.sin());
            self.level = peak.max(self.level - dt * 2.2);
        } else {
            self.level = (self.level - dt * 4.0).max(0.0);
        }
        if self.page_t < 1.0 {
            self.page_t = (self.page_t + dt / PAGE_FADE_S).min(1.0);
        }
        let (tw, th) = self.target_size();
        let dt0 = dt;
        while dt > 0.0 {
            let h = dt.min(1.0 / 240.0);
            self.sw.step(tw, h);
            self.sh.step(th, h);
            dt -= h;
        }
        if self.skip.is_some() {
            self.spin_t += dt0;
            if self.skip_done() {
                self.skip = None;
            }
        }
        let ui_busy = self.step_ui(dt0) || self.skip.is_some();
        let moving = !(self.sw.settled(tw) && self.sh.settled(th)) || self.page_t < 1.0 || ui_busy;
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
            T_DOTS => {
                self.kill_timer(T_DOTS);
                self.cycle_until = None;
                if !self.expanded && !self.hidden {
                    self.render();
                }
            }
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
                self.smart_tick();
                if self.bar_visible() {
                    self.refresh_bar_data();
                    self.render_bar();
                }
                // a meeting entering or leaving the window changes the glance width
                let gx = self.glance_extra_w();
                if (gx - self.glance_last).abs() > 0.5 {
                    self.glance_last = gx;
                    if !self.expanded && !self.hidden {
                        self.retarget();
                    }
                }
                if !self.expanded && (self.call_since.is_some() || (!self.has_media() && !self.timer_running())) {
                    self.render();
                }
            }
            T_LEAVE => {
                self.kill_timer(T_LEAVE);
                if self.expanded && !self.pinned && self.entering.is_none() && self.drag.is_none() && !self.shelf_out && self.cfg.auto_close {
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
            self.page_t = 1.0;
            self.finish_entry(false, false);
            self.drag = None;
            self.seek_frac = None;
            self.hot = None;
            self.hover.clear();
        }
        if on {
            self.snap_tab();
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

    /// Tray: hide while a full-screen app is in front (on by default).
    pub fn toggle_fs_hide(&mut self) {
        self.cfg.fs_hide = !self.cfg.fs_hide;
        crate::store::save_config(&self.cfg);
        self.on_foreground();
    }

    /// Tray: hide the notch until the pointer touches the top edge.
    pub fn toggle_auto_hide(&mut self) {
        self.cfg.auto_hide = !self.cfg.auto_hide;
        crate::store::save_config(&self.cfg);
        self.on_foreground();
    }

    /// Tray: close the open panel automatically (on by default) or keep it until closed.
    pub fn toggle_auto_close(&mut self) {
        self.cfg.auto_close = !self.cfg.auto_close;
        crate::store::save_config(&self.cfg);
    }

    /// Tray: move the notch and bar to another display (0 = primary).
    pub fn set_monitor(&mut self, idx: u8) {
        self.cfg.monitor = idx;
        store::save_config(&self.cfg);
        self.on_display_change(sys::scale_of(sys::pick_monitor(idx)));
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
        if self.cfg.stats || self.glance_kind() == 3 {
            self.cpu_prev = sys::cpu_times();
            self.set_timer(T_STATS, 3000);
        } else {
            self.kill_timer(T_STATS);
            self.stats = None;
        }
    }

    /// Tray: show 13:05 instead of 1:05 PM (on the bar, the pill and the clock pop-up).
    pub fn toggle_clock24(&mut self) {
        self.cfg.clock_24h = !self.cfg.clock_24h;
        store::save_config(&self.cfg);
        self.render_bar();
        self.render();
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

    /// Pulse a short message on the collapsed pill (skipped while the panel is open or a timer runs).
    fn pill_alert(&mut self, msg: &'static str, secs: u64) {
        if !self.expanded && !self.timer_running() {
            self.alert_msg = msg;
            self.alert_act = None;
            self.alert_swatch = None;
            self.alert_label.clear();
            self.alert_start = None;
            self.alert_until = Some(Instant::now() + std::time::Duration::from_secs(secs));
            self.retarget();
            self.render();
        }
    }

    /// Show or hide the call pill when the microphone starts or stops being used.
    pub fn update_call(&mut self) {
        let on = self.cfg.smart && sys::mic_in_use();
        if on != self.call_since.is_some() {
            self.call_since = on.then(Instant::now);
            self.mic_muted = on && sys::mic_muted();
            if !self.expanded && !self.timer_running() {
                self.retarget();
                self.render();
            }
        }
    }

    /// Re-read the calendar link about hourly and announce a meeting that starts within 5 minutes.
    fn meeting_hint(&mut self) {
        self.cal_age = self.cal_age.saturating_add(1);
        if self.cal_age >= 60 {
            self.cal_age = 0;
            crate::cal::refresh(&self.cfg.calendar_url);
        }
        let (now, _) = crate::cal::now();
        let Some(e) = crate::cal::soon(&crate::cal::EVENTS.lock().unwrap(), now).cloned() else { return };
        let mins = (e.start - now + 59).div_euclid(60);
        // stage 1: five minutes out; stage 2: the last minute
        let stage = if mins <= 1 { 2 } else { 1 };
        if self.cal_seen == Some((e.start, stage)) {
            return;
        }
        self.cal_seen = Some((e.start, stage));
        let short: String = if e.title.chars().count() > 24 { e.title.chars().take(23).chain(std::iter::once('\u{2026}')).collect() } else { e.title.clone() };
        self.buddy_say(&format!("{} starts {}", e.title, if mins > 0 { format!("in {mins} min") } else { "now".to_string() }), 7000);
        // stays up until the meeting has started (or you click it)
        self.pill_alert("Meeting", (e.start - now + 60).clamp(30, 900) as u64);
        if self.alert_msg == "Meeting" {
            self.alert_label = short;
            self.alert_start = Some(e.start);
            self.alert_act = e.link.map(AlertAct::Open);
        }
        self.chime();
    }

    /// The Windows notification sound (follows the user's sound scheme and volume).
    fn chime(&self) {
        if self.cfg.smart_sound {
            unsafe {
                let _ = MessageBeep(MB_ICONASTERISK);
            }
        }
    }

    pub fn toggle_smart_sound(&mut self) {
        self.cfg.smart_sound = !self.cfg.smart_sound;
        store::save_config(&self.cfg);
        self.chime();
    }

    pub fn toggle_smart(&mut self) {
        self.cfg.smart = !self.cfg.smart;
        store::save_config(&self.cfg);
    }

    /// Once a minute: warn on low battery, and nudge a break after 90 minutes of steady use.
    fn smart_tick(&mut self) {
        self.update_call();
        if !self.cfg.smart {
            return;
        }
        if let Some(b) = sys::battery_info() {
            if b.plugged || b.charging || b.percent > 20 {
                self.batt_warned = false;
            } else if b.percent <= 15 && !self.batt_warned {
                self.batt_warned = true;
                self.buddy_say("Battery is low, find a charger", 6000);
                self.pill_alert("Battery low", 60);
                self.chime();
            }
        }
        if !self.cfg.calendar_url.is_empty() {
            self.meeting_hint();
        }
        // a 5 minute gap in typing or mouse use counts as a break
        self.active_min = if sys::idle_secs() < 300 { self.active_min + 1 } else { 0 };
        if self.active_min >= 90 {
            self.active_min = 0;
            self.buddy_say("You have been at it for 90 minutes. Stretch!", 7000);
            self.pill_alert("Take a break", 60);
            self.chime();
        }
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
        // sampled for the bar read-out and/or the idle glance, never otherwise
        let on_bar = self.cfg.stats && self.bar_visible();
        let on_pill = self.glance_kind() == 3 && !self.hidden;
        if !on_bar && !on_pill {
            return;
        }
        let (idle, total) = sys::cpu_times();
        let (di, dt) = (idle.wrapping_sub(self.cpu_prev.0), total.wrapping_sub(self.cpu_prev.1));
        self.cpu_prev = (idle, total);
        let cpu = if dt == 0 { 0 } else { (100 - (di.min(dt) * 100 / dt)) as u32 };
        let now = Some((cpu, sys::mem_percent()));
        if now != self.stats {
            let first = self.stats.is_none();
            self.stats = now;
            if on_bar {
                self.render_bar();
            }
            if on_pill && !self.expanded {
                // the slot only changes width when the first sample arrives
                if first {
                    self.retarget();
                }
                self.render();
            }
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
                self.set_timer(T_HOVER, 120);
            }
            return;
        }
        if let Some((i, x0, y0)) = self.shelf_press {
            if unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } >= 0 {
                self.shelf_press = None; // released somewhere we did not see
            } else if (x - x0).hypot(y - y0) > 6.0 {
                self.shelf_press = None;
                self.shelf_drag_out(i);
                return;
            }
        }
        if self.drag.is_some() {
            self.apply_drag(x);
            self.render();
            return;
        }
        let hot = self.hit_at(x, y);
        if hot != self.hot {
            self.hot = hot;
            // the hover fade runs on the frame clock
            if self.anim_ms != 16 {
                self.retarget();
            }
        }
    }

    pub fn on_mouse_leave(&mut self) {
        self.tracking = false;
        self.kill_timer(T_HOVER);
        if self.hot.is_some() {
            self.hot = None;
            if self.expanded && self.anim_ms != 16 {
                self.retarget();
            }
        }
        if self.expanded && !self.pinned && self.entering.is_none() && self.drag.is_none() && !self.shelf_out && self.cfg.auto_close {
            self.set_timer(T_LEAVE, 450);
        }
    }

    pub fn on_lbutton_down(&mut self, px: i32, py: i32) {
        self.mouse_down = true;
        let (x, y) = (px as f32 / self.scale, py as f32 / self.scale);
        if !self.expanded && !self.alerting() && !self.timer_running() && self.call_since.is_some() && x > self.sw.x / 2.0 - 58.0 && x < self.sw.x / 2.0 - 18.0 {
            self.mic_muted = sys::toggle_mic_mute();
            self.render();
            return;
        }
        if !self.expanded && self.alerting() {
            match (self.alert_msg, self.alert_act.take()) {
                ("Link copied" | "Meeting", Some(AlertAct::Open(u))) => {
                    sys::open(&u, "");
                    if let Some(s) = self.alert_start {
                        self.cal_seen = Some((s, 2)); // joined: no second reminder
                    }
                    self.alert_until = None;
                    self.retarget();
                    return;
                }
                ("Path copied", Some(AlertAct::Reveal(p))) => {
                    sys::open("explorer.exe", &format!("/select,\"{}\"", p));
                    self.alert_until = None;
                    self.retarget();
                    return;
                }
                _ => {}
            }
        }
        if !self.expanded {
            self.squish = 1.0;
            self.retarget();
            self.set_expanded(true);
            return;
        }
        if let Some(act) = self.hit_at(x, y) {
            if let Act::ShelfItem(i) = act {
                // a click opens the file, a drag takes it out: decided on release or move
                self.shelf_press = Some((i, x, y));
            } else {
                self.do_act(act, x);
            }
        } else if self.entering.is_some() {
            // clicking elsewhere inside the notch keeps the text box open
        }
        if self.anim_ms == 0 {
            self.render();
        }
    }

    pub fn on_lbutton_up(&mut self) {
        self.mouse_down = false;
        if let Some((i, ..)) = self.shelf_press.take() {
            self.shelf_open(i);
            return;
        }
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
                // scrolling down opens the panel, as always
                self.set_expanded(true);
            } else if delta > 0 && self.cfg.idle_cycle && self.idle_pill() {
                // scrolling up cycles the idle extra
                self.cycle_next();
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
        self.prev_page = self.page;
        self.page = p;
        self.page_t = if self.expanded { 0.0 } else { 1.0 };
        if p == Page::Controls {
            self.refresh_controls();
        }
        if p != Page::Apps {
            self.picking = false;
            self.app_filter.clear();
        }
        self.sync_timers();
        if self.page_t < 1.0 {
            self.retarget(); // runs the cross-fade, and renders the first frame
        } else {
            self.render();
        }
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
                self.begin_skip(false);
                let _ = self.media.tx.send(Cmd::Prev);
            }
            Act::PlayPause => {
                let _ = self.media.tx.send(Cmd::PlayPause);
            }
            Act::Next => {
                self.begin_skip(true);
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
            Act::ShelfItem(i) => self.shelf_open(i),
            Act::ShelfDel(i) => {
                if i < self.shelf.len() {
                    self.shelf.remove(i);
                    self.shelf_changed();
                }
            }
            Act::ShelfClear => {
                self.shelf.clear();
                self.shelf_changed();
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
                self.retarget(); // the bars glide to the new values
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
                // Ctrl+Backspace arrives as DEL: delete the word before the cursor
                0x7F => {
                    delete_word(buf);
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

    /// Previous / Next was pressed: remember the current track so the panel can show
    /// "loading" over it until the new one arrives.
    fn begin_skip(&mut self, next: bool) {
        if self.mstate.has_track() {
            self.skip = Some(Skip { t0: Instant::now(), next, snap: self.mstate.clone(), art: self.art.clone() });
            self.spin_t = 0.0;
            self.retarget();
        }
    }

    /// The press has been answered (a different track, or the same one restarted) or took too long.
    fn skip_done(&self) -> bool {
        let Some(s) = &self.skip else { return false };
        let m = &self.mstate;
        let moved = m.has_track() && (m.title != s.snap.title || m.pos_ticks + 20_000_000 < s.snap.pos_ticks);
        moved || s.t0.elapsed().as_secs_f32() > 4.0
    }

    /// The track change that is still loading, if any.
    pub fn skip_loading(&self) -> Option<&Skip> {
        self.skip.as_ref().filter(|s| s.t0.elapsed().as_secs_f32() <= 4.0)
    }

    pub fn on_media(&mut self) {
        let st = self.media.state.lock().unwrap().clone();
        if st.art_gen != self.art_gen {
            self.art_gen = st.art_gen;
            self.art_bytes = st.art.clone();
            self.load_art();
        }
        let started = st.playing && !self.mstate.playing;
        self.mstate = st;
        if self.skip_done() {
            self.skip = None;
        }
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

    // ------------------------------------------------------ motion polish --

    /// Put the tab highlight straight under the current page (no slide).
    pub(crate) fn snap_tab(&mut self) {
        if let Some(i) = PAGES.iter().position(|p| *p == self.page) {
            let r = crate::ui::tab_rects()[i];
            self.tab_x = r.x;
            self.tab_w = r.w;
        }
    }

    /// Ease the tab highlight, the stat bars and the hover fades toward where they
    /// belong. Returns true while any of them is still moving.
    fn step_ui(&mut self, dt: f32) -> bool {
        let fading = self.step_fade(dt);
        if !self.expanded {
            return fading;
        }
        let mut busy = false;
        if let Some(i) = PAGES.iter().position(|p| *p == self.page) {
            let r = crate::ui::tab_rects()[i];
            let k = (dt * 20.0).min(1.0);
            self.tab_x += (r.x - self.tab_x) * k;
            self.tab_w += (r.w - self.tab_w) * k;
            if (r.x - self.tab_x).abs() > 0.2 || (r.w - self.tab_w).abs() > 0.2 {
                busy = true;
            } else {
                self.tab_x = r.x;
                self.tab_w = r.w;
            }
        }
        if self.page == Page::Stats {
            let k = (dt * 8.0).min(1.0);
            for (shown, real) in [(&mut self.bar_cpu, self.stat_cpu), (&mut self.bar_mem, self.stat_mem)] {
                let want = real as f32 / 100.0;
                *shown += (want - *shown) * k;
                if (want - *shown).abs() > 0.003 {
                    busy = true;
                } else {
                    *shown = want;
                }
            }
        }
        let hot = self.hot;
        if let Some(h) = hot {
            if !self.hover.iter().any(|(a, _)| *a == h) {
                self.hover.push((h, 0.0));
            }
        }
        let k = (dt * 14.0).min(1.0);
        for (a, v) in self.hover.iter_mut() {
            let target = if Some(*a) == hot { 1.0 } else { 0.0 };
            *v += (target - *v) * k;
        }
        self.hover.retain(|(a, v)| Some(*a) == hot || *v > 0.02);
        busy || fading || self.hover.iter().any(|(a, v)| if Some(*a) == hot { *v < 0.98 } else { true })
    }

    fn apply_fade(&self) {
        self.gfx.set_alpha(self.hwnd, self.fade);
        self.set_bar_alpha(self.fade);
    }

    /// Fade the notch and bar in or out (0.18 s). When a fade-out ends, the hide
    /// that was waiting for it goes ahead.
    fn step_fade(&mut self, dt: f32) -> bool {
        if self.fade == self.fade_to {
            return false;
        }
        let step = dt / 0.18;
        self.fade = if self.fade_to > self.fade { (self.fade + step).min(self.fade_to) } else { (self.fade - step).max(self.fade_to) };
        self.apply_fade();
        if self.fade == 0.0 && self.fade_to == 0.0 {
            self.fade_out_done = true;
            self.on_foreground();
            self.fade_out_done = false;
            return false;
        }
        self.fade != self.fade_to
    }

    /// Does `w` cover the notch? Only with the smart-hide option, and never while the
    /// panel is open or for our own and shell windows.
    fn overlaps_notch(&self, w: HWND) -> bool {
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
        if !self.cfg.smart_hide || self.expanded || w.is_invalid() || w == self.hwnd {
            return false;
        }
        if self.is_bar(w) || self.is_pop(w) || self.is_buddy(w) || self.is_stage(w) {
            return false;
        }
        unsafe {
            if !IsWindowVisible(w).as_bool() || IsIconic(w).as_bool() {
                return false;
            }
            if matches!(sys::class_of(w).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
                return false;
            }
            let mut r = RECT::default();
            // the visible frame, without the invisible resize border
            if DwmGetWindowAttribute(w, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_err() {
                let _ = GetWindowRect(w, &mut r);
            }
            let cx = (self.mon.left + self.mon.right) / 2;
            let half = ((self.sw.x * self.scale) / 2.0) as i32;
            let bottom = self.mon.top + (self.col_h() * self.scale) as i32;
            r.top < bottom && r.bottom > self.mon.top && r.left < cx + half && r.right > cx - half
        }
    }

    /// Tray: hide while a window covers the notch, show again when it moves away.
    pub fn toggle_smart_hide(&mut self) {
        self.cfg.smart_hide = !self.cfg.smart_hide;
        store::save_config(&self.cfg);
        self.on_foreground();
    }

    // ------------------------------------------------------------- shelf --

    fn load_shelf_icons(&mut self) {
        self.shelf_icons = self
            .shelf
            .iter()
            .map(|p| {
                let icon = sys::file_icon(std::path::Path::new(p))?;
                let bmp = self.gfx.bitmap_from_hicon(icon).ok();
                unsafe {
                    let _ = DestroyIcon(icon);
                }
                bmp
            })
            .collect();
    }

    fn shelf_changed(&mut self) {
        store::save_shelf(&self.shelf);
        self.load_shelf_icons();
    }

    pub fn shelf_dragging(&self) -> bool {
        self.shelf_out
    }

    /// A file drag from another program reached the notch: open the Shelf tab.
    pub fn on_shelf_enter(&mut self) {
        self.shelf_hover = true;
        if !self.expanded {
            self.shelf_auto_open = true;
            self.tucked = false;
            self.set_expanded(true);
        }
        self.select_page(Page::Shelf);
        if self.anim_ms == 0 {
            self.render();
        }
    }

    /// The drag left without dropping: fold the panel again if the drag opened it.
    pub fn on_shelf_leave(&mut self) {
        self.shelf_hover = false;
        if std::mem::take(&mut self.shelf_auto_open) && !self.pinned {
            self.set_expanded(false);
        } else if self.anim_ms == 0 {
            self.render();
        }
    }

    pub fn on_shelf_drop(&mut self, paths: Vec<String>) {
        self.shelf_hover = false;
        self.shelf_auto_open = false;
        for p in paths {
            if self.shelf.len() < SHELF_MAX && !self.shelf.contains(&p) {
                self.shelf.push(p);
            }
        }
        self.shelf_changed();
        self.render();
    }

    fn shelf_open(&mut self, i: usize) {
        let Some(p) = self.shelf.get(i).cloned() else { return };
        if !std::path::Path::new(&p).exists() {
            self.shelf.remove(i);
            self.shelf_changed();
            return;
        }
        sys::open(&p, "");
        self.pinned = false;
        self.set_expanded(false);
    }

    /// Drag the i-th shelf file out into another program.
    fn shelf_drag_out(&mut self, i: usize) {
        let Some(p) = self.shelf.get(i).cloned() else { return };
        self.shelf_out = true;
        crate::shelf::drag_out(self.hwnd, &p);
        self.shelf_out = false;
        self.mouse_down = false;
        // the target may have moved the file away
        if !std::path::Path::new(&p).exists() {
            self.shelf.retain(|s| *s != p);
            self.shelf_changed();
        }
        unsafe {
            let (mut pt, mut r) = (POINT::default(), RECT::default());
            let _ = GetCursorPos(&mut pt);
            let _ = GetWindowRect(self.hwnd, &mut r);
            if !(pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom) {
                self.on_mouse_leave();
            }
        }
        self.render();
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
        if let Some(h) = self.cfg.smart.then(|| clip_hint(&text)).flatten() {
            self.pill_alert(h.msg, 4);
            if self.alert_msg == h.msg {
                self.alert_act = h.act;
                self.alert_swatch = h.swatch;
            }
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
        // dragging a window up to the top edge must not peek the notch back
        if self.smart_hit && unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0 {
            return;
        }
        if self.fs_peekable && !self.peek {
            // short dwell so a stray flick does nothing; shorter when the notch is
            // meant to live behind the edge
            self.set_timer(T_EDGE, if self.cfg.auto_hide { 80 } else { 250 });
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
        // smart-hide reacts at once (a cheap rectangle test), not after the 150 ms debounce
        if self.cfg.smart_hide {
            let hit = self.overlaps_notch(unsafe { GetForegroundWindow() });
            if hit != self.smart_hit {
                self.on_foreground();
                return;
            }
        }
        self.set_timer(T_FSCHK, 150);
    }

    pub fn on_foreground(&mut self) {
        self.update_call();
        // Never hide because of our own window (typing a task makes us the
        // foreground window), and never hide while a task is being typed.
        let fgw = unsafe { GetForegroundWindow() };
        let ours = fgw == self.hwnd || self.pop.as_ref().map(|p| p.hwnd) == Some(fgw);
        self.smart_hit = self.overlaps_notch(fgw);
        let kind = if self.tucked || ours || self.entering.is_some() {
            0
        } else if self.cfg.auto_hide || self.smart_hit {
            1 // hidden, but the top edge peeks it back (same as a full-screen app)
        } else if !self.cfg.fs_hide {
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
        if fs && !self.hidden && !self.fade_out_done {
            // fade out first; this runs again once the fade has finished
            self.fade_to = 0.0;
            self.retarget();
            return;
        }
        let fs_changed = fs_all != self.fs_hidden;
        self.fs_hidden = fs_all;
        sys::edge_watch(self.fs_peekable, self.hwnd, self.edge_zone());
        // Clicking into any other window closes an open dropdown / unpinned panel.
        if !ours && self.entering.is_none() {
            self.close_pop();
            if self.expanded && !self.pinned && self.drag.is_none() && !self.cursor_inside() && self.cfg.auto_close {
                // the "it closed by itself" case: say which window caused it
                sys::log_visibility(&format!("panel closed: another window came to the front (class '{}')", sys::class_of(fgw)));
                self.set_expanded(false);
            }
        }
        if fs != self.hidden {
            self.hidden = fs;
            unsafe {
                let _ = ShowWindow(self.hwnd, if fs { SW_HIDE } else { SW_SHOWNOACTIVATE });
            }
            if !fs {
                self.fade = 0.0; // fades in from nothing
                self.apply_fade();
                self.render();
            }
            self.sync_timers();
            self.sync_bar();
        }
        if !fs && self.fade_to < 1.0 {
            self.fade_to = 1.0;
            self.retarget();
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
struct Hint {
    msg: &'static str,
    act: Option<AlertAct>,
    swatch: Option<Color>,
}

/// What the pill says about something just copied: a link, a colour, a file path or
/// a short numeric (one-time) code. Links and paths can be opened by clicking the pill.
fn clip_hint(t: &str) -> Option<Hint> {
    let t = t.trim();
    let hint = |msg, act, swatch| Some(Hint { msg, act, swatch });
    if t.starts_with("http://") || t.starts_with("https://") {
        return hint("Link copied", Some(AlertAct::Open(t.to_string())), None);
    }
    if (4..=8).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit()) {
        return hint("Code copied", None, None);
    }
    if let Some(c) = hex_colour(t) {
        return hint("Colour copied", None, Some(c));
    }
    let b = t.as_bytes();
    if t.len() > 3 && b[0].is_ascii_alphabetic() && &t[1..3] == ":\\" && !t.contains('\n') && !t.contains('"') {
        return hint("Path copied", Some(AlertAct::Reveal(t.to_string())), None);
    }
    None
}

/// `#rgb` or `#rrggbb`.
fn hex_colour(t: &str) -> Option<Color> {
    let h = t.strip_prefix('#')?;
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    let (r, g, b) = match h.len() {
        3 => (((v >> 8) & 15) * 17, ((v >> 4) & 15) * 17, (v & 15) * 17),
        6 => ((v >> 16) & 255, (v >> 8) & 255, v & 255),
        _ => return None,
    };
    Some(Color(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0))
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
/// Ctrl+Backspace in a text box: drop trailing spaces, then the word before them.
fn delete_word(buf: &mut Vec<u16>) {
    while buf.last() == Some(&0x20) {
        buf.pop();
    }
    while buf.last().is_some_and(|c| *c != 0x20) {
        buf.pop();
    }
}

#[cfg(test)]
mod smart_tests {
    use super::*;

    #[test]
    fn ctrl_backspace_deletes_a_word() {
        let run = |s: &str| {
            let mut b: Vec<u16> = s.encode_utf16().collect();
            delete_word(&mut b);
            String::from_utf16_lossy(&b)
        };
        assert_eq!(run("Terminal"), "");
        assert_eq!(run("open the Terminal"), "open the ");
        assert_eq!(run("open the  "), "open ");
        assert_eq!(run(""), "");
    }

    fn msg(t: &str) -> Option<&'static str> {
        clip_hint(t).map(|h| h.msg)
    }

    #[test]
    fn clip_hints() {
        assert_eq!(msg("https://example.com"), Some("Link copied"));
        assert_eq!(msg(" 482913 "), Some("Code copied"));
        assert_eq!(msg("#1e90ff"), Some("Colour copied"));
        assert_eq!(msg("#fff"), Some("Colour copied"));
        assert_eq!(msg("C:\\Users\\me\\a.txt"), Some("Path copied"));
        assert_eq!(msg("#12"), None);
        assert_eq!(msg("123"), None);
        assert_eq!(msg("123456789"), None);
        assert_eq!(msg("hello"), None);
    }
}

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
            a.snap_tab();
            (a.stat_cpu, a.stat_mem, a.bar_cpu, a.bar_mem) = (23, 61, 0.23, 0.61);
            render(&a, &name);
        }
        // Next pressed: the old track stays up with a spinner while the player loads
        a.page = Page::Media;
        a.snap_tab();
        a.skip = Some(Skip { t0: Instant::now(), next: true, snap: a.mstate.clone(), art: None });
        a.spin_t = 0.3;
        render(&a, "media_loading");
        a.skip = None;
        // the shelf with files on it, and while a file is dragged over
        a.page = Page::Shelf;
        a.shelf = ["Cargo.toml", "README.md", "docs", "src\\main.rs", "LICENSE"]
            .iter()
            .map(|f| std::env::current_dir().unwrap().join(f).to_string_lossy().into_owned())
            .collect();
        a.load_shelf_icons();
        render(&a, "shelf_full");
        a.shelf_hover = true;
        render(&a, "shelf_drop");
        a.shelf_hover = false;
        a.shelf.clear();
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
        (a.bar_cpu, a.bar_mem) = (0.23, 0.61);
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
            a.snap_tab();
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
        for id in [12usize, 13, 14, 6] {
            let steps = (crate::scene::demo_len(id) * 10.0) as usize + 4;
            for k in 0..steps {
                let t = k as f32 * 0.1;
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
                std::fs::write(format!("{dir}/s{id}_{k:03}.raw"), out).unwrap();
            }
        }
    }


    /// Real-UI pieces for the demo video: the full bar, dropdowns, every page, buddies.
    /// PREVIEW_DIR=... cargo test --release preview::render_demo_assets -- --ignored
    #[test]
    #[ignore]
    fn render_demo_assets() {
        use crate::bar::{Bar, BarAct};
        use crate::pop::Pop;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let dir = std::env::var("PREVIEW_DIR").expect("PREVIEW_DIR");
        let scale = 3.0f32;
        let save = |name: &str, w: i32, h: i32, stride: i32, buf: &[u8]| {
            let mut out = Vec::new();
            out.extend_from_slice(&(w as u32).to_le_bytes());
            out.extend_from_slice(&(h as u32).to_le_bytes());
            for y in 0..h {
                let row = (y * stride * 4) as usize;
                out.extend_from_slice(&buf[row..row + (w * 4) as usize]);
            }
            std::fs::write(format!("{dir}/{name}.raw"), out).unwrap();
        };
        let mut a = App::new(HWND::default(), scale).expect("app");
        a.mon = RECT { left: 0, top: 0, right: (1920.0 * scale) as i32, bottom: (1080.0 * scale) as i32 };
        a.cfg.top_bar = true;
        a.hidden = false;
        a.has_volume = true;
        a.volume = 0.62;
        a.brightness = Some(70);
        a.radios.wifi = Some(true);
        a.radios.bluetooth = Some(true);
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
        a.mstate.has_session = true;
        a.mstate.title = "Midnight City".into();
        a.mstate.artist = "M83".into();
        a.mstate.app = "Spotify".into();
        a.mstate.playing = true;
        a.mstate.dur_ticks = 243 * 10_000_000;
        a.mstate.pos_ticks = 102 * 10_000_000;
        a.mstate.can_seek = true;
        a.mstate.can_prev = true;
        a.mstate.can_next = true;
        a.stat_cpu = 23;
        a.stat_mem = 61;
        (a.bar_cpu, a.bar_mem) = (0.23, 0.61);
        a.stat_hist = (0..40).map(|i| 0.15 + 0.12 * ((i as f32) * 0.5).sin().abs() + (i as f32 % 7.0) * 0.02).collect();
        a.stats = None;

        // ---- the bar, plain and with each icon hovered / its dropdown open
        a.bar = Bar::new(HWND::default(), a.mon.right, scale);
        {
            let b = a.bar.as_mut().unwrap();
            b.app_name = "Visual Studio Code".into();
            b.battery = Some((82, false));
        }
        for (name, hot) in [("bar_idle", None), ("bar_wifi", Some(BarAct::Wifi)), ("bar_bt", Some(BarAct::Bluetooth)), ("bar_vol", Some(BarAct::Volume)), ("bar_clock", Some(BarAct::Clock))] {
            a.bar.as_mut().unwrap().hot = hot;
            a.render_bar();
            let (bw, _bh, buf) = a.bar.as_ref().unwrap().snapshot();
            save(name, a.mon.right, (crate::bar::BAR_H * scale).round() as i32, bw, &buf);
        }
        // icon centres (logical px) so the video can aim the cursor
        let hits: Vec<String> = a
            .bar
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .map(|(r, k)| {
                let n = match k {
                    BarAct::Wifi => "wifi",
                    BarAct::Bluetooth => "bluetooth",
                    BarAct::Volume => "volume",
                    BarAct::Brightness => "brightness",
                    BarAct::Battery => "battery",
                    BarAct::Clock => "clock",
                };
                format!("\"{n}\": [{:.1}, {:.1}]", r.cx(), r.cy())
            })
            .collect();

        // ---- dropdowns
        {
            let mut w = sys::WIFI.lock().unwrap();
            w.connected = Some("Home-5G".into());
            w.nets = vec![
                sys::WifiNet { ssid: "Home-5G".into(), signal: 92, secure: true, saved: true, auth: "WPA2-Personal".into() },
                sys::WifiNet { ssid: "Office-Net".into(), signal: 78, secure: true, saved: true, auth: "WPA2-Personal".into() },
                sys::WifiNet { ssid: "Cafe Guest".into(), signal: 64, secure: false, saved: false, auth: "Open".into() },
                sys::WifiNet { ssid: "Neighbour_2.4".into(), signal: 41, secure: true, saved: false, auth: "WPA2-Personal".into() },
            ];
        }
        {
            let mut d = a.radio.devices.lock().unwrap();
            *d = vec![
                crate::radio::BtDevice { name: "Sony WH-1000XM5".into(), connected: true, address: 1 },
                crate::radio::BtDevice { name: "Keychron K3".into(), connected: false, address: 2 },
                crate::radio::BtDevice { name: "Galaxy Buds".into(), connected: false, address: 3 },
                crate::radio::BtDevice { name: "Pixel 8".into(), connected: false, address: 4 },
            ];
        }
        a.pop = Pop::new(HWND::default(), scale);
        for (name, kind) in [("pop_wifi", BarAct::Wifi), ("pop_bt", BarAct::Bluetooth), ("pop_vol", BarAct::Volume), ("pop_bri", BarAct::Brightness), ("pop_battery", BarAct::Battery), ("pop_clock", BarAct::Clock)] {
            a.pop.as_mut().unwrap().kind = Some(kind);
            a.render_pop();
            let (w, h, stride, buf) = a.pop.as_ref().unwrap().demo_snapshot(scale);
            save(name, w, h, stride, &buf);
        }

        // ---- the pill and every page (600 x 250 logical)
        let (pw, ph) = ((EXP_W * scale) as i32, (EXP_H * scale) as i32);
        let page_shot = |a: &App, name: &str| {
            a.gfx.begin(pw, ph).unwrap();
            let mut hits = Vec::new();
            crate::ui::draw(a, EXP_W, EXP_H, &mut hits);
            let _ = a.gfx.end();
            let (bw, _bh, buf) = a.gfx.snapshot();
            save(name, pw, ph, bw, &buf);
            hits
        };
        a.expanded = true;
        a.sw = Spring::new(EXP_W);
        a.sh = Spring::new(EXP_H);
        let mut tabs: Vec<String> = Vec::new();
        for p in PAGES.iter() {
            a.page = *p;
            let hits = page_shot(&a, &format!("page_{}", format!("{p:?}").to_lowercase()));
            if tabs.is_empty() {
                for h in &hits {
                    if let Act::Tab(t) = h.act {
                        tabs.push(format!("\"{}\": [{:.1}, {:.1}]", format!("{t:?}").to_lowercase(), h.r.cx(), h.r.cy()));
                    }
                    if matches!(h.act, Act::Collapse) {
                        tabs.push(format!("\"collapse\": [{:.1}, {:.1}]", h.r.cx(), h.r.cy()));
                    }
                }
            }
        }
        // Today: typing a task, ticking it, a running focus timer
        a.page = Page::Tasks;
        let text = "Ship the demo";
        for (i, k) in [0usize, 4, 8, 13].iter().enumerate() {
            a.entering = Some(text.chars().take(*k).collect::<String>().encode_utf16().collect());
            page_shot(&a, &format!("today_type{i}"));
        }
        a.entering = None;
        a.todos.insert(0, Todo { done: false, text: text.into() });
        page_shot(&a, "today_added");
        a.todos[1].done = true;
        page_shot(&a, "today_ticked");
        a.timer_task = Some(text.into());
        a.timer_left_ms = 24 * 60 * 1000 + 41_000;
        a.timer_start = Some(Instant::now());
        page_shot(&a, "today_timer");
        // the collapsed pills (idle ones show the clock, so no music session yet)
        a.mstate.has_session = false;
        a.expanded = false;
        a.sh = Spring::new(a.col_h());
        let (cw, chh) = ((COL_TIMER_W * scale) as i32, (a.col_h() * scale) as i32);
        a.gfx.begin(cw, chh).unwrap();
        let mut h2 = Vec::new();
        crate::ui::draw(&a, COL_TIMER_W, a.col_h(), &mut h2);
        let _ = a.gfx.end();
        let (bw, _bh, buf) = a.gfx.snapshot();
        save("pill_timer", cw, chh, bw, &buf);
        a.timer_start = None;
        a.timer_left_ms = 25 * 60 * 1000;
        let (iw, ih) = ((COL_IDLE_W * scale) as i32, (a.col_h() * scale) as i32);
        a.gfx.begin(iw, ih).unwrap();
        let mut h3 = Vec::new();
        crate::ui::draw(&a, COL_IDLE_W, a.col_h(), &mut h3);
        let _ = a.gfx.end();
        let (bw, _bh, buf) = a.gfx.snapshot();
        save("pill_idle", iw, ih, bw, &buf);
        // idle glance variants and the focus progress line
        for (kind, name) in [(1u8, "pill_glance_tasks"), (2, "pill_glance_focus")] {
            a.cfg.idle_glance = kind;
            a.focus_today = 42;
            let gw = ((COL_IDLE_W + a.glance_extra_w()) * scale) as i32;
            a.gfx.begin(gw, ih).unwrap();
            let mut hg = Vec::new();
            crate::ui::draw(&a, COL_IDLE_W + a.glance_extra_w(), a.col_h(), &mut hg);
            let _ = a.gfx.end();
            let (bw, _bh, buf) = a.gfx.snapshot();
            save(name, gw, ih, bw, &buf);
        }
        a.cfg.idle_glance = 3;
        a.stats = Some((12, 63));
        let gw = ((COL_IDLE_W + a.glance_extra_w()) * scale) as i32;
        a.gfx.begin(gw, ih).unwrap();
        let mut hg = Vec::new();
        crate::ui::draw(&a, COL_IDLE_W + a.glance_extra_w(), a.col_h(), &mut hg);
        let _ = a.gfx.end();
        let (bw, _bh, buf) = a.gfx.snapshot();
        save("pill_glance_cpu", gw, ih, bw, &buf);
        // next meeting, date page with dots, and the workday line
        {
            let (now, _) = crate::cal::now();
            *crate::cal::EVENTS.lock().unwrap() = vec![crate::cal::Event { start: now + 90 * 60, title: "Quarterly planning review".into(), link: None }];
        }
        a.cfg.idle_glance = 4;
        let shots: [(&str, u8, bool, bool); 4] = [("pill_glance_meeting", 4, false, false), ("pill_cycle_date_dots", 0, true, false), ("pill_day_line", 0, false, true), ("pill_glance_meeting_day", 4, false, true)];
        for (name, kind, cyc, day) in shots {
            a.cfg.idle_glance = kind;
            a.cfg.idle_cycle = cyc;
            a.cycle = 1;
            a.cycle_until = if cyc { Some(Instant::now() + std::time::Duration::from_secs(30)) } else { None };
            a.cfg.day_line = day;
            a.cfg.day_start = 0;
            a.cfg.day_end = 24;
            let gw = ((COL_IDLE_W + a.glance_extra_w()) * scale) as i32;
            a.gfx.begin(gw, ih).unwrap();
            let mut hg = Vec::new();
            crate::ui::draw(&a, COL_IDLE_W + a.glance_extra_w(), a.col_h(), &mut hg);
            let _ = a.gfx.end();
            let (bw, _bh, buf) = a.gfx.snapshot();
            save(name, gw, ih, bw, &buf);
        }
        a.cfg.idle_glance = 0;
        a.cfg.idle_cycle = false;
        a.cfg.day_line = false;
        a.cycle_until = None;
        crate::cal::EVENTS.lock().unwrap().clear();
        a.cfg.idle_glance = 0;
        a.stats = None;
        a.timer_start = Some(Instant::now() - std::time::Duration::from_secs(10 * 60));
        a.timer_total_s = 25 * 60;
        a.timer_left_ms = 25 * 60 * 1000;
        a.gfx.begin(cw, chh).unwrap();
        let mut hf = Vec::new();
        crate::ui::draw(&a, COL_TIMER_W, a.col_h(), &mut hf);
        let _ = a.gfx.end();
        let (bw, _bh, buf) = a.gfx.snapshot();
        save("pill_focus_line", cw, chh, bw, &buf);
        a.timer_start = None;
        a.timer_left_ms = 25 * 60 * 1000;
        a.mstate.has_session = true;
        let (mw, mh) = ((COL_MEDIA_W * scale) as i32, (a.col_h() * scale) as i32);
        a.gfx.begin(mw, mh).unwrap();
        let mut h4 = Vec::new();
        crate::ui::draw(&a, COL_MEDIA_W, a.col_h(), &mut h4);
        let _ = a.gfx.end();
        let (bw, _bh, buf) = a.gfx.snapshot();
        save("pill_media", mw, mh, bw, &buf);

        // ---- buddies on a 380 x bar strip
        let g = crate::gfx::Gfx::new((384.0 * scale) as i32, (34.0 * scale) as i32, scale).expect("gfx");
        let (sw, sh) = ((380.0 * scale) as i32, (crate::bar::BAR_H * scale) as i32);
        let buddy_shot = |name: &str, id: u8, mode: u8, tick: u32, dir: f32, bubble: Option<&str>| {
            g.begin(sw, sh).unwrap();
            crate::buddy::demo_frame(&g, id, mode, tick, dir, bubble);
            let _ = g.end();
            let (bw, _bh, buf) = g.snapshot();
            save(name, sw, sh, bw, &buf);
        };
        for k in 0..8u32 {
            buddy_shot(&format!("buddy_cat_walk{k}"), 2, 2, k * 3, 1.0, None);
        }
        for k in 0..8u32 {
            buddy_shot(&format!("buddy_robot_walk{k}"), 5, 2, k * 3, 1.0, None);
        }
        buddy_shot("buddy_robot_sleep", 5, 0, 5, 1.0, None);
        buddy_shot("buddy_robot_dance", 5, 3, 6, 1.0, None);
        buddy_shot("buddy_robot_cheer", 5, 4, 6, 1.0, Some("Nice one."));
        buddy_shot("buddy_cat_sleep", 2, 0, 5, 1.0, None);
        buddy_shot("buddy_cat_cheer", 2, 4, 6, 1.0, Some("\u{2665}"));
        for k in 0..8u32 {
            buddy_shot(&format!("buddy_ronin_walk{k}"), 6, 2, k * 3, 1.0, None);
        }
        buddy_shot("buddy_ronin_say", 6, 1, 4, 1.0, Some("Patience."));
        buddy_shot("buddy_courier_say", 7, 1, 4, 1.0, Some("Delivery's late. Again."));
        buddy_shot("buddy_necro_say", 8, 1, 4, 1.0, Some("Death is a pause."));
        std::fs::write(format!("{dir}/coords.json"), format!("{{\n  \"bar\": {{ {} }},\n  \"tabs\": {{ {} }}\n}}", hits.join(", "), tabs.join(", "))).unwrap();
    }
}

/// Everything the idle glance slot may need to format.
pub struct GlanceIn {
    pub kind: u8,
    pub has_todos: bool,
    pub left: usize,
    pub focus_min: u32,
    pub stats: Option<(u32, u32)>,
    pub meeting: Option<String>,
    pub date: String,
}

/// What the idle glance slot shows and the fixed width it reserves (so numbers never jitter).
/// Kinds: 1 tasks left, 2 focus today, 3 CPU and RAM, 4 next meeting, 5 date.
pub fn glance_text(g: &GlanceIn) -> Option<(String, f32)> {
    match g.kind {
        1 if g.has_todos => Some((format!("{} left", g.left), 64.0)),
        2 => Some((format!("{} min", g.focus_min), 58.0)),
        3 => g.stats.map(|(c, m)| (format!("CPU {c}% RAM {m}%"), 112.0)),
        4 => g.meeting.clone().map(|m| (m, 142.0)),
        5 => Some((g.date.clone(), 76.0)),
        _ => None,
    }
}

/// Cut `s` to at most `max` characters, ending with an ellipsis when it was longer.
pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}\u{2026}", head.trim_end())
    }
}

/// "Standup 10:30" (or "Standup 10:30 AM"): `start` is a local epoch in seconds.
pub fn meeting_text(title: &str, start: i64, clock24: bool) -> String {
    let secs = start.rem_euclid(86400);
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    let time = if clock24 {
        format!("{h:02}:{m:02}")
    } else {
        let h12 = match h {
            0 => 12,
            1..=12 => h,
            _ => h - 12,
        };
        format!("{h12}:{m:02} {}", if h < 12 { "AM" } else { "PM" })
    };
    format!("{} {}", ellipsize(title, 11), time)
}

/// The next meeting that starts within `hours` hours from `now` (not one that has begun).
pub fn next_meeting(events: &[crate::cal::Event], now: i64, hours: i64) -> Option<&crate::cal::Event> {
    events.iter().filter(|e| e.start > now && e.start - now <= hours * 3600).min_by_key(|e| e.start)
}

/// "Sun 1 Nov".
pub fn date_text(month: u32, day: u32, wday: u32) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {} {}", DAYS[(wday as usize).min(6)], day, MONTHS[(month as usize).clamp(1, 12) - 1])
}

/// How much of the workday [start, end) hours has passed at hh:mm; None outside the workday.
pub fn day_fraction(hh: u32, mm: u32, start: u8, end: u8) -> Option<f32> {
    if end <= start {
        return None;
    }
    let now = hh as u32 * 60 + mm as u32;
    let (s, e) = (start as u32 * 60, end as u32 * 60);
    if now < s || now >= e {
        return None;
    }
    Some((now - s) as f32 / (e - s) as f32)
}

#[cfg(test)]
mod glance_tests {
    use super::*;

    fn g(kind: u8) -> GlanceIn {
        GlanceIn { kind, has_todos: true, left: 3, focus_min: 42, stats: Some((12, 63)), meeting: Some("Standup 10:30".into()), date: "Sun 1 Nov".into() }
    }

    #[test]
    fn formats_each_glance() {
        assert_eq!(glance_text(&g(1)).map(|x| x.0), Some("3 left".to_string()));
        assert_eq!(glance_text(&GlanceIn { has_todos: false, ..g(1) }), None);
        assert_eq!(glance_text(&g(2)).map(|x| x.0), Some("42 min".to_string()));
        assert_eq!(glance_text(&g(3)).map(|x| x.0), Some("CPU 12% RAM 63%".to_string()));
        assert_eq!(glance_text(&GlanceIn { stats: None, ..g(3) }), None);
        assert_eq!(glance_text(&g(4)).map(|x| x.0), Some("Standup 10:30".to_string()));
        assert_eq!(glance_text(&GlanceIn { meeting: None, ..g(4) }), None);
        assert_eq!(glance_text(&g(5)).map(|x| x.0), Some("Sun 1 Nov".to_string()));
        assert_eq!(glance_text(&g(0)), None);
    }

    #[test]
    fn long_meeting_names_are_truncated() {
        assert_eq!(ellipsize("Quarterly planning review", 11), "Quarterly\u{2026}");
        assert_eq!(ellipsize("Standup", 11), "Standup");
        assert_eq!(meeting_text("Quarterly planning review", 10 * 3600 + 30 * 60, true), "Quarterly\u{2026} 10:30");
        assert_eq!(meeting_text("Standup", 15 * 3600 + 5 * 60, false), "Standup 3:05 PM");
        assert_eq!(meeting_text("Sync", 0, false), "Sync 12:00 AM");
    }

    #[test]
    fn picks_the_next_meeting_within_the_window() {
        let ev = |t: i64, s: &str| crate::cal::Event { start: t, title: s.into(), link: None };
        let events = vec![ev(100, "past"), ev(3 * 3600 + 500, "far"), ev(2 * 3600, "soon"), ev(3600, "first")];
        assert_eq!(next_meeting(&events, 200, 3).map(|e| e.title.as_str()), Some("first"));
        assert_eq!(next_meeting(&events, 200, 3).map(|e| e.start), Some(3600));
        assert!(next_meeting(&events[1..2], 200, 1).is_none());
        assert!(next_meeting(&[], 0, 3).is_none());
    }

    #[test]
    fn date_and_day_progress() {
        assert_eq!(date_text(11, 1, 0), "Sun 1 Nov");
        assert_eq!(day_fraction(9, 0, 9, 18), Some(0.0));
        assert_eq!(day_fraction(13, 30, 9, 18), Some(0.5));
        assert_eq!(day_fraction(8, 59, 9, 18), None);
        assert_eq!(day_fraction(18, 0, 9, 18), None);
        assert_eq!(day_fraction(10, 0, 18, 9), None);
    }
}
