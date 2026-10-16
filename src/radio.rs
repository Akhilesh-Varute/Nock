//! Real Wi-Fi / Bluetooth on-off switches through the Windows radio API.
//! A worker thread owns the work, so the UI never blocks. If Windows exposes
//! no such radio (or access is denied) the state stays `None` and the UI falls
//! back to opening the Settings page.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Posted to the UI window whenever the radio state changed.
pub const WM_RADIO: u32 = 0x8000 + 4; // WM_APP + 4
/// Posted when a Bluetooth connect/disconnect attempt finished (wparam = 1 on success).
pub const WM_BTDONE: u32 = 0x8008;

#[derive(Default, Clone, Copy)]
pub struct Radios {
    pub wifi: Option<bool>,
    pub bluetooth: Option<bool>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Wifi,
    Bluetooth,
}

pub enum Req {
    Refresh,
    /// Reload the paired Bluetooth device list (slow; only when the dropdown opens).
    Devices,
    /// Connect (true) or disconnect (false) a paired device by Bluetooth address.
    BtConnect(u64, bool),
    Toggle(Kind),
}

#[derive(Clone)]
pub struct BtDevice {
    pub name: String,
    pub connected: bool,
    pub address: u64,
}

pub type Devices = Arc<Mutex<Vec<BtDevice>>>;

pub struct Handle {
    pub devices: Devices,
    pub state: Arc<Mutex<Radios>>,
    pub tx: Sender<Req>,
}

pub fn start(hwnd: isize) -> Handle {
    let state = Arc::new(Mutex::new(Radios::default()));
    let (tx, rx) = channel();
    let st = state.clone();
    let devices: Devices = Arc::new(Mutex::new(Vec::new()));
    let dv = devices.clone();
    std::thread::Builder::new()
        .name("radio".into())
        .spawn(move || imp::run(hwnd, st, dv, rx))
        .ok();
    Handle { devices, state, tx }
}

#[cfg(feature = "media")]
mod imp {
    use super::*;
    use windows::Devices::Radios::{Radio, RadioKind, RadioState};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

    pub fn run(hwnd: isize, st: Arc<Mutex<Radios>>, dv: Devices, rx: Receiver<Req>) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        // Ask Windows for permission to control radios (desktop apps get it).
        let _ = Radio::RequestAccessAsync().and_then(|o| o.join());
        refresh(hwnd, &st);
        while let Ok(req) = rx.recv() {
            match req {
                Req::Toggle(kind) => toggle(kind),
                Req::Devices => load_devices(&dv),
                Req::BtConnect(addr, on) => {
                    bt_set(addr, on);
                    // wait (up to ~6 s) until the device really changed state
                    let mut ok = false;
                    for _ in 0..8 {
                        std::thread::sleep(std::time::Duration::from_millis(750));
                        load_devices(&dv);
                        ok = dv
                            .lock()
                            .unwrap()
                            .iter()
                            .any(|d| d.address == addr && d.connected == on);
                        if ok {
                            break;
                        }
                    }
                    unsafe {
                        let _ = PostMessageW(
                            Some(HWND(hwnd as *mut _)),
                            WM_BTDONE,
                            WPARAM(ok as usize),
                            LPARAM(0),
                        );
                    }
                }
                Req::Refresh => {}
            }
            refresh(hwnd, &st);
        }
    }

    fn all() -> Vec<Radio> {
        Radio::GetRadiosAsync()
            .and_then(|o| o.join())
            .map(|v| v.into_iter().collect())
            .unwrap_or_default()
    }

    fn matches(r: &Radio, kind: Kind) -> bool {
        match r.Kind() {
            Ok(RadioKind::WiFi) => kind == Kind::Wifi,
            Ok(RadioKind::Bluetooth) => kind == Kind::Bluetooth,
            _ => false,
        }
    }

    fn is_on(r: &Radio) -> bool {
        r.State().map(|s| s == RadioState::On).unwrap_or(false)
    }

    fn refresh(hwnd: isize, st: &Arc<Mutex<Radios>>) {
        let list = all();
        let mut out = Radios::default();
        for r in &list {
            let on = is_on(r);
            if matches(r, Kind::Wifi) {
                out.wifi = Some(out.wifi.unwrap_or(false) || on);
            } else if matches(r, Kind::Bluetooth) {
                out.bluetooth = Some(out.bluetooth.unwrap_or(false) || on);
            }
        }
        *st.lock().unwrap() = out;
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd as *mut _)),
                WM_RADIO,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    fn load_devices(out: &Devices) {
        use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice};
        use windows::Devices::Enumeration::DeviceInformation;
        let mut v: Vec<BtDevice> = Vec::new();
        if let Ok(sel) = BluetoothDevice::GetDeviceSelectorFromPairingState(true) {
            if let Ok(list) =
                DeviceInformation::FindAllAsyncAqsFilter(&sel).and_then(|o| o.join())
            {
                for info in list {
                    let name = info.Name().map(|n| n.to_string()).unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    let dev = info
                        .Id()
                        .and_then(|id| BluetoothDevice::FromIdAsync(&id))
                        .and_then(|o| o.join())
                        .ok();
                    let (connected, address) = match &dev {
                        Some(d) => (
                            d.ConnectionStatus()
                                .map(|s| s == BluetoothConnectionStatus::Connected)
                                .unwrap_or(false),
                            d.BluetoothAddress().unwrap_or(0),
                        ),
                        None => (false, 0),
                    };
                    v.push(BtDevice {
                        name,
                        connected,
                        address,
                    });
                }
            }
        }
        v.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));
        v.truncate(6);
        *out.lock().unwrap() = v;
    }

    /// Connect or disconnect by enabling/disabling the device's profiles (audio,
    /// hands-free, headset, keyboard/mouse). The pairing itself stays.
    fn bt_set(address: u64, connect: bool) {
        use windows::core::GUID;
        use windows::Win32::Devices::Bluetooth::*;
        unsafe {
            let mut info = BLUETOOTH_DEVICE_INFO {
                dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
                ..Default::default()
            };
            info.Address.Anonymous.ullLong = address;
            if BluetoothGetDeviceInfo(None, &mut info) != 0 {
                return;
            }
            let flag = if connect { 1u32 } else { 0u32 };
            for short in [0x110Bu32, 0x111E, 0x1108, 0x110A, 0x1124] {
                let guid = GUID::from_u128(
                    ((short as u128) << 96) | 0x0000_1000_8000_0080_5f9b_34fb_u128,
                );
                BluetoothSetServiceState(None, &info, &guid, flag);
            }
        }
    }

    fn toggle(kind: Kind) {
        let list: Vec<Radio> = all().into_iter().filter(|r| matches(r, kind)).collect();
        // If anything is on, switch everything of that kind off; otherwise on.
        let any_on = list.iter().any(is_on);
        let target = if any_on { RadioState::Off } else { RadioState::On };
        for r in &list {
            let _ = r.SetStateAsync(target).and_then(|o| o.join());
        }
    }
}

#[cfg(not(feature = "media"))]
mod imp {
    use super::*;
    pub fn run(_hwnd: isize, _st: Arc<Mutex<Radios>>, _dv: Devices, rx: Receiver<Req>) {
        while rx.recv().is_ok() {}
    }
}
