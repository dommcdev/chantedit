//! Editing state and operations, independent of GTK. Operations queue
//! [`Effect`]s that the window applies afterwards.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chantedit::analysis;
use chantedit::document::{Anchor, Chord, Document, Edits, ManualLine, Settings};
use chantedit::export::{Item, PageItems};
use chantedit::layout::{ChordFont, DocLayout, Line, LineRef, Placed};

/// Arrow-key step, in points.
pub const NUDGE: f64 = 1.0;
/// Shift+arrow step.
pub const NUDGE_BIG: f64 = 6.0;
/// Vertical tweak step.
pub const V_STEP: f64 = 0.5;

const UNDO_LIMIT: usize = 500;
/// Repeated small edits on the same target within this time are one undo step.
const MERGE_WINDOW: Duration = Duration::from_millis(1500);

pub enum Effect {
    Toast(String),
    /// Scroll so that this page point is visible.
    Reveal {
        page: usize,
        x: f64,
        y: f64,
    },
    /// Replace the chord entry's text.
    SetEntry(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cursor {
    pub anchor: Anchor,
    pub x: f64,
}

pub struct Preview {
    pub line: LineRef,
    pub x: f64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Insert,
    Edit,
    Nudge,
    NudgeY,
    ResetY,
    AdjustLine,
    ResetLine,
    ResetLines,
    AddLine,
    RemoveLine,
    Delete,
    Drag,
}

impl Op {
    fn merges(self) -> bool {
        matches!(self, Op::Nudge | Op::NudgeY | Op::AdjustLine)
    }
}

struct Snapshot {
    edits: Edits,
    sel: Option<u32>,
}

struct Drag {
    chord: u32,
    page: usize,
    start_x: f64,
    press_y: f64,
    moved: bool,
}

pub struct Editor {
    pub doc: Document,
    /// Where the document is saved; `None` until the first save.
    pub path: Option<PathBuf>,
    /// The PDF this document was created from in this session.
    pub source: Option<PathBuf>,
    pub page_sizes: Vec<(f64, f64)>,
    pub analyses: Vec<Option<Arc<analysis::Page>>>,
    pub font: ChordFont,
    pub layout: DocLayout,
    pub sel: Option<u32>,
    pub editing: bool,
    pub cursor: Option<Cursor>,
    /// Text in the chord entry.
    pub pending: String,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_op: Option<(Op, Option<u32>, Instant)>,
    saved: (Edits, Settings),
    drag: Option<Drag>,
    started: bool,
    effects: Vec<Effect>,
}

impl Editor {
    pub fn new(
        doc: Document,
        path: Option<PathBuf>,
        source: Option<PathBuf>,
        page_sizes: Vec<(f64, f64)>,
    ) -> Editor {
        let font = ChordFont::new(&doc.settings.font, doc.settings.size);
        let saved = (doc.edits.clone(), doc.settings.clone());
        let mut ed = Editor {
            analyses: vec![None; page_sizes.len()],
            page_sizes,
            doc,
            path,
            source,
            font,
            layout: DocLayout::default(),
            sel: None,
            editing: false,
            cursor: None,
            pending: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            last_op: None,
            saved,
            drag: None,
            started: false,
            effects: Vec::new(),
        };
        ed.relayout();
        ed
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.effects.push(Effect::Toast(msg.into()));
    }

    // ---------------------------------------------------------------- file

    pub fn is_dirty(&self) -> bool {
        self.doc.edits != self.saved.0 || self.doc.settings != self.saved.1
    }

    pub fn mark_saved(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.saved = (self.doc.edits.clone(), self.doc.settings.clone());
    }

    /// Display name: the saved file's or the source PDF's stem.
    pub fn name(&self) -> String {
        let from_path = |p: &PathBuf| p.file_stem().map(|s| s.to_string_lossy().into_owned());
        self.path
            .as_ref()
            .and_then(from_path)
            .or_else(|| self.source.as_ref().and_then(from_path))
            .unwrap_or_else(|| {
                let n = &self.doc.source_name;
                n.strip_suffix(".pdf")
                    .or(n.strip_suffix(".PDF"))
                    .unwrap_or(n)
                    .to_owned()
            })
    }

    /// Folder the document belongs in: next to its file, or its source PDF.
    pub fn dir(&self) -> Option<PathBuf> {
        self.path
            .as_ref()
            .or(self.source.as_ref())
            .and_then(|p| p.parent())
            .map(Into::into)
    }

    pub fn analysis_done(&self) -> bool {
        self.analyses.iter().all(Option::is_some)
    }

    pub fn set_analysis(&mut self, page: usize, a: Arc<analysis::Page>) {
        if let Some(slot) = self.analyses.get_mut(page) {
            *slot = Some(a);
        }
        self.relayout();
        if self.started || self.cursor.is_some() || self.sel.is_some() {
            return;
        }
        if self.doc.edits.chords.is_empty() {
            self.started = self.cursor_to_start();
        } else if self.analysis_done() {
            // Continue where the last session left off.
            if let Some(&last) = self.sorted_chords().last() {
                self.select_chord(last, false);
            }
            self.started = true;
        }
    }

    pub fn set_settings(&mut self, s: Settings) {
        if s.font != self.doc.settings.font || s.size != self.doc.settings.size {
            self.font = ChordFont::new(&s.font, s.size);
        }
        self.doc.settings = s;
        self.relayout();
    }

    fn relayout(&mut self) {
        self.layout = DocLayout::compute(&self.doc, &self.analyses, &self.font);
    }

    fn page_width(&self, page: usize) -> f64 {
        self.page_sizes.get(page).map_or(612.0, |s| s.0)
    }

    // --------------------------------------------------------------- lookup

    pub fn placed(&self, id: u32) -> Option<&Placed> {
        self.layout.placed(id)
    }

    pub fn selected(&self) -> Option<&Chord> {
        self.sel.and_then(|id| self.doc.edits.chord(id))
    }

    pub fn cursor_line(&self) -> Option<LineRef> {
        self.cursor.and_then(|c| self.layout.resolve(&c.anchor))
    }

    /// The line of the selected chord, or of the cursor.
    pub fn active_line(&self) -> Option<LineRef> {
        match self.sel {
            Some(id) => self.placed(id).map(|p| p.line),
            None => self.cursor_line(),
        }
    }

    fn line(&self, r: LineRef) -> &Line {
        self.layout.line(r)
    }

    /// Chord ids in reading order.
    pub fn sorted_chords(&self) -> Vec<u32> {
        let key = |c: &Chord| {
            self.placed(c.id)
                .map(|p| (p.line.page, self.line(p.line).y, c.x))
        };
        let mut v: Vec<(Option<(usize, f64, f64)>, u32)> = self
            .doc
            .edits
            .chords
            .iter()
            .map(|c| (key(c), c.id))
            .collect();
        v.sort_by(|(a, _), (b, _)| match (a, b) {
            (Some(a), Some(b)) => {
                a.0.cmp(&b.0)
                    .then(a.1.total_cmp(&b.1))
                    .then(a.2.total_cmp(&b.2))
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        v.into_iter().map(|(_, id)| id).collect()
    }

    /// The chord drawn at a page point.
    pub fn hit(&self, page: usize, x: f64, y: f64) -> Option<u32> {
        self.doc.edits.chords.iter().rev().find_map(|c| {
            let p = self.placed(c.id)?;
            (p.line.page == page && p.bounds.contains(x, y, 2.0)).then_some(c.id)
        })
    }

    /// The line near a page point, if within `max_dist`.
    pub fn line_near(&self, page: usize, y: f64, max_dist: f64) -> Option<LineRef> {
        self.layout
            .nearest_line(page, y)
            .filter(|&r| (self.line(r).y - y).abs() < max_dist)
    }

    // ---------------------------------------------------------------- undo

    fn push_undo(&mut self, op: Op, target: Option<u32>) {
        let now = Instant::now();
        if let Some((last, t, at)) = self.last_op {
            if op.merges() && last == op && t == target && now - at < MERGE_WINDOW {
                self.last_op = Some((op, target, now));
                return;
            }
        }
        self.last_op = Some((op, target, now));
        self.undo.push(Snapshot {
            edits: self.doc.edits.clone(),
            sel: self.sel,
        });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self) {
        match self.undo.pop() {
            Some(s) => {
                self.redo.push(Snapshot {
                    edits: self.doc.edits.clone(),
                    sel: self.sel,
                });
                self.restore(s);
            }
            None => self.toast("Nothing to undo"),
        }
    }

    pub fn redo(&mut self) {
        match self.redo.pop() {
            Some(s) => {
                self.undo.push(Snapshot {
                    edits: self.doc.edits.clone(),
                    sel: self.sel,
                });
                self.restore(s);
            }
            None => self.toast("Nothing to redo"),
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.doc.edits = s.edits;
        self.last_op = None;
        if self.editing {
            self.cancel_edit();
        }
        self.sel = None;
        self.relayout();
        if let Some(id) = s.sel.filter(|&id| self.doc.edits.chord(id).is_some()) {
            self.select_chord(id, true);
        }
    }

    // ------------------------------------------------------ selection/cursor

    pub fn select_chord(&mut self, id: u32, reveal: bool) {
        let Some(c) = self.doc.edits.chord(id) else {
            return;
        };
        let cursor = Cursor {
            anchor: c.anchor,
            x: c.x,
        };
        if self.editing && self.sel != Some(id) {
            self.cancel_edit();
        }
        self.sel = Some(id);
        self.cursor = Some(cursor);
        if reveal {
            if let Some(p) = self.placed(id) {
                self.effects.push(Effect::Reveal {
                    page: p.line.page,
                    x: cursor.x,
                    y: p.baseline,
                });
            }
        }
    }

    pub fn deselect(&mut self) {
        if let Some(c) = self.selected() {
            self.cursor = Some(Cursor {
                anchor: c.anchor,
                x: c.x,
            });
        }
        self.sel = None;
        self.editing = false;
    }

    pub fn place_cursor(&mut self, page: usize, x: f64, y: f64) {
        self.sel = None;
        let Some(r) = self.layout.nearest_line(page, y) else {
            if self.analyses.get(page).is_some_and(Option::is_none) {
                self.toast("Still analysing this page…");
            } else {
                self.toast("No chord lines on this page. Ctrl+click to add one.");
            }
            return;
        };
        let line = self.line(r);
        let x = if self.doc.settings.snap_notes {
            line.nearest_note(x, self.font.size() * 0.9)
        } else {
            x
        };
        self.cursor = Some(Cursor {
            anchor: line.anchor,
            x,
        });
    }

    fn cursor_to_start(&mut self) -> bool {
        let Some(r) = self.layout.all_lines().next() else {
            return false;
        };
        let line = self.line(r);
        let x = line.next_note(line.x0, 1, 0.0).unwrap_or(line.x0 + 20.0);
        self.cursor = Some(Cursor {
            anchor: line.anchor,
            x,
        });
        true
    }

    // ------------------------------------------------------------ inserting

    /// Where a chord goes after one (or the bare cursor, `prev = None`) at `x`.
    fn next_position(&self, r: LineRef, x: f64, prev: Option<&str>, text: &str) -> (LineRef, f64) {
        let Some(prev) = prev else { return (r, x) };
        let snap = self.doc.settings.snap_notes;
        let w = self.font.text(text).metrics.width;
        let pw = self.font.text(prev).metrics.width;
        let line = self.line(r);
        let need = x + pw / 2.0 + self.font.size() * 0.4 + w / 2.0;
        let nx = if snap {
            line.next_note(need - 0.01, 1, 0.0).unwrap_or(need)
        } else {
            need
        };
        if nx + w / 2.0 > line.x1 + self.font.size() {
            if let Some(next) = self.layout.adjacent(r, 1) {
                let l = self.line(next);
                let x0 = l.x0 + w / 2.0;
                let x0 = if snap {
                    l.next_note(l.x0 + w / 4.0, 1, 0.0).unwrap_or(x0)
                } else {
                    x0
                };
                return (next, x0);
            }
        }
        (r, nx)
    }

    /// Where the space-separated chords typed in the entry would go.
    pub fn previews(&self) -> Vec<Preview> {
        let input = self.pending.trim();
        if input.is_empty() || self.editing {
            return Vec::new();
        }
        let start = match self.selected() {
            Some(c) => self
                .placed(c.id)
                .map(|p| (p.line, c.x, Some(c.text.as_str()))),
            None => self
                .cursor
                .and_then(|cur| Some((self.layout.resolve(&cur.anchor)?, cur.x, None))),
        };
        let Some((mut r, mut x, mut prev)) = start else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for t in input.split_whitespace() {
            (r, x) = self.next_position(r, x, prev, t);
            out.push(Preview {
                line: r,
                x,
                text: t.to_owned(),
            });
            prev = Some(t);
        }
        out
    }

    /// Enter in the chord entry.
    pub fn commit_entry(&mut self) {
        let text = self
            .pending
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            if let (Some(id), false) = (self.sel, self.editing) {
                self.start_edit(id);
            }
            return;
        }
        if self.editing {
            if let Some(id) = self.sel {
                self.push_undo(Op::Edit, Some(id));
                if let Some(c) = self.doc.edits.chord_mut(id) {
                    c.text = text;
                }
            }
            self.editing = false;
            self.set_entry("");
            self.relayout();
            return;
        }
        let previews = self.previews();
        let Some(last) = previews.last() else {
            self.toast("Click on the score to choose where the chord goes");
            return;
        };
        let (last_line, last_x) = (last.line, last.x);
        self.push_undo(Op::Insert, None);
        for p in previews {
            let id = self.doc.edits.new_id();
            let anchor = self.line(p.line).anchor;
            self.doc.edits.chords.push(Chord {
                id,
                anchor,
                x: p.x,
                text: p.text,
                dy: None,
            });
            self.sel = Some(id);
        }
        self.cursor = Some(Cursor {
            anchor: self.line(last_line).anchor,
            x: last_x,
        });
        self.set_entry("");
        self.relayout();
        let y = self.line(last_line).y;
        self.effects.push(Effect::Reveal {
            page: last_line.page,
            x: last_x,
            y,
        });
    }

    fn set_entry(&mut self, text: &str) {
        self.pending = text.to_owned();
        self.effects.push(Effect::SetEntry(text.to_owned()));
    }

    pub fn start_edit(&mut self, id: u32) {
        let Some(text) = self.doc.edits.chord(id).map(|c| c.text.clone()) else {
            return;
        };
        self.select_chord(id, false);
        self.editing = true;
        self.set_entry(&text);
    }

    pub fn cancel_edit(&mut self) {
        self.editing = false;
        self.set_entry("");
    }

    // -------------------------------------------------------------- editing

    pub fn nudge(&mut self, dx: f64) {
        if let Some(id) = self.sel {
            let Some(r) = self.placed(id).map(|p| p.line) else {
                return;
            };
            let w = self.page_width(r.page);
            self.push_undo(Op::Nudge, Some(id));
            let c = self.doc.edits.chord_mut(id).expect("selected chord exists");
            c.x = (c.x + dx).clamp(0.0, w);
            let x = c.x;
            if let Some(cur) = &mut self.cursor {
                cur.x = x;
            }
            self.relayout();
        } else if let Some(r) = self.cursor_line() {
            let w = self.page_width(r.page);
            if let Some(cur) = &mut self.cursor {
                cur.x = (cur.x + dx).clamp(0.0, w);
            }
        }
    }

    pub fn jump_note(&mut self, dir: i32) {
        let Some(r) = self.active_line() else { return };
        let x = self
            .selected()
            .map(|c| c.x)
            .or(self.cursor.map(|c| c.x))
            .unwrap_or(0.0);
        let target = match self.line(r).next_note(x, dir, 0.75) {
            Some(nx) => Some((r, nx)),
            None => self.layout.adjacent(r, dir).and_then(|adj| {
                let notes = &self.line(adj).notes;
                let nx = if dir > 0 { notes.first() } else { notes.last() };
                nx.map(|&nx| (adj, nx))
            }),
        };
        if let Some((r, x)) = target {
            self.move_to(r, x);
        }
    }

    fn move_to(&mut self, r: LineRef, x: f64) {
        let anchor = self.line(r).anchor;
        if let Some(id) = self.sel {
            self.push_undo(Op::Nudge, Some(id));
            if let Some(c) = self.doc.edits.chord_mut(id) {
                c.anchor = anchor;
                c.x = x;
            }
            self.relayout();
        }
        self.cursor = Some(Cursor { anchor, x });
        let y = self.line(r).y;
        self.effects.push(Effect::Reveal { page: r.page, x, y });
    }

    pub fn change_line(&mut self, dir: i32) {
        let Some(r) = self.active_line() else {
            if self.cursor.is_none() {
                self.cursor_to_start();
            }
            return;
        };
        let Some(next) = self.layout.adjacent(r, dir) else {
            return;
        };
        let x = self
            .selected()
            .map(|c| c.x)
            .or(self.cursor.map(|c| c.x))
            .unwrap_or(0.0);
        self.move_to(next, x.clamp(0.0, self.page_width(next.page)));
    }

    /// Raises or lowers only the selected chord.
    pub fn nudge_y(&mut self, d: f64) {
        let Some(id) = self.sel else { return };
        let Some(dy) = self.placed(id).map(|p| p.dy) else {
            return;
        };
        self.push_undo(Op::NudgeY, Some(id));
        if let Some(c) = self.doc.edits.chord_mut(id) {
            c.dy = Some(dy + d);
        }
        self.relayout();
    }

    pub fn reset_chord_y(&mut self, id: u32) {
        if self.doc.edits.chord(id).is_some_and(|c| c.dy.is_some()) {
            self.push_undo(Op::ResetY, Some(id));
            if let Some(c) = self.doc.edits.chord_mut(id) {
                c.dy = None;
            }
            self.relayout();
        }
    }

    /// Raises or lowers the whole active line.
    pub fn adjust_line(&mut self, d: f64) {
        let Some(r) = self.active_line() else { return };
        let line = self.line(r).clone();
        self.push_undo(Op::AdjustLine, Some(r.page as u32 * 1000 + r.index as u32));
        match line.anchor {
            Anchor::Staff { page, staff_top } => {
                self.doc
                    .edits
                    .staff_line_mut(page, staff_top, line.tolerance)
                    .offset += d;
            }
            Anchor::Manual { id } => {
                if let Some(m) = self.doc.edits.manual_lines.iter_mut().find(|m| m.id == id) {
                    m.y += d;
                }
            }
        }
        self.relayout();
    }

    pub fn reset_line(&mut self, r: LineRef) {
        let line = self.line(r).clone();
        if let Anchor::Staff { page, staff_top } = line.anchor {
            self.push_undo(Op::ResetLine, None);
            self.doc
                .edits
                .staff_line_mut(page, staff_top, line.tolerance)
                .offset = 0.0;
            self.doc
                .edits
                .staff_lines
                .retain(|e| e.offset != 0.0 || e.hidden);
            self.relayout();
        }
    }

    /// Back to the detected lines: removes manual lines (moving their chords to
    /// the nearest detected line) and restores hidden and moved lines.
    pub fn reset_lines(&mut self) {
        self.push_undo(Op::ResetLines, None);
        let manual: Vec<(u32, usize, f64)> = self
            .doc
            .edits
            .chords
            .iter()
            .filter_map(|c| match c.anchor {
                Anchor::Manual { id } => {
                    self.doc.edits.manual_line(id).map(|m| (c.id, m.page, m.y))
                }
                Anchor::Staff { .. } => None,
            })
            .collect();
        self.doc.edits.staff_lines.clear();
        self.doc.edits.manual_lines.clear();
        self.relayout();
        for (id, page, y) in manual {
            match self
                .layout
                .nearest_line(page, y)
                .map(|r| self.line(r).anchor)
            {
                Some(anchor) => self.doc.edits.chord_mut(id).expect("chord exists").anchor = anchor,
                None => self.doc.edits.chords.retain(|c| c.id != id),
            }
        }
        if self
            .sel
            .is_some_and(|id| self.doc.edits.chord(id).is_none())
        {
            self.sel = None;
        }
        if self.cursor_line().is_none() {
            self.cursor = None;
        }
        self.relayout();
        self.toast("Chord lines reset to the detected positions");
    }

    pub fn add_line_at(&mut self, page: usize, y: f64) {
        let w = self.page_width(page);
        // Borrow the horizontal extent of the nearest staff.
        let (x0, x1) = self
            .analyses
            .get(page)
            .and_then(|a| a.as_deref())
            .and_then(|a| {
                a.staves
                    .iter()
                    .min_by(|a, b| (a.top - y).abs().total_cmp(&(b.top - y).abs()))
            })
            .map_or((w * 0.08, w * 0.92), |st| (st.x0, st.x1));
        self.push_undo(Op::AddLine, None);
        let id = self.doc.edits.new_id();
        self.doc.edits.manual_lines.push(ManualLine {
            id,
            page,
            y,
            x0,
            x1,
        });
        let x = self.cursor.map_or(x0 + 20.0, |c| c.x).clamp(x0, x1);
        self.sel = None;
        self.editing = false;
        self.cursor = Some(Cursor {
            anchor: Anchor::Manual { id },
            x,
        });
        self.relayout();
        self.toast("Chord line added");
    }

    /// Removes a line with its chords (detected lines are hidden).
    pub fn remove_line(&mut self, r: LineRef) {
        let line = self.line(r).clone();
        let doomed: Vec<u32> = self
            .doc
            .edits
            .chords
            .iter()
            .filter(|c| self.placed(c.id).is_some_and(|p| p.line == r))
            .map(|c| c.id)
            .collect();
        self.push_undo(Op::RemoveLine, None);
        self.doc.edits.chords.retain(|c| !doomed.contains(&c.id));
        match line.anchor {
            Anchor::Staff { page, staff_top } => {
                self.doc
                    .edits
                    .staff_line_mut(page, staff_top, line.tolerance)
                    .hidden = true;
            }
            Anchor::Manual { id } => self.doc.edits.manual_lines.retain(|m| m.id != id),
        }
        if self.sel.is_some_and(|id| doomed.contains(&id)) {
            self.sel = None;
            self.editing = false;
        }
        if self.cursor_line() == Some(r) {
            self.cursor = None;
        }
        self.relayout();
        self.toast(match doomed.len() {
            0 => "Chord line removed".to_owned(),
            1 => "Chord line removed with 1 chord (Ctrl+Z to undo)".to_owned(),
            n => format!("Chord line removed with {n} chords (Ctrl+Z to undo)"),
        });
    }

    pub fn remove_active_line(&mut self) {
        if let Some(r) = self.active_line() {
            self.remove_line(r);
        }
    }

    pub fn select_rel(&mut self, dir: i32) {
        let ids = self.sorted_chords();
        let idx = self.sel.and_then(|s| ids.iter().position(|&id| id == s));
        let j = match idx {
            Some(i) => i as isize + dir as isize,
            None if dir > 0 => 0,
            None => ids.len() as isize - 1,
        };
        if let Some(&id) = usize::try_from(j).ok().and_then(|j| ids.get(j)) {
            self.select_chord(id, true);
        }
    }

    pub fn select_first(&mut self) {
        if let Some(&id) = self.sorted_chords().first() {
            self.select_chord(id, true);
        }
    }

    pub fn select_last(&mut self) {
        if let Some(&id) = self.sorted_chords().last() {
            self.select_chord(id, true);
        }
    }

    /// Deletes the selected chord and selects its neighbour.
    pub fn delete_selected(&mut self, backward: bool) {
        let ids = self.sorted_chords();
        let Some(idx) = self.sel.and_then(|s| ids.iter().position(|&id| id == s)) else {
            return;
        };
        let next = if backward {
            idx.checked_sub(1)
        } else {
            Some(idx + 1)
        }
        .and_then(|j| ids.get(j))
        .copied();
        let del = self
            .doc
            .edits
            .chord(ids[idx])
            .expect("chord exists")
            .clone();
        self.push_undo(Op::Delete, Some(del.id));
        self.doc.edits.chords.retain(|c| c.id != del.id);
        self.sel = None;
        self.editing = false;
        self.cursor = Some(Cursor {
            anchor: del.anchor,
            x: del.x,
        });
        self.relayout();
        if let Some(id) = next {
            self.select_chord(id, true);
        }
    }

    // ---------------------------------------------------------------- mouse

    /// Primary button pressed at a page point.
    pub fn press(&mut self, page: usize, x: f64, y: f64, ctrl: bool) {
        self.drag = None;
        if ctrl {
            self.add_line_at(page, y);
            return;
        }
        match self.hit(page, x, y) {
            Some(id) => {
                self.select_chord(id, false);
                let start_x = self.doc.edits.chord(id).map_or(x, |c| c.x);
                self.drag = Some(Drag {
                    chord: id,
                    page,
                    start_x,
                    press_y: y,
                    moved: false,
                });
            }
            None => {
                if self.editing {
                    self.cancel_edit();
                }
                self.place_cursor(page, x, y);
            }
        }
    }

    /// The pointer moved by (`dx`, `dy`) points since the press. Movement
    /// below `threshold` does not start a drag.
    pub fn drag_to(&mut self, dx: f64, dy: f64, threshold: f64) {
        let Some(d) = &self.drag else { return };
        let (id, page, start_x, press_y, moved) = (d.chord, d.page, d.start_x, d.press_y, d.moved);
        if !moved {
            if dx.abs() + dy.abs() < threshold {
                return;
            }
            self.push_undo(Op::Drag, Some(id));
            self.drag.as_mut().expect("dragging").moved = true;
        }
        let w = self.page_width(page);
        let anchor = self
            .layout
            .nearest_line(page, press_y + dy)
            .map(|r| self.line(r).anchor);
        let Some(c) = self.doc.edits.chord_mut(id) else {
            return;
        };
        c.x = (start_x + dx).clamp(0.0, w);
        if let Some(anchor) = anchor {
            c.anchor = anchor;
        }
        self.cursor = Some(Cursor {
            anchor: c.anchor,
            x: c.x,
        });
        self.relayout();
    }

    pub fn release(&mut self) {
        self.drag = None;
    }

    pub fn double_click(&mut self, page: usize, x: f64, y: f64) {
        if let Some(id) = self.hit(page, x, y) {
            self.start_edit(id);
        }
    }

    // --------------------------------------------------------------- output

    /// One-line status for the sidebar.
    pub fn status(&self) -> String {
        let where_ = |r: Option<LineRef>| match r {
            Some(r) => format!("page {}, line {}", r.page + 1, r.index + 1),
            None => "no line".to_owned(),
        };
        match (self.selected(), self.cursor) {
            (Some(c), _) if self.editing => {
                format!("Editing “{}”: Enter to apply, Esc to cancel", c.text)
            }
            (Some(c), _) => {
                let tweak = if c.dy.is_some() {
                    " · height tweaked"
                } else {
                    ""
                };
                let r = self.placed(c.id).map(|p| p.line);
                format!(
                    "“{}” selected · {}{tweak}\nNext chord goes after it. Arrows move it.",
                    c.text,
                    where_(r)
                )
            }
            (None, Some(_)) => {
                format!(
                    "Cursor on {}. Type a chord and press Enter.",
                    where_(self.cursor_line())
                )
            }
            (None, None) if !self.analysis_done() => "Finding chord lines…".to_owned(),
            (None, None) => "Click on the score where the first chord goes.".to_owned(),
        }
    }

    pub fn export_pages(&self) -> Vec<PageItems> {
        let mut pages: Vec<PageItems> = self
            .page_sizes
            .iter()
            .map(|&(width, height)| PageItems {
                width,
                height,
                items: Vec::new(),
            })
            .collect();
        for c in &self.doc.edits.chords {
            if let Some(p) = self.placed(c.id) {
                pages[p.line.page].items.push(Item {
                    text: c.text.clone(),
                    cx: c.x,
                    baseline: p.baseline,
                });
            }
        }
        pages
    }
}
