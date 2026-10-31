//! Tiny plain-text persistence in %APPDATA%\Nock. No serde, no TOML crate.
//!   config.ini  – a few switches
//!   apps.txt    – pinned launcher entries, one per line:  Name|target|optional args
//!   todos.txt   – the to-do list, one per line:           0|text   (1 = done)

use std::fs;
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let d = base.join("Nock");
    // Settings from earlier working names are carried over once.
    if !d.exists() {
        for old in ["Islet", "Notch"] {
            let o = base.join(old);
            if o.exists() {
                let _ = fs::rename(&o, &d);
                break;
            }
        }
    }
    let _ = fs::create_dir_all(&d);
    d
}

pub fn apps_path() -> PathBuf {
    data_dir().join("apps.txt")
}

// ------------------------------------------------------------------ config

pub struct Config {
    /// Open when the pointer rests on the pill (otherwise click to open).
    pub hover_expand: bool,
    /// Animate the little equaliser bars while music plays (≈20 fps, tiny area).
    pub eq_animation: bool,
    /// Idle pill shows 13:05 instead of 1:05 PM.
    pub clock_24h: bool,
    /// Reserve a strip at the top of the screen so maximised windows sit below the notch.
    pub reserve_space: bool,
    /// Full-width status bar along the top; the notch hangs from it.
    pub top_bar: bool,
    /// Buddy on the top bar: 0 = off, 1 blob, 2 cat, 3 bunny, 4 hero, 5 robot.
    pub buddy: u8,
    /// Now and then the characters act out a short fight scene on the bar.
    pub scenes: bool,
    /// Average minutes between automatic scenes.
    pub scene_min: u8,
    /// CPU and memory read-out on the top bar.
    pub stats: bool,
    /// Top bar clock: 0 digital, 1 analog, 2 binary, 3 countdown to Friday.
    pub clock_style: u8,
    /// Minutes between water reminders (0 = off).
    pub water_min: u16,
    /// After a focus session, start a 5 minute break, then focus again.
    pub auto_break: bool,
    /// Minutes between look-away (20-20-20) reminders; 0 = off.
    pub eye_min: u16,
    /// Smart pill: low-battery, long-session break and copied link/code hints on the notch.
    pub smart: bool,
    /// A short chime with the important hints (meeting, low battery, break).
    pub smart_sound: bool,
    /// Optional extra beside the idle clock: 0 off, 1 tasks left, 2 focus minutes today, 3 CPU and RAM, 4 next meeting.
    pub idle_glance: u8,
    /// A thin progress line along the bottom of the notch while a focus session runs.
    pub idle_focus_bar: bool,
    /// Next-meeting glance shows meetings starting within this many hours (1 to 12).
    pub idle_meeting_hours: u8,
    /// A thin line along the bottom of the idle pill: how much of the workday has passed.
    pub day_line: bool,
    /// Workday for the day line, in whole hours (0 to 24).
    pub day_start: u8,
    pub day_end: u8,
    /// Scroll up over the idle pill cycles: time only, date, tasks left, focus today.
    pub idle_cycle: bool,
    /// A state glyph on the idle and timer notch: dot, ring (mic), amber square (battery), arc (focus).
    pub state_dot: bool,
    /// Now and then the buddy peeks over the bottom edge of the notch for a moment.
    pub peek: bool,
    /// Read-only ICS link (Google / Outlook) for "meeting in 5 min" hints; empty = off.
    pub calendar_url: String,
    /// A Start Menu shortcut so Windows search finds Nock.
    pub start_menu: bool,
    /// Hide the notch and bar while a full-screen app or game is in front.
    pub fs_hide: bool,
    /// Keep the notch and bar hidden until the pointer touches the top edge.
    pub auto_hide: bool,
    /// Fade the notch and bar out while a window covers the notch, and back in when it moves away.
    pub smart_hide: bool,
    /// Close the open panel when the pointer leaves it or another window comes to the front.
    pub auto_close: bool,
    /// Which display the notch and bar live on: 0 = primary, 1.. = that display.
    pub monitor: u8,
}

