//! Turns analysis results and document edits into chord lines and chord
//! positions. Shared by the editor and PDF export so both place text
//! identically.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gtk::{cairo, pango, prelude::*};

use crate::analysis;
use crate::document::{Anchor, Chord, Document, Settings};

// ------------------------------------------------------------------------ font

/// Text extents relative to the left edge of the text and its baseline.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// Advance width.
    pub width: f64,
    /// Top of the layout to the baseline.
    pub ascent: f64,
    pub ink_x0: f64,
    pub ink_x1: f64,
    pub ink_y0: f64,
    pub ink_y1: f64,
}

pub struct Text {
    pub layout: pango::Layout,
    pub metrics: Metrics,
}

/// Chord text at a fixed size, measured in points (1 pt = 1 user unit) with
/// hinting off so on-screen and exported text are laid out identically.
pub struct ChordFont {
    size: f64,
    cap_height: f64,
    context: pango::Context,
    desc: pango::FontDescription,
    cache: RefCell<HashMap<String, Rc<Text>>>,
}

impl ChordFont {
    pub fn new(name: &str, size: f64) -> ChordFont {
        let context = pangocairo::FontMap::default().create_context();
        let mut options = cairo::FontOptions::new().expect("cairo font options");
        options.set_hint_metrics(cairo::HintMetrics::Off);
        options.set_hint_style(cairo::HintStyle::None);
        pangocairo::functions::context_set_font_options(&context, Some(&options));
        context.set_round_glyph_positions(false);
        let mut desc = pango::FontDescription::from_string(name);
        desc.set_absolute_size(size * f64::from(pango::SCALE));
        let mut font = ChordFont {
            size,
            cap_height: 0.0,
            context,
            desc,
            cache: RefCell::default(),
        };
        let cap = -font.text("CEGH").metrics.ink_y0;
        font.cap_height = if cap > 0.0 { cap } else { size * 0.72 };
        font
    }

    pub fn size(&self) -> f64 {
        self.size
    }

    /// Height of capital letters above the baseline.
    pub fn cap_height(&self) -> f64 {
        self.cap_height
    }

    pub fn text(&self, s: &str) -> Rc<Text> {
        if let Some(t) = self.cache.borrow().get(s) {
            return t.clone();
        }
        let layout = pango::Layout::new(&self.context);
        layout.set_font_description(Some(&self.desc));
        layout.set_text(s);
        let (ink, logical) = layout.extents();
        let u = |v: i32| f64::from(v) / f64::from(pango::SCALE);
        let base = u(layout.baseline());
        let metrics = Metrics {
            width: u(logical.width()),
            ascent: base,
            ink_x0: u(ink.x()),
            ink_x1: u(ink.x() + ink.width()),
            ink_y0: u(ink.y()) - base,
            ink_y1: u(ink.y() + ink.height()) - base,
        };
        let t = Rc::new(Text { layout, metrics });
        self.cache.borrow_mut().insert(s.to_owned(), t.clone());
        t
    }

    /// Top-left corner of the layout for text centred on `cx` at `baseline`.
    pub fn origin(&self, t: &Text, cx: f64, baseline: f64) -> (f64, f64) {
        (cx - t.metrics.width / 2.0, baseline - t.metrics.ascent)
    }

    pub fn draw(&self, cr: &cairo::Context, s: &str, cx: f64, baseline: f64) {
        let t = self.text(s);
        let (x, y) = self.origin(&t, cx, baseline);
        cr.move_to(x, y);
        pangocairo::functions::show_layout(cr, &t.layout);
    }
}

// ----------------------------------------------------------------------- lines

#[derive(Debug, Clone)]
pub struct Line {
    pub anchor: Anchor,
    pub page: usize,
    /// Index of the staff in the page analysis, `None` for manual lines.
    pub staff: Option<usize>,
    /// Vertical centre of the chord text.
    pub y: f64,
    pub x0: f64,
    pub x1: f64,
    /// Chords may move between `ceil` and `floor` to avoid the music.
    pub ceil: f64,
    pub floor: f64,
    pub tight: bool,
    /// Adjustment applied to this line by hand.
    pub offset: f64,
    pub notes: Arc<[f64]>,
    /// How far a stored staff position may drift and still match this line.
    pub tolerance: f64,
}

impl Line {
    /// The note closest to `x` within `max_dist`, or `x` itself.
    pub fn nearest_note(&self, x: f64, max_dist: f64) -> f64 {
        self.notes
            .iter()
            .copied()
            .filter(|n| (n - x).abs() <= max_dist)
            .min_by(|a, b| (a - x).abs().total_cmp(&(b - x).abs()))
            .unwrap_or(x)
    }

