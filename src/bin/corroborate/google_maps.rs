//! Google's map tiles as a source of the Map nodes (Map Tiles API, 2D tiles): satellite
//! photos, with or without road and place names, or the road map. Online, so it is a choice made
//! each time: the maps start offline at every start of the program, and nothing is sent to
//! Google until it is switched on. While on, Google sees which part of the world is looked at.
//!
//! - The API key is the user's own, kept in `settings\google_maps_key.txt` next to the program,
//!   never in a project file. It goes to curl on its standard input, not on the command line.
//! - A session (`createSession`, valid about two weeks) sets what the tiles look like; every tile
//!   request names it and the key.
//! - Tiles stay in memory only: Google's terms do not allow storing them. The terms also allow
//!   only showing them on a map: no image analysis, no extracting data, no offline use.
//! - Once the view has settled, the viewport request gives the copyright text that has to be on
//!   the map and how far this area can be zoomed in (the map stops there).

use super::*;
use corroborate::plugin::{parse_json, Json};
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;
use walkers::sources::{Attribution, TileSource};
use walkers::{HttpOptions, HttpTiles, TileId};

const ENDPOINT: &str = "https://tile.googleapis.com";
/// How long a view has to stay put before its copyright text is asked for.
const SETTLE: Duration = Duration::from_millis(600);

/// What the Map nodes show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapSource {
    Offline,
    Satellite,
    SatelliteNames,
    Road,
}

impl MapSource {
    pub const ALL: [MapSource; 4] = [MapSource::Offline, MapSource::Satellite, MapSource::SatelliteNames, MapSource::Road];

    pub fn label(self) -> &'static str {
        match self {
            MapSource::Offline => "Offline maps",
            MapSource::Satellite => "Google satellite",
            MapSource::SatelliteNames => "Google satellite + names",
            MapSource::Road => "Google road map",
        }
    }

    /// The `createSession` request for it.
    fn session_body(self) -> &'static str {
        match self {
            MapSource::Road => r#"{"mapType":"roadmap","language":"en-US","region":"US"}"#,
            MapSource::SatelliteNames => r#"{"mapType":"satellite","language":"en-US","region":"US","layerTypes":["layerRoadmap"]}"#,
            _ => r#"{"mapType":"satellite","language":"en-US","region":"US"}"#,
        }
    }
}

struct Source {
    base: String,
    session: String,
    key: String,
    fetched: Arc<AtomicU64>,
}

impl TileSource for Source {
    fn tile_url(&self, t: TileId) -> String {
        self.fetched.fetch_add(1, Ordering::Relaxed);
        format!("{}/v1/2dtiles/{}/{}/{}?session={}&key={}", self.base, t.zoom, t.x, t.y, self.session, self.key)
    }

    fn attribution(&self) -> Attribution {
        Attribution { text: "Google Maps", url: "https://www.google.com/maps", logo_light: None, logo_dark: None }
    }

    fn max_zoom(&self) -> u8 {
        22
    }
}

struct Live {
    source: MapSource,
    expiry: f64,
    session: String,
    tiles: HttpTiles,
}

/// One area of the viewport answer and how far it can be zoomed in.
#[derive(Clone, Debug, PartialEq)]
struct ZoomRect {
    max_zoom: f64,
    north: f64,
    south: f64,
    east: f64,
    west: f64,
}

impl ZoomRect {
    fn holds(&self, lat: f64, lon: f64) -> bool {
        let across = if self.west <= self.east { lon >= self.west && lon <= self.east } else { lon >= self.west || lon <= self.east };
        lat >= self.south && lat <= self.north && across
    }
}

#[derive(Default)]
struct View {
    copyright: String,
    rects: Vec<ZoomRect>,
}

type Reply = Arc<Mutex<Option<(MapSource, Result<(String, f64), String>)>>>;

/// One Map node's view: its copyright text and zoom limits, asked for once it has settled.
#[derive(Default)]
pub struct ViewAsk {
    /// The session these answers belong to.
    session: String,
    view: Arc<Mutex<View>>,
    seen: (String, Option<Instant>),
    asked: String,
}

/// What the Map nodes share of Google.
pub struct Google {
    pub source: MapSource,
    /// The key being typed.
    pub draft: String,
    pub key: String,
    /// Why the last session could not be had (it is not tried again until asked).
    pub error: String,
    /// Tile requests made since the program started (the free allowance counts these).
    pub fetched: Arc<AtomicU64>,
    /// The service's address (tests point it at their own).
    base: String,
    key_read: bool,
    live: Option<Live>,
    asking: bool,
    failed: bool,
    reply: Reply,
}

