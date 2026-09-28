package ui

import (
	"math"
	"time"

	"github.com/diamondburned/gotk4-adwaita/pkg/adw"
	"github.com/diamondburned/gotk4/pkg/cairo"
	"github.com/diamondburned/gotk4/pkg/gdk/v4"
	"github.com/diamondburned/gotk4/pkg/gtk/v4"

	"github.com/dominic/chantedit/internal/layout"
)

type pageView struct {
	w        *Window
	idx      int
	area     *gtk.DrawingArea
	pw, ph   float64
	cache    *cairo.Surface
	cacheKey float64

	dragID     int
	dragStartX float64
	pressY     float64
	moved      bool
}

var (
	lastClickTime time.Time
	lastClickID   int
)

func (w *Window) buildPages() {
	for child := w.pagesBox.FirstChild(); child != nil; child = w.pagesBox.FirstChild() {
		w.pagesBox.Remove(child)
	}
	w.pages = nil
	for i := 0; i < w.pdf.NPages(); i++ {
		pw, ph := w.pdf.PageSize(i)
		p := &pageView{w: w, idx: i, pw: pw, ph: ph}
		p.area = gtk.NewDrawingArea()
		p.area.AddCSSClass("chant-page")
		p.area.SetHAlign(gtk.AlignCenter)
		p.area.SetDrawFunc(func(_ *gtk.DrawingArea, cr *cairo.Context, _, _ int) { p.draw(cr) })

		drag := gtk.NewGestureDrag()
		drag.SetButton(gdk.BUTTON_PRIMARY)
		drag.ConnectDragBegin(func(x, y float64) {
			p.dragBegin(x, y, drag.CurrentEventState())
		})
		drag.ConnectDragUpdate(p.dragUpdate)
		drag.ConnectDragEnd(func(_, _ float64) { p.dragEnd() })
		p.area.AddController(drag)

		menu := gtk.NewGestureClick()
		menu.SetButton(gdk.BUTTON_SECONDARY)
		menu.ConnectPressed(func(_ int, x, y float64) { p.contextMenu(x, y) })
		p.area.AddController(menu)

		w.pagesBox.Append(p.area)
		w.pages = append(w.pages, p)
	}
	w.applyZoom()
}

func (w *Window) applyZoom() {
	for _, p := range w.pages {
		p.area.SetContentWidth(int(p.pw * w.zoomOr1()))
		p.area.SetContentHeight(int(p.ph * w.zoomOr1()))
		p.area.QueueDraw()
	}
}

func (w *Window) zoomOr1() float64 {
	if w.zoom <= 0 {
		return 1.4
	}
	return w.zoom
}

func (w *Window) setZoom(z float64) {
	if w.data == nil {
		return
	}
	z = math.Max(0.3, math.Min(6, z))
	vadj := w.scroller.VAdjustment()
	old := w.zoomOr1()
	// Keep the point in the middle of the view in place.
	mid := (vadj.Value() + vadj.PageSize()/2 - pageMargin) / old
	w.zoom = z
	w.prefs.Zoom = z
	w.applyZoom()
	idle(func() {
		vadj.SetValue(mid*z + pageMargin - vadj.PageSize()/2)
	})
}

func (w *Window) fitZoom() float64 {
	maxW := 0.0
	for _, p := range w.pages {
		maxW = math.Max(maxW, p.pw)
	}
	avail := float64(w.scroller.Width()) - 2*pageMargin - 20
	if maxW <= 0 || avail <= 100 {
		return 1.4
	}
	return avail / maxW
}

func (w *Window) queueDrawAll() {
	for _, p := range w.pages {
		p.area.QueueDraw()
	}
}

// ------------------------------------------------------------------ drawing

func accent() (r, g, b float64) {
	c := adw.StyleManagerGetDefault().AccentColorRGBA()
	if c == nil {
		return 0.21, 0.52, 0.89
	}
	return float64(c.Red()), float64(c.Green()), float64(c.Blue())
}