    /// The first note more than `eps` right (`dir > 0`) or left of `x`.
    pub fn next_note(&self, x: f64, dir: i32, eps: f64) -> Option<f64> {
        if dir > 0 {
            self.notes.iter().copied().find(|&n| n > x + eps)
        } else {
            self.notes.iter().rev().copied().find(|&n| n < x - eps)
        }
    }
}

/// Stored staff positions match a detected staff within this distance.
fn staff_tolerance(space: f64) -> f64 {
    (0.75 * space).max(2.0)
}

/// Computes the chord lines of one page, sorted top to bottom.
pub fn page_lines(
    page: usize,
    analysis: Option<&analysis::Page>,
    doc: &Document,
    font: &ChordFont,
) -> Vec<Line> {
    let cap = font.cap_height();
    let mut out = Vec::new();
    if let Some(a) = analysis {
        for (k, st) in a.staves.iter().enumerate() {
            let tolerance = staff_tolerance(st.space);
            let edit = doc.edits.staff_line(page, st.top, tolerance);
            if edit.is_some_and(|e| e.hidden) {
                continue;
            }
            let offset = edit.map_or(0.0, |e| e.offset);
            let fit = a.fit_line(k, cap);
            let y = fit.y + doc.settings.line_offset + offset;
            out.push(Line {
                anchor: Anchor::Staff {
                    page,
                    staff_top: st.top,
                },
                page,
                staff: Some(k),
                y,
                x0: st.x0,
                x1: st.x1,
                // A line moved by hand still gets some room to dodge notes.
                ceil: fit.ceil.min(y - 1.5 * cap),
                floor: fit.floor.max(y + 0.5 * cap + 1.0),
                tight: fit.tight,
                offset,
                notes: st.notes.clone(),
                tolerance,
            });
        }
    }
    for m in doc.edits.manual_lines.iter().filter(|m| m.page == page) {
        // Snap targets come from the staff the line was added for.
        let notes = analysis
            .and_then(|a| {
                a.staves
                    .iter()
                    .filter(|st| st.x1.min(m.x1) > st.x0.max(m.x0))
                    .min_by(|a, b| (a.top - m.y).abs().total_cmp(&(b.top - m.y).abs()))
            })
            .map_or_else(|| Arc::from([]), |st| st.notes.clone());
        out.push(Line {
            anchor: Anchor::Manual { id: m.id },
            page,
            staff: None,
            y: m.y,
            x0: m.x0,
            x1: m.x1,
            ceil: m.y - 2.0 * font.size(),
            floor: m.y + font.size(),
            tight: false,
            offset: 0.0,
            notes,
            tolerance: 0.0,
        });
    }
    out.sort_by(|a, b| a.y.total_cmp(&b.y));
    out
}

