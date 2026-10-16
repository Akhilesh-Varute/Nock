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
}

const DEFAULT_CONFIG: &str = "\
# Nock settings (1 = on, 0 = off)
hover_expand=0
eq_animation=1
clock_24h=0
reserve_space=0
top_bar=1
# buddy: 0 = off, 1 = blob, 2 = cat, 3 = bunny, 4 = hero, 5 = robot, 6 = ronin, 7 = courier, 8 = necromancer, 9 = ravi kishan
buddy=2
# fight scenes now and then (needs a buddy): 1 = on
scenes=1
# average minutes between scenes (1 to 60)
scene_min=5
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
        buddy: 2,
        scenes: true,
        scene_min: 5,
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
                "scene_min" => cfg.scene_min = v.trim().parse::<u8>().unwrap_or(5).clamp(1, 60),
                "buddy" => cfg.buddy = v.trim().parse::<u8>().unwrap_or(2).min(9),
                _ => {}
            }
        }
    }
    // A config file from an older version lacks the newer settings: write the
    // file back with every setting present (keeping the user's values).
    let keys = ["hover_expand", "eq_animation", "clock_24h", "reserve_space", "top_bar", "buddy", "scenes", "scene_min"];
    let present = |k: &str| text.lines().any(|l| l.trim().split_once('=').map(|(a, _)| a.trim() == k).unwrap_or(false));
    if keys.iter().any(|k| !present(k)) {
        save_config(&cfg);
    }
    cfg
}

pub fn save_config(c: &Config) {
    let b = |v: bool| if v { 1 } else { 0 };
    let text = format!(
        "# Nock settings (1 = on, 0 = off)\nhover_expand={}\neq_animation={}\nclock_24h={}\nreserve_space={}\ntop_bar={}\n# buddy: 0 = off, 1 = blob, 2 = cat, 3 = bunny, 4 = hero, 5 = robot, 6 = ronin, 7 = courier, 8 = necromancer, 9 = ravi kishan\nbuddy={}\n# fight scenes now and then (needs a buddy): 1 = on\nscenes={}\n# average minutes between scenes (1 to 60)\nscene_min={}\n",
        b(c.hover_expand),
        b(c.eq_animation),
        b(c.clock_24h),
        b(c.reserve_space),
        b(c.top_bar),
        c.buddy,
        b(c.scenes),
        c.scene_min
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

pub fn save_todos(todos: &[Todo]) {
    let mut s = String::new();
    for t in todos {
        s.push_str(if t.done { "1|" } else { "0|" });
        s.push_str(&t.text.replace('\n', " "));
        s.push('\n');
    }
    let _ = fs::write(data_dir().join("todos.txt"), s);
}