func (p *pageView) draw(cr *cairo.Context) {
	w := p.w
	if w.pdf == nil {
		return
	}
	z := w.zoomOr1()
	sf := float64(p.area.ScaleFactor())
	key := z * sf
	if p.cache == nil || p.cacheKey != key {
		W := int(p.pw*key + 0.5)
		H := int(p.ph*key + 0.5)
		s := cairo.CreateImageSurface(cairo.FormatRGB24, W, H)
		c := cairo.Create(s)
		c.SetSourceRGB(1, 1, 1)
		c.Paint()
		c.Scale(key, key)
		w.pdf.Render(p.idx, c, false)
		s.Flush()
		p.cache, p.cacheKey = s, key
	}
	cr.Save()
	cr.Scale(1/sf, 1/sf)
	cr.SetSourceSurface(p.cache, 0, 0)
	cr.Paint()
	cr.Restore()

	cr.Scale(z, z)
	ar, ag, ab := accent()
	lines := w.lines[p.idx]
	curLine := w.activeLineKey()

	if w.sb.guides.Active() {
		for _, ln := range lines {
			active := ln.Key == curLine
			alpha := 0.45
			if active {
				alpha = 0.95
			}
			if ln.Tight {
				cr.SetSourceRGBA(0.9, 0.45, 0, alpha)
			} else {
				cr.SetSourceRGBA(ar, ag, ab, alpha)
			}
			if active {
				cr.SetDash(nil, 0)
			} else {
				cr.SetDash([]float64{3 / z, 3 / z}, 0)
			}
			cr.SetLineWidth(1 / z)
			cr.MoveTo(ln.X0, ln.Y)
			cr.LineTo(ln.X1, ln.Y)
			cr.Stroke()
		}
		cr.SetDash(nil, 0)
	}

	for i := range w.data.Chords {
		c := &w.data.Chords[i]
		if c.Page != p.idx {
			continue
		}
		pl, _, ok := w.place(c)
		if !ok {
			continue
		}
		if c.ID == w.sel {
			pad := 1.5
			roundRect(cr, pl.X0-pad, pl.Y0-pad, pl.X1-pl.X0+2*pad, pl.Y1-pl.Y0+2*pad, 2)
			cr.SetSourceRGBA(ar, ag, ab, 0.25)
			cr.FillPreserve()
			cr.SetSourceRGBA(ar, ag, ab, 0.9)
			cr.SetLineWidth(1 / z)
			if w.editing {
				cr.SetLineWidth(2 / z)
			}
			cr.Stroke()
		}
		cr.SetSourceRGB(0, 0, 0)
		w.font.Draw(cr, c.Text, c.X, pl.Baseline)
	}

	// Preview of where the chord being typed will go.
	if text := w.pendingText(); text != "" && !w.editing {
		for _, pv := range w.previewPositions(text) {
			if pv.page != p.idx {
				continue
			}
			cr.SetSourceRGBA(ar, ag, ab, 0.7)
			w.font.Draw(cr, pv.text, pv.x, pv.line.Y+w.font.CapH/2)
		}
	} else if w.sel == 0 && w.cur.ok && w.cur.page == p.idx {
		if ln := w.lineByID(p.idx, w.cur.line); ln != nil {
			capH := w.font.CapH
			cr.SetSourceRGBA(ar, ag, ab, 1)
			cr.SetLineWidth(1.5 / z)
			cr.MoveTo(w.cur.x, ln.Y-capH*0.9)
			cr.LineTo(w.cur.x, ln.Y+capH*0.9)
			cr.Stroke()
			tri := 2.5
			cr.MoveTo(w.cur.x-tri, ln.Y-capH*0.9-tri)
			cr.LineTo(w.cur.x+tri, ln.Y-capH*0.9-tri)
			cr.LineTo(w.cur.x, ln.Y-capH*0.9)
			cr.ClosePath()
			cr.Fill()
		}
	}
}

func roundRect(cr *cairo.Context, x, y, w, h, r float64) {
	cr.NewSubPath()
	cr.Arc(x+w-r, y+r, r, -math.Pi/2, 0)
	cr.Arc(x+w-r, y+h-r, r, 0, math.Pi/2)
	cr.Arc(x+r, y+h-r, r, math.Pi/2, math.Pi)
	cr.Arc(x+r, y+r, r, math.Pi, 3*math.Pi/2)
	cr.ClosePath()
}

// -------------------------------------------------------------------- mouse

func (p *pageView) toPage(x, y float64) (float64, float64) {
	z := p.w.zoomOr1()
	return x / z, y / z
}

func (p *pageView) hit(px, py float64) int {
	w := p.w
	const pad = 2.0
	for i := len(w.data.Chords) - 1; i >= 0; i-- {
		c := &w.data.Chords[i]
		if c.Page != p.idx {
			continue
		}
		pl, _, ok := w.place(c)
		if ok && px >= pl.X0-pad && px <= pl.X1+pad && py >= pl.Y0-pad && py <= pl.Y1+pad {
			return c.ID
		}
	}
	return 0
}

func (p *pageView) nearestLine(py float64) *layout.Line {
	var best *layout.Line
	bd := math.Inf(1)
	for i := range p.w.lines[p.idx] {
		ln := &p.w.lines[p.idx][i]
		if d := math.Abs(ln.Y - py); d < bd {
			best, bd = ln, d
		}
	}
	return best
}

