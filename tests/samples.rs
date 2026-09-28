//! Detection against the chant scores in `~/Downloads/Chant` (or
//! `CHANTEDIT_SAMPLES`). Skipped when the folder is not there.

use std::path::{Path, PathBuf};

use chantedit::{analysis, pdf};
use gtk::glib;

fn sample_dir() -> PathBuf {
    std::env::var_os("CHANTEDIT_SAMPLES")
        .map(PathBuf::from)
        .unwrap_or_else(|| glib::home_dir().join("Downloads/Chant"))
}

fn analyse(path: &Path) -> Vec<analysis::Page> {
    let bytes = glib::Bytes::from_owned(std::fs::read(path).unwrap());
    let pdf = pdf::open(&bytes).unwrap();
    (0..pdf.n_pages())
        .map(|i| {
            let page = pdf.page(i).expect("page");
            pdf::analyze_page(&page).unwrap()
        })
        .collect()
}

fn check(name: &str, want: &[usize]) {
    let path = sample_dir().join(name);
    if !path.exists() {
        eprintln!("skip {name}: not in {}", sample_dir().display());
        return;
    }
    let pages = analyse(&path);
    assert_eq!(pages.len(), want.len(), "{name}: page count");
    for (i, (page, &n)) in pages.iter().zip(want).enumerate() {
        assert_eq!(
            page.staves.len(),
            n,
            "{name} page {}: {} staves, want {n}",
            i + 1,
            page.staves.len()
        );
        for (k, st) in page.staves.iter().enumerate() {
            assert_eq!(
                st.n_lines,
                4,
                "{name} p{} staff {k}: {} lines",
                i + 1,
                st.n_lines
            );
            let f = page.fit_line(k, 6.5);
            assert!(
                f.y < st.top && st.top - f.y <= 4.0 * st.space,
                "{name} p{} staff {k}: line at {:.1}, staff top {:.1}",
                i + 1,
                f.y,
                st.top
            );
            assert!(
                st.notes.len() >= 5,
                "{name} p{} staff {k}: only {} notes",
                i + 1,
                st.notes.len()
            );
        }
    }
}

#[test]
fn te_deum() {
    check("Te Deum (Simple tone).pdf", &[7, 7, 7, 7, 0]);
}

#[test]
fn cantate() {
    check("Cantate Domino 2.pdf", &[8]);
}

#[test]
fn mass() {
    check("Mass5SAE_lg.pdf", &[7, 4, 6, 7, 4]);
}
