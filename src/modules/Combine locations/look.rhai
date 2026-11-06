// Combine locations' look: the mode as a list to pick from, and what it means. The area it
// gives (or CONTRADICTION) is on its status line.
fn draw(node) {
    let any = node.settings.mode == "any";
    [
        dropdown("mode", 84, 6, 80),
        text(84, 32, if any { "any may hold (OR)" } else { "all must hold (AND)" }, #{ size: 11, color: "#8a929e" }),
    ]
}
