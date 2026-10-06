//! Headless preview diagnostics, sharing exactly the GUI's parser and renderer.

use crate::{chant, gabc};
use std::path::PathBuf;

pub fn run() -> Option<gtk::glib::ExitCode> {
    let mut args = std::env::args_os().skip(1);
    let command = args.next()?;
    if command == "--help" || command == "-h" {
        println!(
            "ChantEdit — add chords to GABC chant files\n\nchantedit [score.gabc]\nchantedit --check score.gabc\nchantedit --preview score.gabc --output directory\n\n--check verifies parsing, preview anchors, and a lossless round trip.\n--preview also writes one SVG per staff for inspecting the offline renderer."
        );
        return Some(gtk::glib::ExitCode::SUCCESS);
    }
    if command != "--check" && command != "--preview" {
        return None;
    }
    let result = (|| -> Result<(), String> {
        let path = args.next().ok_or("Expected a .gabc file")?;
        let doc =
            gabc::Document::parse(std::fs::read_to_string(&path).map_err(|e| e.to_string())?)?;
        let original = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if doc.serialize()? != original {
            return Err("Lossless round trip failed".into());
        }
        let preview = chant::render(&doc, 720.0)?;
        if command == "--preview" {
            if args.next().as_deref() != Some(std::ffi::OsStr::new("--output")) {
                return Err("Expected --output directory".into());
            }
            let dir = PathBuf::from(args.next().ok_or("Expected an output directory")?);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            for (i, page) in preview.pages.iter().enumerate() {
                std::fs::write(dir.join(format!("staff-{}.svg", i + 1)), &page.svg)
                    .map_err(|e| e.to_string())?;
            }
        }
        if args.next().is_some() {
            return Err("Unexpected arguments".into());
        }
        println!(
            "{}: {} neume anchors, {} chords, {} staves; lossless round trip OK",
            doc.name,
            doc.anchors.len(),
            doc.chords.len(),
            preview.pages.len()
        );
        Ok(())
    })();
    Some(match result {
        Ok(()) => gtk::glib::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("chantedit: {e}");
            gtk::glib::ExitCode::FAILURE
        }
    })
}
