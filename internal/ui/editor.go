package ui

import (
	"fmt"
	"math"
	"sort"
	"strings"
	"time"

	"github.com/diamondburned/gotk4/pkg/core/glib"
	"github.com/diamondburned/gotk4/pkg/gdk/v4"

	"github.com/dominic/chantedit/internal/doc"
	"github.com/dominic/chantedit/internal/layout"
)

const (
	nudgeStep    = 1.0 // pt, arrow keys
	nudgeBigStep = 6.0 // pt, Shift+arrow
	vStep        = 0.5 // pt, vertical tweaks
)

func idle(f func()) { glib.IdleAdd(f) }

// ------------------------------------------------------------- lines/placing

func (w *Window) recomputeLines() {
	if w.data == nil {
		return
	}
	for i := range w.lines {
		if w.analyses[i] == nil && !w.hasExtra(i) {
			w.lines[i] = nil
			continue
		}
		w.lines[i] = layout.PageLines(i, w.analyses[i], w.data, w.font)
	}
}

func (w *Window) hasExtra(page int) bool {
	for _, e := range w.data.Extra {
		if e.Page == page {
			return true
		}
	}
	return false
}

func (w *Window) lineByID(page int, id string) *layout.Line {
	if page < 0 || page >= len(w.lines) {
		return nil
	}
	for i := range w.lines[page] {
		if w.lines[page][i].ID == id {
			return &w.lines[page][i]
		}
	}
	return nil
}

// chordLine resolves a chord's line, re-attaching it to the nearest line if
// its line no longer exists (e.g. hidden or re-detected differently).
func (w *Window) chordLine(c *doc.Chord) *layout.Line {
	if ln := w.lineByID(c.Page, c.Line); ln != nil {
		c.LineY = ln.Y
		return ln
	}
	if c.Page < 0 || c.Page >= len(w.lines) || w.analyses[c.Page] == nil {
		return nil
	}
	var best *layout.Line
	bd := math.Inf(1)
	for i := range w.lines[c.Page] {
		ln := &w.lines[c.Page][i]
		if d := math.Abs(ln.Y - c.LineY); d < bd {
			best, bd = ln, d
		}
	}
	if best != nil {
		c.Line, c.LineY = best.ID, best.Y
	}
	return best
}

func (w *Window) place(c *doc.Chord) (layout.Placed, *layout.Line, bool) {
	ln := w.chordLine(c)
	if ln == nil {
		return layout.Placed{}, nil, false
	}
	return layout.Place(c, ln, w.font, w.analyses[c.Page], w.data.Settings), ln, true
}

// allLines returns every chord line in reading order.
func (w *Window) allLines() []*layout.Line {
	var out []*layout.Line
	for p := range w.lines {
		for i := range w.lines[p] {
			out = append(out, &w.lines[p][i])
		}
	}
	return out
}

func (w *Window) adjacentLine(ln *layout.Line, dir int) *layout.Line {
	all := w.allLines()
	for i, l := range all {
		if l.Key == ln.Key {
			if j := i + dir; j >= 0 && j < len(all) {
				return all[j]
			}
			return nil
		}
	}
	return nil
}

// sortedChords returns chord pointers in reading order.
func (w *Window) sortedChords() []*doc.Chord {
	out := make([]*doc.Chord, 0, len(w.data.Chords))
	for i := range w.data.Chords {
		out = append(out, &w.data.Chords[i])
	}
	lineY := func(c *doc.Chord) float64 {
		if ln := w.chordLine(c); ln != nil {
			return ln.Y
		}
		return c.LineY
	}
	sort.SliceStable(out, func(i, j int) bool {
		a, b := out[i], out[j]
		if a.Page != b.Page {
			return a.Page < b.Page
		}
		ya, yb := lineY(a), lineY(b)
		if math.Abs(ya-yb) > 0.5 {
			return ya < yb
		}
		return a.X < b.X
	})
	return out
}

