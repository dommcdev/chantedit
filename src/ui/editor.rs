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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cursor {
    pub anchor: Anchor,
    pub x: f64,
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
    cursor: Option<Cursor>,
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
    analysis_finished: Vec<bool>,
    pub font: ChordFont,
    pub layout: DocLayout,
    pub sel: Option<u32>,
    pub editing: bool,
    pub cursor: Option<Cursor>,
    /// Text accumulated during the current on-page typing session.
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
            analysis_finished: vec![false; page_sizes.len()],
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
        self.cancel_edit();
        self.path = Some(path);
        self.saved = (self.doc.edits.clone(), self.doc.settings.clone());
        // A subsequent nudge must undo back to this saved state, even if it
        // occurs within the key-repeat merge window.
        self.last_op = None;
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
        self.analysis_finished.iter().all(|done| *done)
    }

    pub fn analysis_failed(&mut self, page: Option<usize>) {
        match page {
            Some(page) => {
                if let Some(done) = self.analysis_finished.get_mut(page) {
                    *done = true;
                }
            }
            None => self.analysis_finished.fill(true),
        }
    }

    pub fn set_analysis(&mut self, page: usize, a: Arc<analysis::Page>) {
        if let Some(slot) = self.analyses.get_mut(page) {
            *slot = Some(a);
            self.analysis_finished[page] = true;
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
        let mut v: Vec<_> = self
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
        if let Some((last, t, at)) = self.last_op
            && op.merges()
            && last == op
            && t == target
            && now - at < MERGE_WINDOW
        {
            self.last_op = Some((op, target, now));
            return;
        }
        self.last_op = Some((op, target, now));
        self.undo.push(Snapshot {
            edits: self.doc.edits.clone(),
            sel: self.sel,
            cursor: self.cursor,
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
                    cursor: self.cursor,
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
                    cursor: self.cursor,
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
        self.cursor = s.cursor;
        self.relayout();
        if let Some(id) = s.sel.filter(|&id| self.doc.edits.chord(id).is_some()) {
            self.select_chord(id, true);
        }
    }

    // ------------------------------------------------------ selection/cursor

    pub fn select_chord(&mut self, id: u32, reveal: bool) {
        self.cancel_edit();
        let Some(c) = self.doc.edits.chord(id) else {
            return;
        };
        let cursor = Cursor {
            anchor: c.anchor,
            x: c.x,
        };
        self.sel = Some(id);
        self.cursor = Some(cursor);
        if reveal && let Some(p) = self.placed(id) {
            self.effects.push(Effect::Reveal {
                page: p.line.page,
                x: cursor.x,
                y: p.baseline,
            });
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
        self.cancel_edit();
    }

    pub fn place_cursor(&mut self, page: usize, x: f64, y: f64) {
        self.cancel_edit();
        self.sel = None;
        let Some(r) = self.layout.nearest_line(page, y) else {
            self.cursor = None;
            if self.analysis_finished.get(page) == Some(&false) {
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
        if let Some(id) = self.doc.edits.chords.iter().find_map(|c| {
            (self.placed(c.id).is_some_and(|p| p.line == r) && (c.x - x).abs() <= 0.75)
                .then_some(c.id)
        }) {
            self.select_chord(id, false);
        }
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

    pub fn start_edit(&mut self, id: u32) {
        let Some(text) = self.doc.edits.chord(id).map(|c| c.text.clone()) else {
            return;
        };
        self.select_chord(id, false);
        self.push_undo(Op::Edit, Some(id));
        self.editing = true;
        self.pending = text;
    }

    pub fn cancel_edit(&mut self) {
        // Text is already applied; ending a typing session only clears its buffer.
        self.editing = false;
        self.pending.clear();
    }

    // -------------------------------------------------------------- editing

    /// Typing replaces a selected chord on the first keystroke, then appends.
    /// The document and its layout update immediately, including before Save.
    pub fn type_text(&mut self, text: &str) {
        if !self.editing {
            if let Some(id) = self.sel {
                self.push_undo(Op::Edit, Some(id));
            } else {
                let Some(cur) = self.cursor.filter(|_| self.cursor_line().is_some()) else {
                    self.toast("Click on the score to choose where the chord goes");
                    return;
                };
                self.push_undo(Op::Insert, None);
                let id = self.doc.edits.new_id();
                self.doc.edits.chords.push(Chord {
                    id,
                    anchor: cur.anchor,
                    x: cur.x,
                    text: String::new(),
                    dy: None,
                });
                self.sel = Some(id);
            }
            self.pending.clear();
            self.editing = true;
        }
        self.pending.extend(text.chars().map(|ch| match ch {
            '#' => '♯',
            'b' => '♭',
            _ => ch,
        }));
        if let Some(c) = self.sel.and_then(|id| self.doc.edits.chord_mut(id)) {
            c.text = self.pending.clone();
        }
        self.relayout();
    }

    pub fn backspace_text(&mut self) {
        if !self.editing {
            self.delete_selected(true);
            return;
        }
        self.pending.pop();
        if self.pending.is_empty() {
            if let Some(id) = self.sel.take() {
                self.doc.edits.chords.retain(|c| c.id != id);
            }
            self.cancel_edit();
        } else if let Some(c) = self.sel.and_then(|id| self.doc.edits.chord_mut(id)) {
            c.text = self.pending.clone();
        }
        self.relayout();
    }

    /// Visit note positions and existing chords in score reading order. A chord
    /// occupying a note replaces that note's stop, so Tab never selects it twice.
    pub fn tab(&mut self, dir: i32) {
        self.cancel_edit();
        let mut stops = Vec::new();
        for r in self.layout.all_lines() {
            let line = self.line(r);
            let chords: Vec<_> = self
                .doc
                .edits
                .chords
                .iter()
                .filter(|c| self.placed(c.id).is_some_and(|p| p.line == r))
                .collect();
            for &x in line.notes.iter() {
                if !chords.iter().any(|c| (c.x - x).abs() <= 0.75) {
                    stops.push((r, x, None));
                }
            }
            for c in chords {
                stops.push((r, c.x, Some(c.id)));
            }
            if line.notes.is_empty() && !stops.iter().any(|(s, _, _)| *s == r) {
                stops.push((r, line.x0 + 20.0, None));
            }
        }
        stops.sort_by(|a, b| {
            a.0.page
                .cmp(&b.0.page)
                .then(a.0.index.cmp(&b.0.index))
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        });
        let current = self.active_line().zip(self.cursor.map(|c| c.x));
        let exact = stops.iter().position(|(r, x, id)| {
            if let Some(sel) = self.sel {
                *id == Some(sel)
            } else {
                current.is_some_and(|(cr, cx)| cr == *r && (cx - x).abs() <= 0.75)
            }
        });
        let target = if let Some(i) = exact {
            let next = i as isize + dir as isize;
            usize::try_from(next).ok().and_then(|j| stops.get(j))
        } else {
            let beyond = |&&(r, x, _): &&(LineRef, f64, Option<u32>)| {
                current.is_none_or(|(cr, cx)| {
                    let order = r
                        .page
                        .cmp(&cr.page)
                        .then(r.index.cmp(&cr.index))
                        .then(x.total_cmp(&cx));
                    if dir > 0 {
                        order.is_gt()
                    } else {
                        order.is_lt()
                    }
                })
            };
            if dir > 0 {
                stops.iter().find(beyond)
            } else {
                stops.iter().rev().find(beyond)
            }
        }
        .copied();
        if let Some((r, x, id)) = target {
            self.sel = None;
            self.cursor = Some(Cursor {
                anchor: self.line(r).anchor,
                x,
            });
            if let Some(id) = id {
                self.select_chord(id, false);
            }
            self.effects.push(Effect::Reveal {
                page: r.page,
                x,
                y: self.line(r).y,
            });
        }
    }

    pub fn nudge(&mut self, dx: f64) {
        self.cancel_edit();
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
        self.cancel_edit();
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
        self.cancel_edit();
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
        self.cancel_edit();
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
    /// the nearest detected line) and restores hidden and moved lines. Keeps
    /// manual lines on pages where no staves were detected.
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
        // A page with no detected staves still needs its manual lines. Dropping
        // them would silently delete all of the user's chords on that page.
        self.doc.edits.manual_lines.retain(|m| {
            !self
                .analyses
                .get(m.page)
                .and_then(|a| a.as_ref())
                .is_some_and(|a| !a.staves.is_empty())
        });
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
            if self.editing {
                self.cancel_edit();
            }
        }
        if self.cursor_line().is_none() {
            self.cursor = None;
        }
        self.relayout();
        if let Some(id) = self.sel {
            self.select_chord(id, false);
        }
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
        if self.editing {
            self.cancel_edit();
        }
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
            if self.editing {
                self.cancel_edit();
            }
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
        if self.editing {
            self.cancel_edit();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        let doc = Document::new(Vec::new(), "score.pdf".into(), Settings::default());
        let mut ed = Editor::new(doc, None, None, vec![(600.0, 800.0)]);
        ed.add_line_at(0, 100.0);
        ed.take_effects();
        ed
    }

    fn insert(ed: &mut Editor, text: &str) {
        ed.type_text(text);
        ed.cancel_edit();
    }

    fn score_editor() -> Editor {
        let mut gray = vec![255; 600 * 800];
        for top in [150, 300] {
            for y in (top..=top + 18).step_by(6) {
                gray[y * 600 + 40..y * 600 + 560].fill(0);
            }
        }
        let mut page = analysis::Page::analyze(&gray, 600, 800, 600.0, 800.0);
        assert_eq!(page.staves.len(), 2);
        for staff in &mut page.staves {
            staff.notes = Arc::from([80.0, 120.0, 160.0]);
        }
        let doc = Document::new(Vec::new(), "score.pdf".into(), Settings::default());
        let mut ed = Editor::new(doc, None, None, vec![(600.0, 800.0); 2]);
        let page = Arc::new(page);
        ed.set_analysis(0, page.clone());
        ed.set_analysis(1, page);
        ed
    }

    #[test]
    fn live_typing_replaces_selected_text_and_exports_before_enter() {
        let mut ed = score_editor();
        ed.type_text("D");
        ed.type_text("m");
        let id = ed.sel.unwrap();
        assert_eq!(ed.export_pages()[0].items[0].text, "Dm");
        let width = ed.placed(id).unwrap().bounds.x1 - ed.placed(id).unwrap().bounds.x0;
        ed.select_chord(id, false);
        ed.type_text("F");
        ed.type_text("#7");
        assert_eq!(ed.selected().unwrap().text, "F♯7");
        assert_ne!(
            ed.placed(id).unwrap().bounds.x1 - ed.placed(id).unwrap().bounds.x0,
            width
        );
        ed.undo();
        assert_eq!(ed.selected().unwrap().text, "Dm");
        ed.redo();
        assert_eq!(ed.selected().unwrap().text, "F♯7");
    }

    #[test]
    fn tab_visits_notes_and_chords_once_across_lines_and_pages() {
        let mut ed = score_editor();
        assert_eq!(ed.cursor.unwrap().x, 80.0);
        ed.type_text("C");
        let first = ed.sel.unwrap();
        ed.tab(1);
        assert_eq!(ed.cursor.unwrap().x, 120.0);
        assert!(ed.sel.is_none());
        assert!(!ed.editing);
        ed.type_text("Am");
        let second = ed.sel.unwrap();
        ed.tab(-1);
        assert_eq!(ed.sel, Some(first));
        ed.tab(1);
        assert_eq!(ed.sel, Some(second));
        ed.tab(1);
        assert!(ed.sel.is_none());
        assert_eq!(ed.cursor.unwrap().x, 160.0);
        ed.tab(1);
        assert_eq!(ed.cursor_line(), Some(LineRef { page: 0, index: 1 }));
        assert_eq!(ed.cursor.unwrap().x, 80.0);
        ed.tab(1);
        ed.tab(1);
        ed.tab(1);
        assert_eq!(ed.cursor_line(), Some(LineRef { page: 1, index: 0 }));
        ed.tab(-1);
        assert_eq!(ed.cursor_line(), Some(LineRef { page: 0, index: 1 }));
        assert_eq!(ed.cursor.unwrap().x, 160.0);
    }

    #[test]
    fn moved_chord_remains_a_tab_stop_and_backspace_updates_live() {
        let mut ed = score_editor();
        ed.type_text("Dm");
        let id = ed.sel.unwrap();
        ed.nudge(15.0);
        ed.tab(-1);
        assert!(ed.sel.is_none());
        assert_eq!(ed.cursor.unwrap().x, 80.0);
        ed.tab(1);
        assert_eq!(ed.sel, Some(id));
        ed.type_text("Bb");
        assert_eq!(ed.export_pages()[0].items[0].text, "B♭");
        ed.backspace_text();
        assert_eq!(ed.export_pages()[0].items[0].text, "B");
        ed.backspace_text();
        assert!(ed.doc.edits.chords.is_empty());
        assert_eq!(ed.cursor.unwrap().x, 95.0);
        ed.undo();
        assert_eq!(ed.selected().unwrap().text, "Dm");
    }

    #[test]
    fn insert_undo_restores_insertion_point_and_redo_selection() {
        let mut ed = editor();
        let cursor = ed.cursor;
        insert(&mut ed, "Dm");
        assert_eq!(ed.doc.edits.chords.len(), 1);
        let chords = ed.doc.edits.chords.clone();
        let selection = ed.sel;
        ed.undo();
        assert!(ed.doc.edits.chords.is_empty());
        assert_eq!(ed.cursor, cursor);
        ed.redo();
        assert_eq!(ed.doc.edits.chords, chords);
        assert_eq!(ed.sel, selection);
    }

    #[test]
    fn undo_line_removal_restores_cursor() {
        let mut ed = editor();
        let cursor = ed.cursor;
        ed.remove_active_line();
        assert!(ed.cursor.is_none());
        ed.undo();
        assert_eq!(ed.cursor, cursor);
        insert(&mut ed, "Am");
        assert_eq!(ed.doc.edits.chords.len(), 1);
    }

    #[test]
    fn reset_keeps_manual_chords_on_undetected_pages() {
        let mut ed = editor();
        ed.set_analysis(
            0,
            Arc::new(analysis::Page::analyze(&[255; 100], 10, 10, 600.0, 800.0)),
        );
        insert(&mut ed, "Dm");
        let chords = ed.doc.edits.chords.clone();
        ed.reset_lines();
        assert_eq!(ed.doc.edits.chords, chords);
        assert!(chords.iter().all(|c| ed.placed(c.id).is_some()));
    }

    #[test]
    fn removing_edited_chord_finishes_typing() {
        for operation in [
            Editor::remove_active_line,
            |ed: &mut Editor| ed.delete_selected(false),
            |ed: &mut Editor| ed.add_line_at(0, 200.0),
        ] {
            let mut ed = editor();
            insert(&mut ed, "C");
            ed.start_edit(ed.sel.unwrap());
            ed.take_effects();
            operation(&mut ed);
            assert!(!ed.editing);
            assert!(ed.pending.is_empty());
        }
    }

    #[test]
    fn failed_analysis_allows_manual_editing_and_export() {
        let mut ed = editor();
        assert!(!ed.analysis_done());
        ed.analysis_failed(Some(0));
        assert!(ed.analysis_done());
        insert(&mut ed, "Dm");
        assert_eq!(ed.export_pages()[0].items[0].text, "Dm");
    }

    #[test]
    fn save_separates_repeated_nudges_in_undo_history() {
        let mut ed = editor();
        insert(&mut ed, "C");
        ed.nudge(1.0);
        ed.mark_saved("test.ce".into());
        let x = ed.selected().unwrap().x;
        ed.nudge(1.0);
        ed.undo();
        assert_eq!(ed.selected().unwrap().x, x);
        assert!(!ed.is_dirty());
    }

    #[test]
    fn saving_live_text_starts_a_new_undo_step() {
        let mut ed = score_editor();
        ed.type_text("C");
        ed.mark_saved("test.ce".into());
        assert!(!ed.is_dirty());
        ed.type_text("Dm");
        assert!(ed.is_dirty());
        ed.undo();
        assert_eq!(ed.selected().unwrap().text, "C");
        assert!(!ed.is_dirty());
    }

    #[test]
    fn clicking_an_occupied_note_edits_its_chord() {
        let mut ed = score_editor();
        ed.type_text("C");
        let id = ed.sel.unwrap();
        let y = ed.layout.line(ed.active_line().unwrap()).y;
        ed.place_cursor(0, 80.0, y);
        assert_eq!(ed.sel, Some(id));
        ed.type_text("Dm");
        assert_eq!(ed.doc.edits.chords.len(), 1);
        assert_eq!(ed.selected().unwrap().text, "Dm");
    }

    #[test]
    fn clicking_page_without_lines_does_not_insert_on_previous_page() {
        let mut ed = editor();
        ed.place_cursor(1, 100.0, 100.0);
        insert(&mut ed, "C");
        assert!(ed.doc.edits.chords.is_empty());
        assert!(ed.pending.is_empty());
    }
}
