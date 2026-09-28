# ChantEdit

Add guitar chord names above chant (or other) scores in PDF files, fast and
mostly from the keyboard. Chord lines are detected automatically for every
staff, so you only choose *which note* a chord goes over, never its height.

Go + GTK4/libadwaita (gotk4), Poppler for rendering, qpdf for export.

## Build & run

```sh
make            # first build compiles gotk4 and takes several minutes
./bin/chantedit "~/Downloads/Chant/Te Deum (Simple tone).pdf"
make install    # optional: ~/.local/bin/chantedit + desktop entry
```

Build needs `go` plus the system `gtk4`, `libadwaita` (≥ 1.8), `poppler-glib`
and `cairo` headers; at runtime `qpdf` is used for export (it falls back to
re-rendering pages if qpdf is missing). On Arch:
`pacman -S go gtk4 libadwaita poppler-glib qpdf`.

The first build is slow because the gotk4 bindings are thousands of
generated cgo files; Go caches them, so later builds take a couple of seconds.

## Workflow

1. Open a PDF (Ctrl+O). The cursor is already on the first line, first note.
2. Type a chord in the sidebar box and press Enter. The next chord you enter
   goes after the selected one, on the next note. Type several at once
   separated by spaces (`C F Dm`) to place them on consecutive notes.
3. With the box empty, the arrows adjust the selected chord:
   - ← → nudge (Shift for bigger steps), Ctrl+← → jump note to note
   - ↑ ↓ move to the previous/next chord line
   - Shift+↑ ↓ raise/lower just this chord, Ctrl+R to reset it
   - Ctrl+↑ ↓ raise/lower the whole line
4. Click anywhere to put the cursor on the nearest line. Drag chords with the
   mouse. Tab / Shift+Tab walk through chords, Backspace deletes.
5. Ctrl+E exports `Name (chords).pdf` next to the original (or into
   `~/Documents/Chant with chords/` if that folder is read-only).
   Alt+Page Down opens the next PDF in the folder.

Everything is saved automatically to `~/.local/share/chantedit/docs/`, keyed by
a hash of the PDF, so reopening a file (even renamed or moved) brings your
chords back. Text size/font/options are remembered for the next file.

F1 opens the full list of keyboard shortcuts.

## How line detection works

`internal/analysis` works purely on a 150 dpi render, so it doesn't matter
whether the PDF is vector, text, or a scan:

1. **Staff lines** are rows containing long horizontal dark runs; lines with
   regular spacing are grouped into staves (3–6 lines, so 4-line chant and
   5-line modern staves both work).
2. **Text above each staff** (the previous system's lyrics, translations,
   titles) is found by rows that are cut into many thin, closely spaced
   strokes, which letters produce and note heads/stems don't.
3. The **chord line** is centred between the bottom of that text and the top
   staff line, capped at a comfortable distance from the staff when the gap
   is large. Lines where the chosen size doesn't really fit are drawn orange:
   lower the text size for those scores.
4. **Note columns** are found after erasing staff lines, which gives the snap
   targets for clicks, new chords and Ctrl+← →.
5. Each chord is then checked against the actual ink under it; if it would
   touch a high note or a descender it is shifted up (or down) just enough,
   within the free band ("Avoid Notes Automatically").

If detection misses a line, Ctrl+click to add one; right-click a line to
remove it or reset its position.

To inspect detection on a new kind of score:

```sh
make detect PDF="some score.pdf"   # overlays in /tmp/chantdetect
```

## Layout

```
main.go                  app entry
internal/analysis        staff / chord-line / note detection (pure Go)
internal/layout          lines + chord placement + text drawing (shared by UI and export)
internal/doc             saved data + preferences
internal/export          PDF export (cairo overlay + qpdf)
internal/poppler         tiny cgo wrapper for poppler-glib
internal/ui              libadwaita window, page view, keyboard handling
cmd/chantdetect          debug tool that draws detection results to PNGs
```

`CHANTEDIT_SCRIPT` drives the UI for testing (see `internal/ui/debug.go`).