// activeLineKey is the line of the selected chord, or of the cursor.
func (w *Window) activeLineKey() string {
	if w.data == nil {
		return ""
	}
	if c := w.data.Chord(w.sel); c != nil {
		if ln := w.chordLine(c); ln != nil {
			return ln.Key
		}
	}
	if w.cur.ok {
		return doc.LineKey(w.cur.page, w.cur.line)
	}
	return ""
}

func (w *Window) activeLine() *layout.Line {
	if c := w.data.Chord(w.sel); c != nil {
		return w.chordLine(c)
	}
	if w.cur.ok {
		return w.lineByID(w.cur.page, w.cur.line)
	}
	return nil
}

// --------------------------------------------------------------- undo/state

var mergeable = map[string]bool{"nudge": true, "nudgey": true, "adjline": true, "text-settings": true}

// pushUndo records the state before an edit. Repeated small edits of the same
// kind on the same target (e.g. holding an arrow key) collapse into one step.
func (w *Window) pushUndo(op string, target int) {
	key := fmt.Sprintf("%s:%d", op, target)
	now := time.Now()
	if mergeable[op] && key == w.lastOp && now.Sub(w.lastOpTime) < 1500*time.Millisecond {
		w.lastOpTime = now
		return
	}
	w.lastOp, w.lastOpTime = key, now
	w.undo = append(w.undo, snapshot{edits: w.data.Edits.Clone(), sel: w.sel})
	if len(w.undo) > 500 {
		w.undo = w.undo[1:]
	}
	w.redo = nil
}

func (w *Window) undoOp() {
	if len(w.undo) == 0 {
		w.toast("Nothing to undo")
		return
	}
	w.redo = append(w.redo, snapshot{edits: w.data.Edits.Clone(), sel: w.sel})
	s := w.undo[len(w.undo)-1]
	w.undo = w.undo[:len(w.undo)-1]
	w.restore(s)
}

func (w *Window) redoOp() {
	if len(w.redo) == 0 {
		w.toast("Nothing to redo")
		return
	}
	w.undo = append(w.undo, snapshot{edits: w.data.Edits.Clone(), sel: w.sel})
	s := w.redo[len(w.redo)-1]
	w.redo = w.redo[:len(w.redo)-1]
	w.restore(s)
}

func (w *Window) restore(s snapshot) {
	w.data.Edits = s.edits.Clone()
	w.lastOp = ""
	w.editing = false
	w.sel = 0
	if w.data.Chord(s.sel) != nil {
		w.selectChord(s.sel, true)
	}
	w.changed()
}

// changed must be called after every edit.
func (w *Window) changed() {
	w.recomputeLines()
	w.scheduleSave()
	w.updateTitle()
	w.updateStatus()
	w.queueDrawAll()
}

// ----------------------------------------------------------- selection/cursor

func (w *Window) selectChord(id int, scroll bool) {
	c := w.data.Chord(id)
	if c == nil {
		return
	}
	if w.editing && id != w.sel {
		w.cancelEdit()
	}
	w.sel = id
	w.cur = cursor{ok: true, page: c.Page, line: c.Line, x: c.X}
	if scroll {
		if pl, _, ok := w.place(c); ok {
			w.ensureVisible(c.Page, c.X, pl.Baseline)
		}
	}
	w.updateStatus()
	w.queueDrawAll()
}

func (w *Window) deselect() {
	if c := w.data.Chord(w.sel); c != nil {
		w.cur = cursor{ok: true, page: c.Page, line: c.Line, x: c.X}
	}
	w.sel = 0
	w.editing = false
	w.updateStatus()
	w.queueDrawAll()
}

func (w *Window) placeCursor(page int, px, py float64) {
	w.sel = 0
	p := w.pages[page]
	ln := p.nearestLine(py)
	if ln == nil {
		if w.analyses[page] == nil {
			w.toast("Still analysing this page…")
		} else {
			w.toast("No chord lines on this page. Ctrl+click to add one.")
		}
		w.updateStatus()
		w.queueDrawAll()
		return
	}
	x := px
	if w.data.Settings.SnapNotes {
		x = ln.NearestNote(px, w.font.Size*0.9)
	}
	w.cur = cursor{ok: true, page: page, line: ln.ID, x: x}
	w.updateStatus()
	w.queueDrawAll()
}

