//! Draws the score analysis onto page images for inspection.
//!
//!     chantdetect [--size PT] [--out DIR] [--zoom Z] [--dump] FILE.pdf…
//!
//! Blue: staves. Gray: the band chords may move in. Green (orange when tight):
//! the free gap the line is centred in. Red: the chord line. Magenta: note
//! columns. Sample "Am" chords sit on every third note, red when avoidance
//! moved them. `--dump` prints the numbers instead of drawing.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use chantedit::document::{Chord, Document, Settings};
use chantedit::layout::{ChordFont, DocLayout};
use chantedit::{analysis, pdf};
use gtk::{cairo, glib};

struct Args {
    size: f64,
    out: PathBuf,
    zoom: f64,
    dump: bool,
    files: Vec<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        size: 9.0,
        out: "/tmp/chantdetect".into(),
        zoom: 2.0,
        dump: false,
        files: vec![],
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--size" => a.size = value("--size")?.parse().map_err(|_| "bad --size")?,
            "--zoom" => a.zoom = value("--zoom")?.parse().map_err(|_| "bad --zoom")?,
            "--out" => a.out = value("--out")?.into(),
            "--dump" => a.dump = true,
            "-h" | "--help" => return Err(String::new()),
            _ => a.files.push(arg.into()),
        }
    }
    if a.files.is_empty() {
        return Err(String::new());
    }
    Ok(a)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{e}");
            }
            eprintln!("usage: chantdetect [--size PT] [--out DIR] [--zoom Z] [--dump] FILE.pdf…");
            return ExitCode::FAILURE;
        }
    };
    if !args.dump {
        if let Err(e) = std::fs::create_dir_all(&args.out) {
            eprintln!("{}: {e}", args.out.display());
            return ExitCode::FAILURE;
        }
    }
    let mut ok = true;
    for path in &args.files {
        if let Err(e) = process(path, &args) {
            eprintln!("{}: {e}", path.display());
            ok = false;
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn process(path: &Path, args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let doc = pdf::open(&glib::Bytes::from(&bytes))?;
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .replace(' ', "_");
    let font = ChordFont::new("Sans Bold", args.size);
    let settings = Settings {
        size: args.size,
        ..Settings::default()
    };
    for i in 0..doc.n_pages() {
        let page = doc.page(i).ok_or("missing page")?;
        let t = Instant::now();
        let pa = Arc::new(pdf::analyze_page(&page)?);
        let dt = t.elapsed();
        if args.dump {
            dump(&stem, i, &pa, font.cap_height());
            continue;
        }

        // Lay out sample chords exactly like the editor would.
        let mut model = Document::new(Vec::new(), String::new(), settings.clone());
        let mut analyses = vec![None; i as usize];
        analyses.push(Some(pa.clone()));
        let lines = DocLayout::compute(&model, &analyses, &font)
            .lines
            .swap_remove(i as usize);
        for ln in &lines {
            for &x in ln.notes.iter().skip(1).step_by(3) {
                let id = model.edits.new_id();
                model.edits.chords.push(Chord {
                    id,
                    anchor: ln.anchor,
                    x,
                    text: "Am".into(),
                    dy: None,
                });
            }
        }
        let layout = DocLayout::compute(&model, &analyses, &font);

        let (pw, ph) = page.size();
        let z = args.zoom;
        let surface =
            cairo::ImageSurface::create(cairo::Format::Rgb24, (pw * z) as i32, (ph * z) as i32)?;
        let cr = cairo::Context::new(&surface)?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint()?;
        cr.scale(z, z);
        page.render(&cr);
        for (k, st) in pa.staves.iter().enumerate() {
            let f = pa.fit_line(k, font.cap_height());
            let w = st.x1 - st.x0;
            cr.set_source_rgba(0.0, 0.4, 1.0, 0.18);
            cr.rectangle(st.x0, st.top, w, st.bottom - st.top);
            cr.fill()?;
            cr.set_source_rgba(0.6, 0.6, 0.6, 0.15);
            cr.rectangle(st.x0, f.ceil, w, f.floor - f.ceil);
            cr.fill()?;
            if f.tight {
                cr.set_source_rgba(1.0, 0.5, 0.0, 0.3);
            } else {
                cr.set_source_rgba(0.0, 0.8, 0.0, 0.25);
            }
            cr.rectangle(st.x0, f.gap_top, w, f.gap_bottom - f.gap_top);
            cr.fill()?;
            cr.set_source_rgb(1.0, 0.0, 0.0);
            cr.set_line_width(0.4);
            cr.move_to(st.x0, f.y);
            cr.line_to(st.x1, f.y);
            cr.stroke()?;
            cr.set_source_rgba(1.0, 0.0, 1.0, 0.6);
            for &x in st.notes.iter() {
                cr.rectangle(x - 0.5, st.top - 1.5, 1.0, 1.0);
            }
            cr.fill()?;
        }
        for c in &model.edits.chords {
            let Some(p) = layout.placed(c.id) else {
                continue;
            };
            if p.dy != 0.0 {
                cr.set_source_rgb(0.8, 0.0, 0.0);
            } else {
                cr.set_source_rgb(0.0, 0.0, 0.0);
            }
            font.draw(&cr, &c.text, c.x, p.baseline);
        }
        drop(cr);
        let name = args.out.join(format!("{stem}-{}.png", i + 1));
        surface.write_to_png(&mut std::fs::File::create(&name)?)?;
        println!(
            "{stem} p{}: {} staves, {:.0?} -> {}",
            i + 1,
            pa.staves.len(),
            dt,
            name.display()
        );
    }
    Ok(())
}

fn dump(stem: &str, page: i32, pa: &analysis::Page, cap: f64) {
    for (k, st) in pa.staves.iter().enumerate() {
        let f = pa.fit_line(k, cap);
        let notes: Vec<String> = st.notes.iter().map(|n| format!("{n:.2}")).collect();
        println!(
            "{stem} p{} s{k} top={:.2} bot={:.2} sp={:.3} x={:.2}..{:.2} n={} | y={:.2} gap={:.2}..{:.2} ceil={:.2} floor={:.2} tight={} | notes={}",
            page + 1,
            st.top,
            st.bottom,
            st.space,
            st.x0,
            st.x1,
            st.n_lines,
            f.y,
            f.gap_top,
            f.gap_bottom,
            f.ceil,
            f.floor,
            f.tight,
            notes.join(",")
        );
    }
}
