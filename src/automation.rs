//! Headless, versioned JSON interface. No display required.
use crate::document::{Anchor, Chord, Document, ManualLine, Settings};
use crate::layout::{ChordFont, DocLayout};
use crate::{analysis, export, pdf};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Serialize)]
pub struct Score {
    pub version: u32,
    pub pages: Vec<Page>,
    pub notes: Vec<Note>,
}
#[derive(Serialize)]
pub struct Page {
    pub width: f64,
    pub height: f64,
    pub staves: Vec<Staff>,
}
#[derive(Serialize)]
pub struct Staff {
    pub top: f64,
    pub x0: f64,
    pub x1: f64,
}
/// Note/neume columns, not pitch transcription. All indices are zero-based.
#[derive(Serialize)]
pub struct Note {
    pub index: usize,
    pub page: usize,
    pub staff: usize,
    pub x: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub chords: Vec<Placement>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub page: usize,
    pub staff: usize,
    pub x: f64,
    pub text: String,
}

pub fn inspect(analyses: &[Option<Arc<analysis::Page>>]) -> Score {
    let mut notes = Vec::new();
    let pages = analyses
        .iter()
        .enumerate()
        .map(|(page, a)| {
            let a = a.as_ref().expect("complete analysis");
            let staves = a
                .staves
                .iter()
                .enumerate()
                .map(|(staff, st)| {
                    for &x in st.notes.iter() {
                        notes.push(Note {
                            index: notes.len(),
                            page,
                            staff,
                            x,
                        });
                    }
                    Staff {
                        top: st.top,
                        x0: st.x0,
                        x1: st.x1,
                    }
                })
                .collect();
            Page {
                width: a.width,
                height: a.height,
                staves,
            }
        })
        .collect();
    Score {
        version: 1,
        pages,
        notes,
    }
}

fn different(input: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let absolute = |p: &Path| -> std::io::Result<PathBuf> {
        if p.exists() {
            p.canonicalize()
        } else {
            Ok(p.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .canonicalize()?
                .join(
                    p.file_name()
                        .ok_or_else(|| std::io::Error::other("missing filename"))?,
                ))
        }
    };
    if absolute(input)? == absolute(output)? {
        return Err("output must not overwrite an input or another output".into());
    }
    #[cfg(unix)]
    if input.exists() && output.exists() {
        use std::os::unix::fs::MetadataExt;
        let a = input.metadata()?;
        let b = output.metadata()?;
        if a.dev() == b.dev() && a.ino() == b.ino() {
            return Err("output aliases an input or another output".into());
        }
    }
    Ok(())
}

fn process(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (mut input, mut apply, mut output, mut save) = (None, None, None, None);
    let mut json = false;
    let mut spread = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--spread" => spread = true,
            "--apply" => {
                apply = Some(PathBuf::from(
                    it.next().ok_or("--apply needs a JSON file (or -)")?,
                ))
            }
            "--output" => {
                output = Some(PathBuf::from(it.next().ok_or("--output needs a PDF path")?))
            }
            "--save" => save = Some(PathBuf::from(it.next().ok_or("--save needs a .ce path")?)),
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}").into()),
            _ if input.is_none() => input = Some(PathBuf::from(arg)),
            _ => return Err("only one input score is supported".into()),
        }
    }
    let input = input.ok_or(
        "usage: chantedit --json SCORE.pdf | --apply JSON SCORE.pdf --output OUT.pdf --save OUT.ce",
    )?;
    if apply.is_none() && (!json || output.is_some() || save.is_some()) {
        return Err("use --apply to write a PDF or document".into());
    }
    if apply.is_some() && output.is_none() && save.is_none() {
        return Err("--apply requires --output and/or --save".into());
    }
    for path in [&output, &save].into_iter().flatten() {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        different(&input, path)?;
        if let Some(ref request) = apply
            && request != Path::new("-")
        {
            different(request, path)?;
        }
    }
    if let (Some(out), Some(ce)) = (&output, &save) {
        different(out, ce)?;
    }
    let mut model = if input.extension().is_some_and(|e| e == "ce") {
        Document::load(&input)?
    } else {
        Document::new(
            std::fs::read(&input)?,
            input.file_name().unwrap().to_string_lossy().into_owned(),
            Settings::default(),
        )
    };
    let doc = pdf::open(&model.pdf)?;
    let analyses: Vec<_> = (0..doc.n_pages())
        .map(|i| pdf::analyze_page(&doc.page(i).expect("PDF page")).map(|a| Some(Arc::new(a))))
        .collect::<Result<_, _>>()?;
    if let Some(path) = apply {
        let first_import = model.edits.chords.len();
        let request: Request = if path == Path::new("-") {
            serde_json::from_reader(std::io::stdin().lock())?
        } else {
            serde_json::from_slice(&std::fs::read(path)?)?
        };
        if request.version != 1 {
            return Err("unsupported request version".into());
        }
        for c in request.chords {
            let a = analyses
                .get(c.page)
                .and_then(Option::as_ref)
                .ok_or("invalid page index")?;
            if !c.x.is_finite() || !(0.0..=a.width).contains(&c.x) || c.text.trim().is_empty() {
                return Err("chords need nonempty text and a finite x inside the page".into());
            }
            let anchor = if let Some(st) = a.staves.get(c.staff) {
                Anchor::Staff {
                    page: c.page,
                    staff_top: st.top,
                }
            } else if a.staves.is_empty() && c.staff == 0 {
                let id = model
                    .edits
                    .manual_lines
                    .iter()
                    .find(|l| l.page == c.page)
                    .map(|l| l.id)
                    .unwrap_or_else(|| {
                        let id = model.edits.new_id();
                        model.edits.manual_lines.push(ManualLine {
                            id,
                            page: c.page,
                            y: 36.0,
                            x0: 20.0,
                            x1: a.width - 20.0,
                        });
                        id
                    });
                Anchor::Manual { id }
            } else {
                return Err("invalid staff index".into());
            };
            let id = model.edits.new_id();
            model.edits.chords.push(Chord {
                id,
                anchor,
                x: c.x,
                text: c.text,
                dy: None,
            });
        }
        let font = ChordFont::new(&model.settings.font, model.settings.size);
        if spread {
            spread_import(&mut model, &analyses, &font, first_import);
        }
        let layout = DocLayout::compute(&model, &analyses, &font);
        if let Some(path) = output {
            let mut pages: Vec<_> = pdf::page_sizes(&doc)
                .into_iter()
                .map(|(width, height)| export::PageItems {
                    width,
                    height,
                    items: vec![],
                })
                .collect();
            for c in &model.edits.chords {
                if let Some(p) = layout.placed(c.id) {
                    pages[p.line.page].items.push(export::Item {
                        text: c.text.clone(),
                        cx: c.x,
                        baseline: p.baseline,
                    });
                }
            }
            export::prepare(&model.pdf, &pages, &font, &path)?.run()?;
        }
        if let Some(path) = save {
            model.save(&path)?;
        }
    }
    if json {
        serde_json::to_writer(std::io::stdout().lock(), &inspect(&analyses))?;
    }
    Ok(())
}