func (w *Window) cursorToStart() {
	all := w.allLines()
	if len(all) == 0 {
		return
	}
	ln := all[0]
	x := ln.X0 + 20
	if n, ok := ln.NextNote(ln.X0, 1, 0); ok {
		x = n
	}
	w.cur = cursor{ok: true, page: ln.Page, line: ln.ID, x: x}
}

// ------------------------------------------------------------------ inserting

func (w *Window) pendingText() string {
	if w.sb.entry == nil {
		return ""
	}
	return strings.TrimSpace(w.sb.entry.Text())
}

type preview struct {
	page int
	line *layout.Line
	x    float64
	text string
}

// nextPosition computes where a chord with text goes after a chord (or cursor)
// at (ln, x) whose text is prevText ("" for a bare cursor).
func (w *Window) nextPosition(ln *layout.Line, x float64, prevText, text string) (*layout.Line, float64) {
	m := w.font.Measure(text)
	if prevText == "" {
		return ln, x
	}
	pm := w.font.Measure(prevText)
	gap := w.font.Size * 0.4
	need := x + pm.W/2 + gap + m.W/2
	nx := need
	if w.data.Settings.SnapNotes {
		if n, ok := ln.NextNote(need-0.01, 1, 0); ok {
			nx = n
		}
	}
	if nx+m.W/2 > ln.X1+w.font.Size {
		if next := w.adjacentLine(ln, 1); next != nil {
			x0 := next.X0 + m.W/2
			if w.data.Settings.SnapNotes {
				if n, ok := next.NextNote(next.X0+m.W/2*0.5, 1, 0); ok {
					x0 = n
				}
			}
			return next, x0
		}
	}
	return ln, nx
}

// previewPositions lays out the (space separated) chords typed in the entry.
func (w *Window) previewPositions(input string) []preview {
	var out []preview
	var ln *layout.Line
	var x float64
	prev := ""
	if c := w.data.Chord(w.sel); c != nil {
		ln = w.chordLine(c)
		x, prev = c.X, c.Text
	} else if w.cur.ok {
		ln = w.lineByID(w.cur.page, w.cur.line)
		x = w.cur.x
	}
	if ln == nil {
		return nil
	}
	for _, t := range strings.Fields(input) {
		ln, x = w.nextPosition(ln, x, prev, t)
		out = append(out, preview{page: ln.Page, line: ln, x: x, text: t})
		prev = t
	}
	return out
}

func (w *Window) commitEntry() {
	if w.data == nil {
		return
	}
	text := w.pendingText()
	if text == "" {
		if w.sel != 0 && !w.editing {
			w.startEdit(w.sel)
		}
		return
	}
	if w.editing {
		if c := w.data.Chord(w.sel); c != nil {
			w.pushUndo("edit", c.ID)
			c.Text = strings.Join(strings.Fields(text), " ")
		}
		w.editing = false
		w.sb.entry.SetText("")
		w.changed()
		return
	}
	pv := w.previewPositions(text)
	if len(pv) == 0 {
		w.toast("Click on the score to choose where the chord goes")
		return
	}
	w.pushUndo("insert", 0)
	for _, p := range pv {
		c := doc.Chord{ID: w.data.NextID, Page: p.page, Line: p.line.ID, LineY: p.line.Y, X: p.x, Text: p.text}
		w.data.NextID++
		w.data.Chords = append(w.data.Chords, c)
		w.sel = c.ID
	}
	w.sb.entry.SetText("")
	last := pv[len(pv)-1]
	w.cur = cursor{ok: true, page: last.page, line: last.line.ID, x: last.x}
	w.changed()
	w.ensureVisible(last.page, last.x, last.line.Y)
}

