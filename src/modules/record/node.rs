//! The Result record node in the workbench. Anything can be wired into it: each input takes the
//! kind of what is wired in, and a free grey input waits below. Static data (metadata, facts,
//! places) is saved at once with "Save now"; streams (audio, events, tracks) are recorded from
//! "● Record" until "■ End recording": the audio of each source into a WAV file, and the
//! findings into a result JSON kept up to date every 30 s. Also gives the location its location
//! inputs agree on. Compiled into the `corroborate` program.

use super::*;
use corroborate::modules::record::{is_stream, kind_name};
use std::io::{Seek, SeekFrom, Write};

/// 16-bit mono WAV written as the audio arrives; its sizes are filled in when it is closed.
pub struct WavRec {
    w: std::io::BufWriter<std::fs::File>,
    n: u64,
}

impl WavRec {
    pub fn create(path: &std::path::Path) -> std::io::Result<Self> {
        let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
        let sr = corroborate::INPUT_SR as u32;
        w.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
        for v in [16u32, 1 | (1 << 16), sr, sr * 2, 2 | (16 << 16)] {
            w.write_all(&v.to_le_bytes())?;
        }
        w.write_all(b"data\0\0\0\0")?;
        Ok(Self { w, n: 0 })
    }

    pub fn push(&mut self, x: &[f32]) {
        for &v in x {
            let _ = self.w.write_all(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        self.n += x.len() as u64;
    }

    pub fn seconds(&self) -> f64 {
        self.n as f64 / corroborate::INPUT_SR
    }
}

impl Drop for WavRec {
    fn drop(&mut self) {
        let bytes = (self.n * 2).min(u32::MAX as u64 - 36) as u32;
        let _ = self.w.flush();
        let f = self.w.get_mut();
        let _ = f.seek(SeekFrom::Start(4)).and_then(|_| f.write_all(&(36 + bytes).to_le_bytes()));
        let _ = f.seek(SeekFrom::Start(40)).and_then(|_| f.write_all(&bytes.to_le_bytes()));
    }
}

/// Audio taps of a source: (record node, its WAV). The source's read loop writes into them.
pub type Taps = Arc<std::sync::Mutex<Vec<(NodeId, WavRec)>>>;

/// A recording in progress in one record node.
pub struct Recording {
    started: f64,
    json: PathBuf,
    wavs: Vec<String>,
    last_save: Instant,
}

/// What is wired into a record: (input name, node, its output, the output's kind).
pub type Fed = (&'static str, NodeId, &'static str, PortKind);

impl App {
    pub(crate) fn record_inputs(&self, rec: NodeId) -> Vec<Fed> {
        self.graph.nodes()[rec]
            .spec
            .inputs
            .iter()
            .filter_map(|p| {
                let (from, out) = self.graph.feeder(rec, p.name)?;
                let kind = self.graph.nodes()[from].spec.outputs.iter().find(|o| o.name == out)?.kind;
                Some((p.name, from, out, kind))
            })
            .collect()
    }

    /// Every record shows its wired inputs plus one free one.
    pub(crate) fn sync_record_ports(&mut self) {
        for i in 0..self.nodes.len() {
            if matches!(self.nodes[i].kind, Kind::Record { .. }) {
                self.graph.pack_any_inputs(i, &nodes::RECORD_INPUTS);
            }
        }
    }

    /// Wire `from.out` into the record's free input (old project files, tests).
    pub(crate) fn wire_record(&mut self, from: NodeId, out: &str, rec: NodeId) {
        self.sync_record_ports();
        if let Some(free) = self.graph.nodes()[rec].spec.inputs.last().map(|p| p.name) {
            self.wire(from, out, rec, free);
        }
        self.sync_record_ports();
    }

    pub(crate) fn record_folder(&self, rec: NodeId) -> PathBuf {
        match &self.nodes[rec].kind {
            Kind::Record { folder } if !folder.trim().is_empty() => PathBuf::from(folder.trim()),
            _ => results_dir(),
        }
    }

    /// The record's location: the whole world, AND each area wired into it.
    /// Steps: (node, how its area was reached, the fraction of Earth it allows or None while it has none).
    pub(crate) fn record_area(&self, rec: NodeId) -> (WorldMask, Vec<(String, String, Option<f64>)>) {
        let mut m = WorldMask::world();
        let mut steps = Vec::new();
        for (_, from, _, kind) in self.record_inputs(rec) {
            if kind != PortKind::Location {
                continue;
            }
            let label = self.nodes[from].label.clone();
            match self.area_from(from) {
                Ok((a, note)) => {
                    let al = a.allowed();
                    m = m.and(&al);
                    steps.push((label, note, Some(al.fraction())));
                }
                Err(e) => steps.push((label, e, None)),
            }
        }
        (m, steps)
    }

    /// The area on the record's location output.
    pub(crate) fn record_area_out(&self, i: NodeId) -> Result<(Arc<Area>, String), String> {
        let (m, steps) = self.record_area(i);
        let k = steps.iter().filter(|s| s.2.is_some()).count();
        Ok((Arc::new(Area::add(m)), format!("the whole world cut by {k} area(s)")))
    }

    pub(crate) fn record_status(&self, i: NodeId) -> String {
        if let Some(r) = self.recordings.get(&i) {
            return format!("● recording {}", source_node::fmt_secs(timebase::now_unix() - r.started));
        }
        self.record_notes.get(&i).cloned().unwrap_or_else(|| "not recording".into())
    }

    pub(crate) fn record_json(&self, i: NodeId) -> String {
        let (m, steps) = self.record_area(i);
        let st: Vec<String> = steps
            .iter()
            .map(|(n, note, f)| {
                format!(
                    "{{\"node\":\"{}\",\"note\":\"{}\",\"fraction_of_earth\":{}}}",
                    json_escape(n),
                    json_escape(note),
                    f.map_or("null".to_string(), |f| format!("{f:.5}"))
                )
            })
            .collect();
        let ins: Vec<String> = self
            .record_inputs(i)
            .iter()
            .map(|(inp, from, out, k)| {
                format!(
                    "{{\"in\":\"{inp}\",\"from\":{from},\"node\":\"{}\",\"out\":\"{out}\",\"kind\":\"{}\",\"stream\":{}}}",
                    json_escape(&self.nodes[*from].label),
                    kind_name(*k),
                    is_stream(*k)
                )
            })
            .collect();
        let rec = match self.recordings.get(&i) {
            Some(r) => format!(
                ",\"recording\":{{\"start_utc\":\"{}\",\"audio_files\":[{}]}}",
                timebase::fmt_utc(r.started),
                r.wavs.iter().map(|w| format!("\"{}\"", json_escape(w))).collect::<Vec<_>>().join(",")
            ),
            None => String::new(),
        };
        format!(
            ",\"inputs\":[{}]{rec},\"location\":{{\"start\":\"whole world\",\"cell_deg\":1,\"remaining_fraction_of_earth\":{:.5},\"remaining_km2\":{:.0},\"contradiction\":{},\"steps\":[{}],\"cells_row_first_last\":{}}}",
            ins.join(","),
            m.fraction(),
            m.area_km2(),
            m.is_empty(),
            st.join(","),
            m.runs_json()
        )
    }

    fn write_json(&self, path: &std::path::Path, ids: &[NodeId]) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = self.result_json(ids);
        std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| format!("could not save: {e}"))?;
        Ok(())
    }

