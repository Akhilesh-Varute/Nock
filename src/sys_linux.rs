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

#[cfg(test)]
mod tests {
    #[test]
    fn missing_tools_are_skipped() {
        assert!(!super::run_first(&[("nock-no-such-tool", &[]), ("nock-no-such-tool-2", &["x"])]));
        assert!(super::run_first(&[("nock-no-such-tool", &[]), ("true", &[])]));
    }
}