func (w *Window) startEdit(id int) {
	c := w.data.Chord(id)
	if c == nil {
		return
	}
	w.sel = id
	w.editing = true
	w.sb.entry.SetText(c.Text)
	w.refocus()
	w.sb.entry.SetPosition(-1)
	w.updateStatus()
	w.queueDrawAll()
}

func (w *Window) cancelEdit() {
	w.editing = false
	w.sb.entry.SetText("")
	w.updateStatus()
	w.queueDrawAll()
}

// ------------------------------------------------------------------- editing

func (w *Window) nudge(dx float64) {
	if c := w.data.Chord(w.sel); c != nil {
		w.pushUndo("nudge", c.ID)
		c.X = clampF(c.X+dx, 0, w.pages[c.Page].pw)
		w.cur.x = c.X
		w.changed()
		return
	}
	if w.cur.ok {
		w.cur.x = clampF(w.cur.x+dx, 0, w.pages[w.cur.page].pw)
		w.queueDrawAll()
	}
}

func (w *Window) jumpNote(dir int) {
	ln := w.activeLine()
	if ln == nil {
		return
	}
	x := w.cur.x
	if c := w.data.Chord(w.sel); c != nil {
		x = c.X
	}
	nx, ok := ln.NextNote(x, dir, 0.75)
	target := ln
	if !ok {
		adj := w.adjacentLine(ln, dir)
		if adj == nil || len(adj.Notes) == 0 {
			return
		}
		target = adj
		if dir > 0 {
			nx = adj.Notes[0]
		} else {
			nx = adj.Notes[len(adj.Notes)-1]
		}
	}
	w.moveTo(target, nx)
}

func (w *Window) moveTo(ln *layout.Line, x float64) {
	if c := w.data.Chord(w.sel); c != nil {
		w.pushUndo("nudge", c.ID)
		c.Page, c.Line, c.LineY, c.X = ln.Page, ln.ID, ln.Y, x
		w.cur = cursor{ok: true, page: ln.Page, line: ln.ID, x: x}
		w.changed()
	} else {
		w.cur = cursor{ok: true, page: ln.Page, line: ln.ID, x: x}
		w.updateStatus()
		w.queueDrawAll()
	}
	w.ensureVisible(ln.Page, x, ln.Y)
}

func (w *Window) changeLine(dir int) {
	ln := w.activeLine()
	if ln == nil {
		if !w.cur.ok {
			w.cursorToStart()
			w.queueDrawAll()
		}
		return
	}
	next := w.adjacentLine(ln, dir)
	if next == nil {
		return
	}
	x := w.cur.x
	if c := w.data.Chord(w.sel); c != nil {
		x = c.X
	}
	w.moveTo(next, clampF(x, 0, w.pages[next.Page].pw))
}

func (w *Window) nudgeY(d float64) {
	c := w.data.Chord(w.sel)
	if c == nil {
		return
	}
	pl, _, ok := w.place(c)
	if !ok {
		return
	}
	w.pushUndo("nudgey", c.ID)
	v := pl.AutoDY + d
	c.DY = &v
	w.changed()
}

func (w *Window) resetChordY(id int) {
	if c := w.data.Chord(id); c != nil && c.DY != nil {
		w.pushUndo("resety", id)
		c.DY = nil
		w.changed()
	}
}

func (w *Window) adjustLine(d float64) {
	ln := w.activeLine()
	if ln == nil {
		return
	}
	w.pushUndo("adjline", int(hashKey(ln.Key)))
	w.data.LineAdjust[ln.Key] += d
	w.changed()
}

func hashKey(s string) uint32 {
	var h uint32 = 2166136261
	for i := 0; i < len(s); i++ {
		h = (h ^ uint32(s[i])) * 16777619
	}
	return h >> 1
}

func (w *Window) resetLine(key string) {
	w.pushUndo("resetline", 0)
	delete(w.data.LineAdjust, key)
	w.changed()
}

