//! Public headless diagnostics use the same source parser and preview as GTK.
use std::{fs, process::Command};

#[test]
fn headless_preview_does_not_modify_the_score() {
    let dir = std::env::temp_dir().join(format!("chantedit-gabc-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = "name:Test;\n%%\n(c4) Dó([alt:C]ghg/h.)mi(hi)nus(g.) (::)\n";
    let input = dir.join("score.gabc");
    let output = dir.join("preview");
    fs::write(&input, source).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_chantedit"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .arg("--preview")
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("4 neume anchors, 1 chords"));
    let svg = fs::read_to_string(output.join("staff-1.svg")).unwrap();
    assert!(svg.contains("<svg"));
    assert!(svg.contains("<path"));
    assert_eq!(fs::read_to_string(&input).unwrap(), source);
    fs::write(&input, "not a gabc file").unwrap();
    let bad = Command::new(env!("CARGO_BIN_EXE_chantedit"))
        .arg("--check")
        .arg(&input)
        .output()
        .unwrap();
    assert!(!bad.status.success());
    fs::remove_dir_all(dir).unwrap();
}
