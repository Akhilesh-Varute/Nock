//! "Now playing" via the Windows media session API (the same source as the
//! volume flyout). Works for Spotify, browsers, VLC, Apple Music, etc.
//!
//! A single background thread owns the session manager. It is fully event
//! driven: it sleeps until Windows reports a change, updates shared state and
//! posts one window message to the UI thread. No polling.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Posted to the UI window whenever `MediaState` changed.
pub const WM_MEDIA: u32 = 0x8000 + 1; // WM_APP + 1

#[derive(Default, Clone)]
pub struct MediaState {
    pub has_session: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app: String,
    pub playing: bool,
    /// Timeline position (100ns ticks) at `pos_stamp` (FILETIME ticks).
    pub pos_ticks: i64,
    pub pos_stamp: i64,
    pub dur_ticks: i64,
    pub rate: f64,
    pub art: Option<Arc<Vec<u8>>>,
    /// Bumped whenever `art` changes so the UI knows to re-decode.
    pub art_gen: u64,
}

impl MediaState {
    pub fn has_track(&self) -> bool {
        self.has_session && !(self.title.is_empty() && self.artist.is_empty())
    }

    /// (position, duration) in seconds, extrapolated to `now_ft` while playing.
    pub fn progress(&self, now_ft: i64) -> (f64, f64) {
        let dur = self.dur_ticks as f64 / 1e7;
        let mut pos = self.pos_ticks as f64 / 1e7;
        if self.playing && self.pos_stamp > 0 && now_ft > self.pos_stamp {
            let rate = if self.rate > 0.0 { self.rate } else { 1.0 };
            pos += (now_ft - self.pos_stamp) as f64 / 1e7 * rate;
        }
        if dur > 0.0 {
            pos = pos.clamp(0.0, dur);
        }
        (pos.max(0.0), dur.max(0.0))
    }
}

pub enum Cmd {
    PlayPause,
    Next,
    Prev,
    SeekSecs(f64),
}

pub struct Handle {
    pub state: Arc<Mutex<MediaState>>,
    pub tx: Sender<Cmd>,
}

pub fn start(hwnd: isize) -> Handle {
    let state = Arc::new(Mutex::new(MediaState::default()));
    let (tx, rx) = channel();
    let st = state.clone();
    std::thread::Builder::new()
        .name("media".into())
        .spawn(move || imp::run(hwnd, st, rx))
        .ok();
    Handle { state, tx }
}

// ===================================================================== real

#[cfg(feature = "media")]
mod imp {
    use super::*;
    use std::collections::HashSet;
    use windows::core::Interface;
    use windows::Foundation::TypedEventHandler;
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

    struct Ctx {
        hwnd: isize,
        st: Arc<Mutex<MediaState>>,
        mgr: Manager,
        hooked: Mutex<HashSet<usize>>,
        /// Serialises refreshes coming from different WinRT threads.
        lock: Mutex<()>,
        /// A delayed re-check of the play/pause state is already scheduled.
        verifying: AtomicBool,
    }

    pub fn run(hwnd: isize, st: Arc<Mutex<MediaState>>, rx: Receiver<Cmd>) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mgr = match Manager::RequestAsync().and_then(|op| op.join()) {
            Ok(m) => m,
            Err(_) => return,
        };
        let ctx = Arc::new(Ctx {
            hwnd,
            st,
            mgr: mgr.clone(),
            hooked: Mutex::new(HashSet::new()),
            lock: Mutex::new(()),
            verifying: AtomicBool::new(false),
        });

        hook_sessions(&ctx);
        refresh_all(&ctx);

