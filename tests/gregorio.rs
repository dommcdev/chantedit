//! Optional authoritative engraving regression. Docker is only used by this
//! test, never by the editor. Set CHANTEDIT_TEX_IMAGE to select an installed image.

use chantedit::gabc::{Chord, Document};
use std::{fs, process::Command};

#[test]
#[ignore = "requires an installed TeX Live Docker image"]
fn annotations_preserve_gregorio_glyphs_and_horizontal_positions() {
    let dir = std::env::temp_dir().join(format!("chantedit-gregorio-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = std::env::var_os("CHANTEDIT_GREGORIO_SOURCE")
        .map(|path| fs::read_to_string(path).unwrap())
        .unwrap_or_else(|| "name:Spacing test;\n%%\n(c4) Dó(ghg/h!i)mi(hi)nus(g.) (::)\n".into());
    fs::write(dir.join("base.gabc"), &source).unwrap();
    let mut doc = Document::parse(source).unwrap();
    for anchor in 0..doc.anchors.len() {
        doc.chords.push(Chord {
            anchor,
            text: if anchor == 0 { "C♯m7/G♭" } else { "Dm" }.into(),
            dx: -1.25,
            dy: 2.5,
            automatic_raise: if anchor == 1 { 12.6 } else { 0.0 },
        });
    }
    fs::write(dir.join("chords.gabc"), doc.serialize().unwrap()).unwrap();
    let tex = r#"\documentclass{article}
\usepackage{fontspec}
\usepackage{gregoriotex}
\newwrite\positions
\immediate\openout\positions=\jobname.positions
\let\OriginalGreGlyph\GreGlyph
\def\GreGlyph#1#2#3#4#5#6#7{\savepos\write\positions{\the\lastxpos}\OriginalGreGlyph{#1}{#2}{#3}{#4}{#5}{#6}{#7}}
\begin{document}
\input{SCORE.gtex}
\end{document}
"#;
    for name in ["base", "chords"] {
        fs::write(dir.join(format!("{name}.tex")), tex.replace("SCORE", name)).unwrap();
    }
    let image = std::env::var("CHANTEDIT_TEX_IMAGE").unwrap_or("texlive/texlive:latest".into());
    let result = Command::new("docker").args(["run", "--rm", "--network", "none", "-v"])
        .arg(format!("{}:/work", dir.display())).args(["-w", "/work", &image, "sh", "-c",
            "export PATH=/usr/local/texlive/2026/bin/x86_64-linux:$PATH; gregorio base.gabc && gregorio chords.gabc && lualatex -interaction=nonstopmode -halt-on-error base.tex && lualatex -interaction=nonstopmode -halt-on-error base.tex && lualatex -interaction=nonstopmode -halt-on-error chords.tex && lualatex -interaction=nonstopmode -halt-on-error chords.tex"])
        .output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let glyphs = |name| {
        fs::read_to_string(dir.join(format!("{name}.gtex")))
            .unwrap()
            .lines()
            .filter(|l| l.starts_with("\\GreGlyph{"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        glyphs("base"),
        glyphs("chords"),
        "Chord tags changed the chant glyphs"
    );
    let base = fs::read_to_string(dir.join("base.positions")).unwrap();
    let annotated = fs::read_to_string(dir.join("chords.positions")).unwrap();
    assert!(
        !base.is_empty(),
        "Position instrumentation must record glyphs"
    );
    assert_eq!(
        base, annotated,
        "Chord tags changed horizontal glyph positions"
    );
    let missing = |name| {
        fs::read_to_string(dir.join(format!("{name}.log")))
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("Missing character:"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        missing("base"),
        missing("chords"),
        "Annotations introduced missing font glyphs"
    );
    fs::remove_dir_all(dir).unwrap();
}
