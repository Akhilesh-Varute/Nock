//! Small wrappers around the Windows calls the notch uses for its actions.
//! Everything here runs only in response to a user action or a Windows event.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTimePreciseAsFileTime};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Posted when the background brightness query finished.
/// WPARAM = 1 if supported, LPARAM = percent.
pub const WM_BRIGHT: u32 = 0x8000 + 2; // WM_APP + 2

const CF_UNICODETEXT: u32 = 13;

// ------------------------------------------------------------------ volume

thread_local! {
    static ENDPOINT: RefCell<Option<IAudioEndpointVolume>> = const { RefCell::new(None) };
}

fn acquire_endpoint() -> Option<IAudioEndpointVolume> {
    unsafe {
        let en: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

fn endpoint() -> Option<IAudioEndpointVolume> {
    ENDPOINT.with(|e| {
        if e.borrow().is_none() {
            *e.borrow_mut() = acquire_endpoint();
        }
        e.borrow().clone()
    })
}

fn drop_endpoint() {
    ENDPOINT.with(|e| *e.borrow_mut() = None);
}

/// (volume 0..1, muted)
pub fn get_volume() -> Option<(f32, bool)> {
    let ep = endpoint()?;
    unsafe {
        let v = ep.GetMasterVolumeLevelScalar();
        let m = ep.GetMute();
        match (v, m) {
            (Ok(v), Ok(m)) => Some((v, m.as_bool())),
            _ => {
                drop_endpoint(); // default device probably changed
                None
            }
        }
    }
}

pub fn set_volume(v: f32) {
    if let Some(ep) = endpoint() {
        unsafe {
            if ep
                .SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null())
                .is_err()
            {
                drop_endpoint();
            }
        }
    }
}

pub fn set_mute(m: bool) {
    if let Some(ep) = endpoint() {
        unsafe {
            let _ = ep.SetMute(m, std::ptr::null());
        }
    }
}

// -------------------------------------------------------------- brightness
// Internal laptop panels are only reachable through WMI. Spawning PowerShell
// is slow (~0.3 s) but only happens on open / on slider release, never idle.

fn powershell(cmd: &str) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut c = Command::new("powershell.exe");
    // -EncodedCommand (base64 of UTF-16) sidesteps every quoting problem.
    let utf16: Vec<u8> = cmd.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64(&utf16);
    c.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &encoded,
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = match c.output() {
        Ok(o) => o,
        Err(e) => {
            log_brightness(&format!("could not start powershell: {e}"));
            return None;
        }
    };
    if !out.status.success() {
        log_brightness(&format!(
            "powershell failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
        return None;
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        log_brightness(&format!("powershell warning: {}", err.trim()));
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Brightness problems are written to %APPDATA%\Notch\brightness.log.
fn log_brightness(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::store::data_dir().join("brightness.log"))
    {
        let _ = writeln!(f, "{msg}");
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len() * 4 / 3 + 4);
    for ch in data.chunks(3) {
        let b = [ch[0], *ch.get(1).unwrap_or(&0), *ch.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if ch.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if ch.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

// Native path: the display driver's own IOCTLs on \\.\LCD. Takes a millisecond, no
// process to start. Works for built-in laptop panels; anything else falls back to
// the slower WMI route below.

const IOCTL_SUPPORTED: u32 = 0x0023_0406;
const IOCTL_QUERY: u32 = 0x0023_0408;
const IOCTL_SET: u32 = 0x0023_040C;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct DisplayBrightness {
    policy: u8, // 1 = AC, 2 = DC, 3 = both
    ac: u8,
    dc: u8,
}

fn lcd_open() -> Option<HANDLE> {
    use windows::Win32::Storage::FileSystem::*;
    unsafe {
        CreateFileW(
            w!("\\\\.\\LCD"),
            0xC000_0000, // GENERIC_READ | GENERIC_WRITE
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .ok()
    }
}

fn lcd_levels(h: HANDLE) -> Vec<u8> {
    use windows::Win32::System::IO::DeviceIoControl;
    let mut buf = [0u8; 256];
    let mut n = 0u32;
    unsafe {
        if DeviceIoControl(
            h,
            IOCTL_SUPPORTED,
            None,
            0,
            Some(buf.as_mut_ptr() as *mut _),
            buf.len() as u32,
            Some(&mut n),
            None,
        )
        .is_err()
        {
            return Vec::new();
        }
    }
    buf[..(n as usize).min(256)].to_vec()
}

fn native_brightness_get() -> Option<u32> {
    use windows::Win32::System::IO::DeviceIoControl;
    let h = lcd_open()?;
    let mut b = DisplayBrightness::default();
    let mut n = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_QUERY,
            None,
            0,
            Some(&mut b as *mut _ as *mut _),
            std::mem::size_of::<DisplayBrightness>() as u32,
            Some(&mut n),
            None,
        )
        .is_ok()
    };
    unsafe {
        let _ = CloseHandle(h);
    }
    if !ok {
        return None;
    }
    // On battery the DC level is the live one.
    let on_ac = battery_info().map(|x| x.plugged).unwrap_or(true);
    Some(if on_ac { b.ac } else { b.dc } as u32)
}

fn native_brightness_set(percent: u32) -> bool {
    use windows::Win32::System::IO::DeviceIoControl;
    let Some(h) = lcd_open() else { return false };
    let levels = lcd_levels(h);
    // The driver only accepts its own levels, so snap to the nearest one.
    let want = percent.min(100) as i32;
    let v = levels
        .iter()
        .copied()
        .min_by_key(|l| (*l as i32 - want).abs())
        .unwrap_or(want as u8);
    let b = DisplayBrightness { policy: 3, ac: v, dc: v };
    let mut n = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_SET,
            Some(&b as *const _ as *const _),
            std::mem::size_of::<DisplayBrightness>() as u32,
            None,
            0,
            Some(&mut n),
            None,
        )
        .is_ok()
    };
    unsafe {
        let _ = CloseHandle(h);
    }
    if !ok {
        log_brightness("native brightness set failed, falling back to WMI");
    }
    ok
}

pub fn brightness_load(hwnd: isize) {
    std::thread::spawn(move || {
        let v = native_brightness_get().or_else(|| {
            powershell(
                "$b = (Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction SilentlyContinue | Select-Object -First 1).CurrentBrightness; if ($null -eq $b) { $b = (Get-WmiObject -Namespace root/WMI -Class WmiMonitorBrightness -ErrorAction SilentlyContinue | Select-Object -First 1).CurrentBrightness }; Write-Output $b",
            )
            .and_then(|s| {
                let t = s.trim().to_string();
                let p = t.parse::<u32>().ok();
                if p.is_none() {
                    log_brightness(&format!("brightness query returned: {:?}", t));
                }
                p
            })
        });
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd as *mut _)),
                WM_BRIGHT,
                WPARAM(v.is_some() as usize),
                LPARAM(v.unwrap_or(0) as isize),
            );
        }
    });
}