/// Keep dense imported strings readable without deleting any chord labels.
/// Overflow is wrapped, and overlapping items move to rows above their anchor.
fn spread_import(
    model: &mut Document,
    analyses: &[Option<Arc<analysis::Page>>],
    font: &ChordFont,
    first: usize,
) {
    let layout = DocLayout::compute(model, analyses, font);
    let mut occupied: Vec<_> = model.edits.chords[..first]
        .iter()
        .filter_map(|c| layout.placed(c.id).copied())
        .collect();
    let imported = model.edits.chords.split_off(first);
    for c in imported {
        let Some(placed) = layout.placed(c.id) else {
            model.edits.chords.push(c);
            continue;
        };
        let line = &layout.lines[placed.line.page][placed.line.index];
        let page_width = analyses[placed.line.page].as_ref().unwrap().width;
        let max_width = (line.x1 - line.x0).max(30.0).min(page_width - 20.0);
        let mut chunks = Vec::new();
        let mut text = String::new();
        for word in c.text.split_whitespace() {
            let candidate = if text.is_empty() {
                word.to_owned()
            } else {
                format!("{text} {word}")
            };
            if !text.is_empty() && font.text(&candidate).metrics.width > max_width {
                chunks.push(std::mem::take(&mut text));
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(word);
        }
        if !text.is_empty() {
            chunks.push(text);
        }
        for (i, text) in chunks.into_iter().enumerate() {
            let metrics = font.text(&text).metrics;
            let half = metrics.width / 2.0;
            let x = c.x.clamp(
                half.min(page_width / 2.0) + 1.0,
                (page_width - half - 1.0).max(page_width / 2.0 + 1.0),
            );
            let mut dy = placed.dy;
            let mut bounds = placed.bounds;
            bounds.x0 = x - half;
            bounds.x1 = x + half;
            // Same-page bounding boxes, including existing manually placed chords.
            while occupied.iter().any(|p| {
                p.line.page == placed.line.page
                    && bounds.x0 < p.bounds.x1 + 2.0
                    && bounds.x1 + 2.0 > p.bounds.x0
                    && bounds.y0 < p.bounds.y1 + 2.0
                    && bounds.y1 + 2.0 > p.bounds.y0
            }) {
                let step = font.size() * 1.25;
                dy -= step;
                bounds.y0 -= step;
                bounds.y1 -= step;
            }
            occupied.push(crate::layout::Placed {
                bounds,
                dy,
                ..*placed
            });
            let id = if i == 0 { c.id } else { model.edits.new_id() };
            model.edits.chords.push(Chord {
                id,
                text,
                x,
                dy: Some(dy),
                anchor: c.anchor,
            });
        }
    }
}

pub fn run_cli() -> Option<gtk::glib::ExitCode> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Usage: chantedit [SCORE.pdf|DOCUMENT.ce]\n\
                  Headless analysis: chantedit --json SCORE.pdf\n\
                  Headless editing: chantedit --apply JSON SCORE.pdf [--output OUT.pdf] [--save OUT.ce] [--spread]\n\
                  Use --apply - to read JSON from stdin; --json can also accompany --apply.\n\
                  JSON version 1; page/staff/note indices are zero-based; coordinates are PDF points.\n\
                  Import: {{\"version\":1,\"chords\":[{{\"page\":0,\"staff\":0,\"x\":120,\"text\":\"C Dm\"}}]}}\n\
                  --spread wraps/staggers dense imports; manual review is still required."
        );
        return Some(gtk::glib::ExitCode::SUCCESS);
    }
    if !args.iter().any(|a| a == "--json" || a == "--apply") {
        return None;
    }
    Some(match process(&args) {
        Ok(()) => gtk::glib::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("chantedit: {e}");
            gtk::glib::ExitCode::FAILURE
        }
    })
}