        let c = ctx.clone();
        let _ = mgr.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            hook_sessions(&c);
            refresh_all(&c);
            Ok(())
        }));
        let c = ctx.clone();
        let _ = mgr.SessionsChanged(&TypedEventHandler::new(move |_, _| {
            hook_sessions(&c);
            Ok(())
        }));

        // Commands from the UI. Blocks (zero CPU) until the user clicks something.
        while let Ok(cmd) = rx.recv() {
            let Ok(s) = mgr.GetCurrentSession() else {
                continue;
            };
            match cmd {
                Cmd::PlayPause => {
                    let _ = s.TryTogglePlayPauseAsync().and_then(|o| o.join());
                }
                Cmd::Next => {
                    let _ = s.TrySkipNextAsync().and_then(|o| o.join());
                }
                Cmd::Prev => {
                    let _ = s.TrySkipPreviousAsync().and_then(|o| o.join());
                }
                Cmd::SeekSecs(secs) => {
                    let ticks = (secs * 1e7) as i64;
                    let _ = s
                        .TryChangePlaybackPositionAsync(ticks)
                        .and_then(|o| o.join());
                }
            }
        }
    }

    /// Subscribe to change events on every session we have not seen yet.
    fn hook_sessions(ctx: &Arc<Ctx>) {
        let Ok(list) = ctx.mgr.GetSessions() else {
            return;
        };
        for s in list {
            let key = s.as_raw() as usize;
            if !ctx.hooked.lock().unwrap().insert(key) {
                continue;
            }
            let c = ctx.clone();
            let _ = s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                refresh_all(&c);
                Ok(())
            }));
            let c = ctx.clone();
            let _ = s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                refresh_playback(&c);
                Ok(())
            }));
            let c = ctx.clone();
            let _ = s.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                refresh_playback(&c);
                Ok(())
            }));
        }
    }

    fn notify(ctx: &Ctx) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(ctx.hwnd as *mut _)),
                WM_MEDIA,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    /// Track info + artwork + playback.
    fn refresh_all(ctx: &Arc<Ctx>) {
        let _g = ctx.lock.lock().unwrap();
        let session = match ctx.mgr.GetCurrentSession() {
            Ok(s) => s,
            Err(_) => {
                let mut st = ctx.st.lock().unwrap();
                if st.has_session {
                    let gen = st.art_gen + 1;
                    *st = MediaState::default();
                    st.art_gen = gen;
                }
                drop(st);
                notify(ctx);
                return;
            }
        };

        let props = session
            .TryGetMediaPropertiesAsync()
            .and_then(|o| o.join())
            .ok();
        let (title, artist, album) = match &props {
            Some(p) => (
                p.Title().map(|s| s.to_string()).unwrap_or_default(),
                p.Artist().map(|s| s.to_string()).unwrap_or_default(),
                p.AlbumTitle().map(|s| s.to_string()).unwrap_or_default(),
            ),
            None => Default::default(),
        };
        let app = session
            .SourceAppUserModelId()
            .map(|s| friendly_app(&s.to_string()))
            .unwrap_or_default();

        // Always re-read the thumbnail: Apple Music and browsers often publish the
        // new track's title first and its artwork a moment later (browsers even
        // start with their own app icon), and no further "track changed" event
        // follows. Comparing bytes avoids needless redraws.
        let read_art = props
            .as_ref()
            .and_then(|p| p.Thumbnail().ok())
            .and_then(|t| read_thumb(&t))
            .map(Arc::new);

        let changed_track;
        {
            let mut st = ctx.st.lock().unwrap();
            let was_same =
                st.has_session && st.title == title && st.artist == artist && st.album == album;
            let album_changed = st.album != album;
            let old = st.art.clone();
            st.has_session = true;
            st.title = title.clone();
            st.artist = artist;
            st.album = album;
            st.app = app;
            let next = if was_same {
                // keep what we have unless a different image arrived
                read_art.or(old.clone())
            } else {
                // New track. If the image is byte-identical to the previous track's
                // but the album changed, it is the stale old cover: show none until
                // the real one arrives (the delayed re-reads below pick it up).
                let stale = matches!((&old, &read_art), (Some(o), Some(n)) if **o == **n)
                    && album_changed;
                if stale {
                    None
                } else {
                    read_art
                }
            };
            let differs = match (&old, &next) {
                (Some(o), Some(n)) => **o != **n,
                (None, None) => false,
                _ => true,
            };
            st.art = next;
            if differs {
                st.art_gen += 1;
            }
            changed_track = !was_same;
        }
        if changed_track {
            // Artwork often lands a little after the title; look again a few times.
            let c = ctx.clone();
            std::thread::spawn(move || {
                for ms in [700u64, 1500, 2500, 4000] {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    refresh_art(&c, &title);
                }
            });
        }
        fill_playback(ctx, &session);
        notify(ctx);
        drop(_g);
        verify_later(ctx);
    }

    /// Re-read just the artwork for the track called `expect`.
    fn refresh_art(ctx: &Ctx, expect: &str) {
        let _g = ctx.lock.lock().unwrap();
        let Ok(session) = ctx.mgr.GetCurrentSession() else {
            return;
        };
        let Ok(props) = session.TryGetMediaPropertiesAsync().and_then(|o| o.join()) else {
            return;
        };
        if props.Title().map(|s| s.to_string()).unwrap_or_default() != expect {
            return; // the track moved on
        }
        let Some(new) = props
            .Thumbnail()
            .ok()
            .and_then(|t| read_thumb(&t))
        else {
            return;
        };
        let mut st = ctx.st.lock().unwrap();
        if st.art.as_ref().map(|a| **a == new).unwrap_or(false) {
            return;
        }
        st.art = Some(Arc::new(new));
        st.art_gen += 1;
        drop(st);
        notify(ctx);
    }

    fn refresh_playback(ctx: &Arc<Ctx>) {
        {
            let _g = ctx.lock.lock().unwrap();
            if let Ok(session) = ctx.mgr.GetCurrentSession() {
                fill_playback(ctx, &session);
                notify(ctx);
            }
        }
        verify_later(ctx);
    }

    /// Browsers (YouTube especially) can fire the "playback changed" event a moment
    /// before the new play/pause state is readable, which left the notch showing the
    /// opposite of reality until the next event. So look again shortly afterwards.
    fn verify_later(ctx: &Arc<Ctx>) {
        if ctx.verifying.swap(true, Ordering::AcqRel) {
            return; // a re-check is already pending
        }
        let c = ctx.clone();
        std::thread::spawn(move || {
            let mut waited = 0u64;
            for at in [250u64, 700, 1500] {
                std::thread::sleep(std::time::Duration::from_millis(at - waited));
                waited = at;
                let _g = c.lock.lock().unwrap();
                if let Ok(session) = c.mgr.GetCurrentSession() {
                    fill_playback(&c, &session);
                    notify(&c);
                }
            }
            c.verifying.store(false, Ordering::Release);
        });
    }

    fn fill_playback(ctx: &Ctx, s: &Session) {
        let mut playing = false;
        let mut rate = 1.0f64;
        if let Ok(info) = s.GetPlaybackInfo() {
            playing = info
                .PlaybackStatus()
                .map(|x| x == Status::Playing)
                .unwrap_or(false);
            rate = info.PlaybackRate().and_then(|r| r.Value()).unwrap_or(1.0);
        }
        let (mut pos, mut dur, mut stamp) = (0i64, 0i64, 0i64);
        if let Ok(tl) = s.GetTimelineProperties() {
            let start = tl.StartTime().map(|t| t.Duration).unwrap_or(0);
            let end = tl.EndTime().map(|t| t.Duration).unwrap_or(0);
            pos = tl.Position().map(|t| t.Duration).unwrap_or(0) - start;
            dur = (end - start).max(0);
            stamp = tl.LastUpdatedTime().map(|t| t.UniversalTime).unwrap_or(0);
        }
        let now = filetime_now();
        // Some apps report a bogus "last updated"; fall back to "now".
        if stamp <= 0 || stamp > now {
            stamp = now;
        }
        let mut st = ctx.st.lock().unwrap();
        st.playing = playing;
        st.rate = rate;
        st.pos_ticks = pos;
        st.pos_stamp = stamp;
        st.dur_ticks = dur;
    }

    fn read_thumb(r: &IRandomAccessStreamReference) -> Option<Vec<u8>> {
        let stream = r.OpenReadAsync().ok()?.join().ok()?;
        let size = stream.Size().ok()? as u32;
        if size == 0 || size > 8_000_000 {
            return None;
        }
        let reader = DataReader::CreateDataReader(&stream).ok()?;
        reader.LoadAsync(size).ok()?.join().ok()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf).ok()?;
        Some(buf)
    }

    fn filetime_now() -> i64 {
        crate::sys::filetime_now()
    }

    /// "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify" -> "Spotify"; "chrome.exe" -> "Chrome".
    fn friendly_app(id: &str) -> String {
        let base = id.rsplit('!').next().unwrap_or(id);
        let base = base.rsplit('\\').next().unwrap_or(base);
        let base = base.trim_end_matches(".exe");
        let mut cs = base.chars();
        match cs.next() {
            Some(c) => c.to_uppercase().collect::<String>() + cs.as_str(),
            None => String::new(),
        }
    }
}