    /// Save now: the static inputs (and everything they come from), plus the record itself.
    pub(crate) fn record_save_static(&mut self, i: NodeId) {
        let mut ids: Vec<NodeId> = Vec::new();
        for (_, from, _, k) in self.record_inputs(i) {
            if !is_stream(k) {
                for n in self.graph.trail(from).unwrap_or_default() {
                    if !ids.contains(&n) {
                        ids.push(n);
                    }
                }
            }
        }
        ids.push(i);
        let path = self.record_folder(i).join(format!("static-{}.json", timebase::utc_stamp(timebase::now_unix())));
        let note = match self.write_json(&path, &ids) {
            Ok(()) => format!("saved {}", path.file_name().unwrap_or_default().to_string_lossy()),
            Err(e) => e,
        };
        self.record_notes.insert(i, note);
    }

    /// ● Record: a WAV for each audio input's source, and the result JSON of everything wired in.
    pub(crate) fn record_start(&mut self, i: NodeId) {
        let dir = self.record_folder(i);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.record_notes.insert(i, format!("cannot create {}: {e}", dir.display()));
            return;
        }
        let now = timebase::now_unix();
        let stamp = timebase::utc_stamp(now);
        let mut wavs = Vec::new();
        for (_, from, _, k) in self.record_inputs(i) {
            if k != PortKind::Audio {
                continue;
            }
            let name = format!("{stamp}-{}.wav", clean(&self.nodes[from].label).replace([' ', '/', '\\', ':', '(', ')'], "_"));
            match WavRec::create(&dir.join(&name)) {
                Ok(w) => {
                    self.nodes[from].taps.lock().unwrap().push((i, w));
                    wavs.push(name);
                }
                Err(e) => {
                    self.record_notes.insert(i, format!("cannot write {name}: {e}"));
                }
            }
        }
        let json = dir.join(format!("corroborate-{stamp}.json"));
        self.recordings.insert(i, Recording { started: now, json, wavs, last_save: Instant::now() });
        self.record_save(i);
    }

    /// Keep the recording's result JSON up to date (every 30 s, and at the end).
    pub(crate) fn record_save(&mut self, i: NodeId) {
        let Some(path) = self.recordings.get(&i).map(|r| r.json.clone()) else { return };
        let ids = self.graph.trail(i).unwrap_or_else(|_| vec![i]);
        if let Err(e) = self.write_json(&path, &ids) {
            self.record_notes.insert(i, e);
        }
        if let Some(r) = self.recordings.get_mut(&i) {
            r.last_save = Instant::now();
        }
    }

    pub(crate) fn record_end(&mut self, i: NodeId) {
        for n in &self.nodes {
            n.taps.lock().unwrap().retain(|(r, _)| *r != i); // dropping a WAV closes it
        }
        self.record_save(i);
        if let Some(r) = self.recordings.remove(&i) {
            self.record_notes.insert(
                i,
                format!("recorded {}: {}", source_node::fmt_secs(timebase::now_unix() - r.started), r.json.file_name().unwrap_or_default().to_string_lossy()),
            );
        }
    }

    /// Every 30 s while recording.
    pub(crate) fn record_tick(&mut self) {
        let due: Vec<NodeId> = self.recordings.iter().filter(|(_, r)| r.last_save.elapsed() > Duration::from_secs(30)).map(|(i, _)| *i).collect();
        for i in due {
            self.record_save(i);
        }
    }

    fn record_body(&mut self, ui: &mut egui::Ui, i: NodeId) {
        let z = self.zoom;
        let r = look::content(self, ui, i);
        look::area(ui, r, |ui| {
            let recording = self.recordings.contains_key(&i);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("folder").small().color(DIM));
                if let Kind::Record { folder } = &mut self.nodes[i].kind {
                    let hint = results_dir().display().to_string();
                    let w = (ui.available_width() - 60.0 * z).max(40.0);
                    ui.add_enabled(!recording, egui::TextEdit::singleline(folder).hint_text(hint).desired_width(w));
                }
                if ui.small_button("Open").clicked() {
                    let d = self.record_folder(i);
                    let _ = std::fs::create_dir_all(&d);
                    let _ = std::process::Command::new("explorer").arg(d).spawn();
                }
            });
            let fed = self.record_inputs(i);
            let streams = fed.iter().filter(|f| is_stream(f.3)).count();
            ui.horizontal(|ui| {
                if ui.add_enabled(fed.len() > streams, egui::Button::new("Save now")).on_hover_text("save the static inputs (metadata, facts, places) now").clicked() {
                    self.record_save_static(i);
                }
                if recording {
                    if ui.button(egui::RichText::new("■ End recording").color(LIVE)).clicked() {
                        self.record_end(i);
                    }
                } else if ui.add_enabled(!fed.is_empty(), egui::Button::new(egui::RichText::new("● Record").color(LIVE))).clicked() {
                    self.record_start(i);
                }
            });
            if fed.is_empty() {
                look::line(ui, "wire anything into the grey input", DIM);
            }
            for (_, from, out, k) in &fed {
                let what = if is_stream(*k) { if recording { "recording" } else { "stream: ● Record" } } else { "static: Save now" };
                let col = if is_stream(*k) && recording { LIVE } else { kind_color(*k) };
                look::line(ui, format!("{} · {out} ({}) — {what}", self.nodes[*from].label, kind_name(*k)), col);
            }
            if let Some(rec) = self.recordings.get(&i) {
                let secs: Vec<String> = fed
                    .iter()
                    .filter(|f| f.3 == PortKind::Audio)
                    .filter_map(|f| self.nodes[f.1].taps.lock().unwrap().iter().find(|t| t.0 == i).map(|t| source_node::fmt_secs(t.1.seconds())))
                    .collect();
                look::line(ui, format!("{} · audio {}", rec.json.file_name().unwrap_or_default().to_string_lossy(), if secs.is_empty() { "—".into() } else { secs.join(", ") }), DIM);
                if fed.iter().any(|f| f.3 == PortKind::Video) {
                    look::line(ui, "video: its frames are not recorded yet (audio is)", WARN);
                }
            }
            let (m, steps) = self.record_area(i);
            if !steps.is_empty() {
                let t = if m.is_empty() { "location: nothing left (contradiction)".to_string() } else { format!("location: {:.2} % of Earth left", m.fraction() * 100.0) };
                look::line(ui, t, if m.is_empty() { WARN } else { TEXT });
            }
        });
    }
}

pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, standard: Vec2) -> Vec2 {
        let rows = app.record_inputs(i).len() as f32;
        Vec2::new(430.0, standard.y.max(150.0 + rows * 16.0))
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        app.record_body(ui, i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed WAV says how much audio it holds.
    #[test]
    fn wav_sizes_are_filled_in() {
        let p = std::env::temp_dir().join("corroborate-wav-test.wav");
        {
            let mut w = WavRec::create(&p).unwrap();
            w.push(&[0.5; 1600]);
        }
        let b = std::fs::read(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(b.len(), 44 + 3200);
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 3200);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + 3200);
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), corroborate::INPUT_SR as u32);
    }
}
