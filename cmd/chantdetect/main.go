// chantdetect renders the layout analysis onto page images for inspection.
//
//	chantdetect [-size 9] [-out dir] file.pdf...
package main

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/diamondburned/gotk4/pkg/cairo"

	"github.com/dominic/chantedit/internal/analysis"
	"github.com/dominic/chantedit/internal/doc"
	"github.com/dominic/chantedit/internal/layout"
	"github.com/dominic/chantedit/internal/poppler"
)

func main() {
	size := flag.Float64("size", 9, "chord font size (pt)")
	out := flag.String("out", "/tmp/chantdetect", "output directory")
	zoom := flag.Float64("zoom", 2, "output scale")
	flag.Parse()
	os.MkdirAll(*out, 0o755)

	for _, path := range flag.Args() {
		pdf, err := poppler.Open(path)
		if err != nil {
			fmt.Fprintln(os.Stderr, path, err)
			continue
		}
		stem := strings.ReplaceAll(strings.TrimSuffix(filepath.Base(path), ".pdf"), " ", "_")
		for i := 0; i < pdf.NPages(); i++ {
			t := time.Now()
			pw, ph := pdf.PageSize(i)
			gray, w, h, _ := pdf.RenderGray(i, analysis.DPI)
			pa := analysis.Analyze(gray, w, h, pw, ph)
			dt := time.Since(t)

			z := *zoom
			surf := cairo.CreateImageSurface(cairo.FormatRGB24, int(pw*z), int(ph*z))
			cr := cairo.Create(surf)
			cr.SetSourceRGB(1, 1, 1)
			cr.Paint()
			cr.Scale(z, z)
			pdf.Render(i, cr, false)
			font := layout.NewFont("Sans Bold", *size)
			capH := font.CapH
			data := &doc.Data{Settings: doc.DefaultSettings()}
			data.Settings.Size = *size
			lines := layout.PageLines(i, pa, data, font)
			for k, st := range pa.Staves {
				cr.SetSourceRGBA(0, 0.4, 1, 0.18)
				cr.Rectangle(st.X0, st.Top, st.X1-st.X0, st.Bottom-st.Top)
				cr.Fill()
				f := pa.FitLine(k, capH)
				cr.SetSourceRGBA(0.6, 0.6, 0.6, 0.15)
				cr.Rectangle(st.X0, f.Ceil, st.X1-st.X0, f.Floor-f.Ceil)
				cr.Fill()
				if f.Tight {
					cr.SetSourceRGBA(1, 0.5, 0, 0.3)
				} else {
					cr.SetSourceRGBA(0, 0.8, 0, 0.25)
				}
				cr.Rectangle(st.X0, f.GapTop, st.X1-st.X0, f.GapBottom-f.GapTop)
				cr.Fill()
				// A sample chord-sized box at the line.
				cr.SetSourceRGB(1, 0, 0)
				cr.SetLineWidth(0.4)
				cr.MoveTo(st.X0, f.Y)
				cr.LineTo(st.X1, f.Y)
				cr.Stroke()
				cr.SetSourceRGBA(1, 0, 1, 0.6)
				for _, x := range st.Notes {
					cr.Rectangle(x-0.5, st.Top-1.5, 1, 1)
				}
				cr.Fill()
			}
			// Sample chords on every third note, placed like the editor does.
			for li := range lines {
				ln := &lines[li]
				for ni := 1; ni < len(ln.Notes); ni += 3 {
					c := &doc.Chord{X: ln.Notes[ni], Text: "Am"}
					pl := layout.Place(c, ln, font, pa, data.Settings)
					if pl.AutoDY != 0 {
						cr.SetSourceRGB(0.8, 0, 0)
					} else {
						cr.SetSourceRGB(0, 0, 0)
					}
					font.Draw(cr, c.Text, c.X, pl.Baseline)
				}
			}
			name := filepath.Join(*out, fmt.Sprintf("%s-%d.png", stem, i+1))
			surf.WriteToPNG(name)
			fmt.Printf("%s p%d: %d staves, %v -> %s\n", stem, i+1, len(pa.Staves), dt.Round(time.Millisecond), name)
		}
		pdf.Close()
	}
}