static BRI_PENDING: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static BRI_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set the brightness. Safe to call many times while dragging: calls are coalesced
/// so only the latest value is applied, one at a time.
pub fn brightness_set(percent: u32) {
    use std::sync::atomic::Ordering::SeqCst;
    BRI_PENDING.store(percent.min(100) as i32, SeqCst);
    if BRI_RUNNING.swap(true, SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        let v = BRI_PENDING.swap(-1, SeqCst);
        if v >= 0 {
            if !native_brightness_set(v as u32) {
                let _ = powershell(&format!(
                    "Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{{Timeout=1;Brightness={}}}",
                    v
                ));
            }
            continue;
        }
        BRI_RUNNING.store(false, SeqCst);
        // a call may have slipped in between the swap and the store
        if BRI_PENDING.load(SeqCst) >= 0 && !BRI_RUNNING.swap(true, SeqCst) {
            continue;
        }
        break;
    });
}

// ------------------------------------------------------- status bar helpers

/// True when Windows is set to the light theme for the system UI (taskbar etc.).
pub fn light_theme() -> bool {
    unsafe {
        let mut v = 0u32;
        let mut n = 4u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut _),
            Some(&mut n),
        );
        r == ERROR_SUCCESS && v == 1
    }
}

pub struct BatteryInfo {
    pub percent: u8,
    pub charging: bool,
    pub plugged: bool,
    pub secs_left: Option<u32>,
    pub saver: bool,
}

