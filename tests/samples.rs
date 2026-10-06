//! Local real-world GABC fixtures; CHANTEDIT_SAMPLES overrides ~/Downloads.
use chantedit::{
    chant,
    gabc::{Chord, Document},
};
use std::path::PathBuf;

#[test]
fn downloaded_chants_render_and_round_trip_with_chords() {
    let dir = std::env::var_os("CHANTEDIT_SAMPLES")
        .map(PathBuf::from)
        .unwrap_or_else(|| gtk::glib::home_dir().join("Downloads"));
    for name in [
        "hy--o_quam_glorifica--solesmes_1957.1.gabc",
        "hy--salve_festa_dies--solesmes_1957.1.gabc",
    ] {
        let path = dir.join(name);
        if !path.exists() {
            eprintln!("skip {}", path.display());
            continue;
        }
        let source = std::fs::read_to_string(path).unwrap();
        let doc = Document::parse(source.clone()).unwrap();
        assert_eq!(doc.serialize().unwrap(), source);
        // Users can keep editing these local samples; create an unannotated
        // fixture in memory before adding this test's chords.
        let mut doc = Document::parse(doc.music.clone()).unwrap();
        let preview = chant::render(&doc, 720.0).unwrap();
        assert_eq!(preview.positions.len(), doc.anchors.len());
        assert!(preview.positions.len() > 50);
        for anchor in (0..doc.anchors.len()).step_by(5) {
            doc.chords.push(Chord {
                anchor,
                text: "Dm".into(),
                dx: -1.25,
                dy: 2.0,
                automatic_raise: 0.0,
            });
        }
        let saved = doc.serialize().unwrap();
        let reload = Document::parse(saved.clone()).unwrap();
        assert_eq!(doc.chords, reload.chords);
        assert_eq!(doc.music, reload.music);
        assert_eq!(reload.serialize().unwrap(), saved);
        let annotated = chant::render(&reload, 720.0).unwrap();
        for (a, b) in preview.positions.iter().zip(&annotated.positions) {
            assert_eq!((a.page, a.x, a.y), (b.page, b.x, b.y));
        }
    }
}