const DEFAULT_CONFIG: &str = "\
# Nock settings (1 = on, 0 = off)
hover_expand=0
eq_animation=1
clock_24h=0
reserve_space=0
top_bar=1
# buddy: 0 = off, 2 = cat, 5 = robot, 6 = ronin, 7 = courier, 8 = necromancer (1, 3 and 4 were retired and now give the robot)
buddy=5
# fight scenes now and then (needs a buddy): 1 = on
scenes=1
# average minutes between scenes (1 to 60)
scene_min=5
# CPU and memory read-out on the top bar: 1 = on
stats=0
# top bar clock: 0 = digital, 1 = analog, 2 = binary, 3 = countdown to Friday
clock_style=0
# minutes between water reminders (0 = off)
water_min=0
# after a focus session start a 5 minute break, then focus again: 1 = on
auto_break=0
# minutes between look-away reminders for your eyes (0 = off)
eye_min=0
# a Start Menu shortcut so typing Nock in Windows search finds it: 1 = on
start_menu=1
# hide the notch and bar while a full-screen app or game is in front: 1 = on
fs_hide=1
# keep the notch and bar hidden until the pointer touches the top edge: 1 = on
auto_hide=0
# fade the notch out while a window covers it, and back in when the window moves away: 1 = on
smart_hide=0
# close the open panel when the pointer leaves it or another window comes to the front: 1 = on
auto_close=1
# smart notch hints (low battery, take a break after 90 min at the PC, link/code copied): 1 = on
smart=1
# calendar link (Google: Settings > your calendar > Secret address in iCal format) for meeting hints; empty = off
calendar_url=
# a short chime with meeting, low battery and break hints: 1 = on
smart_sound=0
# display for the notch and bar: 0 = primary, 2 = second display, and so on
monitor=0
# extra beside the idle clock: 0 = off, 1 = tasks left, 2 = focus minutes today, 3 = CPU and RAM, 4 = next meeting
idle_glance=0
# progress line on the notch while a focus session runs: 1 = on
idle_focus_bar=1
# next meeting glance (idle_glance=4) shows meetings starting within this many hours (1 to 12)
idle_meeting_hours=3
# thin line along the bottom of the idle notch: how much of your workday has passed: 1 = on
day_line=0
# workday for the day line, in hours (0 to 24)
day_start=9
day_end=18
# scroll up over the idle notch cycles time, date, tasks left and focus today: 1 = on
idle_cycle=0
# a state glyph on the notch: dot, ring (mic in use), amber square (battery low), arc (focus session): 1 = on
state_dot=0
# now and then the buddy peeks over the bottom edge of the notch (robot and kitty): 1 = on
peek=0
";

