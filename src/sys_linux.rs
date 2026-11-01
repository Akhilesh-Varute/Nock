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