/// None on machines without a battery.
pub fn battery_info() -> Option<BatteryInfo> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    unsafe {
        let mut s = SYSTEM_POWER_STATUS::default();
        GetSystemPowerStatus(&mut s).ok()?;
        if s.BatteryFlag & 128 != 0 || s.BatteryLifePercent > 100 {
            return None;
        }
        Some(BatteryInfo {
            percent: s.BatteryLifePercent,
            charging: s.BatteryFlag & 8 != 0,
            plugged: s.ACLineStatus == 1,
            secs_left: (s.BatteryLifeTime != u32::MAX).then_some(s.BatteryLifeTime),
            saver: s.SystemStatusFlag == 1,
        })
    }
}

/// (percent, charging) or None on machines without a battery.
pub fn battery() -> Option<(u8, bool)> {
    battery_info().map(|b| (b.percent, b.charging || b.plugged))
}

/// Posted when the Wi-Fi name lookup finished.
pub const WM_POP: u32 = 0x8000 + 5; // WM_APP + 5

#[derive(Clone, Default)]
pub struct WifiNet {
    pub ssid: String,
    pub signal: u32,
    pub secure: bool,
    pub saved: bool,
    /// Authentication as reported by Windows, e.g. "WPA2-Personal".
    pub auth: String,
}

#[derive(Clone, Default)]
pub struct WifiState {
    pub connected: Option<String>,
    pub nets: Vec<WifiNet>,
    /// Network we are currently trying to join.
    pub busy: Option<String>,
    /// Outcome of the last connect attempt (text, success); taken by the UI.
    pub last: Option<(String, bool)>,
}

pub static WIFI: std::sync::Mutex<WifiState> = std::sync::Mutex::new(WifiState {
    connected: None,
    nets: Vec::new(),
    busy: None,
    last: None,
});

/// Run `netsh` with a raw argument string (so quotes around names survive).
fn netsh(args: &str) -> String {
    use std::process::{Command, Stdio};
    let mut c = Command::new("netsh");
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.raw_arg(args);
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    {
        c.args(args.split_whitespace());
    }
    match c.output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => String::new(),
    }
}

fn value_after_colon(l: &str) -> Option<&str> {
    l.split_once(':').map(|(_, v)| v.trim())
}