// ===================================================================== stub

#[cfg(not(feature = "media"))]
mod imp {
    use super::*;
    pub fn run(_hwnd: isize, _st: Arc<Mutex<MediaState>>, rx: Receiver<Cmd>) {
        while rx.recv().is_ok() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(playing: bool) -> MediaState {
        MediaState {
            has_session: true,
            title: "t".into(),
            playing,
            pos_ticks: 30 * 10_000_000,
            pos_stamp: 1_000_000_000,
            dur_ticks: 200 * 10_000_000,
            rate: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn extrapolates_while_playing() {
        let (pos, dur) = st(true).progress(1_000_000_000 + 5 * 10_000_000);
        assert!((pos - 35.0).abs() < 1e-6);
        assert!((dur - 200.0).abs() < 1e-6);
    }

    #[test]
    fn frozen_while_paused() {
        let (pos, _) = st(false).progress(1_000_000_000 + 50 * 10_000_000);
        assert!((pos - 30.0).abs() < 1e-6);
    }

    #[test]
    fn clamps_to_duration() {
        let (pos, dur) = st(true).progress(1_000_000_000 + 9999 * 10_000_000);
        assert_eq!(pos, dur);
    }

    #[test]
    fn honours_playback_rate() {
        let mut s = st(true);
        s.rate = 2.0;
        let (pos, _) = s.progress(1_000_000_000 + 5 * 10_000_000);
        assert!((pos - 40.0).abs() < 1e-6);
    }

    #[test]
    fn track_detection() {
        assert!(st(true).has_track());
        assert!(!MediaState::default().has_track());
    }
}
