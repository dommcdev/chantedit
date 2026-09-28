//! Writes a copy of the score with the chords added.
//!
//! The chords alone are drawn into a transparent PDF with cairo, then stamped
//! onto the original pages with libqpdf's overlay, so the original content is
//! kept exactly as it was (vector music stays vector, text stays text).

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use gtk::cairo;

use crate::layout::ChordFont;
use crate::qpdf;

pub struct Item {
    pub text: String,
    pub cx: f64,
    pub baseline: f64,
}

pub struct PageItems {
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
}

/// An export whose files are prepared; [`Job::run`] does the slow part and may
/// run on any thread.
pub struct Job {
    dir: TempDir,
    out: PathBuf,
}

/// Draws the chord layer (this needs the font, so it happens on the calling
/// thread) and stages the score for [`Job::run`].
pub fn prepare(
    score: &[u8],
    pages: &[PageItems],
    font: &ChordFont,
    out: &Path,
) -> Result<Job, String> {
    if pages.is_empty() {
        return Err("the score has no pages".into());
    }
    let dir = TempDir::new().map_err(|e| format!("cannot create a temporary folder: {e}"))?;
    fs::write(dir.path.join("score.pdf"), score).map_err(|e| e.to_string())?;
    write_overlay(&dir.path.join("chords.pdf"), pages, font)
        .map_err(|e| format!("drawing chords: {e}"))?;
    Ok(Job {
        dir,
        out: out.to_owned(),
    })
}

impl Job {
    pub fn run(self) -> Result<(), String> {
        if let Some(parent) = self.out.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut part = self.out.clone().into_os_string();
        part.push(".part");
        let part = PathBuf::from(part);
        let score = self.dir.path.join("score.pdf");
        let chords = self.dir.path.join("chords.pdf");
        let result = qpdf::run(&[
            OsStr::new("--warning-exit-0"),
            score.as_os_str(),
            OsStr::new("--overlay"),
            chords.as_os_str(),
            OsStr::new("--"),
            part.as_os_str(),
        ])
        .and_then(|()| fs::rename(&part, &self.out).map_err(|e| e.to_string()));
        if result.is_err() {
            let _ = fs::remove_file(&part);
        }
        result
    }
}

fn write_overlay(path: &Path, pages: &[PageItems], font: &ChordFont) -> Result<(), cairo::Error> {
    let surface = cairo::PdfSurface::new(pages[0].width, pages[0].height, path)?;
    let cr = cairo::Context::new(&surface)?;
    for page in pages {
        surface.set_size(page.width, page.height)?;
        cr.set_source_rgb(0.0, 0.0, 0.0);
        for it in &page.items {
            font.draw(&cr, &it.text, it.cx, it.baseline);
        }
        cr.show_page()?;
    }
    drop(cr);
    surface.finish();
    surface.status()
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> std::io::Result<TempDir> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("chantedit-{}-{n}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(TempDir { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
