//! Source-anchored editing and collision layout, independent of GTK widgets.

use crate::chant::{Position, Preview};
use crate::chord_font::ChordFont;
use crate::gabc::{Chord, Document};

#[derive(Clone, Debug)]
pub struct Placed {
    pub chord: usize,
    pub page: usize,
    pub x: f64,
    pub baseline: f64,
    pub width: f64,
    pub top: f64,
    pub bottom: f64,
}

#[derive(Clone)]
struct Snapshot {
    chords: Vec<Chord>,
    cursor: usize,
    selected: Option<usize>,
}

pub struct Editor {
    pub doc: Document,
    pub preview: Preview,
    pub font: ChordFont,
    pub placed: Vec<Placed>,
    pub cursor: usize,
    pub selected: Option<usize>,
    pub editing: bool,
    pending: String,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl Editor {
    pub fn new(doc: Document, preview: Preview) -> Self {
        let mut editor = Self {
            doc,
            preview,
            font: ChordFont::new("Serif Bold", 9.0),
            placed: Vec::new(),
            cursor: 0,
            selected: None,
            editing: false,
            pending: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        };
        editor.layout(false);
        editor
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            chords: self.doc.chords.clone(),
            cursor: self.cursor,
            selected: self.selected,
        }
    }

    fn record(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 500 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.doc.chords = snapshot.chords;
        self.cursor = snapshot.cursor;
        self.selected = snapshot.selected;
        self.finish();
        self.layout(false);
    }

