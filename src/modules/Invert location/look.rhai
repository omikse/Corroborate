// Invert location's look: drawn by this script. What the wired location allows becomes what it
// rules out, and the other way round; the area it gives is on its status line.
fn draw(node) {
    let c = if node.inputs[0].wired { "#c8ccd2" } else { "#6b7380" };
    [
        text(node.width / 2.0, 2, "adds <-> subtracts", #{ size: 11, color: c, align: "center" }),
    ]
}
