// Package analysis finds staves and chord-line positions on a rendered score
// page. It only looks at pixels, so it works the same for vector music, real
// text, or scanned images.
//
// Per page:
//  1. Threshold a grayscale render into a dark-pixel mask.
//  2. Staff lines are rows containing long horizontal dark runs.
//  3. Lines with regular spacing are grouped into staves (3-6 lines).
//  4. For every staff the band between it and the text above (previous
//     system's lyrics, titles, ...) is examined to find where a row of chord
//     symbols fits with the most clearance.
//  5. Note/neume columns are detected so chords can snap to notes.
//
// All public coordinates are PDF points with the origin at the top-left.
package analysis

import (
	"sort"
)

// DPI is the resolution pages are analysed at.
const DPI = 150.0

type Staff struct {
	Top, Bottom float64 // y of the top and bottom staff lines
	Space       float64 // distance between staff lines
	X0, X1      float64
	NLines      int
	Notes       []float64 // x centres of note columns, ascending
}

// Fit is a chord line fitted above a staff for a given chord height.
type Fit struct {
	Y         float64 // vertical centre of the chord text
	GapTop    float64 // the free band the line was centred in
	GapBottom float64
	Ceil      float64 // chords may be moved between Ceil and Floor to avoid ink
	Floor     float64
	Tight     bool // the chords do not really fit
}

type region struct {
	r0, r1 int       // rows (px) above the staff
	prof   []float64 // dark pixels per row within the staff's x range
	text   []bool    // row looks like it cuts through a line of text
}

type Page struct {
	W, H     float64 // page size (pt)
	Scale    float64 // px per pt
	pw, ph   int
	integral []int32 // summed-area table of the dark mask, (ph+1)*(pw+1)
	Staves   []Staff
	regions  []region
}

// Analyze inspects a grayscale page render (0 = black).
func Analyze(gray []uint8, w, h int, pageW, pageH float64) *Page {
	dark := make([]uint8, w*h)
	for i, g := range gray {
		if g < 150 {
			dark[i] = 1
		}
	}
	p := &Page{W: pageW, H: pageH, Scale: float64(w) / pageW, pw: w, ph: h}
	p.integral = make([]int32, (w+1)*(h+1))
	for y := 0; y < h; y++ {
		var rowSum int32
		for x := 0; x < w; x++ {
			rowSum += int32(dark[y*w+x])
			p.integral[(y+1)*(w+1)+x+1] = p.integral[y*(w+1)+x+1] + rowSum
		}
	}

	lines := findLines(dark, w, h)
	groups := groupStaves(lines, h)
	s := p.Scale
	for _, g := range groups {
		top, bottom := g[0].y, g[len(g)-1].y
		sp := (bottom - top) / float64(len(g)-1)
		x0s := make([]float64, len(g))
		x1s := make([]float64, len(g))
		for i, ln := range g {
			x0s[i], x1s[i] = ln.x0, ln.x1
		}
		x0, x1 := median(x0s), median(x1s)
		notes := noteColumns(dark, w, h, top, bottom, sp, x0, x1, g)
		for i := range notes {
			notes[i] /= s
		}
		p.Staves = append(p.Staves, Staff{
			Top: top / s, Bottom: bottom / s, Space: sp / s,
			X0: x0 / s, X1: x1 / s, NLines: len(g), Notes: notes,
		})
	}

	for i, st := range p.Staves {
		topPx := st.Top * s
		spPx := st.Space * s
		limit := topPx - 9*spPx
		r0 := limit
		for j := i - 1; j >= 0; j-- {
			o := p.Staves[j]
			if o.Bottom < st.Top && min(o.X1, st.X1)-max(o.X0, st.X0) > 0 {
				r0 = max(limit, o.Bottom*s+0.5*spPx)
				break
			}
		}
		ri0 := max(0, int(r0))
		ri1 := max(ri0+1, int(topPx-0.15*spPx))
		c0 := clamp(int(st.X0*s), 0, w)
		c1 := clamp(int(st.X1*s), 0, w)
		n := ri1 - ri0
		rg := region{r0: ri0, r1: ri1, prof: make([]float64, n), text: make([]bool, n)}
		maxShort := max(2, int(0.4*spPx))
		// Letters in a word sit close together; stems and bar lines are
		// isolated thin strokes a note-width apart.
		maxGap := max(3, int(0.7*spPx))
		shortRuns := make([]int, n)
		for k := 0; k < n; k++ {
			y := ri0 + k
			if y >= h {
				break
			}
			row := dark[y*w : y*w+w]
			cnt := 0
			prevEnd, prevShort := -1<<30, false
			x := c0
			for x < c1 {
				if row[x] == 0 {
					x++
					continue
				}
				s := x
				for x < c1 && row[x] == 1 {
					x++
				}
				cnt += x - s
				short := x-s <= maxShort
				if short && s-prevEnd <= maxGap {
					shortRuns[k]++
					if prevShort && shortRuns[k] == 1 {
						shortRuns[k]++ // count the first stroke of the word too
					}
				}
				prevEnd, prevShort = x, short
			}
			rg.prof[k] = float64(cnt)
		}
		// Text rows are cut into many thin, closely spaced strokes; note heads
		// give few wide runs and stems are far apart. Bridge single-row holes.
		for k := 0; k < n; k++ {
			rg.text[k] = shortRuns[k] >= 4
		}
		for k := 1; k+1 < n; k++ {
			if !rg.text[k] && rg.text[k-1] && rg.text[k+1] {
				rg.text[k] = true
			}
		}
		p.regions = append(p.regions, rg)
	}
	return p
}