pub fn load_config() -> Config {
    let path = data_dir().join("config.ini");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            let _ = fs::write(&path, DEFAULT_CONFIG);
            DEFAULT_CONFIG.to_string()
        }
    };
    let mut cfg = Config {
        hover_expand: false,
        eq_animation: true,
        clock_24h: false,
        reserve_space: false,
        top_bar: true,
        buddy: 5,
        scenes: true,
        scene_min: 5,
        stats: false,
        clock_style: 0,
        water_min: 0,
        auto_break: false,
        eye_min: 0,
        start_menu: true,
        fs_hide: true,
        auto_hide: false,
        smart_hide: false,
        auto_close: true,
        smart: true,
        smart_sound: false,
        idle_glance: 0,
        idle_focus_bar: true,
        idle_meeting_hours: 3,
        day_line: false,
        day_start: 9,
        day_end: 18,
        idle_cycle: false,
        state_dot: false,
        peek: false,
        calendar_url: String::new(),
        monitor: 0,
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let on = matches!(v.trim(), "1" | "true" | "yes" | "on");
            match k.trim() {
                "hover_expand" => cfg.hover_expand = on,
                "eq_animation" => cfg.eq_animation = on,
                "clock_24h" => cfg.clock_24h = on,
                "reserve_space" => cfg.reserve_space = on,
                "top_bar" => cfg.top_bar = on,
                "scenes" => cfg.scenes = on,
                "stats" => cfg.stats = on,
                "auto_break" => cfg.auto_break = on,
                "start_menu" => cfg.start_menu = on,
                "fs_hide" => cfg.fs_hide = on,
                "auto_hide" => cfg.auto_hide = on,
                "smart_hide" => cfg.smart_hide = on,
                "auto_close" => cfg.auto_close = on,
                "smart" => cfg.smart = on,
                "smart_sound" => cfg.smart_sound = on,
                "idle_glance" => cfg.idle_glance = v.trim().parse::<u8>().unwrap_or(0).min(4),
                "idle_focus_bar" => cfg.idle_focus_bar = on,
                "idle_meeting_hours" => cfg.idle_meeting_hours = v.trim().parse::<u8>().unwrap_or(3).clamp(1, 12),
                "day_line" => cfg.day_line = on,
                "day_start" => cfg.day_start = v.trim().parse::<u8>().unwrap_or(9).min(24),
                "day_end" => cfg.day_end = v.trim().parse::<u8>().unwrap_or(18).min(24),
                "idle_cycle" => cfg.idle_cycle = on,
                "state_dot" => cfg.state_dot = on,
                "peek" => cfg.peek = on,
                "calendar_url" => cfg.calendar_url = v.trim().to_string(),
                "eye_min" => cfg.eye_min = v.trim().parse::<u16>().unwrap_or(0).min(600),
                "clock_style" => cfg.clock_style = v.trim().parse::<u8>().unwrap_or(0).min(3),
                "water_min" => cfg.water_min = v.trim().parse::<u16>().unwrap_or(0).min(600),
                "scene_min" => cfg.scene_min = v.trim().parse::<u8>().unwrap_or(5).clamp(1, 60),
                "monitor" => cfg.monitor = v.trim().parse::<u8>().unwrap_or(0).min(16),
                "buddy" => cfg.buddy = match v.trim().parse::<u8>() {
                    // retired blob, bunny and hero fall back to the robot
                    Ok(1 | 3 | 4) => 5,
                    Ok(b) if b <= 8 => b,
                    _ => 5,
                },
                _ => {}
            }
        }
    }
    // A config file from an older version lacks the newer settings: write the
    // file back with every setting present (keeping the user's values).
    let keys = ["hover_expand", "eq_animation", "clock_24h", "reserve_space", "top_bar", "buddy", "scenes", "scene_min", "stats", "clock_style", "water_min", "auto_break", "eye_min", "start_menu", "fs_hide", "auto_hide", "smart_hide", "auto_close", "smart", "calendar_url", "smart_sound", "monitor", "idle_glance", "idle_focus_bar", "idle_meeting_hours", "day_line", "day_start", "day_end", "idle_cycle", "state_dot", "peek"];
    let present = |k: &str| text.lines().any(|l| l.trim().split_once('=').map(|(a, _)| a.trim() == k).unwrap_or(false));
    if keys.iter().any(|k| !present(k)) {
        save_config(&cfg);
    }
    cfg
}

