use std::sync::OnceLock;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PhysicalMonitor {
    pub handle: HANDLE,
    pub desc: [u16; 128],
}

type FnGetNumberOfPhysicalMonitorsFromHMONITOR =
    unsafe extern "system" fn(HMONITOR, *mut u32) -> BOOL;
type FnGetPhysicalMonitorsFromHMONITOR =
    unsafe extern "system" fn(HMONITOR, u32, *mut PhysicalMonitor) -> BOOL;
type FnDestroyPhysicalMonitors =
    unsafe extern "system" fn(u32, *const PhysicalMonitor) -> BOOL;
type FnGetMonitorBrightness =
    unsafe extern "system" fn(HANDLE, *mut u32, *mut u32, *mut u32) -> BOOL;
type FnSetMonitorBrightness =
    unsafe extern "system" fn(HANDLE, u32) -> BOOL;

pub struct Dxva2 {
    pub get_count: FnGetNumberOfPhysicalMonitorsFromHMONITOR,
    pub get_monitors: FnGetPhysicalMonitorsFromHMONITOR,
    pub destroy_monitors: FnDestroyPhysicalMonitors,
    pub get_brightness: FnGetMonitorBrightness,
    pub set_brightness: FnSetMonitorBrightness,
}

static DXVA2: OnceLock<Option<Dxva2>> = OnceLock::new();

pub fn dxva2() -> Option<&'static Dxva2> {
    DXVA2
        .get_or_init(|| unsafe {
            let lib = LoadLibraryA(s!("dxva2.dll")).ok()?;
            let get_count = GetProcAddress(lib, s!("GetNumberOfPhysicalMonitorsFromHMONITOR"))?;
            let get_monitors = GetProcAddress(lib, s!("GetPhysicalMonitorsFromHMONITOR"))?;
            let destroy_monitors = GetProcAddress(lib, s!("DestroyPhysicalMonitors"))?;
            let get_brightness = GetProcAddress(lib, s!("GetMonitorBrightness"))?;
            let set_brightness = GetProcAddress(lib, s!("SetMonitorBrightness"))?;

            Some(Dxva2 {
                get_count: std::mem::transmute(get_count),
                get_monitors: std::mem::transmute(get_monitors),
                destroy_monitors: std::mem::transmute(destroy_monitors),
                get_brightness: std::mem::transmute(get_brightness),
                set_brightness: std::mem::transmute(set_brightness),
            })
        })
        .as_ref()
}

#[derive(Clone, Debug, PartialEq)]
pub struct PhysicalDisplayInfo {
    pub name: String,
    pub current: u32,
    pub min: u32,
    pub max: u32,
}

/// Enumerate all physical DDC/CI monitors and their current brightness levels.
pub fn get_ddc_brightness_all() -> Vec<PhysicalDisplayInfo> {
    let Some(api) = dxva2() else { return Vec::new() };
    let mut results = Vec::new();

    struct Ctx<'a> {
        api: &'a Dxva2,
        results: &'a mut Vec<PhysicalDisplayInfo>,
    }

    unsafe extern "system" fn enum_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let ctx = unsafe { &mut *(lparam.0 as *mut Ctx) };

        let mut count = 0u32;
        if unsafe { (ctx.api.get_count)(hmon, &mut count) }.as_bool() && count > 0 {
            let mut monitors = vec![
                PhysicalMonitor {
                    handle: HANDLE(std::ptr::null_mut()),
                    desc: [0; 128],
                };
                count as usize
            ];
            if unsafe { (ctx.api.get_monitors)(hmon, count, monitors.as_mut_ptr()) }.as_bool() {
                for pm in &monitors {
                    let mut min = 0u32;
                    let mut cur = 0u32;
                    let mut max = 0u32;
                    if unsafe { (ctx.api.get_brightness)(pm.handle, &mut min, &mut cur, &mut max) }
                        .as_bool()
                    {
                        let name_len = pm.desc.iter().position(|&c| c == 0).unwrap_or(128);
                        let name = String::from_utf16_lossy(&pm.desc[..name_len]).trim().to_string();
                        let percent = if max > min {
                            ((cur.saturating_sub(min)) as f64 / (max - min) as f64 * 100.0).round()
                                as u32
                        } else {
                            cur
                        };
                        ctx.results.push(PhysicalDisplayInfo {
                            name: if name.is_empty() { "External Monitor".into() } else { name },
                            current: percent.min(100),
                            min,
                            max,
                        });
                    }
                }
                unsafe {
                    let _ = (ctx.api.destroy_monitors)(count, monitors.as_ptr());
                }
            }
        }
        BOOL(1)
    }

    let mut ctx = Ctx {
        api,
        results: &mut results,
    };

    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut ctx as *mut _ as isize),
        );
    }

    results
}