// InkInRect counts dark pixels inside a rectangle given in points.
func (p *Page) InkInRect(x0, y0, x1, y1 float64) int {
	s := p.Scale
	c0 := clamp(int(x0*s), 0, p.pw)
	c1 := clamp(int(x1*s+0.999), 0, p.pw)
	r0 := clamp(int(y0*s), 0, p.ph)
	r1 := clamp(int(y1*s+0.999), 0, p.ph)
	if c1 <= c0 || r1 <= r0 {
		return 0
	}
	W := p.pw + 1
	ii := p.integral
	return int(ii[r1*W+c1] - ii[r0*W+c1] - ii[r1*W+c0] + ii[r0*W+c0])
}

// FitLine finds the chord line above staff i for chords chordH points tall
// (cap height).
func (p *Page) FitLine(i int, chordH float64) Fit {
	st := p.Staves[i]
	rg := p.regions[i]
	s := p.Scale
	sp := st.Space * s
	n := len(rg.prof)
	widthPx := max(1, (st.X1-st.X0)*s)
	need := int(chordH*s*1.2) + 2

	// Lowest text line above the staff (the previous system's lyrics, or a
	// title). Rows right on top of the staff are notes, not text.
	barrier := -1
	minRows := max(3, int(0.3*sp))
	skip := int(0.4 * sp)
	cnt := 0
	for y := n - 1 - skip; y >= 0; y-- {
		if rg.text[y] {
			cnt++
			if cnt >= minRows {
				barrier = y + cnt - 1
				break
			}
		} else {
			cnt = 0
		}
	}
	hi := n
	dense := max(2, 0.05*widthPx)
	top := 0
	if barrier >= 0 {
		// Include descender-heavy rows directly under the text.
		top = barrier + 1
		for top < hi && rg.prof[top] >= dense {
			top++
		}
	} else {
		// No text line (e.g. first system under a title): stop at the first
		// substantial ink above the zone where high notes live.
		y := hi - 1 - int(1.6*sp)
		for y >= 0 && rg.prof[y] < dense {
			y--
		}
		top = max(0, y+1)
	}
	bot := hi

	toPt := func(row int) float64 { return float64(rg.r0+row) / s }
	fit := Fit{Ceil: toPt(top), Floor: st.Top - 0.1*st.Space}

	// The line is centred between the text above and the staff itself. Notes
	// poking above the staff and descenders are left to per-chord collision
	// avoidance, so a single high note doesn't push the whole line around.
	// In a big gap, stay close to the staff instead of floating in the middle.
	maxDist := max(float64(need)*1.8, sp*2.2)
	if float64(bot-top) > maxDist {
		top = bot - int(maxDist)
	}
	fit.Tight = bot-top < need
	fit.GapTop, fit.GapBottom = toPt(top), toPt(bot)
	fit.Y = (fit.GapTop + fit.GapBottom) / 2
	return fit
}

// ---------------------------------------------------------------- staff lines

type line struct {
	y, thick float64
	x0, x1   float64
}

func findLines(dark []uint8, w, h int) []line {
	// Close small horizontal gaps (broken scan lines) and merge each row with
	// the one below so slightly tilted lines still give long runs.
	m := make([]uint8, w*h)
	const gap = 2
	for y := 0; y < h; y++ {
		row := dark[y*w : y*w+w]
		out := m[y*w : y*w+w]
		last := -1000
		for x := 0; x < w; x++ {
			if row[x] == 1 {
				if d := x - last; d > 1 && d <= gap+1 {
					for k := last + 1; k < x; k++ {
						out[k] = 1
					}
				}
				out[x] = 1
				last = x
			}
		}
	}
	for y := 0; y+1 < h; y++ {
		a := m[y*w : y*w+w]
		b := m[(y+1)*w : (y+1)*w+w]
		for x := range a {
			a[x] |= b[x]
		}
	}

	minRun := max(20, int(float64(w)*0.05))
	score := make([]float64, h)
	rx0 := make([]float64, h)
	rx1 := make([]float64, h)
	for y := 0; y < h; y++ {
		row := m[y*w : y*w+w]
		rx0[y], rx1[y] = float64(w), 0
		x := 0
		for x < w {
			if row[x] == 0 {
				x++
				continue
			}
			s := x
			for x < w && row[x] == 1 {
				x++
			}
			if l := x - s; l >= minRun {
				score[y] += float64(l)
				rx0[y] = min(rx0[y], float64(s))
				rx1[y] = max(rx1[y], float64(x))
			}
		}
	}
	thr := max(float64(minRun*2), float64(w)*0.15)
	var lines []line
	for y := 0; y < h; {
		if score[y] < thr {
			y++
			continue
		}
		start := y
		var sw, swy float64
		var xs0, xs1 []float64
		for y < h && score[y] >= thr {
			sw += score[y]
			swy += score[y] * float64(y)
			xs0 = append(xs0, rx0[y])
			xs1 = append(xs1, rx1[y])
			y++
		}
		lines = append(lines, line{
			y: swy/sw + 0.5, thick: float64(y - start),
			x0: median(xs0), x1: median(xs1),
		})
	}
	return lines
}

