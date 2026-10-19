//! Small wrappers around the Windows calls the notch uses for its actions.
//! Everything here runs only in response to a user action or a Windows event.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
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
    static ENUMERATOR: RefCell<Option<IMMDeviceEnumerator>> = const { RefCell::new(None) };
    static ENDPOINT: RefCell<Option<(IAudioEndpointVolume, String)>> = const { RefCell::new(None) };
}

/// The volume control of whatever output device is the default *right now*. The
/// default is looked up on every call (it is cheap) and the control is re-bound
/// the moment it changes, so switching between laptop speakers, Bluetooth or a
/// monitor is followed immediately.
fn endpoint() -> Option<IAudioEndpointVolume> {
    unsafe {
        let en = ENUMERATOR.with(|e| {
            if e.borrow().is_none() {
                *e.borrow_mut() = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok();
            }
            e.borrow().clone()
        })?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()?;
        let id_ptr = dev.GetId().ok()?;
        let id = id_ptr.to_string().unwrap_or_default();
        windows::Win32::System::Com::CoTaskMemFree(Some(id_ptr.0 as *const _));
        ENDPOINT.with(|e| {
            let same = e.borrow().as_ref().map(|(_, cur)| *cur == id).unwrap_or(false);
            if !same {
                let ep = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?;
                *e.borrow_mut() = Some((ep, id));
            }
            e.borrow().as_ref().map(|(ep, _)| ep.clone())
        })
    }
}

fn drop_endpoint() {
    ENDPOINT.with(|e| *e.borrow_mut() = None);
    ENUMERATOR.with(|e| *e.borrow_mut() = None);
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

/// Default microphone: current mute state, or flip it. Returns the new state.
pub fn mic_muted() -> bool {
    mic_endpoint().and_then(|ep| unsafe { ep.GetMute().ok() }).is_some_and(|m| m.as_bool())
}

pub fn toggle_mic_mute() -> bool {
    let now = !mic_muted();
    if let Some(ep) = mic_endpoint() {
        unsafe {
            let _ = ep.SetMute(now, std::ptr::null());
        }
    }
    now
}

fn mic_endpoint() -> Option<IAudioEndpointVolume> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eCapture, eConsole).ok()?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

// -------------------------------------------------------------- brightness
// Internal laptop panels are only reachable through WMI. Spawning PowerShell
// is slow (~0.3 s) but only happens on open / on slider release, never idle.