fn wifi_scan_blocking() -> (Option<String>, Vec<WifiNet>) {
    // connected network
    let mut connected = None;
    for line in netsh("wlan show interfaces").lines() {
        let l = line.trim();
        if l.starts_with("SSID") && !l.starts_with("BSSID") {
            if let Some(v) = value_after_colon(l) {
                if !v.is_empty() {
                    connected = Some(v.to_string());
                }
            }
            break;
        }
    }
    // saved profiles ("All User Profile     : Name")
    let mut saved: Vec<String> = Vec::new();
    for line in netsh("wlan show profiles").lines() {
        let l = line.trim();
        if l.contains("Profile") && l.contains(':') {
            if let Some(v) = value_after_colon(l) {
                if !v.is_empty() {
                    saved.push(v.to_string());
                }
            }
        }
    }
    // visible networks
    let mut nets: Vec<WifiNet> = Vec::new();
    let mut cur: Option<WifiNet> = None;
    for line in netsh("wlan show networks mode=bssid").lines() {
        let l = line.trim();
        if l.starts_with("SSID ") {
            if let Some(n) = cur.take() {
                nets.push(n);
            }
            let name = value_after_colon(l).unwrap_or("").to_string();
            cur = Some(WifiNet {
                saved: saved.iter().any(|s| *s == name),
                ssid: name,
                ..Default::default()
            });
        } else if let Some(n) = cur.as_mut() {
            if l.starts_with("Authentication") {
                let a = value_after_colon(l).unwrap_or("");
                n.auth = a.to_string();
                n.secure = !a.eq_ignore_ascii_case("Open");
            } else if l.starts_with("Signal") {
                let pct = value_after_colon(l)
                    .unwrap_or("")
                    .trim_end_matches('%')
                    .trim()
                    .parse::<u32>()
                    .unwrap_or(0);
                n.signal = n.signal.max(pct);
            }
        }
    }
    if let Some(n) = cur.take() {
        nets.push(n);
    }
    nets.retain(|n| !n.ssid.is_empty());
    nets.sort_by(|a, b| {
        let ac = connected.as_deref() == Some(a.ssid.as_str());
        let bc = connected.as_deref() == Some(b.ssid.as_str());
        bc.cmp(&ac).then(b.signal.cmp(&a.signal))
    });
    nets.dedup_by(|a, b| a.ssid == b.ssid);
    nets.truncate(6);
    (connected, nets)
}

fn wifi_post(hwnd: isize) {
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_POP, WPARAM(0), LPARAM(0));
    }
}

/// Refresh the connected network and the list of visible ones (only when the
/// dropdown opens).
pub fn wifi_scan_async(hwnd: isize) {
    std::thread::spawn(move || {
        let (connected, nets) = wifi_scan_blocking();
        let mut w = WIFI.lock().unwrap();
        w.connected = connected;
        w.nets = nets;
        drop(w);
        wifi_post(hwnd);
    });
}

/// SSID we are fully connected to (None while associating or disconnected).
fn wifi_connected_ssid() -> Option<String> {
    let out = netsh("wlan show interfaces");
    let mut ssid = None;
    let mut up = false;
    for line in out.lines() {
        let l = line.trim();
        if l.starts_with("SSID") && !l.starts_with("BSSID") && ssid.is_none() {
            ssid = value_after_colon(l).filter(|v| !v.is_empty()).map(|v| v.to_string());
        } else if l.starts_with("State") {
            up = value_after_colon(l)
                .map(|v| v.eq_ignore_ascii_case("connected"))
                .unwrap_or(false);
        }
    }
    if up {
        ssid
    } else {
        None
    }
}

/// Wait up to ~10 s for the connection to come up; returns true if it did.
fn wifi_wait_connected(ssid: &str) -> bool {
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        if wifi_connected_ssid().as_deref() == Some(ssid) {
            return true;
        }
    }
    false
}

fn wifi_finish(hwnd: isize, text: String, ok: bool) {
    let (connected, nets) = wifi_scan_blocking();
    let mut w = WIFI.lock().unwrap();
    w.connected = connected;
    w.nets = nets;
    w.busy = None;
    w.last = Some((text, ok));
    drop(w);
    wifi_post(hwnd);
}

/// Only the connected-network name (cheap; used for the hover tooltip).
pub fn wifi_status_async(hwnd: isize) {
    std::thread::spawn(move || {
        let c = wifi_connected_ssid();
        WIFI.lock().unwrap().connected = c;
        wifi_post(hwnd);
    });
}

