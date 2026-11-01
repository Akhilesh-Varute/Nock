//! Linux counterpart of `sys`. Grows as features are ported.

/// (year, month 1-12, day, weekday 0=Sunday, hour, minute, second, millisecond)
pub fn local_time() -> (i32, u32, u32, u32, u32, u32, u32, u32) {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts);
        libc::localtime_r(&ts.tv_sec, &mut tm);
    }
    (
        tm.tm_year + 1900,
        (tm.tm_mon + 1) as u32,
        tm.tm_mday as u32,
        tm.tm_wday as u32,
        tm.tm_hour as u32,
        tm.tm_min as u32,
        tm.tm_sec as u32,
        (ts.tv_nsec / 1_000_000) as u32,
    )
}

/// Windows-only (PowerShell). The calendar feed fetch needs a Linux path (curl) later.
pub(crate) fn powershell(_cmd: &str) -> Option<String> {
    None
}

fn run(cmd: &str, args: &[&str]) {
    // fire and forget; a missing tool or a refused request just does nothing
    let _ = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn lock_pc() {
    run("loginctl", &["lock-session"]);
}

pub fn sleep_pc() {
    run("systemctl", &["suspend"]);
}

pub fn restart_pc() {
    run("systemctl", &["reboot"]);
}

pub fn shutdown_pc() {
    run("systemctl", &["poweroff"]);
}

/// Start the first command that exists on this machine; false if none could start.
fn run_first(cmds: &[(&str, &[&str])]) -> bool {
    cmds.iter().any(|(cmd, args)| {
        std::process::Command::new(cmd)
            .args(*args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
    })
}

/// Region screenshot with whichever tool the desktop has.
pub fn screenshot() -> bool {
    run_first(&[
        ("gnome-screenshot", &["-a"]),
        ("spectacle", &["-r"]),
        ("flameshot", &["gui"]),
        ("xfce4-screenshooter", &["-r"]),
        ("scrot", &["-s"]),
    ])
}

/// Open a system settings page: 0 display, 1 sound, 2 network, 3 power, 4 everything.
pub fn open_settings(page: u8) -> bool {
    const GNOME: [&str; 5] = ["display", "sound", "network", "power", ""];
    const KDE: [&str; 5] = ["kcm_kscreen", "kcm_pulseaudio", "kcm_networkmanagement", "kcm_powerdevilprofilesconfig", ""];
    let i = (page as usize).min(4);
    let (g, k) = (GNOME[i], KDE[i]);
    let gnome: &[&str] = if g.is_empty() { &[] } else { &[g] };
    let kde: &[&str] = if k.is_empty() { &[] } else { &[k] };
    run_first(&[
        ("gnome-control-center", gnome),
        ("systemsettings", kde),
        ("xfce4-settings-manager", &[]),
        ("cinnamon-settings", &[]),
    ])
}

/// Open a pinned entry: a .desktop file, a URL, a folder or a command with arguments.
pub fn open(target: &str, args: &str) {
    let home = std::env::var("HOME").unwrap_or_default();
    let expand = |s: &str| if s == "~" || s.starts_with("~/") { format!("{home}{}", &s[1..]) } else { s.to_string() };
    let target = expand(target);
    let args: Vec<String> = args.split_whitespace().map(expand).collect();
    if target.ends_with(".desktop") {
        let stem = std::path::Path::new(&target).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        run_first(&[("gio", &["launch", &target]), ("gtk-launch", &[&stem])]);
    } else if target.contains("://") || (opens_with_default(&target) && args.is_empty()) {
        run("xdg-open", &[&target]);
    } else {
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        run(&target, &a);
    }
}

/// An existing folder or plain (non-executable) file: hand it to the default app.
fn opens_with_default(path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_dir() || m.permissions().mode() & 0o111 == 0)
}

/// Name of a launchable application from a .desktop file, or None to hide it.
pub fn desktop_name(text: &str) -> Option<String> {
    let (mut name, mut hidden, mut is_app, mut in_entry) = (None, false, false, false);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry {
            match line.split_once('=') {
                Some(("Name", v)) if name.is_none() => name = Some(v.trim().to_string()),
                Some(("Type", v)) => is_app = v.trim() == "Application",
                Some(("NoDisplay" | "Hidden", v)) if v.trim() == "true" => hidden = true,
                _ => {}
            }
        }
    }
    name.filter(|n| is_app && !hidden && !n.is_empty())
}

