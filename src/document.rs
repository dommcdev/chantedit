//! The `.ce` document: a zip archive holding the original score and the chords.
//!
//! ```text
//! mimetype      "application/x-chantedit", stored first and uncompressed
//! chords.json   settings, chords and line edits
//! score.pdf     the untouched original PDF
//! ```
//!
//! Because the score travels inside the file, a `.ce` keeps working after the
//! PDF is moved, renamed or lost.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const EXTENSION: &str = "ce";
pub const MIME_TYPE: &str = "application/x-chantedit";
const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Pango family and style, e.g. "Sans Bold".
    pub font: String,
    /// Chord text size in points.
    pub size: f64,
    pub auto_avoid: bool,
    pub snap_notes: bool,
    /// Added to every detected line, in points (negative is up).
    pub line_offset: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            font: "Sans Bold".into(),
            size: 9.0,
            auto_avoid: true,
            snap_notes: true,
            line_offset: 0.0,
        }
    }
}

/// The chord line a chord (or the cursor) sits on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Anchor {
    /// The detected line above a staff, identified by the staff's top line.
    Staff { page: usize, staff_top: f64 },
    /// A line added by hand.
    Manual { id: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chord {
    pub id: u32,
    pub anchor: Anchor,
    /// Horizontal centre in points.
    pub x: f64,
    pub text: String,
    /// Manual vertical offset from the line; `None` places it automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dy: Option<f64>,
}

/// A tweak to a detected line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaffLineEdit {
    pub page: usize,
    pub staff_top: f64,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualLine {
    pub id: u32,
    pub page: usize,
    pub y: f64,
    pub x0: f64,
    pub x1: f64,
}

/// The undoable part of a document.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Edits {
    pub chords: Vec<Chord>,
    pub staff_lines: Vec<StaffLineEdit>,
    pub manual_lines: Vec<ManualLine>,
    pub next_id: u32,
}

impl Edits {
    pub fn chord(&self, id: u32) -> Option<&Chord> {
        self.chords.iter().find(|c| c.id == id)
    }

    pub fn chord_mut(&mut self, id: u32) -> Option<&mut Chord> {
        self.chords.iter_mut().find(|c| c.id == id)
    }

    pub fn new_id(&mut self) -> u32 {
        self.next_id = self.next_id.max(1);
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn manual_line(&self, id: u32) -> Option<&ManualLine> {
        self.manual_lines.iter().find(|l| l.id == id)
    }

    /// The edit for the detected line of the staff at `staff_top`.
    pub fn staff_line(
        &self,
        page: usize,
        staff_top: f64,
        tolerance: f64,
    ) -> Option<&StaffLineEdit> {
        self.staff_lines
            .iter()
            .filter(|e| e.page == page && (e.staff_top - staff_top).abs() <= tolerance)
            .min_by(|a, b| {
                (a.staff_top - staff_top)
                    .abs()
                    .total_cmp(&(b.staff_top - staff_top).abs())
            })
    }

    pub fn staff_line_mut(
        &mut self,
        page: usize,
        staff_top: f64,
        tolerance: f64,
    ) -> &mut StaffLineEdit {
        let found = self
            .staff_lines
            .iter()
            .position(|e| e.page == page && (e.staff_top - staff_top).abs() <= tolerance);
        let i = found.unwrap_or_else(|| {
            self.staff_lines.push(StaffLineEdit {
                page,
                staff_top,
                offset: 0.0,
                hidden: false,
            });
            self.staff_lines.len() - 1
        });
        &mut self.staff_lines[i]
    }

    fn repair(&mut self) {
        let max_id = self
            .chords
            .iter()
            .map(|c| c.id)
            .chain(self.manual_lines.iter().map(|l| l.id))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max_id + 1);
        self.staff_lines.retain(|e| e.offset != 0.0 || e.hidden);
    }
}

