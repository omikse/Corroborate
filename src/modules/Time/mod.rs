//! Time node: a time corrected in the open. It takes a time (wired or typed), applies its
//! modification rows from the top down, and gives the result. The arithmetic is here; the node on
//! the canvas is in `node.rs`. A node's rows are saved as `in.year`, … and `in.error_s` for the
//! typed time, and `1.year`, …, `1.sign` and `1.error_s` for each modification row.
//!
//! Two stacks run down the rows: the date stack adds or takes away each row's boxes into the
//! result's date, and the error stack adds every row's ± as squares into the result's ±,
//! √(Σ ±²) (root sum of squares). A row's sign does not touch its ±: an uncertainty is never
//! subtracted.

use crate::core::{NodeSpec, Port, PortKind};
use crate::module_manifest::intern;
use crate::timebase::{civil_from_days, days_from_civil};

crate::builtin_manifest!();

/// The boxes of a row, in order.
pub const BOXES: [&str; 7] = ["year", "month", "day", "hours", "min", "sec", "ms"];

/// A saved row's value.
pub fn get<'a>(rows: &'a [(String, String)], key: &str) -> &'a str {
    rows.iter().find(|(k, _)| k == key).map_or("", |(_, v)| v.as_str()).trim()
}

/// The numbers of its modification rows, in order. They stay with a row while others come and go.
pub fn numbers(rows: &[(String, String)]) -> Vec<usize> {
    let mut v: Vec<usize> = rows.iter().filter_map(|(k, _)| k.strip_suffix(".sign")?.parse().ok()).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Adds a modification row (it subtracts until changed), every box and its ± at 0. Its number.
pub fn add_row(rows: &mut Vec<(String, String)>) -> usize {
    let n = numbers(rows).last().map_or(1, |m| m + 1);
    rows.push((format!("{n}.sign"), "-".to_string()));
    for b in BOXES.iter().chain(&["error_s"]) {
        rows.push((format!("{n}.{b}"), "0".to_string()));
    }
    n
}

/// A project saved before every row had a ±: its one `error_s` becomes a row of zeros with that
/// ±, added at the bottom, so the result's date and ± stay as they were.
pub fn migrate(rows: &mut Vec<(String, String)>) {
    let Some(at) = rows.iter().position(|(k, _)| k == "error_s") else { return };
    let (_, old) = rows.remove(at);
    if old.trim().parse::<f64>().is_ok_and(|e| e != 0.0) {
        let n = add_row(rows);
        set(rows, &format!("{n}.sign"), "+");
        set(rows, &format!("{n}.error_s"), old.trim());
    }
}

fn set(rows: &mut [(String, String)], key: &str, v: &str) {
    if let Some((_, x)) = rows.iter_mut().find(|(k, _)| k == key) {
        *x = v.to_string();
    }
}

pub fn remove_row(rows: &mut Vec<(String, String)>, n: usize) {
    let p = format!("{n}.");
    rows.retain(|(k, _)| !k.starts_with(&p));
}

/// The input port of modification row `n`.
pub fn row_port(n: usize) -> &'static str {
    intern(&format!("row {n}"))
}

/// The empty port level with "+ row": a length wired into it adds a row and moves there.
pub const NEXT: &str = "next row";

/// Its ports: its module.toml, a length input for each modification row, and the waiting one.
pub fn spec(rows: &[(String, String)]) -> NodeSpec {
    let mut s = manifest().spec();
    s.inputs.extend(numbers(rows).into_iter().map(|n| Port::opt(row_port(n), PortKind::Custom("length"))));
    s.inputs.push(Port::opt(NEXT, PortKind::Custom("length")));
    s
}

/// A new node's rows: the input's ± at 0.
pub fn defaults() -> Vec<(String, String)> {
    vec![("in.error_s".to_string(), "0".to_string())]
}

