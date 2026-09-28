// Package doc holds the persistent chord data for one PDF and the global
// preferences. Chord data is stored under ~/.local/share/chantedit keyed by a
// hash of the PDF contents, so it survives renames/moves and never needs
// write access to the PDF's folder.
package doc

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"strconv"
)

type Settings struct {
	Font       string  `json:"font"` // Pango family + style, e.g. "Sans Bold"
	Size       float64 `json:"size"` // pt
	AutoAvoid  bool    `json:"auto_avoid"`
	SnapNotes  bool    `json:"snap_notes"`
	LineOffset float64 `json:"line_offset"` // pt added to every line (negative = up)
}

func DefaultSettings() Settings {
	return Settings{Font: "Sans Bold", Size: 9, AutoAvoid: true, SnapNotes: true}
}

type Chord struct {
	ID    int     `json:"id"`
	Page  int     `json:"page"`
	Line  string  `json:"line"`
	LineY float64 `json:"line_y"` // last known line position, to re-attach if the line id changes
	X     float64 `json:"x"`      // horizontal centre (pt)
	Text  string  `json:"text"`
	// Manual vertical offset from the line (pt). nil = automatic placement.
	DY *float64 `json:"dy,omitempty"`
}

// ExtraLine is a chord line the user added by hand.
type ExtraLine struct {
	ID   string  `json:"id"`
	Page int     `json:"page"`
	Y    float64 `json:"y"`
	X0   float64 `json:"x0"`
	X1   float64 `json:"x1"`
}

// Edits is the undoable part of a document.
type Edits struct {
	Chords     []Chord            `json:"chords"`
	LineAdjust map[string]float64 `json:"line_adjust"` // LineKey -> pt
	Hidden     map[string]bool    `json:"hidden"`      // LineKey -> hidden
	Extra      []ExtraLine        `json:"extra_lines"`
	NextID     int                `json:"next_id"`
}

type Data struct {
	Version  int      `json:"version"`
	PDF      string   `json:"pdf"` // last known path, informational
	Hash     string   `json:"hash"`
	Settings Settings `json:"settings"`
	Edits
}

func LineKey(page int, id string) string {
	return strconv.Itoa(page) + "/" + id
}

func (e *Edits) Clone() Edits {
	b, _ := json.Marshal(e)
	var c Edits
	json.Unmarshal(b, &c)
	c.ensure()
	return c
}

func (e *Edits) ensure() {
	if e.LineAdjust == nil {
		e.LineAdjust = map[string]float64{}
	}
	if e.Hidden == nil {
		e.Hidden = map[string]bool{}
	}
	if e.NextID == 0 {
		e.NextID = 1
		for _, c := range e.Chords {
			e.NextID = max(e.NextID, c.ID+1)
		}
	}
}

func (e *Edits) Chord(id int) *Chord {
	for i := range e.Chords {
		if e.Chords[i].ID == id {
			return &e.Chords[i]
		}
	}
	return nil
}

func (e *Edits) Remove(id int) {
	for i := range e.Chords {
		if e.Chords[i].ID == id {
			e.Chords = append(e.Chords[:i], e.Chords[i+1:]...)
			return
		}
	}
}

// ------------------------------------------------------------------ storage

func dataDir() string {
	if d := os.Getenv("XDG_DATA_HOME"); d != "" {
		return filepath.Join(d, "chantedit")
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".local", "share", "chantedit")
}

func HashFile(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil))[:32], nil
}

func storePath(hash string) string {
	return filepath.Join(dataDir(), "docs", hash+".json")
}

// Load returns the saved data for a PDF, or a fresh document using defaults.
func Load(pdfPath string, defaults Settings) (*Data, error) {
	hash, err := HashFile(pdfPath)
	if err != nil {
		return nil, err
	}
	d := &Data{Version: 1, PDF: pdfPath, Hash: hash, Settings: defaults}
	b, err := os.ReadFile(storePath(hash))
	if err == nil {
		if err := json.Unmarshal(b, d); err != nil {
			return nil, err
		}
		d.PDF = pdfPath
	} else if !errors.Is(err, fs.ErrNotExist) {
		return nil, err
	}
	d.ensure()
	return d, nil
}

func (d *Data) Save() error {
	return writeJSON(storePath(d.Hash), d)
}

func writeJSON(path string, v any) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	b, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, b, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// -------------------------------------------------------------- preferences

type Prefs struct {
	Defaults  Settings `json:"defaults"` // applied to newly opened PDFs
	LastDir   string   `json:"last_dir"`
	ExportDir string   `json:"export_dir"` // used when the PDF's folder is not writable
	Zoom      float64  `json:"zoom"`
	Guides    bool     `json:"guides"`
}

func prefsPath() string {
	d, err := os.UserConfigDir()
	if err != nil {
		d = filepath.Join(os.Getenv("HOME"), ".config")
	}
	return filepath.Join(d, "chantedit", "prefs.json")
}

func LoadPrefs() *Prefs {
	p := &Prefs{Defaults: DefaultSettings(), Guides: true}
	if b, err := os.ReadFile(prefsPath()); err == nil {
		json.Unmarshal(b, p)
	}
	if p.Defaults.Size <= 0 {
		p.Defaults = DefaultSettings()
	}
	return p
}

func (p *Prefs) Save() error {
	return writeJSON(prefsPath(), p)
}