pub(crate) fn powershell(cmd: &str) -> Option<String> {
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
/// Press and release a media key (play/pause 0xB3, next 0xB0, previous 0xB1).
pub fn media_key(vk: u16) {
    unsafe {
        let key = |up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        SendInput(&[key(false), key(true)], std::mem::size_of::<INPUT>() as i32);
    }
}

pub fn log_media(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(crate::store::data_dir().join("media.log")) {
        let _ = writeln!(f, "{msg}");
    }
}

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

/// True while some app is using the microphone (Windows records it in the privacy
/// consent store: an app that is recording has a start time and no stop time yet).
pub fn mic_in_use() -> bool {
    mic_session_active() || mic_in_use_registry()
}

/// True when some other process has an active capture stream on a default microphone.
/// Unlike the registry log this is live, so it also sees apps like Sound Recorder.
fn mic_session_active() -> bool {
    use windows::Win32::Media::Audio::{AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2};
    let pid = std::process::id();
    unsafe {
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return false };
        for role in [eConsole, eCommunications] {
            let Ok(dev) = en.GetDefaultAudioEndpoint(eCapture, role) else { continue };
            let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let Ok(list) = mgr.GetSessionEnumerator() else { continue };
            for i in 0..list.GetCount().unwrap_or(0) {
                let Ok(c) = list.GetSession(i) else { continue };
                if c.GetState().ok() != Some(AudioSessionStateActive) {
                    continue;
                }
                if let Ok(c2) = c.cast::<IAudioSessionControl2>() {
                    if c2.GetProcessId().unwrap_or(0) != pid && c2.IsSystemSoundsSession() != S_OK {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn mic_in_use_registry() -> bool {
    const BASE: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone";
    let qword = |path: &str, name: PCWSTR| -> Option<u64> {
        let mut v = 0u64;
        let mut n = 8u32;
        let p = HSTRING::from(path);
        let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, &p, name, RRF_RT_REG_QWORD, None, Some(&mut v as *mut u64 as *mut _), Some(&mut n)) };
        (r == ERROR_SUCCESS).then_some(v)
    };
    let subkeys = |path: &str| -> Vec<String> {
        let mut out = Vec::new();
        let p = HSTRING::from(path);
        unsafe {
            let mut k = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, &p, None, KEY_READ, &mut k) != ERROR_SUCCESS {
                return out;
            }
            for i in 0.. {
                let mut buf = [0u16; 512];
                let mut n = buf.len() as u32;
                if RegEnumKeyExW(k, i, Some(PWSTR(buf.as_mut_ptr())), &mut n, None, None, None, None) != ERROR_SUCCESS {
                    break;
                }
                out.push(String::from_utf16_lossy(&buf[..n as usize]));
            }
            let _ = RegCloseKey(k);
        }
        out
    };
    let mut paths: Vec<String> = Vec::new();
    for name in subkeys(BASE) {
        if name == "NonPackaged" {
            paths.extend(subkeys(&format!("{BASE}\\NonPackaged")).into_iter().map(|n| format!("{BASE}\\NonPackaged\\{n}")));
        } else {
            paths.push(format!("{BASE}\\{name}"));
        }
    }
    paths.iter().any(|p| qword(p, w!("LastUsedTimeStart")).unwrap_or(0) != 0 && qword(p, w!("LastUsedTimeStop")) == Some(0))
}

pub const WM_MIC: u32 = 0x8000 + 11; // WM_APP + 11

const MIC_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

/// Post WM_MIC whenever Windows changes the microphone consent store (an app starting or
/// stopping recording). The thread sleeps inside the registry wait, so it costs nothing idle.
pub fn watch_mic(hwnd: isize) {
    std::thread::spawn(move || loop {
        unsafe {
            let mut k = HKEY::default();
            let key = HSTRING::from(MIC_KEY);
            if RegOpenKeyExW(HKEY_CURRENT_USER, &key, None, KEY_NOTIFY, &mut k) != ERROR_SUCCESS {
                std::thread::sleep(std::time::Duration::from_secs(30)); // no app has used the mic yet
                continue;
            }
            let r = RegNotifyChangeKeyValue(k, true, REG_NOTIFY_CHANGE_LAST_SET | REG_NOTIFY_CHANGE_NAME, None, false);
            let _ = RegCloseKey(k);
            if r == ERROR_SUCCESS {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_MIC, WPARAM(0), LPARAM(0));
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(300)); // one burst of writes = one update
    });
}

/// Tells the window when a microphone session appears or starts/stops capturing.
#[windows::core::implement(windows::Win32::Media::Audio::IAudioSessionEvents)]
struct MicEvents(isize);

impl windows::Win32::Media::Audio::IAudioSessionEvents_Impl for MicEvents_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const GUID) -> Result<()> {
        Ok(())
    }
    fn OnIconPathChanged(&self, _: &PCWSTR, _: *const GUID) -> Result<()> {
        Ok(())
    }
    fn OnSimpleVolumeChanged(&self, _: f32, _: BOOL, _: *const GUID) -> Result<()> {
        Ok(())
    }
    fn OnChannelVolumeChanged(&self, _: u32, _: *const f32, _: u32, _: *const GUID) -> Result<()> {
        Ok(())
    }
    fn OnGroupingParamChanged(&self, _: *const GUID, _: *const GUID) -> Result<()> {
        Ok(())
    }
    fn OnStateChanged(&self, _: windows::Win32::Media::Audio::AudioSessionState) -> Result<()> {
        post_mic(self.0);
        Ok(())
    }
    fn OnSessionDisconnected(&self, _: windows::Win32::Media::Audio::AudioSessionDisconnectReason) -> Result<()> {
        post_mic(self.0);
        Ok(())
    }
}

#[windows::core::implement(windows::Win32::Media::Audio::IAudioSessionNotification)]
struct MicNotify(isize);

impl windows::Win32::Media::Audio::IAudioSessionNotification_Impl for MicNotify_Impl {
    fn OnSessionCreated(&self, new: Ref<windows::Win32::Media::Audio::IAudioSessionControl>) -> Result<()> {
        if let Some(c) = new.as_ref() {
            watch_session(c, self.0);
        }
        post_mic(self.0);
        Ok(())
    }
}

fn post_mic(hwnd: isize) {
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_MIC, WPARAM(0), LPARAM(0));
    }
}