fn whole(s: &str) -> Option<i64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// The time typed into the input row. Year, month and day are needed; the rest counts as 0.
pub fn typed(rows: &[(String, String)]) -> Result<f64, String> {
    let box_of = |b: &str| get(rows, &format!("in.{b}"));
    if BOXES.iter().all(|b| box_of(b).is_empty()) {
        return Err("type a time or wire one in".into());
    }
    let need = |b: &str| whole(box_of(b)).ok_or("the input row isn't a real date and time");
    let opt = |b: &str| if box_of(b).is_empty() { Ok(0) } else { need(b) };
    let (y, m, d, h, mi, s, ms) = (need("year")?, need("month")?, need("day")?, opt("hours")?, opt("min")?, opt("sec")?, opt("ms")?);
    let days = days_from_civil(y, m, d);
    if !(1..=12).contains(&m) || civil_from_days(days) != (y, m, d) || h > 23 || mi > 59 || s > 59 || ms > 999 {
        return Err("the input row isn't a real date and time".into());
    }
    Ok(days as f64 * 86400.0 + (h * 3600 + mi * 60 + s) as f64 + ms as f64 / 1000.0)
}

/// `t` with years and months added (`sign` = 1) or taken away (-1) on the calendar: the same day
/// and time of day. `row`: the row's place from the top, for the message.
fn on_calendar(t: f64, sign: i64, years: i64, months: i64, row: usize) -> Result<f64, String> {
    let day = (t / 86400.0).floor();
    let of_day = t - day * 86400.0;
    let (y, m, d) = civil_from_days(day as i64);
    let total = y * 12 + (m - 1) + sign * (years * 12 + months);
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let days = days_from_civil(ny, nm, d);
    if civil_from_days(days) != (ny, nm, d) {
        return Err(format!("{ny:04}-{nm:02}-{d:02} doesn't exist. Check modification row {row}."));
    }
    Ok(days as f64 * 86400.0 + of_day)
}

/// Modification row `n` applied to `t`. `wired`: the length wired into it (it replaces the boxes).
/// `place`: the row's place from the top, for messages.
pub fn apply(t: f64, rows: &[(String, String)], n: usize, wired: Option<f64>, place: usize) -> Result<f64, String> {
    let sign = if get(rows, &format!("{n}.sign")) == "+" { 1 } else { -1 };
    if let Some(len) = wired {
        return Ok(t + sign as f64 * len);
    }
    let mut v = [0i64; 7];
    for (slot, b) in v.iter_mut().zip(BOXES) {
        let s = get(rows, &format!("{n}.{b}"));
        if !s.is_empty() {
            *slot = whole(s).ok_or(format!("Modification row {place}: whole numbers only."))?;
        }
    }
    let [years, months, days, h, mi, s, ms] = v;
    let t = if years != 0 || months != 0 { on_calendar(t, sign, years, months, place)? } else { t };
    Ok(t + sign as f64 * ((days * 86400 + h * 3600 + mi * 60 + s) as f64 + ms as f64 / 1000.0))
}

/// A row's ± in seconds: `in` for the input row, else the modification row's number. Empty reads
/// 0. `place`: the row's place from the top, for the message (0: the input row).
pub fn row_error(rows: &[(String, String)], row: &str, place: usize) -> Result<f64, String> {
    let s = get(rows, &format!("{row}.error_s"));
    if s.is_empty() {
        return Ok(0.0);
    }
    let name = if place == 0 { "The input row".to_string() } else { format!("Modification row {place}") };
    s.parse::<f64>().ok().filter(|e| e.is_finite() && *e >= 0.0).ok_or(format!("{name}: its ± is a number of seconds, 0 or more."))
}

/// The error stack: √(Σ ±²), the root sum of squares.
pub fn root_sum_squares(errs: &[f64]) -> f64 {
    errs.iter().map(|e| e * e).sum::<f64>().sqrt()
}