/// Join a saved network. `None` disconnects.
pub fn wifi_connect_async(hwnd: isize, ssid: Option<String>) {
    WIFI.lock().unwrap().busy = ssid.clone();
    wifi_post(hwnd);
    std::thread::spawn(move || match ssid {
        Some(s) => {
            netsh(&format!("wlan connect name=\"{s}\" ssid=\"{s}\""));
            let ok = wifi_wait_connected(&s);
            let text = if ok {
                format!("Connected to {s}")
            } else {
                format!("Couldn't connect to {s}")
            };
            wifi_finish(hwnd, text, ok);
        }
        None => {
            netsh("wlan disconnect");
            std::thread::sleep(std::time::Duration::from_millis(1200));
            wifi_finish(hwnd, "Disconnected".into(), true);
        }
    });
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Is this a network type we can join with just a password?
pub fn wifi_auth_supported(auth: &str) -> bool {
    let a = auth.to_ascii_lowercase();
    a == "open" || a == "wpa2-personal" || a == "wpa3-personal" || a == "wpa-personal"
}

fn wifi_profile_xml(ssid: &str, auth: &str, key: Option<&str>) -> String {
    let a = auth.to_ascii_lowercase();
    let (authn, enc) = match (key.is_some(), a.as_str()) {
        (false, _) => ("open", "none"),
        (true, "wpa3-personal") => ("WPA3SAE", "AES"),
        (true, "wpa-personal") => ("WPAPSK", "TKIP"),
        _ => ("WPA2PSK", "AES"),
    };
    let ssid = xml_escape(ssid);
    let shared = match key {
        Some(k) => format!(
            "<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>",
            xml_escape(k)
        ),
        None => String::new(),
    };
    format!(
        "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\
<name>{ssid}</name><SSIDConfig><SSID><name>{ssid}</name></SSID></SSIDConfig>\
<connectionType>ESS</connectionType><connectionMode>auto</connectionMode>\
<MSM><security><authEncryption><authentication>{authn}</authentication><encryption>{enc}</encryption><useOneX>false</useOneX></authEncryption>{shared}</security></MSM></WLANProfile>"
    )
}

/// Join a network that has no saved profile (open, or with a password the user
/// just typed). The profile is kept only if the connection works; the temporary
/// file holding the password is deleted straight away.
pub fn wifi_join_new_async(hwnd: isize, ssid: String, auth: String, key: Option<String>) {
    WIFI.lock().unwrap().busy = Some(ssid.clone());
    wifi_post(hwnd);
    std::thread::spawn(move || {
        let xml = wifi_profile_xml(&ssid, &auth, key.as_deref());
        let path = std::env::temp_dir().join(format!("nock-wlan-{}.xml", std::process::id()));
        let wrote = std::fs::write(&path, xml).is_ok();
        let mut ok = false;
        if wrote {
            netsh(&format!("wlan add profile filename=\"{}\" user=current", path.display()));
            let _ = std::fs::remove_file(&path);
            netsh(&format!("wlan connect name=\"{ssid}\" ssid=\"{ssid}\""));
            ok = wifi_wait_connected(&ssid);
            if !ok {
                // do not keep a profile with a wrong password
                netsh(&format!("wlan delete profile name=\"{ssid}\""));
            }
        }
        let text = if ok {
            format!("Connected to {ssid}")
        } else if key.is_some() {
            format!("Couldn't join {ssid}. Check the password")
        } else {
            format!("Couldn't connect to {ssid}")
        };
        wifi_finish(hwnd, text, ok);
    });
}

/// Friendly name of the app owning the foreground window (None if it is one of `skip`).
pub fn foreground_app_name(skip: &[HWND]) -> Option<String> {
    use windows::Win32::System::Threading::*;
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() || skip.contains(&fg) {
            return None;
        }
        // With nothing open the foreground is the desktop or taskbar, which belong
        // to explorer.exe but are not a File Explorer window.
        let mut cls = [0u16; 64];
        let n = GetClassNameW(fg, &mut cls) as usize;
        match String::from_utf16_lossy(&cls[..n.min(64)]).as_str() {
            "Progman" | "WorkerW" => return Some("Desktop".to_string()),
            "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "NotifyIconOverflowWindow" => return None,
            _ => {}
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let stem = Path::new(&path).file_stem()?.to_string_lossy().to_string();
        Some(friendly(&stem))
    }
}

fn friendly(stem: &str) -> String {
    match stem.to_ascii_lowercase().as_str() {
        "msedge" => "Microsoft Edge".into(),
        "chrome" => "Google Chrome".into(),
        "firefox" => "Firefox".into(),
        "code" => "VS Code".into(),
        "explorer" => "File Explorer".into(),
        "windowsterminal" => "Terminal".into(),
        "applicationframehost" => "App".into(),
        "searchhost" | "startmenuexperiencehost" => "Windows".into(),
        "spotify" => "Spotify".into(),
        "whatsapp" | "whatsapp.root" => "WhatsApp".into(),
        _ => {
            let mut c = stem.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
    }
}

// ------------------------------------------------------------ reserved strip

/// Reserve (or release) a strip of `height_px` at the top of the monitor so
/// maximised windows stop below the notch. Pass `height_px = 0` to release.
pub fn reserve_top(hwnd: HWND, mon: &RECT, height_px: i32) {
    const ABM_NEW: u32 = 0;
    const ABM_REMOVE: u32 = 1;
    const ABM_SETPOS: u32 = 3;
    const ABE_TOP: u32 = 1;
    thread_local! {
        static REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    unsafe {
        let mut abd = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: hwnd,
            ..Default::default()
        };
        let reg = REGISTERED.with(|r| r.get());
        if height_px <= 0 {
            if reg {
                SHAppBarMessage(ABM_REMOVE, &mut abd);
                REGISTERED.with(|r| r.set(false));
            }
            return;
        }
        if !reg {
            SHAppBarMessage(ABM_NEW, &mut abd);
            REGISTERED.with(|r| r.set(true));
        }
        abd.uEdge = ABE_TOP;
        abd.rc = RECT {
            left: mon.left,
            top: mon.top,
            right: mon.right,
            bottom: mon.top + height_px,
        };
        SHAppBarMessage(ABM_SETPOS, &mut abd);
    }
}

// ---------------------------------------------------------------- launching

pub fn open(target: &str, args: &str) {
    unsafe {
        let t = HSTRING::from(target);
        let a = HSTRING::from(args);
        let params = if args.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(a.as_ptr())
        };
        ShellExecuteW(None, w!("open"), &t, params, PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn lock_pc() {
    unsafe {
        let _ = LockWorkStation();
    }
}

pub fn sleep_pc() {
    open("rundll32.exe", "powrprof.dll,SetSuspendState 0,1,0");
}

// ---------------------------------------------------------------- clipboard

/// Password managers mark their copies with this format; we must not record them.
pub fn clipboard_excluded() -> bool {
    unsafe {
        let f = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        f != 0 && IsClipboardFormatAvailable(f).is_ok()
    }
}

pub fn clipboard_text(hwnd: HWND) -> Option<String> {
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return None;
        }
        let result = (|| {
            let h = GetClipboardData(CF_UNICODETEXT).ok()?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return None;
            }
            let mut len = 0usize;
            while *p.add(len) != 0 && len < 20_000 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            let _ = GlobalUnlock(g);
            Some(s)
        })();
        let _ = CloseClipboard();
        result
    }
}

pub fn set_clipboard_text(hwnd: HWND, s: &str) -> bool {
    unsafe {
        let w: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) else {
            return false;
        };
        let p = GlobalLock(hmem) as *mut u16;
        if p.is_null() {
            let _ = GlobalFree(Some(hmem));
            return false;
        }
        std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
        let _ = GlobalUnlock(hmem);
        if OpenClipboard(Some(hwnd)).is_err() {
            let _ = GlobalFree(Some(hmem));
            return false;
        }
        let _ = EmptyClipboard();
        let ok = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0))).is_ok();
        let _ = CloseClipboard();
        if !ok {
            let _ = GlobalFree(Some(hmem));
        }
        ok
    }
}