/// Installed apps for the picker: (name, path of the .desktop file), sorted by name.
pub fn list_apps() -> Vec<(String, String)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let dirs = [
        "/usr/share/applications".to_string(),
        "/usr/local/share/applications".to_string(),
        "/var/lib/flatpak/exports/share/applications".to_string(),
        "/var/lib/snapd/desktop/applications".to_string(),
        format!("{home}/.local/share/flatpak/exports/share/applications"),
        format!("{home}/.local/share/applications"),
    ];
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            if !file.ends_with(".desktop") || !seen.insert(file) {
                continue;
            }
            if let Some(name) = std::fs::read_to_string(e.path()).ok().and_then(|t| desktop_name(&t)) {
                out.push((name, e.path().to_string_lossy().into_owned()));
            }
        }
    }
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out
}

thread_local! {
    // X11 serves the clipboard from the process that set it, so keep one handle alive
    static CLIP: std::cell::RefCell<Option<arboard::Clipboard>> = const { std::cell::RefCell::new(None) };
}

fn with_clip<T>(f: impl FnOnce(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    CLIP.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = arboard::Clipboard::new().ok();
        }
        c.as_mut().and_then(f)
    })
}

pub fn clipboard_text() -> Option<String> {
    with_clip(|c| c.get_text().ok())
}

pub fn set_clipboard_text(text: &str) -> bool {
    with_clip(|c| c.set_text(text.to_string()).ok()).is_some()
}

/// Press Ctrl+V in whatever window regains focus (needs xdotool; without it the user pastes).
pub fn paste_soon() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(170));
        run("xdotool", &["key", "ctrl+v"]);
    });
}

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let o = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// (percent, plugged in) of the first battery, if there is one.
pub fn battery_info() -> Option<(u8, bool)> {
    let mut dirs: Vec<_> = std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("BAT"))
        .collect();
    dirs.sort_by_key(|e| e.file_name());
    let d = dirs.first()?.path();
    let pct = std::fs::read_to_string(d.join("capacity")).ok()?.trim().parse().ok()?;
    let status = std::fs::read_to_string(d.join("status")).unwrap_or_default();
    Some((pct, status.trim() != "Discharging"))
}

/// "Volume: 0.50 [MUTED]" from `wpctl get-volume`.
pub fn parse_wpctl(s: &str) -> Option<(u8, bool)> {
    let v: f32 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some(((v * 100.0).round() as u8, s.contains("[MUTED]")))
}

/// First percentage in `pactl get-sink-volume` output.
pub fn parse_pactl_volume(s: &str) -> Option<u8> {
    s.split_whitespace().find_map(|t| t.strip_suffix('%')?.parse().ok())
}

