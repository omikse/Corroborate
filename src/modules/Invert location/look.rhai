// Invert location's look: drawn by this script. It gives everywhere except the location wired
// in; the area it gives is on its status line.
fn draw(node) {
    let c = if node.inputs[0].wired { "#c8ccd2" } else { "#6b7380" };
    [
        text(node.width / 2.0, 2, "everywhere else", #{ size: 11, color: c, align: "center" }),
    ]
}