/// What a modification row does, in words: "− 2 min 46.200 s", "+ 1 y 3 mo", "+ 0".
pub fn row_words(rows: &[(String, String)], n: usize, wired: Option<f64>) -> String {
    let sign = if get(rows, &format!("{n}.sign")) == "+" { "+" } else { "−" };
    let v: [i64; 7] = match wired {
        Some(l) => {
            let ms = (l * 1000.0).round() as i64;
            [0, 0, ms / 86_400_000, ms / 3_600_000 % 24, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000]
        }
        None => BOXES.map(|b| whole(get(rows, &format!("{n}.{b}"))).unwrap_or(0)),
    };
    let [y, mo, d, h, mi, s, ms] = v;
    let mut parts: Vec<String> = [(y, "y"), (mo, "mo"), (d, "d"), (h, "h"), (mi, "min")].iter().filter(|(x, _)| *x != 0).map(|(x, u)| format!("{x} {u}")).collect();
    if s != 0 || ms != 0 {
        parts.push(if ms == 0 { format!("{s} s") } else { format!("{s}.{ms:03} s") });
    }
    if parts.is_empty() {
        parts.push("0".into());
    }
    format!("{sign} {}", parts.join(" "))
}

/// The boxes showing the moment `t`.
pub fn time_boxes(t: f64) -> [String; 7] {
    let day = (t / 86400.0).floor();
    let (y, m, d) = civil_from_days(day as i64);
    let ms_of_day = ((t - day * 86400.0) * 1000.0).round() as i64;
    let (s, ms) = (ms_of_day.div_euclid(1000), ms_of_day.rem_euclid(1000));
    [format!("{y:04}"), format!("{m:02}"), format!("{d:02}"), format!("{:02}", s / 3600), format!("{:02}", s / 60 % 60), format!("{:02}", s % 60), format!("{ms:03}")]
}

