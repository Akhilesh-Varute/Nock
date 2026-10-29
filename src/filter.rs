use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

static FILTER_HWND: AtomicIsize = AtomicIsize::new(0);
static FILTER_ENABLED: AtomicBool = AtomicBool::new(false);
static FILTER_OPACITY: AtomicU8 = AtomicU8::new(0);

unsafe extern "system" fn filter_wndproc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_ERASEBKGND => {
            let hdc = HDC(wp.0 as *mut _);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let brush = GetStockObject(BLACK_BRUSH);
            let _ = FillRect(hdc, &rc, HBRUSH(brush.0));
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let brush = GetStockObject(BLACK_BRUSH);
            let _ = FillRect(hdc, &rc, HBRUSH(brush.0));
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn init_filter_window() -> Option<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(None).ok()?;
        let class_name = w!("NockScreenFilter");

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(filter_wndproc),
            hInstance: hinstance.into(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: class_name,
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);

        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        let ex_style = WS_EX_LAYERED
            | WS_EX_TRANSPARENT
            | WS_EX_TOOLWINDOW
            | WS_EX_NOACTIVATE
            | WS_EX_TOPMOST;

        let hwnd = CreateWindowExW(
            ex_style,
            class_name,
            w!("Nock Screen Dimmer"),
            WS_POPUP,
            vx,
            vy,
            vw,
            vh,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
        .ok()?;

        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
        FILTER_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        Some(hwnd)
    }
}

pub fn filter_hwnd() -> Option<HWND> {
    let raw = FILTER_HWND.load(Ordering::SeqCst);
    if raw != 0 {
        Some(HWND(raw as *mut _))
    } else {
        init_filter_window()
    }
}

/// Update position and size to match all virtual screens (e.g. after display topology changes).
pub fn update_geometry() {
    let Some(hwnd) = filter_hwnd() else { return };
    unsafe {
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            vx,
            vy,
            vw,
            vh,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = UpdateWindow(hwnd);
    }
}

/// Set filter enabled state and darkness level (0-70%).
/// 0% = no darkening, 70% = maximum night darkness.
pub fn set_filter(enabled: bool, percent: u32) {
    let Some(hwnd) = filter_hwnd() else { return };
    FILTER_ENABLED.store(enabled, Ordering::SeqCst);
    let clamped_pct = percent.min(70);
    // Convert 0..70% to alpha 0..178 (max 70% opacity so screen is never pitch black)
    let alpha = if enabled {
        ((clamped_pct as f64 / 100.0) * 255.0).round() as u8
    } else {
        0
    };
    FILTER_OPACITY.store(alpha, Ordering::SeqCst);

    unsafe {
        if enabled && alpha > 0 {
            update_geometry();
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
            let _ = ShowWindow(hwnd, SW_SHOWNA);
            let _ = InvalidateRect(Some(hwnd), None, true);
            let _ = UpdateWindow(hwnd);
        } else {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[allow(dead_code)]
pub fn is_enabled() -> bool {
    FILTER_ENABLED.load(Ordering::SeqCst)
}

#[allow(dead_code)]
pub fn current_opacity() -> u8 {
    FILTER_OPACITY.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_initialization_does_not_panic() {
        let _ = filter_hwnd();
    }

    #[test]
    fn filter_toggle_and_bounds() {
        set_filter(true, 50);
        assert!(is_enabled());
        set_filter(false, 0);
        assert!(!is_enabled());
    }
}