pub fn save_config(c: &Config) {
    let b = |v: bool| if v { 1 } else { 0 };
    let text = format!(
        "# Nock settings (1 = on, 0 = off)\nhover_expand={}\neq_animation={}\nclock_24h={}\nreserve_space={}\ntop_bar={}\n# buddy: 0 = off, 2 = cat, 5 = robot, 6 = ronin, 7 = courier, 8 = necromancer (1, 3 and 4 were retired and now give the robot)\nbuddy={}\n# fight scenes now and then (needs a buddy): 1 = on\nscenes={}\n# average minutes between scenes (1 to 60)\nscene_min={}\n# CPU and memory read-out on the top bar: 1 = on\nstats={}\n# top bar clock: 0 = digital, 1 = analog, 2 = binary, 3 = countdown to Friday\nclock_style={}\n# minutes between water reminders (0 = off)\nwater_min={}\n# after a focus session start a 5 minute break, then focus again: 1 = on\nauto_break={}\n# minutes between look-away reminders for your eyes (0 = off)\neye_min={}\n# a Start Menu shortcut so typing Nock in Windows search finds it: 1 = on\nstart_menu={}\n# hide the notch and bar while a full-screen app or game is in front: 1 = on\nfs_hide={}\n# keep the notch and bar hidden until the pointer touches the top edge: 1 = on\nauto_hide={}\n# fade the notch out while a window covers it, and back in when the window moves away: 1 = on\nsmart_hide={}\n# close the open panel when the pointer leaves it or another window comes to the front: 1 = on\nauto_close={}\n# smart notch hints (low battery, take a break after 90 min at the PC, link/code copied): 1 = on\nsmart={}\n# calendar link (Google: Settings > your calendar > Secret address in iCal format) for meeting hints; empty = off\ncalendar_url={}\n# a short chime with meeting, low battery and break hints: 1 = on\nsmart_sound={}\n# display for the notch and bar: 0 = primary, 2 = second display, and so on\nmonitor={}\n# extra beside the idle clock: 0 = off, 1 = tasks left, 2 = focus minutes today, 3 = CPU and RAM, 4 = next meeting\nidle_glance={}\n# progress line on the notch while a focus session runs: 1 = on\nidle_focus_bar={}\n# next meeting glance (idle_glance=4) shows meetings starting within this many hours (1 to 12)\nidle_meeting_hours={}\n# thin line along the bottom of the idle notch: how much of your workday has passed: 1 = on\nday_line={}\n# workday for the day line, in hours (0 to 24)\nday_start={}\nday_end={}\n# scroll up over the idle notch cycles time, date, tasks left and focus today: 1 = on\nidle_cycle={}\n# a state glyph on the notch: dot, ring (mic in use), amber square (battery low), arc (focus session): 1 = on\nstate_dot={}\n# now and then the buddy peeks over the bottom edge of the notch (robot and kitty): 1 = on\npeek={}\n",
        b(c.hover_expand),
        b(c.eq_animation),
        b(c.clock_24h),
        b(c.reserve_space),
        b(c.top_bar),
        c.buddy,
        b(c.scenes),
        c.scene_min,
        b(c.stats),
        c.clock_style,
        c.water_min,
        b(c.auto_break),
        c.eye_min,
        b(c.start_menu),
        b(c.fs_hide),
        b(c.auto_hide),
        b(c.smart_hide),
        b(c.auto_close),
        b(c.smart),
        c.calendar_url,
        b(c.smart_sound),
        c.monitor,
        c.idle_glance,
        b(c.idle_focus_bar),
        c.idle_meeting_hours,
        b(c.day_line),
        c.day_start,
        c.day_end,
        b(c.idle_cycle),
        b(c.state_dot),
        b(c.peek)
    );
    let _ = fs::write(data_dir().join("config.ini"), text);
}

// -------------------------------------------------------------------- apps

#[derive(Clone)]
pub struct AppEntry {
    pub name: String,
    pub target: String,
    pub args: String,
}

const DEFAULT_APPS: &str = "\
# Pinned apps for the notch launcher. One per line:  Name|target|optional arguments
# target can be an .exe name or path, a folder, a URL, or something like ms-settings:
# The first 12 entries are shown.
File Explorer|explorer.exe
Terminal|wt.exe
Notepad|notepad.exe
Calculator|calc.exe
Settings|ms-settings:
Browser|https://www.google.com
";

/// Write the pinned apps back (the picker pins and unpins from the panel).
pub fn save_apps(apps: &[AppEntry]) {
    let mut text = String::from("# Pinned apps for the notch launcher. One per line:  Name|target|optional arguments\n# You can edit this file, or just use the + and x buttons in the Apps tab.\n");
    for a in apps {
        text.push_str(&format!("{}|{}", a.name.replace('|', " "), a.target));
        if !a.args.is_empty() {
            text.push_str(&format!("|{}", a.args));
        }
        text.push('\n');
    }
    let _ = fs::write(apps_path(), text);
}

pub fn load_apps() -> Vec<AppEntry> {
    let path = apps_path();
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            let _ = fs::write(&path, DEFAULT_APPS);
            DEFAULT_APPS.to_string()
        }
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut it = l.splitn(3, '|');
            let name = it.next()?.trim().to_string();
            let target = it.next()?.trim().to_string();
            let args = it.next().unwrap_or("").trim().to_string();
            if name.is_empty() || target.is_empty() {
                None
            } else {
                Some(AppEntry { name, target, args })
            }
        })
        .take(12)
        .collect()
}

