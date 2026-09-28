package analysis_test

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/dominic/chantedit/internal/analysis"
	"github.com/dominic/chantedit/internal/poppler"
)

// Sample scores live outside the repo; the test is skipped without them.
var samples = map[string][]int{
	"Te Deum (Simple tone).pdf": {7, 7, 7, 7, 0},
	"Cantate Domino 2.pdf":      {8},
	"Mass5SAE_lg.pdf":           {7, 4, 6, 7, 4},
}

func sampleDir() string {
	if d := os.Getenv("CHANTEDIT_SAMPLES"); d != "" {
		return d
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, "Downloads", "Chant")
}

func TestSamples(t *testing.T) {
	for name, want := range samples {
		path := filepath.Join(sampleDir(), name)
		if _, err := os.Stat(path); err != nil {
			t.Skipf("sample %s not available", path)
		}
		d, err := poppler.Open(path)
		if err != nil {
			t.Fatal(err)
		}
		for i, n := range want {
			pw, ph := d.PageSize(i)
			gray, w, h, _ := d.RenderGray(i, analysis.DPI)
			pa := analysis.Analyze(gray, w, h, pw, ph)
			if len(pa.Staves) != n {
				t.Errorf("%s page %d: %d staves, want %d", name, i+1, len(pa.Staves), n)
				continue
			}
			for k, st := range pa.Staves {
				if st.NLines != 4 {
					t.Errorf("%s p%d staff %d: %d lines", name, i+1, k, st.NLines)
				}
				f := pa.FitLine(k, 6.5)
				// The chord line must be above the staff and within a few
				// staff spaces of it.
				if f.Y >= st.Top || st.Top-f.Y > 4*st.Space {
					t.Errorf("%s p%d staff %d: line at %.1f, staff top %.1f", name, i+1, k, f.Y, st.Top)
				}
				if len(st.Notes) < 5 {
					t.Errorf("%s p%d staff %d: only %d notes", name, i+1, k, len(st.Notes))
				}
			}
		}
		d.Close()
	}
}