/// Set the brightness on all physical DDC/CI monitors matching percentage 0-100.
pub fn set_ddc_brightness_all(percent: u32) -> bool {
    let Some(api) = dxva2() else { return false };

    struct Context<'a> {
        api: &'a Dxva2,
        percent: u32,
        success: bool,
    }

    unsafe extern "system" fn enum_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let ctx = unsafe { &mut *(lparam.0 as *mut Context) };
        let mut count = 0u32;
        if unsafe { (ctx.api.get_count)(hmon, &mut count) }.as_bool() && count > 0 {
            let mut monitors = vec![
                PhysicalMonitor {
                    handle: HANDLE(std::ptr::null_mut()),
                    desc: [0; 128],
                };
                count as usize
            ];
            if unsafe { (ctx.api.get_monitors)(hmon, count, monitors.as_mut_ptr()) }.as_bool() {
                for pm in &monitors {
                    let mut min = 0u32;
                    let mut cur = 0u32;
                    let mut max = 0u32;
                    if unsafe { (ctx.api.get_brightness)(pm.handle, &mut min, &mut cur, &mut max) }
                        .as_bool()
                    {
                        let target = if max > min {
                            min + ((max - min) as f64 * (ctx.percent.min(100) as f64 / 100.0)).round()
                                as u32
                        } else {
                            ctx.percent.min(100)
                        };
                        if unsafe { (ctx.api.set_brightness)(pm.handle, target) }.as_bool() {
                            ctx.success = true;
                        }
                    }
                }
                unsafe {
                    let _ = (ctx.api.destroy_monitors)(count, monitors.as_ptr());
                }
            }
        }
        BOOL(1)
    }

    let mut ctx = Context {
        api,
        percent,
        success: false,
    };

    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut ctx as *mut _ as isize),
        );
    }

    ctx.success
}

/// Set brightness on a specific physical DDC/CI monitor by index.
pub fn set_ddc_brightness_index(target_idx: usize, percent: u32) -> bool {
    let Some(api) = dxva2() else { return false };

    struct Context<'a> {
        api: &'a Dxva2,
        target_idx: usize,
        current_idx: usize,
        percent: u32,
        success: bool,
    }

    unsafe extern "system" fn enum_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let ctx = unsafe { &mut *(lparam.0 as *mut Context) };
        let mut count = 0u32;
        if unsafe { (ctx.api.get_count)(hmon, &mut count) }.as_bool() && count > 0 {
            let mut monitors = vec![
                PhysicalMonitor {
                    handle: HANDLE(std::ptr::null_mut()),
                    desc: [0; 128],
                };
                count as usize
            ];
            if unsafe { (ctx.api.get_monitors)(hmon, count, monitors.as_mut_ptr()) }.as_bool() {
                for pm in &monitors {
                    let mut min = 0u32;
                    let mut cur = 0u32;
                    let mut max = 0u32;
                    if unsafe { (ctx.api.get_brightness)(pm.handle, &mut min, &mut cur, &mut max) }
                        .as_bool()
                    {
                        if ctx.current_idx == ctx.target_idx {
                            let target = if max > min {
                                min + ((max - min) as f64 * (ctx.percent.min(100) as f64 / 100.0)).round()
                                    as u32
                            } else {
                                ctx.percent.min(100)
                            };
                            if unsafe { (ctx.api.set_brightness)(pm.handle, target) }.as_bool() {
                                ctx.success = true;
                            }
                        }
                        ctx.current_idx += 1;
                    }
                }
                unsafe {
                    let _ = (ctx.api.destroy_monitors)(count, monitors.as_ptr());
                }
            }
        }
        BOOL(1)
    }

    let mut ctx = Context {
        api,
        target_idx,
        current_idx: 0,
        percent,
        success: false,
    };

    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut ctx as *mut _ as isize),
        );
    }

    ctx.success
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dxva2_loads_safely() {
        // May be Some on Windows with graphics drivers, or None in headless environments,
        // but must never panic or crash.
        let _ = dxva2();
    }

    #[test]
    fn ddc_query_does_not_panic() {
        let displays = get_ddc_brightness_all();
        for d in &displays {
            assert!(d.current <= 100);
            assert!(d.min <= d.max);
        }
    }
}