fn watch_session(c: &windows::Win32::Media::Audio::IAudioSessionControl, hwnd: isize) {
    use windows::Win32::Media::Audio::IAudioSessionEvents;
    let ev: IAudioSessionEvents = MicEvents(hwnd).into();
    unsafe {
        let _ = c.RegisterAudioSessionNotification(&ev);
    }
    // the endpoint keeps the registration alive only while we hold both; leak them (a few per run)
    std::mem::forget(ev);
    std::mem::forget(c.clone());
}

/// Event-driven live mic watcher: new capture sessions and their start/stop post WM_MIC.
/// Runs on its own COM thread and sleeps; costs nothing while idle.
pub fn watch_mic_sessions(hwnd: isize) {
    std::thread::spawn(move || unsafe {
        use windows::Win32::Media::Audio::{IAudioSessionManager2, IAudioSessionNotification};
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return };
        let mut keep = Vec::new();
        for role in [eConsole, eCommunications] {
            let Ok(dev) = en.GetDefaultAudioEndpoint(eCapture, role) else { continue };
            let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let n: IAudioSessionNotification = MicNotify(hwnd).into();
            let _ = mgr.RegisterSessionNotification(&n);
            if let Ok(list) = mgr.GetSessionEnumerator() {
                for i in 0..list.GetCount().unwrap_or(0) {
                    if let Ok(c) = list.GetSession(i) {
                        watch_session(&c, hwnd);
                    }
                }
            }
            keep.push((mgr, n));
        }
        loop {
            std::thread::park();
        }
    });
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

