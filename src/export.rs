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
        let parent = self
            .out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // Each export owns its staging file, including simultaneous exports to
        // the same destination. Keep it on the destination filesystem for rename.
        let staging = TempDir::new_in(parent).map_err(|e| e.to_string())?;
        let part = staging.path.join("output.pdf");
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
        Self::new_in(&std::env::temp_dir())
    }

    fn new_in(parent: &Path) -> std::io::Result<TempDir> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        loop {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(".chantedit-{}-{n}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(TempDir { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_exports_preserve_score_and_use_independent_staging() {
        let dir = TempDir::new().unwrap();
        let source = dir.path.join("source.pdf");
        let out = dir.path.join("export.pdf");
        let font = ChordFont::new("Sans", 12.0);
        let page = |text: &str, y| PageItems {
            width: 300.0,
            height: 400.0,
            items: vec![Item {
                text: text.into(),
                cx: 100.0,
                baseline: y,
            }],
        };
        write_overlay(&source, &[page("Original score", 200.0)], &font).unwrap();
        let score = fs::read(&source).unwrap();
        let a = prepare(&score, &[page("Dm", 100.0)], &font, &out).unwrap();
        let b = prepare(&score, &[page("Am", 100.0)], &font, &out).unwrap();
        let unrelated = out.with_extension("pdf.part");
        fs::write(&unrelated, b"unrelated file").unwrap();
        let a = std::thread::spawn(move || a.run());
        let b = std::thread::spawn(move || b.run());
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
        assert_eq!(fs::read(&source).unwrap(), score);
        assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated file");
        let bytes = gtk::glib::Bytes::from_owned(fs::read(out).unwrap());
        let pdf = crate::pdf::open(&bytes).unwrap();
        assert_eq!(pdf.n_pages(), 1);
        let page = pdf.page(0).unwrap();
        assert_eq!(page.size(), (300.0, 400.0));
        let text = page.text().unwrap();
        assert!(text.contains("Original score"));
        assert!(text.contains("Dm") || text.contains("Am"));
    }
}