// ------------------------------------------------------------------- todos

#[derive(Clone)]
pub struct Todo {
    pub done: bool,
    pub text: String,
}

pub fn load_todos() -> Vec<Todo> {
    fs::read_to_string(data_dir().join("todos.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (flag, text) = l.split_once('|')?;
            Some(Todo {
                done: flag.trim() == "1",
                text: text.to_string(),
            })
        })
        .collect()
}

fn today_key() -> String {
    let (y, m, d, ..) = crate::sys::local_time();
    format!("{y:04}-{m:02}-{d:02}")
}

/// (focus minutes, tasks finished) for today; resets itself on a new day.
pub fn load_focus_today() -> (u32, u32) {
    let text = fs::read_to_string(data_dir().join("focus_today.txt")).unwrap_or_default();
    let mut it = text.trim().split('|');
    if it.next() != Some(today_key().as_str()) {
        return (0, 0);
    }
    let mins = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let done = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    (mins, done)
}

pub fn save_focus_today(mins: u32, done: u32) {
    let _ = fs::write(data_dir().join("focus_today.txt"), format!("{}|{mins}|{done}", today_key()));
}

/// One pinned item <-> one line of text: newlines and backslashes are escaped.
fn pin_escape(p: &str) -> String {
    p.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r")
}

fn pin_unescape(l: &str) -> String {
    let mut out = String::new();
    let mut chars = l.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Pinned clipboard items (they double as saved snippets) survive restarts.
pub fn load_pins() -> Vec<String> {
    fs::read_to_string(data_dir().join("pinned_clips.txt"))
        .unwrap_or_default()
        .lines()
        .map(pin_unescape)
        .filter(|s| !s.trim().is_empty())
        .collect()
}

pub fn save_pins(pins: &[String]) {
    let text: Vec<String> = pins.iter().map(|p| pin_escape(p)).collect();
    let _ = fs::write(data_dir().join("pinned_clips.txt"), text.join("\n"));
}

/// Files parked on the Shelf tab, one full path per line. Files that have since
/// been moved or deleted are dropped when the list is read.
pub fn load_shelf() -> Vec<String> {
    fs::read_to_string(data_dir().join("shelf.txt"))
        .unwrap_or_default()
        .lines()
        .filter(|p| std::path::Path::new(p).exists())
        .map(String::from)
        .collect()
}

pub fn save_shelf(items: &[String]) {
    let _ = fs::write(data_dir().join("shelf.txt"), items.join("\n"));
}

/// Best scores of the four notch games (flight, bricks, slicer, run). The flying
/// game's old single-score file is read once if the new one is missing.
pub fn load_bests() -> [u32; 4] {
    let text = fs::read_to_string(data_dir().join("games_best.txt")).unwrap_or_default();
    let v: Vec<u32> = text.trim().split('|').filter_map(|p| p.parse().ok()).collect();
    if v.len() == 4 {
        [v[0], v[1], v[2], v[3]]
    } else {
        [load_game_best(), 0, 0, 0]
    }
}

pub fn save_bests(b: &[u32; 4]) {
    let _ = fs::write(data_dir().join("games_best.txt"), format!("{}|{}|{}|{}", b[0], b[1], b[2], b[3]));
}

/// Best score of the notch mini game.
pub fn load_game_best() -> u32 {
    fs::read_to_string(data_dir().join("game_best.txt"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

pub fn save_todos(todos: &[Todo]) {
    let mut s = String::new();
    for t in todos {
        s.push_str(if t.done { "1|" } else { "0|" });
        s.push_str(&t.text.replace('\n', " "));
        s.push('\n');
    }
    let _ = fs::write(data_dir().join("todos.txt"), s);
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    #[test]
    fn pins_survive_multiline_and_backslashes() {
        for t in ["plain", "two\nlines", r"C:\Users\me\file.txt", "tab\there\r\nwindows", r"ends with \"] {
            let one_line = pin_escape(t);
            assert!(!one_line.contains('\n') && !one_line.contains('\r'));
            assert_eq!(pin_unescape(&one_line), t);
        }
    }
}