impl Default for Google {
    fn default() -> Self {
        Google {
            source: MapSource::Offline,
            draft: String::new(),
            key: String::new(),
            error: String::new(),
            fetched: Default::default(),
            base: ENDPOINT.to_string(),
            key_read: false,
            live: None,
            asking: false,
            failed: false,
            reply: Default::default(),
        }
    }
}

fn key_file() -> PathBuf {
    corroborate::home().join("settings").join("google_maps_key.txt")
}

/// A key as Google writes them: letters, digits, `-` and `_`.
pub fn key_is_valid(k: &str) -> bool {
    !k.is_empty() && k.len() < 200 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// An HTTP request through curl: POST when `body` is given. The address goes on curl's standard
/// input, so the key is not on a command line.
fn call(url: &str, body: Option<&str>) -> Result<String, String> {
    let mut conf = format!("url = \"{url}\"\n");
    if let Some(b) = body {
        conf.push_str("header = \"Content-Type: application/json\"\n");
        conf.push_str(&format!("data = \"{}\"\n", b.replace('\\', "\\\\").replace('"', "\\\"")));
    }
    let mut cmd = Command::new("curl");
    corroborate::no_window(&mut cmd).args(["-s", "-S", "--max-time", "20", "--config", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("cannot run curl: {e}"))?;
    child.stdin.take().ok_or("curl has no input")?.write_all(conf.as_bytes()).map_err(|e| format!("curl: {e}"))?;
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Google's answer as JSON, or what it says went wrong.
fn answer(text: &str) -> Result<Json, String> {
    let j = parse_json(text).ok_or("Google's answer could not be read")?;
    match j.get("error") {
        Some(e) => Err(e.get("message").and_then(Json::str).unwrap_or("Google refused the request").to_string()),
        None => Ok(j),
    }
}

/// The session token and when it ends (seconds since 1970).
fn parse_session(text: &str) -> Result<(String, f64), String> {
    let j = answer(text)?;
    let session = j.get("session").and_then(Json::str).ok_or("Google gave no session")?.to_string();
    let expiry = j.get("expiry").and_then(Json::str).and_then(|e| e.parse().ok()).unwrap_or_else(|| timebase::now_unix() + 86_400.0);
    Ok((session, expiry))
}

fn parse_view(text: &str) -> Result<View, String> {
    let j = answer(text)?;
    let num = |r: &Json, k: &str| r.get(k).and_then(Json::num);
    let rects = j
        .get("maxZoomRects")
        .map_or(&[][..], Json::arr)
        .iter()
        .filter_map(|r| Some(ZoomRect { max_zoom: num(r, "maxZoom")?, north: num(r, "north")?, south: num(r, "south")?, east: num(r, "east")?, west: num(r, "west")? }))
        .collect();
    Ok(View { copyright: j.get("copyright").and_then(Json::str).unwrap_or("").to_string(), rects })
}

impl Google {
    /// True while Google's tiles are what the map shows.
    pub fn live(&self) -> bool {
        self.live.is_some()
    }

    /// A session is on its way.
    pub fn connecting(&self) -> bool {
        self.asking
    }

    pub fn tiles_mut(&mut self) -> Option<&mut HttpTiles> {
        self.live.as_mut().map(|l| &mut l.tiles)
    }

    pub fn set_source(&mut self, s: MapSource) {
        if s == self.source {
            return;
        }
        self.source = s;
        self.again();
        self.live = None;
    }

    /// Clear a failure so the session is tried again.
    pub fn again(&mut self) {
        self.failed = false;
        self.error.clear();
    }

    /// Takes a typed key: saved next to the program, used from now on. False if it cannot be one.
    pub fn use_key(&mut self, typed: &str) -> bool {
        let k = typed.trim();
        if !key_is_valid(k) {
            self.error = "that does not look like an API key".into();
            return false;
        }
        self.key = k.to_string();
        self.draft.clear();
        self.again();
        if !cfg!(test) {
            let f = key_file();
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Err(e) = std::fs::write(&f, k) {
                self.error = format!("cannot save the key: {e}");
            }
        }
        true
    }

    pub fn forget_key(&mut self) {
        self.key.clear();
        self.live = None;
        self.again();
        if !cfg!(test) {
            let _ = std::fs::remove_file(key_file());
        }
    }

    /// Each frame: reads the key once, starts a session when one is needed, takes one that came.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if !self.key_read {
            self.key_read = true;
            if !cfg!(test) && self.key.is_empty() {
                self.key = std::fs::read_to_string(key_file()).map(|k| k.trim().to_string()).unwrap_or_default();
            }
        }
        let came = self.reply.lock().unwrap().take();
        if let Some((source, result)) = came {
            self.asking = false;
            if source == self.source {
                match result {
                    Ok((session, expiry)) => {
                        let src = Source { base: self.base.clone(), session: session.clone(), key: self.key.clone(), fetched: self.fetched.clone() };
                        self.live = Some(Live { source, expiry, session, tiles: HttpTiles::with_options(src, HttpOptions::default(), ctx.clone()) });
                        self.error.clear();
                    }
                    Err(e) => {
                        self.error = e.replace(&self.key, "(key)");
                        self.failed = true;
                    }
                }
            }
        }
        if self.source == MapSource::Offline {
            self.live = None;
            return;
        }
        if self.live.as_ref().is_some_and(|l| l.source != self.source || timebase::now_unix() >= l.expiry) {
            self.live = None;
        }
        if self.live.is_none() && !self.asking && !self.failed && !self.key.is_empty() {
            self.asking = true;
            let (reply, source) = (self.reply.clone(), self.source);
            let url = format!("{}/v1/createSession?key={}", self.base, self.key);
            std::thread::spawn(move || {
                let r = call(&url, Some(source.session_body())).and_then(|t| parse_session(&t));
                *reply.lock().unwrap() = Some((source, r));
            });
        }
        if self.asking {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// A map's view (degrees): once it has settled, asks for its copyright text and zoom limits.
    pub fn ask_view(&mut self, ask: &mut ViewAsk, zoom: f64, north: f64, south: f64, east: f64, west: f64, ctx: &egui::Context) {
        let Some(live) = &self.live else { return };
        if ask.session != live.session {
            *ask = ViewAsk { session: live.session.clone(), ..Default::default() };
        }
        let (north, south) = (north.clamp(-85.0, 85.0), south.clamp(-85.0, 85.0));
        let wrap = |x: f64| (x + 180.0).rem_euclid(360.0) - 180.0;
        let (east, west) = if east - west >= 360.0 { (180.0, -180.0) } else { (wrap(east), wrap(west)) };
        let zoom = zoom.round().clamp(0.0, 22.0) as u32;
        let view = format!("zoom={zoom}&north={north:.4}&south={south:.4}&east={east:.4}&west={west:.4}");
        if view != ask.seen.0 {
            ask.seen = (view, Some(Instant::now()));
            ctx.request_repaint_after(SETTLE);
            return;
        }
        if view == ask.asked {
            return;
        }
        if ask.seen.1.is_some_and(|t| t.elapsed() < SETTLE) {
            ctx.request_repaint_after(SETTLE);
            return;
        }
        ask.asked = view.clone();
        let url = format!("{}/tile/v1/viewport?session={}&key={}&{view}", self.base, live.session, self.key);
        let (out, ctx) = (ask.view.clone(), ctx.clone());
        std::thread::spawn(move || {
            if let Ok(v) = call(&url, None).and_then(|t| parse_view(&t)) {
                *out.lock().unwrap() = v;
                ctx.request_repaint();
            }
        });
    }

    /// What has to be written on a map while Google's tiles are on it.
    pub fn credit(&self, ask: &ViewAsk) -> Option<String> {
        let live = self.live.as_ref()?;
        let c = if ask.session == live.session { ask.view.lock().unwrap().copyright.clone() } else { String::new() };
        Some(if c.is_empty() { "Google Maps".to_string() } else { format!("Google Maps  ·  {c}") })
    }

    /// How far the imagery at a place goes (the area holding it with the most detail).
    pub fn max_zoom_at(&self, ask: &ViewAsk, lat: f64, lon: f64) -> Option<f64> {
        ask.view.lock().unwrap().rects.iter().filter(|r| r.holds(lat, lon)).map(|r| r.max_zoom).max_by(f64::total_cmp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive::{self, Drive};
    use egui_kittest::kittest::Queryable;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;

    /// A 1 × 1 green PNG: the fake server's every tile, so the map turns green.
    const TILE: [u8; 69] = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 208, 88, 16, 5, 0, 2, 22, 1, 35, 45, 46, 62, 168, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    const KEY: &str = "TEST-key_123";

    /// A stand-in for Google on this machine; returns its address and every request it got
    /// (`METHOD path body`). The key `BAD` is refused, as Google refuses one.
    fn fake_google() -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let log: Arc<Mutex<Vec<String>>> = Default::default();
        let seen = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let log = seen.clone();
                std::thread::spawn(move || {
                    let mut r = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if r.read_line(&mut line).is_err() {
                        return;
                    }
                    let (mut len, mut h) = (0usize, String::new());
                    loop {
                        h.clear();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                            break;
                        }
                        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0; len];
                    let _ = r.read_exact(&mut body);
                    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                    log.lock().unwrap().push(format!("{} {path} {}", line.split(' ').next().unwrap_or(""), String::from_utf8_lossy(&body)));
                    let (status, kind, out): (&str, &str, Vec<u8>) = if path.contains("key=BAD") {
                        ("403 Forbidden", "application/json", br#"{"error":{"code":403,"message":"API key not valid (BAD)"}}"#.to_vec())
                    } else if path.starts_with("/v1/createSession") {
                        ("200 OK", "application/json", br#"{"session":"SESSION1","expiry":"4102444800","tileWidth":256,"tileHeight":256,"imageFormat":"png"}"#.to_vec())
                    } else if path.starts_with("/tile/v1/viewport") {
                        ("200 OK", "application/json", br#"{"copyright":"Imagery 2026 Test Sat","maxZoomRects":[{"maxZoom":9,"north":90,"south":-90,"east":180,"west":-180}]}"#.to_vec())
                    } else {
                        ("200 OK", "image/png", TILE.to_vec())
                    };
                    let mut s = stream;
                    let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len());
                    let _ = s.write_all(&out);
                });
            }
        });
        (addr, log)
    }

    /// Opens the drop-down list showing `value`.
    fn open_list(d: &mut Drive, value: &str) {
        d.h.query_all_by(|n| n.value().as_deref() == Some(value)).next().expect("the list").click();
        d.frames(3);
    }

    /// A place wired into a Map node, served by `addr`; the node's stable id.
    fn on_map(d: &mut Drive, addr: &str) -> String {
        d.app().maps.google.base = addr.to_string();
        let v = d.app().add(drive::place(48.85, 2.35, 0.0), Pos2::new(40.0, 40.0));
        let m = d.app().add(Kind::Map, Pos2::new(420.0, 40.0));
        d.app().wire_many(v, "value", m);
        d.frames(3);
        d.app().nodes[m].uid.clone()
    }

    fn credit(d: &mut Drive, uid: &str) -> Option<String> {
        let m = &d.app().maps;
        m.google.credit(&m.views[uid].ask)
    }

    fn memory<'a>(d: &'a mut Drive, uid: &str) -> &'a mut walkers::MapMemory {
        &mut d.app().maps.views.get_mut(uid).unwrap().memory
    }

    /// Offline sends nothing. Switched on through the list: a session, tiles naming it and the
    /// key, the copyright on the map, the zoom stopped where the imagery ends. Back to offline
    /// and the requests stop.
    #[test]
    fn google_on_a_map_node() {
        let (addr, log) = fake_google();
        let mut d = Drive::new();
        let uid = on_map(&mut d, &addr);
        d.app().maps.google.key = KEY.into();
        d.wait(0.3);
        assert!(log.lock().unwrap().is_empty(), "offline sends nothing, key or no key");
        assert_eq!(d.app().maps.google.source, MapSource::Offline, "offline at the start");
        d.shot("google_1_offline");

        open_list(&mut d, "Offline maps");
        d.click("Google satellite");
        assert_eq!(d.app().maps.google.source, MapSource::Satellite);
        assert!(d.wait_until(10.0, |a| a.maps.google.live()), "a session: {}", d.app().maps.google.error);
        let _ = memory(&mut d, &uid).set_zoom(14.0);
        let got = |log: &Arc<Mutex<Vec<String>>>, part: &str| log.lock().unwrap().iter().any(|l| l.contains(part));
        let end = Instant::now() + Duration::from_secs(15);
        while Instant::now() < end && !(got(&log, "/v1/2dtiles/") && got(&log, "/tile/v1/viewport") && credit(&mut d, &uid).is_some_and(|c| c.contains("Test Sat"))) {
            d.h.step();
            std::thread::sleep(Duration::from_millis(50));
        }
        d.wait(1.0);
        let l = log.lock().unwrap().clone();
        let first = &l[0];
        assert!(first.starts_with("POST /v1/createSession?key=TEST-key_123") && first.contains(r#""mapType":"satellite""#), "{first}");
        let tile = l.iter().find(|x| x.contains("/v1/2dtiles/")).expect("a tile was asked for");
        assert!(tile.contains("session=SESSION1") && tile.contains("key=TEST-key_123"), "{tile}");
        assert!(l.iter().any(|x| x.contains("/tile/v1/viewport?session=SESSION1") && x.contains("zoom=")), "{l:?}");
        assert_eq!(credit(&mut d, &uid).unwrap(), "Google Maps  ·  Imagery 2026 Test Sat");
        assert!(d.app().maps.google.fetched.load(Ordering::Relaxed) > 0);
        let z = memory(&mut d, &uid).zoom();
        assert!(z <= 9.0, "stops where the imagery ends: {z}");
        d.shot("google_2_satellite");

        open_list(&mut d, "Google satellite");
        d.click("Offline maps");
        assert!(!d.app().maps.google.live());
        d.wait(0.5);
        let n = log.lock().unwrap().len();
        memory(&mut d, &uid).center_at(walkers::lat_lon(10.0, 10.0));
        d.wait(1.0);
        assert_eq!(log.lock().unwrap().len(), n, "back offline, nothing more is asked");
        assert!(credit(&mut d, &uid).is_none());
        d.shot("google_3_offline_again");
    }

    /// With no key the node asks for one and sends nothing; a refused key shows the reason
    /// without the key, and is not tried over and over.
    #[test]
    fn google_keys() {
        let (addr, log) = fake_google();
        let mut d = Drive::new();
        on_map(&mut d, &addr);
        d.app().maps.google.set_source(MapSource::Road);
        d.wait(0.3);
        assert!(log.lock().unwrap().is_empty(), "no key, no request");
        assert!(d.h.query_by_label("API key").is_some(), "the node asks for a key");
        d.shot("google_4_key_field");

        assert!(!d.app().maps.google.use_key("not a key"));
        assert!(!d.app().maps.google.error.is_empty());
        assert!(d.app().maps.google.use_key("  BAD\n"), "spaces around a pasted key go");
        assert!(d.wait_until(10.0, |a| !a.maps.google.error.is_empty()), "refused");
        d.wait(1.0);
        let e = d.app().maps.google.error.clone();
        assert_eq!(e, "API key not valid ((key))", "the reason, without the key");
        assert!(log.lock().unwrap()[0].contains(r#""mapType":"roadmap""#));
        assert_eq!(log.lock().unwrap().len(), 1, "not tried over and over");
        assert!(!d.app().maps.google.live());
        d.shot("google_5_refused");
        d.app().maps.google.again();
        d.wait(1.0);
        assert_eq!(log.lock().unwrap().len(), 2, "Try again asks once more");
        assert!(d.wait_until(5.0, |a| !a.maps.google.error.is_empty()), "refused again");
        d.frames(2);
        d.click("Change key");
        assert!(d.app().maps.google.key.is_empty() && d.app().maps.google.error.is_empty());
        assert!(d.h.query_by_label("API key").is_some(), "the field is back");
    }

    #[test]
    fn answers_and_areas() {
        let s = parse_session(r#"{"session":"a","expiry":"100"}"#);
        assert_eq!(s, Ok(("a".to_string(), 100.0)));
        assert_eq!(parse_session(r#"{"error":{"message":"no"}}"#), Err("no".to_string()));
        assert!(parse_session("<html>").is_err());
        let v = parse_view(r#"{"copyright":"c","maxZoomRects":[{"maxZoom":19,"north":10,"south":0,"east":-170,"west":170}]}"#).unwrap();
        assert_eq!(v.copyright, "c");
        assert!(v.rects[0].holds(5.0, 175.0) && v.rects[0].holds(5.0, -175.0), "an area across the date line");
        assert!(!v.rects[0].holds(5.0, 0.0) && !v.rects[0].holds(11.0, 175.0));
        assert!(key_is_valid("AIza-abc_1") && !key_is_valid("") && !key_is_valid("a\"b") && !key_is_valid("a b"));
    }
}