/// Seconds since the last keyboard or mouse input.
pub fn idle_secs() -> u32 {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    unsafe {
        let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if !GetLastInputInfo(&mut li).as_bool() {
            return 0;
        }
        GetTickCount().wrapping_sub(li.dwTime) / 1000
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

/// True when a window is really full screen: it matches the monitor rectangle exactly
/// AND has no title bar. A maximised normal window also matches the monitor exactly when
/// the taskbar is set to auto-hide, so the title bar is what tells the two apart (browsers,
/// video players and games drop it when they go full screen).
pub(crate) fn is_fullscreen_window(r: &RECT, mon: &RECT, style: u32) -> bool {
    const WS_CAPTION_BITS: u32 = 0x00C0_0000; // WS_BORDER | WS_DLGFRAME
    let exact = r.left == mon.left && r.top == mon.top && r.right == mon.right && r.bottom == mon.bottom;
    exact && style & WS_CAPTION_BITS != WS_CAPTION_BITS
}

/// Window class name of `h` (for log lines).
pub fn class_of(h: HWND) -> String {
    unsafe {
        let mut cls = [0u16; 64];
        let n = GetClassNameW(h, &mut cls) as usize;
        String::from_utf16_lossy(&cls[..n.min(64)])
    }
}

/// Windows' own overlay windows (Start, Search, Task View, the Alt-Tab switcher, flyouts).
/// They can cover the whole monitor, and some linger as the foreground window after a
/// flyout closes, but they are not full-screen apps and must not hide Nock.
pub(crate) fn is_shell_overlay(class: &str) -> bool {
    matches!(
        class,
        "Windows.UI.Core.CoreWindow"
            | "XamlExplorerHostIslandWindow"
            | "MultitaskingViewFrame"
            | "ForegroundStaging"
            | "Windows.UI.Input.InputSite.WindowClass"
            | "NativeHWNDHost"
            | "TopLevelWindowForOverflowXamlIsland"
    )
}

static LAST_KIND: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn log_visibility(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(crate::store::data_dir().join("visibility.log")) {
        let _ = writeln!(f, "{msg}");
    }
}

/// 0 = none, 1 = a full-screen video/browser/app (the bar can be peeked at),
/// 2 = a game or presentation (stay out of the way completely).
/// `skip` are our own windows; `mon` is the primary monitor rectangle.
///
/// The shell's "user notification state" is only trusted when the foreground window
/// itself is really full screen: on some PCs it reports "busy" all the time, which used
/// to keep the notch hidden even with nothing full screen.
pub fn fullscreen_kind(skip: &[HWND], mon: &RECT) -> u8 {
    unsafe {
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
        let style = GetWindowLongPtrW(fg, GWL_STYLE) as u32;
        let kind = if is_fullscreen_window(&r, mon, style) {
            // Windows itself says a game or presentation owns the screen: trust that.
            // Otherwise a shell overlay (Start, Search, Task View...) is not full screen.
            let hard = matches!(SHQueryUserNotificationState(), Ok(s) if s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE);
            if hard {
                2
            } else if is_shell_overlay(&cls) {
                0
            } else {
                1
            }
        } else {
            0
        };
        // leave a trace when the verdict changes, so "it hides for no reason" can be diagnosed
        if LAST_KIND.swap(kind, std::sync::atomic::Ordering::Relaxed) != kind && kind != 0 {
            log_visibility(&format!(
                "hiding (kind {kind}): class '{cls}', rect {},{},{},{}, style {style:#x}, monitor {},{},{},{}",
                r.left, r.top, r.right, r.bottom, mon.left, mon.top, mon.right, mon.bottom
            ));
        }
        kind
    }
}

#[cfg(test)]
mod fullscreen_tests {
    use super::*;

    fn rc(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT { left: l, top: t, right: r, bottom: b }
    }

    #[test]
    fn maximised_window_is_not_full_screen() {
        let mon = rc(0, 0, 1920, 1080);
        // maximised with an auto-hidden taskbar: exact size, but it has a title bar
        assert!(!is_fullscreen_window(&rc(0, 0, 1920, 1080), &mon, 0x16CF_0000));
        // maximised normally: overhangs the monitor
        assert!(!is_fullscreen_window(&rc(-8, -8, 1928, 1040), &mon, 0x16CF_0000));
    }

    #[test]
    fn windows_overlays_do_not_count_as_full_screen_apps() {
        assert!(is_shell_overlay("Windows.UI.Core.CoreWindow"));
        assert!(is_shell_overlay("MultitaskingViewFrame"));
        assert!(!is_shell_overlay("Chrome_WidgetWin_1"));
        assert!(!is_shell_overlay("ApplicationFrameWindow"));
    }

    #[test]
    fn borderless_full_screen_is_detected() {
        let mon = rc(0, 0, 1920, 1080);
        assert!(is_fullscreen_window(&rc(0, 0, 1920, 1080), &mon, 0x9400_0000));
        assert!(!is_fullscreen_window(&rc(0, 0, 1900, 1080), &mon, 0x9400_0000));
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

thread_local! {
    static METER: RefCell<Option<(IAudioMeterInformation, std::time::Instant)>> = const { RefCell::new(None) };
}

/// Current output peak level 0..1 (what the speakers are playing), if available.
pub fn audio_peak() -> Option<f32> {
    METER.with(|m| {
        // re-bind every few seconds so a changed default output device is followed
        let stale = m.borrow().as_ref().map(|(_, t)| t.elapsed().as_secs() >= 3).unwrap_or(true);
        if stale {
            let got = unsafe {
                let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                let dev = en.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()?;
                dev.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None).ok()
            };
            *m.borrow_mut() = got.map(|g| (g, std::time::Instant::now()));
        }
        let meter = m.borrow().as_ref()?.0.clone();
        match unsafe { meter.GetPeakValue() } {
            Ok(v) => Some(v.clamp(0.0, 1.0)),
            Err(_) => {
                *m.borrow_mut() = None; // default device probably changed
                None
            }
        }
    })
}

fn shutdown_cmd(arg: &str) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("shutdown.exe").args([arg, "/t", "0"]).creation_flags(0x0800_0000).spawn();
}

pub fn restart_pc() {
    shutdown_cmd("/r");
}

pub fn shutdown_pc() {
    shutdown_cmd("/s");
}

/// Seconds since Windows started.
pub fn uptime_secs() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() / 1000 }
}

