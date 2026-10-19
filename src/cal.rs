//! Calendar hints: reads a read-only ICS link (Google "secret address in iCal format",
//! Outlook "published calendar") so the pill can say "Standup 5m" before a meeting.
//! Off unless `calendar_url` is set in config.ini. The link is fetched at start and
//! about once an hour, never on a tighter loop.
//!
//! All times are "local epoch" seconds: the local wall clock read as if it were UTC.
// ponytail: only DAILY/WEEKLY rules (INTERVAL, BYDAY, UNTIL, EXDATE) are expanded; COUNT,
// monthly/yearly rules and TZID conversion are not (TZID times are taken as this PC's time,
// UTC times drift an hour across a DST change). Upgrade path: a real iCal crate.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub start: i64,
    pub title: String,
    pub link: Option<String>,
}

/// Upcoming occurrences from the last fetch.
pub static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());
static BUSY: AtomicBool = AtomicBool::new(false);

const LOOKBACK: i64 = 600;
const LOOKAHEAD: i64 = 36 * 3600;

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

/// 0 = Sunday.
fn weekday(day: i64) -> i64 {
    (day + 4).rem_euclid(7)
}

/// (local epoch now, seconds the local clock is ahead of UTC).
pub fn now() -> (i64, i64) {
    let (y, mo, d, _, h, mi, s, _) = crate::sys::local_time();
    let local = days_from_civil(y as i64, mo as i64, d as i64) * 86400 + (h * 3600 + mi * 60 + s) as i64;
    let utc = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|t| t.as_secs() as i64).unwrap_or(local);
    (local, local - utc)
}

/// `20261019T100000Z` / `20261019T100000` -> local epoch; None for all-day dates.
fn parse_dt(v: &str, off: i64) -> Option<i64> {
    let v = v.trim();
    if v.len() < 15 || !v.is_char_boundary(8) || v.as_bytes()[8] != b'T' {
        return None;
    }
    let n = |a: usize, b: usize| v.get(a..b)?.parse::<i64>().ok();
    let t = days_from_civil(n(0, 4)?, n(4, 6)?, n(6, 8)?) * 86400 + n(9, 11)? * 3600 + n(11, 13)? * 60 + n(13, 15)?;
    Some(if v.ends_with('Z') { t + off } else { t })
}

fn unescape(s: &str) -> String {
    s.replace("\\n", " ").replace("\\N", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// A meeting link (Zoom, Meet, Teams, Webex) found in the text, else None.
fn meeting_link(text: &str) -> Option<String> {
    const HOSTS: [&str; 5] = ["zoom.us", "meet.google.com", "teams.microsoft.com", "teams.live.com", "webex.com"];
    let mut rest = text;
    while let Some(i) = rest.find("https://") {
        let tail = &rest[i..];
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '\\' | '<' | '>' | '"')).unwrap_or(tail.len());
        let url = &tail[..end];
        if HOSTS.iter().any(|h| url.contains(h)) {
            return Some(url.to_string());
        }
        rest = &tail[8..];
    }
    None
}

#[derive(Default)]
struct Raw {
    uid: String,
    start: Option<i64>,
    title: String,
    text: String,
    url: String,
    rrule: String,
    exdates: Vec<i64>,
    recurrence_id: Option<i64>,
    cancelled: bool,
}

