// Map's look: the location port's name is left out, so the map (drawn by the module's Rust
// code, as there is no draw() here) takes the node's whole width.
fn layout(node) {
    #{ ports: #{ location: #{ label: "hidden" } } }
}