func (w *Window) resetLines() {
	if w.data == nil {
		return
	}
	w.pushUndo("resetlines", 0)
	w.data.LineAdjust = map[string]float64{}
	w.data.Hidden = map[string]bool{}
	w.data.Extra = nil
	w.changed()
	w.toast("Chord lines reset to the detected positions")
}

func (w *Window) addLineAt(page int, py float64) {
	if w.data == nil {
		return
	}
	pw := w.pages[page].pw
	x0, x1 := pw*0.08, pw*0.92
	// Borrow the horizontal extent of the nearest staff if there is one.
	if a := w.analyses[page]; a != nil && len(a.Staves) > 0 {
		best := math.Inf(1)
		for _, st := range a.Staves {
			if d := math.Abs(st.Top - py); d < best {
				best, x0, x1 = d, st.X0, st.X1
			}
		}
	}
	w.pushUndo("addline", 0)
	id := fmt.Sprintf("m%d", w.data.NextID)
	w.data.NextID++
	w.data.Extra = append(w.data.Extra, doc.ExtraLine{ID: id, Page: page, Y: py, X0: x0, X1: x1})
	w.sel = 0
	w.cur = cursor{ok: true, page: page, line: id, x: clampF(w.cur.x, x0, x1)}
	if w.cur.page != page || w.cur.x == 0 {
		w.cur.x = x0 + 20
	}
	w.changed()
	w.toast("Chord line added")
}

func (w *Window) removeLine(key string) {
	var page int
	var id string
	fmt.Sscanf(strings.Replace(key, "/", " ", 1), "%d %s", &page, &id)
	w.pushUndo("removeline", 0)
	kept := w.data.Chords[:0]
	removed := 0
	for _, c := range w.data.Chords {
		if c.Page == page && c.Line == id {
			removed++
			continue
		}
		kept = append(kept, c)
	}
	w.data.Chords = kept
	extra := w.data.Extra[:0]
	isExtra := false
	for _, e := range w.data.Extra {
		if e.Page == page && e.ID == id {
			isExtra = true
			continue
		}
		extra = append(extra, e)
	}
	w.data.Extra = extra
	if !isExtra {
		w.data.Hidden[key] = true
	}
	if w.data.Chord(w.sel) == nil {
		w.sel = 0
	}
	if w.cur.ok && w.cur.page == page && w.cur.line == id {
		w.cur.ok = false
	}
	w.changed()
	msg := "Chord line removed"
	if removed > 0 {
		msg = fmt.Sprintf("Chord line removed with %d chords (Ctrl+Z to undo)", removed)
	}
	w.toast(msg)
}

func (w *Window) removeCurrentLine() {
	if w.data == nil {
		return
	}
	if key := w.activeLineKey(); key != "" {
		w.removeLine(key)
	}
}

func (w *Window) selectRel(dir int) {
	cs := w.sortedChords()
	if len(cs) == 0 {
		return
	}
	idx := -1
	for i, c := range cs {
		if c.ID == w.sel {
			idx = i
		}
	}
	var j int
	switch {
	case idx >= 0:
		j = idx + dir
	case dir > 0:
		j = 0
	default:
		j = len(cs) - 1
	}
	if j < 0 || j >= len(cs) {
		return
	}
	w.selectChord(cs[j].ID, true)
}

func (w *Window) deleteSel(backward bool) {
	cs := w.sortedChords()
	idx := -1
	for i, c := range cs {
		if c.ID == w.sel {
			idx = i
		}
	}
	if idx < 0 {
		return
	}
	del := *cs[idx]
	var nextID int
	if backward && idx > 0 {
		nextID = cs[idx-1].ID
	} else if !backward && idx+1 < len(cs) {
		nextID = cs[idx+1].ID
	}
	w.pushUndo("delete", del.ID)
	w.data.Remove(del.ID)
	w.sel = 0
	w.editing = false
	w.cur = cursor{ok: true, page: del.Page, line: del.Line, x: del.X}
	if nextID != 0 {
		w.selectChord(nextID, true)
	}
	w.changed()
}

