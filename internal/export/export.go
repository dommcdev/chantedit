// Package export writes a copy of the PDF with chord symbols added.
//
// Preferred path: draw the chords alone into a transparent PDF with cairo and
// stamp it on top of the original with `qpdf --overlay`, which leaves the
// original page content untouched. Without qpdf, pages are re-rendered
// through Poppler into a new PDF instead.
package export

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/diamondburned/gotk4/pkg/cairo"

	"github.com/dominic/chantedit/internal/layout"
	"github.com/dominic/chantedit/internal/poppler"
)

type Item struct {
	Text         string
	CX, Baseline float64
}

type Page struct {
	W, H  float64
	Items []Item
}

func Export(src, out string, pages []Page, font *layout.Font) error {
	if err := os.MkdirAll(filepath.Dir(out), 0o755); err != nil {
		return err
	}
	part := out + ".part"
	defer os.Remove(part)

	if qpdf, err := exec.LookPath("qpdf"); err == nil {
		overlay, err := os.CreateTemp("", "chantedit-overlay-*.pdf")
		if err != nil {
			return err
		}
		overlay.Close()
		defer os.Remove(overlay.Name())
		if err := writePDF(overlay.Name(), pages, font, nil); err != nil {
			return err
		}
		cmd := exec.Command(qpdf, "--warning-exit-0", src, "--overlay", overlay.Name(), "--", part)
		if msg, err := cmd.CombinedOutput(); err != nil {
			return fmt.Errorf("qpdf: %v: %s", err, strings.TrimSpace(string(msg)))
		}
	} else {
		d, err := poppler.Open(src)
		if err != nil {
			return err
		}
		defer d.Close()
		if err := writePDF(part, pages, font, d); err != nil {
			return err
		}
	}
	return os.Rename(part, out)
}

// writePDF draws the chords (optionally on top of the rendered source pages).
func writePDF(path string, pages []Page, font *layout.Font, src *poppler.Document) error {
	if len(pages) == 0 {
		return fmt.Errorf("document has no pages")
	}
	surf, err := cairo.CreatePDFSurface(path, pages[0].W, pages[0].H)
	if err != nil {
		return err
	}
	cr := cairo.Create(surf)
	for i, p := range pages {
		poppler.SetPDFPageSize(surf, p.W, p.H)
		if src != nil {
			cr.Save()
			src.Render(i, cr, true)
			cr.Restore()
		}
		cr.SetSourceRGB(0, 0, 0)
		for _, it := range p.Items {
			font.Draw(cr, it.Text, it.CX, it.Baseline)
		}
		cr.ShowPage()
	}
	poppler.FinishSurface(surf)
	if st := surf.Status(); st != cairo.StatusSuccess {
		return fmt.Errorf("cairo: %v", st)
	}
	return nil
}