pub const WM_APPLIST: u32 = 0x8000 + 9; // WM_APP + 9 (8 is radio::WM_BTDONE)

/// Installed apps for the picker: (name, app id). Filled by `list_start_apps`.
pub static START_APPS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Ask Windows for every app in the Start menu (classic, Store and game launchers) in
/// the background, then post WM_APPLIST. Takes about a second.
pub fn list_start_apps(hwnd: isize) {
    std::thread::spawn(move || {
        let out = powershell("[Console]::OutputEncoding = [Text.Encoding]::UTF8; Get-StartApps | ForEach-Object { $_.Name + '|' + $_.AppID }").unwrap_or_default();
        let mut v: Vec<(String, String)> = out
            .lines()
            .filter_map(|l| {
                let (n, id) = l.split_once('|')?;
                let (n, id) = (n.trim(), id.trim());
                (!n.is_empty() && !id.is_empty()).then(|| (n.to_string(), id.to_string()))
            })
            .collect();
        // hide the entries nobody wants to pin
        let noisy = ["uninstall", "verifier", "readme", "release notes", "documentation", "eula"];
        v.retain(|(n, _)| {
            let l = n.to_lowercase();
            !noisy.iter().any(|x| l.contains(x))
        });
        v.sort_by_key(|(n, _)| n.to_lowercase());
        v.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
        *START_APPS.lock().unwrap() = v;
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APPLIST, WPARAM(0), LPARAM(0));
        }
    });
}

/// Icon of a Start-menu app (works for Store apps too) from its `shell:AppsFolder\...` name.
pub fn shell_icon(parse_name: &str) -> Option<HICON> {
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{SHParseDisplayName, SHGFI_PIDL};
    unsafe {
        let name = HSTRING::from(parse_name);
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(&name, None::<&windows::Win32::System::Com::IBindCtx>, &mut pidl, 0, None).ok()?;
        let mut sfi = SHFILEINFOW::default();
        let r = SHGetFileInfoW(
            PCWSTR(pidl as *const u16),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut sfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_ICON | SHGFI_LARGEICON,
        );
        windows::Win32::System::Com::CoTaskMemFree(Some(pidl as *const _));
        if r == 0 || sfi.hIcon.is_invalid() {
            None
        } else {
            Some(sfi.hIcon)
        }
    }
}

/// Where the Start Menu shortcut lives (per user, so no admin rights are needed).
fn start_menu_link() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(std::path::PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs\Nock.lnk"))
}

/// Add (or remove) a Start Menu shortcut so typing "Nock" in Windows search finds it. A
/// plain .exe is invisible to search; a shortcut in this folder is what makes an app
/// "installed". Rewritten at every start so it follows the exe if it is moved.
pub fn sync_start_menu(on: bool) {
    use windows::core::Interface;
    use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    let Some(lnk) = start_menu_link() else { return };
    if !on {
        let _ = std::fs::remove_file(&lnk);
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    unsafe {
        let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) else { return };
        let _ = link.SetPath(&HSTRING::from(exe.as_os_str().to_string_lossy().as_ref()));
        if let Some(dir) = exe.parent() {
            let _ = link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str().to_string_lossy().as_ref()));
        }
        let _ = link.SetDescription(w!("Nock - a notch and top bar for Windows"));
        let _ = link.SetIconLocation(&HSTRING::from(exe.as_os_str().to_string_lossy().as_ref()), 0);
        if let Ok(file) = link.cast::<IPersistFile>() {
            let _ = file.Save(&HSTRING::from(lnk.as_os_str().to_string_lossy().as_ref()), true);
        }
    }
}

#[cfg(test)]
mod start_menu_tests {
    #[test]
    #[ignore]
    fn creates_and_removes_the_shortcut() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        super::sync_start_menu(true);
        let lnk = super::start_menu_link().unwrap();
        let made = lnk.exists();
        println!("PROBE created={made} path={}", lnk.display());
        std::thread::sleep(std::time::Duration::from_secs(2));
        super::sync_start_menu(false);
        println!("PROBE removed={}", !lnk.exists());
    }
}

// ------------------------------------------------------------------ updates ----