// ------------------------------------------------------------------ keyboard

func (w *Window) refocus() {
	if w.sb.entry != nil && w.data != nil {
		w.sb.entry.GrabFocus()
	}
}

func (w *Window) onKey(keyval uint, state gdk.ModifierType) bool {
	if w.data == nil || w.win.VisibleDialog() != nil {
		return false
	}
	ctrl := state&gdk.ControlMask != 0
	shift := state&gdk.ShiftMask != 0
	alt := state&gdk.AltMask != 0
	if !w.entryFocused {
		if keyval == gdk.KEY_Escape && w.popover.Visible() == false {
			w.refocus()
			return true
		}
		return false
	}
	empty := w.sb.entry.Text() == ""
	free := empty || alt

	dir := 0
	switch keyval {
	case gdk.KEY_Left, gdk.KEY_KP_Left, gdk.KEY_Up, gdk.KEY_KP_Up:
		dir = -1
	case gdk.KEY_Right, gdk.KEY_KP_Right, gdk.KEY_Down, gdk.KEY_KP_Down:
		dir = 1
	}

	switch keyval {
	case gdk.KEY_Left, gdk.KEY_Right, gdk.KEY_KP_Left, gdk.KEY_KP_Right:
		if !free {
			return false
		}
		switch {
		case ctrl:
			w.jumpNote(dir)
		case shift:
			w.nudge(float64(dir) * nudgeBigStep)
		default:
			w.nudge(float64(dir) * nudgeStep)
		}
		return true
	case gdk.KEY_Up, gdk.KEY_Down, gdk.KEY_KP_Up, gdk.KEY_KP_Down:
		switch {
		case ctrl:
			w.adjustLine(float64(dir) * vStep)
		case shift:
			w.nudgeY(float64(dir) * vStep)
		default:
			w.changeLine(dir)
		}
		return true
	case gdk.KEY_Tab, gdk.KEY_KP_Tab:
		if ctrl {
			return false
		}
		if shift {
			w.selectRel(-1)
		} else {
			w.selectRel(1)
		}
		return true
	case gdk.KEY_ISO_Left_Tab:
		w.selectRel(-1)
		return true
	case gdk.KEY_Delete, gdk.KEY_KP_Delete:
		if !empty || ctrl {
			return false
		}
		w.deleteSel(false)
		return true
	case gdk.KEY_BackSpace:
		if !empty {
			return false
		}
		w.deleteSel(true)
		return true
	case gdk.KEY_Escape:
		switch {
		case w.editing:
			w.cancelEdit()
		case !empty:
			w.sb.entry.SetText("")
		case w.sel != 0:
			w.deselect()
		}
		return true
	case gdk.KEY_F2:
		w.startEdit(w.sel)
		return true
	case gdk.KEY_Home, gdk.KEY_End:
		if !empty {
			return false
		}
		if keyval == gdk.KEY_Home {
			w.sel = 0
			w.selectRel(1)
		} else {
			w.sel = 0
			w.selectRel(-1)
		}
		return true
	case gdk.KEY_Page_Up, gdk.KEY_Page_Down:
		if alt {
			return false
		}
		vadj := w.scroller.VAdjustment()
		step := vadj.PageSize() * 0.85
		if keyval == gdk.KEY_Page_Up {
			step = -step
		}
		vadj.SetValue(vadj.Value() + step)
		return true
	case gdk.KEY_z, gdk.KEY_Z:
		if !ctrl || !empty {
			return false
		}
		if shift {
			w.redoOp()
		} else {
			w.undoOp()
		}
		return true
	case gdk.KEY_y, gdk.KEY_Y:
		if !ctrl || !empty {
			return false
		}
		w.redoOp()
		return true
	case gdk.KEY_r, gdk.KEY_R:
		if !ctrl || !empty {
			return false
		}
		w.resetChordY(w.sel)
		return true
	}
	return false
}