/// The occurrences between a little ago and 36 hours ahead, soonest first.
pub fn parse(ics: &str, now: i64, off: i64) -> Vec<Event> {
    let mut unfolded: Vec<String> = Vec::new();
    for l in ics.lines() {
        if l.starts_with(' ') || l.starts_with('\t') {
            if let Some(last) = unfolded.last_mut() {
                last.push_str(&l[1..]);
            }
        } else {
            unfolded.push(l.to_string());
        }
    }
    let mut raws: Vec<Raw> = Vec::new();
    let mut cur: Option<Raw> = None;
    for l in &unfolded {
        if l == "BEGIN:VEVENT" {
            cur = Some(Raw::default());
        } else if l == "END:VEVENT" {
            raws.extend(cur.take());
        } else if let (Some(r), Some((head, val))) = (cur.as_mut(), l.split_once(':')) {
            let name = head.split(';').next().unwrap_or("").to_ascii_uppercase();
            let all_day = head.to_ascii_uppercase().contains("VALUE=DATE") && !head.to_ascii_uppercase().contains("DATE-TIME");
            match name.as_str() {
                "UID" => r.uid = val.to_string(),
                "DTSTART" if !all_day => r.start = parse_dt(val, off),
                "SUMMARY" => r.title = unescape(val),
                "DESCRIPTION" | "LOCATION" => {
                    r.text.push(' ');
                    r.text.push_str(val);
                }
                "URL" => r.url = val.to_string(),
                "RRULE" => r.rrule = val.to_string(),
                "EXDATE" if !all_day => r.exdates.extend(val.split(',').filter_map(|v| parse_dt(v, off))),
                "RECURRENCE-ID" if !all_day => r.recurrence_id = parse_dt(val, off),
                "STATUS" => r.cancelled = val.trim().eq_ignore_ascii_case("CANCELLED"),
                _ => {}
            }
        }
    }
    let (lo, hi) = (now - LOOKBACK, now + LOOKAHEAD);
    // a moved instance of a repeating meeting replaces the original day
    let moved: Vec<(&str, i64)> = raws.iter().filter_map(|r| r.recurrence_id.map(|t| (r.uid.as_str(), t.div_euclid(86400)))).collect();
    let mut out = Vec::new();
    for r in raws.iter().filter(|r| !r.cancelled) {
        let Some(start) = r.start else { continue };
        let link = meeting_link(&r.text).or_else(|| meeting_link(&r.url));
        let mut push = |t: i64| {
            if (lo..=hi).contains(&t) {
                out.push(Event { start: t, title: r.title.clone(), link: link.clone() });
            }
        };
        let rule: Vec<(&str, &str)> = r.rrule.split(';').filter_map(|p| p.split_once('=')).collect();
        let get = |k: &str| rule.iter().find(|(a, _)| *a == k).map(|(_, v)| *v);
        let (base, tod) = (start.div_euclid(86400), start.rem_euclid(86400));
        let freq = get("FREQ");
        if r.rrule.is_empty() || !matches!(freq, Some("DAILY" | "WEEKLY")) {
            push(start);
            continue;
        }
        let interval = get("INTERVAL").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1).max(1);
        let until = get("UNTIL").and_then(|v| parse_dt(v, off).or_else(|| parse_dt(&format!("{v}T235959"), off)));
        let byday: Vec<i64> = get("BYDAY")
            .map(|v| {
                v.split(',')
                    .filter_map(|d| match d.trim_start_matches(|c: char| c.is_ascii_digit() || c == '+' || c == '-') {
                        "SU" => Some(0),
                        "MO" => Some(1),
                        "TU" => Some(2),
                        "WE" => Some(3),
                        "TH" => Some(4),
                        "FR" => Some(5),
                        "SA" => Some(6),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for day in (lo.div_euclid(86400) - 1)..=(hi.div_euclid(86400) + 1) {
            if day < base || r.exdates.iter().any(|e| e.div_euclid(86400) == day) || moved.contains(&(r.uid.as_str(), day)) {
                continue;
            }
            let hit = if freq == Some("DAILY") {
                (day - base) % interval == 0
            } else {
                let days = if byday.is_empty() { vec![weekday(base)] } else { byday.clone() };
                (day - base).div_euclid(7) % interval == 0 && days.contains(&weekday(day))
            };
            let t = day * 86400 + tod;
            if hit && until.is_none_or(|u| t <= u) {
                push(t);
            }
        }
    }
    out.sort_by_key(|e| e.start);
    out.dedup();
    out
}

/// Fetch the calendar in the background and replace EVENTS when it arrives.
pub fn refresh(url: &str) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let url = url.trim().replacen("webcal://", "https://", 1).replace('\'', "''");
    std::thread::spawn(move || {
        let cmd = format!(
            "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; [Text.Encoding]::UTF8.GetString((Invoke-WebRequest -UseBasicParsing -Uri '{url}').RawContentStream.ToArray())"
        );
        if let Some(text) = crate::sys::powershell(&cmd) {
            let (now, off) = now();
            *EVENTS.lock().unwrap() = parse(&text, now, off);
        }
        BUSY.store(false, Ordering::SeqCst);
    });
}

/// The next meeting starting within five minutes (or begun in the last minute).
pub fn soon(events: &[Event], now: i64) -> Option<&Event> {
    events.iter().find(|e| (-60..=300).contains(&(e.start - now)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICS: &str = "BEGIN:VCALENDAR\r\n\
BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20261019T100000Z\r\nSUMMARY:Standup\r\nDESCRIPTION:Join https://meet.google.com/abc-defg-hij\\nthanks\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:b\r\nDTSTART:20261019T090000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,TU\r\nEXDATE:20261020T090000\r\nSUMMARY:Sync\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:c\r\nDTSTART:20261019T110000Z\r\nSTATUS:CANCELLED\r\nSUMMARY:Gone\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:d\r\nDTSTART;VALUE=DATE:20261019\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn parses_one_off_recurring_and_skips() {
        let now = days_from_civil(2026, 10, 19) * 86400 + 8 * 3600; // Monday 08:00
        let ev = parse(ICS, now, 0);
        let titles: Vec<_> = ev.iter().map(|e| (e.title.as_str(), (e.start - now) / 3600)).collect();
        // Mon 09:00 Sync, Mon 10:00 Standup; Tue 09:00 Sync is excluded; cancelled and all-day skipped
        assert_eq!(titles, vec![("Sync", 1), ("Standup", 2)]);
        assert_eq!(ev[1].link.as_deref(), Some("https://meet.google.com/abc-defg-hij"));
    }

    #[test]
    fn utc_converts_to_local_and_soon_window() {
        let now = days_from_civil(2026, 10, 19) * 86400 + 11 * 3600; // 11:00 local
        let ev = parse(ICS, now, 3600); // local is UTC+1: Standup 10:00Z = 11:00 local
        assert_eq!(ev.iter().find(|e| e.title == "Standup").map(|e| e.start), Some(now));
        assert!(soon(&ev, now - 120).is_some_and(|e| e.title == "Standup"));
        assert!(soon(&ev, now - 600).is_none());
    }
}