// ---------------------------------------------------------------------- chords

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64, pad: f64) -> bool {
        x >= self.x0 - pad && x <= self.x1 + pad && y >= self.y0 - pad && y <= self.y1 + pad
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineRef {
    pub page: usize,
    pub index: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Placed {
    pub line: LineRef,
    pub baseline: f64,
    /// Vertical offset from the line, chosen by hand or by avoidance.
    pub dy: f64,
    /// Ink box, for hit testing and highlighting.
    pub bounds: Rect,
}

/// Where chord text centred on `x` goes on `line`: its baseline, the automatic
/// offset, and its ink box.
pub fn place(
    text: &Text,
    x: f64,
    dy: Option<f64>,
    line: &Line,
    font: &ChordFont,
    analysis: Option<&analysis::Page>,
    settings: &Settings,
) -> (f64, f64, Rect) {
    let m = &text.metrics;
    let cap = font.cap_height();
    let base = line.y + cap / 2.0;
    let left = x - m.width / 2.0;
    let dy = match (dy, analysis) {
        (Some(dy), _) => dy,
        (None, Some(a)) if settings.auto_avoid => avoid(a, line, font, m, left, base),
        _ => 0.0,
    };
    let b = base + dy;
    let bounds = Rect {
        x0: left + m.ink_x0.min(0.0),
        x1: left + m.width.max(m.ink_x1),
        y0: b + m.ink_y0.min(-cap),
        y1: b + m.ink_y1.max(0.0),
    };
    (b, dy, bounds)
}

/// The smallest vertical shift (preferring up) that keeps the chord clear of
/// the score's ink within the line's allowed range.
fn avoid(
    a: &analysis::Page,
    line: &Line,
    font: &ChordFont,
    m: &Metrics,
    left: f64,
    base: f64,
) -> f64 {
    let pad_x = font.size() * 0.08;
    let pad_y = font.size() * 0.12;
    let ink = |dy: f64| {
        a.ink_in_rect(
            left + m.ink_x0 - pad_x,
            base + dy + m.ink_y0 - pad_y,
            left + m.ink_x1 + pad_x,
            base + dy + m.ink_y1 + pad_y,
        )
    };
    let mut best = (0.0, ink(0.0));
    if best.1 == 0 {
        return 0.0;
    }
    const STEP: f64 = 0.25;
    let max_shift = font.size() * 1.6;
    let mut d = STEP;
    while d <= max_shift {
        for dy in [-d, d] {
            if base + dy + m.ink_y0 < line.ceil || base + dy + m.ink_y1 > line.floor {
                continue;
            }
            let n = ink(dy);
            if n == 0 {
                return dy;
            }
            if n < best.1 {
                best = (dy, n);
            }
        }
        d += STEP;
    }
    best.0
}

// -------------------------------------------------------------- whole document

/// Lines of every page and the position of every chord.
#[derive(Default)]
pub struct DocLayout {
    pub lines: Vec<Vec<Line>>,
    placed: HashMap<u32, Placed>,
}

impl DocLayout {
    pub fn compute(
        doc: &Document,
        analyses: &[Option<Arc<analysis::Page>>],
        font: &ChordFont,
    ) -> DocLayout {
        let lines = analyses
            .iter()
            .enumerate()
            .map(|(page, a)| page_lines(page, a.as_deref(), doc, font))
            .collect();
        let mut layout = DocLayout {
            lines,
            placed: HashMap::new(),
        };
        for c in &doc.edits.chords {
            if let Some(p) = layout.place_chord(c, doc, analyses, font) {
                layout.placed.insert(c.id, p);
            }
        }
        layout
    }

    fn place_chord(
        &self,
        c: &Chord,
        doc: &Document,
        analyses: &[Option<Arc<analysis::Page>>],
        font: &ChordFont,
    ) -> Option<Placed> {
        let r = self.resolve(&c.anchor)?;
        let line = self.line(r);
        let a = analyses.get(r.page).and_then(|a| a.as_deref());
        let (baseline, dy, bounds) =
            place(&font.text(&c.text), c.x, c.dy, line, font, a, &doc.settings);
        Some(Placed {
            line: r,
            baseline,
            dy,
            bounds,
        })
    }

    pub fn line(&self, r: LineRef) -> &Line {
        &self.lines[r.page][r.index]
    }

    pub fn placed(&self, chord: u32) -> Option<&Placed> {
        self.placed.get(&chord)
    }

    /// Finds the line an anchor refers to. A staff anchor whose line was not
    /// detected exactly as before falls back to the nearest line on its page.
    pub fn resolve(&self, anchor: &Anchor) -> Option<LineRef> {
        match *anchor {
            Anchor::Manual { id } => self.lines.iter().enumerate().find_map(|(page, ls)| {
                let index = ls.iter().position(|l| l.anchor == Anchor::Manual { id })?;
                Some(LineRef { page, index })
            }),
            Anchor::Staff { page, staff_top } => {
                let lines = self.lines.get(page)?;
                let dist = |l: &Line| match l.anchor {
                    Anchor::Staff { staff_top: t, .. } => (t - staff_top).abs(),
                    Anchor::Manual { .. } => (l.y - staff_top).abs(),
                };
                let exact = lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.staff.is_some() && dist(l) <= l.tolerance)
                    .min_by(|a, b| dist(a.1).total_cmp(&dist(b.1)));
                let (index, _) = exact.or_else(|| {
                    lines
                        .iter()
                        .enumerate()
                        .min_by(|a, b| dist(a.1).total_cmp(&dist(b.1)))
                })?;
                Some(LineRef { page, index })
            }
        }
    }

    /// All lines in reading order.
    pub fn all_lines(&self) -> impl Iterator<Item = LineRef> + '_ {
        self.lines
            .iter()
            .enumerate()
            .flat_map(|(page, ls)| (0..ls.len()).map(move |index| LineRef { page, index }))
    }

    /// The line before (`dir < 0`) or after `r` in reading order.
    pub fn adjacent(&self, r: LineRef, dir: i32) -> Option<LineRef> {
        if dir > 0 {
            self.all_lines().skip_while(|&l| l != r).nth(1)
        } else {
            self.all_lines().take_while(|&l| l != r).last()
        }
    }

    /// The line on `page` closest to `y`.
    pub fn nearest_line(&self, page: usize, y: f64) -> Option<LineRef> {
        let lines = self.lines.get(page)?;
        let (index, _) = lines
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1.y - y).abs().total_cmp(&(b.1.y - y).abs()))?;
        Some(LineRef { page, index })
    }
}
