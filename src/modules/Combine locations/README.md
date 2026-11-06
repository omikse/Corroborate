# Combine locations (transform)

Joins up to four locations. In "all" mode the result is where every input holds; in "any" mode it is
where at least one holds. The mode is the setting `mode` (all or any), picked in the node's list;
each change is kept in the result file. Inputs: `location 1` to `location 4`. Output: `location`.

Files: `module.toml` (ports, setting, size), `look.rhai` (the list and its meaning), `mod.rs` (the
mode's names), `node.rs` (the area it gives).