pub struct Document {
    /// The original PDF, byte for byte.
    pub pdf: gtk::glib::Bytes,
    /// File name of the PDF the document was created from.
    pub source_name: String,
    pub settings: Settings,
    pub edits: Edits,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    source_name: String,
    settings: Settings,
    #[serde(flatten)]
    edits: Edits,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Zip(zip::result::ZipError),
    Json(serde_json::Error),
    NotADocument,
    TooNew(u32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => e.fmt(f),
            Error::Zip(e) => write!(f, "damaged file ({e})"),
            Error::Json(e) => write!(f, "damaged chord data ({e})"),
            Error::NotADocument => f.write_str("not a ChantEdit document"),
            Error::TooNew(v) => write!(f, "made by a newer version of ChantEdit (format {v})"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Error::Zip(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

impl Document {
    pub fn new(pdf: Vec<u8>, source_name: String, settings: Settings) -> Document {
        Document {
            pdf: gtk::glib::Bytes::from_owned(pdf),
            source_name,
            settings,
            edits: Edits {
                next_id: 1,
                ..Edits::default()
            },
        }
    }

    pub fn load(path: &Path) -> Result<Document, Error> {
        let mut zip = ZipArchive::new(BufReader::new(File::open(path)?)).map_err(|e| match e {
            zip::result::ZipError::InvalidArchive(_) => Error::NotADocument,
            e => Error::Zip(e),
        })?;
        let manifest: Manifest = match zip.by_name("chords.json") {
            Ok(f) => serde_json::from_reader(BufReader::new(f))?,
            Err(zip::result::ZipError::FileNotFound) => return Err(Error::NotADocument),
            Err(e) => return Err(e.into()),
        };
        if manifest.format > FORMAT {
            return Err(Error::TooNew(manifest.format));
        }
        let mut pdf = Vec::new();
        zip.by_name("score.pdf")?.read_to_end(&mut pdf)?;
        let mut edits = manifest.edits;
        edits.repair();
        Ok(Document {
            pdf: gtk::glib::Bytes::from_owned(pdf),
            source_name: manifest.source_name,
            settings: manifest.settings,
            edits,
        })
    }

    /// Writes the document atomically: a crash never leaves a half-written file.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = path
            .file_name()
            .ok_or(Error::NotADocument)?
            .to_string_lossy();
        let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
        let result = self
            .write_to(&tmp)
            .and_then(|()| Ok(fs::rename(&tmp, path)?));
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    fn write_to(&self, path: &Path) -> Result<(), Error> {
        let file = File::create(path)?;
        let mut zip = ZipWriter::new(BufWriter::new(file));
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("mimetype", stored)?;
        zip.write_all(MIME_TYPE.as_bytes())?;

        let mut edits = self.edits.clone();
        edits.repair();
        let manifest = Manifest {
            format: FORMAT,
            source_name: self.source_name.clone(),
            settings: self.settings.clone(),
            edits,
        };
        zip.start_file("chords.json", deflated)?;
        serde_json::to_writer_pretty(&mut zip, &manifest)?;

        // PDFs are compressed already.
        zip.start_file("score.pdf", stored)?;
        zip.write_all(&self.pdf)?;

        let mut out = zip.finish()?;
        out.flush()?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("chantedit-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Te Deum.ce");

        let mut doc = Document::new(
            b"%PDF-1.4 fake".to_vec(),
            "Te Deum.pdf".into(),
            Settings::default(),
        );
        let line = doc.edits.new_id();
        doc.edits.manual_lines.push(ManualLine {
            id: line,
            page: 0,
            y: 50.0,
            x0: 10.0,
            x1: 500.0,
        });
        let id = doc.edits.new_id();
        doc.edits.chords.push(Chord {
            id,
            anchor: Anchor::Staff {
                page: 1,
                staff_top: 123.4,
            },
            x: 99.5,
            text: "Dm7".into(),
            dy: Some(-1.5),
        });
        doc.edits.staff_line_mut(1, 123.4, 1.0).offset = 2.0;
        doc.settings.size = 7.5;
        doc.save(&path).unwrap();

        let back = Document::load(&path).unwrap();
        assert_eq!(&back.pdf[..], &doc.pdf[..]);
        assert_eq!(back.source_name, "Te Deum.pdf");
        assert_eq!(back.settings, doc.settings);
        assert_eq!(back.edits, doc.edits);

        // The mimetype entry comes first, uncompressed, like ODF.
        let raw = fs::read(&path).unwrap();
        assert_eq!(&raw[30..38], b"mimetype");
        assert_eq!(&raw[38..38 + MIME_TYPE.len()], MIME_TYPE.as_bytes());

        fs::write(dir.join("bad.ce"), b"hello").unwrap();
        assert!(matches!(
            Document::load(&dir.join("bad.ce")),
            Err(Error::NotADocument)
        ));
        fs::remove_dir_all(&dir).unwrap();
    }
}