pub const WM_UPDATE: u32 = 0x8000 + 10; // WM_APP + 10
const REPO: &str = "Akhilesh-Varute/Nock";

/// Newest release tag found by `check_update` (e.g. "v0.9.1").
pub static LATEST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// True if `latest` ("v1.2.3" or "1.2.3") is a higher version than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let nums = |s: &str| -> Vec<u32> { s.trim().trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    nums(latest) > nums(current)
}

/// Ask GitHub for the latest release in the background, then post WM_UPDATE:
/// WPARAM 0 = could not reach GitHub, 1 = up to date, 2 = a newer version exists.
pub fn check_update(hwnd: isize) {
    std::thread::spawn(move || {
        let cmd = format!(
            "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; (Invoke-RestMethod -UseBasicParsing -Headers @{{'User-Agent'='Nock'}} -Uri 'https://api.github.com/repos/{REPO}/releases/latest').tag_name"
        );
        let tag = powershell(&cmd).unwrap_or_default().trim().to_string();
        let code = if !tag.starts_with('v') {
            0
        } else if is_newer(&tag, env!("CARGO_PKG_VERSION")) {
            2
        } else {
            1
        };
        if code != 0 {
            *LATEST.lock().unwrap() = tag;
        }
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_UPDATE, WPARAM(code), LPARAM(0));
        }
    });
}

/// Whether this exe is the one the installer put in place (so the installer can replace it).
pub fn installed_by_setup() -> bool {
    let (Ok(exe), Ok(local)) = (std::env::current_exe(), std::env::var("LOCALAPPDATA")) else { return false };
    exe.starts_with(Path::new(&local).join("Programs").join("Nock"))
}

/// Download the installer for `tag`, run it silently (it stops this Nock and replaces the
/// exe), then start Nock again. Runs in a detached PowerShell so it outlives this process.
pub fn install_update(tag: &str) {
    let tag = tag.to_string();
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default().replace('\'', "''");
    std::thread::spawn(move || {
        let ver = tag.trim_start_matches('v');
        let cmd = format!(
            "$ErrorActionPreference='Stop'; try {{ [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
             $f = Join-Path $env:TEMP 'NockSetup-update.exe'; \
             Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/{REPO}/releases/download/{tag}/NockSetup-{ver}.exe' -OutFile $f; \
             Start-Process -FilePath $f -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -Wait }} catch {{}}; \
             Start-Process -FilePath '{exe}'"
        );
        let _ = powershell(&cmd);
    });
}

/// Update a copy that was not installed by the installer (the portable exe): download
/// `Nock-portable-<ver>.exe`, and only once that worked stop this Nock, put the new file
/// over this exe and start it. If the file cannot be replaced (a read-only folder) the
/// downloads page opens instead. A failed download leaves Nock running untouched.
pub fn update_portable(tag: &str) {
    let tag = tag.to_string();
    let pid = std::process::id();
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default().replace('\'', "''");
    std::thread::spawn(move || {
        let ver = tag.trim_start_matches('v');
        let cmd = format!(
            "$ErrorActionPreference='Stop'; try {{ [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
             $f = Join-Path $env:TEMP 'Nock-portable-update.exe'; \
             Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/{REPO}/releases/download/{tag}/Nock-portable-{ver}.exe' -OutFile $f; \
             if ((Get-Item $f).Length -lt 100000) {{ throw 'bad download' }} }} catch {{ exit 1 }}; \
             Stop-Process -Id {pid} -Force; Start-Sleep -Seconds 2; \
             try {{ Copy-Item -Force $f '{exe}' }} catch {{ Start-Process 'https://github.com/{REPO}/releases/latest' }}; \
             Start-Process -FilePath '{exe}'"
        );
        let _ = powershell(&cmd);
    });
}

/// Open the releases page (for copies that were not installed by the installer).
pub fn open_releases() {
    open(&format!("https://github.com/{REPO}/releases/latest"), "");
}

#[cfg(test)]
mod update_tests {
    use super::is_newer;

    #[test]
    fn version_compare() {
        assert!(is_newer("v0.9.1", "0.9.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("v0.9.0", "0.9.0"));
        assert!(!is_newer("v0.8.1", "0.9.0"));
    }
}
