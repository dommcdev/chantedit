// Package layout turns analysis results + document edits into concrete chord
// lines and chord positions, and draws chord text. It is shared by the editor
// view and PDF export so both place text identically.
package layout

import (
	"fmt"
	"math"
	"sort"

	"github.com/diamondburned/gotk4/pkg/cairo"
	"github.com/diamondburned/gotk4/pkg/pango"
	"github.com/diamondburned/gotk4/pkg/pangocairo"

	"github.com/dominic/chantedit/internal/analysis"
	"github.com/dominic/chantedit/internal/doc"
)

// ---------------------------------------------------------------------- font

type Metrics struct {
	W       float64 // advance width
	Ascent  float64 // layout top to baseline
	InkX0   float64 // ink box relative to (left edge, baseline)
	InkX1   float64
	InkY0   float64
	InkY1   float64
	LogTop  float64 // logical box relative to baseline (negative = above)
	LogDown float64
}

type Font struct {
	Name  string
	Size  float64
	CapH  float64 // height of capital letters above the baseline
	desc  *pango.FontDescription
	cache map[string]Metrics
	mcr   *cairo.Context
}

func NewFont(name string, size float64) *Font {
	desc := pango.FontDescriptionFromString(name)
	desc.SetAbsoluteSize(size * pango.SCALE)
	surf := cairo.CreateImageSurface(cairo.FormatARGB32, 1, 1)
	f := &Font{Name: name, Size: size, desc: desc, cache: map[string]Metrics{}, mcr: cairo.Create(surf)}
	m := f.Measure("CEGH")
	f.CapH = -m.InkY0
	if f.CapH <= 0 {
		f.CapH = size * 0.72
	}
	return f
}

func (f *Font) Layout(cr *cairo.Context, text string) *pango.Layout {
	l := pangocairo.CreateLayout(cr)
	opts := cairo.CreateFontOptions()
	opts.SetHintMetrics(cairo.HintMetricsOff)
	opts.SetHintStyle(cairo.HintStyleNone)
	pangocairo.ContextSetFontOptions(l.Context(), opts)
	l.ContextChanged()
	l.SetFontDescription(f.desc)
	l.SetText(text)
	return l
}

func (f *Font) Measure(text string) Metrics {
	if m, ok := f.cache[text]; ok {
		return m
	}
	l := f.Layout(f.mcr, text)
	ink, logical := l.Extents()
	base := float64(l.Baseline()) / pango.SCALE
	sc := func(v int) float64 { return float64(v) / pango.SCALE }
	m := Metrics{
		W:       sc(logical.Width()),
		Ascent:  base,
		InkX0:   sc(ink.X()),
		InkX1:   sc(ink.X() + ink.Width()),
		InkY0:   sc(ink.Y()) - base,
		InkY1:   sc(ink.Y()+ink.Height()) - base,
		LogTop:  sc(logical.Y()) - base,
		LogDown: sc(logical.Y()+logical.Height()) - base,
	}
	f.cache[text] = m
	return m
}

// Draw renders text horizontally centred on cx with its baseline at baseline.
func (f *Font) Draw(cr *cairo.Context, text string, cx, baseline float64) {
	m := f.Measure(text)
	l := f.Layout(cr, text)
	cr.MoveTo(cx-m.W/2, baseline-m.Ascent)
	pangocairo.ShowLayout(cr, l)
}

// --------------------------------------------------------------------- lines

type Line struct {
	ID     string
	Key    string // doc.LineKey
	Page   int
	Staff  int // index into analysis staves, -1 for manual lines
	Y      float64
	X0, X1 float64
	Ceil   float64 // allowed vertical range for collision avoidance
	Floor  float64
	Tight  bool
	Notes  []float64
}

func StaffLineID(st analysis.Staff) string {
	return fmt.Sprintf("s%d", int(math.Round(st.Top*10)))
}

