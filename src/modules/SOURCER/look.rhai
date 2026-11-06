// SOURCER's look: outputs it does not give yet are hidden (none until a source is chosen, video
// only for sources with pictures), and the node grows to hold the waveform and the video viewer.
// Inside it is the module's own drawing: source box, system streams, waveform, video.
fn layout(node) {
    let ports = #{};
    let source = false;
    let video = false;
    for p in node.outputs {
        if p.on {
            source = true;
            if p.name == "video" { video = true; }
        } else {
            ports[p.name] = #{ hidden: true };
        }
    }
    let h = if !source { 130 } else if video { 428 } else { 270 };
    #{ size: [420, h], ports: ports }
}
