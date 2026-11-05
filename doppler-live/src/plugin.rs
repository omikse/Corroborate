//! Plugins: separate programs that analyse the same audio as the built-in modules.
//!
//! A plugin is an executable. The viewer starts it when a source opens and talks to it through
//! pipes, so a plugin can be written in any language and a crash cannot take the viewer down.
//!
//! - **arguments**: `--source <path or URL>` and `--file` or `--live`, so a plugin can read the
//!   source's own metadata. Plugins that only need audio can ignore them.
//! - **stdin**: the audio, raw f32 little-endian, mono, 16 000 Hz, from the first sample of the
//!   source. End of input means the source ended: report what is still held, then exit.
//! - **stdout**: one JSON object per line. Times are seconds from the first sample; frequencies Hz.
//!   ```text
//!   {"type":"hello","name":"ENF","about":"mains hum","color":"#78aaff"}       first line
//!   {"type":"status","text":"now 59.9976 Hz"}         latest state for the plugin's box ("\n" = new line)
//!   {"type":"view","f_lo":59.5,"f_hi":60.5}           the band the plugin's view should show
//!   {"type":"track","id":"h1","t":12.0,"hz":59.993,"ok":true}   a point of a measured frequency line
//!   {"type":"finding","id":1,"t_start":17.0,"t_end":119.0,"f_lo":59.96,"f_hi":60.01,"label":"…","detail":"…"}
//!   {"type":"meta","key":"coordinates","value":"40.7128, -74.0060"}   a fact for the result JSON
//!   ```
//! - A plugin must read stdin to the end, even if it ignores the audio: a file's analysis waits
//!   for every plugin to take its audio.
//! - **stderr**: free text for people; the viewer shows the last line.
//!
//! Unknown message types and fields are ignored, so the protocol can grow.

use crate::timebase::json_escape;

/// A value in a flat JSON object. Nested objects and arrays are skipped (read as `Null`).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
}

/// Something a plugin reports.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginFinding {
    pub id: u64,
    pub t_start: f64,
    pub t_end: f64,
    pub f_lo: f64,
    pub f_hi: f64,
    /// One line for the list.
    pub label: String,
    /// More detail, shown on hover.
    pub detail: String,
}

/// One line of plugin output.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    Hello { name: String, about: String, color: Option<[u8; 3]> },
    Status(String),
    View { f_lo: f64, f_hi: f64 },
    Track { id: String, t: f64, hz: f64, ok: bool },
    Finding(PluginFinding),
    /// A fact about the source for the result JSON; a later value for the same key replaces it.
    Meta { key: String, value: String },
}

// ---------------------------------------------------------------- reading

struct Parser<'a> {
    c: std::iter::Peekable<std::str::Chars<'a>>,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.c.peek().is_some_and(|ch| ch.is_whitespace()) {
            self.c.next();
        }
    }

    fn word(&mut self, w: &str) -> Option<()> {
        for ch in w.chars() {
            if self.c.next()? != ch {
                return None;
            }
        }
        Some(())
    }

    fn string(&mut self) -> Option<String> {
        if self.c.next()? != '"' {
            return None;
        }
        let mut out = String::new();
        loop {
            match self.c.next()? {
                '"' => return Some(out),
                '\\' => match self.c.next()? {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' | 'f' => {}
                    'u' => {
                        let hex: String = (0..4).filter_map(|_| self.c.next()).collect();
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            out.push(ch);
                        }
                    }
                    other => out.push(other),
                },
                ch => out.push(ch),
            }
        }
    }

    /// Skip a nested object or array, strings included.
    fn skip_nested(&mut self) -> Option<()> {
        let mut depth = 0i32;
        loop {
            let ch = *self.c.peek()?;
            if ch == '"' {
                self.string()?;
                continue;
            }
            self.c.next();
            match ch {
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
                _ => {}
            }
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.ws();
        match *self.c.peek()? {
            '"' => Some(Value::Str(self.string()?)),
            '{' | '[' => {
                self.skip_nested()?;
                Some(Value::Null)
            }
            't' => self.word("true").map(|_| Value::Bool(true)),
            'f' => self.word("false").map(|_| Value::Bool(false)),
            'n' => self.word("null").map(|_| Value::Null),
            _ => {
                let mut t = String::new();
                while let Some(&ch) = self.c.peek() {
                    if ch.is_ascii_digit() || "+-.eE".contains(ch) {
                        t.push(ch);
                        self.c.next();
                    } else {
                        break;
                    }
                }
                t.parse().ok().map(Value::Num)
            }
        }
    }
}

