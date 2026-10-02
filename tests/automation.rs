//! Verify the public interface without a graphical session.
use chantedit::document::{Anchor, Document};
use std::fs;
use std::process::Command;

#[test]
fn headless_json_import_export_and_editable_document() {
    let dir = std::env::temp_dir().join(format!("chantedit-automation-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let score = dir.join("score.pdf");
    let out = dir.join("output.pdf");
    let ce = dir.join("output.ce");
    let request = dir.join("request.json");
    let surface = cairo::PdfSurface::new(300.0, 400.0, &score).unwrap();
    let cr = cairo::Context::new(&surface).unwrap();
    cr.show_page().unwrap();
    surface.finish();
    let original = fs::read(&score).unwrap();
    let run = |args: &[&std::ffi::OsStr]| {
        Command::new(env!("CARGO_BIN_EXE_chantedit"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .args(args)
            .output()
            .unwrap()
    };
    let result = run(&["--json".as_ref(), score.as_os_str()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["pages"][0]["width"], 300.0);
    fs::write(
        &request,
        r#"{"version":1,"chords":[{"page":0,"staff":0,"x":100,"text":"C Dm G"}]}"#,
    )
    .unwrap();
    let result = run(&[
        "--apply".as_ref(),
        request.as_os_str(),
        score.as_os_str(),
        "--output".as_ref(),
        out.as_os_str(),
        "--save".as_ref(),
        ce.as_os_str(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let document = Document::load(&ce).unwrap();
    assert_eq!(document.edits.chords[0].text, "C Dm G");
    assert!(matches!(
        document.edits.chords[0].anchor,
        Anchor::Manual { .. }
    ));
    assert_eq!(&document.pdf[..], &original);
    let pdf = chantedit::pdf::open(&gtk::glib::Bytes::from_owned(fs::read(&out).unwrap())).unwrap();
    assert!(pdf.page(0).unwrap().text().unwrap().contains("C Dm G"));
    let bad = run(&[
        "--apply".as_ref(),
        request.as_os_str(),
        score.as_os_str(),
        "--output".as_ref(),
        score.as_os_str(),
    ]);
    assert!(!bad.status.success());
    assert_eq!(fs::read(&score).unwrap(), original);
    fs::write(&request, r#"{"version":1,"chords":[{"page":0,"staff":0,"x":100,"text":"C Dm G"},{"page":0,"staff":0,"x":100,"text":"Am F"}]}"#).unwrap();
    let spread = run(&[
        "--apply".as_ref(),
        request.as_os_str(),
        score.as_os_str(),
        "--spread".as_ref(),
        "--save".as_ref(),
        ce.as_os_str(),
    ]);
    assert!(spread.status.success());
    let document = Document::load(&ce).unwrap();
    assert_eq!(document.edits.chords.len(), 2);
    assert!(document.edits.chords[1].dy.unwrap() < document.edits.chords[0].dy.unwrap());
    fs::remove_dir_all(dir).unwrap();
}