// PageLines computes the chord lines of one page, sorted top to bottom.
func PageLines(page int, a *analysis.Page, d *doc.Data, f *Font) []Line {
	var out []Line
	if a != nil {
		for k, st := range a.Staves {
			id := StaffLineID(st)
			key := doc.LineKey(page, id)
			if d.Hidden[key] {
				continue
			}
			fit := a.FitLine(k, f.CapH)
			y := fit.Y + d.Settings.LineOffset + d.LineAdjust[key]
			out = append(out, Line{
				ID: id, Key: key, Page: page, Staff: k,
				Y: y, X0: st.X0, X1: st.X1,
				// A line moved by hand still gets some room to dodge notes.
				Ceil:  min(fit.Ceil, y-1.5*f.CapH),
				Floor: max(fit.Floor, y+0.5*f.CapH+1),
				Tight: fit.Tight, Notes: st.Notes,
			})
		}
	}
	for _, e := range d.Extra {
		if e.Page != page {
			continue
		}
		key := doc.LineKey(page, e.ID)
		y := e.Y + d.LineAdjust[key]
		out = append(out, Line{
			ID: e.ID, Key: key, Page: page, Staff: -1, Y: y, X0: e.X0, X1: e.X1,
			Ceil: y - 2*f.Size, Floor: y + f.Size,
		})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Y < out[j].Y })
	return out
}

// ------------------------------------------------------------------- chords

type Placed struct {
	Baseline float64
	AutoDY   float64
	// Ink box in page coordinates, used for hit testing and highlighting.
	X0, Y0, X1, Y1 float64
}

// Place computes where a chord is drawn on its line.
func Place(c *doc.Chord, ln *Line, f *Font, a *analysis.Page, s doc.Settings) Placed {
	m := f.Measure(c.Text)
	base := ln.Y + f.CapH/2
	left := c.X - m.W/2
	var dy float64
	if c.DY != nil {
		dy = *c.DY
	} else if s.AutoAvoid && a != nil {
		dy = avoid(a, ln, f, m, left, base)
	}
	b := base + dy
	return Placed{
		Baseline: b, AutoDY: dy,
		X0: left + min(0, m.InkX0), X1: left + max(m.W, m.InkX1),
		Y0: b + min(-f.CapH, m.InkY0), Y1: b + max(0, m.InkY1),
	}
}

// avoid finds the smallest vertical shift (preferring up) that keeps the chord
// clear of the score's ink, within the line's allowed range.
func avoid(a *analysis.Page, ln *Line, f *Font, m Metrics, left, base float64) float64 {
	padX := f.Size * 0.08
	padY := f.Size * 0.12
	ink := func(dy float64) int {
		return a.InkInRect(left+m.InkX0-padX, base+dy+m.InkY0-padY,
			left+m.InkX1+padX, base+dy+m.InkY1+padY)
	}
	best, bestInk := 0.0, ink(0)
	if bestInk == 0 {
		return 0
	}
	step := 0.25
	maxShift := f.Size * 1.6
	for d := step; d <= maxShift; d += step {
		for _, dy := range []float64{-d, d} {
			top := base + dy + m.InkY0
			bot := base + dy + m.InkY1
			if top < ln.Ceil || bot > ln.Floor {
				continue
			}
			n := ink(dy)
			if n == 0 {
				return dy
			}
			if n < bestInk {
				best, bestInk = dy, n
			}
		}
	}
	return best
}

// NearestNote returns the note x on the line closest to x within maxDist, or
// x itself.
func (ln *Line) NearestNote(x, maxDist float64) float64 {
	best := x
	bd := maxDist
	for _, n := range ln.Notes {
		if d := math.Abs(n - x); d <= bd {
			best, bd = n, d
		}
	}
	return best
}

// NextNote returns the first note strictly right (dir>0) or left (dir<0) of x
// by more than eps.
func (ln *Line) NextNote(x float64, dir int, eps float64) (float64, bool) {
	if dir > 0 {
		for _, n := range ln.Notes {
			if n > x+eps {
				return n, true
			}
		}
	} else {
		for i := len(ln.Notes) - 1; i >= 0; i-- {
			if n := ln.Notes[i]; n < x-eps {
				return n, true
			}
		}
	}
	return 0, false
}