impl Parser<'_> {
    /// An object's fields. With `want`, stop at that key instead and return its own object's
    /// fields (one level down); None if the key is missing or not an object.
    fn object(&mut self, want: Option<&str>) -> Option<Vec<(String, Value)>> {
        self.ws();
        if self.c.next()? != '{' {
            return None;
        }
        let mut out = Vec::new();
        self.ws();
        if self.c.peek() == Some(&'}') {
            self.c.next();
            return if want.is_some() { None } else { Some(out) };
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            if self.c.next()? != ':' {
                return None;
            }
            self.ws();
            if want == Some(k.as_str()) {
                return if self.c.peek() == Some(&'{') { self.object(None) } else { None };
            }
            let v = self.value()?;
            out.push((k, v));
            self.ws();
            match self.c.next()? {
                ',' => continue,
                '}' => return if want.is_some() { None } else { Some(out) },
                _ => return None,
            }
        }
    }
}

/// Parse one JSON object into its top-level fields; None if it is not a well-formed object.
pub fn parse_object(line: &str) -> Option<Vec<(String, Value)>> {
    Parser { c: line.chars().peekable() }.object(None)
}

/// The fields of the object stored under top-level `key` in a JSON document, e.g. the
/// `"manual": {…}` block of a result file.
pub fn get_object(doc: &str, key: &str) -> Option<Vec<(String, Value)>> {
    Parser { c: doc.chars().peekable() }.object(Some(key))
}