    pub fn undo(&mut self) {
        if let Some(snapshot) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(snapshot);
        }
    }

    pub fn redo(&mut self) {
        if let Some(snapshot) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(snapshot);
        }
    }

    /// Only chord rectangles move. Chant layout is immutable while editing.
    pub fn layout(&mut self, recalculate: bool) {
        self.placed.clear();
        let mut order: Vec<usize> = (0..self.doc.chords.len()).collect();
        order.sort_by_key(|&i| self.doc.chords[i].anchor);
        for index in order {
            let chord = &mut self.doc.chords[index];
            let pos = &self.preview.positions[chord.anchor];
            let metrics = self.font.text(&chord.text).metrics;
            let x = pos.x + chord.dx;
            let mut raise = if recalculate {
                0.0
            } else {
                chord.automatic_raise
            };
            loop {
                let baseline = pos.y - chord.dy - raise;
                let top = baseline + metrics.ink_y0;
                let bottom = baseline + metrics.ink_y1;
                let collides = self.placed.iter().any(|p| {
                    p.page == pos.page
                        && x < p.x + p.width + 3.0
                        && x + metrics.width + 3.0 > p.x
                        && top < p.bottom + 2.0
                        && bottom + 2.0 > p.top
                });
                if !collides || !recalculate {
                    break;
                }
                raise += self.font.size() * 1.4;
            }
            if recalculate {
                chord.automatic_raise = raise;
            }
            let baseline = pos.y - chord.dy - raise;
            self.placed.push(Placed {
                chord: index,
                page: pos.page,
                x,
                baseline,
                width: metrics.width,
                top: baseline + metrics.ink_y0,
                bottom: baseline + metrics.ink_y1,
            });
        }
    }

    pub fn position(&self) -> &Position {
        &self.preview.positions[self.cursor]
    }

    pub fn hit(&self, page: usize, x: f64, y: f64) -> Option<usize> {
        self.placed
            .iter()
            .rev()
            .find(|p| {
                p.page == page
                    && x >= p.x - 3.0
                    && x <= p.x + p.width + 3.0
                    && y >= p.top - 3.0
                    && y <= p.bottom + 3.0
            })
            .map(|p| p.chord)
    }

    pub fn select(&mut self, anchor: usize) {
        self.finish();
        self.cursor = anchor;
        self.selected = self.doc.chords.iter().position(|c| c.anchor == anchor);
    }

    pub fn click(&mut self, page: usize, x: f64, y: f64) {
        if let Some(index) = self.hit(page, x, y) {
            self.finish();
            self.selected = Some(index);
            self.cursor = self.doc.chords[index].anchor;
        } else if let Some(pos) = self
            .preview
            .positions
            .iter()
            .filter(|p| p.page == page)
            .min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs()))
        {
            self.select(pos.index);
        }
    }

    pub fn finish(&mut self) {
        self.editing = false;
        self.pending.clear();
    }

    pub fn start_edit(&mut self) {
        if let Some(index) = self.selected {
            self.record();
            self.pending = self.doc.chords[index].text.clone();
            self.editing = true;
        }
    }

    pub fn type_text(&mut self, text: &str) {
        if !self.editing {
            self.record();
            if self.selected.is_none() {
                self.doc.chords.push(Chord {
                    anchor: self.cursor,
                    text: String::new(),
                    dx: 0.0,
                    dy: 0.0,
                    automatic_raise: 0.0,
                });
                self.selected = Some(self.doc.chords.len() - 1);
            }
            self.pending.clear();
            self.editing = true;
        }
        self.pending.extend(text.chars().map(|c| match c {
            '#' => '♯',
            'b' => '♭',
            _ => c,
        }));
        if let Some(i) = self.selected {
            self.doc.chords[i].text = self.pending.clone();
        }
        self.layout(true);
    }

    pub fn backspace(&mut self) {
        if !self.editing {
            self.delete();
            return;
        }
        self.pending.pop();
        if let Some(i) = self.selected {
            self.doc.chords[i].text = self.pending.clone();
        }
        if self.pending.is_empty() {
            if let Some(i) = self.selected.take() {
                self.doc.chords.remove(i);
            }
            self.finish();
        }
        self.layout(true);
    }

    pub fn delete(&mut self) {
        if let Some(i) = self.selected {
            self.record();
            self.doc.chords.remove(i);
            self.selected = None;
            self.finish();
            self.layout(true);
        }
    }

    pub fn tab(&mut self, direction: i32) {
        let next = (self.cursor as i64 + i64::from(direction))
            .clamp(0, self.preview.positions.len() as i64 - 1);
        self.select(next as usize);
    }

    pub fn move_anchor(&mut self, direction: i32) {
        let next = (self.cursor as i64 + i64::from(direction))
            .clamp(0, self.preview.positions.len() as i64 - 1) as usize;
        self.move_to(next);
    }

    fn move_to(&mut self, next: usize) {
        self.finish();
        if let Some(i) = self.selected {
            if self
                .doc
                .chords
                .iter()
                .enumerate()
                .any(|(j, c)| j != i && c.anchor == next)
            {
                return;
            }
            if self.cursor != next {
                self.record();
                self.doc.chords[i].anchor = next;
                self.layout(true);
            }
            self.cursor = next;
        } else {
            self.select(next);
        }
    }

    pub fn move_staff(&mut self, direction: i32) {
        let pos = self.position();
        let next_page = pos.page as i64 + i64::from(direction);
        if next_page < 0 {
            return;
        }
        if let Some(next) = self
            .preview
            .positions
            .iter()
            .filter(|p| p.page == next_page as usize)
            .min_by(|a, b| (a.x - pos.x).abs().total_cmp(&(b.x - pos.x).abs()))
            .map(|p| p.index)
        {
            self.move_to(next);
        }
    }

    pub fn nudge(&mut self, dx: f64, dy: f64) {
        self.finish();
        if let Some(i) = self.selected {
            self.record();
            self.doc.chords[i].dx += dx;
            self.doc.chords[i].dy += dy;
            self.layout(true);
        }
    }

    pub fn reset_offsets(&mut self) {
        if let Some(i) = self.selected {
            self.record();
            self.doc.chords[i].dx = 0.0;
            self.doc.chords[i].dy = 0.0;
            self.layout(true);
        }
    }

    pub fn drag_to(&mut self, page: usize, x: f64) {
        if let Some(next) = self
            .preview
            .positions
            .iter()
            .filter(|p| p.page == page)
            .min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs()))
            .map(|p| p.index)
        {
            self.move_to(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collision_stagger_and_keyboard_edits_preserve_source_anchors() {
        let doc = Document::parse("name:T;\n%%\n(c4) A(g) B(h) (::)\n".into()).unwrap();
        let preview = Preview {
            pages: Vec::new(),
            positions: vec![
                Position {
                    index: 0,
                    page: 0,
                    x: 30.0,
                    y: 50.0,
                    note_y: 80.0,
                },
                Position {
                    index: 1,
                    page: 0,
                    x: 35.0,
                    y: 50.0,
                    note_y: 80.0,
                },
            ],
        };
        let mut ed = Editor::new(doc, preview);
        ed.type_text("Cmaj7");
        ed.tab(1);
        ed.type_text("Dm");
        assert!(ed.doc.chords[1].automatic_raise > 0.0);
        ed.nudge(1.5, 2.0);
        let saved = ed.doc.serialize().unwrap();
        let reloaded = Document::parse(saved).unwrap();
        assert_eq!(ed.doc.chords, reloaded.chords);
        assert_eq!(ed.doc.music, reloaded.music);
        ed.undo();
        assert_eq!(ed.doc.chords[1].dx, 0.0);
        ed.redo();
        assert_eq!(ed.doc.chords[1].dx, 1.5);
    }
}