pub fn send_ctrl_v() {
    unsafe {
        let key = |vk: u16, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [
            key(0x11, false),
            key(0x56, false),
            key(0x56, true),
            key(0x11, true),
        ];
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

// ---------------------------------------------------------------- autostart

pub fn autostart_enabled() -> bool {
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            w!("Nock"),
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        ) == ERROR_SUCCESS
    }
}

pub fn set_autostart(on: bool) {
    unsafe {
        let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        if on {
            if let Ok(exe) = std::env::current_exe() {
                let cmd = format!("\"{}\"", exe.display());
                let w: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key,
                    w!("Nock"),
                    REG_SZ.0,
                    Some(w.as_ptr() as *const _),
                    (w.len() * 2) as u32,
                );
            }
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, key, w!("Nock"));
        }
    }
}

// ------------------------------------------------------------------- state

/// 0 = none, 1 = a full-screen video/browser/app (the bar can be peeked at),
/// 2 = a game or presentation (stay out of the way completely).
/// `skip` are our own windows; `mon` is the primary monitor rectangle.
pub fn fullscreen_kind(skip: &[HWND], mon: &RECT) -> u8 {
    unsafe {
        match SHQueryUserNotificationState() {
            Ok(s) if s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE => return 2,
            Ok(s) if s == QUNS_BUSY => return 1,
            _ => {}
        }
        // The shell does not report every full-screen window (browser/YouTube full
        // screen often slips through), so also look at the foreground window itself.
        // A maximised window overhangs the monitor by its invisible border; a
        // full-screen one matches the monitor rectangle exactly.
        let fg = GetForegroundWindow();
        if fg.is_invalid() || skip.contains(&fg) {
            return 0;
        }
        let mut cls = [0u16; 64];
        let n = GetClassNameW(fg, &mut cls) as usize;
        let cls = String::from_utf16_lossy(&cls[..n.min(64)]);
        if matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return 0;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return 0;
        }
        let exact = r.left == mon.left && r.top == mon.top && r.right == mon.right && r.bottom == mon.bottom;
        if exact {
            1
        } else {
            0
        }
    }
}