/// The boxes showing a length: every box, 0 where it has nothing.
pub fn length_boxes(len: f64) -> [String; 7] {
    let ms_all = (len * 1000.0).round() as i64;
    let (s, ms) = (ms_all.div_euclid(1000), ms_all.rem_euclid(1000));
    ["0".into(), "0".into(), (s / 86400).to_string(), (s / 3600 % 24).to_string(), (s / 60 % 60).to_string(), (s % 60).to_string(), format!("{ms:03}")]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timebase::parse_iso8601;

    fn rows(a: &[(&str, &str)]) -> Vec<(String, String)> {
        a.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn t(s: &str) -> f64 {
        parse_iso8601(s).unwrap()
    }

    #[test]
    fn years_move_on_the_calendar() {
        let r = rows(&[("1.sign", "-"), ("1.year", "10")]);
        assert_eq!(apply(t("2036-11-04T12:30:01.250Z"), &r, 1, None, 1), Ok(t("2026-11-04T12:30:01.250Z")));
        let r = rows(&[("1.sign", "+"), ("1.month", "3")]);
        assert_eq!(apply(t("2026-11-04T00:00:00Z"), &r, 1, None, 1), Ok(t("2027-02-04T00:00:00Z")));
    }

    #[test]
    fn days_and_less_are_exact_seconds() {
        let r = rows(&[("1.sign", "+"), ("1.day", "1"), ("1.hours", "2"), ("1.min", "3"), ("1.sec", "4"), ("1.ms", "500")]);
        assert_eq!(apply(1000.0, &r, 1, None, 1), Ok(1000.0 + 86400.0 + 7200.0 + 180.0 + 4.5));
    }

    #[test]
    fn a_date_that_does_not_exist_is_an_error() {
        let r = rows(&[("1.sign", "-"), ("1.year", "2")]);
        let e = apply(t("2020-02-29T10:00:00Z"), &r, 1, None, 1).unwrap_err();
        assert!(e.contains("2018-02-29") && e.contains("row 1"), "{e}");
        let r = rows(&[("1.sign", "-"), ("1.sec", "x")]);
        assert!(apply(0.0, &r, 1, None, 3).unwrap_err().contains("row 3"));
    }

    #[test]
    fn a_wired_length_takes_the_place_of_the_boxes() {
        let r = rows(&[("2.sign", "-"), ("2.year", "9")]);
        assert_eq!(apply(1000.0, &r, 2, Some(60.5), 1), Ok(939.5));
    }

    #[test]
    fn typed_times() {
        let r = rows(&[("in.year", "2026"), ("in.month", "11"), ("in.day", "4"), ("in.hours", "20"), ("in.min", "6"), ("in.sec", "39"), ("in.ms", "312")]);
        assert_eq!(typed(&r), Ok(t("2026-11-04T20:06:39.312Z")));
        assert_eq!(typed(&rows(&[("in.year", "2026"), ("in.month", "11"), ("in.day", "4")])), Ok(t("2026-11-04T00:00:00Z")));
        assert!(typed(&rows(&[("in.year", "2026"), ("in.month", "2"), ("in.day", "30")])).is_err());
        assert!(typed(&rows(&[("in.year", "2026"), ("in.month", "13"), ("in.day", "1")])).is_err());
        assert!(typed(&[]).unwrap_err().contains("wire"));
    }

    #[test]
    fn rows_come_and_go_with_their_ports() {
        let mut r = defaults();
        add_row(&mut r);
        add_row(&mut r);
        assert_eq!(numbers(&r), [1, 2]);
        assert_eq!(spec(&r).inputs.iter().map(|p| p.name).collect::<Vec<_>>(), ["time", "row 1", "row 2", NEXT]);
        remove_row(&mut r, 1);
        assert_eq!(numbers(&r), [2], "the other row keeps its number, so its wire stays");
        add_row(&mut r);
        assert_eq!(numbers(&r), [2, 3]);
    }

    #[test]
    fn boxes_show_a_time_and_a_length() {
        assert_eq!(time_boxes(t("2026-11-04T20:06:39.312Z")), ["2026", "11", "04", "20", "06", "39", "312"].map(String::from));
        assert_eq!(length_boxes(3723.5), ["0", "0", "0", "1", "2", "3", "500"].map(String::from));
    }

    #[test]
    fn the_error_stack_adds_as_squares() {
        assert!((root_sum_squares(&[0.0, 0.02, 0.0, 1.0]) - 1.0002).abs() < 1e-4);
        assert_eq!(format!("{:.3}", root_sum_squares(&[0.0, 0.02, 0.0, 1.0])), "1.000");
        assert_eq!(root_sum_squares(&[3.0, 4.0]), 5.0);
        let r = rows(&[("in.error_s", ""), ("1.sign", "-"), ("1.error_s", "0.5"), ("2.sign", "+"), ("2.error_s", "x")]);
        assert_eq!(row_error(&r, "in", 0), Ok(0.0), "empty reads 0");
        assert_eq!(row_error(&r, "1", 1), Ok(0.5));
        assert!(row_error(&r, "2", 2).unwrap_err().contains("Modification row 2"));
    }

    #[test]
    fn a_new_row_shows_zeros() {
        let mut r = defaults();
        let n = add_row(&mut r);
        assert!(BOXES.iter().all(|b| get(&r, &format!("{n}.{b}")) == "0"));
        assert_eq!(row_error(&r, "1", 1), Ok(0.0));
        assert_eq!(apply(10.0, &r, n, None, 1), Ok(10.0));
        assert_eq!(row_words(&r, n, None), "− 0");
        assert_eq!(row_words(&r, n, Some(166.2)), "− 2 min 46.200 s");
    }

    #[test]
    fn an_old_error_margin_becomes_a_row() {
        let mut r = rows(&[("error_s", "1.5"), ("1.sign", "-"), ("1.hours", "1")]);
        migrate(&mut r);
        assert_eq!(numbers(&r), [1, 2]);
        assert_eq!(get(&r, "2.sign"), "+");
        assert_eq!(row_error(&r, "2", 2), Ok(1.5));
        assert_eq!(apply(10_000.0, &r, 2, None, 2), Ok(10_000.0), "the new row moves nothing");
        let mut z = rows(&[("error_s", "0")]);
        migrate(&mut z);
        assert!(numbers(&z).is_empty() && z.is_empty(), "a ± of 0 needs no row");
    }
}
