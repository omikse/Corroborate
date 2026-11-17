# Invert location (transform)

Everywhere except the location wired in, as a plain area: "it was not here" becomes a place you
can combine, map or record. To take one area away from others, use a − row in
[Combine locations](<../Combine locations/README.md>) instead. Status: built.

Input: `location` (required). Output: `location`, its 1° cells turned over.

Files: `module.toml` (ports), `look.rhai` (its look), `mod.rs`, `node.rs` (the area it gives).
Tested in `area.rs::invert_gives_the_rest_of_the_world` and `drive.rs::combine_adds_and_takes_away`.