fn parse_color(c: &str) -> Option<[u8; 3]> {
    let h = c.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Read one line of plugin output; None for anything that is not a known, complete message.
pub fn parse_line(line: &str) -> Option<Msg> {
    let obj = parse_object(line)?;
    let get = |k: &str| obj.iter().find(|(n, _)| n == k).map(|(_, v)| v);
    let s = |k: &str| match get(k) {
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    };
    let n = |k: &str| match get(k) {
        Some(Value::Num(v)) => Some(*v),
        _ => None,
    };
    match s("type").as_str() {
        "hello" => Some(Msg::Hello { name: s("name"), about: s("about"), color: parse_color(&s("color")) }),
        "status" => Some(Msg::Status(s("text"))),
        "view" => Some(Msg::View { f_lo: n("f_lo")?, f_hi: n("f_hi")? }),
        "track" => Some(Msg::Track { id: s("id"), t: n("t")?, hz: n("hz")?, ok: get("ok") != Some(&Value::Bool(false)) }),
        "finding" => Some(Msg::Finding(PluginFinding {
            id: n("id").unwrap_or(0.0) as u64,
            t_start: n("t_start")?,
            t_end: n("t_end")?,
            f_lo: n("f_lo")?,
            f_hi: n("f_hi")?,
            label: s("label"),
            detail: s("detail"),
        })),
        "meta" => {
            let value = match get("value")? {
                Value::Str(v) => v.clone(),
                Value::Num(v) => v.to_string(),
                Value::Bool(v) => v.to_string(),
                Value::Null => String::new(),
            };
            Some(Msg::Meta { key: s("key"), value })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- writing (for plugins)

pub fn hello(name: &str, about: &str, color: [u8; 3]) -> String {
    format!(
        r##"{{"type":"hello","name":"{}","about":"{}","color":"#{:02x}{:02x}{:02x}"}}"##,
        json_escape(name),
        json_escape(about),
        color[0],
        color[1],
        color[2]
    )
}

pub fn status(text: &str) -> String {
    format!(r#"{{"type":"status","text":"{}"}}"#, json_escape(text))
}

pub fn view(f_lo: f64, f_hi: f64) -> String {
    format!(r#"{{"type":"view","f_lo":{f_lo:.5},"f_hi":{f_hi:.5}}}"#)
}

pub fn track(id: &str, t: f64, hz: f64, ok: bool) -> String {
    format!(r#"{{"type":"track","id":"{}","t":{t:.3},"hz":{hz:.5},"ok":{ok}}}"#, json_escape(id))
}

pub fn finding(f: &PluginFinding) -> String {
    format!(
        r#"{{"type":"finding","id":{},"t_start":{:.3},"t_end":{:.3},"f_lo":{:.5},"f_hi":{:.5},"label":"{}","detail":"{}"}}"#,
        f.id,
        f.t_start,
        f.t_end,
        f.f_lo,
        f.f_hi,
        json_escape(&f.label),
        json_escape(&f.detail)
    )
}

pub fn meta(key: &str, value: &str) -> String {
    format!(r#"{{"type":"meta","key":"{}","value":"{}"}}"#, json_escape(key), json_escape(value))
}

/// The value after `--source` in this program's arguments, and whether `--live` was given.
pub fn source_args() -> (Option<String>, bool) {
    let args: Vec<String> = std::env::args().collect();
    let source = args.iter().position(|a| a == "--source").and_then(|i| args.get(i + 1)).cloned();
    (source, args.iter().any(|a| a == "--live"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let f = PluginFinding {
            id: 3,
            t_start: 17.0,
            t_end: 119.5,
            f_lo: 59.96,
            f_hi: 60.01,
            label: "59.9925 Hz  -35..+11 mHz".into(),
            detail: "harmonic 2 \"quoted\"\nsecond line".into(),
        };
        assert_eq!(parse_line(&finding(&f)), Some(Msg::Finding(f)));
        assert_eq!(
            parse_line(&hello("ENF", "mains hum", [120, 170, 255])),
            Some(Msg::Hello { name: "ENF".into(), about: "mains hum".into(), color: Some([120, 170, 255]) })
        );
        assert_eq!(parse_line(&status("a\nb")), Some(Msg::Status("a\nb".into())));
        assert_eq!(parse_line(&view(59.5, 60.5)), Some(Msg::View { f_lo: 59.5, f_hi: 60.5 }));
        assert_eq!(parse_line(&track("h1", 12.0, 59.993, false)), Some(Msg::Track { id: "h1".into(), t: 12.0, hz: 59.993, ok: false }));
        assert_eq!(parse_line(&meta("place", "Oyster Bay, \"NY\"")), Some(Msg::Meta { key: "place".into(), value: "Oyster Bay, \"NY\"".into() }));
        assert_eq!(parse_line(r#"{"type":"meta","key":"n","value":3}"#), Some(Msg::Meta { key: "n".into(), value: "3".into() }));
    }

    #[test]
    fn nested_object_by_key() {
        let doc = "{
  \"file\": \"a.mp3\",
  \"events\": [{\"x\": 1}],
  \"manual\": {\"date\": \"2026-11-04\", \"notes\": \"} inside\"},
  \"tail\": 2
}";
        assert_eq!(
            get_object(doc, "manual"),
            Some(vec![("date".into(), Value::Str("2026-11-04".into())), ("notes".into(), Value::Str("} inside".into()))])
        );
        assert_eq!(get_object(doc, "missing"), None);
        assert_eq!(get_object(doc, "file"), None);
    }

    #[test]
    fn tolerant_reading() {
        // Unknown fields, nested values, spacing and unicode escapes are fine.
        let line = r#" { "type" : "status", "extra": {"a": [1, "}"]}, "text": "café" } "#;
        assert_eq!(parse_line(line), Some(Msg::Status("café".into())));
        // Unknown types, missing fields and junk are ignored.
        assert_eq!(parse_line(r#"{"type":"future_thing"}"#), None);
        assert_eq!(parse_line(r#"{"type":"view","f_lo":1}"#), None);
        assert_eq!(parse_line("not json"), None);
        // A track without "ok" counts as ok.
        assert_eq!(parse_line(r#"{"type":"track","id":"x","t":1,"hz":2}"#), Some(Msg::Track { id: "x".into(), t: 1.0, hz: 2.0, ok: true }));
    }
}