// ------------------------------------------- full-screen change notifications
// Entering full screen inside the same window (YouTube, F11) does not change the
// foreground window, so also listen for the foreground window moving or resizing.

/// Posted when the foreground window changed size or position.
pub const WM_FSCHK: u32 = 0x8000 + 7; // WM_APP + 7

static LOC_HOOK: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static LOC_NOTIFY: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

unsafe extern "system" fn loc_proc(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    use std::sync::atomic::Ordering::Relaxed;
    if object == 0 && hwnd == GetForegroundWindow() {
        let _ = PostMessageW(
            Some(HWND(LOC_NOTIFY.load(Relaxed) as *mut _)),
            WM_FSCHK,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

/// (Re)attach the location listener to the process that owns the foreground window.
pub fn watch_foreground_location(notify: HWND) {
    use std::sync::atomic::Ordering::Relaxed;
    use windows::Win32::UI::Accessibility::*;
    unsafe {
        LOC_NOTIFY.store(notify.0 as isize, Relaxed);
        let old = LOC_HOOK.swap(0, Relaxed);
        if old != 0 {
            let _ = UnhookWinEvent(HWINEVENTHOOK(old as *mut _));
        }
        let fg = GetForegroundWindow();
        if fg.is_invalid() || fg == notify {
            return;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == 0 {
            return;
        }
        let h = SetWinEventHook(
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(loc_proc),
            pid,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        LOC_HOOK.store(h.0 as isize, Relaxed);
    }
}

// ------------------------------------------------------- top-edge detector
// While a full-screen app is running we watch for the pointer touching the
// top edge (centre zone) so the bar can slide in. The hook exists only during
// full screen; its callback does a few integer compares and nothing else.

/// Posted once when the pointer enters the top-edge zone.
pub const WM_EDGE: u32 = 0x8000 + 6; // WM_APP + 6

static EDGE_HOOK: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static EDGE_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static EDGE_L: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static EDGE_R: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static EDGE_Y: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static EDGE_IN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

unsafe extern "system" fn edge_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    use std::sync::atomic::Ordering::Relaxed;
    if code >= 0 && wp.0 as u32 == WM_MOUSEMOVE {
        let ms = &*(lp.0 as *const MSLLHOOKSTRUCT);
        let inside = ms.pt.y <= EDGE_Y.load(Relaxed)
            && ms.pt.x >= EDGE_L.load(Relaxed)
            && ms.pt.x < EDGE_R.load(Relaxed);
        if inside != EDGE_IN.swap(inside, Relaxed) && inside {
            let _ = PostMessageW(
                Some(HWND(EDGE_HWND.load(Relaxed) as *mut _)),
                WM_EDGE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

/// Start or stop watching the top edge. `zone` = (left, right, y) in screen pixels.
pub fn edge_watch(on: bool, hwnd: HWND, zone: (i32, i32, i32)) {
    use std::sync::atomic::Ordering::Relaxed;
    unsafe {
        EDGE_L.store(zone.0, Relaxed);
        EDGE_R.store(zone.1, Relaxed);
        EDGE_Y.store(zone.2, Relaxed);
        EDGE_HWND.store(hwnd.0 as isize, Relaxed);
        let cur = EDGE_HOOK.load(Relaxed);
        if on && cur == 0 {
            EDGE_IN.store(false, Relaxed);
            let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                .map(|m| HINSTANCE(m.0))
                .ok();
            if let Ok(h) = SetWindowsHookExW(WH_MOUSE_LL, Some(edge_proc), hinst, 0) {
                EDGE_HOOK.store(h.0 as isize, Relaxed);
            }
        } else if !on && cur != 0 {
            let _ = UnhookWindowsHookEx(HHOOK(cur as *mut _));
            EDGE_HOOK.store(0, Relaxed);
        }
    }
}

/// (year, month 1-12, day, weekday 0=Sunday, hour, minute, second, millisecond)
pub fn local_time() -> (i32, u32, u32, u32, u32, u32, u32, u32) {
    unsafe {
        let t = GetLocalTime();
        (
            t.wYear as i32,
            t.wMonth as u32,
            t.wDay as u32,
            t.wDayOfWeek as u32,
            t.wHour as u32,
            t.wMinute as u32,
            t.wSecond as u32,
            t.wMilliseconds as u32,
        )
    }
}

pub fn filetime_now() -> i64 {
    unsafe {
        let ft = GetSystemTimePreciseAsFileTime();
        ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
    }
}

// -------------------------------------------------------------------- icons

/// Resolve "calc.exe" / "C:\\...\\app.exe" to a real path so we can fetch its icon.
pub fn resolve_path(target: &str) -> Option<PathBuf> {
    let p = Path::new(target);
    if p.is_absolute() {
        return p.exists().then(|| p.to_path_buf());
    }
    if target.contains("://") || target.ends_with(':') || target.contains(':') {
        return None; // URL or ms-settings: etc.
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(root) = std::env::var_os("SystemRoot") {
        let root = PathBuf::from(root);
        dirs.push(root.join("System32"));
        dirs.push(root);
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.into_iter()
        .map(|d| d.join(target))
        .find(|c| c.exists())
}

pub fn file_icon(path: &Path) -> Option<HICON> {
    unsafe {
        let w = HSTRING::from(path.as_os_str().to_string_lossy().as_ref());
        let mut sfi = SHFILEINFOW::default();
        let r = SHGetFileInfoW(
            &w,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut sfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if r == 0 || sfi.hIcon.is_invalid() {
            None
        } else {
            Some(sfi.hIcon)
        }
    }
}

/// Idle and total CPU time (100 ns ticks) since boot, for load between two reads.
pub fn cpu_times() -> (u64, u64) {
    use windows::Win32::System::Threading::GetSystemTimes;
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    let q = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    unsafe {
        if GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).is_err() {
            return (0, 0);
        }
    }
    // kernel time already includes idle time
    (q(idle), q(kernel) + q(user))
}

/// Percent of physical memory in use.
pub fn mem_percent() -> u32 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe {
        let _ = GlobalMemoryStatusEx(&mut m);
    }
    m.dwMemoryLoad
}
