//! File naming and folder navigation.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use chantedit::document::EXTENSION;

/// Suffix of exported PDFs, which are skipped when browsing a folder.
pub const EXPORT_SUFFIX: &str = " (chords)";

pub fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

pub fn is_document(path: &Path) -> bool {
    has_ext(path, EXTENSION)
}

/// The `.ce` file that belongs to a PDF.
pub fn document_for(pdf: &Path) -> PathBuf {
    pdf.with_extension(EXTENSION)
}

/// Pieces in a folder, in natural order: `.ce` documents, plus PDFs that have
/// no document yet. Exported PDFs are left out.
pub fn pieces_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
        .map(|e| e.path())
        .filter(|p| {
            let stem = p.file_stem().unwrap_or_default().to_string_lossy();
            if is_document(p) {
                return true;
            }
            has_ext(p, "pdf") && !stem.ends_with(EXPORT_SUFFIX) && !document_for(p).exists()
        })
        .collect();
    out.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    out
}

/// Case-insensitive comparison with numbers compared by value
/// ("Hymn 2" < "Hymn 10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.next_if(char::is_ascii_digit) {
                        s.push(c);
                    }
                    s.trim_start_matches('0').to_owned()
                };
                let (n, m) = (take(&mut x), take(&mut y));
                let ord = n.len().cmp(&m.len()).then_with(|| n.cmp(&m));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                x.next();
                y.next();
            }
        }
    }
}

pub fn export_name(stem: &str) -> String {
    format!("{stem}{EXPORT_SUFFIX}.pdf")
}

/// Whether new files can be created in `dir`.
pub fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".chantedit-probe-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

pub fn default_export_dir() -> PathBuf {
    gtk::glib::user_special_dir(gtk::glib::UserDirectory::Documents)
        .unwrap_or_else(gtk::glib::home_dir)
        .join("Chant with chords")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec![
            "Hymn 10.pdf",
            "hymn 2.pdf",
            "Hymn 1.ce",
            "Alleluia.pdf",
            "Hymn 02b.pdf",
        ];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            v,
            [
                "Alleluia.pdf",
                "Hymn 1.ce",
                "hymn 2.pdf",
                "Hymn 02b.pdf",
                "Hymn 10.pdf"
            ]
        );
    }

    #[test]
    fn folder_pieces() {
        let dir = std::env::temp_dir().join(format!("chantedit-files-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for f in [
            "Kyrie.pdf",
            "Kyrie.ce",
            "Gloria.pdf",
            "Gloria (chords).pdf",
            "Credo.ce",
            "notes.txt",
        ] {
            fs::write(dir.join(f), b"").unwrap();
        }
        let names: Vec<String> = pieces_in(&dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Credo.ce", "Gloria.pdf", "Kyrie.ce"]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
