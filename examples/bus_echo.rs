//! A module program on the data bus (protocol 2), small enough to read as an example and used
//! by the workbench's tests. Its module.toml would say:
//!
//! ```toml
//! id = "bus-echo"
//! label = "Bus echo"
//! category = "Checks"
//! role = "transform"
//!
//! [[input]]
//! name = "place"
//! kind = "location"
//!
//! [[output]]
//! name = "seen"
//! kind = "event"
//!
//! [[output]]
//! name = "place"
//! kind = "location"
//!
//! [[setting]]
//! name = "label"
//! type = "text"
//! default = "got a place"
//!
//! [processing]
//! program = "bus_echo.exe"
//! protocol = 2
//! ```
//!
//! For each place it is given it sends an event (labelled by its `label` setting) and the
//! place back unchanged; it counts the audio it is fed.

use corroborate::core::PortKind;
use corroborate::data::Data;
use corroborate::plugin::{self, In};
use corroborate::timebase::json_escape;
use std::io::{BufRead, Write};

fn main() {
    let label = plugin::settings_args().into_iter().find(|(k, _)| k == "label").map_or("got a place".to_string(), |(_, v)| v);
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", plugin::hello("Bus echo", "sends back what it is given", [210, 200, 90]));
    let _ = out.flush();
    let mut audio_s = 0.0;
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        match plugin::parse_in(&line) {
            Some(In::Value { port, value }) if port == "place" => {
                let detail = match Data::from_json(PortKind::Location, &value) {
                    Ok(Data::Location { area, .. }) => format!("{:.5} of Earth", area.allowed().fraction()),
                    Ok(_) => String::new(),
                    Err(e) => e,
                };
                let event = format!(r#"[{{"t_start":{audio_s},"t_end":{audio_s},"label":"{}","detail":"{}"}}]"#, json_escape(&label), json_escape(&detail));
                let _ = writeln!(out, "{}", plugin::out("seen", &event));
                let _ = writeln!(out, "{}", plugin::out("place", &corroborate::data::write_json(&value)));
                let _ = writeln!(out, "{}", plugin::status(&format!("{label}: {detail}")));
            }
            Some(In::Audio(x)) => audio_s += x.len() as f64 / corroborate::INPUT_SR,
            Some(In::End) => {
                let _ = writeln!(out, "{}", plugin::status(&format!("audio ended after {audio_s:.1} s")));
            }
            _ => {}
        }
        let _ = out.flush();
    }
}