func groupStaves(lines []line, h int) [][]line {
	maxSpace := float64(h) * 0.03
	overlap := func(a, b line) float64 {
		inter := min(a.x1, b.x1) - max(a.x0, b.x0)
		return inter / max(1, min(a.x1-a.x0, b.x1-b.x0))
	}
	var groups [][]line
	var cur []line
	flush := func() {
		if len(cur) >= 3 && len(cur) <= 6 {
			groups = append(groups, cur)
		}
		cur = nil
	}
	for _, ln := range lines {
		if len(cur) == 0 {
			cur = []line{ln}
			continue
		}
		prev := cur[len(cur)-1]
		d := ln.y - prev.y
		ok := d >= 3 && d <= maxSpace && overlap(ln, prev) > 0.6
		if ok && len(cur) >= 2 {
			sp := (prev.y - cur[0].y) / float64(len(cur)-1)
			ok = abs(d-sp) <= 0.25*sp+1
		}
		if ok && len(cur) < 6 {
			cur = append(cur, ln)
		} else {
			flush()
			cur = []line{ln}
		}
	}
	flush()
	return groups
}

// --------------------------------------------------------------- note columns

func noteColumns(dark []uint8, w, h int, top, bottom, sp, x0, x1 float64, lines []line) []float64 {
	r0 := clamp(int(top-2*sp), 0, h)
	r1 := clamp(int(bottom+1.5*sp), 0, h)
	c0 := clamp(int(x0), 0, w)
	c1 := clamp(int(x1), 0, w)
	if r1 <= r0 || c1 <= c0 {
		return nil
	}
	bw := c1 - c0
	bh := r1 - r0
	band := make([]uint8, bw*bh)
	for y := 0; y < bh; y++ {
		copy(band[y*bw:(y+1)*bw], dark[(r0+y)*w+c0:(r0+y)*w+c1])
	}
	// Remove staff line pixels, but keep note heads that straddle a line
	// (dark both just above and just below it).
	for _, ln := range lines {
		a := int(ln.y-ln.thick/2-1) - r0
		b := int(ln.y+ln.thick/2+1.5) - r0
		a, b = max(0, a), min(bh, b)
		if b <= a {
			continue
		}
		for x := 0; x < bw; x++ {
			keep := a-1 >= 0 && b < bh && band[(a-1)*bw+x] == 1 && band[b*bw+x] == 1
			if !keep {
				for y := a; y < b; y++ {
					band[y*bw+x] = 0
				}
			}
		}
	}
	col := make([]float64, bw)
	for y := 0; y < bh; y++ {
		for x := 0; x < bw; x++ {
			col[x] += float64(band[y*bw+x])
		}
	}
	thr := max(2, sp*0.35)
	minW := max(2, int(sp*0.35))
	head := max(sp*1.1, 1)
	var xs []float64
	for c := 0; c < bw; {
		if col[c] < thr {
			c++
			continue
		}
		s := c
		minCol := col[c]
		for c < bw && col[c] >= thr {
			minCol = min(minCol, col[c])
			c++
		}
		width := c - s
		if width < minW {
			continue // thin bar line, stem or noise
		}
		if minCol >= 0.85*(bottom-top) && width < int(sp) {
			continue // thick bar line: full staff height in every column
		}
		// Neumes written side by side merge into one wide cluster; split it
		// into roughly note-head sized pieces.
		k := max(1, int(float64(width)/head+0.5))
		for i := 0; i < k; i++ {
			x := float64(c0+s) + (float64(i)+0.5)*float64(width)/float64(k)
			// The clef sits right at the start of the staff.
			if x < x0+1.6*sp {
				continue
			}
			xs = append(xs, x)
		}
	}
	return xs
}

// ---------------------------------------------------------------------- utils

func median(v []float64) float64 {
	if len(v) == 0 {
		return 0
	}
	c := append([]float64(nil), v...)
	sort.Float64s(c)
	return c[len(c)/2]
}

func clamp(v, lo, hi int) int { return max(lo, min(hi, v)) }

func abs(v float64) float64 {
	if v < 0 {
		return -v
	}
	return v
}
