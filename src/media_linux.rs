//! "Now playing" on Linux through MPRIS (D-Bus): Spotify, browsers, VLC, mpv and
//! most other players expose `org.mpris.MediaPlayer2.*` on the session bus.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app: String,
    pub playing: bool,
    pub can_seek: bool,
    pub can_prev: bool,
    pub can_next: bool,
    /// position when polled, and length, in seconds (0 = unknown)
    pub pos_s: f64,
    pub dur_s: f64,
    pub art_url: String,
    pub bus_name: String,
    pub track_id: String,
}

pub struct Mpris {
    conn: Option<Connection>,
}

fn s(v: &OwnedValue) -> Option<String> {
    String::try_from(v.try_clone().ok()?).ok()
}

fn num(v: &OwnedValue) -> Option<i64> {
    let v = v.try_clone().ok()?;
    i64::try_from(v.try_clone().ok()?).ok().or_else(|| u64::try_from(v).ok().map(|x| x as i64))
}

impl Mpris {
    pub fn new() -> Mpris {
        Mpris { conn: Connection::session().ok() }
    }

    fn player<'a>(&'a self, bus_name: &'a str) -> Option<Proxy<'a>> {
        Proxy::new(self.conn.as_ref()?, bus_name, PATH, PLAYER).ok()
    }

    fn read(&self, name: &str) -> Option<Track> {
        let p = self.player(name)?;
        let status: String = p.get_property("PlaybackStatus").ok()?;
        let md: HashMap<String, OwnedValue> = p.get_property("Metadata").ok()?;
        let artist = md
            .get("xesam:artist")
            .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
            .map(|a| a.join(", "))
            .unwrap_or_default();
        let app = Proxy::new(self.conn.as_ref()?, name, PATH, "org.mpris.MediaPlayer2")
            .ok()
            .and_then(|r| r.get_property::<String>("Identity").ok())
            .unwrap_or_else(|| name.trim_start_matches(PREFIX).to_string());
        Some(Track {
            title: md.get("xesam:title").and_then(s).unwrap_or_default(),
            artist,
            album: md.get("xesam:album").and_then(s).unwrap_or_default(),
            app,
            playing: status == "Playing",
            can_seek: p.get_property("CanSeek").unwrap_or(false),
            can_prev: p.get_property("CanGoPrevious").unwrap_or(false),
            can_next: p.get_property("CanGoNext").unwrap_or(false),
            pos_s: p.get_property::<i64>("Position").unwrap_or(0) as f64 / 1e6,
            dur_s: md.get("mpris:length").and_then(num).unwrap_or(0) as f64 / 1e6,
            art_url: md.get("mpris:artUrl").and_then(s).unwrap_or_default(),
            bus_name: name.to_string(),
            track_id: md
                .get("mpris:trackid")
                .and_then(|v| OwnedObjectPath::try_from(v.try_clone().ok()?).ok())
                .map(|p| p.to_string())
                .unwrap_or_default(),
        })
    }

    /// The player to show: one that is playing, else the first paused one with a track.
    pub fn poll(&mut self) -> Option<Track> {
        if self.conn.is_none() {
            self.conn = Connection::session().ok();
        }
        let dbus = Proxy::new(self.conn.as_ref()?, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus").ok()?;
        let names: Vec<String> = dbus.call("ListNames", &()).ok()?;
        let tracks: Vec<Track> = names
            .iter()
            .filter(|n| n.starts_with(PREFIX))
            .filter_map(|n| self.read(n))
            .filter(|t| !(t.title.is_empty() && t.artist.is_empty()))
            .collect();
        tracks.iter().find(|t| t.playing).or(tracks.first()).cloned()
    }

    fn call(&self, t: &Track, method: &str) {
        if let Some(p) = self.player(&t.bus_name) {
            let _ = p.call_method(method, &());
        }
    }

    pub fn play_pause(&self, t: &Track) {
        self.call(t, "PlayPause");
    }

    pub fn next(&self, t: &Track) {
        self.call(t, "Next");
    }

    pub fn prev(&self, t: &Track) {
        self.call(t, "Previous");
    }

    pub fn seek_to(&self, t: &Track, secs: f64) {
        if let (Some(p), Ok(id)) = (self.player(&t.bus_name), ObjectPath::try_from(t.track_id.as_str())) {
            let _ = p.call_method("SetPosition", &(id, (secs * 1e6) as i64));
        }
    }
}

/// Fetch cover art: a local file or an http(s) URL (through curl, which every distro has).
pub fn fetch_art(url: &str) -> Option<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path.replace("%20", " ")).ok();
    }
    let out = std::process::Command::new("curl")
        .args(["-sL", "--max-time", "6", "--max-filesize", "8000000", url])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Fake {
        fn play_pause(&self) {}
        #[zbus(property)]
        fn playback_status(&self) -> String {
            "Playing".into()
        }
        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedValue> {
            let mut m = HashMap::new();
            m.insert("xesam:title".to_string(), OwnedValue::try_from(zbus::zvariant::Value::from("Song")).unwrap());
            m.insert(
                "xesam:artist".to_string(),
                OwnedValue::try_from(zbus::zvariant::Value::from(vec!["A".to_string(), "B".to_string()])).unwrap(),
            );
            m.insert("mpris:length".to_string(), OwnedValue::try_from(zbus::zvariant::Value::from(200_000_000i64)).unwrap());
            m
        }
        #[zbus(property)]
        fn position(&self) -> i64 {
            30_000_000
        }
        #[zbus(property)]
        fn can_seek(&self) -> bool {
            true
        }
    }

    /// Needs a D-Bus session bus: serve a fake player and read it back.
    #[test]
    #[ignore]
    fn reads_a_player_over_dbus() {
        let _srv = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name("org.mpris.MediaPlayer2.nocktest")
            .unwrap()
            .serve_at(PATH, Fake)
            .unwrap()
            .build()
            .unwrap();
        let mut m = Mpris::new();
        let t = m.poll().expect("the fake player");
        assert_eq!((t.title.as_str(), t.artist.as_str()), ("Song", "A, B"));
        assert!(t.playing && t.can_seek && !t.can_next);
        assert_eq!((t.pos_s, t.dur_s), (30.0, 200.0));
        assert_eq!(t.app, "nocktest");
        m.play_pause(&t); // reaches the fake without error
    }
}