/// (percent, muted) of the default output, through PipeWire or PulseAudio.
pub fn volume() -> Option<(u8, bool)> {
    if let Some(v) = output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).and_then(|s| parse_wpctl(&s)) {
        return Some(v);
    }
    let vol = parse_pactl_volume(&output("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?)?;
    let muted = output("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]).is_some_and(|m| m.contains("yes"));
    Some((vol, muted))
}

/// Network name from `nmcli -t -f ACTIVE,SSID dev wifi` ("yes:Name", colons inside names escaped).
pub fn parse_nmcli_wifi(s: &str) -> Option<String> {
    let line = s.lines().find_map(|l| l.strip_prefix("yes:"))?;
    Some(line.replace("\\:", ":"))
}

pub fn wifi_name() -> Option<String> {
    parse_nmcli_wifi(&output("nmcli", &["-t", "-f", "ACTIVE,SSID", "dev", "wifi"])?)
}

/// Class name of the focused window ("firefox" -> "Firefox"); empty without xdotool.
pub fn active_app() -> String {
    let raw = output("xdotool", &["getactivewindow", "getwindowclassname"]).unwrap_or_default();
    let name = raw.trim();
    let mut c = name.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Split one `nmcli -t` line into fields; a colon inside a value is written `\:`.
pub fn split_nmcli(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut esc) = (Vec::new(), String::new(), false);
    for c in line.chars() {
        match (esc, c) {
            (true, c) => {
                cur.push(c);
                esc = false;
            }
            (false, '\\') => esc = true,
            (false, ':') => out.push(std::mem::take(&mut cur)),
            (false, c) => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Networks from `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY dev wifi list`:
/// (in use, name, signal %, secured). One entry per name, strongest first, the current one on top.
pub fn parse_wifi_list(s: &str) -> Vec<(bool, String, u8, bool)> {
    let mut nets: Vec<(bool, String, u8, bool)> = Vec::new();
    for f in s.lines().map(split_nmcli) {
        let [in_use, ssid, signal, security] = &f[..] else { continue };
        if ssid.is_empty() {
            continue;
        }
        let net = (in_use == "*", ssid.clone(), signal.parse().unwrap_or(0), !security.is_empty() && security != "--");
        match nets.iter_mut().find(|n| n.1 == *ssid) {
            Some(old) => {
                if net.0 || (!old.0 && net.2 > old.2) {
                    *old = net;
                }
            }
            None => nets.push(net),
        }
    }
    nets.sort_by(|a, b| b.0.cmp(&a.0).then(b.2.cmp(&a.2)));
    nets
}

/// NetworkManager answers (so the bar shows a Wi-Fi item at all).
pub fn wifi_present() -> bool {
    output("nmcli", &["radio", "wifi"]).is_some()
}

pub fn wifi_enabled() -> bool {
    output("nmcli", &["radio", "wifi"]).is_some_and(|s| s.trim() == "enabled")
}

pub fn set_wifi(on: bool) {
    let _ = output("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
}

pub fn wifi_networks() -> Vec<(bool, String, u8, bool)> {
    output("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list"]).map(|s| parse_wifi_list(&s)).unwrap_or_default()
}

/// Names of saved connections (a saved Wi-Fi network joins without asking for the password).
pub fn saved_connections() -> Vec<String> {
    output("nmcli", &["-t", "-f", "NAME", "connection", "show"])
        .map(|s| s.lines().map(|l| split_nmcli(l).concat()).collect())
        .unwrap_or_default()
}

/// Join a network. The password goes in through stdin, not the command line, so it
/// does not show up in the process list.
pub fn wifi_connect(ssid: &str, password: Option<&str>) -> bool {
    use std::io::Write;
    let mut cmd = std::process::Command::new("nmcli");
    if password.is_some() {
        cmd.arg("--ask");
    }
    cmd.args(["dev", "wifi", "connect", ssid]);
    let Ok(mut child) = cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() else {
        return false;
    };
    if let (Some(pw), Some(mut stdin)) = (password, child.stdin.take()) {
        let _ = writeln!(stdin, "{pw}");
    }
    child.wait().is_ok_and(|s| s.success())
}

/// "Device AA:BB:CC:DD:EE:FF Some Name" lines from `bluetoothctl devices`.
pub fn parse_bt_devices(s: &str) -> Vec<(String, String)> {
    s.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (mac, name) = rest.split_once(' ')?;
            Some((mac.to_string(), name.trim().to_string()))
        })
        .collect()
}

pub fn bt_powered() -> Option<bool> {
    Some(output("bluetoothctl", &["show"])?.contains("Powered: yes"))
}

/// Paired devices as (address, name, connected).
pub fn bt_devices() -> Vec<(String, String, bool)> {
    let paired = output("bluetoothctl", &["devices", "Paired"]).or_else(|| output("bluetoothctl", &["devices"])).unwrap_or_default();
    parse_bt_devices(&paired)
        .into_iter()
        .map(|(mac, name)| {
            let on = output("bluetoothctl", &["info", &mac]).is_some_and(|i| i.contains("Connected: yes"));
            (mac, name, on)
        })
        .collect()
}

pub fn set_bt(on: bool) {
    let _ = output("bluetoothctl", &["power", if on { "on" } else { "off" }]);
}

pub fn bt_connect(mac: &str, connect: bool) -> bool {
    output("bluetoothctl", &[if connect { "connect" } else { "disconnect" }, mac]).is_some()
}

/// Percent from `brightnessctl -m` ("intel_backlight,backlight,2400,50%,4800").
pub fn parse_brightnessctl(s: &str) -> Option<u8> {
    s.trim().split(',').nth(3)?.trim_end_matches('%').parse().ok()
}

pub fn brightness() -> Option<u8> {
    parse_brightnessctl(&output("brightnessctl", &["-m"])?)
}

pub fn set_brightness(pct: u8) {
    let _ = output("brightnessctl", &["set", &format!("{}%", pct.clamp(1, 100))]);
}

pub fn set_volume(pct: u8) {
    let v = pct.min(100);
    if output("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{}", v as f32 / 100.0)]).is_none() {
        let _ = output("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &format!("{v}%")]);
    }
}

pub fn toggle_mute() {
    if output("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]).is_none() {
        let _ = output("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "toggle"]);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_tools_are_skipped() {
        assert!(!super::run_first(&[("nock-no-such-tool", &[]), ("nock-no-such-tool-2", &["x"])]));
        assert!(super::run_first(&[("nock-no-such-tool", &[]), ("true", &[])]));
    }

    #[test]
    fn status_parsers() {
        assert_eq!(super::parse_wpctl("Volume: 0.50\n"), Some((50, false)));
        assert_eq!(super::parse_wpctl("Volume: 0.35 [MUTED]\n"), Some((35, true)));
        assert_eq!(super::parse_pactl_volume("Volume: front-left: 42598 /  65% / -11.26 dB,   front-right: 42598 /  65% / -11.26 dB"), Some(65));
        assert_eq!(super::parse_nmcli_wifi("no:Other\nyes:Home\\:5G\n"), Some("Home:5G".to_string()));
        assert_eq!(super::parse_nmcli_wifi("no:Other\n"), None);
    }

    #[test]
    fn network_and_device_parsers() {
        assert_eq!(super::split_nmcli(r"yes:Home\:5G:80:WPA2"), vec!["yes", "Home:5G", "80", "WPA2"]);
        let list = " :Cafe:55:\n*:Home:90:WPA2\n :Home:40:WPA2\n :Far:20:WPA2\n :Hidden:90:\n::50:WPA2";
        let nets = super::parse_wifi_list(list);
        let names: Vec<_> = nets.iter().map(|n| n.1.as_str()).collect();
        assert_eq!(names, ["Home", "Hidden", "Cafe", "Far"], "current first, then strongest; unnamed skipped");
        assert!(nets[0].0 && nets[0].3 && !nets[2].3);
        assert_eq!(
            super::parse_bt_devices("Device AA:BB:CC:DD:EE:01 WH-1000XM4\nDevice AA:BB:CC:DD:EE:02 My Keyboard\nnoise"),
            vec![("AA:BB:CC:DD:EE:01".to_string(), "WH-1000XM4".to_string()), ("AA:BB:CC:DD:EE:02".to_string(), "My Keyboard".to_string())]
        );
        assert_eq!(super::parse_brightnessctl("intel_backlight,backlight,2400,50%,4800\n"), Some(50));
    }

    #[test]
    fn desktop_files() {
        let app = "[Desktop Entry]\nType=Application\nName=Calculator\nName[de]=Rechner\nExec=calc\n[Desktop Action x]\nName=Other\n";
        assert_eq!(super::desktop_name(app).as_deref(), Some("Calculator"));
        assert_eq!(super::desktop_name("[Desktop Entry]\nType=Application\nName=X\nNoDisplay=true\n"), None);
        assert_eq!(super::desktop_name("[Desktop Entry]\nType=Link\nName=X\n"), None);
    }
}
