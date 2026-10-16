#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bar;
mod buddy;
mod cast;
mod game;
mod pixdata;
mod scene;
mod gfx;
mod pop;
mod media;
mod radio;
mod store;
mod sys;
mod ui;

use std::sync::atomic::{AtomicPtr, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, RemoveClipboardFormatListener,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, TerminateProcess};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use app::App;

const WM_TRAY: u32 = WM_APP + 3;
const WM_MOUSELEAVE: u32 = 0x02A3;

static APP: AtomicPtr<App> = AtomicPtr::new(std::ptr::null_mut());

/// The app lives for the whole process and is only touched from the UI thread.
fn app() -> Option<&'static mut App> {
    unsafe { APP.load(Ordering::Relaxed).as_mut() }
}

fn lo(lp: LPARAM) -> i32 {
    (lp.0 & 0xffff) as i16 as i32
}
fn hi(lp: LPARAM) -> i32 {
    ((lp.0 >> 16) & 0xffff) as i16 as i32
}

fn primary_scale() -> f32 {
    unsafe {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let (mut dx, mut dy) = (0u32, 0u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() && dx > 0 {
            dx as f32 / 96.0
        } else {
            1.0
        }
    }
}

fn main() -> Result<()> {
    // Release builds abort on panic, which would make the notch silently vanish.
    // Leave a note in %APPDATA%\\Notch\\crash.log so the cause can be found.
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::write(store::data_dir().join("crash.log"), format!("{info}\n"));
    }));
    // Programs we launch inherit our working directory. Move it somewhere
    // harmless so Snipping Tool & co. never keep the project folder locked.
    let _ = std::env::set_current_dir(store::data_dir());
    unsafe {
        // one instance only
        let _single = CreateMutexW(None, true, w!("Local\\NockSingleInstance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(());
        }

        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = w!("NockWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!("Nock"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(hinst),
            None,
        )?;

        let boxed = Box::new(App::new(hwnd, primary_scale())?);
        APP.store(Box::into_raw(boxed), Ordering::SeqCst);

        // top bar window (same class; messages are routed by handle in wndproc)
        let bar_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!("NockBar"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(hinst),
            None,
        )?;
        let pop_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!("NockPopup"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(hinst),
            None,
        )?;
        let buddy_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!("NockBuddy"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(hinst),
            None,
        )?;
        let stage_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!("NockStage"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(hinst),
            None,
        )?;
        if let Some(a) = app() {
            a.buddy = buddy::Buddy::new(buddy_hwnd, a.scale);
            a.stage = scene::Stage::new(stage_hwnd, a.scale);
            a.plan_scene();
            a.sync_stats();
            a.sync_water();
            let mon_w = a.mon.right - a.mon.left;
            a.bar = bar::Bar::new(bar_hwnd, mon_w, a.scale);
            a.pop = pop::Pop::new(pop_hwnd, a.scale);
        }

        let _ = AddClipboardFormatListener(hwnd);
        let _ = RegisterHotKey(
            Some(hwnd),
            1,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            'N' as u32,
        );
        let _ = RegisterHotKey(
            Some(hwnd),
            2,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            'H' as u32,
        );
        let _ = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        add_tray(hwnd);

        if let Some(a) = app() {
            sys::watch_foreground_location(a.hwnd);
            a.on_foreground();
            a.update_reserve();
            a.sync_bar();
            a.load_brightness_once();
            a.on_media(); // pick up anything the media thread already found
            a.render();
            if !a.hidden {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            a.arm_minute_timer();
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        #[allow(unreachable_code)]
        quit_now(hwnd)
    }
}

/// Remove everything we registered and terminate the process immediately.
fn quit_now(hwnd: HWND) -> ! {
    unsafe {
        remove_tray(hwnd);
        let _ = RemoveClipboardFormatListener(hwnd);
        let _ = UnregisterHotKey(Some(hwnd), 1);
        let _ = UnregisterHotKey(Some(hwnd), 2);
        let _ = TerminateProcess(GetCurrentProcess(), 0);
    }
    std::process::exit(0)
}

unsafe extern "system" fn win_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if let Some(a) = app() {
        sys::watch_foreground_location(a.hwnd);
        a.on_foreground();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some(a) = app() else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    if a.is_bar(hwnd) {
        return a.bar_msg(hwnd, msg, wp, lp);
    }
    if a.is_pop(hwnd) {
        return a.pop_msg(hwnd, msg, wp, lp);
    }
    if a.is_buddy(hwnd) || a.is_stage(hwnd) {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    match msg {
        WM_TIMER => a.on_timer(wp.0),
        WM_MOUSEMOVE => a.on_mouse_move(lo(lp), hi(lp)),
        WM_MOUSELEAVE => a.on_mouse_leave(),
        WM_LBUTTONDOWN => a.on_lbutton_down(lo(lp), hi(lp)),
        WM_LBUTTONUP => a.on_lbutton_up(),
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xffff) as u16 as i16 as i32;
            let mut p = POINT {
                x: lo(lp),
                y: hi(lp),
            };
            let _ = ScreenToClient(hwnd, &mut p);
            a.on_wheel(delta, p.y);
        }
        WM_CHAR => a.on_char(wp.0 as u16),
        WM_KILLFOCUS => a.on_kill_focus(),
        WM_MOUSEACTIVATE => {
            let r = if a.is_entering() {
                MA_ACTIVATE
            } else {
                MA_NOACTIVATE
            };
            return LRESULT(r as isize);
        }
        WM_CLIPBOARDUPDATE => a.on_clipboard(),
        WM_SETTINGCHANGE => a.on_theme_change(),
        WM_HOTKEY => {
            if wp.0 == 2 {
                a.toggle_tucked()
            } else {
                a.toggle_pinned_open()
            }
        }
        WM_DPICHANGED => a.on_display_change((wp.0 & 0xffff) as f32 / 96.0),
        WM_DISPLAYCHANGE => a.on_display_change(primary_scale()),
        WM_TRAY => on_tray(hwnd, a, lp),
        WM_DESTROY => {
            sys::reserve_top(hwnd, &RECT::default(), 0);
            PostQuitMessage(0)
        }
        m if m == media::WM_MEDIA => a.on_media(),
        m if m == radio::WM_RADIO => a.on_radio(),
        m if m == radio::WM_BTDONE => a.on_bt_done(wp.0 != 0),
        m if m == sys::WM_POP => a.on_pop_data(),
        m if m == sys::WM_EDGE => a.on_edge(),
        m if m == sys::WM_FSCHK => a.on_fullscreen_check(),
        m if m == sys::WM_BRIGHT => a.on_brightness(wp.0 != 0, lp.0 as u32),
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    LRESULT(0)
}

// --------------------------------------------------------------------- tray

/// The Notch icon, built from the PNG compiled into the exe (no resource compiler
/// needed). Falls back to the stock application icon if that ever fails.
fn app_icon() -> HICON {
    static PNG: &[u8] = include_bytes!("../assets/icon.png");
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        CreateIconFromResourceEx(PNG, true, 0x0003_0000, size, size, LR_DEFAULTCOLOR)
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default()
    }
}

fn add_tray(hwnd: HWND) {
    unsafe {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: app_icon(),
            ..Default::default()
        };
        let tip: Vec<u16> = "Nock".encode_utf16().collect();
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn remove_tray(hwnd: HWND) {
    unsafe {
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn on_tray(hwnd: HWND, a: &mut App, lp: LPARAM) {
    let ev = (lp.0 & 0xffff) as u32;
    if ev != WM_RBUTTONUP && ev != WM_CONTEXTMENU {
        if ev == WM_LBUTTONUP {
            a.toggle_pinned_open();
        }
        return;
    }
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let auto = if sys::autostart_enabled() {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            5,
            if a.tucked { w!("Show notch  (Ctrl+Alt+H)") } else { w!("Hide notch  (Ctrl+Alt+H)") },
        );
        let tb = if a.cfg.top_bar { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | tb, 7, w!("Top bar"));
        let st = if a.cfg.stats { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | st, 10, w!("CPU and memory on the bar"));
        let res = if a.cfg.reserve_space { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | res, 6, w!("Reserve screen space (when top bar is off)"));
        if let Ok(sub) = CreatePopupMenu() {
            let cur = a.cfg.buddy;
            let mark = |id: u8| if cur == id { MF_CHECKED } else { MF_UNCHECKED };
            let _ = AppendMenuW(sub, MF_STRING | mark(0), 20, w!("Off"));
            let _ = AppendMenuW(sub, MF_STRING | mark(1), 21, w!("Blob"));
            let _ = AppendMenuW(sub, MF_STRING | mark(2), 22, w!("Kitty"));
            let _ = AppendMenuW(sub, MF_STRING | mark(3), 23, w!("Bunny"));
            let _ = AppendMenuW(sub, MF_STRING | mark(4), 24, w!("Hero"));
            let _ = AppendMenuW(sub, MF_STRING | mark(5), 25, w!("Robot"));
            let _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(sub, MF_STRING | mark(6), 26, w!("Ronin (pixel, talks)"));
            let _ = AppendMenuW(sub, MF_STRING | mark(7), 27, w!("Courier (pixel, talks)"));
            let _ = AppendMenuW(sub, MF_STRING | mark(8), 28, w!("Necromancer (pixel, talks)"));
            let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("Buddy"));
        }
        if let Ok(sub) = CreatePopupMenu() {
            let on = if a.cfg.scenes { MF_CHECKED } else { MF_UNCHECKED };
            let _ = AppendMenuW(sub, MF_STRING | on, 30, w!("Play scenes now and then"));
            let _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(sub, MF_STRING, 31, w!("Play: Showdown"));
            let _ = AppendMenuW(sub, MF_STRING, 32, w!("Play: Heist"));
            let _ = AppendMenuW(sub, MF_STRING, 33, w!("Play: Team-up"));
            let _ = AppendMenuW(sub, MF_STRING, 34, w!("Play: Duel at dusk"));
            let _ = AppendMenuW(sub, MF_STRING, 35, w!("Play: Rise"));
            let _ = AppendMenuW(sub, MF_STRING, 36, w!("Play: Alley fight"));
            let _ = AppendMenuW(sub, MF_STRING, 37, w!("Play: Press conference"));
            let _ = AppendMenuW(sub, MF_STRING, 38, w!("Play: Harry vs Voldemort"));
            let _ = AppendMenuW(sub, MF_STRING, 39, w!("Play: Owl post"));
            let _ = AppendMenuW(sub, MF_STRING, 40, w!("Play: Harry and friends vs Voldemort"));
            let _ = AppendMenuW(sub, MF_STRING, 41, w!("Play: Quidditch"));
            let _ = AppendMenuW(sub, MF_STRING, 42, w!("Play: Dementor"));
            let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("Scenes"));
        }
        if let Ok(sub) = CreatePopupMenu() {
            let cur = a.cfg.clock_style;
            for (i, name) in [w!("Digital"), w!("Analog"), w!("Binary"), w!("Hours until Friday")].into_iter().enumerate() {
                let m = if cur as usize == i { MF_CHECKED } else { MF_UNCHECKED };
                let _ = AppendMenuW(sub, MF_STRING | m, 60 + i, name);
            }
            let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("Clock style"));
        }
        if let Ok(sub) = CreatePopupMenu() {
            let cur = a.cfg.water_min;
            for (i, (mins, name)) in [(0u16, w!("Off")), (30, w!("Every 30 minutes")), (45, w!("Every 45 minutes")), (60, w!("Every hour")), (90, w!("Every 90 minutes"))].into_iter().enumerate() {
                let m = if cur == mins { MF_CHECKED } else { MF_UNCHECKED };
                let _ = AppendMenuW(sub, MF_STRING | m, 70 + i, name);
            }
            let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("Water reminder"));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING | auto, 1, w!("Start with Windows"));
        let _ = AppendMenuW(menu, MF_STRING, 2, w!("Edit pinned apps"));
        let _ = AppendMenuW(menu, MF_STRING, 3, w!("Reload pinned apps"));
        let _ = AppendMenuW(menu, MF_STRING, 4, w!("Open settings folder"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, 9, w!("Quit"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        match cmd.0 {
            1 => sys::set_autostart(!sys::autostart_enabled()),
            2 => sys::open(
                "notepad.exe",
                &format!("\"{}\"", store::apps_path().display()),
            ),
            3 => a.reload_apps(),
            5 => a.toggle_tucked(),
            6 => a.toggle_reserve(),
            7 => a.toggle_top_bar(),
            10 => a.toggle_stats(),
            4 => sys::open(&store::data_dir().to_string_lossy(), ""),
            20..=29 => a.set_buddy((cmd.0 - 20) as u8),
            30 => a.toggle_scenes(),
            60..=63 => a.set_clock_style((cmd.0 - 60) as u8),
            70..=74 => a.set_water_min([0u16, 30, 45, 60, 90][(cmd.0 - 70) as usize]),
            31..=42 => a.play_scene((cmd.0 - 31) as usize),
            9 => {
                // Clean up, then end the process outright so no helper thread,
                // hook or COM apartment can keep notch.exe alive.
                let _ = DestroyWindow(hwnd);
                quit_now(hwnd);
            }
            _ => {}
        }
    }
}