func (p *pageView) dragBegin(x, y float64, state gdk.ModifierType) {
	w := p.w
	defer w.refocus()
	px, py := p.toPage(x, y)
	p.pressY = py
	p.moved = false
	p.dragID = 0
	if state&gdk.ControlMask != 0 {
		w.addLineAt(p.idx, py)
		return
	}
	if id := p.hit(px, py); id != 0 {
		now := time.Now()
		if id == lastClickID && now.Sub(lastClickTime) < 400*time.Millisecond {
			w.startEdit(id)
		} else if w.editing && id != w.sel {
			w.cancelEdit()
		}
		lastClickID, lastClickTime = id, now
		w.selectChord(id, false)
		p.dragID = id
		p.dragStartX = w.data.Chord(id).X
		return
	}
	lastClickID = 0
	if w.editing {
		w.cancelEdit()
	}
	w.placeCursor(p.idx, px, py)
}

func (p *pageView) dragUpdate(dx, dy float64) {
	w := p.w
	if p.dragID == 0 {
		return
	}
	z := w.zoomOr1()
	if !p.moved {
		if math.Abs(dx)+math.Abs(dy) < 4 {
			return
		}
		w.pushUndo("drag", p.dragID)
		p.moved = true
	}
	c := w.data.Chord(p.dragID)
	if c == nil {
		return
	}
	c.X = clampF(p.dragStartX+dx/z, 0, p.pw)
	if ln := p.nearestLine(p.pressY + dy/z); ln != nil {
		c.Line, c.LineY = ln.ID, ln.Y
	}
	w.cur = cursor{ok: true, page: p.idx, line: c.Line, x: c.X}
	w.queueDrawAll()
}

func (p *pageView) dragEnd() {
	if p.moved {
		p.w.changed()
	}
	p.dragID = 0
	p.moved = false
}

func (p *pageView) contextMenu(x, y float64) {
	w := p.w
	px, py := p.toPage(x, y)
	box := gtk.NewBox(gtk.OrientationVertical, 0)
	item := func(label string, f func()) {
		b := gtk.NewButtonWithLabel(label)
		b.AddCSSClass("flat")
		b.SetHAlign(gtk.AlignFill)
		if l, ok := b.Child().(*gtk.Label); ok {
			l.SetXAlign(0)
		}
		b.ConnectClicked(func() {
			w.popover.Popdown()
			f()
		})
		box.Append(b)
	}
	if id := p.hit(px, py); id != 0 {
		w.selectChord(id, false)
		item("Edit Chord (F2)", func() { w.startEdit(id) })
		if c := w.data.Chord(id); c != nil && c.DY != nil {
			item("Reset Vertical Position", func() { w.resetChordY(id) })
		}
		item("Delete Chord", func() { w.selectChord(id, false); w.deleteSel(true) })
	}
	item("Add Chord Line Here (Ctrl+Click)", func() { w.addLineAt(p.idx, py) })
	if ln := p.nearestLine(py); ln != nil && math.Abs(ln.Y-py) < 12 {
		key := ln.Key
		item("Remove This Chord Line", func() { w.removeLine(key) })
		if w.data.LineAdjust[key] != 0 {
			item("Reset This Line’s Position", func() { w.resetLine(key) })
		}
	}
	w.popover.SetChild(box)
	sx, sy, ok := p.area.TranslateCoordinates(w.scroller, x, y)
	if !ok {
		return
	}
	rect := gdk.NewRectangle(int(sx), int(sy), 1, 1)
	w.popover.SetPointingTo(&rect)
	w.popover.Popup()
}

// ensureVisible scrolls so that page point (x, y) is in view.
func (w *Window) ensureVisible(page int, x, y float64) {
	if page < 0 || page >= len(w.pages) {
		return
	}
	z := w.zoomOr1()
	top := float64(pageMargin)
	for i := 0; i < page; i++ {
		top += w.pages[i].ph*z + pageSpacing
	}
	target := top + y*z
	vadj := w.scroller.VAdjustment()
	margin := math.Min(140, vadj.PageSize()/4)
	if target < vadj.Value()+margin {
		vadj.SetValue(target - margin)
	} else if target > vadj.Value()+vadj.PageSize()-margin {
		vadj.SetValue(target - vadj.PageSize() + margin)
	}
	hadj := w.scroller.HAdjustment()
	if hadj.Upper() > hadj.PageSize()+1 {
		left := (hadj.Upper() - w.pages[page].pw*z) / 2
		tx := left + x*z
		hm := math.Min(100, hadj.PageSize()/4)
		if tx < hadj.Value()+hm {
			hadj.SetValue(tx - hm)
		} else if tx > hadj.Value()+hadj.PageSize()-hm {
			hadj.SetValue(tx - hadj.PageSize() + hm)
		}
	}
}

func clampF(v, lo, hi float64) float64 { return math.Max(lo, math.Min(hi, v)) }
